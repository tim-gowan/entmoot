//! Source observation, validated cache reuse and structural inventory assembly.
use crate::*;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

pub use crate::scan_topology::build_topology_from_scan;
use crate::scan_topology::build_topology_from_scan_with_snapshot;
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

pub fn build_topology_with_syntax_cache(
    root: &Path,
    options: TopologyBuildOptions<'_>,
    store: Option<&Path>,
) -> Result<(TopologyGraph, knowledge_db::syntax_cache::SyntaxCache), Box<dyn std::error::Error>> {
    build_with_file_facts(root, options, store, None, false, BuildTarget::Graph)?.into_graph()
}

/// Retain immutable file units before graph derivation. Failed production leaves
/// sealed units reusable; it does not change the current publication manifest.
pub fn build_topology_with_retained_facts(
    root: &Path,
    options: TopologyBuildOptions<'_>,
    prior: Option<&Path>,
    units: &Path,
) -> Result<(TopologyGraph, knowledge_db::syntax_cache::SyntaxCache), Box<dyn std::error::Error>> {
    build_with_file_facts(root, options, prior, Some(units), false, BuildTarget::Graph)?
        .into_graph()
}

/// A verified unchanged scan has no repository graph allocation. The façade
/// keeps the existing publication and refreshes its operator receipt/discovery.
pub enum StructuralRefresh {
    Unchanged(knowledge_db::syntax_cache::SyntaxCache),
    /// A sealed view whose full syntax and relations remain in the repository.
    Retained {
        /// Publication identity, source manifest, configuration and coverage only;
        /// this is not a complete graph or a navigation projection.
        metadata: Box<TopologyGraph>,
        view: String,
        outcome: PublicationOutcome,
        cache: knowledge_db::syntax_cache::SyntaxCache,
    },
    Rebuilt {
        graph: Box<TopologyGraph>,
        cache: knowledge_db::syntax_cache::SyntaxCache,
    },
}

impl StructuralRefresh {
    fn into_graph(
        self,
    ) -> Result<(TopologyGraph, knowledge_db::syntax_cache::SyntaxCache), Box<dyn std::error::Error>>
    {
        match self {
            Self::Rebuilt { graph, cache } => Ok((*graph, cache)),
            Self::Unchanged(_) | Self::Retained { .. } => {
                Err("graph projection required by this caller".into())
            }
        }
    }
}

enum BuildTarget<'a> {
    Graph,
    Retained {
        subject: Option<&'a entmoot_contracts::SourceSubject>,
        aliases: &'a [entmoot_contracts::SourceSubject],
    },
}

/// Retain a structural publication with file-sized projections. `reuse` is valid
/// only when the façade has verified unchanged subject/evidence selection.
/// The result is private until the façade verifies source and publishes a locator.
pub fn retain_structural_inventory(
    root: &Path,
    options: TopologyBuildOptions<'_>,
    prior: Option<&Path>,
    units: &Path,
    reuse: bool,
    subject: Option<&entmoot_contracts::SourceSubject>,
    aliases: &[entmoot_contracts::SourceSubject],
) -> Result<StructuralRefresh, Box<dyn std::error::Error>> {
    build_with_file_facts(
        root,
        options,
        prior,
        Some(units),
        reuse,
        BuildTarget::Retained { subject, aliases },
    )
}

fn build_with_file_facts(
    root: &Path,
    options: TopologyBuildOptions<'_>,
    store: Option<&Path>,
    units: Option<&Path>,
    reuse_publication: bool,
    target: BuildTarget<'_>,
) -> Result<StructuralRefresh, Box<dyn std::error::Error>> {
    let started = std::time::Instant::now();
    let observe = crate::measurement::PhaseTimer::start("source_observation");
    let prepared = prepare_topology_scan(root, &options)?;
    let policy = options.policy.cloned().unwrap_or_default();
    use inventory_stitch::{CallPart, SyntaxPart};
    use knowledge_db::syntax_cache::SyntaxCache;
    let context = SyntaxCache::context(&prepared.repository_id, &policy)?;
    let prior = match store {
        Some(path) => {
            SqliteKnowledgeRepository::open_read_snapshot(path)?.load_syntax_cache(&context)?
        }
        None => SyntaxCache::new(context)?,
    };
    let mut cache = SyntaxCache::new(prior.context.clone())?;
    if let Some(units) = units {
        cache.retain_units_in(units)?;
    }
    let manifest = entmoot_core::scan::source_manifest(&prepared.root)?;
    cache.phases.push(observe.finish());
    let parse = crate::measurement::PhaseTimer::start("parse_and_retain_file_facts");
    let workers = entmoot_core::scan::ScanWorkers::new()?;
    let mut scan = entmoot_core::scan::ScanMetadata::from_manifest(&prepared.root, &manifest);
    // Persist parser IR before deriving any graph. Each file is an independent
    // parse job; source commitments and explicit skips survive the phase barrier.
    for inputs in manifest.batches().flat_map(|batch| batch.chunks(4)) {
        let mut facts = prior.facts_for_inputs(inputs)?;
        let missing: Vec<_> = inputs
            .iter()
            .filter(|input| !facts.contains_key(&input.path))
            .cloned()
            .collect();
        facts.extend(cache.facts_for_inputs(&missing)?);
        workers.scan_facts_into(
            &prepared.root,
            &prepared.repository_id,
            &policy,
            inputs,
            &facts,
            |mut outcome| {
                cache.reused += usize::from(outcome.reused);
                if outcome.facts.is_none() {
                    cache.retain_source(&outcome.input.content_digest, &outcome.source)?;
                }
                if let Some(facts) = outcome.facts.take() {
                    cache.insert_facts(
                        outcome.input.path.clone(),
                        outcome.input.content_digest.clone(),
                        &facts,
                        Some(&outcome.source),
                    )?;
                }
                if let Some(reason) = outcome.skipped {
                    scan.skipped.push(entmoot_core::scan::SkippedFile {
                        path: prepared.root.join(outcome.input.path),
                        reason,
                    });
                }
                Ok::<_, Box<dyn std::error::Error>>(())
            },
        )?;
    }
    drop(workers);
    cache.phases.push(parse.finish());
    let snapshot = snapshot_from_scan_with_subject(
        &prepared.root,
        &prepared.repository_id,
        &prepared.observed,
        &scan.snapshot_files,
        None,
        Vec::new(),
    )?;
    let gaps: Vec<_> = scan
        .skipped
        .iter()
        .map(|file| (relative_scan_path(&prepared.root, &file.path), &file.reason))
        .collect();
    cache.scan_commitment = Some(entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
        "entmoot.structural_scan_inputs.v1",
        &prepared.root,
        &snapshot,
        &cache.context,
        &manifest.inputs,
        &manifest.exclusions,
        gaps,
    ))?));
    if reuse_publication {
        let reuse = crate::measurement::PhaseTimer::start("verify_sealed_scan_reuse");
        let unchanged = store
            .map(|path| cache.reuse_sealed_scan(&prior, path))
            .transpose()?
            .unwrap_or(false);
        cache.phases.push(reuse.finish());
        if unchanged {
            cache.build_wall_seconds = started.elapsed().as_secs_f64();
            return Ok(StructuralRefresh::Unchanged(cache));
        }
    }
    if let BuildTarget::Retained { subject, aliases } = target {
        let finalize = crate::measurement::PhaseTimer::start("inventory_metadata");
        let mut extractor = crate::extractor::MarkerTopologyExtractor::with_snapshot(
            prepared.root.to_string_lossy(),
            snapshot,
        );
        extractor.on_repo_begin(&prepared.root.to_string_lossy());
        let (mut metadata, _) = extractor.finish_syntax();
        // Metadata finalization needs only file membership and configuration;
        // parsed file nodes replace its placeholders during retention.
        finalize_topology_inventory(&prepared, &scan, &mut metadata)?;
        if !cache.is_empty() {
            metadata.record_policy_id(policy.fingerprint().as_str());
        }
        metadata.snapshot.subject = subject.cloned();
        metadata.snapshot.subject_aliases = aliases.to_vec();
        for coverage in &mut metadata.coverages {
            coverage.snapshot = metadata.snapshot.clone();
            coverage.coverage_id = coverage.derive_id()?;
        }
        cache.phases.push(finalize.finish());
        let database = units.ok_or("retained structural production requires a unit store")?;
        let mut repository = SqliteKnowledgeRepository::open(database)?;
        let retain = crate::measurement::PhaseTimer::start("streamed_structural_publication");
        let paths: Vec<_> = cache.paths().map(str::to_owned).collect();
        let (view, outcome) =
            repository.seal_structural_parts(&metadata, &mut cache, |writer, cache| {
                let mut index = crate::resolve_edges::StructuralCallIndex::default();
                let mut header = TopologyGraph::new(metadata.root.clone());
                header.snapshot = metadata.snapshot.clone();
                header.policy_ids = metadata.policy_ids.clone();
                let extract = crate::measurement::PhaseTimer::start("syntax_projection");
                for path in &paths {
                    let digest = cache.source_digest(path).ok_or_else(|| {
                        KnowledgeRepositoryError::InvalidRecord {
                            collection: "file_facts",
                            reason: "IR lacks source commitment".into(),
                        }
                    })?;
                    let mut part = header.clone();
                    part.snapshot_files = vec![SnapshotFileDigest {
                        path: path.clone(),
                        content_digest: digest.to_owned(),
                    }]
                    .into();
                    let unit = cache.retained_unit(path).ok_or_else(|| {
                        KnowledgeRepositoryError::InvalidRecord {
                            collection: "file_facts",
                            reason: "file lacks a local sealed parser unit".into(),
                        }
                    })?;
                    let source = cache.source(path)?.ok_or_else(|| {
                        KnowledgeRepositoryError::InvalidRecord {
                            collection: "file_facts",
                            reason: "file lacks captured source".into(),
                        }
                    })?;
                    if let Some(part) = writer.reuse_syntax(unit, &part)? {
                        index.extend_nodes(&part.nodes);
                        cache.reused_extraction += 1;
                        continue;
                    }
                    let record = cache.record(path)?.ok_or_else(|| {
                        KnowledgeRepositoryError::InvalidRecord {
                            collection: "file_facts",
                            reason: "file lacks parser IR".into(),
                        }
                    })?;
                    let syntax =
                        SyntaxPart::build_source(&prepared.root, path, digest, &record, &source)
                            .map_err(|error| KnowledgeRepositoryError::InvalidRecord {
                                collection: "inventory",
                                reason: error.to_string(),
                            })?;
                    syntax.apply(&mut part);
                    index.extend_nodes(&part.nodes);
                    writer.append_syntax(unit, &part)?;
                }
                cache.phases.push(extract.finish());
                let resolve = crate::measurement::PhaseTimer::start("structural_resolution");
                for path in &paths {
                    let digest = cache.source_digest(path).ok_or_else(|| {
                        KnowledgeRepositoryError::InvalidRecord {
                            collection: "file_facts",
                            reason: "IR lacks source commitment".into(),
                        }
                    })?;
                    let calls = match prior
                        .qualified_calls(path, digest)?
                        .filter(|calls| calls.matches(&index))
                    {
                        Some(calls) => {
                            cache.reused_stitching += 1;
                            calls
                        }
                        None => {
                            let record = cache.record(path)?.ok_or_else(|| {
                                KnowledgeRepositoryError::InvalidRecord {
                                    collection: "file_facts",
                                    reason: "file lacks parser IR".into(),
                                }
                            })?;
                            CallPart::build_record(&index, path, Some(&record))
                        }
                    };
                    let mut part = header.clone();
                    calls.apply(&mut part);
                    writer.append(&part)?;
                    cache.insert_calls(path, &calls)?;
                }
                cache.phases.push(resolve.finish());
                Ok(())
            })?;
        cache.phases.push(retain.finish());
        cache.build_wall_seconds = started.elapsed().as_secs_f64();
        return Ok(StructuralRefresh::Retained {
            metadata: Box::new(metadata),
            view,
            outcome,
            cache,
        });
    }
    let extract = crate::measurement::PhaseTimer::start("syntax_projection");
    let mut extractor = crate::extractor::MarkerTopologyExtractor::with_snapshot(
        prepared.root.to_string_lossy(),
        snapshot,
    );
    extractor.on_repo_begin(&prepared.root.to_string_lossy());
    let (mut graph, _) = extractor.finish_syntax();
    let paths: Vec<_> = cache.paths().map(str::to_owned).collect();
    for path in &paths {
        let digest = cache
            .source_digest(path)
            .ok_or("IR lacks source commitment")?;
        let record = cache.record(path)?.ok_or("file lacks parser IR")?;
        let source = cache
            .source(path)?
            .ok_or("file facts lack captured source")?;
        let syntax = SyntaxPart::build_source(&prepared.root, path, digest, &record, &source)?;
        syntax.apply(&mut graph);
    }
    let prior_files = prior.len();
    cache.phases.push(extract.finish());
    let resolve = crate::measurement::PhaseTimer::start("structural_resolution");
    // Imports are lookup inputs, not proof of a closed component. All symbol
    // inventory must exist before positive, negative or ambiguous call lookups.
    let index = crate::resolve_edges::StructuralCallIndex::new(&graph);
    for path in &paths {
        let digest = cache
            .source_digest(path)
            .ok_or("IR lacks source commitment")?;
        let cached = prior.qualified_calls(path, digest)?;
        let calls = match cached.filter(|calls| calls.matches(&index)) {
            Some(calls) => {
                cache.reused_stitching += 1;
                calls
            }
            None => {
                let record = cache.record(path)?.ok_or("file lacks parser IR")?;
                CallPart::build_record(&index, path, Some(&record))
            }
        };
        calls.apply(&mut graph);
        cache.insert_calls(path, &calls)?;
    }
    drop(index);
    cache.phases.push(resolve.finish());
    let finalize = crate::measurement::PhaseTimer::start("inventory_finalization");
    cache.unchanged = cache.len() == prior_files
        && cache.reused == cache.len()
        && cache.reused_stitching == cache.len()
        && store.is_some();
    finalize_topology_inventory(&prepared, &scan, &mut graph)?;
    cache.phases.push(finalize.finish());
    cache.build_wall_seconds = started.elapsed().as_secs_f64();
    Ok(StructuralRefresh::Rebuilt {
        graph: Box::new(graph),
        cache,
    })
}

/// Build a structural working-tree head, reusing unchanged-path inventory from `base`.
///
/// Compatible paths with matching content digests can reuse inventory. Unqualified
/// bases and call-bearing graphs without qualified syntax cache records are rebuilt.
/// Changed and added paths are rehashed; removed paths are dropped. Callers must
/// still run [`crate::location::verify_sources`] as the second full path/digest fence.
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
    if crate::location::storage_directory(root)?
        .join("current.json")
        .is_file()
    {
        let publication = crate::location::discover_publication(root)?;
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

pub struct PreparedTopologyScan {
    pub root: PathBuf,
    pub repository_id: String,
    pub observed: SourceObservation,
    pub hasher: StructuralHasher,
}

pub fn prepare_topology_scan(
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

pub fn finalize_topology_graph(
    prepared: &PreparedTopologyScan,
    scan: &RepoScanResult,
    graph: &mut TopologyGraph,
) -> Result<(), Box<dyn std::error::Error>> {
    finalize_topology_inventory(
        prepared,
        &entmoot_core::scan::ScanMetadata::from(scan),
        graph,
    )
}

/// Complete graph assembly consumes source membership/gaps, not hydrated parser IR.
pub fn finalize_topology_inventory(
    prepared: &PreparedTopologyScan,
    scan: &entmoot_core::scan::ScanMetadata,
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
            entmoot_contracts::ArtifactDiagnostic::error(code, message).with_pointer(&relative),
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
    graph.canonicalize_inventory();
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

pub fn snapshot_from_scan(
    root: &Path,
    repository_id: &str,
    observed: &SourceObservation,
    scan: &RepoScanResult,
) -> Result<entmoot_contracts::SourceSnapshot, Box<dyn std::error::Error>> {
    snapshot_from_scan_with_subject(
        root,
        repository_id,
        observed,
        &scan.snapshot_files,
        None,
        Vec::new(),
    )
}

fn snapshot_from_scan_with_subject(
    root: &Path,
    repository_id: &str,
    observed: &SourceObservation,
    files: &[entmoot_core::scan::SnapshotFile],
    subject: Option<entmoot_contracts::SourceSubject>,
    subject_aliases: Vec<entmoot_contracts::SourceSubject>,
) -> Result<entmoot_contracts::SourceSnapshot, Box<dyn std::error::Error>> {
    let entries: Vec<(String, String)> = files
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

pub fn validated_graph_relative_path(path: &str) -> Result<PathBuf, std::io::Error> {
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

use crate::location::{failure, WorkspaceError};
use std::fs;
/// Digest-only source walk shared by producer submission and publication fences.
pub fn observe_source_inventory(
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
    inventory.snapshot_files = files.into();
    Ok(inventory)
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

#[cfg(test)]
mod retained_structural_tests {
    use super::*;

    #[test]
    fn streamed_file_publication_matches_fresh_graph_across_revision_changes() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let database = data.path().join("knowledge.sqlite3");
        std::fs::write(
            source.path().join("a.py"),
            "from .b import leaf\ndef caller():\n    leaf()\n    missing()\n",
        )
        .unwrap();
        std::fs::write(source.path().join("b.py"), "def leaf():\n    return 1\n").unwrap();
        std::fs::write(
            source.path().join("notes.txt"),
            "unsupported inventory member",
        )
        .unwrap();
        let options = || TopologyBuildOptions {
            repository: Some("streamed-fixture"),
            ..Default::default()
        };
        let mut previous = None;
        let mut histories = Vec::new();
        let mut stable_caller = None;
        std::fs::write(source.path().join("service.yaml"), "apiVersion: v1\nkind: Service\nmetadata:\n  name: endpoint\nspec:\n  ports:\n    - port: 80\n").unwrap();
        for step in 0..5 {
            match step {
                1 => std::fs::write(source.path().join("b.py"), "def leaf():\n    return 2\n")
                    .unwrap(),
                2 => std::fs::write(source.path().join("c.py"), "def another():\n    return 3\n")
                    .unwrap(),
                3 => std::fs::rename(source.path().join("b.py"), source.path().join("moved.py"))
                    .unwrap(),
                4 => std::fs::remove_file(source.path().join("moved.py")).unwrap(),
                _ => (),
            }
            let expected = build_topology_with_syntax_cache(source.path(), options(), None)
                .unwrap()
                .0;
            let StructuralRefresh::Retained {
                metadata,
                view,
                cache,
                ..
            } = retain_structural_inventory(
                source.path(),
                options(),
                previous.as_deref(),
                &database,
                false,
                None,
                &[],
            )
            .unwrap()
            else {
                panic!("changed publication must be retained")
            };
            assert_eq!(
                cache.reused_extraction,
                [0, 1, 2, 2, 2][step],
                "unchanged files must reuse their sealed syntax projection",
            );
            assert!(
                metadata
                    .nodes
                    .iter()
                    .all(|node| node.kind != NodeKind::Callable),
                "publication metadata must not collect parsed nodes"
            );
            let caller = expected
                .nodes
                .iter()
                .find(|node| node.kind == NodeKind::Callable && node.label == "caller")
                .unwrap();
            if let Some(stable) = &stable_caller {
                assert_eq!(&caller.id, stable);
            } else {
                stable_caller = Some(caller.id.clone());
            }
            assert_eq!(
                expected.resources.len(),
                1,
                "configuration inventory survives streaming publication"
            );
            entmoot_contracts::ContentDigest::parse(view.clone()).unwrap();
            let locator = data.path().join(format!("{step}.view.json"));
            std::fs::write(
                &locator,
                serde_json::to_vec(&KnowledgeViewLocator {
                    database: database.clone(),
                    view,
                })
                .unwrap(),
            )
            .unwrap();
            let repository = SqliteKnowledgeRepository::open_read_snapshot(&locator).unwrap();
            let mut export = blake3::Hasher::new();
            serde_json::to_writer(&mut export, &expected).unwrap();
            export.update(b"\n");
            assert_eq!(
                repository.inventory_export_digest().unwrap().unwrap(),
                format!("blake3:{}", export.finalize().to_hex())
            );
            let actual = repository.load_inventory(false).unwrap().unwrap();
            let notes = actual
                .snapshot_files
                .iter()
                .find(|file| file.path == "notes.txt")
                .unwrap();
            let retained_source = repository
                .retained_source(crate::SourceReadRequest {
                    snapshot: &actual.snapshot,
                    path: &notes.path,
                    expected_content_digest: &notes.content_digest,
                })
                .unwrap()
                .unwrap();
            assert_eq!(retained_source.bytes, b"unsupported inventory member");
            actual.validate().unwrap();
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            let unresolved = actual.unresolved_relations.len();
            assert_eq!(
                unresolved,
                if step < 3 { 1 } else { 2 },
                "renamed/removed imports must reopen unresolved targets"
            );
            histories.push((locator.clone(), expected.content_digest().unwrap()));
            previous = Some(locator.clone());
            drop(repository);
            drop(cache);
            let repeat = retain_structural_inventory(
                source.path(),
                options(),
                Some(&locator),
                &database,
                true,
                None,
                &[],
            )
            .unwrap();
            assert!(
                matches!(repeat, StructuralRefresh::Unchanged(_)),
                "identical refresh must skip graph production"
            );
        }
        let database_read = rusqlite::Connection::open(&database).unwrap();
        let versions: i64 = database_read
            .query_row(
                "SELECT COUNT(*) FROM inventory_node_versions WHERE node_id=?1",
                [&stable_caller],
                |row| row.get(0),
            )
            .unwrap();
        let memberships: i64 = database_read.query_row("SELECT COUNT(*) FROM inventory_nodes JOIN inventory_node_versions USING(node_key) WHERE node_id=?1", [&stable_caller], |row| row.get(0)).unwrap();
        assert_eq!(
            versions, 1,
            "unchanged logical occurrence retains one node version across revisions"
        );
        assert_eq!(
            memberships, 5,
            "each revision still selects its exact node membership"
        );
        drop(database_read);
        for (locator, expected) in histories {
            let graph = SqliteKnowledgeRepository::open_read_snapshot(&locator)
                .unwrap()
                .load_inventory(false)
                .unwrap()
                .unwrap();
            assert_eq!(
                graph.content_digest().unwrap(),
                expected,
                "later writes must preserve historical revision membership"
            );
        }
    }

    fn retain_test_revision(
        source: &Path,
        data: &Path,
        previous: Option<&Path>,
        step: usize,
    ) -> (PathBuf, TopologyGraph, usize) {
        let StructuralRefresh::Retained { view, cache, .. } = retain_structural_inventory(
            source,
            TopologyBuildOptions::default(),
            previous,
            &data.join("knowledge.sqlite3"),
            false,
            None,
            &[],
        )
        .unwrap() else {
            panic!("changed revision must be retained")
        };
        let reused = cache.reused_extraction;
        drop(cache);
        let locator = data.join(format!("{step}.view.json"));
        std::fs::write(
            &locator,
            serde_json::to_vec(&KnowledgeViewLocator {
                database: data.join("knowledge.sqlite3"),
                view,
            })
            .unwrap(),
        )
        .unwrap();
        let actual = SqliteKnowledgeRepository::open_read_snapshot(&locator)
            .unwrap()
            .load_inventory(false)
            .unwrap()
            .unwrap();
        let expected =
            build_topology_with_syntax_cache(source, TopologyBuildOptions::default(), None)
                .unwrap()
                .0;
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        (locator, actual, reused)
    }

    #[test]
    fn damaged_projection_aborts_new_view_and_preserves_history() {
        for damage in [
            "DELETE FROM structural_file_nodes WHERE unit_id=(SELECT unit_id FROM file_fact_units WHERE path='a.py') AND ordinal=0",
            "DELETE FROM structural_file_edges WHERE unit_id=(SELECT unit_id FROM file_fact_units WHERE path='a.py') AND ordinal=0",
            "UPDATE inventory_node_versions SET label='damaged' WHERE node_key=(SELECT node_key FROM structural_file_nodes WHERE unit_id=(SELECT unit_id FROM file_fact_units WHERE path='a.py') AND ordinal=0)",
        ] {
            let source = tempfile::tempdir().unwrap();
            let data = tempfile::tempdir().unwrap();
            std::fs::write(source.path().join("a.py"), "def keep():\n    return 1\n").unwrap();
            std::fs::write(source.path().join("b.py"), "def change():\n    return 1\n").unwrap();
            let (locator, before, _) = retain_test_revision(source.path(), data.path(), None, 0);
            let database = data.path().join("knowledge.sqlite3");
            let backup = data.path().join("before.sqlite3");
            // No live repositories remain; retain the exact pre-damage database.
            std::fs::copy(&database, &backup).unwrap();
            let connection = rusqlite::Connection::open(&database).unwrap();
            assert!(connection.execute(damage, []).unwrap() > 0);
            drop(connection);
            std::fs::write(source.path().join("b.py"), "def change():\n    return 2\n").unwrap();
            let error = retain_structural_inventory(source.path(), TopologyBuildOptions::default(),
                Some(&locator), &database, false, None, &[]).err().expect("damaged sealed reuse must fail");
            assert!(error.to_string().contains("commitment"), "{error}");
            let connection = rusqlite::Connection::open(&database).unwrap();
            assert_eq!(connection.query_row("SELECT COUNT(*) FROM inventory_headers", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
            drop(connection);
            std::fs::copy(&backup, &database).unwrap();
            let historical = SqliteKnowledgeRepository::open_read_snapshot(&locator).unwrap()
                .load_inventory(false).unwrap().unwrap();
            assert_eq!(before.content_digest().unwrap(), historical.content_digest().unwrap());
        }
    }

    #[test]
    fn collecting_history_retains_shared_projections_and_removes_orphans() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("a.py"), "def keep():\n    return 1\n").unwrap();
        std::fs::write(source.path().join("b.py"), "def change():\n    return 1\n").unwrap();
        let (first, _, _) = retain_test_revision(source.path(), data.path(), None, 0);
        std::fs::write(source.path().join("b.py"), "def change():\n    return 2\n").unwrap();
        let (second, expected, _) =
            retain_test_revision(source.path(), data.path(), Some(&first), 1);
        let old: KnowledgeViewLocator =
            serde_json::from_slice(&std::fs::read(first).unwrap()).unwrap();
        let database = data.path().join("knowledge.sqlite3");
        {
            let mut maintenance = knowledge_db::retention::Maintenance::open(&database).unwrap();
            maintenance
                .collect(&std::collections::BTreeSet::from([old.view]))
                .unwrap();
        }
        let retained = SqliteKnowledgeRepository::open_read_snapshot(&second)
            .unwrap()
            .load_inventory(false)
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.content_digest().unwrap(),
            expected.content_digest().unwrap()
        );
        let connection = rusqlite::Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM structural_file_projections",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        drop(connection);
        std::fs::write(source.path().join("b.py"), "def change():\n    return 3\n").unwrap();
        let (_, _, reused) = retain_test_revision(source.path(), data.path(), Some(&second), 2);
        assert_eq!(reused, 1);
    }
}
