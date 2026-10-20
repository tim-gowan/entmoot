//! Read snapshots and narrowly scoped continuation storage.
//! Query consumers receive these capabilities rather than a mutable repository.
use crate::knowledge_db::*;
use crate::{KnowledgeStore, TopologyGraph};
use entmoot_contracts::*;
use std::path::{Path, PathBuf};

/// Small publication header sufficient for source reads and deferred navigation.
/// It deliberately cannot be mistaken for a complete inventory projection.
#[derive(Debug, Clone)]
pub struct InventoryMetadata {
    pub root: String,
    pub snapshot: SourceSnapshot,
    pub policy_ids: Vec<String>,
    pub snapshot_files: Vec<crate::SnapshotFileDigest>,
}

/// Internal keyset page over an immutable inventory. Ordinals are storage
/// positions, never public identities or portable cursors.
pub struct InventoryNodePage {
    pub nodes: Vec<crate::TopologyNode>,
    pub next_ordinal: Option<i64>,
}

/// Exact counts accompany a bounded keyset page over retained relation edges.
pub struct RelationReadPage {
    pub edges: Vec<RelationEdge>,
    pub offset: usize,
    pub total: usize,
}

/// The collection commitment is streamed across the complete selected attempt
/// set. Only this page is retained in memory; absence never implies resolution.
pub struct UnresolvedReadPage {
    pub items: Vec<UnresolvedRelation>,
    pub offset: usize,
    pub total: usize,
    pub collection_digest: ContentDigest,
    pub after_present: bool,
}

/// Evidence required to interpret selected edges, including incoming explicit
/// contradictions. Coverage retains one lookahead item for bounded receipts.
pub struct RelationReadContext {
    pub edge_claims: Vec<Vec<RelationClaim>>,
    pub related_claims: Vec<RelationClaim>,
    pub artifacts: Vec<ArtifactManifest>,
    pub resolver_coverages: Vec<ResolverCoverageObservation>,
}

/// One transactionally pinned, immutable knowledge view.
///
/// ```compile_fail
/// use entmoot_store::{read::KnowledgeRead, KnowledgeStore};
/// fn cannot_publish(reader: &mut dyn KnowledgeRead) {
///     reader.replace_all(&KnowledgeStore::default());
/// }
/// ```
pub trait KnowledgeRead {
    fn retained_call_edges(
        &self,
        _addresses: &[Address],
        _direction: query::CallDirection,
        _after: Option<&str>,
        _limit: usize,
    ) -> Result<Option<RelationReadPage>, KnowledgeRepositoryError> {
        Ok(None)
    }

    fn retained_unresolved(
        &self,
        _addresses: &[Address],
        _after: Option<&str>,
        _limit: usize,
    ) -> Result<Option<UnresolvedReadPage>, KnowledgeRepositoryError> {
        Ok(None)
    }

    fn retained_relation_context(
        &self,
        _edges: &[&RelationEdge],
        _artifact_ids: &[String],
        _coverage_limit: usize,
    ) -> Result<Option<RelationReadContext>, KnowledgeRepositoryError> {
        Ok(None)
    }

    fn inventory_resource(
        &self,
        _id: &str,
    ) -> Result<Option<ResourceRecord>, KnowledgeRepositoryError> {
        Ok(None)
    }
    /// Indexed exact identities, including all supplied hash spellings. An
    /// unsupported backend returns None, never a false empty match.
    fn inventory_nodes_matching(
        &self,
        path: Option<&str>,
        exact: &[String],
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<InventoryNodePage>, KnowledgeRepositoryError> {
        if exact.is_empty() {
            self.inventory_nodes(path, after, limit)
        } else {
            Ok(None)
        }
    }

    /// Path narrowing happens before hydration. None means unsupported by this
    /// backend; it does not establish an empty inventory.
    fn inventory_nodes(
        &self,
        _path: Option<&str>,
        _after: Option<i64>,
        _limit: usize,
    ) -> Result<Option<InventoryNodePage>, KnowledgeRepositoryError> {
        Ok(None)
    }

    /// Structural edges incident to the supplied node ids. None means unsupported.
    fn inventory_structural_edges_for_nodes(
        &self,
        _node_ids: &[String],
    ) -> Result<Option<Vec<crate::StructuralEdge>>, KnowledgeRepositoryError> {
        Ok(None)
    }

    /// Nodes under exact or nested path prefixes. Empty prefixes yield an empty
    /// page; None means the backend cannot answer.
    fn inventory_nodes_under_prefixes(
        &self,
        _prefixes: &[String],
        _after: Option<i64>,
        _limit: usize,
    ) -> Result<Option<InventoryNodePage>, KnowledgeRepositoryError> {
        Ok(None)
    }

    /// None means this backend does not retain a source inventory header.
    fn inventory_metadata(&self) -> Result<Option<InventoryMetadata>, KnowledgeRepositoryError> {
        Ok(None)
    }

    fn query_checkpoint(
        &self,
        key: &str,
        generation: &str,
        now_ms: u64,
    ) -> Result<Option<(usize, Vec<u8>)>, OperationError>;
    fn inventory_export_digest(&self) -> Result<Option<String>, KnowledgeRepositoryError>;
    fn read_state_digest(
        &self,
    ) -> Result<entmoot_contracts::ContentDigest, KnowledgeRepositoryError>;
    fn relation_claim_by_id(
        &self,
        id: &str,
    ) -> Result<Option<RelationClaim>, KnowledgeRepositoryError>;
    fn relation_claims_for_address(
        &self,
        address: &Address,
        predicate: &str,
        limit: usize,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError>;
    /// Whether this repository contains a successfully committed snapshot.
    ///
    /// An existing database file is not sufficient: SQLite creates the file
    /// before the first knowledge transaction commits.
    fn is_initialized(&self) -> Result<bool, KnowledgeRepositoryError>;
    fn load(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError>;
    fn assertions_for_subject(
        &self,
        subject_id: &str,
    ) -> Result<Vec<Assertion>, KnowledgeRepositoryError>;
    fn relation_edges_for_subject(
        &self,
        subject: &Address,
    ) -> Result<Vec<RelationEdge>, KnowledgeRepositoryError>;
    fn proposition_by_id(
        &self,
        proposition_id: &str,
    ) -> Result<Option<Proposition>, KnowledgeRepositoryError>;
    fn propositions_for_subject(
        &self,
        subject: &Address,
    ) -> Result<Vec<Proposition>, KnowledgeRepositoryError>;
    fn propositions_for_object(
        &self,
        object: &Address,
    ) -> Result<Vec<Proposition>, KnowledgeRepositoryError>;
    fn relation_edges_for_proposition(
        &self,
        proposition_id: &str,
    ) -> Result<Vec<RelationEdge>, KnowledgeRepositoryError>;
    fn relation_claims_for_relation_edge(
        &self,
        relation_edge_id: &str,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError>;
    fn relation_claims_for_proposition(
        &self,
        proposition_id: &str,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError>;
    /// Batch exact-edge and proposition evidence in request order. Evidence is
    /// still checked against each original edge's scope by query consumers.
    fn relation_claims_for_edges(
        &self,
        edges: &[&RelationEdge],
    ) -> Result<Vec<Vec<RelationClaim>>, KnowledgeRepositoryError>;
    fn unresolved_for_source(
        &self,
        source: &Address,
    ) -> Result<Vec<UnresolvedRelation>, KnowledgeRepositoryError>;
    fn unresolved_by_id(
        &self,
        unresolved_id: &str,
    ) -> Result<Option<UnresolvedRelation>, KnowledgeRepositoryError>;
    fn artifact_by_id(
        &self,
        artifact_id: &str,
    ) -> Result<Option<ArtifactManifest>, KnowledgeRepositoryError>;
    fn trace_by_id(&self, trace_id: &str) -> Result<Option<TraceRecord>, KnowledgeRepositoryError>;
    fn trace_steps_for_trace(
        &self,
        trace_id: &str,
        max_steps: usize,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError>;
    fn resources_by_logical_id(
        &self,
        resource_id: &str,
    ) -> Result<Vec<entmoot_contracts::ResourceRecord>, KnowledgeRepositoryError>;
    fn records_for_owner(
        &self,
        owner_id: &str,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError>;
    fn owners_for_record(
        &self,
        record: &KnowledgeRecordRef,
        max_owners: usize,
    ) -> Result<Vec<String>, KnowledgeRepositoryError>;
    fn record_state(
        &self,
        record: &KnowledgeRecordRef,
    ) -> Result<Option<KnowledgeRecordState>, KnowledgeRepositoryError>;
    fn dependency_inputs_for_record(
        &self,
        record: &KnowledgeRecordRef,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError>;
    fn dependent_records_for_input(
        &self,
        record: &KnowledgeRecordRef,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError>;
    /// Read a bounded page from the backend-local visibility transition log.
    fn visibility_changes(
        &self,
        after_cursor: Option<u64>,
        max_events: usize,
    ) -> Result<LocalVisibilityPage, KnowledgeRepositoryError>;
    /// Return the current backend-local visibility event cursor. This orders
    /// local observations but is not a semantic or cross-backend identity.
    fn current_visibility_cursor(&self) -> Result<u64, KnowledgeRepositoryError>;
    fn load_inventory(
        &self,
        query_only: bool,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError>;
    fn load_assessment_inventory(&self) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError>;
    fn load_assessment(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError>;
    fn trace_step_by_id(
        &self,
        id: &str,
    ) -> Result<Option<TraceStepRecord>, KnowledgeRepositoryError>;
    fn trace_steps_at_location(
        &self,
        id: &str,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError>;
}
impl KnowledgeRead for Box<dyn KnowledgeRead> {
    fn retained_call_edges(
        &self,
        addresses: &[Address],
        direction: query::CallDirection,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Option<RelationReadPage>, KnowledgeRepositoryError> {
        self.as_ref()
            .retained_call_edges(addresses, direction, after, limit)
    }
    fn retained_unresolved(
        &self,
        addresses: &[Address],
        after: Option<&str>,
        limit: usize,
    ) -> Result<Option<UnresolvedReadPage>, KnowledgeRepositoryError> {
        self.as_ref().retained_unresolved(addresses, after, limit)
    }
    fn retained_relation_context(
        &self,
        edges: &[&RelationEdge],
        artifact_ids: &[String],
        coverage_limit: usize,
    ) -> Result<Option<RelationReadContext>, KnowledgeRepositoryError> {
        self.as_ref()
            .retained_relation_context(edges, artifact_ids, coverage_limit)
    }
    fn inventory_resource(
        &self,
        id: &str,
    ) -> Result<Option<ResourceRecord>, KnowledgeRepositoryError> {
        self.as_ref().inventory_resource(id)
    }
    fn inventory_nodes_matching(
        &self,
        path: Option<&str>,
        exact: &[String],
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<InventoryNodePage>, KnowledgeRepositoryError> {
        self.as_ref()
            .inventory_nodes_matching(path, exact, after, limit)
    }

    fn inventory_nodes(
        &self,
        path: Option<&str>,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<InventoryNodePage>, KnowledgeRepositoryError> {
        self.as_ref().inventory_nodes(path, after, limit)
    }

    fn inventory_structural_edges_for_nodes(
        &self,
        node_ids: &[String],
    ) -> Result<Option<Vec<crate::StructuralEdge>>, KnowledgeRepositoryError> {
        self.as_ref().inventory_structural_edges_for_nodes(node_ids)
    }

    fn inventory_nodes_under_prefixes(
        &self,
        prefixes: &[String],
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<InventoryNodePage>, KnowledgeRepositoryError> {
        self.as_ref()
            .inventory_nodes_under_prefixes(prefixes, after, limit)
    }

    fn inventory_metadata(&self) -> Result<Option<InventoryMetadata>, KnowledgeRepositoryError> {
        self.as_ref().inventory_metadata()
    }

    fn query_checkpoint(
        &self,
        key: &str,
        generation: &str,
        now_ms: u64,
    ) -> Result<Option<(usize, Vec<u8>)>, OperationError> {
        self.as_ref().query_checkpoint(key, generation, now_ms)
    }
    fn inventory_export_digest(&self) -> Result<Option<String>, KnowledgeRepositoryError> {
        self.as_ref().inventory_export_digest()
    }
    fn read_state_digest(
        &self,
    ) -> Result<entmoot_contracts::ContentDigest, KnowledgeRepositoryError> {
        self.as_ref().read_state_digest()
    }
    fn relation_claim_by_id(
        &self,
        id: &str,
    ) -> Result<Option<RelationClaim>, KnowledgeRepositoryError> {
        self.as_ref().relation_claim_by_id(id)
    }
    fn relation_claims_for_address(
        &self,
        address: &Address,
        predicate: &str,
        limit: usize,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        self.as_ref()
            .relation_claims_for_address(address, predicate, limit)
    }
    fn is_initialized(&self) -> Result<bool, KnowledgeRepositoryError> {
        self.as_ref().is_initialized()
    }
    fn load(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError> {
        self.as_ref().load()
    }
    fn assertions_for_subject(
        &self,
        subject_id: &str,
    ) -> Result<Vec<Assertion>, KnowledgeRepositoryError> {
        self.as_ref().assertions_for_subject(subject_id)
    }
    fn relation_edges_for_subject(
        &self,
        subject: &Address,
    ) -> Result<Vec<RelationEdge>, KnowledgeRepositoryError> {
        self.as_ref().relation_edges_for_subject(subject)
    }
    fn proposition_by_id(
        &self,
        proposition_id: &str,
    ) -> Result<Option<Proposition>, KnowledgeRepositoryError> {
        self.as_ref().proposition_by_id(proposition_id)
    }
    fn propositions_for_subject(
        &self,
        subject: &Address,
    ) -> Result<Vec<Proposition>, KnowledgeRepositoryError> {
        self.as_ref().propositions_for_subject(subject)
    }
    fn propositions_for_object(
        &self,
        object: &Address,
    ) -> Result<Vec<Proposition>, KnowledgeRepositoryError> {
        self.as_ref().propositions_for_object(object)
    }
    fn relation_edges_for_proposition(
        &self,
        proposition_id: &str,
    ) -> Result<Vec<RelationEdge>, KnowledgeRepositoryError> {
        self.as_ref().relation_edges_for_proposition(proposition_id)
    }
    fn relation_claims_for_relation_edge(
        &self,
        relation_edge_id: &str,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        self.as_ref()
            .relation_claims_for_relation_edge(relation_edge_id)
    }
    fn relation_claims_for_proposition(
        &self,
        proposition_id: &str,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        self.as_ref()
            .relation_claims_for_proposition(proposition_id)
    }
    fn relation_claims_for_edges(
        &self,
        edges: &[&RelationEdge],
    ) -> Result<Vec<Vec<RelationClaim>>, KnowledgeRepositoryError> {
        self.as_ref().relation_claims_for_edges(edges)
    }
    fn unresolved_for_source(
        &self,
        source: &Address,
    ) -> Result<Vec<UnresolvedRelation>, KnowledgeRepositoryError> {
        self.as_ref().unresolved_for_source(source)
    }
    fn unresolved_by_id(
        &self,
        unresolved_id: &str,
    ) -> Result<Option<UnresolvedRelation>, KnowledgeRepositoryError> {
        self.as_ref().unresolved_by_id(unresolved_id)
    }
    fn artifact_by_id(
        &self,
        artifact_id: &str,
    ) -> Result<Option<ArtifactManifest>, KnowledgeRepositoryError> {
        self.as_ref().artifact_by_id(artifact_id)
    }
    fn trace_by_id(&self, trace_id: &str) -> Result<Option<TraceRecord>, KnowledgeRepositoryError> {
        self.as_ref().trace_by_id(trace_id)
    }
    fn trace_steps_for_trace(
        &self,
        trace_id: &str,
        max_steps: usize,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError> {
        self.as_ref().trace_steps_for_trace(trace_id, max_steps)
    }
    fn resources_by_logical_id(
        &self,
        resource_id: &str,
    ) -> Result<Vec<entmoot_contracts::ResourceRecord>, KnowledgeRepositoryError> {
        self.as_ref().resources_by_logical_id(resource_id)
    }
    fn records_for_owner(
        &self,
        owner_id: &str,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
        self.as_ref().records_for_owner(owner_id, max_records)
    }
    fn owners_for_record(
        &self,
        record: &KnowledgeRecordRef,
        max_owners: usize,
    ) -> Result<Vec<String>, KnowledgeRepositoryError> {
        self.as_ref().owners_for_record(record, max_owners)
    }
    fn record_state(
        &self,
        record: &KnowledgeRecordRef,
    ) -> Result<Option<KnowledgeRecordState>, KnowledgeRepositoryError> {
        self.as_ref().record_state(record)
    }
    fn dependency_inputs_for_record(
        &self,
        record: &KnowledgeRecordRef,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
        self.as_ref()
            .dependency_inputs_for_record(record, max_records)
    }
    fn dependent_records_for_input(
        &self,
        record: &KnowledgeRecordRef,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
        self.as_ref()
            .dependent_records_for_input(record, max_records)
    }
    fn visibility_changes(
        &self,
        after_cursor: Option<u64>,
        max_events: usize,
    ) -> Result<LocalVisibilityPage, KnowledgeRepositoryError> {
        self.as_ref().visibility_changes(after_cursor, max_events)
    }
    fn current_visibility_cursor(&self) -> Result<u64, KnowledgeRepositoryError> {
        self.as_ref().current_visibility_cursor()
    }
    fn load_inventory(
        &self,
        query_only: bool,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        self.as_ref().load_inventory(query_only)
    }
    fn load_assessment_inventory(&self) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        self.as_ref().load_assessment_inventory()
    }
    fn load_assessment(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError> {
        self.as_ref().load_assessment()
    }
    fn trace_step_by_id(
        &self,
        id: &str,
    ) -> Result<Option<TraceStepRecord>, KnowledgeRepositoryError> {
        self.as_ref().trace_step_by_id(id)
    }
    fn trace_steps_at_location(
        &self,
        id: &str,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError> {
        self.as_ref().trace_steps_at_location(id)
    }
}

/// Opens a fresh pinned read transaction for each operation; safe to share
/// across request threads without sharing a database connection.
pub trait RepositoryReader: Send + Sync {
    fn open_read(&self) -> Result<Box<dyn KnowledgeRead>, KnowledgeRepositoryError>;
    fn exists(&self) -> bool;
}

/// Operational checkpoints are separate from immutable knowledge publication.
pub trait QueryCheckpoints: Send + Sync {
    fn load_checkpoint(
        &self,
        key: &str,
        generation: &str,
        now_ms: u64,
    ) -> Result<Option<(usize, Vec<u8>)>, OperationError>;
    #[allow(clippy::too_many_arguments)]
    fn save_checkpoint(
        &self,
        key: &str,
        generation: &str,
        expected: usize,
        sequence: usize,
        expires_ms: u64,
        body: &[u8],
        capacity: usize,
        bytes: usize,
    ) -> Result<(), OperationError>;
}

/// SQLite implementation of the read and checkpoint capabilities.
pub struct SqliteReader {
    path: PathBuf,
}
impl SqliteReader {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_owned(),
        }
    }
}
impl RepositoryReader for SqliteReader {
    fn open_read(&self) -> Result<Box<dyn KnowledgeRead>, KnowledgeRepositoryError> {
        Ok(Box::new(SqliteKnowledgeRepository::open_read_snapshot(
            &self.path,
        )?))
    }
    fn exists(&self) -> bool {
        self.path.exists()
    }
}
impl QueryCheckpoints for SqliteReader {
    fn load_checkpoint(
        &self,
        key: &str,
        generation: &str,
        now_ms: u64,
    ) -> Result<Option<(usize, Vec<u8>)>, OperationError> {
        SqliteKnowledgeRepository::open_read_snapshot(&self.path)?
            .query_checkpoint(key, generation, now_ms)
    }
    fn save_checkpoint(
        &self,
        key: &str,
        generation: &str,
        expected: usize,
        sequence: usize,
        expires_ms: u64,
        body: &[u8],
        capacity: usize,
        bytes: usize,
    ) -> Result<(), OperationError> {
        SqliteKnowledgeRepository::open(&self.path)?.save_query_checkpoint(
            key, generation, expected, sequence, expires_ms, body, capacity, bytes,
        )
    }
}
