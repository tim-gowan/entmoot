//! Read snapshots and narrowly scoped continuation storage.
//! Query consumers receive these capabilities rather than a mutable repository.
use crate::knowledge_db::*;
use crate::{KnowledgeStore, TopologyGraph};
use entmoot_contracts::*;
use std::path::{Path, PathBuf};

/// One transactionally pinned, immutable knowledge view.
///
/// ```compile_fail
/// use entmoot_store::{read::KnowledgeRead, KnowledgeStore};
/// fn cannot_publish(reader: &mut dyn KnowledgeRead) {
///     reader.replace_all(&KnowledgeStore::default());
/// }
/// ```
pub trait KnowledgeRead {
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
