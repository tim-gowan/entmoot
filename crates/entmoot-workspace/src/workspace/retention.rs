//! Operator-controlled retention of complete publication views.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    pub keep_recent: usize,
    #[serde(default)]
    pub pinned_views: BTreeSet<String>,
    /// Explicit maintenance compaction; requires spare disk space.
    #[serde(default)]
    pub compact: bool,
    /// Admission high-water mark, not a filesystem quota or a peak-write bound.
    #[serde(default)]
    pub max_workspace_bytes: Option<u64>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_recent: 2,
            pinned_views: BTreeSet::new(),
            compact: false,
            max_workspace_bytes: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPlan {
    pub schema_version: String,
    pub root: PathBuf,
    pub policy: RetentionPolicy,
    pub fence: String,
    pub retained_views: BTreeSet<String>,
    pub remove_generations: Vec<String>,
    pub remove_views: BTreeSet<String>,
    pub remove_objects: Vec<String>,
    pub file_bytes_before: u64,
    pub file_bytes_removable: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionReceipt {
    pub schema_version: String,
    pub plan_digest: String,
    pub removed_views: usize,
    pub removed_generations: usize,
    pub removed_objects: usize,
    pub file_bytes_after: u64,
}

pub fn plan_retention(
    root: &Path,
    policy: RetentionPolicy,
) -> Result<RetentionPlan, WorkspaceError> {
    let root = fs::canonicalize(root)?;
    checked_path(&storage_directory(&root)?, &storage_directory(&root)?)?;
    checked_path(
        &storage_directory(&root)?,
        &storage_directory(&root)?.join("knowledge.sqlite3"),
    )?;
    let _writer = WriteGuard::acquire(&root)?;
    let database = storage_directory(&root)?.join("knowledge.sqlite3");
    let store = crate::knowledge_db::retention::Maintenance::open(&database).map_err(failure)?;
    let _objects = object_lease(&database)?;
    build_plan(&root, policy, &store)
}

pub fn apply_retention(
    root: &Path,
    plan: &RetentionPlan,
) -> Result<RetentionReceipt, WorkspaceError> {
    let root = fs::canonicalize(root)?;
    if plan.schema_version != "entmoot.retention_plan.v1" || root != plan.root {
        return Err(failure("retention plan schema/root mismatch"));
    }
    checked_path(&storage_directory(&root)?, &storage_directory(&root)?)?;
    checked_path(
        &storage_directory(&root)?,
        &storage_directory(&root)?.join("knowledge.sqlite3"),
    )?;
    let _writer = WriteGuard::acquire(&root)?;
    let database = storage_directory(&root)?.join("knowledge.sqlite3");
    let mut store =
        crate::knowledge_db::retention::Maintenance::open(&database).map_err(failure)?;
    let _objects = object_lease(&database)?;
    let mut fresh = build_plan(&root, plan.policy.clone(), &store)?;
    // Saving a plan/receipt changes administrative file usage, not its roots.
    fresh.file_bytes_before = plan.file_bytes_before;
    if serde_json::to_value(&fresh)? != serde_json::to_value(plan)? {
        return Err(failure(
            "stale or altered retention plan; generate and review a new plan",
        ));
    }
    let receipt_path = storage_directory(&root)?.join("retention-receipt.json");
    // A crash after the DB commit can leave unreachable files, never live rows
    // referring to deleted bytes. The bounded journal makes this state explicit.
    write_json(
        &receipt_path,
        &serde_json::json!({"schema_version":"entmoot.retention_journal.v1", "state":"applying", "plan":plan}),
    )?;
    write_json(
        &storage_directory(&root)?.join("retention-policy.json"),
        &plan.policy,
    )?;
    store.collect(&plan.remove_views).map_err(failure)?;
    for name in &plan.remove_generations {
        let path = storage_directory(&root)?.join("current").join(name);
        // build_plan checked every entry without following links. Recheck each
        // exact target immediately before deletion, under the lifecycle lock.
        checked_path(&storage_directory(&root)?.join("current"), &path)?;
        fs::remove_dir_all(path)?;
    }
    let objects = artifact_store_path(&database)?;
    for digest in &plan.remove_objects {
        let path = object_path(&objects, digest)?;
        checked_path(&objects, &path)?;
        fs::remove_file(path)?;
    }
    if plan.policy.compact {
        store.compact().map_err(failure)?;
    } else {
        store.checkpoint().map_err(failure)?;
    }
    let receipt = RetentionReceipt {
        schema_version: "entmoot.retention_receipt.v1".into(),
        plan_digest: blake3_hex(&serde_json::to_vec(plan)?),
        removed_views: plan.remove_views.len(),
        removed_generations: plan.remove_generations.len(),
        removed_objects: plan.remove_objects.len(),
        file_bytes_after: tree_bytes(&storage_directory(&root)?)?,
    };
    write_json(&receipt_path, &receipt)?;
    Ok(receipt)
}

fn object_lease(
    database: &Path,
) -> Result<Option<crate::storage_lease::StorageLease>, WorkspaceError> {
    let objects = artifact_store_path(database)?;
    if objects.exists() {
        checked_path(
            database
                .parent()
                .ok_or_else(|| failure("database has no parent"))?,
            &objects,
        )?;
        Ok(Some(
            crate::storage_lease::exclusive(&objects.join("objects")).map_err(failure)?,
        ))
    } else {
        Ok(None)
    }
}

struct Generation {
    name: String,
    view: String,
    modified: std::time::SystemTime,
    summary: WorkspaceSummary,
}

fn build_plan(
    root: &Path,
    policy: RetentionPolicy,
    store: &crate::knowledge_db::retention::Maintenance,
) -> Result<RetentionPlan, WorkspaceError> {
    if policy.keep_recent < 2 {
        return Err(failure(
            "retention keep_recent must be at least 2; current and comparison base are roots",
        ));
    }
    if policy.max_workspace_bytes == Some(0) {
        return Err(failure("max_workspace_bytes must be positive"));
    }
    let publication = read_publication(&storage_directory(root)?.join("current.json"))?;
    let directory = storage_directory(root)?.join("current");
    let catalog = store.catalog().map_err(failure)?;
    for view in &policy.pinned_views {
        if !catalog.views.contains(view) {
            return Err(failure(format!("unknown retention pin {view}")));
        }
    }
    let mut generations = Vec::new();
    let mut disk_fence = blake3::Hasher::new();
    hash_tree(&directory, &directory, &mut disk_fence)?;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        checked_path(&directory, &entry.path())?;
        if !entry.file_type()?.is_dir() {
            return Err(failure("unexpected file in generation directory"));
        }
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| failure("non-UTF8 generation name"))?
            .to_string();
        if !name.starts_with("generation-") {
            return Err(failure(
                "unknown generation directory; inspect before retention",
            ));
        }
        let summary: WorkspaceSummary =
            serde_json::from_slice(&fs::read(entry.path().join("summary.json"))?)?;
        if summary.schema_version != "entmoot.workspace_summary.v1" {
            return Err(failure("unsupported generation summary schema"));
        }
        if summary.publication.graph != entry.path().join("graph.json")
            || summary.publication.store != entry.path().join("graph.json.knowledge.view.json")
        {
            return Err(failure(
                "generation summary paths do not match managed layout",
            ));
        }
        let locator = crate::KnowledgeViewLocator::read(&summary.publication.store)
            .map_err(failure)?
            .ok_or_else(|| failure("generation has no view locator"))?;
        if fs::canonicalize(locator.database)?
            != fs::canonicalize(storage_directory(root)?.join("knowledge.sqlite3"))?
        {
            return Err(failure("generation references another database"));
        }
        generations.push(Generation {
            name,
            view: locator.view,
            modified: entry.metadata()?.modified()?,
            summary,
        });
    }
    generations.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
    let mut keep_names = BTreeSet::new();
    for location in std::iter::once(&publication.current).chain(publication.base.iter()) {
        let generation = generations
            .iter()
            .find(|g| {
                g.summary.publication.graph == location.graph
                    && g.summary.publication.store == location.store
            })
            .ok_or_else(|| failure("current/base outside known managed generations"))?;
        if !catalog.views.contains(&generation.view) {
            return Err(failure("current/base view has expired or is missing"));
        }
        keep_names.insert(generation.name.clone());
    }
    let mut retained_views = policy.pinned_views.clone();
    let mut recent = BTreeSet::new();
    for generation in &generations {
        if recent.len() < policy.keep_recent && catalog.views.contains(&generation.view) {
            recent.insert(generation.view.clone());
        }
    }
    retained_views.extend(recent);
    // An unrepresented view can belong to an external export/operator. Lack of
    // a workspace generation is never authority to delete it.
    retained_views.extend(
        catalog
            .views
            .iter()
            .filter(|v| !generations.iter().any(|g| &g.view == *v))
            .cloned(),
    );
    for view in &retained_views {
        if let Some(generation) = generations.iter().find(|g| &g.view == view) {
            keep_names.insert(generation.name.clone());
        }
    }
    loop {
        let before = keep_names.len();
        for generation in &generations {
            if !keep_names.contains(&generation.name) {
                continue;
            }
            for manifest in generation
                .summary
                .stages
                .iter()
                .filter_map(|s| s.manifest.as_ref())
            {
                if let Ok(relative) = manifest.strip_prefix(&directory) {
                    let name = relative
                        .components()
                        .next()
                        .ok_or_else(|| failure("invalid retained manifest"))?
                        .as_os_str()
                        .to_string_lossy()
                        .to_string();
                    if !generations.iter().any(|g| g.name == name) || !manifest.is_file() {
                        return Err(failure("retained producer manifest unavailable"));
                    }
                    keep_names.insert(name);
                }
            }
        }
        if keep_names.len() == before {
            break;
        }
    }
    retained_views.extend(
        generations
            .iter()
            .filter(|g| keep_names.contains(&g.name))
            .map(|g| g.view.clone()),
    );
    let remove_views = catalog.views.difference(&retained_views).cloned().collect();
    let remove_generations: Vec<_> = generations
        .iter()
        .filter(|g| !keep_names.contains(&g.name))
        .map(|g| g.name.clone())
        .collect();
    let live_objects = store.artifact_digests(&retained_views).map_err(failure)?;
    let mut dead_objects = store.artifact_digests(&remove_views).map_err(failure)?;
    // Recover a crash after row retirement but before object removal. Recompute
    // liveness from today's roots; a newly referenced object is never swept.
    let journal_path = storage_directory(root)?.join("retention-receipt.json");
    if journal_path.exists() {
        let journal: serde_json::Value = serde_json::from_slice(&fs::read(journal_path)?)?;
        if journal.get("state").and_then(|v| v.as_str()) == Some("applying") {
            if journal.get("schema_version").and_then(|v| v.as_str())
                != Some("entmoot.retention_journal.v1")
            {
                return Err(failure("unsupported retention journal"));
            }
            let interrupted: RetentionPlan = serde_json::from_value(journal["plan"].clone())?;
            if interrupted.root != root || interrupted.schema_version != "entmoot.retention_plan.v1"
            {
                return Err(failure("interrupted retention root/schema mismatch"));
            }
            dead_objects.extend(interrupted.remove_objects);
        }
    }
    let objects = artifact_store_path(&storage_directory(root)?.join("knowledge.sqlite3"))?;
    let mut remove_objects = Vec::new();
    let mut file_bytes_removable = 0;
    for name in &remove_generations {
        file_bytes_removable += tree_bytes(&directory.join(name))?;
    }
    if objects.exists() {
        hash_tree(&objects, &objects, &mut disk_fence)?;
        for digest in dead_objects.difference(&live_objects) {
            let path = object_path(&objects, digest)?;
            if path.exists() {
                checked_path(&objects, &path)?;
                file_bytes_removable += path.metadata()?.len();
                remove_objects.push(digest.clone());
            }
        }
    }
    let policy_file = storage_directory(root)?.join("retention-policy.json");
    let policy_digest = if policy_file.exists() {
        Some(blake3_hex(&fs::read(policy_file)?))
    } else {
        None
    };
    let fence = blake3_hex(&serde_json::to_vec(&(
        catalog.fence,
        policy_digest,
        blake3_hex(&fs::read(storage_directory(root)?.join("current.json"))?),
        disk_fence.finalize().to_hex().to_string(),
    ))?);
    Ok(RetentionPlan {
        schema_version: "entmoot.retention_plan.v1".into(),
        root: root.into(),
        policy,
        fence,
        retained_views,
        remove_generations,
        remove_views,
        remove_objects,
        file_bytes_before: tree_bytes(&storage_directory(root)?)?,
        file_bytes_removable,
    })
}

pub fn read_retention_policy(root: &Path) -> Result<RetentionPolicy, WorkspaceError> {
    let path = storage_directory(root)?.join("retention-policy.json");
    if path.exists() {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Ok(Default::default())
    }
}

pub(crate) fn check_admission(root: &Path) -> Result<(), WorkspaceError> {
    let policy = read_retention_policy(root)?;
    if let Some(limit) = policy.max_workspace_bytes {
        let bytes = tree_bytes(&storage_directory(root)?)?;
        if bytes >= limit {
            return Err(failure(format!("storage admission refused: {bytes} workspace bytes meet/exceed high-water mark {limit}; retain/export/prune explicitly or raise the operator budget")));
        }
    }
    Ok(())
}

/// Apply the workspace budget to explicit CLI index/attach writes into its
/// shared store. Independent stores have their own operator admission boundary.
pub fn check_store_admission(store: &Path) -> Result<(), WorkspaceError> {
    let database = crate::KnowledgeViewLocator::read(store)
        .map_err(failure)?
        .map_or_else(|| store.to_owned(), |locator| locator.database);
    if database
        .file_name()
        .is_some_and(|name| name == "knowledge.sqlite3")
    {
        if let Some(directory) = database.parent() {
            let policy_path = directory.join("retention-policy.json");
            if policy_path.is_file() {
                let policy: RetentionPolicy = serde_json::from_slice(&fs::read(policy_path)?)?;
                if let Some(limit) = policy.max_workspace_bytes {
                    if tree_bytes(directory)? >= limit {
                        return Err(failure(
                            "storage admission refused; inspect the workspace retention budget",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn object_path(root: &Path, digest: &str) -> Result<PathBuf, WorkspaceError> {
    let digest = entmoot_contracts::ContentDigest::parse(digest.to_string()).map_err(failure)?;
    let hex = &digest.as_str()[7..];
    Ok(root.join("blake3").join(&hex[..2]).join(&hex[2..]))
}

fn checked_path(root: &Path, path: &Path) -> Result<(), WorkspaceError> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(failure("retention refuses reparse points"));
        }
    }
    if metadata.file_type().is_symlink()
        || !fs::canonicalize(path)?.starts_with(fs::canonicalize(root)?)
    {
        return Err(failure("retention path escapes managed root"));
    }
    Ok(())
}

fn tree_bytes(root: &Path) -> Result<u64, WorkspaceError> {
    let mut bytes = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        checked_path(root, &entry.path())?;
        bytes += if entry.file_type()?.is_dir() {
            tree_bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(bytes)
}

fn hash_tree(root: &Path, path: &Path, hasher: &mut blake3::Hasher) -> Result<(), WorkspaceError> {
    checked_path(root, path)?;
    // Lock bytes are administrative, not evidence. Windows locks are mandatory
    // and an exclusive lease intentionally forbids reopening its bytes to read.
    if path
        .extension()
        .is_some_and(|extension| extension == "lease")
    {
        return Ok(());
    }
    if path.is_dir() {
        let mut paths = fs::read_dir(path)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        for path in paths {
            hash_tree(root, &path, hasher)?;
        }
    } else {
        let relative = path.strip_prefix(root).map_err(failure)?.to_string_lossy();
        hasher.update(&(relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(&path.metadata()?.len().to_le_bytes());
        std::io::copy(&mut fs::File::open(path)?, hasher)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeStore;

    fn publish(root: &Path, value: usize) -> WorkspaceSummary {
        fs::write(
            root.join("a.py"),
            format!("LIMIT = {value}\ndef unchanged():\n    return LIMIT\n"),
        )
        .unwrap();
        initialize_workspace(
            root,
            Some("retention-fixture"),
            &Default::default(),
            &BindOptions {
                structural_only: true,
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn retention_preserves_current_base_and_shared_unchanged_callable() {
        let dir = tempfile::tempdir().unwrap();
        let first = publish(dir.path(), 0);
        let first_view = crate::KnowledgeViewLocator::read(&first.publication.store)
            .unwrap()
            .unwrap()
            .view;
        for value in 1..6 {
            publish(dir.path(), value);
        }
        let plan = plan_retention(
            dir.path(),
            RetentionPolicy {
                compact: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(plan.remove_views.len(), 4);
        assert!(plan.remove_views.contains(&first_view));
        assert_eq!(plan.retained_views.len(), 2);
        let receipt = apply_retention(dir.path(), &plan).unwrap();
        assert_eq!(receipt.removed_views, 4);
        assert!(receipt.file_bytes_after < plan.file_bytes_before);
        assert!(!first.publication.store.exists());
        let current = discover_publication(dir.path()).unwrap();
        for location in [current.current, current.base.unwrap()] {
            let db = SqliteKnowledgeRepository::open_read_snapshot(&location.store).unwrap();
            let graph = db.load_inventory(false).unwrap().unwrap();
            graph.validate().unwrap();
            assert!(graph.nodes.iter().any(|n| n.label == "unchanged"));
        }
    }

    #[test]
    fn retention_pins_stale_plans_and_live_readers_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let first = publish(dir.path(), 0);
        let view = crate::KnowledgeViewLocator::read(&first.publication.store)
            .unwrap()
            .unwrap()
            .view;
        for value in 1..4 {
            publish(dir.path(), value);
        }
        let policy = RetentionPolicy {
            pinned_views: [view.clone()].into(),
            ..Default::default()
        };
        let plan = plan_retention(dir.path(), policy.clone()).unwrap();
        assert!(plan.retained_views.contains(&view));
        let opened = open_query_publication(
            &first.publication.graph,
            &first.publication.store,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(apply_retention(dir.path(), &plan)
            .unwrap_err()
            .to_string()
            .contains("busy"));
        drop(opened);
        publish(dir.path(), 4);
        assert!(apply_retention(dir.path(), &plan)
            .unwrap_err()
            .to_string()
            .contains("stale"));
        let plan = plan_retention(dir.path(), policy).unwrap();
        apply_retention(dir.path(), &plan).unwrap();
        assert!(first.publication.store.exists());
    }

    #[test]
    fn structural_repeat_does_not_allocate_another_generation() {
        let dir = tempfile::tempdir().unwrap();
        let first = publish(dir.path(), 0);
        let second = publish(dir.path(), 0);
        let left: serde_json::Value =
            serde_json::from_slice(&fs::read(&first.publication.graph).unwrap()).unwrap();
        let right: serde_json::Value =
            serde_json::from_slice(&fs::read(&second.publication.graph).unwrap()).unwrap();
        for (key, value) in left.as_object().unwrap() {
            if key == "nodes" {
                for (a, b) in value
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(right[key].as_array().unwrap())
                {
                    assert_eq!(a, b, "node");
                }
            }
            assert_eq!(value, &right[key], "{key}");
        }
        assert_eq!(second.publication.graph, first.publication.graph);
        assert_eq!(second.publication_outcome, "unchanged");
        assert_eq!(second.indexed_files, 0);
        assert_eq!(second.reused_syntax_files, 1);
    }

    #[test]
    fn retention_rejects_unknown_pins_tampering_and_admission_over_budget() {
        let dir = tempfile::tempdir().unwrap();
        for value in 0..3 {
            publish(dir.path(), value);
        }
        let unknown = RetentionPolicy {
            pinned_views: [blake3_hex(b"unknown")].into(),
            ..Default::default()
        };
        assert!(plan_retention(dir.path(), unknown)
            .unwrap_err()
            .to_string()
            .contains("unknown retention pin"));
        let mut plan = plan_retention(dir.path(), Default::default()).unwrap();
        plan.remove_generations.push("../../outside".into());
        assert!(apply_retention(dir.path(), &plan)
            .unwrap_err()
            .to_string()
            .contains("altered"));
        let pointer = fs::read(
            crate::workspace::storage_directory(dir.path())
                .unwrap()
                .join("current.json"),
        )
        .unwrap();
        write_json(
            &crate::workspace::storage_directory(dir.path())
                .unwrap()
                .join("retention-policy.json"),
            &RetentionPolicy {
                max_workspace_bytes: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        std::fs::write(dir.path().join("a.py"), "LIMIT = 999\n").unwrap();
        let result = initialize_workspace(
            dir.path(),
            Some("retention-fixture"),
            &Default::default(),
            &BindOptions {
                structural_only: true,
                ..Default::default()
            },
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("storage admission refused"));
        assert_eq!(
            pointer,
            fs::read(
                crate::workspace::storage_directory(dir.path())
                    .unwrap()
                    .join("current.json")
            )
            .unwrap()
        );
    }

    #[test]
    fn retention_marks_inactive_evidence_and_recovers_after_row_commit() {
        use entmoot_contracts::{
            ArtifactFormat, ArtifactManifest, ArtifactProducer, ContentDigest, ScopeCompleteness,
            ScopeUniverse,
        };
        let dir = tempfile::tempdir().unwrap();
        let first = publish(dir.path(), 0);
        let pool = crate::FilesystemArtifactObjectRepository::new(
            artifact_store_path(&first.publication.store).unwrap(),
        );
        let attach = |publication: &WorkspaceSummary, owner: &str, bytes: &[u8]| {
            let digest = ContentDigest::parse(blake3_hex(bytes)).unwrap();
            pool.put(&digest, bytes).unwrap();
            let manifest = ArtifactManifest::build(
                ArtifactFormat::Sarif21,
                ArtifactProducer {
                    name: owner.into(),
                    version: Some("1".into()),
                    information_uri: None,
                    ruleset_id: None,
                    config_class: None,
                },
                publication.snapshot.clone(),
                ScopeUniverse::from_selectors(
                    "fixture",
                    vec![],
                    vec!["**".into()],
                    vec![],
                    ScopeCompleteness::Partial,
                    None,
                ),
                digest.to_string(),
                None,
            )
            .unwrap();
            let mut db = SqliteKnowledgeRepository::open(&publication.publication.store).unwrap();
            db.replace_owner(
                owner,
                &KnowledgeStore {
                    artifacts: vec![manifest],
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
            digest
        };
        let obsolete = attach(&first, "obsolete", b"obsolete-native");
        let shared = attach(&first, "shared", b"shared-native");
        publish(dir.path(), 1);
        let last = publish(dir.path(), 2);
        attach(&last, "shared", b"shared-native");
        let inactive = attach(&last, "inactive", b"inactive-native");
        SqliteKnowledgeRepository::open(&last.publication.store)
            .unwrap()
            .remove_owner("inactive")
            .unwrap();
        let plan = plan_retention(dir.path(), Default::default()).unwrap();
        assert_eq!(plan.remove_objects, vec![obsolete.to_string()]);
        // Simulate interruption immediately after committed row retirement.
        write_json(&crate::workspace::storage_directory(dir.path()).unwrap().join("retention-receipt.json"), &serde_json::json!({"schema_version":"entmoot.retention_journal.v1", "state":"applying", "plan":plan})).unwrap();
        let database = crate::KnowledgeViewLocator::read(&last.publication.store)
            .unwrap()
            .unwrap()
            .database;
        crate::knowledge_db::retention::Maintenance::open(&database)
            .unwrap()
            .collect(&plan.remove_views)
            .unwrap();
        let recovery = plan_retention(dir.path(), Default::default()).unwrap();
        assert_eq!(recovery.remove_objects, vec![obsolete.to_string()]);
        apply_retention(dir.path(), &recovery).unwrap();
        assert!(pool.get(&obsolete).is_err());
        assert_eq!(pool.get(&shared).unwrap(), b"shared-native");
        assert_eq!(pool.get(&inactive).unwrap(), b"inactive-native");
        assert!(SqliteKnowledgeRepository::open_view_read_only(
            &database,
            &plan.remove_views.into_iter().next().unwrap()
        )
        .err()
        .unwrap()
        .to_string()
        .contains("expired"));
    }
    #[test]
    fn explicit_store_admission_honors_policy_beside_any_database_or_view() {
        let fixture = tempfile::tempdir().unwrap();
        for name in ["store", "relocated-volume", ".entmoot"] {
            let volume = fixture.path().join(name);
            fs::create_dir(&volume).unwrap();
            let database = volume.join("knowledge.sqlite3");
            fs::write(&database, b"budgeted bytes").unwrap();
            assert!(
                check_store_admission(&database).is_ok(),
                "no policy means no configured limit"
            );
            fs::write(
                volume.join("retention-policy.json"),
                br#"{"keep_recent":2,"max_workspace_bytes":1}"#,
            )
            .unwrap();
            let view = fixture.path().join(format!("{name}.view.json"));
            super::super::write_json(
                &view,
                &crate::KnowledgeViewLocator {
                    database: database.clone(),
                    view: entmoot_contracts::blake3_hex(b"authored-view"),
                },
            )
            .unwrap();
            for store in [&database, &view] {
                let error = check_store_admission(store).unwrap_err();
                assert!(
                    error.to_string().contains("storage admission refused"),
                    "{error}"
                );
            }
            fs::write(
                volume.join("retention-policy.json"),
                br#"{"keep_recent":2,"max_workspace_bytes":1000000}"#,
            )
            .unwrap();
            assert!(check_store_admission(&view).is_ok());
            fs::write(volume.join("retention-policy.json"), b"invalid policy").unwrap();
            assert!(
                check_store_admission(&database).is_err(),
                "malformed policy must not disable admission"
            );
        }
        assert!(check_store_admission(&fixture.path().join("independent.sqlite3")).is_ok());
    }
}
