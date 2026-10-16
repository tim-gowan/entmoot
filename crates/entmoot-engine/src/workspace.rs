//! Local, explicit operator publication lifecycle. Reads only discover paths.
//!
//! Paths and execution knobs belong to this local composition boundary, not to
//! portable evidence contracts. Each successful write publishes a new generation;
//! the previous current pointer survives failed indexing, production or attachment.
pub mod impact;
pub mod location;
pub use location::{
    canonicalize_override_path, configure_data_directory, data_root, default_entmoot_home,
    ensure_global_config, ensure_workspace_config, entmoot_override_configured,
    host_home_directory, prepare_discovered_workspace, prepare_workspace, resolve_entmoot_override,
    storage_directory, storage_directory_in, workspace_namespace,
};
pub mod open;
pub mod retention;

pub use open::{
    open_query_bundle, open_query_bundle_paths, DeferredQueryBundle, OpenedQueryBundle,
    PublicationOpenSpec,
};

use crate::{
    native_resolver::*, AtomicGraphWrite, KnowledgeRepository, KnowledgeStore,
    KnowledgeStoreBatches, PublicationBuffer, SqliteKnowledgeRepository, TopologyGraph,
};
use entmoot_contracts::{blake3_hex, SourceSnapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

fn failure(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::Invalid(error.to_string())
}

/// Machine-local paths; never a portable operation payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationLocation {
    pub graph: PathBuf,
    pub store: PathBuf,
    pub source_root: PathBuf,
}

/// Atomic local pointer. A bind of the same snapshot preserves the compare base.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePublication {
    pub schema_version: String,
    pub current: PublicationLocation,
    pub base: Option<PublicationLocation>,
    pub snapshot: SourceSnapshot,
    pub summary: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindStage {
    pub family: String,
    pub languages: Vec<String>,
    /// planned | produced | reused | skipped | failed.
    pub stage: String,
    pub input_key: String,
    pub reuse_policy: String,
    pub adapter_path: Option<PathBuf>,
    pub producer_digest: String,
    pub manifest: Option<PathBuf>,
    pub native_receipt: Option<NativeReceipt>,
    pub attachment: Option<crate::ArtifactIngestStats>,
    pub work: Option<entmoot_contracts::AnalysisWorkItem>,
    pub events: Vec<String>,
    pub limitations: Vec<String>,
}

/// Local execution receipt, including exact native and attachment receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSummary {
    #[serde(default)]
    pub indexed_files: usize,
    #[serde(default)]
    pub reused_syntax_files: usize,
    #[serde(default)]
    pub reused_extraction_files: usize,
    #[serde(default)]
    pub reused_stitching_files: usize,
    pub schema_version: String,
    pub publication: PublicationLocation,
    pub snapshot: SourceSnapshot,
    pub policy_ids: Vec<String>,
    pub execution_policy: ExecutionPolicy,
    pub full: bool,
    pub require_native: bool,
    pub plan_only: bool,
    pub structural_only: bool,
    pub stages: Vec<BindStage>,
    pub coverage: crate::CoverageReport,
    pub wall_seconds: f64,
    pub budget_seconds: u64,
    pub limitations: Vec<String>,
    /// Workspace adapter profile refreshed during init/bind discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_profile: Option<entmoot_contracts::AdapterProfile>,
    /// unchanged | replaced (empty on plan-only).
    #[serde(default)]
    pub publication_outcome: String,
    #[serde(default)]
    pub publication_digest: String,
}

#[derive(Debug, Clone, Default)]
pub struct BindOptions {
    pub canon: bool,
    /// Declared publication subject written onto the snapshot (PURL or repository).
    pub subject: Option<entmoot_contracts::SourceSubject>,
    /// Additional subject aliases recorded on the snapshot.
    pub subject_aliases: Vec<entmoot_contracts::SourceSubject>,
    pub rust_profile: entmoot_contracts::RustBuildProfile,
    pub full: bool,
    pub require_native: bool,
    pub plan_only: bool,
    pub structural_only: bool,
    pub budget: ProducerBudget,
}

/// Resolve Git top-level, falling back to the current directory for non-Git input.
pub fn workspace_root(start: &Path) -> Result<PathBuf, WorkspaceError> {
    let start = fs::canonicalize(start)?;
    Ok(gix::discover(&start)
        .ok()
        .and_then(|repo| repo.workdir().map(Path::to_path_buf))
        .unwrap_or(start))
}

/// Read-only discovery; never creates directories or invokes producers.
pub fn discover_publication(start: &Path) -> Result<WorkspacePublication, WorkspaceError> {
    let start = fs::canonicalize(start)?;
    for ancestor in start.ancestors() {
        let pointer = storage_directory(ancestor)?.join("current.json");
        if pointer.is_file() {
            return read_publication(&pointer);
        }
    }
    Err(WorkspaceError::Invalid(
        "no current workspace publication; run entmoot init or pass --graph/--publication".into(),
    ))
}

pub fn read_publication(path: &Path) -> Result<WorkspacePublication, WorkspaceError> {
    let publication: WorkspacePublication = serde_json::from_slice(&fs::read(path)?)?;
    if publication.schema_version != "entmoot.workspace_publication.v1" {
        return Err(WorkspaceError::Invalid(
            "unsupported workspace publication schema".into(),
        ));
    }
    // Reading a pointer must not initialize an empty store.
    if !publication.current.graph.is_file() || !publication.current.store.is_file() {
        return Err(WorkspaceError::Invalid(
            "publication graph/store missing; run entmoot init".into(),
        ));
    }
    Ok(publication)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), WorkspaceError> {
    let mut write = AtomicGraphWrite::begin(path).map_err(failure)?;
    write.write_json(value).map_err(failure)?;
    write.commit().map_err(failure)
}

fn sync_graph(
    graph: &TopologyGraph,
    location: &PublicationLocation,
) -> Result<crate::BufferedPublicationReceipt, WorkspaceError> {
    let mut store = SqliteKnowledgeRepository::open(&location.store).map_err(failure)?;
    let buffer = PublicationBuffer::default();
    let mut batches = KnowledgeStoreBatches::new(KnowledgeStore::extract_from_graph(graph), buffer)
        .map_err(failure)?;
    let receipt = store
        .replace_owner_buffered(
            &format!("snapshot:{}", graph.snapshot.repository_id),
            &mut batches,
            buffer,
        )
        .map_err(failure)?;
    // Rematerialize the disposable graph projection only when membership changed
    // or the projection is missing (PLANNING publication working-set ceiling).
    if !receipt.outcome.is_unchanged() || !location.graph.is_file() {
        crate::write_graph_json(graph, &location.graph).map_err(failure)?;
    }
    Ok(receipt)
}

// The OS releases this lock on process exit, including abnormal termination.
// Keep the sidecar: unlinking it could give concurrent writers different inodes.
struct WriteGuard {
    _file: fs::File,
}
impl WriteGuard {
    fn acquire(root: &Path) -> Result<Self, WorkspaceError> {
        let directory = storage_directory(root)?;
        fs::create_dir_all(&directory)?;
        let path = directory.join("write.lock");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.try_lock().map_err(|error| {
            failure(format!(
                "publication write lock {}: {error}; wait for the active writer",
                path.display()
            ))
        })?;
        file.set_len(0)?;
        use std::io::Write;
        write!(file, "{}", std::process::id())?;
        Ok(Self { _file: file })
    }
}

fn ignore_publications(root: &Path) -> Result<(), WorkspaceError> {
    let directory = storage_directory(root)?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(".gitignore");
    if !path.exists() {
        fs::write(path, "*\n")?;
    }
    Ok(())
}

fn languages(graph: &TopologyGraph) -> BTreeSet<String> {
    let hasher = entmoot_core::StructuralHasher::new();
    graph
        .snapshot_files
        .iter()
        .filter_map(|file| {
            let ext = Path::new(&file.path).extension()?.to_str()?;
            hasher.language_for_extension(&format!(".{ext}"))
        })
        .collect()
}

fn family(language: &str) -> &str {
    match language {
        "javascript" | "typescript" | "tsx" | "jsx" => {
            if typescript::explicitly_selected() {
                "typescript"
            } else {
                "oxc"
            }
        }
        "c_sharp" | "csharp" => "roslyn",
        "java" => "java",
        "c" | "cpp" => "clang",
        other => other,
    }
}

fn adapter_path(family: &str) -> Option<PathBuf> {
    match family {
        "python" => python::configured_binary(),
        "typescript" => typescript::configured_binary(),
        "go" => go::configured_binary(),
        "roslyn" => roslyn::configured_binary(),
        "java" => java::configured_binary(),
        "clang" => clang::configured_binary(),
        _ => None,
    }
}

fn available(family: &str) -> bool {
    match family {
        "python" => python::configured_binary().is_some(),
        "oxc" => cfg!(feature = "resolved-oxc"),
        "typescript" => typescript::configured_binary().is_some(),
        "rust" => cfg!(feature = "resolved-rust-analyzer"),
        "go" => go::configured_binary().is_some(),
        "roslyn" => roslyn::configured_binary().is_some(),
        "java" => java::configured_binary().is_some(),
        "clang" => clang::configured_binary().is_some(),
        _ => false,
    }
}

fn produce(
    family: &str,
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let selected =
        serde_json::from_value(serde_json::Value::String(family.into())).map_err(|_| {
            NativeProducerError::Message(format!(
                "invalid_environment: no shipping {family} producer"
            ))
        })?;
    crate::native_resolver::pipeline::produce_family(selected, request)
}

fn previous_summary(
    previous: Option<&WorkspacePublication>,
) -> Result<Option<WorkspaceSummary>, WorkspaceError> {
    previous
        .map(|p| {
            let summary: WorkspaceSummary = serde_json::from_slice(&fs::read(&p.summary)?)?;
            for receipt in summary
                .stages
                .iter()
                .filter_map(|stage| stage.native_receipt.as_ref())
            {
                receipt
                    .validate(&crate::implementation_fingerprint())
                    .map_err(failure)?;
            }
            Ok(summary)
        })
        .transpose()
}

/// Same-snapshot reuse is qualified only for local lexical Oxc evidence.
/// Package routes also depend on ignored node_modules and negative file lookups.
fn reusable_lexical_oxc(
    root: &Path,
    manifest_path: &Path,
    runtime: &str,
) -> Result<bool, WorkspaceError> {
    if root.join("package.json").exists() {
        return Ok(false);
    }
    let manifest: entmoot_contracts::ArtifactManifest =
        serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest.artifact_path.as_deref() != Some("resolver.json") {
        return Err(failure("unexpected cached resolver artifact path"));
    }
    let parent = manifest_path
        .parent()
        .ok_or_else(|| failure("cached manifest has no parent"))?;
    let bytes = fs::read(parent.join("resolver.json"))?;
    if blake3_hex(&bytes) != manifest.content_digest {
        return Err(failure("cached resolver artifact digest mismatch"));
    }
    let artifact =
        crate::artifact::resolver::parse_scoped_artifact(&bytes, &manifest).map_err(failure)?;
    let raw_bytes = fs::read(parent.join("native.raw.json"))?;
    if blake3_hex(&raw_bytes) != artifact.build_context_digest.as_str() {
        return Err(failure("cached native input capture digest mismatch"));
    }
    let raw: serde_json::Value = serde_json::from_slice(&raw_bytes)?;
    let _: entmoot_contracts::NativeRawSeries =
        serde_json::from_value(raw["schema_version"].clone())?;
    let implementation: entmoot_contracts::ImplementationFingerprint =
        serde_json::from_value(raw["implementation"].clone())?;
    if implementation != crate::implementation_fingerprint() {
        return Err(failure(
            "cached native capture implementation fingerprint mismatch",
        ));
    }
    if raw["build_context"]["javascript_runtime"].as_str() != Some(runtime) {
        return Ok(false);
    }
    let Some(calls) = raw["calls"].as_array() else {
        return Ok(false);
    };
    Ok(!calls.iter().any(|call| {
        call["targets"].as_array().is_some_and(|targets| {
            targets
                .iter()
                .any(|target| target["module_path"].is_string())
        })
    }))
}

fn bind_graph(
    graph: &mut TopologyGraph,
    location: &PublicationLocation,
    objects: &Path,
    options: &BindOptions,
    previous: Option<&WorkspaceSummary>,
    decisions: Option<&[impact::BindDecision]>,
) -> Result<Vec<BindStage>, WorkspaceError> {
    use crate::AnalysisWorkQueue;
    let languages = languages(graph);
    let families: BTreeSet<_> = languages.iter().map(|lang| family(lang)).collect();
    let executable = blake3_hex(&fs::read(std::env::current_exe()?)?);
    let mut planned = Vec::new();
    let mut jobs = Vec::new();
    for family in families {
        let runtime = (family == "oxc").then(oxc::configured_runtime);
        let input_key = blake3_hex(&serde_json::to_vec(&(
            "entmoot.bind.inputs.v1",
            &graph.snapshot,
            &graph.snapshot_files,
            &graph.policy_ids,
            family,
            &executable,
            ExecutionPolicy::Safe,
            (family == "rust").then_some(&options.rust_profile),
            &runtime,
        ))?);
        let mut stage = BindStage {
            family: family.into(),
            languages: languages
                .iter()
                .filter(|lang| self::family(lang) == family)
                .cloned()
                .collect(),
            stage: "planned".into(),
            input_key,
            reuse_policy: if family == "oxc" {
                "entmoot.oxc.same_snapshot_local_lexical.v3"
            } else {
                "unqualified_external_closure"
            }
            .into(),
            adapter_path: adapter_path(family),
            producer_digest: executable.clone(),
            manifest: None,
            native_receipt: None,
            attachment: None,
            work: None,
            events: vec!["planned".into()],
            limitations: Vec::new(),
        };
        let decision = decisions.and_then(|rows| rows.iter().find(|row| row.family == family));
        tracing::info!(family, stage = "starting", "bind family");
        if decisions.is_some()
            && !decision.is_some_and(|row| {
                row.decision.action == entmoot_contracts::AnalysisAction::RequestArtifact
            })
        {
            stage.stage = "skipped".into();
            stage.limitations.push(
                "impact_plan_did_not_request_fresh_artifact; missing evidence remains unknown"
                    .into(),
            );
        } else if options.structural_only || !available(family) {
            stage.stage = "skipped".into();
            stage.limitations.push(if options.structural_only {
                "structural_only_requested; run entmoot bind for available resolver evidence".into()
            } else if family == "python" {
                python::unavailable_message().into()
            } else {
                format!("invalid_environment: {family} producer unavailable; use a producer-enabled build or attach an external artifact")
            });
        } else if !options.full && family == "oxc" {
            if let Some(old) = previous.and_then(|p| {
                p.stages.iter().find(|row| {
                    row.family == family
                        && row.input_key == stage.input_key
                        && row.native_receipt.as_ref().is_some_and(|r| {
                            r.validate(&crate::implementation_fingerprint()).is_ok()
                                && !matches!(
                                    r.semantic_status.as_str(),
                                    "failed" | "invalid_environment" | "budget_exceeded"
                                )
                        })
                        && row.manifest.is_some()
                })
            }) {
                // Original manifest and bytes go through normal validation/attachment.
                if let Some(manifest) = &old.manifest {
                    if reusable_lexical_oxc(
                        &location.source_root,
                        manifest,
                        runtime.as_deref().unwrap_or("unspecified"),
                    )? {
                        stage.manifest = old.manifest.clone();
                        stage.native_receipt = old.native_receipt.clone();
                        stage.stage = "reused".into();
                    }
                }
            }
        }
        let mut queue = None;
        if stage.stage == "planned" && !options.plan_only {
            let parent = location
                .graph
                .parent()
                .ok_or_else(|| failure("graph requires a parent directory"))?;
            if let Some(decision) = decision {
                let mut work_queue =
                    crate::SqliteAnalysisWorkQueue::open(&parent.join("work.sqlite3"))
                        .map_err(failure)?;
                let enqueued = work_queue
                    .enqueue(decision.decision.clone())
                    .map_err(failure)?;
                stage.work = Some(
                    work_queue
                        .claim(enqueued.work_id.as_str(), impact::policy_producer())
                        .map_err(failure)?,
                );
                stage.events.push("fenced".into());
                queue = Some(work_queue);
            }
            jobs.push((
                family,
                NativeProduceRequest {
                    canon: options.canon,
                    graph_path: location.graph.clone(),
                    source_root: location.source_root.clone(),
                    output_dir: parent.join("bind").join(family),
                    budget: options.budget,
                    execution_policy: ExecutionPolicy::Safe,
                    rust_profile: options.rust_profile.clone(),
                },
            ));
        }
        planned.push((stage, queue));
    }
    // Producers only read the frozen graph and write distinct artifact directories.
    // Queue transitions, attachment and publication remain ordered on this thread.
    let produced = crate::native_resolver::parallel::ordered_map(&jobs, |(family, request)| {
        (*family, produce(family, request.clone()))
    });
    let mut produced: std::collections::BTreeMap<_, _> = produced.into_iter().collect();
    let mut stages = Vec::new();
    let mut canon_facts = Vec::new();
    let mut canon_context = Vec::new();
    for (mut stage, mut queue) in planned {
        let family = stage.family.clone();
        let family = family.as_str();
        let decision = decisions.and_then(|rows| rows.iter().find(|row| row.family == family));
        if let Some(result) = produced.remove(family) {
            match result {
                Ok(result) => {
                    if options.canon {
                        canon_context.push(entmoot_contracts::CanonNativeContext {
                            producer: result.receipt.producer.clone(),
                            semantic_status: result.receipt.semantic_status,
                            limitations: result.receipt.limitations.clone(),
                            compiler_version: result.raw.compiler_version.clone(),
                            environment_digest: entmoot_contracts::ContentDigest::from_trusted(
                                blake3_hex(&serde_json::to_vec(&(
                                    &result.raw.build_context,
                                    &result.raw.tool_environment,
                                ))?),
                            ),
                        });
                        canon_facts.extend(result.raw.expressions);
                    }
                    stage.stage = "produced".into();
                    stage.events.push("produced".into());
                    if let (Some(queue), Some(work)) = (&mut queue, &stage.work) {
                        let manifest = serde_json::from_slice(&fs::read(&result.manifest_path)?)?;
                        let verified = crate::ManifestUnverified::new(manifest)
                            .verify(&fs::read(&result.artifact_path)?, &graph.snapshot)
                            .map_err(failure)?;
                        let token = work
                            .fencing_token
                            .as_ref()
                            .ok_or_else(|| failure("claimed work has no fence"))?;
                        stage.work = Some(
                            queue
                                .artifact_ready(work.work_id.as_str(), token.as_str(), &verified)
                                .map_err(failure)?,
                        );
                        stage.events.push("artifact_ready".into());
                    }
                    stage.manifest = Some(result.manifest_path);
                    stage.native_receipt = Some(result.receipt);
                }
                Err(error) => {
                    stage.stage = "failed".into();
                    if let (Some(queue), Some(work)) = (&mut queue, &stage.work) {
                        let token = work
                            .fencing_token
                            .as_ref()
                            .ok_or_else(|| failure("claimed work has no fence"))?;
                        stage.work = Some(
                            queue
                                .fail(work.work_id.as_str(), token.as_str(), error.to_string())
                                .map_err(failure)?,
                        );
                    }
                    stage.limitations.push(error.to_string());
                }
            }
        }
        if !options.plan_only {
            if let Some(manifest) = &stage.manifest {
                stage.attachment =
                    Some(crate::ingest_artifact_manifest(graph, manifest, true).map_err(failure)?);
                stage.events.push("attached".into());
                crate::FilesystemArtifactObjectRepository::new(objects)
                    .archive_manifest(manifest)
                    .map_err(failure)?;
            }
        }
        if options.require_native
            && !options.plan_only
            && (decisions.is_none()
                || decision.is_some_and(|row| {
                    row.decision.action == entmoot_contracts::AnalysisAction::RequestArtifact
                }))
            && (matches!(stage.stage.as_str(), "skipped" | "failed")
                || stage.native_receipt.as_ref().is_some_and(|r| {
                    matches!(
                        r.semantic_status.as_str(),
                        "failed" | "invalid_environment" | "budget_exceeded"
                    )
                }))
        {
            let category = stage
                .native_receipt
                .as_ref()
                .map(|receipt| receipt.semantic_status.as_str())
                .filter(|status| matches!(*status, "budget_exceeded" | "invalid_environment"))
                .unwrap_or("invalid_environment");
            let mut detail = stage.limitations.clone();
            if let Some(receipt) = stage.native_receipt.as_ref() {
                for limitation in &receipt.limitations {
                    if !detail.iter().any(|row| row == limitation) {
                        detail.push(limitation.clone());
                    }
                }
            }
            return Err(failure(format!(
                "{category}: required {} binding unavailable: {:?}",
                family, detail
            )));
        }
        if family != "oxc" && !options.structural_only {
            stage.limitations.push("fresh_production_required: external/build input closure is not qualified for automatic reuse".into());
        }
        let semantic_status = stage
            .native_receipt
            .as_ref()
            .map(|r| r.semantic_status.as_str())
            .unwrap_or("-");
        let wall_seconds = stage.native_receipt.as_ref().map(|r| r.wall_seconds);
        let first_limitation = stage
            .native_receipt
            .as_ref()
            .and_then(|r| r.limitations.first())
            .or_else(|| stage.limitations.first())
            .map(String::as_str);
        let bad_semantics = matches!(
            semantic_status,
            "failed" | "invalid_environment" | "budget_exceeded"
        );
        match stage.stage.as_str() {
            "skipped" if options.structural_only => {
                tracing::debug!(
                    family,
                    "structural inventory selected; native evidence not requested"
                );
            }
            "skipped" | "failed" => {
                tracing::warn!(
                    family,
                    stage = stage.stage.as_str(),
                    semantic_status,
                    wall_seconds,
                    limitation = first_limitation,
                    limitations = ?stage.limitations,
                    "bind family skipped or failed; fix environment or attach an external artifact, then re-run entmoot bind"
                );
            }
            other if bad_semantics => {
                tracing::warn!(
                    family,
                    stage = other,
                    semantic_status,
                    wall_seconds,
                    limitation = first_limitation,
                    events = ?stage.events,
                    "bind family produced with incomplete semantic_status; fix adapter profile / adapters (bind requires native by default; use --structural-only to waive)"
                );
            }
            other => {
                tracing::info!(
                    family,
                    stage = other,
                    semantic_status,
                    wall_seconds,
                    limitation = first_limitation,
                    events = ?stage.events,
                    "bind family"
                );
            }
        }
        stages.push(stage);
    }
    if options.canon {
        let mut events = Vec::new();
        if !options.plan_only {
            let repository = previous
                .map(|p| SqliteKnowledgeRepository::open_read_snapshot(&p.publication.store))
                .transpose()
                .map_err(failure)?;
            let result = crate::canon::produce(
                graph,
                &location.source_root,
                &canon_facts,
                &canon_context,
                repository.as_ref().map(|r| r as &dyn KnowledgeRepository),
            )
            .map_err(failure)?;
            events.push(format!("parsed_files:{}", result.parsed_files));
            events.push(format!("reused_files:{}", result.reused_files));
        }
        stages.push(BindStage {
            family: "canon".into(),
            languages: languages.iter().cloned().collect(),
            stage: if options.plan_only {
                "planned"
            } else {
                "produced"
            }
            .into(),
            input_key: blake3_hex(&serde_json::to_vec(&(&graph.snapshot, &canon_context))?),
            reuse_policy: "pure_fold_inputs_only_native_evidence_is_fresh".into(),
            adapter_path: None,
            producer_digest: executable,
            manifest: None,
            native_receipt: None,
            attachment: None,
            work: None,
            events,
            limitations: vec!["model_relative_candidates_not_proofs".into()],
        });
    }
    Ok(stages)
}

/// Publish a source snapshot for explicit init or first-use frontend preparation.
/// Query-service reads do not call this; frontends may bootstrap missing inventory.
pub fn initialize_workspace(
    root: &Path,
    repository: Option<&str>,
    policy: &entmoot_contracts::StructuralHashPolicy,
    options: &BindOptions,
) -> Result<WorkspaceSummary, WorkspaceError> {
    let root = fs::canonicalize(root)?;
    let _guard = if options.plan_only {
        None
    } else {
        Some(WriteGuard::acquire(&root)?)
    };
    if !options.plan_only {
        ignore_publications(&root)?;
    }
    let pointer = storage_directory(&root)?.join("current.json");
    let previous = if pointer.exists() {
        Some(read_publication(&pointer)?)
    } else {
        None
    };
    let build_options = crate::TopologyBuildOptions {
        policy: Some(policy),
        repository,
    };
    if let Some(previous) = &previous {
        SqliteKnowledgeRepository::open_read_snapshot(&previous.current.store)
            .map_err(failure)?
            .verify_inventory_export(&previous.current.graph)
            .map_err(failure)?;
    }
    let (graph, cache) = crate::build_topology_with_syntax_cache(
        &root,
        build_options,
        previous.as_ref().map(|p| p.current.store.as_path()),
    )
    .map_err(failure)?;
    publish(root, graph, previous, options, None, Some(&cache))
}

/// Refresh the current snapshot. Source/policy changes require a new init/index.
pub fn bind_workspace(
    root: &Path,
    options: &BindOptions,
) -> Result<WorkspaceSummary, WorkspaceError> {
    let location = discover_publication(root)?.current;
    let _guard = if options.plan_only {
        None
    } else {
        Some(WriteGuard::acquire(&location.source_root)?)
    };
    let previous = discover_publication(root)?;
    let source = previous.current.source_root.clone();
    let graph = crate::load_graph_json(&previous.current.graph).map_err(failure)?;
    verify_sources(&graph, &source)?;
    publish(source, graph, Some(previous), options, None, None)
}

/// Adopt an explicitly indexed graph/store into the local default publication,
/// then bind it. The source snapshot is verified; structural identity is preserved.
pub fn bind_publication(
    graph_path: &Path,
    source_root: Option<&Path>,
    store_path: Option<&Path>,
    options: &BindOptions,
) -> Result<WorkspaceSummary, WorkspaceError> {
    let graph = crate::load_graph_json(graph_path).map_err(failure)?;
    let root = fs::canonicalize(source_root.unwrap_or(Path::new(&graph.root)))?;
    let _guard = if options.plan_only {
        None
    } else {
        Some(WriteGuard::acquire(&root)?)
    };
    let store_path = store_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(format!("{}.knowledge.sqlite3", graph_path.display())));
    let repository = SqliteKnowledgeRepository::open_read_only(&store_path).map_err(failure)?;
    let mut graph = repository
        .load()
        .map_err(failure)?
        .apply_to_inventory(crate::inventory_skeleton(&graph));
    graph.validate().map_err(failure)?;
    verify_sources(&graph, &root)?;
    // Physical source location is local wiring, never a structural identity.
    graph.root = root.to_string_lossy().into_owned();
    if !options.plan_only {
        // An imported snapshot cannot tolerate changes to its root .gitignore.
        // A directory-local ignore file hides every generated publication instead.
        fs::write(storage_directory(&root)?.join(".gitignore"), "*\n")?;
        let source_objects =
            crate::FilesystemArtifactObjectRepository::new(artifact_store_path(&store_path)?);
        let destination_objects = crate::FilesystemArtifactObjectRepository::new(
            storage_directory(&root)?.join("knowledge.sqlite3.objects"),
        );
        for artifact in &graph.artifacts {
            let digest = entmoot_contracts::ContentDigest::parse(artifact.content_digest.clone())
                .map_err(failure)?;
            match source_objects.get(&digest) {
                Ok(bytes) => {
                    destination_objects.put(&digest, &bytes).map_err(failure)?;
                }
                Err(crate::ArtifactStoreError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(failure(error)),
            }
        }
    }
    let pointer = storage_directory(&root)?.join("current.json");
    let previous = if pointer.is_file() {
        Some(read_publication(&pointer)?)
    } else {
        None
    };
    publish(root, graph, previous, options, None, None)
}

/// Compare a published baseline with a freshly indexed, digest-fenced working tree.
/// This operation does not publish, attach evidence, or execute a producer.
pub fn compare_working_tree_impact(
    base: &TopologyGraph,
    root: &Path,
    policy: &entmoot_contracts::StructuralHashPolicy,
) -> Result<entmoot_contracts::query::RevisionImpactManifest, WorkspaceError> {
    let head = working_tree_graph(base, root, policy)?;
    crate::compare_revision_impact(base, &head).map_err(failure)
}

/// Read-only, source-verified working-tree inventory for revision assessments.
pub fn working_tree_graph(
    base: &TopologyGraph,
    root: &Path,
    policy: &entmoot_contracts::StructuralHashPolicy,
) -> Result<TopologyGraph, WorkspaceError> {
    let policy_id = policy.fingerprint().to_string();
    if !base
        .policy_ids
        .iter()
        .any(|candidate| candidate == &policy_id)
    {
        return Err(failure(
            "working-tree policy does not match the current publication; pass the publication policy or compare an explicit graph pair",
        ));
    }
    let head = crate::build_topology_from_repo_reusing_base(
        root,
        crate::TopologyBuildOptions {
            policy: Some(policy),
            repository: Some(&base.snapshot.repository_id),
        },
        base,
    )
    .map_err(failure)?;
    verify_sources(&head, root)?;
    Ok(head)
}

/// Verify that a graph still names the complete bytes currently visible at `root`.
///
/// Working-tree impact uses this after indexing so a concurrent edit cannot turn
/// a mixed scan into an apparently stable snapshot.
///
/// Declared `subject` / `subject_aliases` are publication metadata stamped after
/// observation; the fence compares materialization identity and file digests only.
pub fn verify_sources(graph: &TopologyGraph, root: &Path) -> Result<(), WorkspaceError> {
    let observed = observe_source_inventory(root, &graph.snapshot.repository_id)?;
    if !materialization_eq(&observed.snapshot, &graph.snapshot)
        || observed.snapshot_files != graph.snapshot_files
    {
        return Err(failure(
            "source snapshot changed; run entmoot init (or index) before bind",
        ));
    }
    Ok(())
}

fn materialization_eq(
    left: &entmoot_contracts::SourceSnapshot,
    right: &entmoot_contracts::SourceSnapshot,
) -> bool {
    left.repository_id == right.repository_id
        && left.revision == right.revision
        && left.source_state == right.source_state
        && left.snapshot_digest == right.snapshot_digest
        && left.estate_id == right.estate_id
}

/// Digest-only source walk shared by producer submission and final publication fences.
pub(crate) fn observe_source_inventory(
    root: &Path,
    repository: &str,
) -> Result<TopologyGraph, WorkspaceError> {
    use crate::SourceStore;
    let root = fs::canonicalize(root)?;
    let scan = entmoot_core::RepoScanResult {
        reused_files: 0,
        excluded_directories: Vec::new(),
        ignored_files: Vec::new(),
        files: Vec::new(),
        skipped: Vec::new(),
        snapshot_files: entmoot_core::scan::snapshot_file_digests(&root).map_err(failure)?,
    };
    let observed = crate::GixSourceStore::open(&root)
        .map_err(failure)?
        .observe()
        .map_err(failure)?;
    let snapshot =
        crate::snapshot_from_scan(&root, repository, &observed, &scan).map_err(failure)?;
    let files: Vec<_> = scan
        .snapshot_files
        .iter()
        .map(|file| crate::SnapshotFileDigest {
            path: file
                .path
                .strip_prefix(&root)
                .unwrap_or(&file.path)
                .to_string_lossy()
                .replace('\\', "/"),
            content_digest: file.content_digest.clone(),
        })
        .collect();
    let mut inventory = TopologyGraph::new(root.to_string_lossy().into_owned());
    inventory.snapshot = snapshot;
    inventory.snapshot_files = files;
    Ok(inventory)
}

fn publish(
    root: PathBuf,
    mut graph: TopologyGraph,
    previous: Option<WorkspacePublication>,
    options: &BindOptions,
    decisions: Option<&[impact::BindDecision]>,
    syntax_cache: Option<&crate::knowledge_db::syntax_cache::SyntaxCache>,
) -> Result<WorkspaceSummary, WorkspaceError> {
    if options.subject.is_some() || !options.subject_aliases.is_empty() {
        graph.snapshot.subject = options.subject.clone();
        graph.snapshot.subject_aliases = options.subject_aliases.clone();
        // Coverages embed a full SourceSnapshot and derive coverage_id from it.
        // Re-stamp and re-hash after subject so validation stays consistent.
        for coverage in &mut graph.coverages {
            coverage.snapshot = graph.snapshot.clone();
            coverage.coverage_id = coverage.derive_id().map_err(failure)?;
        }
    }
    let storage = storage_directory(&root)?;
    let database = storage.join("knowledge.sqlite3");
    let objects = artifact_store_path(&database)?;
    let started = std::time::Instant::now();
    let inventory_families: Vec<String> = languages(&graph).into_iter().collect();
    // plan_only must not write a publication or adapter profile to workspace storage.
    let adapter_profile = if options.plan_only {
        crate::native_resolver::adapter_profile::discover(&root, Some(&inventory_families))
    } else {
        crate::native_resolver::adapter_profile::discover_and_store(
            &root,
            Some(&inventory_families),
        )
        .map_err(failure)?
    };
    let previous_receipt = previous_summary(previous.as_ref())?;
    // A structural-only repeat need not allocate another export or generation.
    // Re-observe source bytes before returning; equality grants no producer reuse.
    if options.structural_only && !options.plan_only {
        if let (Some(previous), Some(receipt)) = (&previous, &previous_receipt) {
            let digest = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph)?);
            if crate::KnowledgeViewLocator::read(&previous.current.store)
                .map_err(failure)?
                .is_some_and(|locator| locator.view == digest)
            {
                verify_sources(&graph, &root)?;
                let mut receipt = receipt.clone();
                receipt.publication_outcome = "unchanged".into();
                receipt.wall_seconds = started.elapsed().as_secs_f64();
                receipt.adapter_profile = Some(adapter_profile);
                append_adapter_profile_limitations(&mut receipt);
                if let Some(cache) = syntax_cache {
                    if cache.reused != cache.records.len() {
                        retention::check_admission(&root)?;
                    }
                    SqliteKnowledgeRepository::open(&previous.current.store)
                        .map_err(failure)?
                        .store_syntax_cache(cache)
                        .map_err(failure)?;
                    receipt.indexed_files = cache.records.len() - cache.reused;
                    receipt.reused_syntax_files = cache.reused;
                    receipt.reused_extraction_files = cache.reused_extraction;
                    receipt.reused_stitching_files = cache.reused_stitching;
                }
                return Ok(receipt);
            }
        }
    }
    if !options.plan_only {
        retention::check_admission(&root)?;
    }
    let generations = storage.join("current");
    if options.plan_only {
        let location =
            previous
                .as_ref()
                .map(|p| p.current.clone())
                .unwrap_or(PublicationLocation {
                    graph: generations.join("graph.json"),
                    store: generations.join("graph.json.knowledge.sqlite3"),
                    source_root: root.clone(),
                });
        let stages = crate::native_resolver::adapter_profile::with_workspace_root(&root, || {
            bind_graph(
                &mut graph,
                &location,
                &objects,
                options,
                previous_receipt.as_ref(),
                decisions,
            )
        })?;
        let mut report = summary(
            &graph,
            location,
            stages,
            options,
            started.elapsed().as_secs_f64(),
            &crate::PublicationOutcome::default(),
        );
        report.adapter_profile = Some(adapter_profile);
        append_adapter_profile_limitations(&mut report);
        return Ok(report);
    }
    fs::create_dir_all(&generations)?;
    let generation = tempfile::Builder::new()
        .prefix("generation-")
        .tempdir_in(&generations)?;
    let mut location = PublicationLocation {
        graph: generation.path().join("graph.json"),
        store: generation.path().join("graph.json.knowledge.sqlite3"),
        source_root: root.clone(),
    };
    crate::write_graph_json(&graph, &location.graph).map_err(failure)?;
    let mut stages = crate::native_resolver::adapter_profile::with_workspace_root(&root, || {
        bind_graph(
            &mut graph,
            &location,
            &objects,
            options,
            previous_receipt.as_ref(),
            decisions,
        )
    })?;
    // Verify bytes and the full path set again before the publication becomes visible.
    verify_sources(&graph, &root)?;
    let staged_store = location.store.clone();
    retention::check_admission(&root)?;
    let locator = crate::KnowledgeViewLocator {
        database,
        view: entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph)?),
    };
    location.store = generation.path().join("graph.json.knowledge.view.json");
    write_json(&location.store, &locator)?;
    let publication_receipt = sync_graph(&graph, &location)?;
    let mut shared = SqliteKnowledgeRepository::open(&location.store).map_err(failure)?;
    let source = crate::GixSourceStore::open(&root).map_err(failure)?;
    shared
        .store_inventory(&graph, Some(&source))
        .map_err(failure)?;
    if let Some(cache) = syntax_cache {
        shared.store_syntax_cache(cache).map_err(failure)?;
    }
    drop(shared);
    // The shared commit and inventory are complete before exposing the pointer.
    // Only this generation's private staging database can be discarded here.
    crate::knowledge_db::remove_store_files(&staged_store).map_err(failure)?;
    for stage in &mut stages {
        if let Some(work) = &stage.work {
            if work.state == entmoot_contracts::AnalysisWorkState::ArtifactReady {
                use crate::AnalysisWorkQueue;
                let mut queue =
                    crate::SqliteAnalysisWorkQueue::open(&generation.path().join("work.sqlite3"))
                        .map_err(failure)?;
                let token = work
                    .fencing_token
                    .as_ref()
                    .ok_or_else(|| failure("ready work has no fence"))?;
                stage.work = Some(
                    queue
                        .ingested(work.work_id.as_str(), token.as_str())
                        .map_err(failure)?,
                );
                stage.events.push("ingested".into());
            }
        }
    }
    let mut summary = summary(
        &graph,
        location.clone(),
        stages,
        options,
        started.elapsed().as_secs_f64(),
        &publication_receipt.outcome,
    );
    summary.adapter_profile = Some(adapter_profile);
    if let Some(cache) = syntax_cache {
        summary.indexed_files = cache.records.len() - cache.reused;
        summary.reused_syntax_files = cache.reused;
        summary.reused_extraction_files = cache.reused_extraction;
        summary.reused_stitching_files = cache.reused_stitching;
    }
    append_adapter_profile_limitations(&mut summary);
    let summary_path = generation.path().join("summary.json");
    write_json(&summary_path, &summary)?;
    let base = previous.as_ref().and_then(|p| {
        if p.snapshot == graph.snapshot {
            p.base.clone()
        } else {
            Some(p.current.clone())
        }
    });
    let publication = WorkspacePublication {
        schema_version: "entmoot.workspace_publication.v1".into(),
        current: location,
        base,
        snapshot: graph.snapshot.clone(),
        summary: summary_path,
    };
    // Retain the complete generation before the single atomic visibility change.
    let _generation_path = generation.keep();
    write_json(&storage.join("current.json"), &publication)?;
    Ok(summary)
}

fn summary(
    graph: &TopologyGraph,
    publication: PublicationLocation,
    stages: Vec<BindStage>,
    options: &BindOptions,
    wall_seconds: f64,
    outcome: &crate::PublicationOutcome,
) -> WorkspaceSummary {
    WorkspaceSummary {
        indexed_files: 0, reused_syntax_files: 0,
        reused_extraction_files: 0, reused_stitching_files: 0,
        schema_version: "entmoot.workspace_summary.v1".into(), publication,
        snapshot: graph.snapshot.clone(), policy_ids: graph.policy_ids.clone(),
        execution_policy: ExecutionPolicy::Safe, full: options.full,
        require_native: options.require_native, plan_only: options.plan_only,
        structural_only: options.structural_only, stages,
        coverage: crate::build_coverage_report(graph), wall_seconds, budget_seconds: options.budget.wall.as_secs(),
        limitations: vec![
            "binding evidence is not runtime validation or dispatch completeness".into(),
            "automatic reuse is same-snapshot Oxc lexical only; other families require fresh production".into(),
            "peak RSS is not measured by this local receipt".into(),
        ],
        adapter_profile: None,
        publication_outcome: outcome.as_str().into(),
        publication_digest: outcome.digest().into(),
    }
}

fn append_adapter_profile_limitations(summary: &mut WorkspaceSummary) {
    let Some(profile) = &summary.adapter_profile else {
        return;
    };
    for family in profile.hard_gaps() {
        summary.limitations.push(format!(
            "adapter_profile_gap:{family}; build adapter then re-run init, or bind --structural-only"
        ));
    }
}

/// Adjacent knowledge store path used when callers omit `--store`.
pub fn default_store_path(graph_path: &Path) -> PathBuf {
    let locator = PathBuf::from(format!("{}.knowledge.view.json", graph_path.display()));
    if locator.is_file() {
        return locator;
    }
    let mut path = graph_path.as_os_str().to_os_string();
    path.push(".knowledge.sqlite3");
    PathBuf::from(path)
}

pub fn artifact_store_path(store: &Path) -> Result<PathBuf, WorkspaceError> {
    let database = crate::KnowledgeViewLocator::read(store)
        .map_err(failure)?
        .map_or_else(|| store.to_owned(), |locator| locator.database);
    let mut path = database.into_os_string();
    path.push(".objects");
    Ok(PathBuf::from(path))
}

/// Opened publication for shared CLI/MCP read paths.
pub struct OpenedPublication {
    pub engine: crate::QueryEngine,
    /// Digest of the export paired with this inventory, pinned at open time.
    pub publication_digest: String,
    pub compare: Option<std::sync::Arc<TopologyGraph>>,
    /// Effective source root after graph.root discovery.
    pub source_root: Option<PathBuf>,
}

/// Load graph + store, attach optional SourceStore and compare graph.
///
/// When `require_source` is true, a configured source root that fails to open is
/// an error. When false and the root was only inferred from `graph.root`, soft-fail.
pub fn open_query_publication(
    graph_path: &Path,
    store: &Path,
    source_root: Option<&Path>,
    compare_graph: Option<&Path>,
    require_source: bool,
) -> Result<OpenedPublication, WorkspaceError> {
    open_query_publication_options(
        graph_path,
        store,
        source_root,
        compare_graph,
        require_source,
        crate::query::QueryEngineOptions::default(),
    )
}

fn open_query_publication_options(
    graph_path: &Path,
    store: &Path,
    source_root: Option<&Path>,
    compare_graph: Option<&Path>,
    require_source: bool,
    options: crate::query::QueryEngineOptions,
) -> Result<OpenedPublication, WorkspaceError> {
    let repository = SqliteKnowledgeRepository::open_read_snapshot(store).map_err(failure)?;
    repository
        .verify_inventory_export(graph_path)
        .map_err(failure)?;
    let stored = repository.load_inventory(true).map_err(failure)?;
    let retained_inventory = stored.is_some();
    let (graph, publication_digest) = match stored {
        Some(graph) => {
            let digest = repository
                .inventory_export_digest()
                .map_err(failure)?
                .ok_or_else(|| failure("stored inventory has no publication digest"))?;
            (graph, digest)
        }
        None => {
            let bytes = fs::read(graph_path)?;
            let mut graph: TopologyGraph = serde_json::from_slice(&bytes)?;
            graph.rebuild_indexes();
            graph.validate().map_err(failure)?;
            (graph, blake3_hex(&bytes))
        }
    };
    let lifecycle_lease = repository.lifecycle_lease();
    drop(repository);
    let explicit_source = source_root.is_some();
    let source_root = source_root.map(PathBuf::from).or_else(|| {
        let root = PathBuf::from(&graph.root);
        root.is_dir().then_some(root)
    });
    let mut engine = crate::QueryEngine::with_options(graph, store, options)
        .with_lifecycle_lease(lifecycle_lease);
    if retained_inventory {
        engine = engine.with_source_store(std::sync::Arc::new(
            crate::source_store::PublicationSourceStore::new(store),
        ));
    }
    if let Some(root) = &source_root {
        match crate::GixSourceStore::open(root) {
            Ok(source) => {
                let source: std::sync::Arc<dyn crate::SourceStore> = std::sync::Arc::new(source);
                engine = engine.with_source_store(if retained_inventory {
                    std::sync::Arc::new(
                        crate::source_store::PublicationSourceStore::new(store)
                            .with_fallback(source),
                    )
                } else {
                    source
                });
            }
            Err(error) if require_source || explicit_source => {
                return Err(failure(error));
            }
            Err(_) => {}
        }
    }
    let mut compare = None;
    if let Some(path) = compare_graph {
        let compare_graph = crate::load_graph_json(path).map_err(failure)?;
        compare = Some(std::sync::Arc::new(compare_graph.clone()));
        engine = engine.with_compare_graph(compare_graph);
    }
    Ok(OpenedPublication {
        engine,
        publication_digest,
        compare,
        source_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_lock_child_exit() {
        let Some(root) = std::env::var_os("ENTMOOT_TEST_WRITE_LOCK_ROOT") else {
            return;
        };
        let _guard = WriteGuard::acquire(Path::new(&root)).unwrap();
        // Deliberately bypass Drop, as with abrupt process termination.
        std::process::exit(0);
    }

    #[test]
    fn workspace_lock_excludes_live_writers_and_recovers_after_process_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let guard = WriteGuard::acquire(tmp.path()).unwrap();
        assert!(WriteGuard::acquire(tmp.path()).is_err());
        drop(guard);
        let path = crate::workspace::storage_directory(tmp.path())
            .unwrap()
            .join("write.lock");
        assert!(
            path.is_file(),
            "lock identity must survive ordinary release"
        );
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "workspace::tests::workspace_lock_child_exit"])
            .env("ENTMOOT_TEST_WRITE_LOCK_ROOT", tmp.path())
            .status()
            .unwrap();
        assert!(status.success());
        let _recovered = WriteGuard::acquire(tmp.path()).unwrap();
        assert!(path.is_file());
        assert!(WriteGuard::acquire(tmp.path()).is_err());
    }
    fn policy() -> entmoot_contracts::StructuralHashPolicy {
        entmoot_core::ground_truth::policy_from_config_name("v1").unwrap()
    }

    #[test]
    fn init_with_subject_keeps_coverage_snapshots_aligned() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.py"), "def leaf():\n    return 1\n").unwrap();
        let subject = entmoot_contracts::SourceSubject::Package {
            coordinate_purl: "pkg:pypi/fixture".into(),
        };
        let summary = initialize_workspace(
            dir.path(),
            Some("authored/fixture"),
            &policy(),
            &BindOptions {
                structural_only: true,
                subject: Some(subject.clone()),
                ..BindOptions::default()
            },
        )
        .expect("structural init with subject");
        let graph = crate::load_graph_json(&summary.publication.graph).expect("load graph");
        assert_eq!(graph.snapshot.subject.as_ref(), Some(&subject));
        for coverage in &graph.coverages {
            assert_eq!(
                coverage.snapshot, graph.snapshot,
                "coverage snapshot must match graph after subject stamp"
            );
        }
        graph.validate().expect("graph validates");
    }

    #[test]
    fn structural_publication_discovery_drift_and_failure_atomicity() {
        use std::sync::{Mutex, OnceLock};
        fn env_lock() -> &'static Mutex<()> {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            LOCK.get_or_init(|| Mutex::new(()))
        }
        let _guard = env_lock().lock().expect("env lock");
        let prior_adapter = std::env::var_os("ENTMOOT_PYRIGHT_ADAPTER");
        let prior_env = std::env::var_os("ENTMOOT_PYTHON_ENVIRONMENT");
        let dir = tempfile::tempdir().unwrap();
        // Exclusive typo paths: host workstation defaults must not satisfy Python.
        std::env::set_var(
            "ENTMOOT_PYRIGHT_ADAPTER",
            dir.path().join("missing-entmoot-pyright.cjs"),
        );
        std::env::set_var(
            "ENTMOOT_PYTHON_ENVIRONMENT",
            dir.path().join("missing-python-environment.json"),
        );
        let restore = || {
            match &prior_adapter {
                Some(value) => std::env::set_var("ENTMOOT_PYRIGHT_ADAPTER", value),
                None => std::env::remove_var("ENTMOOT_PYRIGHT_ADAPTER"),
            }
            match &prior_env {
                Some(value) => std::env::set_var("ENTMOOT_PYTHON_ENVIRONMENT", value),
                None => std::env::remove_var("ENTMOOT_PYTHON_ENVIRONMENT"),
            }
        };
        fs::write(dir.path().join("app.py"), "def work():\n    return 1\n").unwrap();
        let opts = BindOptions {
            structural_only: true,
            ..Default::default()
        };
        let first = match initialize_workspace(dir.path(), Some("fixture"), &policy(), &opts) {
            Ok(value) => value,
            Err(error) => {
                restore();
                panic!("{error}");
            }
        };
        assert_eq!(first.stages.len(), 1);
        assert_eq!(first.stages[0].stage, "skipped");
        assert!(first.publication.store.is_file());
        let pointer = crate::workspace::storage_directory(dir.path())
            .unwrap()
            .join("current.json");
        let original = fs::read(&pointer).unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        let publication = discover_publication(&nested).unwrap();
        assert_eq!(publication.current.graph, first.publication.graph);
        let graph = crate::load_graph_json(&first.publication.graph).unwrap();
        assert!(!graph
            .snapshot_files
            .iter()
            .any(|f| f.path.starts_with(".entmoot/")));
        assert!(!fs::read(publication.summary)
            .unwrap()
            .starts_with(&[239, 187, 191]));
        let strict = BindOptions {
            require_native: true,
            ..Default::default()
        };
        let strict_err = match bind_workspace(dir.path(), &strict) {
            Ok(summary) => {
                restore();
                panic!("expected invalid_environment, got {summary:?}");
            }
            Err(error) => error.to_string(),
        };
        assert!(strict_err.contains("invalid_environment"), "{strict_err}");
        assert_eq!(fs::read(&pointer).unwrap(), original);
        fs::write(dir.path().join("new.py"), "def added():\n    pass\n").unwrap();
        let drift_err = match bind_workspace(dir.path(), &BindOptions::default()) {
            Ok(summary) => {
                restore();
                panic!("expected snapshot changed, got {summary:?}");
            }
            Err(error) => error.to_string(),
        };
        assert!(drift_err.contains("snapshot changed"), "{drift_err}");
        assert_eq!(fs::read(&pointer).unwrap(), original);
        let second = match initialize_workspace(dir.path(), Some("fixture"), &policy(), &opts) {
            Ok(value) => value,
            Err(error) => {
                restore();
                panic!("{error}");
            }
        };
        assert_ne!(second.snapshot, first.snapshot);
        let left = crate::KnowledgeViewLocator::read(&first.publication.store)
            .unwrap()
            .unwrap();
        let right = crate::KnowledgeViewLocator::read(&second.publication.store)
            .unwrap()
            .unwrap();
        assert_eq!(left.database, right.database);
        assert_ne!(left.view, right.view);
        let old = SqliteKnowledgeRepository::open_read_snapshot(&first.publication.store).unwrap();
        assert_eq!(
            old.load_inventory(true).unwrap().unwrap().snapshot,
            first.snapshot
        );
        assert!(old
            .verify_inventory_export(&second.publication.graph)
            .is_err());
        assert_eq!(
            discover_publication(dir.path())
                .unwrap()
                .base
                .unwrap()
                .graph,
            first.publication.graph
        );
        restore();
    }

    #[test]
    fn plan_only_does_not_create_publication_or_ignore_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("app.py"), "def work():\n    pass\n").unwrap();
        let opts = BindOptions {
            plan_only: true,
            ..Default::default()
        };
        let report = initialize_workspace(dir.path(), Some("fixture"), &policy(), &opts).unwrap();
        assert!(report.plan_only);
        assert!(!dir.path().join(".entmoot").exists());
        assert!(!dir.path().join(".gitignore").exists());
    }

    #[test]
    #[cfg(feature = "resolved-oxc")]
    fn ignored_dependency_changes_reproduce_oxc_package_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("app.js"),
            "import {leaf} from 'dep'; export function caller() { return leaf(1); }",
        )
        .unwrap();
        let dep = dir.path().join("node_modules/dep");
        fs::create_dir_all(&dep).unwrap();
        fs::write(
            dep.join("package.json"),
            r#"{"name":"dep","type":"module","exports":"./index.js"}"#,
        )
        .unwrap();
        fs::write(
            dep.join("index.js"),
            "export function leaf(x) { return x; }",
        )
        .unwrap();
        let first = initialize_workspace(
            dir.path(),
            Some("fixture"),
            &policy(),
            &BindOptions::default(),
        )
        .unwrap();
        fs::write(
            dep.join("index.js"),
            "export function leaf(x, required) { return x; }",
        )
        .unwrap();
        let second = bind_workspace(dir.path(), &BindOptions::default()).unwrap();
        let stage = &second.stages[0];
        assert_eq!(stage.stage, "produced");
        assert_ne!(stage.manifest, first.stages[0].manifest);
        let boundary = |stage: &BindStage| {
            let path = stage
                .manifest
                .as_ref()
                .unwrap()
                .parent()
                .unwrap()
                .join("resolver.json");
            let artifact: entmoot_contracts::ResolverArtifact =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            artifact.package_api.unwrap().references[0]
                .expected_boundary
                .clone()
        };
        assert_ne!(boundary(&first.stages[0]), boundary(stage));
    }

    #[test]
    #[cfg(feature = "resolved-oxc")]
    fn mixed_inventory_reuses_original_oxc_and_full_reproduces() {
        use std::sync::{Mutex, OnceLock};
        fn env_lock() -> &'static Mutex<()> {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            LOCK.get_or_init(|| Mutex::new(()))
        }
        let _guard = env_lock().lock().expect("env lock");
        let prior_adapter = std::env::var_os("ENTMOOT_PYRIGHT_ADAPTER");
        let prior_env = std::env::var_os("ENTMOOT_PYTHON_ENVIRONMENT");
        let dir = tempfile::tempdir().unwrap();
        // Exclusive typo paths: host workstation defaults must not satisfy Python.
        std::env::set_var(
            "ENTMOOT_PYRIGHT_ADAPTER",
            dir.path().join("missing-entmoot-pyright.cjs"),
        );
        std::env::set_var(
            "ENTMOOT_PYTHON_ENVIRONMENT",
            dir.path().join("missing-python-environment.json"),
        );
        let restore = || {
            match &prior_adapter {
                Some(value) => std::env::set_var("ENTMOOT_PYRIGHT_ADAPTER", value),
                None => std::env::remove_var("ENTMOOT_PYRIGHT_ADAPTER"),
            }
            match &prior_env {
                Some(value) => std::env::set_var("ENTMOOT_PYTHON_ENVIRONMENT", value),
                None => std::env::remove_var("ENTMOOT_PYTHON_ENVIRONMENT"),
            }
        };
        fs::write(
            dir.path().join("app.js"),
            "function target() { return 1; }\nfunction caller() { return target(); }\n",
        )
        .unwrap();
        fs::write(dir.path().join("app.py"), "def work():\n    pass\n").unwrap();
        let first = match initialize_workspace(
            dir.path(),
            Some("fixture"),
            &policy(),
            &BindOptions::default(),
        ) {
            Ok(value) => value,
            Err(error) => {
                restore();
                panic!("{error}");
            }
        };
        let oxc = first.stages.iter().find(|s| s.family == "oxc").unwrap();
        assert_eq!(oxc.stage, "produced");
        assert!(oxc.attachment.as_ref().unwrap().resolver_observations > 0);
        assert_eq!(
            first
                .stages
                .iter()
                .find(|s| s.family == "python")
                .unwrap()
                .stage,
            "skipped"
        );
        let before = fs::read(oxc.manifest.as_ref().unwrap()).unwrap();
        let second = bind_workspace(dir.path(), &BindOptions::default()).unwrap();
        let reused = second.stages.iter().find(|s| s.family == "oxc").unwrap();
        assert_eq!(reused.stage, "reused");
        assert_eq!(reused.manifest, oxc.manifest);
        let runtime = oxc::configured_runtime();
        let other_runtime = if runtime == "node" { "browser" } else { "node" };
        assert!(!reusable_lexical_oxc(
            dir.path(),
            reused.manifest.as_ref().unwrap(),
            other_runtime
        )
        .unwrap());
        assert_eq!(fs::read(reused.manifest.as_ref().unwrap()).unwrap(), before);
        let a = crate::load_graph_json(&first.publication.graph).unwrap();
        let b = crate::load_graph_json(&second.publication.graph).unwrap();
        assert_eq!(a.resolved_identities, b.resolved_identities);
        assert_eq!(a.artifacts, b.artifacts);
        let third = bind_workspace(
            dir.path(),
            &BindOptions {
                full: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            third
                .stages
                .iter()
                .find(|s| s.family == "oxc")
                .unwrap()
                .stage,
            "produced"
        );
        // Bad immutable artifact bytes must fail closed, preserving the pointer.
        let pointer = crate::workspace::storage_directory(dir.path())
            .unwrap()
            .join("current.json");
        let previous = fs::read(&pointer).unwrap();
        let manifest = third
            .stages
            .iter()
            .find(|s| s.family == "oxc")
            .unwrap()
            .manifest
            .as_ref()
            .unwrap();
        fs::write(manifest, "{}").unwrap();
        assert!(bind_workspace(dir.path(), &BindOptions::default()).is_err());
        assert_eq!(fs::read(&pointer).unwrap(), previous);
        restore();
    }
    #[test]
    fn writer_lock_and_explicit_graph_preserve_source_identity() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        fs::write(source.path().join("app.py"), "def work():\n    return 1\n").unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            source.path(),
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("fixture/import"),
            },
        )
        .unwrap();
        let location = PublicationLocation {
            graph: output.path().join("graph.json"),
            store: output.path().join("store.sqlite3"),
            source_root: source.path().to_path_buf(),
        };
        sync_graph(&graph, &location).unwrap();
        let options = BindOptions {
            structural_only: true,
            ..Default::default()
        };
        let guard = WriteGuard::acquire(source.path()).unwrap();
        assert!(bind_publication(
            &location.graph,
            Some(source.path()),
            Some(&location.store),
            &options
        )
        .unwrap_err()
        .to_string()
        .contains("write lock"));
        drop(guard);
        let adopted = bind_publication(
            &location.graph,
            Some(source.path()),
            Some(&location.store),
            &options,
        )
        .unwrap();
        assert_eq!(adopted.snapshot, graph.snapshot);
        assert!(!source.path().join(".gitignore").exists());
        let published = crate::load_graph_json(&adopted.publication.graph).unwrap();
        assert_eq!(
            serde_json::to_value(&graph.nodes).unwrap(),
            serde_json::to_value(&published.nodes).unwrap()
        );
        assert_eq!(graph.snapshot_files, published.snapshot_files);
        assert!(crate::workspace::storage_directory(source.path())
            .unwrap()
            .join(".gitignore")
            .is_file());
        let relocated = tempfile::tempdir().unwrap();
        fs::copy(
            source.path().join("app.py"),
            relocated.path().join("app.py"),
        )
        .unwrap();
        let moved = bind_publication(
            &location.graph,
            Some(relocated.path()),
            Some(&location.store),
            &options,
        )
        .unwrap();
        let moved_graph = crate::load_graph_json(&moved.publication.graph).unwrap();
        assert_eq!(moved_graph.snapshot, graph.snapshot);
        assert_eq!(
            PathBuf::from(&moved_graph.root),
            fs::canonicalize(relocated.path()).unwrap()
        );
        crate::read_graph_sources(&moved_graph).unwrap();
    }

    #[test]
    fn publication_preserves_incompatible_workspace_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(".entmoot/current/generation-x/graph.json.knowledge.sqlite3");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 6).unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        assert!(matches!(
            SqliteKnowledgeRepository::open(&path),
            Err(crate::KnowledgeRepositoryError::UnsupportedSchema {
                actual: 6,
                supported: 1
            })
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
