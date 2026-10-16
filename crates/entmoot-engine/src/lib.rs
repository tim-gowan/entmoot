//! Entmoot knowledge and topology reference engine.
//!
//! # Stability
//!
//! Stable external wire contracts live in `entmoot-contracts`.
//! Public items in this unpublished reference engine are capability or embedding
//! surfaces unless their rustdoc explicitly says they are internal. Rust `pub`
//! visibility alone is not a wire-compatibility promise.

pub mod analysis_plan;
pub mod analysis_work;
pub mod artifact;
pub mod artifact_store;
pub mod assertions;
pub mod atomic_write;
pub mod binding_operations;
pub mod canon;
pub mod collaboration;
pub mod collaboration_git;
pub mod compatibility;
pub mod config_inventory;
pub mod display_ref;
pub mod export;
pub mod extractor;
pub mod graph;
pub mod impact;
mod implementation;
pub mod integration_operations;
mod inventory_stitch;
pub mod overlay;
pub mod producer_operations;
pub mod revalidation_operations;
pub use implementation::implementation_fingerprint;
pub mod ingest;
pub mod knowledge;
pub mod knowledge_db;
pub mod location_join;
pub mod manifest_state;
pub mod materialize;
mod process;
pub mod producer;
pub mod query;
pub mod relation_claims;
pub mod revalidation;
pub mod test_validation;
pub use analysis_plan::{plan_analysis, plan_revalidation_analysis, AnalysisPlanError};
pub use analysis_work::{AnalysisWorkError, AnalysisWorkQueue, SqliteAnalysisWorkQueue};
pub use revalidation::{
    evaluate_artifact_revalidation, publish_artifact_evidence, publish_revalidation_ledger,
    revalidate_transition, ArtifactInputReceipt, ArtifactRevalidationReceipt, RevalidationError,
};
pub mod native_resolver;
pub mod resolve_edges;
pub mod resolved_identity;
pub mod resolver_accounting;
pub mod source_store;
pub mod span_index;
mod sqlite_startup;
pub(crate) mod storage_lease;
pub mod workspace;

pub use artifact::{
    adapter_for_manifest, decode_text_bytes, ArtifactAdapter, ArtifactDiagnostic, ArtifactError,
    CycloneDxAdapter, DiagnosticSeverity, InspectedArtifact, NormalizedArtifact, OsvScannerAdapter,
    ResourceInventoryAdapter, SarifAdapter, SemanticClusterAdapter,
};
pub use artifact_store::{ArtifactStoreError, FilesystemArtifactObjectRepository};
pub use assertions::derive_assertions_for_artifact;
pub use atomic_write::AtomicGraphWrite;
pub use config_inventory::{
    apply_configuration_inventory, detect_configuration_syntax, index_configuration_source,
    index_repository_configurations, ConfigDiagnostic, ConfigInventory, ConfigInventoryError,
    ConfigSource, ConfigSyntax,
};
pub use display_ref::{build_fqn, fqn_from_logic_address, DisplayRefKind};
pub use entmoot_contracts::query::{
    FindPayload, InspectPayload, QueryGapCode, QueryPayload, QueryResult, QueryService,
    QueryServiceError, QueryStatus, ReadPayload, ResultBounds, RetrievalReceipt,
    RevisionImpactManifest, SchemaJson, SubjectCandidate, TraversePayload,
};
pub use export::{
    build_coverage_report, load_graph_json, write_graph_json, AttachmentDensity, CoverageReport,
    ProviderCoverage,
};
pub use extractor::{
    build_topology_from_records, build_topology_from_scan, MarkerTopologyExtractor,
};
pub use graph::{
    EdgeKind, GraphSummary, NodeKind, SnapshotFileDigest, StructuralEdge, TopologyGraph,
    TopologyNode, TopologyValidationError, TOPOLOGY_SCHEMA_VERSION,
};
pub use impact::{
    compare_revision_impact, compare_revision_impact_with_correspondence, ImpactError,
};
pub use ingest::{ingest_artifact_manifest, ArtifactIngestStats};
pub use knowledge::{inventory_skeleton, KnowledgeStore};
pub use knowledge_db::{
    storage_fingerprint, BufferedPublicationReceipt, KnowledgeDependency, KnowledgeOwnershipStats,
    KnowledgeRecordRef, KnowledgeRecordState, KnowledgeRepository, KnowledgeRepositoryError,
    KnowledgeStoreBatches, KnowledgeSyncStats, KnowledgeViewLocator, LocalOperations,
    LocalVisibilityChange, LocalVisibilityPage, LocalVisibilityReason, OperationError, OwnerBatch,
    OwnerReplacement, PublicationBuffer, PublicationOutcome, SqliteKnowledgeRepository,
};
pub use location_join::{resolve_location, AttachmentIndexes};
pub use manifest_state::{ManifestUnverified, ManifestVerified};
pub use native_resolver::{
    prepare_resolver_manifest, produce_clang_resolver, produce_go_resolver, produce_java_resolver,
    produce_oxc_resolver, produce_python_resolver, produce_roslyn_resolver, produce_rust_resolver,
    produce_typescript_resolver, publish_dogfood, DogfoodPublishRequest, DogfoodPublishResult,
    ExecutionPolicy, NativeProduceRequest, NativeProduceResult, NativeProducerError, NativeReceipt,
    ProducerBudget, PublicationManifest, PUBLICATION_SCHEMA, RA_AP_PIN,
};
pub use overlay::{compare_inventory_overlay, OverlayError};
pub use producer::AssertError;
pub use query::{
    artifact_result, dependency_result, expert_proposition_result, expert_relation_claims_result,
    owner_result, trace_result, unresolved_result, BoundedWalkBudget, ExpertPage,
    ExpertQueryResult, FindQuery, InspectQuery, QueryEngine, QueryError, ReadQuery, TraverseQuery,
    EXPERT_QUERY_RESULT_SCHEMA,
};
pub use relation_claims::{
    record_structural_resolved_call, record_structural_unresolved_invoke, upsert_relation_claim,
    upsert_relation_edge,
};
pub use resolve_edges::apply_resolved_call_edges;
pub use resolved_identity::{
    derive_resolved_identities_from_graph, derive_resolved_identity,
    inventoried_calls_for_callable, InventoriedReference, RESOLUTION_POLICY_ID,
};
pub use source_store::{
    GixSourceStore, SourceFile, SourceObservation, SourceReadRequest, SourceStore, SourceStoreError,
};
pub use span_index::{CallableMatch, SpanIndex};

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::extractor::build_topology_from_scan_with_snapshot;
use entmoot_contracts::StructuralHashPolicy;
use entmoot_core::{
    scan_repo_with_analysis, scan_repo_with_analysis_digest_gated, RepoScanResult, SkipReason,
    StructuralHasher,
};

/// Options for [`build_topology_from_repo_with_options`].
#[derive(Debug, Clone, Default)]
pub struct TopologyBuildOptions<'a> {
    pub policy: Option<&'a StructuralHashPolicy>,
    pub repository: Option<&'a str>,
}

/// Build topology and structural identity only. Evidence is ingested separately.
pub fn build_topology_from_repo_with_options(
    root: &Path,
    options: TopologyBuildOptions<'_>,
) -> Result<TopologyGraph, Box<dyn std::error::Error>> {
    let prepared = prepare_topology_scan(root, &options)?;
    let scan = scan_repo_with_analysis(
        &prepared.hasher,
        &prepared.root,
        options.policy,
        Some(&prepared.repository_id),
    )?;
    finalize_topology_from_scan(&prepared, scan)
}

fn build_topology_with_syntax_cache(
    root: &Path,
    options: TopologyBuildOptions<'_>,
    store: Option<&Path>,
) -> Result<(TopologyGraph, knowledge_db::syntax_cache::SyntaxCache), Box<dyn std::error::Error>> {
    let prepared = prepare_topology_scan(root, &options)?;
    let policy = options.policy.cloned().unwrap_or_default();
    use inventory_stitch::{CallPart, SyntaxPart};
    use knowledge_db::syntax_cache::{FileParts, SyntaxCache};
    let context = SyntaxCache::context(&prepared.repository_id, &policy)?;
    let mut prior = match store {
        Some(path) => {
            SqliteKnowledgeRepository::open_read_snapshot(path)?.load_syntax_cache(&context)?
        }
        None => SyntaxCache {
            context,
            ..Default::default()
        },
    };
    let scan = entmoot_core::scan::scan_repo_with_analysis_cached(
        &prepared.hasher,
        &prepared.root,
        Some(&policy),
        Some(&prepared.repository_id),
        &prior.records,
    )?;
    let snapshot = snapshot_from_scan(
        &prepared.root,
        &prepared.repository_id,
        &prepared.observed,
        &scan,
    )?;
    let mut extractor = crate::extractor::MarkerTopologyExtractor::with_snapshot(
        prepared.root.to_string_lossy(),
        snapshot,
    );
    extractor.on_repo_begin(&prepared.root.to_string_lossy());
    let (mut graph, _) = extractor.finish_syntax();
    let digests: HashMap<_, _> = scan
        .snapshot_files
        .iter()
        .map(|f| (&f.path, &f.content_digest))
        .collect();
    let mut cache = SyntaxCache {
        context: prior.context.clone(),
        reused: scan.reused_files,
        ..Default::default()
    };
    let mut syntax_parts = HashMap::new();
    let mut prior_calls = HashMap::new();
    for file in &scan.files {
        let path = relative_scan_path(&prepared.root, &file.path);
        let digest = *digests
            .get(&file.path)
            .ok_or("parsed file lacks its source commitment")?;
        let reusable = prior.qualified_parts(&path, digest).is_some();
        let syntax = match reusable.then(|| prior.parts.remove(&path)).flatten() {
            Some(parts) => {
                cache.reused_extraction += 1;
                prior_calls.insert(path.clone(), parts.calls);
                parts.syntax
            }
            None => SyntaxPart::build_record(&prepared.root, &path, digest, Some(&file.record))?,
        };
        syntax.clone().apply(&mut graph);
        syntax_parts.insert(path.clone(), syntax);
        cache
            .records
            .insert(path, (digest.clone(), file.record.clone()));
    }
    let index = crate::resolve_edges::StructuralCallIndex::new(&graph);
    // Match ordinary resolution's path/occurrence order on every platform.
    let ordered: std::collections::BTreeMap<_, _> = cache.records.iter().collect();
    for (path, (_, record)) in ordered {
        let calls = match prior_calls
            .remove(path)
            .filter(|calls| calls.matches(&index))
        {
            Some(calls) => {
                cache.reused_stitching += 1;
                calls
            }
            None => CallPart::build_record(&index, path, Some(record)),
        };
        calls.apply(&mut graph);
        let syntax = syntax_parts
            .remove(path)
            .ok_or("file lacks its syntax fragment")?;
        cache
            .parts
            .insert(path.clone(), FileParts { syntax, calls });
    }
    cache.unchanged = cache.records.len() == prior.records.len()
        && cache.reused_extraction == cache.records.len()
        && cache.reused_stitching == cache.records.len();
    finalize_topology_graph(&prepared, &scan, &mut graph)?;
    Ok((graph, cache))
}

/// Build a structural working-tree head, reusing unchanged-path inventory from `base`.
///
/// Paths whose content digest still matches `base.snapshot_files` skip parse/hash.
/// Changed and added paths are rehashed; removed paths are dropped. Callers must
/// still run [`workspace::verify_sources`] as the second full path/digest fence.
pub fn build_topology_from_repo_reusing_base(
    root: &Path,
    options: TopologyBuildOptions<'_>,
    base: &TopologyGraph,
) -> Result<TopologyGraph, Box<dyn std::error::Error>> {
    let prepared = prepare_topology_scan(root, &options)?;
    let policy = options.policy.cloned().unwrap_or_default();
    if base.policy_ids != vec![policy.fingerprint().to_string()]
        || base.snapshot.repository_id != prepared.repository_id
        || Path::new(&base.root) != prepared.root
    {
        return build_topology_from_repo_with_options(root, options);
    }
    if root.join(".entmoot/current.json").is_file() {
        let publication = workspace::discover_publication(root)?;
        return build_topology_with_syntax_cache(root, options, Some(&publication.current.store))
            .map(|(graph, _)| graph);
    }
    // A topology-only base lacks the complete lexical binding inputs. Reparse
    // call-bearing inventories unless qualified syntax records are available.
    if base
        .nodes
        .iter()
        .any(|node| node.kind == NodeKind::CallStub)
    {
        return build_topology_from_repo_with_options(root, options);
    }
    if !base.coverages.iter().any(|coverage| {
        matches!(&coverage.measurement,
        entmoot_contracts::CoverageMeasurement::StructuralScan { method_id, .. }
        if method_id == entmoot_core::scan::STRUCTURAL_SCAN_POLICY_ID)
    }) || base.nodes.iter().any(|node| {
        node.kind == NodeKind::CallStub
            && node
                .call_site
                .as_ref()
                .is_none_or(|site| site.ports.is_none())
    }) {
        return build_topology_from_repo_with_options(root, options);
    }
    let parsed: HashSet<_> = base
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && n.file_hash.is_some())
        .filter_map(|n| n.path.as_deref())
        .collect();
    let prior_digests: HashMap<String, String> = base
        .snapshot_files
        .iter()
        .filter(|file| parsed.contains(file.path.as_str()))
        .map(|file| (file.path.clone(), file.content_digest.clone()))
        .collect();
    let scan = scan_repo_with_analysis_digest_gated(
        &prepared.hasher,
        &prepared.root,
        options.policy,
        Some(&prepared.repository_id),
        Some(&prior_digests),
    )?;
    let rehashed_paths: HashSet<String> = scan
        .files
        .iter()
        .map(|file| relative_scan_path(&prepared.root, &file.path))
        .collect();
    let skipped_paths: HashSet<String> = scan
        .skipped
        .iter()
        .map(|file| relative_scan_path(&prepared.root, &file.path))
        .collect();
    let unchanged_paths: HashSet<String> = scan
        .snapshot_files
        .iter()
        .filter_map(|file| {
            let rel = relative_scan_path(&prepared.root, &file.path);
            let reusable = prior_digests
                .get(rel.as_str())
                .is_some_and(|digest| digest == &file.content_digest)
                && !rehashed_paths.contains(rel.as_str())
                && !skipped_paths.contains(rel.as_str());
            reusable.then_some(rel)
        })
        .collect();

    let snapshot = snapshot_from_scan(
        &prepared.root,
        &prepared.repository_id,
        &prepared.observed,
        &scan,
    )?;
    let seed = splice_structural_inventory(
        base,
        &unchanged_paths,
        snapshot,
        &prepared.root.to_string_lossy(),
    );
    let mut extractor = crate::extractor::MarkerTopologyExtractor::from_seeded_graph(seed);
    extractor.on_repo_begin(&prepared.root.to_string_lossy());
    for file in &scan.files {
        let rel = relative_scan_path(&prepared.root, &file.path);
        let source = std::fs::read_to_string(&file.path).ok();
        let file_id = crate::graph::file_node_id(&rel);
        extractor.on_file(&rel, &file.record, source.as_deref());
        extractor.on_file_end(&file_id);
    }
    let mut graph = extractor.finish();
    finalize_topology_graph(&prepared, &scan, &mut graph)?;
    Ok(graph)
}

struct PreparedTopologyScan {
    root: PathBuf,
    repository_id: String,
    observed: SourceObservation,
    hasher: StructuralHasher,
}

fn prepare_topology_scan(
    root: &Path,
    options: &TopologyBuildOptions<'_>,
) -> Result<PreparedTopologyScan, Box<dyn std::error::Error>> {
    let root = std::fs::canonicalize(root).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "failed to resolve topology root {}: {error}",
                root.display()
            ),
        )
    })?;
    let source_store = GixSourceStore::open(&root)?;
    let observed = source_store.observe()?;
    let repository_id = options
        .repository
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| observed.repository_id.clone());
    Ok(PreparedTopologyScan {
        root,
        repository_id,
        observed,
        hasher: StructuralHasher::new(),
    })
}

fn finalize_topology_from_scan(
    prepared: &PreparedTopologyScan,
    scan: RepoScanResult,
) -> Result<TopologyGraph, Box<dyn std::error::Error>> {
    let snapshot = snapshot_from_scan(
        &prepared.root,
        &prepared.repository_id,
        &prepared.observed,
        &scan,
    )?;
    let mut graph = build_topology_from_scan_with_snapshot(&prepared.root, &scan, Some(snapshot));
    finalize_topology_graph(prepared, &scan, &mut graph)?;
    Ok(graph)
}

fn finalize_topology_graph(
    prepared: &PreparedTopologyScan,
    scan: &RepoScanResult,
    graph: &mut TopologyGraph,
) -> Result<(), Box<dyn std::error::Error>> {
    for skipped in &scan.skipped {
        let (code, message) = match &skipped.reason {
            SkipReason::ReadError(error) => (
                "source.read_failed",
                format!("source inventory skipped: {error}"),
            ),
            SkipReason::HashError(error) => (
                "source.hash_failed",
                format!("source inventory skipped: {error}"),
            ),
            SkipReason::UnknownExtension => continue,
        };
        let relative = relative_scan_path(&prepared.root, &skipped.path);
        let file_id = crate::graph::file_node_id(&relative);
        if graph.node(&file_id).is_none() {
            let label = Path::new(&relative)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&relative);
            let mut file = TopologyNode::shell(
                &file_id,
                NodeKind::File,
                label,
                Some(crate::graph::repo_node_id()),
            );
            file.path = Some(relative.clone());
            file.display_ref = relative.clone();
            file.display_ref_kind = Some(DisplayRefKind::File);
            graph.upsert_node(file);
            graph.add_edge(StructuralEdge::structural(
                crate::graph::repo_node_id(),
                &file_id,
                EdgeKind::Contains,
            ));
        }
        graph.diagnostics.push(
            crate::artifact::ArtifactDiagnostic::error(code, message).with_pointer(&relative),
        );
    }
    graph.snapshot_files = scan
        .snapshot_files
        .iter()
        .map(|file| SnapshotFileDigest {
            path: relative_scan_path(&prepared.root, &file.path),
            content_digest: file.content_digest.clone(),
        })
        .collect();
    materialize_snapshot_file_nodes(graph);
    let mut excluded_directories: Vec<_> = scan
        .excluded_directories
        .iter()
        .map(|p| relative_scan_path(&prepared.root, p))
        .collect();
    excluded_directories.sort();
    excluded_directories.dedup();
    let mut excludes = entmoot_core::scan::scan_scope_exclusions();
    excludes.extend(excluded_directories.iter().map(|p| format!("{p}/**")));
    excludes.extend(
        scan.ignored_files
            .iter()
            .map(|path| relative_scan_path(&prepared.root, path)),
    );
    let observed: Vec<_> = graph
        .snapshot_files
        .iter()
        .map(|f| f.path.clone())
        .collect();
    let mut skipped: Vec<_> = scan
        .skipped
        .iter()
        .map(|f| relative_scan_path(&prepared.root, &f.path))
        .collect();
    skipped.sort();
    let unsupported = scan
        .skipped
        .iter()
        .filter(|s| matches!(s.reason, SkipReason::UnknownExtension))
        .count() as u64;
    let failed = scan.skipped.len() as u64 - unsupported;
    let mut coverage = entmoot_contracts::CoverageObservation {
        coverage_id: String::new(), snapshot: graph.snapshot.clone(),
        scope: entmoot_contracts::ScopeUniverse::from_selectors("entmoot.repository_scan.v1", vec![], vec!["**".into()], excludes, entmoot_contracts::ScopeCompleteness::Partial, Some(&observed)),
        measurement: entmoot_contracts::CoverageMeasurement::StructuralScan {
            method_id: entmoot_core::scan::STRUCTURAL_SCAN_POLICY_ID.into(),
            content_manifest_digest: entmoot_contracts::ContentDigest::from_trusted(entmoot_contracts::content_manifest_digest(&graph.snapshot_files.iter().map(|f| (f.path.clone(), f.content_digest.clone())).collect::<Vec<_>>())),
            observed_files: observed.len() as u64, parsed: (observed.len() - skipped.len()) as u64,
            heuristic_skipped: 0, unsupported, failed, excluded_directories, excluded_files: skipped,
        },
        message: Some("Source inventory only; excluded directories were not enumerated; no resolver, scanner or origin claim".into()),
    };
    coverage.coverage_id = coverage.derive_id()?;
    coverage.validate()?;
    graph.coverages = vec![coverage];
    let configuration = index_repository_configurations(
        &prepared.root,
        &graph.snapshot.repository_id,
        &graph.snapshot.revision,
        graph.snapshot.snapshot_digest.as_deref(),
        &graph.snapshot_files,
    )?;
    apply_configuration_inventory(graph, configuration);
    // Rehydration and file splicing may enumerate equivalent memberships in a
    // different order. Canonical inventory ordering keeps repeats byte-stable.
    graph.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    graph
        .edges
        .sort_by(|a, b| (&a.from, &a.to, a.kind as u8).cmp(&(&b.from, &b.to, b.kind as u8)));
    graph.rebuild_indexes();
    graph.validate()?;
    Ok(())
}

fn relative_scan_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Copy structural inventory for unchanged paths from `base` into a fresh head graph.
///
/// Evidence collections stay empty. Cross-path module stubs referenced by kept
/// imports are retained. Structural edges keep endpoints wholly inside the keep set;
/// dangling `relation_edge_id` pointers are cleared.
fn splice_structural_inventory(
    base: &TopologyGraph,
    keep_paths: &HashSet<String>,
    snapshot: entmoot_contracts::SourceSnapshot,
    root: &str,
) -> TopologyGraph {
    let mut graph = TopologyGraph::new(root);
    graph.snapshot = snapshot;

    let mut keep_ids = HashSet::new();
    for node in &base.nodes {
        match node.kind {
            NodeKind::Repo => {
                keep_ids.insert(node.id.clone());
            }
            NodeKind::Resource => {}
            NodeKind::ModuleStub if node.path.is_none() => {}
            _ => {
                if node
                    .path
                    .as_ref()
                    .is_some_and(|path| keep_paths.contains(path))
                {
                    keep_ids.insert(node.id.clone());
                }
            }
        }
    }
    for edge in &base.edges {
        if !keep_ids.contains(&edge.from) {
            continue;
        }
        if let Some(to) = base.node(&edge.to) {
            if to.kind == NodeKind::ModuleStub && to.path.is_none() {
                keep_ids.insert(edge.to.clone());
            }
        }
    }

    for node in &base.nodes {
        if !keep_ids.contains(&node.id) {
            continue;
        }
        let copied = node.clone();
        graph.upsert_node(copied);
    }
    for policy_id in &base.policy_ids {
        graph.record_policy_id(policy_id);
    }
    for edge in &base.edges {
        if keep_ids.contains(&edge.from)
            && keep_ids.contains(&edge.to)
            && edge.relation_edge_id.is_none()
            && !(edge.kind == EdgeKind::Calls && edge.resolved)
        {
            let mut copied = edge.clone();
            copied.relation_edge_id = None;
            graph.add_edge(copied);
        }
    }
    graph
}

fn materialize_snapshot_file_nodes(graph: &mut TopologyGraph) {
    let paths: Vec<_> = graph
        .snapshot_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    for path in paths {
        let file_id = crate::graph::file_node_id(&path);
        if graph.node(&file_id).is_some() {
            continue;
        }
        let label = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&path);
        let mut file = TopologyNode::shell(
            &file_id,
            NodeKind::File,
            label,
            Some(crate::graph::repo_node_id()),
        );
        file.path = Some(path.clone());
        file.display_ref = path;
        file.display_ref_kind = Some(DisplayRefKind::File);
        graph.upsert_node(file);
        graph.add_edge(StructuralEdge::structural(
            crate::graph::repo_node_id(),
            file_id,
            EdgeKind::Contains,
        ));
    }
}

fn snapshot_from_scan(
    root: &Path,
    repository_id: &str,
    observed: &SourceObservation,
    scan: &RepoScanResult,
) -> Result<entmoot_contracts::SourceSnapshot, Box<dyn std::error::Error>> {
    snapshot_from_scan_with_subject(root, repository_id, observed, scan, None, Vec::new())
}

fn snapshot_from_scan_with_subject(
    root: &Path,
    repository_id: &str,
    observed: &SourceObservation,
    scan: &RepoScanResult,
    subject: Option<entmoot_contracts::SourceSubject>,
    subject_aliases: Vec<entmoot_contracts::SourceSubject>,
) -> Result<entmoot_contracts::SourceSnapshot, Box<dyn std::error::Error>> {
    let entries: Vec<(String, String)> = scan
        .snapshot_files
        .iter()
        .map(|f| {
            let rel = f
                .path
                .strip_prefix(root)
                .unwrap_or(&f.path)
                .to_string_lossy()
                .replace('\\', "/");
            Ok((rel, f.content_digest.clone()))
        })
        .collect::<Result<_, std::io::Error>>()?;
    let digest = entmoot_contracts::content_manifest_digest(&entries);
    let revision = observed
        .revision
        .clone()
        .unwrap_or_else(|| entmoot_contracts::strip_digest_prefix(&digest).to_string());
    Ok(entmoot_contracts::SourceSnapshot {
        repository_id: repository_id.to_owned(),
        revision,
        source_state: observed.source_state,
        snapshot_digest: (observed.source_state == entmoot_contracts::SourceState::WorkingTree)
            .then_some(digest),
        estate_id: None,
        subject,
        subject_aliases,
    })
}

/// Read every source file represented by a graph.
///
/// Evidence joins require exact source coordinates, so unreadable files are
/// reported instead of being silently omitted and converted into false unmatched rows.
pub fn read_graph_sources(
    graph: &TopologyGraph,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let paths: Vec<&str> = graph
        .snapshot_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    read_graph_sources_for_paths(graph, &paths)
}

/// Digest-verified source reads for an explicit path set (attach location join).
///
/// Paths present in `snapshot_files` are digest-fenced; missing or changed bytes
/// fail loud. Paths absent from the snapshot are omitted (same as the former
/// full-corpus reader), so out-of-inventory SARIF locations stay unresolved
/// rather than aborting attach.
pub fn read_graph_sources_for_paths(
    graph: &TopologyGraph,
    paths: &[&str],
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let root = std::fs::canonicalize(Path::new(&graph.root)).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("failed to resolve graph root {}: {error}", graph.root),
        )
    })?;
    let by_path: HashMap<&str, &str> = graph
        .snapshot_files
        .iter()
        .map(|file| (file.path.as_str(), file.content_digest.as_str()))
        .collect();
    let mut unique = paths
        .iter()
        .map(|path| path.replace('\\', "/"))
        .collect::<Vec<_>>();
    unique.sort();
    unique.dedup();
    let mut sources = Vec::new();
    for path in unique {
        let Some(content_digest) = by_path.get(path.as_str()) else {
            continue;
        };
        let relative = validated_graph_relative_path(&path)?;
        let candidate = root.join(relative);
        let full = std::fs::canonicalize(&candidate).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "failed to resolve topology source {}: {error}",
                    candidate.display()
                ),
            )
        })?;
        if !full.starts_with(&root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "topology source resolves outside graph root: {}",
                    full.display()
                ),
            )
            .into());
        }
        let bytes = std::fs::read(&full).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("snapshot file {path} is unavailable: {error}"),
            )
        })?;
        if entmoot_contracts::blake3_hex(&bytes) != *content_digest {
            return Err(format!(
                "source snapshot changed after indexing at {path}; regenerate the graph before attaching evidence"
            )
            .into());
        }
        // File-grain attachment does not require textual decoding. Region
        // attachment remains unresolved for non-UTF-8 source, never lossy.
        if let Ok(source) = String::from_utf8(bytes) {
            sources.push((path, source));
        }
    }
    Ok(sources)
}

fn validated_graph_relative_path(path: &str) -> Result<PathBuf, std::io::Error> {
    let relative = PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("graph file path must stay relative to graph.root: {path}"),
        ));
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn graph_sources_reject_paths_outside_root() {
        let dir = tempdir().unwrap();
        let mut graph = TopologyGraph::new(dir.path().to_string_lossy());
        graph.snapshot_files.push(SnapshotFileDigest {
            path: "../outside.rs".into(),
            content_digest: format!("blake3:{}", "0".repeat(64)),
        });

        let error = read_graph_sources(&graph).unwrap_err();
        assert!(error.to_string().contains("must stay relative"));
    }

    #[test]
    fn every_snapshot_file_is_a_navigable_file_node() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("dependencies.lock"), "opaque lock data\n").unwrap();

        let graph =
            build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default())
                .unwrap();
        let file_id = crate::graph::file_node_id("dependencies.lock");
        let file = graph
            .node(&file_id)
            .expect("snapshot-only file must remain navigable");
        assert_eq!(file.kind, NodeKind::File);
        assert_eq!(file.path.as_deref(), Some("dependencies.lock"));
        assert!(graph.edges.iter().any(|edge| {
            edge.from == crate::graph::repo_node_id()
                && edge.to == file_id
                && edge.kind == EdgeKind::Contains
        }));

        let mut incomplete = graph.clone();
        incomplete.nodes.retain(|node| node.id != file_id);
        incomplete.rebuild_indexes();
        assert!(incomplete
            .validate()
            .unwrap_err()
            .to_string()
            .contains("has no file node"));
    }

    #[test]
    fn unhashable_source_is_a_navigable_file_with_an_explicit_gap() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("broken.py"), "def broken(:\n").unwrap();

        let graph =
            build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default())
                .unwrap();
        let file_id = crate::graph::file_node_id("broken.py");
        let file = graph.node(&file_id).expect("failed file remains navigable");
        assert_eq!(file.path.as_deref(), Some("broken.py"));
        assert!(graph.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "source.hash_failed"
                && diagnostic.provider_pointer.as_deref() == Some("broken.py")
        }));
    }

    #[test]
    fn minified_filename_does_not_suppress_inventory() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("bundle.min.js"),
            "function noop() { return 1; }\n",
        )
        .unwrap();

        let graph =
            build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default())
                .unwrap();
        let file_id = crate::graph::file_node_id("bundle.min.js");
        let file = graph
            .node(&file_id)
            .expect("included file remains navigable");
        assert_eq!(file.path.as_deref(), Some("bundle.min.js"));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Callable && node.label == "noop"));
        assert!(graph.diagnostics.is_empty());
    }

    #[test]
    fn reusing_base_matches_full_rebuild_callable_hashes() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("keep.py"), "def keep():\n    return 1\n").unwrap();
        std::fs::write(
            dir.path().join("change.py"),
            "def change():\n    return 1\n",
        )
        .unwrap();
        let base =
            build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default())
                .unwrap();
        std::fs::write(
            dir.path().join("change.py"),
            "def change():\n    return 2\n",
        )
        .unwrap();
        let full =
            build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default())
                .unwrap();
        let reused = build_topology_from_repo_reusing_base(
            dir.path(),
            TopologyBuildOptions {
                repository: Some(&base.snapshot.repository_id),
                ..TopologyBuildOptions::default()
            },
            &base,
        )
        .unwrap();
        assert_eq!(reused.snapshot_files, full.snapshot_files);
        let callable_hashes = |graph: &TopologyGraph| {
            graph
                .nodes
                .iter()
                .filter(|node| node.kind == NodeKind::Callable)
                .map(|node| {
                    (
                        node.logic_address.clone(),
                        node.structural_hash.clone(),
                        node.signature_hash.clone(),
                    )
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(callable_hashes(&reused), callable_hashes(&full));
    }

    #[test]
    fn reuse_requires_policy_repository_and_scan_identity() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.py"),
            "# comment\ndef a():\n    return 1\n",
        )
        .unwrap();
        let base = build_topology_from_repo_with_options(dir.path(), Default::default()).unwrap();
        let mut stale_policy = base.clone();
        stale_policy.policy_ids = vec!["0".repeat(64)];
        let mut stale_scan = base.clone();
        stale_scan.coverages.clear();
        // Poison reusable inventory: a rejected base must be rebuilt, not copied.
        for stale in [&mut stale_policy, &mut stale_scan] {
            for node in &mut stale.nodes {
                if node.kind == NodeKind::Callable {
                    node.label = "stale-inventory".into();
                }
            }
        }
        for (prior, repository) in [
            (&stale_policy, None),
            (&stale_scan, None),
            (&base, Some("another-repository")),
        ] {
            let options = TopologyBuildOptions {
                policy: None,
                repository,
            };
            let full = build_topology_from_repo_with_options(dir.path(), options.clone()).unwrap();
            let reused = build_topology_from_repo_reusing_base(dir.path(), options, prior).unwrap();
            assert_eq!(
                serde_json::to_value(reused).unwrap(),
                serde_json::to_value(full).unwrap()
            );
        }
    }

    #[test]
    fn reuse_never_copies_resolver_edges_into_a_new_snapshot() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.py"),
            "def a():\n    return b()\ndef b():\n    return 1\n",
        )
        .unwrap();
        let mut base =
            build_topology_from_repo_with_options(dir.path(), Default::default()).unwrap();
        let ids: Vec<_> = base
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Callable)
            .map(|n| n.id.clone())
            .collect();
        let mut edge = StructuralEdge::structural(&ids[0], &ids[1], EdgeKind::Calls);
        edge.resolved = true;
        base.add_edge(edge);
        let reused =
            build_topology_from_repo_reusing_base(dir.path(), Default::default(), &base).unwrap();
        let full = build_topology_from_repo_with_options(dir.path(), Default::default()).unwrap();
        assert_eq!(
            serde_json::to_value(reused).unwrap(),
            serde_json::to_value(full).unwrap()
        );
    }
}
