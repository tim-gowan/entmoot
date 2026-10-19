//! Revision membership, shared definitions, and source content. No binding,
//! claim, source span, or exact body is promoted to a shared definition.
use super::*;
use crate::{SourceReadRequest, SourceStore, TopologyGraph};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS shared_definitions (
    definition_id INTEGER PRIMARY KEY, policy_id TEXT NOT NULL, structural_hash TEXT NOT NULL,
    UNIQUE(policy_id,structural_hash)
);
CREATE TABLE IF NOT EXISTS shared_paths(path_id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS shared_source_files (
    file_id INTEGER PRIMARY KEY, digest TEXT NOT NULL UNIQUE, bytes BLOB
);
CREATE TABLE IF NOT EXISTS inventory_headers (
    view_id TEXT PRIMARY KEY, payload INTEGER NOT NULL REFERENCES shared_payloads,
    graph_digest TEXT NOT NULL, source_checked INTEGER NOT NULL CHECK(source_checked IN(0,1)), inventory_digest TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS inventory_nodes (
    view_id TEXT NOT NULL, ordinal INTEGER NOT NULL, node_id TEXT NOT NULL,
    payload INTEGER NOT NULL REFERENCES shared_payloads, definition_id INTEGER REFERENCES shared_definitions,
    path_id INTEGER REFERENCES shared_paths,
    PRIMARY KEY(view_id,ordinal), UNIQUE(view_id,node_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS inventory_by_definition ON inventory_nodes(definition_id,view_id,node_id);
CREATE INDEX IF NOT EXISTS inventory_by_path ON inventory_nodes(view_id,path_id,ordinal);
CREATE TABLE IF NOT EXISTS inventory_files (
    view_id TEXT NOT NULL, ordinal INTEGER NOT NULL, path_id INTEGER NOT NULL REFERENCES shared_paths,
    file_id INTEGER NOT NULL REFERENCES shared_source_files,
    PRIMARY KEY(view_id,ordinal), UNIQUE(view_id,path_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS inventory_by_content ON inventory_files(file_id,view_id,path_id);
"#;

/// Local storage locator. Portable query payloads never contain this layout.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeViewLocator {
    pub database: PathBuf,
    pub view: String,
}

impl KnowledgeViewLocator {
    pub fn read(path: &Path) -> Result<Option<Self>, KnowledgeRepositoryError> {
        if !path.to_string_lossy().ends_with(".view.json") {
            return Ok(None);
        }
        let mut value: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        entmoot_contracts::ContentDigest::parse(value.view.clone())
            .map_err(|e| invalid_record("view", e))?;
        if value.database.is_relative() {
            value.database = path.parent().unwrap_or(Path::new(".")).join(value.database);
        }
        Ok(Some(value))
    }
}

impl SqliteKnowledgeRepository {
    /// Persist inventory independently of the graph export. The caller commits
    /// knowledge before this step and exposes the locator only after success.
    pub fn store_inventory(
        &mut self,
        graph: &TopologyGraph,
        source: Option<&dyn SourceStore>,
    ) -> Result<(), KnowledgeRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        store_inventory_in_transaction(&transaction, graph, source)?;
        transaction.commit()?;
        Ok(())
    }

    /// Hydrate hot navigation records only. Provider symbol dumps, resolution
    /// payloads and ordered traces stay in the repository until requested.
    pub fn load_inventory(
        &self,
        query_only: bool,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        self.load_inventory_projection(if query_only {
            super::ReadProjection::Query
        } else {
            super::ReadProjection::Complete
        })
    }

    pub fn load_assessment_inventory(
        &self,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        self.load_inventory_projection(super::ReadProjection::Assessment)
    }

    fn load_inventory_projection(
        &self,
        projection: super::ReadProjection,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        let payload: Option<i64> = self.connection.query_row("SELECT payload FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r| r.get(0)).optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let mut graph: TopologyGraph = hydrate_record(&self.connection, payload)?;
        // Preserve the exact membership encoding while releasing syntax that
        // will be reattached from visible source-of-record rows below.
        let mut commitment = blake3::Hasher::new();
        commitment.update(b"[");
        serde_json::to_writer(&mut commitment, "entmoot.inventory.membership.v1")?;
        commitment.update(b",");
        serde_json::to_writer(&mut commitment, &graph.snapshot)?;
        commitment.update(b",");
        serde_json::to_writer(&mut commitment, &graph.policy_ids)?;
        commitment.update(b",[");
        let mut statement = self.connection.prepare_cached("SELECT n.payload,d.structural_hash,p.path FROM inventory_nodes n LEFT JOIN shared_definitions d USING(definition_id) LEFT JOIN shared_paths p USING(path_id) WHERE n.view_id=(SELECT id FROM temp.selected_knowledge_view) ORDER BY ordinal")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (payload, definition, path) = row?;
            let mut node: crate::TopologyNode = hydrate_record(&self.connection, payload)?;
            if let Some(hash) = definition {
                node.structural_hash = Some(hash);
            }
            node.path = path;
            if !graph.nodes.is_empty() {
                commitment.update(b",");
            }
            serde_json::to_writer(&mut commitment, &node)?;
            node.declaration = None;
            node.code_contract = None;
            graph.nodes.push(node);
        }
        let mut statement = self.connection.prepare_cached("SELECT p.path,f.digest FROM inventory_files m JOIN shared_paths p USING(path_id) JOIN shared_source_files f USING(file_id) WHERE m.view_id=(SELECT id FROM temp.selected_knowledge_view) ORDER BY ordinal")?;
        graph.snapshot_files = statement
            .query_map([], |r| {
                Ok(crate::SnapshotFileDigest {
                    path: r.get(0)?,
                    content_digest: r.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let expected: String = self.connection.query_row("SELECT inventory_digest FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r|r.get(0))?;
        commitment.update(b"],");
        serde_json::to_writer(&mut commitment, &graph.snapshot_files)?;
        commitment.update(b"]");
        if format!("blake3:{}", commitment.finalize().to_hex()) != expected {
            return Err(invalid_record(
                "inventory",
                "membership or shared definition commitment mismatch",
            ));
        }
        let knowledge = self.load_mode(projection)?;
        Ok(Some(knowledge.into_inventory(graph)))
    }

    pub fn inventory_export_digest(&self) -> Result<Option<String>, KnowledgeRepositoryError> {
        Ok(self.connection.query_row("SELECT graph_digest FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r|r.get(0)).optional()?)
    }

    pub fn verify_inventory_export(&self, path: &Path) -> Result<(), KnowledgeRepositoryError> {
        if let Some(expected) = self.inventory_export_digest()? {
            let mut file = std::fs::File::open(path)?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            if format!("blake3:{}", hasher.finalize().to_hex()) != expected {
                return Err(invalid_record(
                    "inventory",
                    "graph export does not match selected revision/build view",
                ));
            }
        }
        Ok(())
    }

    pub fn retained_source(
        &self,
        request: SourceReadRequest<'_>,
    ) -> Result<Option<crate::SourceFile>, KnowledgeRepositoryError> {
        let path = crate::source_store::normalize_source_path(request.path)
            .map_err(|e| invalid_record("source", e))?;
        let payload: i64 = self.connection.query_row("SELECT payload FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r| r.get(0))?;
        let header: TopologyGraph = hydrate_record(&self.connection, payload)?;
        if header.snapshot != *request.snapshot {
            return Err(invalid_record(
                "source",
                "snapshot does not match selected view",
            ));
        }
        let row: Option<(String,Option<Vec<u8>>)> = self.connection.query_row("SELECT f.digest,f.bytes FROM inventory_files m JOIN shared_paths p USING(path_id) JOIN shared_source_files f USING(file_id) WHERE m.view_id=(SELECT id FROM temp.selected_knowledge_view) AND p.path=?1", [path.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((digest, bytes)) = row else {
            return Err(invalid_record("source", "path not in selected view"));
        };
        if digest != request.expected_content_digest {
            return Err(invalid_record(
                "source",
                "requested digest does not match membership",
            ));
        }
        let Some(bytes) = bytes else { return Ok(None) };
        if entmoot_contracts::blake3_hex(&bytes) != digest {
            return Err(invalid_record("source", "retained source digest mismatch"));
        }
        Ok(Some(crate::SourceFile {
            path,
            content_digest: digest,
            bytes,
        }))
    }
}

fn intern_path(connection: &Connection, path: &str) -> Result<i64, KnowledgeRepositoryError> {
    connection.execute(
        "INSERT OR IGNORE INTO shared_paths(path) VALUES(?1)",
        [path],
    )?;
    Ok(connection.query_row(
        "SELECT path_id FROM shared_paths WHERE path=?1",
        [path],
        |r| r.get(0),
    )?)
}

fn inventory_digest(graph: &TopologyGraph) -> Result<String, KnowledgeRepositoryError> {
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(
        &mut hasher,
        &(
            "entmoot.inventory.membership.v1",
            &graph.snapshot,
            &graph.policy_ids,
            &graph.nodes,
            &graph.snapshot_files,
        ),
    )?;
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn store_inventory_in_transaction(
    transaction: &Transaction<'_>,
    graph: &TopologyGraph,
    source: Option<&dyn SourceStore>,
) -> Result<(), KnowledgeRepositoryError> {
    let mut digest = blake3::Hasher::new();
    serde_json::to_writer(&mut digest, graph)?;
    digest.update(b"\n");
    let graph_digest = format!("blake3:{}", digest.finalize().to_hex());
    let prior: Option<(String,bool)> = transaction.query_row("SELECT graph_digest,source_checked FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if prior
        .is_some_and(|(digest, checked)| digest == graph_digest && (source.is_none() || checked))
    {
        return Ok(());
    }
    graph
        .validate()
        .map_err(|e| invalid_record("inventory", e))?;
    let mut header = TopologyGraph::new(graph.root.clone());
    header.schema_version = graph.schema_version.clone();
    header.snapshot = graph.snapshot.clone();
    header.policy_ids = graph.policy_ids.clone();
    for table in ["inventory_nodes", "inventory_files", "inventory_headers"] {
        transaction.execute(
            &format!(
                "DELETE FROM {table} WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)"
            ),
            [],
        )?;
    }
    let payload = intern_payload(transaction, &serde_json::to_vec(&header)?)?;
    transaction.execute("INSERT INTO inventory_headers VALUES((SELECT id FROM temp.selected_knowledge_view),?1,?2,?3,?4)", params![payload,graph_digest,source.is_some(),inventory_digest(graph)?])?;
    for (ordinal, node) in graph.nodes.iter().enumerate() {
        let mut value = serde_json::to_value(node)?;
        // Multiple policies without a node->policy witness cannot be
        // coalesced. Preserve the original occurrence instead of guessing.
        let definition = if let ([policy], Some(hash)) =
            (graph.policy_ids.as_slice(), node.structural_hash.as_deref())
        {
            transaction.execute(
                "INSERT OR IGNORE INTO shared_definitions(policy_id,structural_hash) VALUES(?1,?2)",
                params![policy, hash],
            )?;
            let id: i64 = transaction.query_row("SELECT definition_id FROM shared_definitions WHERE policy_id=?1 AND structural_hash=?2", params![policy,hash], |r| r.get(0))?;
            if let Some(object) = value.as_object_mut() {
                object.remove("structural_hash");
            }
            Some(id)
        } else {
            None
        };
        let path = node
            .path
            .as_deref()
            .map(|path| intern_path(transaction, path))
            .transpose()?;
        if let Some(object) = value.as_object_mut() {
            object.remove("path");
        }
        let payload = intern_payload(transaction, &serde_json::to_vec(&value)?)?;
        transaction.execute("INSERT INTO inventory_nodes VALUES((SELECT id FROM temp.selected_knowledge_view),?1,?2,?3,?4,?5)", params![i64::try_from(ordinal).map_err(|e| invalid_record("inventory", e))?,node.id,payload,definition,path])?;
    }
    for (ordinal, file) in graph.snapshot_files.iter().enumerate() {
        let path = intern_path(transaction, &file.path)?;
        let id = intern_source_file(transaction, &graph.snapshot, file, source)?;
        transaction.execute("INSERT INTO inventory_files VALUES((SELECT id FROM temp.selected_knowledge_view),?1,?2,?3)", params![i64::try_from(ordinal).map_err(|e| invalid_record("inventory", e))?,path,id])?;
    }
    Ok(())
}

/// Shared byte identity for inventory and explicit Git replay inputs.
pub(super) fn intern_source_digest(
    transaction: &Transaction<'_>,
    digest: &str,
) -> Result<(i64, bool), KnowledgeRepositoryError> {
    entmoot_contracts::ContentDigest::parse(digest).map_err(|e| invalid_record("source", e))?;
    transaction.execute(
        "INSERT OR IGNORE INTO shared_source_files(digest) VALUES(?1)",
        [digest],
    )?;
    let (id, retained): (i64, bool) = transaction.query_row(
        "SELECT file_id,bytes IS NOT NULL FROM shared_source_files WHERE digest=?1",
        [digest],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((id, retained))
}

/// Callers verify bytes before entering mutation; never replace retained content.
pub(super) fn retain_source_bytes(
    transaction: &Transaction<'_>,
    id: i64,
    bytes: &[u8],
) -> Result<(), KnowledgeRepositoryError> {
    transaction.execute(
        "UPDATE shared_source_files SET bytes=?1 WHERE file_id=?2 AND bytes IS NULL",
        params![bytes, id],
    )?;
    Ok(())
}

fn intern_source_file(
    transaction: &Transaction<'_>,
    snapshot: &entmoot_contracts::SourceSnapshot,
    file: &crate::SnapshotFileDigest,
    source: Option<&dyn SourceStore>,
) -> Result<i64, KnowledgeRepositoryError> {
    let (id, retained) = intern_source_digest(transaction, &file.content_digest)?;
    if !retained {
        if let Some(source) = source {
            let request = SourceReadRequest {
                snapshot,
                path: &file.path,
                expected_content_digest: &file.content_digest,
            };
            if source
                .read_immutable(request)
                .map_err(|e| invalid_record("source", e))?
                .is_none()
            {
                let content = source
                    .read_path(request)
                    .map_err(|e| invalid_record("source", e))?;
                retain_source_bytes(transaction, id, &content.bytes)?;
            }
        }
    }
    Ok(id)
}

/// Import one complete content-addressed result. Caller owns publication visibility.
pub(super) fn import_graph_in_transaction(
    transaction: &Transaction<'_>,
    graph: &TopologyGraph,
    source: Option<&dyn SourceStore>,
) -> Result<(), KnowledgeRepositoryError> {
    graph
        .validate()
        .map_err(|e| invalid_record("inventory", e))?;
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut hasher, graph)?;
    let view = format!("blake3:{}", hasher.finalize().to_hex());
    let selected: String =
        transaction.query_row("SELECT id FROM temp.selected_knowledge_view", [], |r| {
            r.get(0)
        })?;
    if selected != view {
        return Err(invalid_record(
            "inventory",
            "immutable view must equal graph digest",
        ));
    }
    let prior: Option<String> = transaction
        .query_row(
            "SELECT graph_digest FROM inventory_headers WHERE view_id=?1",
            [&selected],
            |r| r.get(0),
        )
        .optional()?;
    hasher.update(b"\n");
    let export = format!("blake3:{}", hasher.finalize().to_hex());
    if let Some(prior) = prior {
        if prior != export {
            return Err(invalid_record(
                "inventory",
                "immutable view already contains another graph",
            ));
        }
        // Adding digest-verified source bytes does not mutate a sealed view's
        // identities, records or visibility. Imports may have omitted these bytes.
        if source.is_some() {
            for file in &graph.snapshot_files {
                intern_source_file(transaction, &graph.snapshot, file, source)?;
            }
        }
        return Ok(());
    }
    let staged = super::buffered_stage::StagedBatches::new(transaction)?;
    for batch in KnowledgeStoreBatches::from_graph(graph, PublicationBuffer::default())? {
        let batch = batch?;
        let mut rows = BTreeMap::new();
        for_each_store_row(&batch.store, |row| {
            rows.insert(
                entmoot_contracts::KnowledgeRecordRef::new(row.collection, row.record_id.clone()),
                row,
            );
            Ok(())
        })?;
        staged.push(&batch.store, &rows)?;
    }
    staged.finish()?;
    staged.visit(|store| validate_store_records_in_context(&store, &staged))?;
    // New view: both knowledge and inventory become durable in this transaction.
    // Row order is a derivation detail fenced by the implementation fingerprint.
    let mut digest = store_digest_hasher();
    staged.visit(|store| {
        insert_store_chunk(transaction, &store, &mut digest)?;
        Ok(())
    })?;
    set_store_digest(
        transaction,
        &format!("blake3:{}", digest.finalize().to_hex()),
    )?;
    store_inventory_in_transaction(transaction, graph, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_inventory_preserves_all_family_occurrences_and_source_revisions() {
        let source = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        let fixtures = [
            ("a.rs", "fn work() -> i32 { 1 }\n", "// comment\n"),
            ("a.py", "def work():\n    return 1\n", "# comment\n"),
            ("a.js", "function work() { return 1; }\n", "// comment\n"),
            (
                "a.ts",
                "function work(): number { return 1; }\n",
                "// comment\n",
            ),
            (
                "a.tsx",
                "function Work() { return <div/>; }\n",
                "// comment\n",
            ),
            (
                "a.go",
                "package a\nfunc Work() int { return 1 }\n",
                "// comment\n",
            ),
            (
                "A.java",
                "class A { int work() { return 1; } }\n",
                "// comment\n",
            ),
            (
                "A.cs",
                "class A { int Work() { return 1; } }\n",
                "// comment\n",
            ),
            ("a.c", "int work(void) { return 1; }\n", "// comment\n"),
            ("a.cpp", "int work() { return 1; }\n", "// comment\n"),
        ];
        for (path, bytes, _) in fixtures {
            std::fs::write(source.path().join(path), bytes).unwrap();
        }
        let graph = crate::build_topology_from_repo_with_options(
            source.path(),
            crate::TopologyBuildOptions::default(),
        )
        .unwrap();
        let source_store = crate::GixSourceStore::open(source.path()).unwrap();
        let db = database.path().join("shared.sqlite3");
        let mut a =
            SqliteKnowledgeRepository::open_view(&db, &entmoot_contracts::blake3_hex(b"a/build-a"))
                .unwrap();
        a.replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        a.store_inventory(&graph, Some(&source_store)).unwrap();
        let changes = a.connection.total_changes();
        a.store_inventory(&graph, Some(&source_store)).unwrap();
        assert_eq!(
            changes,
            a.connection.total_changes(),
            "unchanged inventory must not rewrite shared rows"
        );
        let count: i64 = a
            .connection
            .query_row("SELECT COUNT(*) FROM shared_definitions", [], |r| r.get(0))
            .unwrap();
        assert!(count > 0);
        let roundtrip = a.load_inventory(false).unwrap().unwrap();
        let expected = serde_json::to_value(&graph).unwrap();
        let observed = serde_json::to_value(&roundtrip).unwrap();
        for (key, value) in expected.as_object().unwrap() {
            let normalize = |value: &serde_json::Value| {
                if let Some(array) = value.as_array() {
                    let mut rows: Vec<_> = array
                        .iter()
                        .map(|v| serde_json::to_string(v).unwrap())
                        .collect();
                    rows.sort();
                    serde_json::to_value(rows).unwrap()
                } else {
                    value.clone()
                }
            };
            assert_eq!(
                normalize(value),
                normalize(&observed[key]),
                "collection {key}"
            );
        }
        for (path, bytes, comment) in fixtures {
            std::fs::write(source.path().join(path), format!("{comment}{bytes}")).unwrap();
        }
        let newer = crate::build_topology_from_repo_with_options(
            source.path(),
            crate::TopologyBuildOptions::default(),
        )
        .unwrap();
        let mut b =
            SqliteKnowledgeRepository::open_view(&db, &entmoot_contracts::blake3_hex(b"b/build-b"))
                .unwrap();
        b.replace_all(&KnowledgeStore::extract_from_graph(&newer))
            .unwrap();
        b.store_inventory(&newer, Some(&source_store)).unwrap();
        let next_count: i64 = b
            .connection
            .query_row("SELECT COUNT(*) FROM shared_definitions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, next_count,
            "ordinary comments do not create definitions"
        );
        for file in &graph.snapshot_files {
            let request = SourceReadRequest {
                snapshot: &graph.snapshot,
                path: &file.path,
                expected_content_digest: &file.content_digest,
            };
            assert!(a.retained_source(request).unwrap().is_some());
            assert!(
                b.retained_source(request).is_err(),
                "cross-revision source must be refused"
            );
        }
        let old = a.load_inventory(true).unwrap().unwrap();
        assert_eq!(old.snapshot, graph.snapshot);
        assert_ne!(old.snapshot, newer.snapshot);
        assert_eq!(
            old.nodes
                .iter()
                .find(|n| n.kind == crate::NodeKind::Callable)
                .unwrap()
                .start_line,
            graph
                .nodes
                .iter()
                .find(|n| n.kind == crate::NodeKind::Callable)
                .unwrap()
                .start_line
        );
        a.connection.execute("UPDATE shared_definitions SET structural_hash=?1 WHERE definition_id=(SELECT MIN(definition_id) FROM shared_definitions)", [entmoot_contracts::blake3_hex(b"corrupt definition pointer")]).unwrap();
        assert!(a
            .load_inventory(true)
            .unwrap_err()
            .to_string()
            .contains("commitment mismatch"));
    }
}
