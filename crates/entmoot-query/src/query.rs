//! Read-only, transport-neutral query engine for CLI, SDK, and MCP consumers.
//!
//! The public boundary is four composable operations: find an anchor, inspect an
//! exact subject/evidence record, traverse typed relations, and read verified
//! source. Provider- or task-specific views stay behind these operations.

mod adjacency;
mod admission;
mod assessments;
mod boundaries;
mod call_components;
mod checkpoint_map;
mod continuation;
mod contraction;
mod relationships;
pub use assessments::{assess_test_impact, assess_unused};
mod clones;
mod conflicts;
mod expert;
pub use entmoot_contracts::query::QueryEngineOptions;

#[cfg(test)]
mod identity_regressions;
mod impact_relay;
mod inventory;
mod knowledge;
mod neighborhood;
mod output;
mod read;
mod receipt;
mod requests;
mod retained_calls;
mod roles;
mod seeds;
mod selection;
mod status;
mod subject_search;
mod substring;
mod unresolved;
mod walk;
mod walk_cache;

pub use expert::{
    artifact_result, assertions_result, dependency_result, expert_proposition_result,
    expert_relation_claims_result, owner_result, relations_result, resources_result, trace_result,
    unresolved_result, ExpertPage, ExpertQueryResult, EXPERT_QUERY_RESULT_SCHEMA,
};
pub use requests::{BoundedWalkBudget, FindQuery, InspectQuery, ReadQuery, TraverseQuery};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use entmoot_contracts::query::{
    CallDirection, CallPageCursor, QueryResult, QueryService, QueryServiceError, ResultBounds,
};
use entmoot_contracts::{
    address_identity_key, Address, PropositionId, RelationClaimState, RelationClaimValue,
    RelationEdgeId, RelationKind, ResolutionState,
};
use serde_json::{json, Value};
use thiserror::Error;

use crate::{KnowledgeRead, NodeKind, SourceStore, SourceStoreError, TopologyGraph, TopologyNode};

pub use entmoot_contracts::query::QUERY_RESULT_SCHEMA;

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("query input is invalid: {0}")]
    InvalidInput(String),
    #[error("query service is unavailable: {0}")]
    Unavailable(String),
    #[error("knowledge query failed: {0}")]
    Repository(#[from] crate::KnowledgeRepositoryError),
    #[error("query result serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("query implementation produced an invalid result: {0}")]
    Contract(#[from] entmoot_contracts::query::QueryContractError),
    #[error("authoritative source read failed: {0}")]
    Source(#[from] SourceStoreError),
    #[error("source file is not valid UTF-8: {0}")]
    UnsupportedSourceEncoding(String),
}

#[derive(Debug, Default)]
struct QueryIndexes {
    substring: substring::SubstringIndex,
    nodes_by_initializer_integer: HashMap<String, Vec<usize>>,
    roles: roles::RoleIndex,
    composition_scopes: Vec<Option<usize>>,
    nodes_by_path: HashMap<String, Vec<usize>>,
    nodes_by_exact_selector: HashMap<String, Vec<usize>>,
    nodes_by_label_or_fqn: HashMap<String, Vec<usize>>,
    nodes_by_address: HashMap<String, Vec<usize>>,
    structural_edges_by_node: Vec<Vec<usize>>,
    attachments_by_node: Vec<Vec<usize>>,
    attachments_by_address: HashMap<String, Vec<usize>>,
    findings_by_location: HashMap<String, Vec<usize>>,
    locations_by_id: HashMap<String, usize>,
    diagnostics_by_path: HashMap<String, Vec<usize>>,
    snapshot_digest_by_path: HashMap<String, String>,
    relations: adjacency::RelationIndex,
    assertions_by_address: HashMap<String, Vec<usize>>,
    unresolved_calls_by_address: HashMap<String, Vec<usize>>,
}

impl QueryIndexes {
    fn new(graph: &TopologyGraph, substring_postings_budget: usize) -> Self {
        let mut nodes_by_path: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            if let Some(path) = &node.path {
                nodes_by_path
                    .entry(path.replace('\\', "/"))
                    .or_default()
                    .push(index);
            }
        }
        let mut nodes_by_exact_selector: HashMap<String, Vec<usize>> = HashMap::new();
        let mut nodes_by_initializer_integer: HashMap<String, Vec<usize>> = HashMap::new();
        let mut nodes_by_label_or_fqn: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            let mut selectors = vec![node.id.clone(), node.display_ref.clone()];
            if let Some(integer) = node
                .declaration
                .as_ref()
                .and_then(|d| d.syntax.as_ref())
                .and_then(|s| s.initializer.as_ref())
                .and_then(|i| i.integer())
            {
                nodes_by_initializer_integer
                    .entry(integer)
                    .or_default()
                    .push(index);
            }
            if let Some(address_id) = node.address_id.as_deref() {
                selectors.push(address_id.to_owned());
            }
            if let Some(logic_address) = node.logic_address.as_deref() {
                selectors.push(logic_address.to_owned());
            }
            if let Some(structural_hash) = node.structural_hash.as_deref() {
                selectors.extend(structural_hash_exact_selectors(structural_hash));
            }
            selectors.sort_unstable();
            selectors.dedup();
            for selector in selectors {
                nodes_by_exact_selector
                    .entry(selector)
                    .or_default()
                    .push(index);
            }
            nodes_by_label_or_fqn
                .entry(node.label.to_lowercase())
                .or_default()
                .push(index);
            if let Some(fqn) = node.fqn.as_deref() {
                nodes_by_label_or_fqn
                    .entry(fqn.to_lowercase())
                    .or_default()
                    .push(index);
            }
        }
        let mut structural_edges_by_node = vec![Vec::new(); graph.nodes.len()];
        for (index, edge) in graph.edges.iter().enumerate() {
            if let Some(node_index) = graph.node_by_id.get(&edge.from) {
                structural_edges_by_node[*node_index].push(index);
            }
            if edge.to != edge.from {
                if let Some(node_index) = graph.node_by_id.get(&edge.to) {
                    structural_edges_by_node[*node_index].push(index);
                }
            }
        }
        let snapshot_digest_by_path = graph
            .snapshot_files
            .iter()
            .map(|file| (file.path.replace('\\', "/"), file.content_digest.clone()))
            .collect();
        let mut nodes_by_address: HashMap<String, Vec<usize>> = HashMap::new();
        let resource_addresses: HashMap<_, _> = graph
            .resources
            .iter()
            .map(|resource| (resource.occurrence_id.as_str(), &resource.logical_address))
            .collect();
        for (index, node) in graph.nodes.iter().enumerate() {
            for address in node_addresses_with_resource(
                node,
                &graph.snapshot.repository_id,
                resource_addresses.get(node.id.as_str()).copied(),
            ) {
                nodes_by_address
                    .entry(address_identity_key(&address))
                    .or_default()
                    .push(index);
            }
        }
        let relations = adjacency::RelationIndex::new(graph, &nodes_by_address);
        let mut attachments_by_node = vec![Vec::new(); graph.nodes.len()];
        let mut attachments_by_address: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, attachment) in graph.attachments.iter().enumerate() {
            let Some(target) = &attachment.selected_target else {
                continue;
            };
            if let Some(node_id) = &target.node_id {
                if let Some(node_index) = graph.node_by_id.get(node_id) {
                    attachments_by_node[*node_index].push(index);
                }
            }
            if let Some(address_id) = &target.address_id {
                attachments_by_address
                    .entry(address_id.clone())
                    .or_default()
                    .push(index);
            }
        }
        let mut findings_by_location: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, finding) in graph.findings.iter().enumerate() {
            for location_id in &finding.primary_location_ids {
                findings_by_location
                    .entry(location_id.clone())
                    .or_default()
                    .push(index);
            }
        }
        let locations_by_id = graph
            .locations
            .iter()
            .enumerate()
            .map(|(index, location)| (location.location_id.clone(), index))
            .collect();
        let mut diagnostics_by_path: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, diagnostic) in graph.diagnostics.iter().enumerate() {
            let Some(pointer) = diagnostic.provider_pointer.as_deref() else {
                continue;
            };
            let path = pointer.split_once(':').map_or(pointer, |(path, _)| path);
            diagnostics_by_path
                .entry(path.to_owned())
                .or_default()
                .push(index);
        }
        let mut assertions_by_address: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, assertion) in graph.assertions.iter().enumerate() {
            let address = assertion
                .subject_address
                .as_ref()
                .map(|subject| subject.address.clone())
                .unwrap_or_else(|| {
                    entmoot_contracts::topology_node_address(&assertion.subject.entity_id)
                });
            assertions_by_address
                .entry(address_identity_key(&address))
                .or_default()
                .push(index);
        }
        for indexes in assertions_by_address.values_mut() {
            indexes.sort_by_key(|index| &graph.assertions[*index].assertion_id);
        }
        let mut unresolved_calls_by_address: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, attempt) in graph.unresolved_relations.iter().enumerate() {
            if attempt.predicate == RelationKind::Calls {
                unresolved_calls_by_address
                    .entry(address_identity_key(&attempt.source))
                    .or_default()
                    .push(index);
            }
        }
        for indexes in unresolved_calls_by_address.values_mut() {
            indexes.sort_by_key(|index| graph.unresolved_relations[*index].id.as_str());
        }
        Self {
            roles: roles::RoleIndex::new(graph),
            substring: if substring_postings_budget == 0 {
                substring::SubstringIndex::unindexed(graph.nodes.len())
            } else {
                substring::SubstringIndex::new(
                    graph.nodes.iter().map(searchable_node_text),
                    substring_postings_budget,
                )
            },
            composition_scopes: {
                let mut interned = HashMap::new();
                graph
                    .relation_edges
                    .iter()
                    .map(|edge| {
                        composition_scope(graph, &edge.scope).map(|scope| {
                            let next = interned.len();
                            *interned.entry(scope).or_insert(next)
                        })
                    })
                    .collect()
            },
            nodes_by_path,
            nodes_by_initializer_integer,
            nodes_by_exact_selector,
            nodes_by_label_or_fqn,
            nodes_by_address,
            structural_edges_by_node,
            attachments_by_node,
            attachments_by_address,
            findings_by_location,
            locations_by_id,
            diagnostics_by_path,
            snapshot_digest_by_path,
            relations,
            assertions_by_address,
            unresolved_calls_by_address,
        }
    }
}

/// Read-only query engine shared by protocol transports.
///
/// The SQLite connection is opened per operation, so this handle is safe to
/// share between asynchronous MCP requests without wrapping a connection in a
/// mutex or leaking the physical store into the public query contract.
#[derive(Clone)]
pub struct QueryEngine {
    lifecycle_lease: Option<crate::storage_lease::StorageLease>,
    walk_cache: Option<std::sync::Arc<walk_cache::WalkCache>>,
    shortcuts: std::sync::Arc<walk_cache::BoundedCache<contraction::ShortcutOverlay>>,
    shortcut_build: std::sync::Arc<std::sync::Mutex<()>>,
    continuations: std::sync::Arc<continuation::Searches>,
    options: QueryEngineOptions,
    graph: std::sync::Arc<TopologyGraph>,
    deferred_navigation: Option<std::sync::Arc<DeferredNavigation>>,
    publication_digest: Option<String>,
    store_path: PathBuf,
    reader: std::sync::Arc<dyn crate::RepositoryReader>,
    checkpoints: Option<std::sync::Arc<dyn crate::QueryCheckpoints>>,
    indexes: std::sync::Arc<QueryIndexes>,
    output_generation: std::sync::Arc<std::sync::OnceLock<entmoot_contracts::ContentDigest>>,
    source_store: Option<std::sync::Arc<dyn SourceStore>>,
    compare_graph: Option<std::sync::Arc<QueryComparison>>,
    compatibility: Option<std::sync::Arc<QueryCompatibility>>,
}

type DeferredNavigation = std::sync::OnceLock<Result<Box<QueryEngine>, String>>;

enum QueryComparison {
    Ready(TopologyGraph),
    Deferred {
        load: Box<dyn Fn() -> Result<TopologyGraph, QueryError> + Send + Sync>,
        value: std::sync::OnceLock<Result<TopologyGraph, String>>,
    },
}

impl QueryComparison {
    fn get(&self) -> Result<&TopologyGraph, QueryError> {
        match self {
            Self::Ready(graph) => Ok(graph),
            Self::Deferred { load, value } => value
                .get_or_init(|| load().map_err(|error| error.to_string()))
                .as_ref()
                .map_err(|message| QueryError::Unavailable(message.clone())),
        }
    }
}

type CompatibilityLoader = dyn Fn(&TopologyGraph) -> Result<crate::compatibility::CompatibilityCatalog, QueryServiceError>
    + Send
    + Sync;

enum QueryCompatibility {
    Ready(crate::compatibility::CompatibilityCatalog),
    Deferred {
        load: Box<CompatibilityLoader>,
        value: std::sync::OnceLock<
            Result<crate::compatibility::CompatibilityCatalog, QueryServiceError>,
        >,
    },
}

impl QueryCompatibility {
    fn get(
        &self,
        graph: &TopologyGraph,
    ) -> Result<&crate::compatibility::CompatibilityCatalog, QueryServiceError> {
        match self {
            Self::Ready(catalog) => Ok(catalog),
            Self::Deferred { load, value } => value
                .get_or_init(|| load(graph))
                .as_ref()
                .map_err(Clone::clone),
        }
    }
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct WalkCounters {
    prune_selection: Option<entmoot_contracts::query::SelectionReceipt>,
    pruned_edges: usize,
    evidence_filtered_edges: usize,
    admission_claims_examined: usize,
    expanded_nodes: usize,
    examined_edges: usize,
    expansion_budget_exhausted: bool,
    fanout_truncated: bool,
    edge_budget_exhausted: bool,
    examined_shortcuts: usize,
    index_examined_edges: usize,
    index_micros: u64,
}

impl QueryEngine {
    pub fn new(graph: TopologyGraph, store_path: impl Into<PathBuf>) -> Self {
        Self::with_options(graph, store_path, QueryEngineOptions::default())
    }

    pub fn with_options(
        graph: TopologyGraph,
        store_path: impl Into<PathBuf>,
        options: QueryEngineOptions,
    ) -> Self {
        let store_path = store_path.into();
        let backend = std::sync::Arc::new(entmoot_store::read::SqliteReader::new(&store_path));
        Self::with_backend(graph, store_path, options, backend.clone(), Some(backend))
    }

    /// Construct a query engine over a read-only backend. Durable continuation
    /// writes require a separate explicit capability. Backend selection is fixed
    /// at construction so cached generations and traversals cannot cross stores.
    pub fn with_backend(
        graph: TopologyGraph,
        store_path: impl Into<PathBuf>,
        options: QueryEngineOptions,
        reader: std::sync::Arc<dyn crate::RepositoryReader>,
        checkpoints: Option<std::sync::Arc<dyn crate::QueryCheckpoints>>,
    ) -> Self {
        let indexes = QueryIndexes::new(&graph, options.substring_postings_budget);
        Self {
            graph: std::sync::Arc::new(graph),
            deferred_navigation: None,
            publication_digest: None,
            lifecycle_lease: None,
            walk_cache: options.cache_walk_discovery.then(Default::default),
            shortcuts: std::sync::Arc::new(walk_cache::BoundedCache::new(
                options.shortcut_cache_entries,
                options.shortcut_cache_bytes,
            )),
            shortcut_build: Default::default(),
            continuations: std::sync::Arc::new(continuation::Searches::new(options)),
            options,
            reader,
            checkpoints,
            store_path: store_path.into(),
            indexes: std::sync::Arc::new(indexes),
            output_generation: Default::default(),
            source_store: None,
            compare_graph: None,
            compatibility: None,
        }
    }

    /// Open a retained publication without hydrating nodes, relations or provider
    /// evidence. Navigation is projected once, on demand; source reads use only
    /// the verified source manifest. Failure never becomes an empty graph.
    pub fn from_retained(
        store: impl Into<PathBuf>,
        options: QueryEngineOptions,
    ) -> Result<Self, QueryError> {
        let store = store.into();
        let repository = entmoot_store::SqliteKnowledgeRepository::open_read_snapshot(&store)?;
        let metadata = repository.inventory_metadata()?.ok_or_else(|| {
            QueryError::Unavailable("publication has no retained inventory".into())
        })?;
        let digest = repository.inventory_export_digest()?.ok_or_else(|| {
            QueryError::Unavailable("publication has no inventory commitment".into())
        })?;
        Ok(
            Self::with_retained_metadata(metadata, digest, store, options)
                .with_lifecycle_lease(repository.lifecycle_lease()),
        )
    }

    /// Construct from metadata verified in the caller's pinned store read.
    pub fn with_retained_metadata(
        metadata: entmoot_store::read::InventoryMetadata,
        digest: String,
        store: impl Into<PathBuf>,
        options: QueryEngineOptions,
    ) -> Self {
        let mut header = TopologyGraph::new(metadata.root);
        header.snapshot = metadata.snapshot;
        header.policy_ids = metadata.policy_ids;
        header.snapshot_files = metadata.snapshot_files.into();
        let mut engine = Self::with_options(header, store, options);
        engine.deferred_navigation = Some(Default::default());
        engine.publication_digest = Some(digest);
        engine
    }

    /// Identity available without allocating a navigation projection.
    pub fn publication_snapshot(&self) -> &entmoot_contracts::SourceSnapshot {
        &self.graph.snapshot
    }

    /// Local source wiring, not an assertion that the checkout still matches.
    pub fn publication_root(&self) -> &str {
        &self.graph.root
    }

    fn reset_navigation(&mut self) {
        if self.deferred_navigation.is_some() {
            self.deferred_navigation = Some(Default::default());
        }
    }

    fn navigation(&self) -> Result<&Self, QueryError> {
        let Some(deferred) = &self.deferred_navigation else {
            return Ok(self);
        };
        deferred
            .get_or_init(|| {
                let graph = self
                    .repository()
                    .and_then(|repository| {
                        repository.load_inventory(true)?.ok_or_else(|| {
                            QueryError::Unavailable(
                                "retained navigation inventory is missing".into(),
                            )
                        })
                    })
                    .map_err(|e| e.to_string())?;
                if graph.snapshot != self.graph.snapshot
                    || graph.policy_ids != self.graph.policy_ids
                {
                    return Err("publication inventory changed; reopen the query engine".into());
                }
                let mut projected = self.clone();
                projected.indexes = std::sync::Arc::new(QueryIndexes::new(
                    &graph,
                    self.options.substring_postings_budget,
                ));
                projected.graph = std::sync::Arc::new(graph);
                projected.deferred_navigation = None;
                Ok(Box::new(projected))
            })
            .as_ref()
            .map(Box::as_ref)
            .map_err(|message| QueryError::Unavailable(message.clone()))
    }

    pub fn with_lifecycle_lease(
        mut self,
        lease: Option<crate::storage_lease::StorageLease>,
    ) -> Self {
        self.reset_navigation();
        self.lifecycle_lease = lease;
        self
    }

    /// Configure authoritative source fallback without coupling queries to a
    /// checkout path or source-system implementation.
    pub fn with_source_store(mut self, source_store: std::sync::Arc<dyn SourceStore>) -> Self {
        self.reset_navigation();
        self.source_store = Some(source_store);
        self
    }

    /// Explicitly materialize the navigation projection. Cold evidence may remain
    /// unloaded; complete export/comparison uses `complete_graph` instead.
    pub fn graph(&self) -> Result<&TopologyGraph, QueryError> {
        Ok(&self.navigation()?.graph)
    }

    /// Rehydrate complete evidence in one pinned read transaction. Comparison
    /// and complete-graph validation must never interpret unloaded rows as absent.
    pub fn complete_graph(&self) -> Result<TopologyGraph, QueryError> {
        let repository = self.repository()?;
        let graph = match repository.load_inventory(false)? {
            Some(graph) => graph,
            None => repository
                .load()?
                .into_inventory(crate::knowledge::inventory_skeleton(&self.graph)),
        };
        if graph.snapshot != self.graph.snapshot || graph.policy_ids != self.graph.policy_ids {
            return Err(QueryError::InvalidInput(
                "publication inventory changed; reopen the query engine".into(),
            ));
        }
        Ok(graph)
    }

    /// Configure independent publications for read-only package judgments.
    pub fn with_compatibility(
        mut self,
        catalog: crate::compatibility::CompatibilityCatalog,
    ) -> Self {
        self.reset_navigation();
        self.compatibility = Some(std::sync::Arc::new(QueryCompatibility::Ready(catalog)));
        self
    }

    /// Defer native package evidence until first use. Clones share initialization
    /// and its exact result; reopening a publication retries unavailable evidence.
    pub fn with_compatibility_loader(
        mut self,
        load: impl Fn(
                &TopologyGraph,
            ) -> Result<crate::compatibility::CompatibilityCatalog, QueryServiceError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.reset_navigation();
        self.compatibility = Some(std::sync::Arc::new(QueryCompatibility::Deferred {
            load: Box::new(load),
            value: Default::default(),
        }));
        self
    }

    fn compatibility_catalog(
        &self,
    ) -> Result<&crate::compatibility::CompatibilityCatalog, QueryServiceError> {
        self.compatibility
            .as_ref()
            .ok_or_else(|| {
                QueryServiceError::Unavailable(
                    "compatibility publications are not configured".into(),
                )
            })?
            .get(&self.graph)
    }

    pub fn compatibility_request(
        &self,
    ) -> Result<entmoot_contracts::query::CompatibilityRequest, QueryServiceError> {
        Ok(self.compatibility_catalog()?.request())
    }

    pub fn find(&self, request: FindQuery) -> Result<QueryResult, QueryError> {
        if self.deferred_navigation.is_some() {
            if let Some(result) = self.retained_subject_find(&request)? {
                return Ok(result);
            }
            return self.navigation()?.find(request);
        }
        let value = match request {
            FindQuery::CallBoundary { request } => self.call_boundaries(&request),
            FindQuery::Initializer {
                integer,
                path,
                limit,
            } => self.find_initializer(&integer, path.as_deref(), limit),
            FindQuery::Subject {
                selector,
                query,
                path,
                line,
                limit,
            } => self.resolve_code_subject(
                query.as_deref(),
                path.as_deref(),
                line,
                limit,
                selector.as_ref(),
            ),
            FindQuery::Location { path, line, column } => {
                self.resolve_code_location(&path, line, column)
            }
            FindQuery::Finding { query, path, limit } => {
                self.list_findings(query.as_deref(), path.as_deref(), limit)
            }
            FindQuery::EvidenceRole { role, path, limit } => {
                self.list_evidence_roles(role.as_deref(), path.as_deref(), limit)
            }
            FindQuery::ChangedSubject { .. }
            | FindQuery::UnresolvedBoundary { .. }
            | FindQuery::ExternalBoundary { .. }
            | FindQuery::ConflictedRelation { .. }
            | FindQuery::StaleEvidence { .. }
            | FindQuery::ProducerGap { .. }
            | FindQuery::HighFanInOrOut { .. } => self.find_facet(request),
        }?;
        self.finish_result(value)
    }

    pub fn inspect(&self, request: InspectQuery) -> Result<QueryResult, QueryError> {
        if self.deferred_navigation.is_some() {
            if let Some((_, result)) = self.retained_call_inspect(&request)? {
                return Ok(result);
            }
            return self.navigation()?.inspect(request);
        }
        let value = match request {
            InspectQuery::Subject { id, limit } => self.get_subject_context(&id, limit),
            InspectQuery::Evidence { id } => self.explain_evidence(&id),
            InspectQuery::Trace { id, offset, limit } => self.get_finding_trace(&id, offset, limit),
            InspectQuery::Proposition { id } => self.proposition(&id),
            InspectQuery::RelationEdge { id } => self.relation_claims(&id),
            InspectQuery::Unresolved { id, after, limit } => {
                self.unresolved_page(&id, after.as_ref(), limit)
            }
            InspectQuery::Callers { id, after, limit } => {
                self.call_neighborhood(&id, CallDirection::Callers, after.as_ref(), limit)
            }
            InspectQuery::Callees { id, after, limit } => {
                self.call_neighborhood(&id, CallDirection::Callees, after.as_ref(), limit)
            }
        }?;
        self.finish_result(value)
    }

    pub fn traverse(&self, request: TraverseQuery) -> Result<QueryResult, QueryError> {
        if self.deferred_navigation.is_some() {
            return self.navigation()?.traverse(request);
        }
        if request.resumable() {
            return self.resume_traverse(request);
        }
        if request.continuation().is_some() {
            return Err(QueryError::InvalidInput(
                "continuation requires resumable=true".into(),
            ));
        }
        let value = match request {
            request @ TraverseQuery::Neighborhood { .. } => {
                self.bounded_neighborhood_query(&request)
            }
            TraverseQuery::Path {
                evidence_filter,
                prune,
                start,
                target,
                predicates,
                budget,
                ..
            } => self.bounded_walk_filtered(
                &start,
                &target,
                &predicates,
                budget,
                evidence_filter,
                &prune,
            ),
        }?;
        self.finish_result(value)
    }

    pub fn read(&self, request: ReadQuery) -> Result<QueryResult, QueryError> {
        let value = match request {
            ReadQuery::Source {
                path,
                start_line,
                end_line,
                context_lines,
                max_bytes,
            } => self.read_source_span(&path, start_line, end_line, context_lines, max_bytes),
        }?;
        self.finish_result(value)
    }

    fn finish_result(&self, mut value: Value) -> Result<QueryResult, QueryError> {
        let empty_receipt = entmoot_contracts::query::QueryEvidenceReceipt::inventory_only(
            self.graph.snapshot.clone(),
        );
        value["receipt"] = serde_json::to_value(empty_receipt)?;
        let mut result = QueryResult::from_value(value)?;
        result.receipt = receipt::build_query_receipt(&self.graph, &result, || self.repository())?;
        Ok(result)
    }

    fn repository(&self) -> Result<Box<dyn KnowledgeRead>, QueryError> {
        let repository = self.reader.open_read()?;
        if let Some(expected) = &self.publication_digest {
            if repository.inventory_export_digest()?.as_ref() != Some(expected) {
                return Err(QueryError::Unavailable(
                    "publication changed; reopen the query engine".into(),
                ));
            }
        }
        Ok(repository)
    }

    fn relation_claims(&self, edge: &str) -> Result<Value, QueryError> {
        let mut value = knowledge::relation_claims_result(&self.repository()?, edge)?;
        value["snapshot"] = json!(self.graph.snapshot);
        Ok(value)
    }

    fn proposition(&self, proposition: &str) -> Result<Value, QueryError> {
        let mut value = knowledge::proposition_result(&self.repository()?, proposition)?;
        value["snapshot"] = json!(self.graph.snapshot);
        Ok(value)
    }

    fn call_neighborhood(
        &self,
        subject: &str,
        direction: CallDirection,
        after: Option<&CallPageCursor>,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        self.call_neighborhood_page(subject, direction, after, requested_limit, None)
    }

    fn call_neighborhood_page(
        &self,
        subject: &str,
        direction: CallDirection,
        after: Option<&CallPageCursor>,
        requested_limit: Option<usize>,
        retained: Option<entmoot_store::read::RelationReadPage>,
    ) -> Result<Value, QueryError> {
        let limit = requested_limit.unwrap_or(50);
        if !(1..=1_000).contains(&limit) {
            return Err(QueryError::InvalidInput(
                "call-neighborhood limit must be in 1..=1000".into(),
            ));
        }
        let repository = self.repository()?;
        let visibility_cursor = repository.current_visibility_cursor()?;
        if let Some(cursor) = after {
            RelationEdgeId::parse(&cursor.relation_edge_id)
                .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
            if cursor.revision != self.graph.snapshot.revision
                || cursor.snapshot_digest != self.graph.snapshot.snapshot_digest
                || cursor.visibility_cursor != visibility_cursor
            {
                return Err(QueryError::InvalidInput(
                    "call-neighborhood cursor does not match the current snapshot and visibility state"
                        .into(),
                ));
            }
        }
        let candidates = self.exact_subject_nodes(subject.trim());
        if candidates.len() != 1 {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "inspect",
                "view": "calls",
                "snapshot": self.graph.snapshot,
                "status": if candidates.is_empty() { "unknown" } else { "conflict" },
                "subject": subject,
                "direction": direction,
                "items": [], "unresolved_relations": [], "next_cursor": null,
                "visibility_cursor": visibility_cursor,
                "bounds": ResultBounds::complete(0),
                "unresolved_bounds": ResultBounds::complete(0),
                "gaps": if candidates.is_empty() { vec!["subject_not_resolved"] } else { vec!["subject_is_ambiguous"] },
            }));
        }
        let keys = node_addresses(&self.graph, candidates[0])
            .iter()
            .map(address_identity_key)
            .collect::<HashSet<_>>();
        let query_generation_digest = entmoot_contracts::ContentDigest::from_trusted(
            entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
                self.publication_generation_digest(&repository)?,
                candidates[0].id.as_str(),
                direction,
            ))?),
        );
        if after.is_some_and(|cursor| cursor.query_generation_digest != query_generation_digest) {
            return Err(QueryError::InvalidInput(
                "call-neighborhood cursor does not match subject, direction or publication state"
                    .into(),
            ));
        }
        let forward = direction == CallDirection::Callees;
        let (page, total, offset) = if let Some(retained) = &retained {
            (
                retained.edges.iter().collect::<Vec<_>>(),
                retained.total,
                retained.offset,
            )
        } else {
            let mut edges: Vec<_> = keys
                .iter()
                .flat_map(|key| {
                    self.indexes
                        .relations
                        .postings(key, forward)
                        .into_iter()
                        .flatten()
                })
                .map(|index| &self.graph.relation_edges[*index])
                .filter(|edge| edge.predicate == RelationKind::Calls)
                .collect();
            edges.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
            edges.dedup_by(|left, right| left.id == right.id);
            let total = edges.len();
            let offset = after.map_or(0, |cursor| {
                edges.partition_point(|edge| edge.id.as_str() <= cursor.relation_edge_id.as_str())
            });
            let page = edges
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            (page, total, offset)
        };
        let mut items = Vec::new();
        let claim_batches = repository.relation_claims_for_edges(&page)?;
        for (edge, mut claims) in page.iter().zip(claim_batches) {
            claims.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
            claims.dedup_by(|left, right| left.id == right.id);
            let opposite = match direction {
                CallDirection::Callers => &edge.subject,
                CallDirection::Callees => &edge.object,
            };
            items.push(json!({"relation": edge, "opposite": opposite, "claims": claims}));
        }
        let unresolved_page = if direction == CallDirection::Callees {
            Some(self.unresolved_page_from_repository(&repository, subject, None, Some(limit))?)
        } else {
            None
        };
        let unresolved = unresolved_page
            .as_ref()
            .map_or_else(|| json!([]), |page| page["items"].clone());
        let unresolved_bounds: ResultBounds = unresolved_page.as_ref().map_or_else(
            || Ok(ResultBounds::complete(0)),
            |page| serde_json::from_value(page["bounds"].clone()),
        )?;
        let unresolved_continuation = unresolved_page
            .as_ref()
            .and_then(|page| page.get("next_cursor").filter(|cursor| !cursor.is_null()))
            .map(|cursor| {
                serde_json::from_value(cursor.clone()).map(|after| InspectQuery::Unresolved {
                    id: candidates[0].id.clone(),
                    after: Some(after),
                    limit: Some(limit),
                })
            })
            .transpose()?;
        let returned = page.len();
        let truncated = offset.saturating_add(returned) < total;
        let next_cursor = truncated
            .then(|| {
                page.last().map(|edge| CallPageCursor {
                    query_generation_digest: query_generation_digest.clone(),
                    relation_edge_id: edge.id.as_str().to_owned(),
                    visibility_cursor,
                    revision: self.graph.snapshot.revision.clone(),
                    snapshot_digest: self.graph.snapshot.snapshot_digest.clone(),
                })
            })
            .flatten();
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "inspect", "view": "calls",
            "snapshot": self.graph.snapshot,
            "status": if truncated || unresolved_bounds.truncated { "truncated" } else if returned > 0 || unresolved_bounds.total_matches > 0 { "known" } else { "unknown" },
            "subject": subject, "direction": direction, "items": items,
            "unresolved_relations": unresolved, "next_cursor": next_cursor,
            "unresolved_continuation": unresolved_continuation,
            "visibility_cursor": visibility_cursor,
            "bounds": {"offset":offset,"limit":limit,"total_matches":total,"truncated":truncated},
            "unresolved_bounds": unresolved_bounds,
            "gaps": ["repository_call_graph_completeness_not_established", "dependency_call_graph_completeness_not_established", "native_library_call_graph_completeness_not_established"],
        }))
    }

    /// Find bounded directed paths between two exact inventory subjects using
    /// meet-in-the-middle search. Backward expansion follows incoming edges; it
    /// never reverses the stored relation semantics.
    #[cfg(test)]
    fn bounded_walk(
        &self,
        start: &str,
        target: &str,
        predicates: &[String],
        budget: BoundedWalkBudget,
    ) -> Result<Value, QueryError> {
        self.bounded_walk_filtered(
            start,
            target,
            predicates,
            budget,
            entmoot_contracts::query::HopEvidenceFilter::Any,
            &Default::default(),
        )
    }

    fn bounded_walk_filtered(
        &self,
        start: &str,
        target: &str,
        predicates: &[String],
        budget: BoundedWalkBudget,
        evidence: entmoot_contracts::query::HopEvidenceFilter,
        prune: &entmoot_contracts::query::TraversalPrune,
    ) -> Result<Value, QueryError> {
        let started = std::time::Instant::now();
        let admission = admission::EdgeAdmission::new(self, prune, evidence)?;
        let budget = validate_walk_budget(budget)?;
        let allowed_predicates = parse_walk_predicates(predicates)?;
        let start_keys = self.walk_endpoint_keys(start);
        let target_keys = self.walk_endpoint_keys(target);
        if start_keys.is_empty() || target_keys.is_empty() {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "traverse",
            "view": "path",
            "snapshot": self.graph.snapshot,
                "status": "unknown",
                "start": start,
                "target": target,
                "paths": [],
                "bounds": ResultBounds::complete(0),
                "gaps": if start_keys.is_empty() { vec!["start_not_materialized"] } else { vec!["target_not_materialized"] },
                "search_receipt": {
                    "expanded_nodes": 0,
                    "examined_edges": 0,
                    "elapsed_micros": u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                    "budget": walk_budget_json(budget),
                }
            }));
        }

        let repository = self.repository()?;
        let visibility = if evidence == entmoot_contracts::query::HopEvidenceFilter::ResolvedOnly {
            Some(repository.read_state_digest()?)
        } else {
            None
        };
        let key = serde_json::to_string(&(
            start, target, predicates, budget, evidence, prune, visibility,
        ))?;
        let cached = self.walk_cache.as_ref().and_then(|cache| cache.get(&key));
        let reused_discovery = cached.is_some();
        let discovery = if let Some(cached) = cached {
            cached
        } else {
            let mut search = walk::PathSearch::with_admission(
                self,
                &start_keys,
                &target_keys,
                allowed_predicates,
                start == target,
                admission,
            );
            search.advance(self, budget)?;
            let value = std::sync::Arc::new(search.discovery(self, budget));
            if let Some(cache) = &self.walk_cache {
                cache.insert(key, value.clone());
            }
            value
        };
        let mut result = self.walk_result(
            (start, target),
            budget,
            &discovery,
            &repository,
            reused_discovery,
        )?;
        result["search_receipt"]["elapsed_micros"] =
            json!(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        Ok(result)
    }

    fn walk_result(
        &self,
        (start, target): (&str, &str),
        budget: BoundedWalkBudget,
        discovery: &walk_cache::WalkDiscovery,
        repository: &impl KnowledgeRead,
        reused_discovery: bool,
    ) -> Result<Value, QueryError> {
        let counters = &discovery.counters;
        let total_paths = discovery.total_paths;
        let incompatible_meets = discovery.incompatible_meets;
        let evidence_started = std::time::Instant::now();
        let mut paths = Vec::new();
        for (hop_count, meet, edge_indexes) in &discovery.candidates {
            paths.push(self.walk_path_receipt(
                repository,
                *hop_count,
                meet,
                edge_indexes,
                budget.max_evidence_per_hop,
            )?);
        }
        let evidence_micros =
            u64::try_from(evidence_started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let reconstructed_edges: usize = discovery
            .candidates
            .iter()
            .map(|(_, _, edges)| edges.len())
            .sum();
        let evidence_claims_examined: usize = paths
            .iter()
            .flat_map(|path| path["hops"].as_array().into_iter().flatten())
            .filter_map(|hop| hop["relation_claim_count"].as_u64())
            .filter_map(|count| usize::try_from(count).ok())
            .sum();
        tracing::debug!(
            reconstructed_edges,
            evidence_claims_examined,
            evidence_micros,
            "hydrated original path witnesses"
        );
        let truncated = counters.expansion_budget_exhausted
            || counters.edge_budget_exhausted
            || counters.fanout_truncated
            || total_paths > budget.max_results;
        let found_path = !paths.is_empty();
        let mut gaps = Vec::new();
        if !found_path {
            gaps.push("no_path_within_budget");
        }
        if incompatible_meets > 0 {
            gaps.push("scope_composition_not_established");
        }
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "traverse",
            "view": "path",
            "snapshot": self.graph.snapshot,
            "status": if truncated { "truncated" } else if !found_path { "unknown" } else { "known" },
            "start": start,
            "target": target,
            "paths": paths,
            "bounds": ResultBounds { offset: 0, limit: budget.max_results, total_matches: total_paths, truncated },
            "gaps": gaps,
            "search_receipt": {
                "prune_selection": counters.prune_selection,
                "pruned_edges": if reused_discovery { 0 } else { counters.pruned_edges },
                "evidence_filtered_edges": if reused_discovery { 0 } else { counters.evidence_filtered_edges },
                "admission_claims_examined": if reused_discovery { 0 } else { counters.admission_claims_examined },
                "expanded_nodes": if reused_discovery { 0 } else { counters.expanded_nodes },
                "examined_edges": if reused_discovery { 0 } else { counters.examined_edges },
                "examined_shortcuts": if reused_discovery { 0 } else { counters.examined_shortcuts },
                "index_examined_edges": if reused_discovery { 0 } else { counters.index_examined_edges },
                "index_micros": if reused_discovery { 0 } else { counters.index_micros },
                "evidence_micros": evidence_micros,
                "reconstructed_edges": reconstructed_edges,
                "evidence_claims_examined": evidence_claims_examined,
                "reused_discovery": reused_discovery,
                "elapsed_micros": 0,
                "fanout_truncated": counters.fanout_truncated,
                "expansion_budget_exhausted": counters.expansion_budget_exhausted,
                "edge_budget_exhausted": counters.edge_budget_exhausted,
                "candidate_meet_count": total_paths,
                "budget": walk_budget_json(budget),
            }
        }))
    }

    fn walk_endpoint_keys(&self, subject: &str) -> Vec<String> {
        let mut keys: Vec<_> = self
            .exact_subject_nodes(subject)
            .into_iter()
            .flat_map(|node| node_addresses(&self.graph, node))
            .map(|address| address_identity_key(&address))
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    fn walk_path_receipt(
        &self,
        repository: &impl KnowledgeRead,
        hop_count: usize,
        meet: &str,
        edge_indexes: &[usize],
        detail_limit: usize,
    ) -> Result<Value, QueryError> {
        let mut evidence_kinds = HashSet::new();
        let mut candidate_edges = Vec::new();
        let mut conflicted_edges = Vec::new();
        let mut stale_edges = Vec::new();
        let mut unclaimed_edges = Vec::new();
        let mut scope_domains = HashSet::new();
        let mut context_ids = HashSet::new();
        let mut hops = Vec::new();
        let edges: Vec<_> = edge_indexes
            .iter()
            .map(|&index| &self.graph.relation_edges[index])
            .collect();
        let claim_batches = repository.relation_claims_for_edges(&edges)?;
        let explicit_conflicts = conflicts::retained_conflicts(&self.graph, repository)?;
        for (edge, relation_claims) in edges.into_iter().zip(claim_batches) {
            if explicit_conflicts.contains(edge.id.as_str()) {
                conflicted_edges.push(edge.id.as_str());
            }
            scope_domains.insert(edge.scope.domain.as_str());
            if let Some(context_id) = edge.scope.context_id.as_deref() {
                context_ids.insert(context_id);
            }
            let mut states = Vec::new();
            let active_claims: Vec<_> = relation_claims
                .iter()
                .filter(|claim| claim.state == RelationClaimState::Active)
                .collect();
            for claim in &active_claims {
                // A review note (even one carrying a typed state), or support
                // for another occurrence/build/snapshot, cannot qualify this
                // occurrence. Keep all claims in the evidence counts/IDs.
                if let Some(state) = applicable_resolution_state(claim, edge) {
                    let name = resolution_state_name(state);
                    evidence_kinds.insert(name);
                    states.push(name);
                    match state {
                        ResolutionState::Candidate | ResolutionState::Unknown => {
                            candidate_edges.push(edge.id.as_str())
                        }
                        ResolutionState::Refuted => conflicted_edges.push(edge.id.as_str()),
                        ResolutionState::Stale => stale_edges.push(edge.id.as_str()),
                        ResolutionState::Structural if edge.predicate == RelationKind::Calls => {
                            candidate_edges.push(edge.id.as_str())
                        }
                        _ => {}
                    }
                }
            }
            states.sort_unstable();
            states.dedup();
            if states.is_empty() {
                candidate_edges.push(edge.id.as_str());
                unclaimed_edges.push(edge.id.as_str());
            }
            // Exact endpoint attachment only; no inheritance of file-level roles.
            // Index slices avoid scanning or hydrating all evidence per hop.
            let keys = [
                address_identity_key(&edge.subject),
                address_identity_key(&edge.object),
            ];
            let mut assertions = std::collections::BTreeSet::new();
            let mut unresolved = std::collections::BTreeSet::new();
            for key in &keys {
                if let Some(indexes) = self.indexes.assertions_by_address.get(key) {
                    assertions.extend(
                        indexes
                            .iter()
                            .map(|index| (&self.graph.assertions[*index].assertion_id, *index)),
                    );
                }
                if let Some(indexes) = self.indexes.unresolved_calls_by_address.get(key) {
                    unresolved.extend(indexes.iter().map(|index| {
                        (self.graph.unresolved_relations[*index].id.as_str(), *index)
                    }));
                }
            }
            hops.push(json!({
                "relation_edge": edge,
                "active_resolution_states": states,
                "relation_claim_count": relation_claims.len(),
                "active_claim_count": active_claims.len(),
                "relation_claim_ids": relation_claims.iter().take(detail_limit).map(|claim| claim.id.as_str()).collect::<Vec<_>>(),
                "relation_claims_truncated": relation_claims.len() > detail_limit,
                "endpoint_assertions": assertions.iter().take(detail_limit).map(|(_, index)| &self.graph.assertions[*index]).collect::<Vec<_>>(),
                "endpoint_assertion_bounds": ResultBounds::page(detail_limit, assertions.len().min(detail_limit), assertions.len()),
                "unresolved_calls": unresolved.iter().take(detail_limit).map(|(_, index)| &self.graph.unresolved_relations[*index]).collect::<Vec<_>>(),
                "unresolved_call_bounds": ResultBounds::page(detail_limit, unresolved.len().min(detail_limit), unresolved.len()),
            }));
        }
        let mut evidence_kinds: Vec<_> = evidence_kinds.into_iter().collect();
        evidence_kinds.sort_unstable();
        let mut scope_domains: Vec<_> = scope_domains.into_iter().collect();
        scope_domains.sort_unstable();
        let mut context_ids: Vec<_> = context_ids.into_iter().collect();
        context_ids.sort_unstable();
        candidate_edges.sort_unstable();
        candidate_edges.dedup();
        conflicted_edges.sort_unstable();
        conflicted_edges.dedup();
        stale_edges.sort_unstable();
        stale_edges.dedup();
        unclaimed_edges.sort_unstable();
        unclaimed_edges.dedup();
        let classification = if !conflicted_edges.is_empty() {
            "conflicted"
        } else if !candidate_edges.is_empty()
            || !stale_edges.is_empty()
            || !unclaimed_edges.is_empty()
            || context_ids.len() > 1
            || scope_domains.len() > 1
        {
            "candidate"
        } else {
            "supported"
        };
        Ok(json!({
            "classification": classification,
            "hop_count": hop_count,
            "meet_address_key": meet,
            "evidence_kinds": evidence_kinds,
            "scope_domains": scope_domains,
            "context_ids": context_ids,
            "candidate_relation_edge_ids": candidate_edges,
            "conflicted_relation_edge_ids": conflicted_edges,
            "stale_relation_edge_ids": stale_edges,
            "unclaimed_relation_edge_ids": unclaimed_edges,
            "hops": hops,
        }))
    }

    /// Resolve a familiar path, symbol, display reference, or durable id to a
    /// bounded set of inventory subjects. Multiple results remain explicit.
    fn find_initializer(
        &self,
        integer: &str,
        path: Option<&str>,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        if entmoot_contracts::decimal_integer(integer).as_deref() != Some(integer) {
            return Err(QueryError::InvalidInput(
                "initializer requires canonical plain decimal integer syntax (up to 256 bytes)"
                    .into(),
            ));
        }
        let path = path
            .map(entmoot_contracts::normalize_repo_path)
            .transpose()
            .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
        let limit = bounded_limit(requested_limit);
        let mut matches: Vec<_> = self
            .indexes
            .nodes_by_initializer_integer
            .get(integer)
            .into_iter()
            .flatten()
            .map(|i| &self.graph.nodes[*i])
            .filter(|node| path.as_ref().is_none_or(|p| node.path.as_ref() == Some(p)))
            .collect();
        matches.sort_by(|a, b| a.id.cmp(&b.id));
        let total = matches.len();
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA, "operation": "find", "view": "subject",
            "snapshot": self.graph.snapshot,
            "status": if total > limit { "truncated" } else if total == 0 { "unknown" } else { "known" },
            "items": matches.into_iter().take(limit).map(|n| subject_candidate(n, 4)).collect::<Vec<_>>(),
            "gaps": ["declaration_inventory_is_partial", "initializer_syntax_is_not_runtime_value_or_property_equivalence"],
            "bounds": ResultBounds::page(limit, limit.min(total), total),
        }))
    }

    fn resolve_code_subject(
        &self,
        query: Option<&str>,
        path: Option<&str>,
        line: Option<usize>,
        requested_limit: Option<usize>,
        selector: Option<&entmoot_contracts::query::SubjectSelector>,
    ) -> Result<Value, QueryError> {
        let query = query.map(str::trim).filter(|value| !value.is_empty());
        let path = path
            .map(|value| value.trim().replace('\\', "/"))
            .filter(|value| !value.is_empty());
        if query.is_none() && path.is_none() && selector.is_none() {
            return Err(QueryError::InvalidInput(
                "query, path or selector is required".into(),
            ));
        }
        let path_ok = |node: &TopologyNode| {
            path.as_deref()
                .is_none_or(|expected| node.path.as_deref() == Some(expected))
        };
        let mut scored: HashMap<usize, u8> = HashMap::new();
        match (&query, &path) {
            (None, Some(expected_path)) => {
                for &index in self
                    .indexes
                    .nodes_by_path
                    .get(expected_path.as_str())
                    .into_iter()
                    .flatten()
                {
                    scored.insert(index, 1);
                }
            }
            (Some(raw), _) => {
                let digest_selectors = structural_hash_exact_selectors(raw);
                let exact_only = !digest_selectors.is_empty();
                let exact_keys: Vec<&str> = if exact_only {
                    digest_selectors.iter().map(String::as_str).collect()
                } else {
                    vec![*raw]
                };
                for key in exact_keys {
                    for &index in self
                        .indexes
                        .nodes_by_exact_selector
                        .get(key)
                        .into_iter()
                        .flatten()
                    {
                        let node = &self.graph.nodes[index];
                        if path_ok(node) {
                            scored.insert(index, 4);
                        }
                    }
                }
                if !exact_only {
                    let lower = raw.to_lowercase();
                    for &index in self
                        .indexes
                        .nodes_by_label_or_fqn
                        .get(lower.as_str())
                        .into_iter()
                        .flatten()
                    {
                        let node = &self.graph.nodes[index];
                        if path_ok(node) {
                            scored.entry(index).or_insert(3);
                        }
                    }
                    let substring_indexes: Box<dyn Iterator<Item = usize>> =
                        if let Some(expected_path) = path.as_deref() {
                            Box::new(
                                self.indexes
                                    .nodes_by_path
                                    .get(expected_path)
                                    .into_iter()
                                    .flatten()
                                    .copied(),
                            )
                        } else {
                            Box::new(self.indexes.substring.candidates(&lower))
                        };
                    for index in substring_indexes {
                        if scored.contains_key(&index) {
                            continue;
                        }
                        let node = &self.graph.nodes[index];
                        if !path_ok(node) {
                            continue;
                        }
                        if searchable_node_text(node).contains(&lower) {
                            scored.insert(index, 2);
                        }
                    }
                }
            }
            (None, None) => scored.extend((0..self.graph.nodes.len()).map(|i| (i, 1))),
        }
        let mut search = subject_search::SubjectSearch::new(
            query,
            path.as_deref(),
            line,
            requested_limit,
            selector,
        )?;
        for index in scored.keys() {
            search.consider(&self.graph.nodes[*index]);
        }
        Ok(search.finish(&self.graph.snapshot))
    }

    /// Resolve an exact source location to the most specific represented site,
    /// its enclosing callable, and its file. Hierarchical matches are not
    /// reported as ambiguity; conflict means multiple peer sites overlap.
    fn resolve_code_location(
        &self,
        path: &str,
        line: usize,
        column: Option<usize>,
    ) -> Result<Value, QueryError> {
        if line == 0 || column == Some(0) {
            return Err(QueryError::InvalidInput(
                "line and column are one-based".into(),
            ));
        }
        let path = path.trim().replace('\\', "/");
        if path.is_empty() {
            return Err(QueryError::InvalidInput("path is required".into()));
        }
        let Some(source_file) = self.read_source_file(&path)? else {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "find",
            "view": "location",
                "snapshot": self.graph.snapshot,
                "status": "unknown",
                "path": path,
                "line": line,
                "column": column,
                "selected": [],
                "gaps": [if self.source_store.is_some() { "path_not_in_snapshot" } else { "source_store_unavailable" }],
                "bounds": ResultBounds::complete(0),
            }));
        };
        let source = std::str::from_utf8(&source_file.bytes)
            .map_err(|_| QueryError::UnsupportedSourceEncoding(path.clone()))?;
        let Some((query_start, query_end)) = source_query_span(source, line, column) else {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "find",
            "view": "location",
                "snapshot": self.graph.snapshot,
                "status": "unknown",
                "path": path,
                "line": line,
                "column": column,
                "selected": [],
                "gaps": ["location_out_of_range"],
                "bounds": ResultBounds::complete(0),
            }));
        };
        let nodes = self
            .indexes
            .nodes_by_path
            .get(&path)
            .into_iter()
            .flatten()
            .filter_map(|index| self.graph.nodes.get(*index));
        let mut call_sites = Vec::new();
        let mut control_sites = Vec::new();
        let mut callables = Vec::new();
        let mut declarations = Vec::new();
        let mut resources = Vec::new();
        let mut file = None;
        for node in nodes {
            if node.kind == NodeKind::File {
                file = Some(subject_candidate(node, 1));
                continue;
            }
            let Some((start, end)) = node.start_byte.zip(node.end_byte) else {
                continue;
            };
            if start >= query_end || end <= query_start {
                continue;
            }
            let candidate = subject_candidate(node, 4);
            match node.kind {
                NodeKind::CallStub => call_sites.push((end.saturating_sub(start), candidate)),
                NodeKind::ControlSite => control_sites.push((end.saturating_sub(start), candidate)),
                NodeKind::Callable => callables.push((end.saturating_sub(start), candidate)),
                NodeKind::Declaration => declarations.push((end.saturating_sub(start), candidate)),
                NodeKind::Resource => resources.push((end.saturating_sub(start), candidate)),
                _ => {}
            }
        }
        for group in [
            &mut call_sites,
            &mut control_sites,
            &mut callables,
            &mut declarations,
            &mut resources,
        ] {
            group.sort_by(|(left_size, left), (right_size, right)| {
                left_size
                    .cmp(right_size)
                    .then_with(|| left["node_id"].as_str().cmp(&right["node_id"].as_str()))
            });
        }
        let selected_group = if !call_sites.is_empty() {
            &call_sites
        } else if !control_sites.is_empty() {
            &control_sites
        } else if !resources.is_empty() {
            &resources
        } else if !declarations.is_empty() {
            &declarations
        } else {
            &callables
        };
        let selected_size = selected_group.first().map(|(size, _)| *size);
        let selected: Vec<_> = selected_group
            .iter()
            .take_while(|(size, _)| Some(*size) == selected_size)
            .map(|(_, value)| value)
            .collect();
        let status = match selected.len() {
            0 if file.is_some() => "known",
            0 => "unknown",
            1 => "known",
            _ => "conflict",
        };
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "find",
            "view": "location",
            "snapshot": self.graph.snapshot,
            "status": status,
            "path": path,
            "line": line,
            "column": column,
            "content_digest": source_file.content_digest,
            "selected": selected,
            "bounds": ResultBounds::complete(selected.len().max(usize::from(file.is_some()))),
            "call_sites": call_sites.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            "control_sites": control_sites.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            "enclosing_callables": callables.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            "declarations": declarations.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            "resources": resources.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            "file": file,
            "gaps": if status == "unknown" { vec!["no_inventory_subject_at_location"] } else { Vec::<&str>::new() },
        }))
    }

    fn get_subject_context(
        &self,
        subject: &str,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        let subject = subject.trim();
        if subject.is_empty() {
            return Err(QueryError::InvalidInput("subject is required".into()));
        }
        let limit = bounded_limit(requested_limit);
        let candidates = self.exact_subject_nodes(subject);
        if candidates.is_empty() {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "inspect",
            "view": "subject",
                "snapshot": self.graph.snapshot,
                "status": "unknown",
                "subject": subject,
                "items": [],
                "gaps": ["subject_not_resolved"],
                "bounds": ResultBounds::complete(0),
            }));
        }
        if candidates.len() > 1 {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "inspect",
            "view": "subject",
                "snapshot": self.graph.snapshot,
                "status": "conflict",
                "subject": subject,
                "items": candidates.iter().take(limit).map(|node| subject_candidate(node, 4)).collect::<Vec<_>>(),
                "gaps": ["subject_is_ambiguous"],
                "bounds": ResultBounds::page(limit, limit.min(candidates.len()), candidates.len()),
            }));
        }
        let node = candidates[0];
        let structural_edge_indexes = self
            .indexes
            .structural_edges_by_node
            .get(self.graph_node_index(node)?)
            .cloned()
            .unwrap_or_default();
        let mut seen_related = HashSet::new();
        let related_ids: Vec<&str> = structural_edge_indexes
            .iter()
            .map(|index| &self.graph.edges[*index])
            .flat_map(|edge| [edge.from.as_str(), edge.to.as_str()])
            .filter(|candidate| *candidate != node.id && seen_related.insert(*candidate))
            .collect();
        let structural_total = structural_edge_indexes.len();
        let neighbor_total = related_ids.len();
        let neighbors: Vec<_> = related_ids
            .iter()
            .filter_map(|id| self.graph.node(id))
            .take(limit)
            .map(|candidate| subject_candidate(candidate, 1))
            .collect();
        let structural_edges: Vec<_> = structural_edge_indexes
            .iter()
            .filter_map(|index| self.graph.edges.get(*index))
            .take(limit)
            .collect();

        let repository = self.repository()?;
        let mut relations = Vec::new();
        let mut relation_claims = Vec::new();
        let mut unresolved = Vec::new();
        for address in node_addresses(&self.graph, node) {
            for edge in repository.relation_edges_for_subject(&address)? {
                relation_claims
                    .extend(repository.relation_claims_for_relation_edge(edge.id.as_str())?);
                relation_claims.extend(
                    repository.relation_claims_for_proposition(edge.proposition_id.as_str())?,
                );
                relations.push(edge);
            }
            unresolved.extend(repository.unresolved_for_source(&address)?);
        }
        relations.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        relations.dedup_by(|left, right| left.id == right.id);
        relation_claims.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        relation_claims.dedup_by(|left, right| left.id == right.id);
        unresolved.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        unresolved.dedup_by(|left, right| left.id == right.id);

        let mut relation_neighbors = Vec::new();
        let mut unmapped_relation_endpoint = false;
        let node_address_keys: HashSet<_> = node_addresses(&self.graph, node)
            .iter()
            .map(address_identity_key)
            .collect();
        for edge in relations.iter().take(limit) {
            let subject_key = address_identity_key(&edge.subject);
            let (direction, opposite) = if node_address_keys.contains(&subject_key) {
                ("outgoing", &edge.object)
            } else {
                ("incoming", &edge.subject)
            };
            let opposite_key = address_identity_key(opposite);
            let subjects: Vec<_> = self
                .indexes
                .nodes_by_address
                .get(&opposite_key)
                .into_iter()
                .flatten()
                .filter_map(|index| self.graph.nodes.get(*index))
                .take(limit)
                .map(|candidate| subject_candidate(candidate, 4))
                .collect();
            unmapped_relation_endpoint |= subjects.is_empty();
            relation_neighbors.push(json!({
                "edge_id": edge.id,
                "predicate": edge.predicate,
                "direction": direction,
                "address": opposite,
                "subjects": subjects,
            }));
        }

        let direct_findings = self.attached_findings(node)?;
        let path_nodes: Vec<_> = node
            .path
            .as_deref()
            .and_then(|path| self.indexes.nodes_by_path.get(path))
            .into_iter()
            .flatten()
            .filter_map(|index| self.graph.nodes.get(*index))
            .collect();
        let file = path_nodes
            .iter()
            .copied()
            .find(|candidate| candidate.kind == NodeKind::File);
        let descendants: Vec<_> = path_nodes
            .iter()
            .copied()
            .filter(|candidate| candidate.kind != NodeKind::File)
            .collect();
        let contained_callables: Vec<_> = descendants
            .iter()
            .copied()
            .filter(|candidate| candidate.kind == NodeKind::Callable)
            .collect();
        let contained_declarations: Vec<_> = descendants
            .iter()
            .copied()
            .filter(|candidate| candidate.kind == NodeKind::Declaration)
            .collect();
        let contained_call_sites: Vec<_> = descendants
            .iter()
            .copied()
            .filter(|candidate| candidate.kind == NodeKind::CallStub)
            .collect();
        let mut enclosing_callables = Vec::new();
        let mut structural_path = Vec::new();
        let mut cursor = node;
        let mut visited = HashSet::new();
        while let Some(parent_id) = cursor.parent.as_deref() {
            if !visited.insert(cursor.id.as_str()) {
                break;
            }
            let Some(parent) = self.graph.node(parent_id) else {
                break;
            };
            let Some(edge) = self
                .indexes
                .structural_edges_by_node
                .get(self.graph_node_index(cursor)?)
                .into_iter()
                .flatten()
                .filter_map(|index| self.graph.edges.get(*index))
                .find(|edge| edge.from == parent.id && edge.to == cursor.id)
            else {
                break;
            };
            structural_path.push(edge);
            if parent.kind == NodeKind::Callable {
                enclosing_callables.push(parent);
            }
            cursor = parent;
            if parent.kind == NodeKind::File {
                break;
            }
        }
        structural_path.reverse();
        let containment_complete =
            node.kind == NodeKind::File || file.is_some_and(|file_node| cursor.id == file_node.id);
        let inherited_file_findings = if node.kind == NodeKind::File {
            Vec::new()
        } else if let Some(file) = file.filter(|file_node| cursor.id == file_node.id) {
            self.attached_findings(file)?
                .into_iter()
                .map(|attached| {
                    json!({
                        "attached": attached,
                        "file": subject_candidate(file, 4),
                        "structural_path": &structural_path,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let descendant_counts = json!({
            "total": descendants.len(),
            "callables": contained_callables.len(),
            "declarations": contained_declarations.len(),
            "call_sites": contained_call_sites.len(),
            "control_sites": descendants.iter().filter(|candidate| candidate.kind == NodeKind::ControlSite).count(),
            "resources": descendants.iter().filter(|candidate| candidate.kind == NodeKind::Resource).count(),
            "module_bindings": descendants.iter().filter(|candidate| candidate.kind == NodeKind::ModuleStub).count(),
        });
        let direct_finding_total = direct_findings.len();
        let inherited_finding_total = inherited_file_findings.len();
        let diagnostics: Vec<_> = node
            .path
            .as_deref()
            .map(|path| {
                self.indexes
                    .diagnostics_by_path
                    .get(path)
                    .iter()
                    .flat_map(|indexes| indexes.iter())
                    .filter_map(|index| self.graph.diagnostics.get(*index))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let configuration_parse_failed = diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "config.parse_failed");
        let source_inventory_failed = diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "source.read_failed" | "source.hash_failed"
            )
        });
        let counts = [
            neighbor_total,
            structural_total,
            relations.len(),
            relation_claims.len(),
            unresolved.len(),
            direct_finding_total,
            inherited_finding_total,
            descendants.len(),
            diagnostics.len(),
        ];
        let total = counts.iter().sum();
        let context_truncated = counts.iter().any(|count| *count > limit);
        let diagnostics: Vec<_> = diagnostics.into_iter().take(limit).collect();
        let mut gaps = Vec::new();
        if unmapped_relation_endpoint {
            gaps.push("relation_endpoint_not_materialized");
        }
        if configuration_parse_failed {
            gaps.push("configuration_parse_failed");
        }
        if source_inventory_failed {
            gaps.push("source_inventory_failed");
        }
        if node.path.is_some() && !containment_complete {
            gaps.push("containment_path_not_materialized");
        }
        if node.declaration.is_some() {
            gaps.extend([
                "declaration_inventory_is_partial",
                "initializer_syntax_is_not_runtime_value_or_property_equivalence",
            ]);
        }
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "inspect",
            "view": "subject",
            "snapshot": self.graph.snapshot,
            "status": if context_truncated { "truncated" } else { "known" },
            "subject": subject_candidate(node, 4),
            "call_site": node.call_site,
            "declaration": node.declaration,
            "code_contract": node.code_contract,
            "structural_edges": structural_edges,
            "neighbors": neighbors,
            "relation_neighbors": relation_neighbors,
            "relations": relations.into_iter().take(limit).collect::<Vec<_>>(),
            "relation_claims": relation_claims.into_iter().take(limit).collect::<Vec<_>>(),
            "unresolved_relations": unresolved.into_iter().take(limit).collect::<Vec<_>>(),
            "direct_findings": direct_findings.into_iter().take(limit).collect::<Vec<_>>(),
            "inherited_file_findings": inherited_file_findings.into_iter().take(limit).collect::<Vec<_>>(),
            "containment": {
                "file": file.map(|value| subject_candidate(value, 4)),
                "enclosing_callables": enclosing_callables.into_iter().take(limit).map(|value| subject_candidate(value, 4)).collect::<Vec<_>>(),
                "contained_callables": contained_callables.into_iter().take(limit).map(|value| subject_candidate(value, 1)).collect::<Vec<_>>(),
                "contained_declarations": contained_declarations.into_iter().take(limit).map(|value| subject_candidate(value, 1)).collect::<Vec<_>>(),
                "contained_call_sites": contained_call_sites.into_iter().take(limit).map(|value| subject_candidate(value, 1)).collect::<Vec<_>>(),
                "descendant_counts": descendant_counts,
            },
            "diagnostics": diagnostics,
            "gaps": gaps,
            "bounds": ResultBounds { offset: 0, limit, total_matches: total, truncated: context_truncated },
        }))
    }

    fn attached_findings(&self, node: &TopologyNode) -> Result<Vec<Value>, QueryError> {
        let mut attachment_indexes = self
            .indexes
            .attachments_by_node
            .get(self.graph_node_index(node)?)
            .cloned()
            .unwrap_or_default();
        if let Some(address_id) = &node.address_id {
            attachment_indexes.extend(
                self.indexes
                    .attachments_by_address
                    .get(address_id)
                    .into_iter()
                    .flatten()
                    .copied(),
            );
        }
        attachment_indexes.sort_unstable();
        attachment_indexes.dedup();

        let mut finding_attachments: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for attachment_index in attachment_indexes {
            let attachment = &self.graph.attachments[attachment_index];
            for finding_index in self
                .indexes
                .findings_by_location
                .get(&attachment.location_id)
                .into_iter()
                .flatten()
            {
                finding_attachments
                    .entry(*finding_index)
                    .or_default()
                    .push(attachment_index);
            }
        }

        Ok(finding_attachments
            .into_iter()
            .map(|(finding_index, mut attachment_indexes)| {
                attachment_indexes.sort_unstable();
                attachment_indexes.dedup();
                let attachments: Vec<_> = attachment_indexes
                    .iter()
                    .map(|index| &self.graph.attachments[*index])
                    .collect();
                let mut location_indexes: Vec<_> = attachments
                    .iter()
                    .filter_map(|attachment| {
                        self.indexes.locations_by_id.get(&attachment.location_id)
                    })
                    .copied()
                    .collect();
                location_indexes.sort_unstable();
                location_indexes.dedup();
                let locations: Vec<_> = location_indexes
                    .into_iter()
                    .map(|index| &self.graph.locations[index])
                    .collect();
                json!({
                    "finding": self.graph.findings[finding_index],
                    "locations": locations,
                    "attachments": attachments,
                })
            })
            .collect())
    }

    fn graph_node_index(&self, node: &TopologyNode) -> Result<usize, QueryError> {
        self.graph.node_by_id.get(&node.id).copied().ok_or_else(|| {
            QueryError::Contract(entmoot_contracts::query::QueryContractError::Invalid(
                format!(
                    "materialized subject `{}` is absent from the graph node index",
                    node.id
                ),
            ))
        })
    }

    fn exact_subject_nodes(&self, subject: &str) -> Vec<&TopologyNode> {
        self.indexes
            .nodes_by_exact_selector
            .get(subject)
            .into_iter()
            .flatten()
            .filter_map(|index| self.graph.nodes.get(*index))
            .collect()
    }

    /// List only semantic roles explicitly stated by a provider and normalized
    /// through the versioned exact-token policy. This never derives roles from
    /// trace ordering, rule names, or scanner brand.
    fn list_evidence_roles(
        &self,
        role: Option<&str>,
        path: Option<&str>,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        let role = role
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        let path = path.map(|value| value.trim().replace('\\', "/"));
        let limit = bounded_limit(requested_limit);
        let locations: HashMap<_, _> = self
            .graph
            .locations
            .iter()
            .map(|location| (location.location_id.as_str(), location))
            .collect();
        let attachments: HashMap<_, Vec<_>> = self.graph.attachments.iter().fold(
            HashMap::<&str, Vec<_>>::new(),
            |mut result, attachment| {
                result
                    .entry(attachment.location_id.as_str())
                    .or_default()
                    .push(attachment);
                result
            },
        );
        let repository = self.repository()?;
        let findings: HashMap<_, _> = self
            .graph
            .findings
            .iter()
            .map(|finding| (finding.observation_id.as_str(), finding))
            .collect();
        let mut items = Vec::new();
        let mut total = 0usize;
        for assertion in
            self.graph.assertions.iter().filter(|assertion| {
                assertion.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE
            })
        {
            let Some(asserted_role) = assertion.object.value.as_deref() else {
                continue;
            };
            if role
                .as_ref()
                .is_some_and(|expected| !asserted_role.eq_ignore_ascii_case(expected))
            {
                continue;
            }
            let location_id = assertion.subject.entity_id.as_str();
            let location = locations.get(location_id).copied();
            if path
                .as_ref()
                .is_some_and(|expected| location.is_none_or(|value| value.path != *expected))
            {
                continue;
            }
            total += 1;
            if items.len() >= limit {
                continue;
            }
            let steps = repository.trace_steps_at_location(location_id)?;
            let mut traces = HashMap::new();
            for step in &steps {
                if let Some(trace) = repository.trace_by_id(&step.trace_id)? {
                    traces.insert(step.trace_id.clone(), trace);
                }
            }
            let mut finding_records: Vec<_> = assertion
                .object
                .entity
                .as_ref()
                .and_then(|entity| findings.get(entity.entity_id.as_str()))
                .copied()
                .into_iter()
                .chain(
                    steps
                        .iter()
                        .filter_map(|step| traces.get(step.trace_id.as_str()))
                        .filter_map(|trace| findings.get(trace.finding_observation_id.as_str()))
                        .copied(),
                )
                .collect();
            finding_records.sort_by(|left, right| left.observation_id.cmp(&right.observation_id));
            finding_records.dedup_by(|left, right| left.observation_id == right.observation_id);
            items.push(json!({
                "role": asserted_role,
                "assertion": assertion,
                "location": location,
                "attachments": attachments.get(location_id).cloned().unwrap_or_default(),
                "trace_steps": steps,
                "findings": finding_records,
            }));
        }
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "find",
            "view": "evidence_role",
            "snapshot": self.graph.snapshot,
            "status": if total > limit { "truncated" } else if total == 0 { "unknown" } else { "known" },
            "role": role,
            "path": path,

            "items": items,
            "gaps": if total == 0 { vec!["no_provider_stated_role_matched"] } else { Vec::<&str>::new() },
            "bounds": ResultBounds::page(limit, limit.min(total), total),
        }))
    }

    fn list_findings(
        &self,
        query: Option<&str>,
        path: Option<&str>,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        let limit = bounded_limit(requested_limit);
        let query = query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let path = path.map(|value| value.trim().replace('\\', "/"));
        let locations: HashMap<_, _> = self
            .graph
            .locations
            .iter()
            .map(|location| (location.location_id.as_str(), location))
            .collect();
        let attachments: HashMap<_, Vec<_>> = self.graph.attachments.iter().fold(
            HashMap::<&str, Vec<_>>::new(),
            |mut result, attachment| {
                result
                    .entry(attachment.location_id.as_str())
                    .or_default()
                    .push(attachment);
                result
            },
        );
        let mut items = Vec::new();
        let mut total = 0usize;
        for finding in &self.graph.findings {
            let text = format!(
                "{} {} {} {}",
                finding.observation_id,
                finding.rule_id.as_deref().unwrap_or_default(),
                finding.level.as_deref().unwrap_or_default(),
                finding.message.as_deref().unwrap_or_default()
            )
            .to_lowercase();
            if query.as_ref().is_some_and(|needle| !text.contains(needle)) {
                continue;
            }
            let finding_locations: Vec<_> = finding
                .primary_location_ids
                .iter()
                .filter_map(|id| locations.get(id.as_str()).copied())
                .collect();
            if path.as_ref().is_some_and(|expected| {
                !finding_locations
                    .iter()
                    .any(|location| location.path == *expected)
            }) {
                continue;
            }
            total += 1;
            if items.len() < limit {
                let finding_attachments: Vec<_> = finding
                    .primary_location_ids
                    .iter()
                    .flat_map(|id| attachments.get(id.as_str()).into_iter().flatten().copied())
                    .collect();
                items.push(json!({
                    "finding": finding,
                    "locations": finding_locations,
                    "attachments": finding_attachments,
                }));
            }
        }
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "find",
            "view": "finding",
            "snapshot": self.graph.snapshot,
            "status": if total > limit { "truncated" } else if total == 0 { "unknown" } else { "known" },

            "items": items,
            "gaps": if total == 0 { vec!["no_finding_matched"] } else { Vec::<&str>::new() },
            "bounds": ResultBounds::page(limit, limit.min(total), total),
        }))
    }

    /// Return ordered provider trace steps with their normalized locations and
    /// deterministic attachment outcomes. Provider kinds are preserved exactly;
    /// ordinal position is never promoted to a source/sink role.
    fn get_finding_trace(
        &self,
        id: &str,
        offset: Option<usize>,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(QueryError::InvalidInput(
                "finding observation id or trace id is required".into(),
            ));
        }
        let finding = self
            .graph
            .findings
            .iter()
            .find(|finding| finding.observation_id == id);
        let repository = self.repository()?;
        let mut traces = Vec::new();
        let ids: Vec<&str> = finding.map_or_else(
            || vec![id],
            |finding| finding.trace_ids.iter().map(String::as_str).collect(),
        );
        for id in ids {
            if let Some(trace) = repository.trace_by_id(id)? {
                traces.push(trace);
            }
        }
        traces.sort_by(|a, b| {
            (a.code_flow_index, a.thread_index, &a.trace_id).cmp(&(
                b.code_flow_index,
                b.thread_index,
                &b.trace_id,
            ))
        });
        if traces.is_empty() {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "inspect",
            "view": "trace",
                "snapshot": self.graph.snapshot,
                "status": "unknown",
                "id": id,
                "items": [],
                "gaps": ["finding_or_trace_not_found"],
                "bounds": ResultBounds::complete(0),
            }));
        }

        let locations: HashMap<_, _> = self
            .graph
            .locations
            .iter()
            .map(|location| (location.location_id.as_str(), location))
            .collect();
        let attachments: HashMap<_, Vec<_>> = self.graph.attachments.iter().fold(
            HashMap::<&str, Vec<_>>::new(),
            |mut result, attachment| {
                result
                    .entry(attachment.location_id.as_str())
                    .or_default()
                    .push(attachment);
                result
            },
        );
        let total_steps: usize = traces.iter().map(|trace| trace.step_ids.len()).sum();
        let offset = offset.unwrap_or(0).min(total_steps);
        let limit = requested_limit.unwrap_or(100).clamp(1, 200);
        let mut items = Vec::new();
        for (trace, ordinal, id) in traces
            .iter()
            .flat_map(|trace| {
                trace
                    .step_ids
                    .iter()
                    .enumerate()
                    .map(move |(ordinal, id)| (trace, ordinal, id))
            })
            .skip(offset)
            .take(limit)
        {
            if let Some(step) = repository.trace_step_by_id(id)? {
                if step.trace_id != trace.trace_id
                    || usize::try_from(step.ordinal).ok() != Some(ordinal)
                {
                    return Err(QueryError::InvalidInput(
                        "trace step does not match retained provider order".into(),
                    ));
                }
                items.push(json!({"step":step,"location":locations.get(step.location_id.as_str()).copied(),"attachments":attachments.get(step.location_id.as_str()).cloned().unwrap_or_default()}));
            } else {
                return Err(crate::KnowledgeRepositoryError::InvalidRecord {
                    collection: "trace_steps",
                    reason: "declared trace step is unavailable; evidence is incomplete".into(),
                }
                .into());
            }
        }
        let count = items.len();
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "inspect",
            "view": "trace",
            "snapshot": self.graph.snapshot,
            "status": if offset + count < total_steps { "truncated" } else { "known" },
            "finding": finding,
            "traces": traces,

            "items": items,
            "gaps": [],
            "bounds": ResultBounds::window(offset, limit, count, total_steps),
        }))
    }

    fn explain_evidence(&self, id: &str) -> Result<Value, QueryError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(QueryError::InvalidInput("id is required".into()));
        }
        if PropositionId::parse(id.to_owned()).is_ok() {
            let result = self.proposition(id)?;
            if !result["proposition"].is_null() {
                return Ok(result);
            }
        }
        if RelationEdgeId::parse(id.to_owned()).is_ok() {
            let result = self.relation_claims(id)?;
            if result["relation_claims"]
                .as_array()
                .is_some_and(|claims| !claims.is_empty())
            {
                return Ok(result);
            }
        }
        if let Some(finding) = self
            .graph
            .findings
            .iter()
            .find(|value| value.observation_id == id)
        {
            let location_ids: HashSet<_> = finding.primary_location_ids.iter().collect();
            let locations: Vec<_> = self
                .graph
                .locations
                .iter()
                .filter(|value| location_ids.contains(&value.location_id))
                .collect();
            let attachments: Vec<_> = self
                .graph
                .attachments
                .iter()
                .filter(|value| location_ids.contains(&value.location_id))
                .collect();
            let artifact = self
                .graph
                .artifacts
                .iter()
                .find(|value| value.artifact_id.as_str() == finding.artifact_id);
            let repository = self.repository()?;
            let mut traces = Vec::new();
            for id in &finding.trace_ids {
                if let Some(trace) = repository.trace_by_id(id)? {
                    traces.push(trace);
                }
            }
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "inspect",
            "view": "evidence",

                "snapshot": self.graph.snapshot,
                "status": "known",
                "body": { "evidence_kind": "finding", "finding": finding, "artifact": artifact, "locations": locations, "attachments": attachments, "traces": traces },
                "gaps": [],
                "bounds": ResultBounds::complete(0),
            }));
        }
        let assertions: Vec<_> = self
            .graph
            .assertions
            .iter()
            .filter(|value| value.assertion_id == id)
            .collect();
        if !assertions.is_empty() {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "inspect",
            "view": "evidence",

                "snapshot": self.graph.snapshot,
                "status": "known",
                "body": { "evidence_kind": "assertions", "assertions": assertions },
                "gaps": [],
                "bounds": ResultBounds::complete(0),
            }));
        }
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "inspect",
            "view": "evidence",

            "snapshot": self.graph.snapshot,
            "status": "unknown",
            "body": { "evidence_kind": "missing", "id": id },
            "gaps": ["evidence_id_not_found"],
            "bounds": ResultBounds::complete(0),
        }))
    }
}

impl QueryService for QueryEngine {
    fn unused(
        &self,
        request: entmoot_contracts::UnusedRequest,
    ) -> Result<entmoot_contracts::UnusedReport, QueryServiceError> {
        QueryEngine::unused(self, request).map_err(QueryServiceError::from)
    }
    fn test_impact(
        &self,
        request: entmoot_contracts::TestImpactRequest,
    ) -> Result<entmoot_contracts::TestImpactReport, QueryServiceError> {
        QueryEngine::test_impact(self, request).map_err(QueryServiceError::from)
    }
    fn relay_impact(
        &self,
        request: entmoot_contracts::query::ImpactRelayRequest,
    ) -> Result<entmoot_contracts::query::ImpactRelayResult, QueryServiceError> {
        self.deliver_impact_relay(request)
            .map_err(QueryServiceError::from)
    }

    fn execute(
        &self,
        request: entmoot_contracts::query::QueryRequest,
    ) -> Result<entmoot_contracts::query::QueryResponse, QueryServiceError> {
        self.deliver_query(request).map_err(QueryServiceError::from)
    }
    fn compatibility(
        &self,
        request: entmoot_contracts::query::CompatibilityRequest,
    ) -> Result<entmoot_contracts::query::CompatibilityResult, QueryServiceError> {
        self.compatibility_catalog()?.assess(request)
    }
    fn trace_seeds(
        &self,
        request: entmoot_contracts::query::SeededPathRequest,
    ) -> Result<entmoot_contracts::query::SeededPathResponse, QueryServiceError> {
        self.deliver_seeds(request).map_err(QueryServiceError::from)
    }
    fn status(
        &self,
        request: entmoot_contracts::query::StatusRequest,
    ) -> Result<entmoot_contracts::query::StatusReport, QueryServiceError> {
        QueryEngine::status(self, request).map_err(QueryServiceError::from)
    }
    fn inventory(
        &self,
        request: entmoot_contracts::query::InventoryRequest,
    ) -> Result<entmoot_contracts::query::InventoryReport, QueryServiceError> {
        QueryEngine::inventory(self, request).map_err(QueryServiceError::from)
    }
    fn call_components(
        &self,
        request: entmoot_contracts::query::CallComponentsRequest,
    ) -> Result<entmoot_contracts::query::CallComponentsReport, QueryServiceError> {
        QueryEngine::call_components(self, request).map_err(QueryServiceError::from)
    }
    fn clones(
        &self,
        request: entmoot_contracts::CloneRequest,
    ) -> Result<entmoot_contracts::CloneReport, QueryServiceError> {
        QueryEngine::clones(self, request).map_err(QueryServiceError::from)
    }
    fn find(&self, request: FindQuery) -> Result<QueryResult, QueryServiceError> {
        QueryEngine::find(self, request).map_err(QueryServiceError::from)
    }

    fn inspect(&self, request: InspectQuery) -> Result<QueryResult, QueryServiceError> {
        QueryEngine::inspect(self, request).map_err(QueryServiceError::from)
    }

    fn traverse(&self, request: TraverseQuery) -> Result<QueryResult, QueryServiceError> {
        QueryEngine::traverse(self, request).map_err(QueryServiceError::from)
    }

    fn read(&self, request: ReadQuery) -> Result<QueryResult, QueryServiceError> {
        QueryEngine::read(self, request).map_err(QueryServiceError::from)
    }
}

impl From<QueryError> for QueryServiceError {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::InvalidInput(message) => Self::InvalidRequest(message),
            QueryError::Unavailable(message) => Self::Unavailable(message),
            QueryError::Repository(error) => Self::Unavailable(error.to_string()),
            QueryError::Source(error) => Self::Unavailable(error.to_string()),
            QueryError::Serialization(error) => Self::Execution(error.to_string()),
            QueryError::Contract(error) => Self::Execution(error.to_string()),
            QueryError::UnsupportedSourceEncoding(path) => {
                Self::Execution(format!("source file is not valid UTF-8: {path}"))
            }
        }
    }
}

fn bounded_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(20).clamp(1, 100)
}

fn validate_walk_budget(budget: BoundedWalkBudget) -> Result<BoundedWalkBudget, QueryError> {
    budget.validate().map_err(QueryError::InvalidInput)?;
    Ok(budget)
}

fn walk_budget_json(budget: BoundedWalkBudget) -> Value {
    json!({
        "max_hops": budget.max_hops,
        "max_expansions": budget.max_expansions,
        "max_examined_edges": budget.max_examined_edges,
        "max_edges_per_node": budget.max_edges_per_node,
        "max_results": budget.max_results,
        "max_evidence_per_hop": budget.max_evidence_per_hop,
    })
}

fn parse_walk_predicates(values: &[String]) -> Result<HashSet<RelationKind>, QueryError> {
    values
        .iter()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "calls" => Ok(RelationKind::Calls),
            "imports" => Ok(RelationKind::Imports),
            "implements" => Ok(RelationKind::Implements),
            "depends_on" => Ok(RelationKind::DependsOn),
            "selects" => Ok(RelationKind::Selects),
            "routes_to" => Ok(RelationKind::RoutesTo),
            "uses_configuration" => Ok(RelationKind::UsesConfiguration),
            "connects_to" => Ok(RelationKind::ConnectsTo),
            "observed_as" => Ok(RelationKind::ObservedAs),
            "has_canonical_form" => Ok(RelationKind::HasCanonicalForm),
            other => Err(QueryError::InvalidInput(format!(
                "unsupported walk predicate `{other}`"
            ))),
        })
        .collect()
}

fn walk_edge_key(graph: &TopologyGraph, indexes: &[usize]) -> String {
    indexes
        .iter()
        .map(|index| graph.relation_edges[*index].id.as_str())
        .collect::<Vec<_>>()
        .join(":")
}

fn walk_edges_compatible(graph: &TopologyGraph, indexes: &[usize]) -> bool {
    let mut expected = None;
    let mut previous = None;
    for index in indexes {
        let edge = &graph.relation_edges[*index];
        let Some(scope) = composition_scope(graph, &edge.scope) else {
            return false;
        };
        if expected.as_ref().is_some_and(|existing| existing != &scope)
            && !previous.is_some_and(|prior| observation_scope_bridge(prior, edge))
        {
            return false;
        }
        expected = Some(scope);
        previous = Some(edge);
    }
    true
}

/// The only existing explicit context bridge: a declared resource is observed
/// as another resource. The directed bridge has the destination observation scope.
fn is_observation_boundary(edge: &entmoot_contracts::RelationEdge) -> bool {
    edge.predicate == RelationKind::ObservedAs
        && matches!(edge.subject, Address::Resource { .. })
        && matches!(edge.object, Address::Resource { .. })
}

fn observation_scope_bridge(
    previous: &entmoot_contracts::RelationEdge,
    next: &entmoot_contracts::RelationEdge,
) -> bool {
    is_observation_boundary(next)
        && previous.scope.snapshot_revision == next.scope.snapshot_revision
        && previous.scope.snapshot_digest == next.scope.snapshot_digest
}

/// Site identity is allowed to vary; repository/build/snapshot identity is not.
fn composition_scope(
    graph: &TopologyGraph,
    scope: &entmoot_contracts::Scope,
) -> Option<entmoot_contracts::Scope> {
    if scope
        .snapshot_revision
        .as_ref()
        .is_some_and(|r| r != &graph.snapshot.revision)
        || scope
            .snapshot_digest
            .as_ref()
            .is_some_and(|d| graph.snapshot.snapshot_digest.as_ref() != Some(d))
    {
        return None;
    }
    let mut result = scope.clone();
    result.occurrence_id = None;
    Some(result)
}

/// Applicability is stricter than retrieval: proposition history, review notes,
/// and claims from other scopes remain evidence but cannot qualify this edge.
/// In particular, a context-limited native claim can share a structural callsite
/// projection without establishing context-free dispatch closure.
fn applicable_resolution_state(
    claim: &entmoot_contracts::RelationClaim,
    edge: &entmoot_contracts::RelationEdge,
) -> Option<ResolutionState> {
    if claim.state != RelationClaimState::Active
        || claim.predicate != "resolution_state"
        || claim.scope != edge.scope
    {
        return None;
    }
    let applies = match &claim.subject {
        entmoot_contracts::SubjectId::RelationEdge { edge_id } => edge_id == &edge.id,
        entmoot_contracts::SubjectId::Proposition { proposition_id } => {
            proposition_id == &edge.proposition_id
        }
        _ => false,
    };
    match claim.value {
        RelationClaimValue::ResolutionState { state } if applies => Some(state),
        _ => None,
    }
}

fn resolution_state_name(state: ResolutionState) -> &'static str {
    match state {
        ResolutionState::Structural => "structural",
        ResolutionState::Candidate => "candidate",
        ResolutionState::ArtifactResolved => "artifact_resolved",
        ResolutionState::SemanticResolved => "semantic_resolved",
        ResolutionState::Validated => "validated",
        ResolutionState::Refuted => "refuted",
        ResolutionState::Stale => "stale",
        ResolutionState::Unknown => "unknown",
    }
}

fn source_query_span(source: &str, line: usize, column: Option<usize>) -> Option<(usize, usize)> {
    let mut starts = vec![0usize];
    starts.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    let line_start = *starts.get(line.checked_sub(1)?)?;
    let line_end_with_newline = starts.get(line).copied().unwrap_or(source.len());
    let mut line_end = line_end_with_newline;
    if source.as_bytes().get(line_end.saturating_sub(1)) == Some(&b'\n') {
        line_end = line_end.saturating_sub(1);
    }
    if source.as_bytes().get(line_end.saturating_sub(1)) == Some(&b'\r') {
        line_end = line_end.saturating_sub(1);
    }
    if let Some(column) = column {
        let line_text = source.get(line_start..line_end)?;
        let offset = if column == 1 {
            0
        } else {
            line_text.char_indices().nth(column - 1)?.0
        };
        let start = line_start + offset;
        let end = source
            .get(start..)?
            .chars()
            .next()
            .map(|character| start + character.len_utf8())?;
        Some((start, end))
    } else {
        Some((
            line_start,
            line_end.max(line_start + usize::from(line_start < source.len())),
        ))
    }
}

fn searchable_node_text(node: &TopologyNode) -> String {
    format!(
        "{} {} {} {} {} {}",
        node.id,
        node.label,
        node.path.as_deref().unwrap_or_default(),
        node.fqn.as_deref().unwrap_or_default(),
        node.display_ref,
        node.logic_address.as_deref().unwrap_or_default()
    )
    .to_lowercase()
}

const STRUCTURAL_HASH_PREFIX: &str = "blake3:";

/// Exact `structural_hash` selectors: prefixed wire form and bare 64-hex.
/// Empty when `raw` is not a digest-shaped query (no fuzzy / substring hashing).
fn structural_hash_exact_selectors(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let bare = if let Some(hex) = trimmed.strip_prefix(STRUCTURAL_HASH_PREFIX) {
        hex
    } else if trimmed.len() == 64 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        trimmed
    } else {
        return Vec::new();
    };
    let bare = bare.to_ascii_lowercase();
    if bare.len() != 64 || !bare.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Vec::new();
    }
    vec![format!("{STRUCTURAL_HASH_PREFIX}{bare}"), bare]
}

fn line_distance(node: &TopologyNode, line: Option<usize>) -> usize {
    line.zip(node.start_line)
        .map(|(expected, actual)| expected.abs_diff(actual))
        .unwrap_or(0)
}

fn subject_candidate(node: &TopologyNode, match_score: u8) -> Value {
    json!(inventory::subject_card(node, match_score))
}

fn node_addresses(graph: &TopologyGraph, node: &TopologyNode) -> Vec<Address> {
    let resource_address = graph
        .resources
        .iter()
        .find(|resource| resource.occurrence_id == node.id)
        .map(|resource| &resource.logical_address);
    node_addresses_with_resource(node, &graph.snapshot.repository_id, resource_address)
}

fn node_addresses_with_resource(
    node: &TopologyNode,
    repository_id: &str,
    resource_address: Option<&Address>,
) -> Vec<Address> {
    let mut addresses = vec![entmoot_contracts::topology_node_address(&node.id)];
    if let Some(address_id) = &node.address_id {
        addresses.push(Address::AddressId {
            address_id: address_id.clone(),
        });
    }
    if let Some(logic_address) = &node.logic_address {
        addresses.push(Address::LogicAddress {
            logic_address: logic_address.clone(),
        });
    }
    if let Some(path) = &node.path {
        addresses.push(Address::RepoPath {
            repository_id: Some(repository_id.to_owned()),
            path: path.clone(),
            start_byte: node.start_byte.and_then(|value| u32::try_from(value).ok()),
            end_byte: node.end_byte.and_then(|value| u32::try_from(value).ok()),
        });
    }
    if let Some(resource_address) = resource_address {
        addresses.push(resource_address.clone());
    }
    let mut seen = HashSet::new();
    addresses.retain(|address| seen.insert(address_identity_key(address)));
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build_topology_from_repo_with_options, KnowledgeStore, SourceFile, SourceObservation,
        SourceReadRequest, SourceStore, SourceStoreError, TopologyBuildOptions,
    };
    use crate::{KnowledgeRepository, SqliteKnowledgeRepository};
    use entmoot_contracts::{
        AttachmentDisposition, AttachmentFreshness, AttachmentReason, AttachmentTarget,
        AttachmentTargetKind, FindingObservation, LocationAttachment, LocationExtent,
        LocationRecord, SourceRegion, TraceRecord, TraceStepRecord,
    };

    struct MemorySourceStore {
        files: HashMap<String, Vec<u8>>,
    }

    #[test]
    fn comparisons_are_lazy_shared_and_never_needed_by_subject_output() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut graph = TopologyGraph::new(directory.path().to_string_lossy());
        graph.upsert_node(TopologyNode::shell(
            "target-id",
            NodeKind::Callable,
            "target",
            None,
        ));
        let store = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let base = graph.clone();
        let engine = QueryEngine::new(graph, store).with_compare_loader(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(base.clone())
        });
        assert!(engine.has_comparison());
        engine
            .deliver_query(entmoot_contracts::query::QueryRequest::Find {
                request: FindQuery::Subject {
                    query: Some("target".into()),
                    path: None,
                    line: None,
                    limit: Some(1),
                    selector: None,
                },
                output: Default::default(),
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let cloned = engine.clone();
        let first = engine.comparison_graph().unwrap().unwrap();
        assert_eq!(first.nodes[0].id, "target-id");
        assert!(std::ptr::eq(
            first,
            cloned.comparison_graph().unwrap().unwrap()
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let failed_calls = Arc::new(AtomicUsize::new(0));
        let failed_observed = failed_calls.clone();
        let failed = engine.with_compare_loader(move || {
            failed_observed.fetch_add(1, Ordering::SeqCst);
            Err(QueryError::Unavailable(
                "fixture base is unavailable".into(),
            ))
        });
        for _ in 0..2 {
            assert!(failed
                .comparison_graph()
                .unwrap_err()
                .to_string()
                .contains("fixture base is unavailable"));
        }
        assert_eq!(failed_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn retained_subject_search_counts_all_pages_and_ranks_only_bounded_cards() {
        let directory = tempfile::tempdir().unwrap();
        let mut graph = TopologyGraph::new(directory.path().to_string_lossy());
        for i in 0..1100 {
            let mut node =
                TopologyNode::shell(format!("node-{i:04}"), NodeKind::Callable, "target", None);
            node.path = Some(if i % 2 == 0 { "a.py" } else { "b.py" }.into());
            node.start_line = Some(i + 1);
            graph.upsert_node(node);
        }
        let store = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        let eager = QueryEngine::new(graph, &store);
        let retained = QueryEngine::from_retained(&store, Default::default()).unwrap();
        let request = FindQuery::Subject {
            selector: None,
            query: Some("TARGET".into()),
            path: Some("a.py".into()),
            line: Some(601),
            limit: Some(3),
        };
        let result = retained.find(request.clone()).unwrap();
        assert_eq!(result, eager.find(request).unwrap());
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["bounds"]["total_matches"], 550);
        assert_eq!(json["items"].as_array().unwrap().len(), 3);
        assert_eq!(json["items"][0]["node_id"], "node-0600");
        assert!(retained
            .deferred_navigation
            .as_ref()
            .unwrap()
            .get()
            .is_none());
        let unknown = retained
            .find(FindQuery::Subject {
                selector: None,
                query: Some("missing".into()),
                path: Some("missing.py".into()),
                line: None,
                limit: Some(3),
            })
            .unwrap();
        assert_eq!(serde_json::to_value(unknown).unwrap()["status"], "unknown");
    }

    #[test]
    fn retained_open_and_verified_reads_do_not_materialize_navigation() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let text = "def caller():\n    return target()\ndef target():\n    return 7\n";
        std::fs::write(source.path().join("a.py"), text).unwrap();
        let graph = build_topology_from_repo_with_options(
            source.path(),
            TopologyBuildOptions {
                repository: Some("deferred-fixture"),
                ..Default::default()
            },
        )
        .unwrap();
        let store = data.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        let sources = std::sync::Arc::new(MemorySourceStore {
            files: HashMap::from([("a.py".into(), text.as_bytes().to_vec())]),
        });
        repository
            .store_inventory(&graph, Some(sources.as_ref()))
            .unwrap();
        drop(repository);
        let eager = QueryEngine::new(graph.clone(), &store).with_source_store(sources.clone());
        let deferred = QueryEngine::from_retained(&store, Default::default())
            .unwrap()
            .with_source_store(sources);
        assert!(deferred.graph.nodes.is_empty());
        let read = ReadQuery::Source {
            path: "a.py".into(),
            start_line: 1,
            end_line: Some(2),
            context_lines: Some(0),
            max_bytes: Some(1024),
        };
        assert_eq!(
            deferred.read(read.clone()).unwrap(),
            eager.read(read).unwrap()
        );
        assert!(deferred
            .deferred_navigation
            .as_ref()
            .unwrap()
            .get()
            .is_none());
        assert_eq!(deferred.publication_snapshot(), &graph.snapshot);
        let request = FindQuery::Subject {
            selector: None,
            query: Some("target".into()),
            path: None,
            line: None,
            limit: Some(10),
        };
        assert_eq!(
            deferred.find(request.clone()).unwrap(),
            eager.find(request).unwrap()
        );
        assert!(deferred
            .deferred_navigation
            .as_ref()
            .unwrap()
            .get()
            .is_none());
        assert!(!deferred.graph().unwrap().nodes.is_empty());
        assert!(deferred
            .deferred_navigation
            .as_ref()
            .unwrap()
            .get()
            .is_some());
    }

    #[test]
    fn repeated_discovery_reuses_witnesses_but_preserves_budget_and_generation() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("reuse.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        for family in [
            "rust",
            "python",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "java",
            "csharp",
            "c",
            "cpp-experimental",
        ] {
            let mut graph = TopologyGraph::new(family);
            for id in ["a", "b", "c"] {
                graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
            }
            for (from, to) in [("a", "b"), ("b", "c")] {
                graph
                    .relation_edges
                    .push(entmoot_contracts::relation_edge_with_id(
                        "fixture",
                        entmoot_contracts::topology_node_address(from),
                        RelationKind::Calls,
                        entmoot_contracts::topology_node_address(to),
                        entmoot_contracts::Scope {
                            domain: family.into(),
                            snapshot_revision: None,
                            snapshot_digest: None,
                            occurrence_id: None,
                            context_id: None,
                        },
                        None,
                    ));
            }
            let engine = QueryEngine::new(graph.clone(), &store);
            let budget = BoundedWalkBudget::default();
            let cold = engine
                .bounded_walk("a", "c", &["calls".into()], budget)
                .unwrap();
            let warm = engine
                .clone()
                .bounded_walk("a", "c", &["calls".into()], budget)
                .unwrap();
            assert_eq!(cold["paths"], warm["paths"], "{family}");
            assert_eq!(warm["search_receipt"]["reused_discovery"], true, "{family}");
            assert_eq!(warm["search_receipt"]["examined_edges"], 0);
            let short = engine
                .bounded_walk(
                    "a",
                    "c",
                    &["calls".into()],
                    BoundedWalkBudget {
                        max_hops: 1,
                        ..budget
                    },
                )
                .unwrap();
            assert_eq!(short["status"], "unknown");
            graph.relation_edges.clear();
            let replaced = QueryEngine::new(graph, &store)
                .bounded_walk("a", "c", &["calls".into()], budget)
                .unwrap();
            assert_eq!(replaced["status"], "unknown");
            assert_ne!(replaced["search_receipt"]["reused_discovery"], true);
        }
    }

    #[test]
    fn directed_bidirectional_witnesses_match_independent_small_graph_oracle() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("oracle.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        let pairs = [(0, 1), (0, 2), (1, 0), (1, 2), (2, 0), (2, 1)];
        for mask in 0..64 {
            let mut graph = TopologyGraph::new("oracle");
            for n in 0..3 {
                graph.upsert_node(crate::TopologyNode::shell(
                    n.to_string(),
                    crate::NodeKind::Callable,
                    n.to_string(),
                    None,
                ));
            }
            let mut adjacent = [[false; 3]; 3];
            for (bit, &(a, b)) in pairs.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    adjacent[a][b] = true;
                    graph
                        .relation_edges
                        .push(entmoot_contracts::relation_edge_with_id(
                            "source",
                            entmoot_contracts::topology_node_address(&a.to_string()),
                            RelationKind::Calls,
                            entmoot_contracts::topology_node_address(&b.to_string()),
                            entmoot_contracts::Scope {
                                domain: "oracle".into(),
                                snapshot_revision: None,
                                snapshot_digest: None,
                                occurrence_id: None,
                                context_id: None,
                            },
                            None,
                        ));
                }
            }
            let engine = QueryEngine::new(graph, store.clone());
            for (a, b) in pairs {
                let mut reached = [false; 3];
                reached[a] = true;
                for hops in 1..=2 {
                    let previous = reached;
                    for (from, row) in adjacent.iter().enumerate() {
                        for (to, edge) in row.iter().enumerate() {
                            reached[to] |= previous[from] && *edge;
                        }
                    }
                    let result = engine
                        .bounded_walk(
                            &a.to_string(),
                            &b.to_string(),
                            &["calls".into()],
                            BoundedWalkBudget {
                                max_hops: hops,
                                max_edges_per_node: 0,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    let neighborhood = engine
                        .bounded_neighborhood(
                            &a.to_string(),
                            CallDirection::Callees,
                            &["calls".into()],
                            entmoot_contracts::query::HopEvidenceFilter::Any,
                            None,
                            BoundedWalkBudget {
                                max_hops: hops,
                                max_edges_per_node: 0,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    let expected = serde_json::to_value(entmoot_contracts::topology_node_address(
                        &b.to_string(),
                    ))
                    .unwrap();
                    assert_eq!(
                        neighborhood["items"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|hit| hit["subject"] == expected),
                        reached[b],
                        "neighborhood mask={mask}, {a}->{b}, max_hops={hops}"
                    );
                    assert_eq!(
                        !result["paths"].as_array().unwrap().is_empty(),
                        reached[b],
                        "mask={mask}, {a}->{b}, max_hops={hops}"
                    );
                    for path in result["paths"].as_array().unwrap() {
                        assert!(path["hop_count"].as_u64().unwrap() <= hops as u64);
                    }
                }
            }
        }
    }

    #[test]
    fn predicate_filter_precedes_local_fanout_but_counts_all_examined_edges() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("walk.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        let mut graph = TopologyGraph::new("fixture");
        for id in ["a", "b"] {
            graph.upsert_node(crate::TopologyNode::shell(
                id,
                crate::NodeKind::Callable,
                id,
                None,
            ));
        }
        let edge = |predicate, context| {
            entmoot_contracts::relation_edge_with_id(
                "source",
                entmoot_contracts::topology_node_address("a"),
                predicate,
                entmoot_contracts::topology_node_address("b"),
                entmoot_contracts::Scope {
                    domain: "fixture".into(),
                    snapshot_revision: None,
                    snapshot_digest: None,
                    occurrence_id: None,
                    context_id: context,
                },
                None,
            )
        };
        let call = edge(RelationKind::Calls, None);
        // Force a genuine, valid non-call edge before the call in stable ID order.
        let import = (0..1000)
            .map(|i| edge(RelationKind::Imports, Some(i.to_string())))
            .find(|candidate| candidate.id.as_str() < call.id.as_str())
            .unwrap();
        graph.relation_edges = vec![import, call];
        let engine = QueryEngine::new(graph, store);
        let budget = BoundedWalkBudget {
            max_hops: 1,
            max_edges_per_node: 1,
            ..Default::default()
        };
        let result = engine
            .bounded_walk("a", "b", &["calls".into()], budget)
            .unwrap();
        assert_eq!(result["paths"].as_array().unwrap().len(), 1);
        assert_eq!(result["search_receipt"]["examined_edges"], 2);
        assert_eq!(result["search_receipt"]["fanout_truncated"], false);
        let result = engine
            .bounded_walk(
                "a",
                "b",
                &["calls".into()],
                BoundedWalkBudget {
                    max_examined_edges: 1,
                    ..budget
                },
            )
            .unwrap();
        assert_eq!(result["search_receipt"]["edge_budget_exhausted"], true);
        assert!(result["paths"].as_array().unwrap().is_empty());
    }

    #[test]
    fn narrow_reverse_frontier_finds_path_without_expanding_large_source() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("walk.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        let mut graph = TopologyGraph::new("fixture");
        for id in ["start", "bridge", "target"] {
            graph.upsert_node(crate::TopologyNode::shell(
                id,
                crate::NodeKind::Callable,
                id,
                None,
            ));
        }
        let mut add = |from: &str, to: &str| {
            graph
                .relation_edges
                .push(entmoot_contracts::relation_edge_with_id(
                    "source",
                    entmoot_contracts::topology_node_address(from),
                    RelationKind::Calls,
                    entmoot_contracts::topology_node_address(to),
                    entmoot_contracts::Scope {
                        domain: "fixture".into(),
                        snapshot_revision: None,
                        snapshot_digest: None,
                        occurrence_id: None,
                        context_id: None,
                    },
                    None,
                ))
        };
        for i in 0..2000 {
            add("start", &format!("leaf-{i}"));
        }
        add("start", "bridge");
        add("bridge", "target");
        let engine = QueryEngine::with_options(
            graph,
            store,
            QueryEngineOptions {
                contract_paths: false,
                ..Default::default()
            },
        );
        let result = engine
            .bounded_walk(
                "start",
                "target",
                &["calls".into()],
                BoundedWalkBudget {
                    max_hops: 3,
                    max_expansions: 2,
                    max_edges_per_node: 50,
                    max_results: 1,
                    max_examined_edges: 100_000,
                    max_evidence_per_hop: 20,
                },
            )
            .unwrap();
        assert_eq!(result["paths"][0]["hop_count"], 2);
        assert_eq!(result["search_receipt"]["examined_edges"], 2);
        let limited = engine
            .bounded_walk(
                "start",
                "target",
                &["calls".into()],
                BoundedWalkBudget {
                    max_hops: 256,
                    max_expansions: 10,
                    max_edges_per_node: 0,
                    max_results: 1,
                    max_examined_edges: 1,
                    max_evidence_per_hop: 20,
                },
            )
            .unwrap();
        assert_eq!(limited["search_receipt"]["edge_budget_exhausted"], true);
        assert_eq!(limited["search_receipt"]["examined_edges"], 1);
        assert_eq!(limited["status"], "truncated");
    }

    #[test]
    fn callers_and_callees_are_directional_keyset_pages_with_open_world_gap() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("calls.sqlite3");
        let mut graph = TopologyGraph::new(directory.path().to_string_lossy());
        for id in ["a", "b", "c"] {
            graph.upsert_node(crate::TopologyNode::shell(
                id,
                crate::NodeKind::Callable,
                id,
                None,
            ));
        }
        for target in ["b", "c"] {
            graph
                .relation_edges
                .push(entmoot_contracts::relation_edge_with_id(
                    "source",
                    entmoot_contracts::topology_node_address("a"),
                    RelationKind::Calls,
                    entmoot_contracts::topology_node_address(target),
                    entmoot_contracts::Scope {
                        domain: graph.snapshot.repository_id.clone(),
                        snapshot_revision: Some(graph.snapshot.revision.clone()),
                        snapshot_digest: graph.snapshot.snapshot_digest.clone(),
                        occurrence_id: None,
                        context_id: None,
                    },
                    None,
                ));
        }
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_owner(
                "snapshot:test/query-calls",
                &KnowledgeStore::extract_from_graph(&graph),
                &[],
            )
            .unwrap();
        let engine = QueryEngine::new(graph, store.clone());
        let first = engine
            .inspect(InspectQuery::Callees {
                id: "a".into(),
                after: None,
                limit: Some(1),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(first["status"], "truncated");
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            first["gaps"][0],
            "repository_call_graph_completeness_not_established"
        );
        let cursor: CallPageCursor = serde_json::from_value(first["next_cursor"].clone()).unwrap();
        assert!(engine
            .inspect(InspectQuery::Callers {
                id: "b".into(),
                after: Some(cursor.clone()),
                limit: Some(1)
            })
            .is_err());
        let second = engine
            .inspect(InspectQuery::Callees {
                id: "a".into(),
                after: Some(cursor.clone()),
                limit: Some(1),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_ne!(
            first["items"][0]["relation"]["id"],
            second["items"][0]["relation"]["id"]
        );
        let callers = engine
            .inspect(InspectQuery::Callers {
                id: "b".into(),
                after: None,
                limit: Some(10),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(callers["direction"], "callers");
        assert_eq!(callers["items"][0]["opposite"]["value"], "a");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .remove_owner("snapshot:test/query-calls")
            .unwrap();
        let stale = engine.inspect(InspectQuery::Callees {
            id: "a".into(),
            after: Some(cursor),
            limit: Some(1),
        });
        assert!(
            matches!(stale, Err(QueryError::InvalidInput(message)) if message.contains("visibility state"))
        );
    }

    #[test]
    fn path_support_requires_resolution_predicate_and_exact_claim_scope() {
        let directory = tempfile::tempdir().unwrap();
        let scope = entmoot_contracts::Scope {
            domain: "test/path-scope".into(),
            snapshot_revision: None,
            snapshot_digest: None,
            occurrence_id: None,
            context_id: None,
        };
        let producer = entmoot_contracts::ProducerRef {
            id: "reviewer".into(),
            name: "fixture".into(),
            version: Some("1".into()),
            config_fingerprint: None,
            environment_fingerprint: None,
            capabilities: vec![],
        };
        for (index, (predicate, value, foreign, expected)) in [
            (
                "review_note",
                RelationClaimValue::Literal {
                    value: "reviewed".into(),
                },
                None,
                "candidate",
            ),
            (
                "review_note",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Validated,
                },
                None,
                "candidate",
            ),
            (
                "resolution_state",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Validated,
                },
                Some("snapshot"),
                "candidate",
            ),
            (
                "resolution_state",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Validated,
                },
                None,
                "supported",
            ),
            (
                "resolution_state",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Validated,
                },
                Some("context"),
                "candidate",
            ),
            (
                "resolution_state",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Refuted,
                },
                None,
                "conflicted",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let store = directory.path().join(format!("scope-{index}.sqlite3"));
            let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
            let mut graph = TopologyGraph::new("test/path-scope");
            for id in ["a", "b"] {
                graph.upsert_node(crate::TopologyNode::shell(
                    id,
                    crate::NodeKind::Callable,
                    id,
                    None,
                ));
            }
            let edge = entmoot_contracts::relation_edge_with_id(
                "source",
                entmoot_contracts::topology_node_address("a"),
                RelationKind::Calls,
                entmoot_contracts::topology_node_address("b"),
                scope.clone(),
                None,
            );
            let mut claim_scope = scope.clone();
            match foreign {
                Some("snapshot") => claim_scope.snapshot_revision = Some("a".repeat(40)),
                Some("context") => claim_scope.context_id = Some("build-only".into()),
                _ => {}
            }
            // Proposition claims are intentionally retrievable across snapshots.
            // Retrieval alone does not establish applicability to this edge.
            graph
                .relation_claims
                .push(entmoot_contracts::relation_claim_with_id(
                    entmoot_contracts::SubjectId::Proposition {
                        proposition_id: edge.proposition_id.clone(),
                    },
                    predicate,
                    value,
                    producer.clone(),
                    claim_scope,
                    vec![],
                    None,
                ));
            graph.relation_edges.push(edge);
            repository
                .replace_all(&KnowledgeStore::extract_from_graph(&graph))
                .unwrap();
            let service = QueryEngine::new(graph, store);
            let result = service
                .walk_path_receipt(&repository, 1, "b", &[0], 20)
                .unwrap();
            assert_eq!(result["classification"], expected, "case {index}");
            let cold = service
                .bounded_walk("a", "b", &["calls".into()], BoundedWalkBudget::default())
                .unwrap();
            assert_eq!(cold["paths"][0]["classification"], expected);
            repository.replace_all(&KnowledgeStore::default()).unwrap();
            let warm = service
                .bounded_walk("a", "b", &["calls".into()], BoundedWalkBudget::default())
                .unwrap();
            assert_eq!(warm["search_receipt"]["reused_discovery"], true);
            assert_eq!(warm["paths"][0]["classification"], "candidate");
            repository
                .replace_all(&KnowledgeStore::extract_from_graph(
                    service.graph().unwrap(),
                ))
                .unwrap();
            assert_eq!(result["hops"][0]["relation_claim_count"], 1);
            assert_eq!(
                result["hops"][0]["relation_claim_ids"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            let filtered = service
                .traverse(TraverseQuery::Neighborhood {
                    endpoint_selector: None,
                    prune: Default::default(),
                    resumable: false,
                    continuation: None,
                    start: "a".into(),
                    direction: CallDirection::Callees,
                    predicates: vec!["calls".into()],
                    evidence_filter: entmoot_contracts::query::HopEvidenceFilter::ResolvedOnly,
                    endpoint_role: None,
                    budget: BoundedWalkBudget::default(),
                })
                .unwrap()
                .into_value()
                .unwrap();
            assert_eq!(
                !filtered["items"].as_array().unwrap().is_empty(),
                expected == "supported"
            );
            let envelope = service
                .traverse(TraverseQuery::Path {
                    evidence_filter: Default::default(),
                    prune: Default::default(),
                    resumable: false,
                    continuation: None,
                    start: "a".into(),
                    target: "b".into(),
                    predicates: vec!["calls".into()],
                    budget: BoundedWalkBudget::default(),
                })
                .unwrap()
                .into_value()
                .unwrap();
            if expected == "candidate" {
                assert_eq!(
                    envelope["receipt"]["relations"]["active_resolution_states"],
                    json!([])
                );
            }
        }
    }

    #[test]
    fn path_receipts_retain_provider_properties_without_deriving_flow() {
        let directory = tempfile::tempdir().unwrap();
        let store_path = directory.path().join("path.sqlite3");
        let repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        let mut graph = TopologyGraph::new("test/path");
        let subject = entmoot_contracts::topology_node_address("a");
        let edge = entmoot_contracts::relation_edge_with_id(
            "source",
            subject,
            RelationKind::Calls,
            entmoot_contracts::topology_node_address("b"),
            entmoot_contracts::Scope {
                domain: "test/path".into(),
                snapshot_revision: None,
                snapshot_digest: None,
                occurrence_id: None,
                context_id: None,
            },
            None,
        );
        graph.relation_edges.push(edge);
        graph.assertions.push(serde_json::from_value(json!({
            "assertion_id": "provider-sanitizer", "subject": {"entity_kind": "callable", "entity_id": "a"},
            "predicate": "is_sanitizer", "object": {"value": "provider-label"},
            "epistemic_status": "observed"
        })).unwrap());
        let service = QueryEngine::new(graph, store_path);
        let receipt = service
            .walk_path_receipt(&repository, 1, "b", &[0], 20)
            .unwrap();
        assert_eq!(
            receipt["hops"][0]["endpoint_assertions"][0]["assertion_id"],
            "provider-sanitizer"
        );
        assert_eq!(receipt["classification"], "candidate");
        let compact = service
            .walk_path_receipt(&repository, 1, "b", &[0], 0)
            .unwrap();
        assert_eq!(compact["classification"], receipt["classification"]);
        assert_eq!(compact["hops"][0]["endpoint_assertions"], json!([]));
        assert_eq!(
            compact["hops"][0]["endpoint_assertion_bounds"]["total_matches"],
            1
        );
        assert_eq!(
            compact["hops"][0]["endpoint_assertion_bounds"]["truncated"],
            true
        );
    }

    #[test]
    fn subject_context_exposes_configuration_and_source_failures_as_gaps() {
        let directory = tempfile::tempdir().unwrap();
        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository.replace_all(&KnowledgeStore::default()).unwrap();
        drop(repository);

        let mut graph = TopologyGraph::new("example/query");
        let mut node = TopologyNode::shell("broken-file", NodeKind::File, "broken.yaml", None);
        node.path = Some("deploy/broken.yaml".into());
        node.display_ref = "deploy/broken.yaml".into();
        graph.nodes.push(node);
        graph.diagnostics.push(crate::ArtifactDiagnostic {
            code: "config.parse_failed".into(),
            severity: crate::DiagnosticSeverity::Error,
            message: "configuration inventory skipped for this file".into(),
            artifact_id: None,
            provider_pointer: Some("deploy/broken.yaml".into()),
        });
        graph.diagnostics.push(crate::ArtifactDiagnostic {
            code: "source.hash_failed".into(),
            severity: crate::DiagnosticSeverity::Error,
            message: "source inventory skipped for this file".into(),
            artifact_id: None,
            provider_pointer: Some("deploy/broken.yaml".into()),
        });
        graph.rebuild_indexes();

        let context = QueryEngine::new(graph, store_path)
            .get_subject_context("broken-file", Some(20))
            .unwrap();
        assert_eq!(context["diagnostics"][0]["code"], "config.parse_failed");
        assert_eq!(context["gaps"][0], "configuration_parse_failed");
        assert_eq!(context["gaps"][1], "source_inventory_failed");
    }

    impl SourceStore for MemorySourceStore {
        fn observe(&self) -> Result<SourceObservation, SourceStoreError> {
            Ok(SourceObservation {
                repository_id: "memory/query".into(),
                revision: Some("a".repeat(40)),
                source_state: entmoot_contracts::SourceState::Committed,
            })
        }

        fn read_path(
            &self,
            request: SourceReadRequest<'_>,
        ) -> Result<SourceFile, SourceStoreError> {
            let bytes =
                self.files.get(request.path).cloned().ok_or_else(|| {
                    SourceStoreError::SnapshotUnavailable(request.path.to_owned())
                })?;
            let observed = entmoot_contracts::blake3_hex(&bytes);
            if observed != request.expected_content_digest {
                return Err(SourceStoreError::ContentDigestMismatch {
                    path: request.path.to_owned(),
                    expected: request.expected_content_digest.to_owned(),
                    observed,
                });
            }
            Ok(SourceFile {
                path: request.path.to_owned(),
                content_digest: observed,
                bytes,
            })
        }
    }

    #[test]
    fn discovery_is_bounded_and_reports_ambiguity() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("main.rs"),
            "fn same() {}\nfn caller() { same(); }\n",
        )
        .unwrap();
        let graph = build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("example/query"),
            },
        )
        .unwrap();
        let relation_source_key = address_identity_key(&graph.relation_edges[0].subject);
        let relation_source_node_id = graph
            .nodes
            .iter()
            .find(|node| {
                node_addresses(&graph, node)
                    .iter()
                    .any(|address| address_identity_key(address) == relation_source_key)
            })
            .map(|node| node.id.clone())
            .unwrap();
        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        let service = QueryEngine::new(graph, store_path);
        let result = service
            .resolve_code_subject(Some("same"), None, None, Some(10), None)
            .unwrap();
        assert_eq!(result["status"], "known");
        assert!(result["items"].as_array().unwrap().len() >= 2);
        service.finish_result(result).expect("typed subject result");
        let truncated = service
            .resolve_code_subject(Some("same"), None, None, Some(1), None)
            .unwrap();
        assert_eq!(truncated["status"], "truncated");
        assert_eq!(truncated["bounds"]["truncated"], true);
        let inspected = service
            .inspect(InspectQuery::Subject {
                id: relation_source_node_id,
                limit: Some(10),
            })
            .unwrap();
        // Aggregate proposition support and its exact syntactic occurrence.
        assert_eq!(inspected.receipt.relations.relation_edge_count, 2);
        assert!(inspected.receipt.relations.relation_claim_count > 0);
        assert!(!inspected.receipt.references.relation_claim_ids.is_empty());
    }

    #[test]
    fn subject_find_matches_exact_structural_hash_forms() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("main.rs"), "fn target() {}\n").unwrap();
        let graph = build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("example/query"),
            },
        )
        .unwrap();
        let callable = graph
            .nodes
            .iter()
            .find(|node| node.label == "target" && node.structural_hash.is_some())
            .expect("hashed callable");
        let structural_hash = callable.structural_hash.clone().expect("structural_hash");
        let bare = structural_hash
            .strip_prefix(STRUCTURAL_HASH_PREFIX)
            .expect("blake3 prefix")
            .to_owned();
        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        let service = QueryEngine::new(graph, store_path);

        for query in [
            structural_hash.as_str(),
            bare.as_str(),
            &bare.to_ascii_uppercase(),
        ] {
            let result = service
                .resolve_code_subject(Some(query), None, None, Some(10), None)
                .unwrap();
            assert_eq!(result["status"], "known", "query={query}");
            let items = result["items"].as_array().expect("items");
            assert_eq!(items.len(), 1, "query={query}");
            assert_eq!(items[0]["label"], "target");
            assert_eq!(items[0]["structural_hash"], structural_hash);
            assert_eq!(items[0]["match_score"], 4);
        }

        // Digest-shaped queries must not substring-match unrelated subjects.
        let noise = format!("prefix-{bare}-suffix");
        let missed = service
            .resolve_code_subject(Some(&noise), None, None, Some(10), None)
            .unwrap();
        assert_eq!(missed["status"], "unknown");
        assert!(missed["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn file_findings_remain_direct_and_are_inherited_with_containment_receipts() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("main.rs"),
            "fn caller() { sink(); }\n",
        )
        .unwrap();
        let mut graph = build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("example/query"),
            },
        )
        .unwrap();
        let file_id = crate::graph::file_node_id("main.rs");
        let call_site_id = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::CallStub)
            .map(|node| node.id.clone())
            .unwrap();
        let mut disconnected =
            TopologyNode::shell("disconnected-site", NodeKind::CallStub, "orphan", None);
        disconnected.path = Some("main.rs".into());
        graph.nodes.push(disconnected);
        graph.locations.push(LocationRecord {
            location_id: "file-location".into(),
            artifact_id: "artifact".into(),
            path: "main.rs".into(),
            extent: LocationExtent::File,

            provider_pointer: "/runs/0/results/0/locations/0".into(),
            provider_confidence: None,
        });
        graph.findings.push(FindingObservation {
            observation_id: "file-finding".into(),
            artifact_id: "artifact".into(),
            run_index: 0,
            result_index: 0,
            rule_id: Some("dependency".into()),
            level: None,
            message: None,
            fingerprints: Default::default(),
            taxonomies: vec![],
            primary_location_ids: vec!["file-location".into()],
            related_location_ids: vec![],
            logical_location_ids: vec![],
            trace_ids: vec![],
            suppressions: vec![],
            baseline_state: None,
            fix_pointers: vec![],
            rule_metadata_pointer: None,
        });
        graph.attachments.push(LocationAttachment {
            verified_span: None,
            attachment_id: "file-attachment".into(),
            artifact_id: "artifact".into(),
            location_id: "file-location".into(),
            disposition: AttachmentDisposition::Attached,
            selected_target: Some(AttachmentTarget {
                port: None,
                kind: AttachmentTargetKind::File,
                node_id: Some(file_id.clone()),
                address_id: None,
            }),
            candidate_targets: vec![],
            freshness: AttachmentFreshness::Current,
            reason_codes: vec![AttachmentReason::ExactFile],
            conflicts: vec![],
            location_ref: None,
        });
        graph.rebuild_indexes();

        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository.replace_all(&KnowledgeStore::default()).unwrap();
        drop(repository);
        let service = QueryEngine::new(graph, store_path);

        let file = service.get_subject_context(&file_id, Some(20)).unwrap();
        assert_eq!(
            file["direct_findings"][0]["finding"]["observation_id"],
            "file-finding"
        );
        assert_eq!(
            file["direct_findings"][0]["attachments"][0]["selected_target"]["kind"],
            "file"
        );
        assert!(file["inherited_file_findings"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(file["containment"]["descendant_counts"]["call_sites"], 2);
        let file = service.finish_result(file).expect("typed file aggregate");
        assert_eq!(file.receipt.attachments.attachment_count, 1);
        assert_eq!(
            file.receipt.attachments.selected_grains[0].target_kind,
            AttachmentTargetKind::File
        );
        assert_eq!(file.receipt.references.finding_ids, ["file-finding"]);
        let bounded_file = service.get_subject_context(&file_id, Some(1)).unwrap();
        assert_eq!(bounded_file["status"], "truncated");
        assert_eq!(
            bounded_file["containment"]["contained_call_sites"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let call_site = service
            .get_subject_context(&call_site_id, Some(20))
            .unwrap();
        assert!(call_site["direct_findings"].as_array().unwrap().is_empty());
        assert_eq!(
            call_site["inherited_file_findings"][0]["attached"]["finding"]["observation_id"],
            "file-finding"
        );
        let path = call_site["inherited_file_findings"][0]["structural_path"]
            .as_array()
            .unwrap();
        assert_eq!(path[0]["kind"], "contains");
        assert_eq!(path[1]["kind"], "calls");
        assert_eq!(
            call_site["containment"]["enclosing_callables"][0]["label"],
            "caller"
        );
        service
            .finish_result(call_site)
            .expect("typed inherited file context");

        let disconnected = service
            .get_subject_context("disconnected-site", Some(20))
            .unwrap();
        assert!(disconnected["inherited_file_findings"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(disconnected["gaps"]
            .as_array()
            .unwrap()
            .contains(&json!("containment_path_not_materialized")));
    }

    #[test]
    fn exact_location_and_source_span_use_snapshot_addressed_source_store() {
        let directory = tempfile::tempdir().unwrap();
        let source = "fn callee() {}\nfn caller() {\n    callee();\n}\n";
        std::fs::write(directory.path().join("main.rs"), source).unwrap();
        let graph = build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("example/query"),
            },
        )
        .unwrap();
        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        let source_store = MemorySourceStore {
            files: HashMap::from([("main.rs".into(), source.as_bytes().to_vec())]),
        };
        let service = QueryEngine::new(graph, store_path)
            .with_source_store(std::sync::Arc::new(source_store));

        let resolved = service
            .resolve_code_location("main.rs", 3, Some(5))
            .unwrap();
        assert_eq!(resolved["status"], "known");
        assert_eq!(resolved["selected"][0]["kind"], "call_stub");
        assert_eq!(resolved["selected"][0]["label"], "callee");
        assert_eq!(resolved["enclosing_callables"][0]["label"], "caller");
        let resolved_receipt = service.finish_result(resolved).unwrap().receipt;
        assert_eq!(
            resolved_receipt.source_verification.status,
            entmoot_contracts::query::SourceVerificationStatus::Verified
        );

        let span = service
            .read_source_span("main.rs", 3, Some(3), Some(1), None)
            .unwrap();
        assert_eq!(span["status"], "known");
        assert_eq!(span["items"][1]["line"], 3);
        assert_eq!(span["items"][1]["text"], "    callee();");
        assert_eq!(
            span["content_digest"],
            entmoot_contracts::blake3_hex(source.as_bytes())
        );
        let span_receipt = service.finish_result(span).unwrap().receipt;
        assert_eq!(
            span_receipt.source_verification.status,
            entmoot_contracts::query::SourceVerificationStatus::Verified
        );
    }

    #[test]
    fn finding_trace_is_ordered_and_explain_advertises_trace_records() {
        let directory = tempfile::tempdir().unwrap();
        let source = "fn main() {}\n// trace location\n";
        std::fs::write(directory.path().join("main.rs"), source).unwrap();
        let mut graph = build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("example/query"),
            },
        )
        .unwrap();
        let manifest = entmoot_contracts::ArtifactManifest::build(
            entmoot_contracts::ArtifactFormat::Sarif21,
            entmoot_contracts::ArtifactProducer {
                name: "fixture".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            entmoot_contracts::ScopeUniverse::from_selectors(
                "fixture",
                vec![],
                vec![],
                vec![],
                entmoot_contracts::ScopeCompleteness::Partial,
                None,
            ),
            entmoot_contracts::blake3_hex(b"fixture"),
            None,
        )
        .unwrap();
        let artifact_id = manifest.artifact_id.clone();
        graph.artifacts.push(manifest);
        let finding_id = "finding-1".to_string();
        let trace_id = "trace-1".to_string();
        let location = |id: &str, line: u32| LocationRecord {
            location_id: id.into(),
            artifact_id: artifact_id.clone(),
            path: "main.rs".into(),
            extent: LocationExtent::Region {
                region: SourceRegion::Text {
                    range: entmoot_contracts::SourceTextRange {
                        start: entmoot_contracts::SourcePosition { line, column: 1 },
                        end: entmoot_contracts::SourceTextEnd::Position {
                            position: entmoot_contracts::SourcePosition { line, column: 2 },
                        },
                    },
                    encoding: entmoot_contracts::ColumnEncoding::Utf32CodePoints,
                },
            },
            provider_pointer: format!("/locations/{line}"),
            provider_confidence: None,
        };
        graph
            .locations
            .extend([location("location-1", 1), location("location-2", 2)]);
        graph.findings.push(FindingObservation {
            observation_id: finding_id.clone(),
            artifact_id: artifact_id.clone(),
            run_index: 0,
            result_index: 0,
            rule_id: Some("rule".into()),
            level: None,
            message: None,
            fingerprints: Default::default(),
            taxonomies: vec![],
            primary_location_ids: vec!["location-1".into()],
            related_location_ids: vec![],
            logical_location_ids: vec![],
            trace_ids: vec![trace_id.clone()],
            suppressions: vec![],
            baseline_state: None,
            fix_pointers: vec![],
            rule_metadata_pointer: None,
        });
        graph.traces.push(TraceRecord {
            trace_id: trace_id.clone(),
            finding_observation_id: finding_id.clone(),
            code_flow_index: 0,
            thread_index: 0,
            step_ids: vec!["step-1".into(), "step-2".into()],
        });
        graph.trace_steps.extend([
            TraceStepRecord {
                step_id: "step-2".into(),
                trace_id: trace_id.clone(),
                ordinal: 1,
                location_id: "location-2".into(),
                provider_importance: None,
                provider_message: None,
                provider_kinds: vec!["sink".into()],
            },
            TraceStepRecord {
                step_id: "step-1".into(),
                trace_id: trace_id.clone(),
                ordinal: 0,
                location_id: "location-1".into(),
                provider_importance: None,
                provider_message: None,
                provider_kinds: vec![],
            },
        ]);
        let sources = vec![("main.rs".into(), source.into())];
        let indexes = crate::location_join::AttachmentIndexes::new(&graph, &sources);
        let attachments: Vec<_> = graph
            .locations
            .iter()
            .map(|location| {
                crate::location_join::resolve_location(location, &graph.artifacts[0], &indexes)
            })
            .collect();
        assert!(attachments
            .iter()
            .all(|attachment| attachment.disposition == AttachmentDisposition::Attached));
        graph
            .assertions
            .extend(crate::assertions::derive_attachment_assertions(
                &graph,
                &attachments,
                &[],
                &graph.trace_steps,
            ));
        graph.attachments = attachments;
        graph.validate().unwrap();
        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        drop(repository);
        let service = QueryEngine::new(graph, store_path);

        let result = service
            .get_finding_trace(&finding_id, None, Some(10))
            .unwrap();
        assert_eq!(result["status"], "known");
        assert_eq!(result["bounds"]["total_matches"], 2);
        let typed = service
            .finish_result(result.clone())
            .expect("typed trace result");
        assert_eq!(
            typed.receipt.references.trace_ids,
            std::slice::from_ref(&trace_id)
        );
        assert_eq!(result["items"][0]["step"]["ordinal"], 0);
        assert_eq!(result["items"][1]["step"]["provider_kinds"][0], "sink");

        let explained = service.explain_evidence(&finding_id).unwrap();
        service
            .finish_result(explained.clone())
            .expect("typed evidence result");
        assert_eq!(explained["body"]["traces"][0]["trace_id"], trace_id);

        let cold_graph = SqliteKnowledgeRepository::open_read_snapshot(&service.store_path)
            .unwrap()
            .load_inventory(true)
            .unwrap()
            .unwrap();
        assert!(cold_graph.traces.is_empty());
        let cold = QueryEngine::new(cold_graph, service.store_path.clone());
        let eager_comparison = service
            .clone()
            .with_compare_graph(service.graph().unwrap().clone());
        let cold_comparison = cold
            .clone()
            .with_compare_graph(service.graph().unwrap().clone());
        let mut relay = entmoot_contracts::query::ImpactRelayRequest::default();
        relay.output.limit = 4;
        assert_eq!(
            eager_comparison.relay_impact(relay.clone()).unwrap(),
            cold_comparison.relay_impact(relay).unwrap(),
        );
        let status = entmoot_contracts::query::StatusRequest {
            compare_revision: Some(service.graph().unwrap().snapshot.revision.clone()),
            ..Default::default()
        };
        assert_eq!(
            eager_comparison.status(status.clone()).unwrap(),
            cold_comparison.status(status).unwrap(),
        );
        assert_eq!(
            result,
            cold.get_finding_trace(&finding_id, None, Some(10)).unwrap()
        );
        assert_eq!(explained, cold.explain_evidence(&finding_id).unwrap());

        let roles = service
            .list_evidence_roles(Some("sink"), None, Some(10))
            .unwrap();
        service
            .finish_result(roles.clone())
            .expect("typed role result");
        assert_eq!(roles["status"], "known");
        assert_eq!(
            roles,
            cold.list_evidence_roles(Some("sink"), None, Some(10))
                .unwrap()
        );
        assert_eq!(roles["items"][0]["location"]["location_id"], "location-2");
        assert_eq!(
            roles["items"][0]["findings"][0]["observation_id"],
            finding_id
        );
    }
}
