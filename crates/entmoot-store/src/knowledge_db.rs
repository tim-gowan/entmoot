//! Transactional relational persistence for the Entmoot knowledge plane.
//!
//! SQLite is a physical implementation behind an Entmoot-owned port. JSON payloads
//! preserve the versioned wire contracts; indexed columns provide bounded lookup
//! without making SQLite row ids semantic identities.

pub use crate::read::KnowledgeRead;
use crate::TopologyGraph;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use entmoot_contracts::{
    address_identity_key, Address, ArtifactId, ArtifactManifest, Assertion, LocationRecord,
    Proposition, PropositionId, RelationClaim, RelationEdge, RelationEdgeId, TraceRecord,
    TraceStepRecord, UnresolvedRelation, UnresolvedRelationId,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use crate::knowledge::KnowledgeStore;

const STORAGE_SCHEMA_VERSION: i64 = 1;
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadProjection {
    Complete,
    Query,
    Assessment,
}

mod fingerprint;
mod operations;
pub use fingerprint::current as storage_fingerprint;
pub use operations::{OperationError, OperationRepository};
mod file_facts;
mod indexed_reads;
mod inventory;
mod layout;
pub mod retention;
mod structural;
mod structural_files;
pub mod syntax_cache;
pub use inventory::KnowledgeViewLocator;
use layout::{hydrate_payload, hydrate_record, intern_payload, payload_matches};

mod buffered;
mod buffered_stage;
pub use buffered::{BufferedPublicationReceipt, OwnerBatch, PublicationBuffer, PublicationOutcome};
mod store_batches;
pub use store_batches::{GraphBatches, KnowledgeStoreBatches};

#[derive(Debug, Error)]
pub enum KnowledgeRepositoryError {
    #[error("knowledge store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite knowledge store error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("knowledge record serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("unsupported knowledge-store schema version {actual}; expected {supported}")]
    UnsupportedSchema { actual: i64, supported: i64 },
    #[error("incompatible knowledge-store {component} fingerprint (actual {actual:?}, expected {expected}); regenerate from retained inputs; the store was not reset")]
    IncompatibleFingerprint {
        component: &'static str,
        actual: Option<String>,
        expected: String,
    },
    #[error("knowledge store has no committed snapshot (view missing or expired); reopen a retained publication")]
    NotInitialized,
    #[error("invalid knowledge {collection} record: {reason}")]
    InvalidRecord {
        collection: &'static str,
        reason: String,
    },
    #[error("invalid knowledge query address: {0}")]
    InvalidQueryAddress(String),
    #[error("invalid knowledge query artifact id: {0}")]
    InvalidQueryArtifactId(String),
    #[error("invalid knowledge query unresolved relation id: {0}")]
    InvalidQueryUnresolvedId(String),
    #[error("invalid knowledge query relation edge id: {0}")]
    InvalidQueryRelationEdgeId(String),
    #[error("invalid knowledge query proposition id: {0}")]
    InvalidQueryPropositionId(String),
    #[error("knowledge owner id must be non-empty")]
    InvalidOwner,
    #[error("knowledge record reference is invalid: {0}")]
    InvalidRecordRef(String),
    #[error("immutable knowledge record {collection}/{record_id} has conflicting payloads")]
    ImmutableRecordCollision {
        collection: String,
        record_id: String,
    },
    #[error("knowledge dependency input {collection}/{record_id} is missing")]
    MissingDependency {
        collection: String,
        record_id: String,
    },
    #[error("visibility page size must be greater than zero")]
    InvalidPageSize,
    #[error("knowledge backend returned an invalid count: {0}")]
    BackendCount(String),
    #[error("publication buffer exceeded or invalid: {0}")]
    BufferLimit(&'static str),
}

impl From<entmoot_contracts::KnowledgeRecordRefError> for KnowledgeRepositoryError {
    fn from(error: entmoot_contracts::KnowledgeRecordRefError) -> Self {
        Self::InvalidRecordRef(error.to_string())
    }
}

/// Durable source-of-record boundary. Physical backends must preserve semantic ids.
pub trait KnowledgeRepository: crate::read::KnowledgeRead {
    /// Pull bounded normalized batches and publish one owner atomically. Iterator
    /// failure cancels publication. Dependency inputs may appear in later batches.
    fn replace_owner_buffered(
        &mut self,
        owner_id: &str,
        batches: &mut dyn Iterator<Item = Result<OwnerBatch, KnowledgeRepositoryError>>,
        buffer: PublicationBuffer,
    ) -> Result<BufferedPublicationReceipt, KnowledgeRepositoryError>;
    /// Atomically replace independent owners, in dependency order. Failure leaves
    /// all prior records, ownership, and visibility events intact.
    fn replace_owners(
        &mut self,
        publications: &[OwnerReplacement<'_>],
    ) -> Result<Vec<KnowledgeOwnershipStats>, KnowledgeRepositoryError>;
    fn replace_all(&mut self, store: &KnowledgeStore) -> Result<(), KnowledgeRepositoryError>;
    fn synchronize(
        &mut self,
        store: &KnowledgeStore,
    ) -> Result<KnowledgeSyncStats, KnowledgeRepositoryError>;
    /// Replace every immutable record independently regenerated by one owner.
    fn replace_owner(
        &mut self,
        owner_id: &str,
        store: &KnowledgeStore,
        dependencies: &[KnowledgeDependency],
    ) -> Result<KnowledgeOwnershipStats, KnowledgeRepositoryError>;
    /// Remove one owner without deleting immutable record history.
    fn remove_owner(
        &mut self,
        owner_id: &str,
    ) -> Result<KnowledgeOwnershipStats, KnowledgeRepositoryError>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KnowledgeSyncStats {
    pub upserted: usize,
    pub deleted: usize,
}

pub use entmoot_contracts::KnowledgeRecordRef;

/// Storage state for one retained immutable knowledge record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeRecordState {
    pub visible: bool,
    pub ownership_managed: bool,
}

/// AND prerequisites for one derived record. Independent producers remain OR
/// owners and are represented separately by [`KnowledgeRepository::replace_owner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeDependency {
    pub dependent: KnowledgeRecordRef,
    pub inputs: Vec<KnowledgeRecordRef>,
}

/// Reference-engine publication batch item; borrows immutable staged records.
pub struct OwnerReplacement<'a> {
    pub owner_id: &'a str,
    pub store: &'a KnowledgeStore,
    pub dependencies: &'a [KnowledgeDependency],
}

pub fn record_references(
    store: &KnowledgeStore,
) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
    let mut refs = Vec::new();
    for_each_store_row(store, |row| {
        refs.push(KnowledgeRecordRef::new(row.collection, row.record_id));
        Ok(())
    })?;
    refs.sort();
    refs.dedup();
    Ok(refs)
}

impl KnowledgeDependency {
    pub fn new(dependent: KnowledgeRecordRef, mut inputs: Vec<KnowledgeRecordRef>) -> Self {
        inputs.sort();
        inputs.dedup();
        Self { dependent, inputs }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KnowledgeOwnershipStats {
    pub inserted_records: usize,
    pub added_owners: usize,
    pub removed_owners: usize,
    /// Records evaluated in the changed owner/dependency frontier.
    pub evaluated_records: usize,
    pub became_visible: usize,
    pub became_inactive: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalVisibilityReason {
    Published,
    OwnerRemoved,
    DependencyChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVisibilityChange {
    pub cursor: u64,
    pub record: KnowledgeRecordRef,
    pub visible: bool,
    pub reason: LocalVisibilityReason,
    pub cause_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVisibilityPage {
    pub after_cursor: Option<u64>,
    pub next_cursor: u64,
    pub events: Vec<LocalVisibilityChange>,
    pub truncated: bool,
}

pub struct SqliteKnowledgeRepository {
    connection: Connection,
    lease: Option<crate::storage_lease::StorageLease>,
}

impl SqliteKnowledgeRepository {
    pub fn trace_step_by_id(
        &self,
        id: &str,
    ) -> Result<Option<TraceStepRecord>, KnowledgeRepositoryError> {
        validate_query_token(id)?;
        self.load_record("trace_steps", id)
    }

    pub fn trace_steps_at_location(
        &self,
        id: &str,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError> {
        validate_query_token(id)?;
        let mut statement = self.connection.prepare_cached("SELECT payload FROM knowledge_records_by_location WHERE collection='trace_steps' AND location_id=?1 AND visible=1 ORDER BY record_id")?;
        let mut values = Vec::new();
        for row in statement.query_map([id], |r| r.get::<_, i64>(0))? {
            values.push(hydrate_record(&self.connection, row?)?);
        }
        Ok(values)
    }
    fn load_mode(
        &self,
        projection: ReadProjection,
    ) -> Result<KnowledgeStore, KnowledgeRepositoryError> {
        let query_only = projection == ReadProjection::Query;
        let store = KnowledgeStore {
            code_contracts: self.load_collection("code_contracts")?,
            declaration_sites: self.load_collection("declaration_sites")?,
            call_sites: self.load_collection("call_sites")?,
            coverages: self.load_collection("coverages")?,
            input_roots: self.load_collection("input_roots")?,
            artifacts: self.load_collection("artifacts")?,
            capabilities: self.load_collection("capabilities")?,
            findings: self.load_collection("findings")?,
            resources: self.load_collection("resources")?,
            components: self.load_collection("components")?,
            component_dependencies: self.load_collection("component_dependencies")?,
            component_vulnerabilities: self.load_collection("component_vulnerabilities")?,
            bom_compositions: self.load_collection("bom_compositions")?,
            locations: self.load_collection("locations")?,
            traces: if query_only {
                Vec::new()
            } else {
                self.load_collection("traces")?
            },
            trace_steps: if query_only {
                Vec::new()
            } else {
                self.load_collection("trace_steps")?
            },
            attachments: self.load_collection("attachments")?,
            assertions: self.load_collection("assertions")?,
            resolutions: if query_only {
                Vec::new()
            } else {
                self.load_collection("resolutions")?
            },
            resolver_coverages: self.load_collection("resolver_coverages")?,
            resolved_identities: self.load_collection("resolved_identities")?,
            propositions: self.load_collection("propositions")?,
            relation_edges: self.load_collection("relation_edges")?,
            relation_claims: if projection == ReadProjection::Complete {
                self.load_collection("relation_claims")?
            } else {
                self.load_non_canon_claims()?
            },
            unresolved_relations: self.load_collection("unresolved_relations")?,
            diagnostics: self.load_collection("diagnostics")?,
            structural_edges: self.load_collection("structural_edges")?,
        };
        if !query_only {
            validate_store_records(&store)?;
        }
        Ok(store)
    }

    pub fn load_assessment(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError> {
        self.load_mode(ReadProjection::Assessment)
    }

    fn load_non_canon_claims(&self) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        let mut statement = self.connection.prepare_cached("SELECT payload FROM knowledge_records WHERE collection='relation_claims' AND visible=1 AND predicate NOT IN ('canon.graph','canon.file') ORDER BY record_id")?;
        let mut claims = Vec::new();
        for row in statement.query_map([], |row| row.get::<_, i64>(0))? {
            let claim: RelationClaim = hydrate_record(&self.connection, row?)?;
            claim
                .validate()
                .map_err(|error| invalid_record("relation_claims", error))?;
            claims.push(claim);
        }
        Ok(claims)
    }

    /// Select an independent revision/build publication in a shared database.
    /// The key is a content digest of the publication input manifest; it never
    /// changes the meaning of occurrence-scoped addresses inside that view.
    /// Writable startup holds a per-database initialization lock, released before
    /// returning the repository. Contention has a five-second wait budget; ordinary
    /// reads and transactions retain SQLite's concurrency rules.
    pub fn open_view(path: &Path, view: &str) -> Result<Self, KnowledgeRepositoryError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let lease = crate::storage_lease::shared(path, true)?;
        // Serialize before opening SQLite: overlapping first opens and closes can
        // strand peers in a WAL protocol race even after reopening connections.
        let sqlite_path = crate::sqlite_startup::file_path(path);
        let _initialization = crate::storage_lease::initialization(&sqlite_path)?;
        let mut repository = Self::from_connection_view(Connection::open(&sqlite_path)?, view)?;
        repository.lease = Some(lease);
        Ok(repository)
    }

    pub fn shared_payload_count(&self) -> Result<usize, KnowledgeRepositoryError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM shared_payloads", [], |r| r.get(0))?;
        usize::try_from(count).map_err(|e| KnowledgeRepositoryError::BackendCount(e.to_string()))
    }

    pub fn open(path: &Path) -> Result<Self, KnowledgeRepositoryError> {
        if let Some(locator) = KnowledgeViewLocator::read(path)? {
            return Self::open_view(&locator.database, &locator.view);
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        Self::open_view(path, "default")
    }

    pub fn open_in_memory() -> Result<Self, KnowledgeRepositoryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// Open an existing committed store without creating tables, migrating, or
    /// otherwise changing persistent state.
    pub fn open_read_only(path: &Path) -> Result<Self, KnowledgeRepositoryError> {
        if let Some(locator) = KnowledgeViewLocator::read(path)? {
            return Self::open_view_read_only(&locator.database, &locator.view);
        }
        Self::open_view_read_only(path, "default")
    }

    pub fn open_view_read_only(path: &Path, view: &str) -> Result<Self, KnowledgeRepositoryError> {
        Self::open_view_read_once(path, view, false)
    }

    fn open_view_read_once(
        path: &Path,
        view: &str,
        pin_snapshot: bool,
    ) -> Result<Self, KnowledgeRepositoryError> {
        // Readers must join the same startup gate as writers before the first
        // WAL read. Reopening after SQLITE_PROTOCOL is too late for peers that
        // already raced during shared-memory initialization on Windows.
        let sqlite_path = crate::sqlite_startup::file_path(path);
        let _initialization = crate::storage_lease::initialization_read_only(&sqlite_path)?;
        // Version preflight precedes the lifetime lease so old layouts fail with
        // a schema error instead of an incidental missing sidecar error.
        let connection =
            Connection::open_with_flags(sqlite_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != STORAGE_SCHEMA_VERSION {
            return Err(KnowledgeRepositoryError::UnsupportedSchema {
                actual: version,
                supported: STORAGE_SCHEMA_VERSION,
            });
        }
        fingerprint::verify(&connection)?;
        let lease = crate::storage_lease::shared(path, false)?;
        layout::select_view(&connection, view)?;
        let repository = Self {
            connection,
            lease: Some(lease),
        };
        if pin_snapshot {
            repository.connection.execute_batch("BEGIN DEFERRED")?;
        }
        // BEGIN alone is lazy: this read establishes the WAL end mark.
        if !repository.is_initialized()? {
            return Err(KnowledgeRepositoryError::NotInitialized);
        }
        Ok(repository)
    }

    /// Pin one short multi-statement hydration to a coherent committed state.
    pub fn open_read_snapshot(path: &Path) -> Result<Self, KnowledgeRepositoryError> {
        let locator = KnowledgeViewLocator::read(path)?;
        let (database, view) = locator.as_ref().map_or((path, "default"), |locator| {
            (locator.database.as_path(), locator.view.as_str())
        });
        Self::open_view_read_once(database, view, true)
    }

    pub fn lifecycle_lease(&self) -> Option<crate::storage_lease::StorageLease> {
        self.lease.clone()
    }

    /// Equality fence for query delivery, including legacy full-store replacement
    /// which may clear the visibility log. Not an exported knowledge identity.
    pub fn read_state_digest(
        &self,
    ) -> Result<entmoot_contracts::ContentDigest, KnowledgeRepositoryError> {
        let (store, cursor, view): (String, i64, String) = self.connection.query_row(
            "SELECT value, (SELECT COALESCE(MAX(cursor), 0) FROM knowledge_visibility_events), (SELECT id FROM temp.selected_knowledge_view)
             FROM knowledge_metadata WHERE key = 'store_digest'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(entmoot_contracts::ContentDigest::from_trusted(
            entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
                "entmoot.query.store_state.v1",
                view,
                store,
                cursor,
            ))?),
        ))
    }

    fn from_connection(connection: Connection) -> Result<Self, KnowledgeRepositoryError> {
        Self::from_connection_view(connection, "default")
    }

    fn from_connection_view(
        mut connection: Connection,
        view: &str,
    ) -> Result<Self, KnowledgeRepositoryError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        // Inspect and initialize under the same writer reservation. Another
        // process may have completed initialization since this connection opened.
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != 0 && version != STORAGE_SCHEMA_VERSION {
            return Err(KnowledgeRepositoryError::UnsupportedSchema {
                actual: version,
                supported: STORAGE_SCHEMA_VERSION,
            });
        }
        if version == 0 {
            let occupied: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name NOT LIKE 'sqlite_%')",
                [],
                |row| row.get(0),
            )?;
            if occupied {
                return Err(KnowledgeRepositoryError::UnsupportedSchema {
                    actual: 0,
                    supported: STORAGE_SCHEMA_VERSION,
                });
            }
            transaction.execute_batch(layout::SCHEMA)?;
            transaction.execute_batch(inventory::SCHEMA)?;
            transaction.execute_batch(file_facts::SCHEMA)?;
            transaction.execute_batch(&operations::schema())?;
            fingerprint::initialize(&transaction)?;
        } else {
            fingerprint::verify(&transaction)?;
        }
        transaction.commit()?;
        crate::sqlite_startup::enable_wal(&connection)?;
        connection.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA temp_store=FILE; PRAGMA foreign_keys=ON;",
        )?;
        layout::select_view(&connection, view)?;
        Ok(Self {
            connection,
            lease: None,
        })
    }

    fn load_collection<T: DeserializeOwned>(
        &self,
        collection: &str,
    ) -> Result<Vec<T>, KnowledgeRepositoryError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT payload FROM knowledge_records
             WHERE collection = ?1 AND visible = 1 ORDER BY record_id",
        )?;
        let rows = statement.query_map([collection], |row| row.get::<_, i64>(0))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(hydrate_record(&self.connection, row?)?);
        }
        Ok(values)
    }

    fn load_subject_collection<T: DeserializeOwned>(
        &self,
        collection: &str,
        subject_id: &str,
    ) -> Result<Vec<T>, KnowledgeRepositoryError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT payload FROM knowledge_records_by_subject
             WHERE collection = ?1 AND subject_id = ?2 AND visible = 1 ORDER BY record_id",
        )?;
        let rows =
            statement.query_map(params![collection, subject_id], |row| row.get::<_, i64>(0))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(hydrate_record(&self.connection, row?)?);
        }
        Ok(values)
    }

    fn load_object_collection<T: DeserializeOwned>(
        &self,
        collection: &str,
        object_id: &str,
    ) -> Result<Vec<T>, KnowledgeRepositoryError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT payload FROM knowledge_records_by_object
             WHERE collection = ?1 AND object_id = ?2 AND visible = 1 ORDER BY record_id",
        )?;
        let rows =
            statement.query_map(params![collection, object_id], |row| row.get::<_, i64>(0))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(hydrate_record(&self.connection, row?)?);
        }
        Ok(values)
    }

    fn load_record<T: DeserializeOwned>(
        &self,
        collection: &str,
        record_id: &str,
    ) -> Result<Option<T>, KnowledgeRepositoryError> {
        let payload = self
            .connection
            .query_row(
                "SELECT payload FROM knowledge_records
                 WHERE collection = ?1 AND record_id = ?2 AND visible = 1",
                params![collection, record_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        payload
            .map(|payload| hydrate_record(&self.connection, payload))
            .transpose()
    }

    /// Count visible and inactive payload rows retained by the local backend.
    pub fn retained_record_count(&self) -> Result<usize, KnowledgeRepositoryError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM knowledge_records", [], |row| {
                    row.get(0)
                })?;
        usize::try_from(count)
            .map_err(|error| KnowledgeRepositoryError::BackendCount(error.to_string()))
    }
}

impl KnowledgeRepository for SqliteKnowledgeRepository {
    fn replace_owner_buffered(
        &mut self,
        owner_id: &str,
        batches: &mut dyn Iterator<Item = Result<OwnerBatch, KnowledgeRepositoryError>>,
        buffer: PublicationBuffer,
    ) -> Result<BufferedPublicationReceipt, KnowledgeRepositoryError> {
        buffered::publish(&mut self.connection, owner_id, batches, buffer)
    }

    fn replace_all(&mut self, store: &KnowledgeStore) -> Result<(), KnowledgeRepositoryError> {
        // Validate before opening the transaction. Serialization or insertion
        // failures still roll back the transaction without materializing every
        // payload in memory first.
        validate_store_records(store)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM scoped_visibility_events WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_record_dependency_sets WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_record_dependencies WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_record_owners WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_records WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        let (_, digest) = insert_store_rows(&transaction, store)?;
        set_store_digest(&transaction, &digest)?;
        transaction.commit()?;
        Ok(())
    }

    fn synchronize(
        &mut self,
        store: &KnowledgeStore,
    ) -> Result<KnowledgeSyncStats, KnowledgeRepositoryError> {
        validate_store_records(store)?;
        let current: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM knowledge_metadata WHERE key = 'store_digest'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if current.is_none() {
            let transaction = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            // An uncommitted database can contain rows left by an interrupted
            // initialization. Replace them in one transaction instead of paying
            // for a second full-size temporary table and INSERT ... SELECT.
            transaction.execute("DELETE FROM scoped_visibility_events WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
            transaction.execute("DELETE FROM scoped_record_dependency_sets WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
            transaction.execute("DELETE FROM scoped_record_dependencies WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
            transaction.execute("DELETE FROM scoped_record_owners WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
            let deleted = transaction.execute("DELETE FROM scoped_records WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
            let (upserted, digest) = insert_store_rows(&transaction, store)?;
            set_store_digest(&transaction, &digest)?;
            transaction.commit()?;
            return Ok(KnowledgeSyncStats { upserted, deleted });
        }
        let digest = digest_store_rows(store)?;
        if current.as_deref() == Some(digest.as_str()) {
            return Ok(KnowledgeSyncStats::default());
        }
        let rows = rows_from_store(store)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM scoped_visibility_events WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_record_dependency_sets WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_record_dependencies WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute("DELETE FROM scoped_record_owners WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [])?;
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS incoming_knowledge_records (
                 collection TEXT NOT NULL,
                 record_id TEXT NOT NULL,
                 artifact_id TEXT,
                 subject_id TEXT,
                 object_id TEXT,
                 proposition_id TEXT,
                 predicate TEXT,
                 location_id TEXT,
                 payload INTEGER NOT NULL,
                 PRIMARY KEY (collection, record_id)
             ) WITHOUT ROWID;
             DELETE FROM incoming_knowledge_records;",
        )?;
        insert_rows_into_incoming(&transaction, &rows)?;
        let deleted = transaction.execute(
            "DELETE FROM scoped_records AS current
             WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND NOT EXISTS (
                 SELECT 1 FROM incoming_knowledge_records AS incoming
                 WHERE incoming.collection = current.collection
                   AND incoming.record_id = current.record_id
             )",
            [],
        )?;
        let upserted = transaction.execute(
            "INSERT INTO scoped_records
                 (view_id, collection, record_id, artifact_id, subject_id, object_id, proposition_id, predicate, location_id, payload)
             SELECT (SELECT id FROM temp.selected_knowledge_view), collection, record_id, artifact_id, subject_id, object_id, proposition_id, predicate, location_id, payload
             FROM incoming_knowledge_records WHERE 1
             ON CONFLICT(view_id, collection, record_id) DO UPDATE SET
                 artifact_id=excluded.artifact_id,
                 subject_id=excluded.subject_id,
                 object_id=excluded.object_id,
                 proposition_id=excluded.proposition_id,
                 predicate=excluded.predicate,
                 location_id=excluded.location_id,
                 payload=excluded.payload,
                 visible=1,
                 ownership_managed=0
             WHERE scoped_records.artifact_id IS NOT excluded.artifact_id
                OR scoped_records.subject_id IS NOT excluded.subject_id
                OR scoped_records.object_id IS NOT excluded.object_id
                OR scoped_records.proposition_id IS NOT excluded.proposition_id
                OR scoped_records.predicate IS NOT excluded.predicate
                OR scoped_records.location_id IS NOT excluded.location_id
                OR scoped_records.payload != excluded.payload
                OR scoped_records.visible != 1
                OR scoped_records.ownership_managed != 0",
            [],
        )?;
        set_store_digest(&transaction, &digest)?;
        transaction.commit()?;
        Ok(KnowledgeSyncStats { upserted, deleted })
    }

    fn replace_owner(
        &mut self,
        owner_id: &str,
        store: &KnowledgeStore,
        dependencies: &[KnowledgeDependency],
    ) -> Result<KnowledgeOwnershipStats, KnowledgeRepositoryError> {
        let mut results = self.replace_owners(&[OwnerReplacement {
            owner_id,
            store,
            dependencies,
        }])?;
        Ok(results.remove(0))
    }

    fn replace_owners(
        &mut self,
        publications: &[OwnerReplacement<'_>],
    ) -> Result<Vec<KnowledgeOwnershipStats>, KnowledgeRepositoryError> {
        let planned = prepare_owner_replacements(publications)?;

        let mut any_change = false;
        for (publication, digest) in &planned {
            if !owner_publication_digest_matches(&self.connection, publication.owner_id, digest)? {
                any_change = true;
                break;
            }
        }
        if !any_change {
            return Ok(vec![KnowledgeOwnershipStats::default(); planned.len()]);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let results = apply_owner_replacements(&transaction, &planned)?;
        transaction.commit()?;
        Ok(results)
    }
    fn remove_owner(
        &mut self,
        owner_id: &str,
    ) -> Result<KnowledgeOwnershipStats, KnowledgeRepositoryError> {
        validate_owner(owner_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let seeds = owner_records(&transaction, owner_id)?;
        let removed_owners = transaction.execute(
            "DELETE FROM scoped_record_owners WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND owner_id = ?1",
            [owner_id],
        )?;
        let key = owner_publication_metadata_key(owner_id);
        transaction.execute("DELETE FROM scoped_metadata WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND key = ?1", [key])?;
        let visibility = recompute_visibility(&transaction, seeds, Some(owner_id))?;
        transaction.commit()?;
        Ok(KnowledgeOwnershipStats {
            removed_owners,
            evaluated_records: visibility.evaluated_records,
            became_visible: visibility.became_visible,
            became_inactive: visibility.became_inactive,
            ..KnowledgeOwnershipStats::default()
        })
    }
}

impl crate::read::KnowledgeRead for SqliteKnowledgeRepository {
    fn retained_call_edges(
        &self,
        addresses: &[Address],
        direction: entmoot_contracts::query::CallDirection,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Option<crate::read::RelationReadPage>, KnowledgeRepositoryError> {
        self.read_call_edges(addresses, direction, after, limit)
            .map(Some)
    }
    fn retained_unresolved(
        &self,
        addresses: &[Address],
        after: Option<&str>,
        limit: usize,
    ) -> Result<Option<crate::read::UnresolvedReadPage>, KnowledgeRepositoryError> {
        self.read_unresolved_page(addresses, after, limit).map(Some)
    }
    fn retained_relation_context(
        &self,
        edges: &[&RelationEdge],
        artifact_ids: &[String],
        coverage_limit: usize,
    ) -> Result<Option<crate::read::RelationReadContext>, KnowledgeRepositoryError> {
        self.read_relation_context(edges, artifact_ids, coverage_limit)
            .map(Some)
    }
    fn inventory_resource(
        &self,
        id: &str,
    ) -> Result<Option<entmoot_contracts::ResourceRecord>, KnowledgeRepositoryError> {
        self.load_record("resources", id)
    }
    fn inventory_nodes_matching(
        &self,
        path: Option<&str>,
        exact: &[String],
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<crate::read::InventoryNodePage>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::inventory_nodes_matching(self, path, exact, after, limit)
    }
    fn inventory_nodes(
        &self,
        path: Option<&str>,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<crate::read::InventoryNodePage>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::inventory_nodes(self, path, after, limit)
    }
    fn inventory_metadata(
        &self,
    ) -> Result<Option<crate::read::InventoryMetadata>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::inventory_metadata(self)
    }

    fn relation_claim_by_id(
        &self,
        id: &str,
    ) -> Result<Option<RelationClaim>, KnowledgeRepositoryError> {
        entmoot_contracts::RelationClaimId::parse(id)
            .map_err(|e| invalid_record("relation_claims", e))?;
        let claim: Option<RelationClaim> = self.load_record("relation_claims", id)?;
        if let Some(claim) = &claim {
            claim
                .validate()
                .map_err(|e| invalid_record("relation_claims", e))?;
        }
        Ok(claim)
    }
    fn relation_claims_for_address(
        &self,
        address: &Address,
        predicate: &str,
        limit: usize,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        validate_query_address(address)?;
        let mut statement = self.connection.prepare_cached("SELECT payload FROM knowledge_records_by_subject WHERE collection='relation_claims' AND subject_id=?1 AND predicate=?2 AND visible=1 ORDER BY record_id LIMIT ?3")?;
        let rows = statement.query_map(
            params![
                format!("address:{}", address_identity_key(address)),
                predicate,
                i64::try_from(limit.min(10_000))
                    .map_err(|e| KnowledgeRepositoryError::BackendCount(e.to_string()))?
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let mut claims = Vec::new();
        for row in rows {
            let claim: RelationClaim = hydrate_record(&self.connection, row?)?;
            claim
                .validate()
                .map_err(|e| invalid_record("relation_claims", e))?;
            claims.push(claim);
        }
        Ok(claims)
    }
    fn is_initialized(&self) -> Result<bool, KnowledgeRepositoryError> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM knowledge_metadata WHERE key = 'store_digest'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
    fn load(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError> {
        self.load_mode(ReadProjection::Complete)
    }
    fn assertions_for_subject(
        &self,
        subject_id: &str,
    ) -> Result<Vec<Assertion>, KnowledgeRepositoryError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT payload FROM knowledge_records_by_subject
             WHERE collection = 'assertions' AND subject_id = ?1
               AND visible = 1
             ORDER BY record_id",
        )?;
        let rows = statement.query_map([subject_id], |row| row.get::<_, i64>(0))?;
        let mut assertions = Vec::new();
        for row in rows {
            assertions.push(hydrate_record(&self.connection, row?)?);
        }
        Ok(assertions)
    }
    fn relation_edges_for_subject(
        &self,
        subject: &Address,
    ) -> Result<Vec<RelationEdge>, KnowledgeRepositoryError> {
        validate_query_address(subject)?;
        let edges: Vec<RelationEdge> =
            self.load_subject_collection("relation_edges", &address_identity_key(subject))?;
        for edge in &edges {
            edge.validate()
                .map_err(|error| invalid_record("relation_edges", error))?;
        }
        Ok(edges)
    }
    fn proposition_by_id(
        &self,
        proposition_id: &str,
    ) -> Result<Option<Proposition>, KnowledgeRepositoryError> {
        PropositionId::parse(proposition_id).map_err(|error| {
            KnowledgeRepositoryError::InvalidQueryPropositionId(error.to_string())
        })?;
        let proposition: Option<Proposition> = self.load_record("propositions", proposition_id)?;
        if let Some(proposition) = &proposition {
            proposition
                .validate()
                .map_err(|error| invalid_record("propositions", error))?;
        }
        Ok(proposition)
    }
    fn propositions_for_subject(
        &self,
        subject: &Address,
    ) -> Result<Vec<Proposition>, KnowledgeRepositoryError> {
        validate_query_address(subject)?;
        let propositions: Vec<Proposition> =
            self.load_subject_collection("propositions", &address_identity_key(subject))?;
        validate_propositions(&propositions)?;
        Ok(propositions)
    }
    fn propositions_for_object(
        &self,
        object: &Address,
    ) -> Result<Vec<Proposition>, KnowledgeRepositoryError> {
        validate_query_address(object)?;
        let propositions: Vec<Proposition> =
            self.load_object_collection("propositions", &address_identity_key(object))?;
        validate_propositions(&propositions)?;
        Ok(propositions)
    }
    fn relation_claims_for_relation_edge(
        &self,
        relation_edge_id: &str,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        RelationEdgeId::parse(relation_edge_id).map_err(|error| {
            KnowledgeRepositoryError::InvalidQueryRelationEdgeId(error.to_string())
        })?;
        let relation_claims: Vec<RelationClaim> = self.load_subject_collection(
            "relation_claims",
            &format!("relation_edge:{relation_edge_id}"),
        )?;
        for claim in &relation_claims {
            claim
                .validate()
                .map_err(|error| invalid_record("relation_claims", error))?;
        }
        Ok(relation_claims)
    }
    fn relation_edges_for_proposition(
        &self,
        proposition_id: &str,
    ) -> Result<Vec<RelationEdge>, KnowledgeRepositoryError> {
        PropositionId::parse(proposition_id).map_err(|error| {
            KnowledgeRepositoryError::InvalidQueryPropositionId(error.to_string())
        })?;
        let mut statement = self.connection.prepare_cached(
            "SELECT payload FROM knowledge_records_by_proposition
             WHERE collection = 'relation_edges' AND proposition_id = ?1
               AND visible = 1
             ORDER BY record_id",
        )?;
        let rows = statement.query_map([proposition_id], |row| row.get::<_, i64>(0))?;
        let mut edges = Vec::new();
        for row in rows {
            let edge: RelationEdge = hydrate_record(&self.connection, row?)?;
            edge.validate()
                .map_err(|error| invalid_record("relation_edges", error))?;
            edges.push(edge);
        }
        Ok(edges)
    }
    fn relation_claims_for_proposition(
        &self,
        proposition_id: &str,
    ) -> Result<Vec<RelationClaim>, KnowledgeRepositoryError> {
        PropositionId::parse(proposition_id).map_err(|error| {
            KnowledgeRepositoryError::InvalidQueryPropositionId(error.to_string())
        })?;
        let relation_claims: Vec<RelationClaim> = self
            .load_subject_collection("relation_claims", &format!("proposition:{proposition_id}"))?;
        for claim in &relation_claims {
            claim
                .validate()
                .map_err(|error| invalid_record("relation_claims", error))?;
        }
        Ok(relation_claims)
    }
    fn relation_claims_for_edges(
        &self,
        edges: &[&RelationEdge],
    ) -> Result<Vec<Vec<RelationClaim>>, KnowledgeRepositoryError> {
        let keys: Vec<_> = edges
            .iter()
            .flat_map(|edge| {
                [
                    format!("relation_edge:{}", edge.id),
                    format!("proposition:{}", edge.proposition_id),
                ]
            })
            .collect();
        let unique: Vec<_> = keys.iter().collect::<BTreeSet<_>>().into_iter().collect();
        let mut claims: HashMap<String, Vec<RelationClaim>> = HashMap::new();
        // Keep statement sizes below SQLite's parameter ceiling. The caller's
        // repository read snapshot covers every chunk and payload hydration.
        for batch in unique.chunks(256) {
            let placeholders = vec!["?"; batch.len()].join(",");
            let mut statement = self.connection.prepare_cached(&format!(
                "SELECT subject_id,payload FROM knowledge_records_by_subject WHERE collection='relation_claims' AND visible=1 AND subject_id IN ({placeholders}) ORDER BY record_id"
            ))?;
            let rows = statement.query_map(rusqlite::params_from_iter(batch), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (subject, payload) = row?;
                let claim: RelationClaim = hydrate_record(&self.connection, payload)?;
                claim
                    .validate()
                    .map_err(|error| invalid_record("relation_claims", error))?;
                claims.entry(subject).or_default().push(claim);
            }
        }
        Ok(keys
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                pair.iter()
                    .flat_map(|key| claims.get(key).into_iter().flatten().cloned())
                    .collect()
            })
            .collect())
    }
    fn unresolved_for_source(
        &self,
        source: &Address,
    ) -> Result<Vec<UnresolvedRelation>, KnowledgeRepositoryError> {
        validate_query_address(source)?;
        let relations: Vec<UnresolvedRelation> =
            self.load_subject_collection("unresolved_relations", &address_identity_key(source))?;
        for relation in &relations {
            relation
                .validate()
                .map_err(|error| invalid_record("unresolved_relations", error))?;
        }
        Ok(relations)
    }
    fn unresolved_by_id(
        &self,
        unresolved_id: &str,
    ) -> Result<Option<UnresolvedRelation>, KnowledgeRepositoryError> {
        UnresolvedRelationId::parse(unresolved_id).map_err(|error| {
            KnowledgeRepositoryError::InvalidQueryUnresolvedId(error.to_string())
        })?;
        let relation: Option<UnresolvedRelation> =
            self.load_record("unresolved_relations", unresolved_id)?;
        if let Some(relation) = &relation {
            relation
                .validate()
                .map_err(|error| invalid_record("unresolved_relations", error))?;
        }
        Ok(relation)
    }
    fn artifact_by_id(
        &self,
        artifact_id: &str,
    ) -> Result<Option<ArtifactManifest>, KnowledgeRepositoryError> {
        ArtifactId::parse(artifact_id)
            .map_err(|error| KnowledgeRepositoryError::InvalidQueryArtifactId(error.to_string()))?;
        let artifact: Option<ArtifactManifest> = self.load_record("artifacts", artifact_id)?;
        if let Some(artifact) = &artifact {
            artifact
                .validate_structure()
                .map_err(|error| invalid_record("artifacts", error))?;
        }
        Ok(artifact)
    }
    fn trace_by_id(&self, trace_id: &str) -> Result<Option<TraceRecord>, KnowledgeRepositoryError> {
        validate_query_token(trace_id)?;
        self.load_record("traces", trace_id)
    }
    fn trace_steps_for_trace(
        &self,
        trace_id: &str,
        max_steps: usize,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError> {
        validate_query_token(trace_id)?;
        validate_page_size(max_steps)?;
        let Some(trace) = self.load_record::<TraceRecord>("traces", trace_id)? else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        for (ordinal, step_id) in trace.step_ids.iter().take(max_steps).enumerate() {
            if let Some(step) = self.load_record::<TraceStepRecord>("trace_steps", step_id)? {
                if step.trace_id != trace_id
                    || !matches!(usize::try_from(step.ordinal), Ok(step_ordinal) if step_ordinal == ordinal)
                {
                    return Err(invalid_record(
                        "trace_steps",
                        format!("step {step_id} does not match trace order"),
                    ));
                }
                steps.push(step);
            }
        }
        Ok(steps)
    }
    fn resources_by_logical_id(
        &self,
        resource_id: &str,
    ) -> Result<Vec<entmoot_contracts::ResourceRecord>, KnowledgeRepositoryError> {
        entmoot_contracts::ArtifactId::parse(resource_id)
            .map_err(|error| KnowledgeRepositoryError::InvalidQueryAddress(error.to_string()))?;
        let resources: Vec<entmoot_contracts::ResourceRecord> =
            self.load_subject_collection("resources", resource_id)?;
        for resource in &resources {
            resource
                .validate()
                .map_err(|error| invalid_record("resources", error))?;
        }
        Ok(resources)
    }
    fn records_for_owner(
        &self,
        owner_id: &str,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
        validate_owner(owner_id)?;
        validate_page_size(max_records)?;
        let limit = i64::try_from(max_records).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare_cached(
            "SELECT collection, record_id FROM knowledge_record_owners
             WHERE owner_id = ?1 ORDER BY collection, record_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![owner_id, limit], |row| {
            Ok(KnowledgeRecordRef::new(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    fn owners_for_record(
        &self,
        record: &KnowledgeRecordRef,
        max_owners: usize,
    ) -> Result<Vec<String>, KnowledgeRepositoryError> {
        record.validate()?;
        validate_page_size(max_owners)?;
        let limit = i64::try_from(max_owners).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare_cached(
            "SELECT owner_id FROM knowledge_record_owners
             WHERE collection = ?1 AND record_id = ?2 ORDER BY owner_id LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![record.collection, record.record_id, limit], |row| {
                row.get::<_, String>(0)
            })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    fn record_state(
        &self,
        record: &KnowledgeRecordRef,
    ) -> Result<Option<KnowledgeRecordState>, KnowledgeRepositoryError> {
        record_state_in_connection(&self.connection, record)
    }
    fn dependency_inputs_for_record(
        &self,
        record: &KnowledgeRecordRef,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
        record.validate()?;
        validate_page_size(max_records)?;
        load_record_refs(
            &self.connection,
            "SELECT input_collection, input_record_id FROM knowledge_record_dependencies
             WHERE collection = ?1 AND record_id = ?2
             ORDER BY input_collection, input_record_id LIMIT ?3",
            record,
            max_records,
        )
    }
    fn dependent_records_for_input(
        &self,
        record: &KnowledgeRecordRef,
        max_records: usize,
    ) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
        record.validate()?;
        validate_page_size(max_records)?;
        load_record_refs(
            &self.connection,
            "SELECT collection, record_id FROM knowledge_record_dependencies
             WHERE input_collection = ?1 AND input_record_id = ?2
             ORDER BY collection, record_id LIMIT ?3",
            record,
            max_records,
        )
    }
    fn visibility_changes(
        &self,
        after_cursor: Option<u64>,
        max_events: usize,
    ) -> Result<LocalVisibilityPage, KnowledgeRepositoryError> {
        if max_events == 0 {
            return Err(KnowledgeRepositoryError::InvalidPageSize);
        }
        let after = after_cursor.unwrap_or(0);
        let after_sql = i64::try_from(after)
            .map_err(|error| KnowledgeRepositoryError::BackendCount(error.to_string()))?;
        let limit = i64::try_from(max_events.saturating_add(1)).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare_cached(
            "SELECT cursor, collection, record_id, visible, reason, cause_ids
             FROM knowledge_visibility_events WHERE cursor > ?1
             ORDER BY cursor LIMIT ?2",
        )?;
        let rows = statement.query_map(params![after_sql, limit], |row| {
            let reason: String = row.get(4)?;
            let reason = match reason.as_str() {
                "published" => LocalVisibilityReason::Published,
                "owner_removed" => LocalVisibilityReason::OwnerRemoved,
                "dependency_changed" => LocalVisibilityReason::DependencyChanged,
                _ => {
                    return Err(rusqlite::Error::InvalidColumnType(
                        4,
                        "reason".into(),
                        rusqlite::types::Type::Text,
                    ))
                }
            };
            let cause_bytes: Vec<u8> = row.get(5)?;
            let cause_ids = serde_json::from_slice(&cause_bytes).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    cause_bytes.len(),
                    rusqlite::types::Type::Blob,
                    Box::new(error),
                )
            })?;
            Ok(LocalVisibilityChange {
                cursor: u64::try_from(row.get::<_, i64>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
                record: KnowledgeRecordRef::new(row.get::<_, String>(1)?, row.get::<_, String>(2)?),
                visible: row.get::<_, i64>(3)? != 0,
                reason,
                cause_ids,
            })
        })?;
        let mut events = rows.collect::<Result<Vec<_>, _>>()?;
        let truncated = events.len() > max_events;
        if truncated {
            events.truncate(max_events);
        }
        let next_cursor = events.last().map_or(after, |event| event.cursor);
        Ok(LocalVisibilityPage {
            after_cursor,
            next_cursor,
            events,
            truncated,
        })
    }
    fn current_visibility_cursor(&self) -> Result<u64, KnowledgeRepositoryError> {
        let cursor: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(cursor), 0) FROM knowledge_visibility_events",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(cursor)
            .map_err(|error| KnowledgeRepositoryError::BackendCount(error.to_string()))
    }
    fn query_checkpoint(
        &self,
        key: &str,
        generation: &str,
        now_ms: u64,
    ) -> Result<Option<(usize, Vec<u8>)>, OperationError> {
        SqliteKnowledgeRepository::query_checkpoint(self, key, generation, now_ms)
    }
    fn inventory_export_digest(&self) -> Result<Option<String>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::inventory_export_digest(self)
    }
    fn read_state_digest(
        &self,
    ) -> Result<entmoot_contracts::ContentDigest, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::read_state_digest(self)
    }
    fn load_inventory(
        &self,
        query_only: bool,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::load_inventory(self, query_only)
    }
    fn load_assessment_inventory(&self) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::load_assessment_inventory(self)
    }
    fn load_assessment(&self) -> Result<KnowledgeStore, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::load_assessment(self)
    }
    fn trace_step_by_id(
        &self,
        id: &str,
    ) -> Result<Option<TraceStepRecord>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::trace_step_by_id(self, id)
    }
    fn trace_steps_at_location(
        &self,
        id: &str,
    ) -> Result<Vec<TraceStepRecord>, KnowledgeRepositoryError> {
        SqliteKnowledgeRepository::trace_steps_at_location(self, id)
    }
}

fn validate_owner(owner_id: &str) -> Result<(), KnowledgeRepositoryError> {
    if owner_id.trim().is_empty() {
        return Err(KnowledgeRepositoryError::InvalidOwner);
    }
    Ok(())
}

fn validate_query_token(value: &str) -> Result<(), KnowledgeRepositoryError> {
    if value.trim().is_empty() || value.trim() != value || value.contains('\0') {
        return Err(KnowledgeRepositoryError::InvalidRecordRef(value.into()));
    }
    Ok(())
}

fn validate_page_size(value: usize) -> Result<(), KnowledgeRepositoryError> {
    if value == 0 {
        Err(KnowledgeRepositoryError::InvalidPageSize)
    } else {
        Ok(())
    }
}

fn load_record_refs(
    connection: &Connection,
    sql: &str,
    record: &KnowledgeRecordRef,
    max_records: usize,
) -> Result<Vec<KnowledgeRecordRef>, KnowledgeRepositoryError> {
    let limit = i64::try_from(max_records).unwrap_or(i64::MAX);
    let mut statement = connection.prepare_cached(sql)?;
    let rows = statement.query_map(params![record.collection, record.record_id, limit], |row| {
        Ok(KnowledgeRecordRef::new(
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn unique_owned_rows(
    store: &KnowledgeStore,
) -> Result<BTreeMap<KnowledgeRecordRef, StoredRow>, KnowledgeRepositoryError> {
    let mut rows = BTreeMap::new();
    for_each_store_row(store, |row| {
        let record = KnowledgeRecordRef::new(row.collection, row.record_id.clone());
        if let Some(existing) = rows.get(&record) {
            let existing: &StoredRow = existing;
            if existing.payload != row.payload {
                return Err(KnowledgeRepositoryError::ImmutableRecordCollision {
                    collection: record.collection,
                    record_id: record.record_id,
                });
            }
        } else {
            rows.insert(record, row);
        }
        Ok(())
    })?;
    Ok(rows)
}

fn validate_dependency_batch(
    rows: &BTreeMap<KnowledgeRecordRef, StoredRow>,
    dependencies: &[KnowledgeDependency],
) -> Result<BTreeMap<KnowledgeRecordRef, BTreeSet<KnowledgeRecordRef>>, KnowledgeRepositoryError> {
    let mut result = BTreeMap::new();
    for dependency in dependencies {
        dependency.dependent.validate()?;
        if !rows.contains_key(&dependency.dependent) {
            return Err(KnowledgeRepositoryError::InvalidRecordRef(format!(
                "dependent {}/{} is not owned by this replacement",
                dependency.dependent.collection, dependency.dependent.record_id
            )));
        }
        let mut inputs = BTreeSet::new();
        for input in &dependency.inputs {
            input.validate()?;
            inputs.insert(input.clone());
        }
        if result
            .insert(dependency.dependent.clone(), inputs)
            .is_some()
        {
            return Err(KnowledgeRepositoryError::InvalidRecordRef(format!(
                "duplicate dependency declaration for {}/{}",
                dependency.dependent.collection, dependency.dependent.record_id
            )));
        }
    }
    Ok(result)
}

fn owner_records(
    transaction: &Transaction<'_>,
    owner_id: &str,
) -> Result<BTreeSet<KnowledgeRecordRef>, KnowledgeRepositoryError> {
    let mut statement = transaction.prepare_cached(
        "SELECT collection, record_id FROM knowledge_record_owners
         WHERE owner_id = ?1 ORDER BY collection, record_id",
    )?;
    let rows = statement.query_map([owner_id], |row| {
        Ok(KnowledgeRecordRef::new(
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn inactive_dependency_inputs(
    transaction: &Transaction<'_>,
    dependent: &KnowledgeRecordRef,
) -> Result<BTreeSet<KnowledgeRecordRef>, KnowledgeRepositoryError> {
    let mut statement = transaction.prepare_cached(
        "SELECT d.input_collection, d.input_record_id FROM knowledge_record_dependencies d
         LEFT JOIN knowledge_records i ON i.collection=d.input_collection AND i.record_id=d.input_record_id
         WHERE d.collection = ?1 AND d.record_id = ?2 AND (i.visible IS NULL OR i.visible=0)
         ORDER BY d.input_collection, d.input_record_id",
    )?;
    let rows = statement.query_map(params![dependent.collection, dependent.record_id], |row| {
        Ok(KnowledgeRecordRef::new(
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn dependency_set_digest(inputs: &BTreeSet<KnowledgeRecordRef>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"entmoot.knowledge-dependency-set.v1\0");
    for input in inputs {
        for field in [&input.collection, &input.record_id] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field.as_bytes());
        }
    }
    format!("blake3:{}", hasher.finalize().to_hex())
}

fn record_has_owner(
    transaction: &Transaction<'_>,
    record: &KnowledgeRecordRef,
) -> Result<bool, KnowledgeRepositoryError> {
    Ok(transaction
        .query_row(
            "SELECT 1 FROM knowledge_record_owners
             WHERE collection = ?1 AND record_id = ?2 LIMIT 1",
            params![record.collection, record.record_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn record_visibility(
    transaction: &Transaction<'_>,
    record: &KnowledgeRecordRef,
) -> Result<Option<(bool, bool)>, KnowledgeRepositoryError> {
    Ok(transaction
        .query_row(
            "SELECT visible, ownership_managed FROM knowledge_records
             WHERE collection = ?1 AND record_id = ?2",
            params![record.collection, record.record_id],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
        )
        .optional()?)
}

fn prepare_visibility_queue(transaction: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS visibility_pending (
        collection TEXT NOT NULL, record_id TEXT NOT NULL,
        PRIMARY KEY(collection,record_id)) WITHOUT ROWID;
        DELETE FROM visibility_pending;",
    )
}

#[derive(Debug, Clone, Copy, Default)]
struct VisibilityRecomputeStats {
    evaluated_records: usize,
    became_visible: usize,
    became_inactive: usize,
}

fn recompute_visibility(
    transaction: &Transaction<'_>,
    pending: BTreeSet<KnowledgeRecordRef>,
    removed_owner: Option<&str>,
) -> Result<VisibilityRecomputeStats, KnowledgeRepositoryError> {
    prepare_visibility_queue(transaction)?;
    for record in pending {
        transaction.execute(
            "INSERT OR IGNORE INTO visibility_pending VALUES (?1,?2)",
            params![record.collection, record.record_id],
        )?;
    }
    drain_visibility_queue(transaction, removed_owner)
}

fn drain_visibility_queue(
    transaction: &Transaction<'_>,
    removed_owner: Option<&str>,
) -> Result<VisibilityRecomputeStats, KnowledgeRepositoryError> {
    let mut stats = VisibilityRecomputeStats::default();
    loop {
        let record = transaction.query_row("SELECT collection, record_id FROM visibility_pending ORDER BY collection, record_id LIMIT 1", [],
            |r| Ok(KnowledgeRecordRef::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional()?;
        let Some(record) = record else {
            break;
        };
        transaction.execute(
            "DELETE FROM visibility_pending WHERE collection=?1 AND record_id=?2",
            params![record.collection, record.record_id],
        )?;
        let Some((prior, ownership_managed)) = record_visibility(transaction, &record)? else {
            continue;
        };
        if !ownership_managed {
            continue;
        }
        stats.evaluated_records += 1;
        let has_owner = record_has_owner(transaction, &record)?;
        let inactive_inputs = inactive_dependency_inputs(transaction, &record)?
            .into_iter()
            .map(|input| format!("{}/{}", input.collection, input.record_id))
            .collect::<Vec<_>>();
        let next = has_owner && inactive_inputs.is_empty();
        if prior == next {
            continue;
        }
        transaction.execute(
            "UPDATE scoped_records SET visible = ?3
             WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND collection = ?1 AND record_id = ?2",
            params![record.collection, record.record_id, i64::from(next)],
        )?;
        let (reason, cause_ids) = if next {
            stats.became_visible += 1;
            ("published", Vec::new())
        } else if !has_owner {
            stats.became_inactive += 1;
            (
                "owner_removed",
                removed_owner.map(str::to_owned).into_iter().collect(),
            )
        } else {
            stats.became_inactive += 1;
            ("dependency_changed", inactive_inputs)
        };
        let cause_bytes = serde_json::to_vec(&cause_ids)?;
        transaction.execute(
            "INSERT INTO scoped_visibility_events
             (view_id, collection, record_id, visible, reason, cause_ids)
             VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2, ?3, ?4, ?5)",
            params![
                record.collection,
                record.record_id,
                i64::from(next),
                reason,
                cause_bytes
            ],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO visibility_pending
            SELECT collection, record_id FROM knowledge_record_dependencies
            WHERE input_collection=?1 AND input_record_id=?2",
            params![record.collection, record.record_id],
        )?;
    }
    Ok(stats)
}

fn intern_row_payload(
    connection: &Connection,
    row: &StoredRow,
) -> Result<i64, KnowledgeRepositoryError> {
    let payload = intern_payload(connection, &row.payload)?;
    for target in &row.contradicts {
        connection
            .prepare_cached("INSERT OR IGNORE INTO shared_claim_links VALUES(?1,?2)")?
            .execute(params![payload, target])?;
    }
    Ok(payload)
}

fn insert_owned_row(
    transaction: &Transaction<'_>,
    row: &StoredRow,
) -> Result<(), KnowledgeRepositoryError> {
    transaction.execute(
        "INSERT INTO scoped_records
         (view_id, collection, record_id, artifact_id, subject_id, object_id, proposition_id,
          predicate, location_id, payload, visible, ownership_managed)
         VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 1)",
        params![
            row.collection,
            row.record_id,
            row.artifact_id,
            row.subject_id,
            row.object_id,
            row.proposition_id,
            row.predicate,
            row.location_id,
            intern_row_payload(transaction, row)?,
        ],
    )?;
    Ok(())
}

fn invalid_record(
    collection: &'static str,
    error: impl std::fmt::Display,
) -> KnowledgeRepositoryError {
    KnowledgeRepositoryError::InvalidRecord {
        collection,
        reason: error.to_string(),
    }
}

fn validate_query_address(address: &Address) -> Result<(), KnowledgeRepositoryError> {
    address
        .validate()
        .map_err(|error| KnowledgeRepositoryError::InvalidQueryAddress(error.to_string()))
}

fn validate_propositions(values: &[Proposition]) -> Result<(), KnowledgeRepositoryError> {
    for proposition in values {
        proposition
            .validate()
            .map_err(|error| invalid_record("propositions", error))?;
    }
    Ok(())
}

fn validate_store_records(store: &KnowledgeStore) -> Result<(), KnowledgeRepositoryError> {
    validate_store_records_in_context(store, &StoreValidationContext::new([store]))
}

/// Borrowed indexes over the complete publication, built once rather than once
/// per batch. Existing database rows do not supply missing publication context.
trait StoreRecordContext {
    fn artifact(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, ArtifactManifest>>, KnowledgeRepositoryError>;
    fn location(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, LocationRecord>>, KnowledgeRepositoryError>;
    fn component_exists(&self, id: &str) -> Result<bool, KnowledgeRepositoryError>;
    fn proposition(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, Proposition>>, KnowledgeRepositoryError>;
    fn relation_edge(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, RelationEdge>>, KnowledgeRepositoryError>;
}

struct StoreValidationContext<'a> {
    artifacts: BTreeMap<&'a str, &'a ArtifactManifest>,
    locations: BTreeMap<&'a str, &'a LocationRecord>,
    component_ids: BTreeSet<&'a str>,
    propositions: BTreeMap<&'a str, PropositionContext<'a>>,
    relation_edges: BTreeMap<&'a str, &'a entmoot_contracts::RelationEdge>,
}

enum PropositionContext<'a> {
    Record(&'a Proposition),
    Edge(&'a RelationEdge),
}

impl<'a> StoreValidationContext<'a> {
    fn new(stores: impl IntoIterator<Item = &'a KnowledgeStore>) -> Self {
        let mut context = Self {
            artifacts: BTreeMap::new(),
            locations: BTreeMap::new(),
            component_ids: BTreeSet::new(),
            propositions: BTreeMap::new(),
            relation_edges: BTreeMap::new(),
        };
        for store in stores {
            context.artifacts.extend(
                store
                    .artifacts
                    .iter()
                    .map(|value| (value.artifact_id.as_str(), value)),
            );
            context.locations.extend(
                store
                    .locations
                    .iter()
                    .map(|value| (value.location_id.as_str(), value)),
            );
            context.component_ids.extend(
                store
                    .components
                    .iter()
                    .map(|value| value.occurrence_id.as_str()),
            );
            context.propositions.extend(
                store
                    .propositions
                    .iter()
                    .map(|value| (value.id.as_str(), PropositionContext::Record(value))),
            );
            context.relation_edges.extend(
                store
                    .relation_edges
                    .iter()
                    .map(|edge| (edge.id.as_str(), edge)),
            );
        }
        context
    }

    /// An already validated graph supplies the same forward-reference context as
    /// a complete knowledge publication. Borrow it instead of writing and reading
    /// another temporary database. Propositions are projected one at a time.
    fn from_graph(graph: &'a crate::TopologyGraph) -> Self {
        Self {
            artifacts: graph
                .artifacts
                .iter()
                .map(|value| (value.artifact_id.as_str(), value))
                .collect(),
            locations: graph
                .locations
                .iter()
                .map(|value| (value.location_id.as_str(), value))
                .collect(),
            component_ids: graph
                .components
                .iter()
                .map(|value| value.occurrence_id.as_str())
                .collect(),
            propositions: graph
                .relation_edges
                .iter()
                .map(|edge| (edge.proposition_id.as_str(), PropositionContext::Edge(edge)))
                .collect(),
            relation_edges: graph
                .relation_edges
                .iter()
                .map(|edge| (edge.id.as_str(), edge))
                .collect(),
        }
    }
}

impl StoreRecordContext for StoreValidationContext<'_> {
    fn artifact(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, ArtifactManifest>>, KnowledgeRepositoryError> {
        Ok(self
            .artifacts
            .get(id)
            .map(|value| std::borrow::Cow::Borrowed(*value)))
    }
    fn location(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, LocationRecord>>, KnowledgeRepositoryError> {
        Ok(self
            .locations
            .get(id)
            .map(|value| std::borrow::Cow::Borrowed(*value)))
    }
    fn component_exists(&self, id: &str) -> Result<bool, KnowledgeRepositoryError> {
        Ok(self.component_ids.contains(id))
    }
    fn proposition(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, Proposition>>, KnowledgeRepositoryError> {
        Ok(self.propositions.get(id).map(|value| match value {
            PropositionContext::Record(value) => std::borrow::Cow::Borrowed(*value),
            PropositionContext::Edge(edge) => std::borrow::Cow::Owned(edge.proposition()),
        }))
    }
    fn relation_edge(
        &self,
        id: &str,
    ) -> Result<Option<std::borrow::Cow<'_, RelationEdge>>, KnowledgeRepositoryError> {
        Ok(self
            .relation_edges
            .get(id)
            .map(|value| std::borrow::Cow::Borrowed(*value)))
    }
}

fn validate_store_records_in_context(
    store: &KnowledgeStore,
    context: &impl StoreRecordContext,
) -> Result<(), KnowledgeRepositoryError> {
    for root in &store.input_roots {
        root.validate()
            .map_err(|error| invalid_record("input_roots", error))?;
    }
    for artifact in &store.artifacts {
        artifact
            .validate_structure()
            .map_err(|error| invalid_record("artifacts", error))?;
    }
    for location in &store.locations {
        if context.artifact(location.artifact_id.as_str())?.is_none() {
            return Err(invalid_record("locations", "location artifact is missing"));
        }
        if let Some(region) = location.extent.region() {
            region
                .validate()
                .map_err(|error| invalid_record("locations", error))?;
        }
    }
    for attachment in &store.attachments {
        let location = context
            .location(attachment.location_id.as_str())?
            .ok_or_else(|| invalid_record("attachments", "attachment location is missing"))?;
        attachment
            .validate_location(&location)
            .map_err(|error| invalid_record("attachments", error))?;
    }
    for resource in &store.resources {
        resource
            .validate()
            .map_err(|error| invalid_record("resources", error))?;
    }
    for component in &store.components {
        component
            .validate()
            .map_err(|error| invalid_record("components", error))?;
        if context.artifact(component.artifact_id.as_str())?.is_none() {
            return Err(invalid_record(
                "components",
                format!("missing artifact {}", component.artifact_id),
            ));
        }
    }
    for dependency in &store.component_dependencies {
        dependency
            .validate()
            .map_err(|error| invalid_record("component_dependencies", error))?;
        if context.artifact(dependency.artifact_id.as_str())?.is_none() {
            return Err(invalid_record(
                "component_dependencies",
                format!("missing artifact {}", dependency.artifact_id),
            ));
        }
        for occurrence_id in [
            &dependency.subject_occurrence_id,
            &dependency.object_occurrence_id,
        ]
        .into_iter()
        .flatten()
        {
            if !context.component_exists(occurrence_id.as_str())? {
                return Err(invalid_record(
                    "component_dependencies",
                    format!("missing component {occurrence_id}"),
                ));
            }
        }
    }
    for vulnerability in &store.component_vulnerabilities {
        vulnerability
            .validate()
            .map_err(|error| invalid_record("component_vulnerabilities", error))?;
        if context
            .artifact(vulnerability.artifact_id.as_str())?
            .is_none()
        {
            return Err(invalid_record(
                "component_vulnerabilities",
                format!("missing artifact {}", vulnerability.artifact_id),
            ));
        }
        for affect in &vulnerability.affects {
            if let Some(occurrence_id) = &affect.component_occurrence_id {
                if !context.component_exists(occurrence_id.as_str())? {
                    return Err(invalid_record(
                        "component_vulnerabilities",
                        format!("missing component {occurrence_id}"),
                    ));
                }
            }
        }
    }
    for composition in &store.bom_compositions {
        composition
            .validate()
            .map_err(|error| invalid_record("bom_compositions", error))?;
        if context
            .artifact(composition.artifact_id.as_str())?
            .is_none()
        {
            return Err(invalid_record(
                "bom_compositions",
                format!("missing artifact {}", composition.artifact_id),
            ));
        }
    }
    for assertion in &store.assertions {
        assertion
            .validate_epistemic()
            .map_err(|error| invalid_record("assertions", error))?;
        if let Some(subject) = &assertion.subject_address {
            subject
                .address
                .validate()
                .map_err(|error| invalid_record("assertions", error))?;
        }
    }
    for resolution in &store.resolutions {
        resolution
            .validate()
            .map_err(|error| invalid_record("resolutions", error))?;
    }
    for coverage in &store.resolver_coverages {
        coverage
            .validate()
            .map_err(|error| invalid_record("resolver_coverages", error))?;
    }
    let mut resolved_ids = BTreeSet::new();
    for record in &store.resolved_identities {
        record
            .validate()
            .map_err(|error| invalid_record("resolved_identities", error))?;
        if !resolved_ids.insert(record.record_id()) {
            return Err(invalid_record(
                "resolved_identities",
                format!("duplicate resolved identity for `{}`", record.address_id),
            ));
        }
        if let Some(source) = record.binding.source() {
            let artifact_id = source.artifact.artifact_id.as_str();
            let manifest = context.artifact(artifact_id)?.ok_or_else(|| {
                invalid_record(
                    "resolved_identities",
                    format!("missing artifact {artifact_id}"),
                )
            })?;
            source
                .artifact
                .verify(&manifest)
                .map_err(|error| invalid_record("resolved_identities", error))?;
        }
    }
    validate_propositions(&store.propositions)?;
    if store
        .propositions
        .iter()
        .map(|p| p.id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        != store.propositions.len()
    {
        return Err(invalid_record("propositions", "duplicate proposition id"));
    }
    for edge in &store.relation_edges {
        edge.validate()
            .map_err(|error| invalid_record("relation_edges", error))?;
        let expected = edge.proposition();
        if context
            .proposition(edge.proposition_id.as_str())?
            .as_deref()
            != Some(&expected)
        {
            return Err(invalid_record(
                "relation_edges",
                format!(
                    "edge {} does not reference its materialized proposition {}",
                    edge.id, edge.proposition_id
                ),
            ));
        }
    }
    for claim in &store.relation_claims {
        claim
            .validate()
            .map_err(|error| invalid_record("relation_claims", error))?;
        if let entmoot_contracts::SubjectId::RelationEdge { edge_id } = &claim.subject {
            if let Some(edge) = context.relation_edge(edge_id.as_str())? {
                if !edge.accepts_claim_scope(&claim.scope) {
                    return Err(invalid_record(
                        "relation_claims",
                        "claim scope disagrees with its relation edge",
                    ));
                }
            }
        }
    }
    for relation in &store.unresolved_relations {
        relation
            .validate()
            .map_err(|error| invalid_record("unresolved_relations", error))?;
    }
    Ok(())
}

#[derive(Clone)]
struct StoredRow {
    collection: &'static str,
    record_id: String,
    artifact_id: Option<String>,
    subject_id: Option<String>,
    object_id: Option<String>,
    proposition_id: Option<String>,
    predicate: Option<String>,
    location_id: Option<String>,
    payload: Vec<u8>,
    contradicts: Vec<String>,
}

impl StoredRow {
    fn new<T: Serialize>(collection: &'static str, value: &T) -> Result<Self, serde_json::Error> {
        let payload = serde_json::to_vec(value)?;
        let digest = entmoot_contracts::blake3_hex(&payload);
        Ok(Self {
            collection,
            record_id: digest,
            artifact_id: None,
            subject_id: None,
            object_id: None,
            proposition_id: None,
            predicate: None,
            location_id: None,
            payload,
            contradicts: Vec::new(),
        })
    }
}

fn for_each_store_row(
    store: &KnowledgeStore,
    mut visit: impl FnMut(StoredRow) -> Result<(), KnowledgeRepositoryError>,
) -> Result<usize, KnowledgeRepositoryError> {
    let mut count = 0usize;

    macro_rules! emit {
        ($row:expr) => {{
            visit($row)?;
            count += 1;
        }};
    }

    macro_rules! plain_rows {
        ($collection:literal, $values:expr) => {
            for value in $values {
                emit!(StoredRow::new($collection, value)?);
            }
        };
    }

    for artifact in &store.artifacts {
        let mut row = StoredRow::new("artifacts", artifact)?;
        row.record_id = artifact.artifact_id.clone();
        row.artifact_id = Some(artifact.artifact_id.clone());
        emit!(row);
    }
    for component in &store.components {
        let mut row = StoredRow::new("components", component)?;
        row.record_id = component.occurrence_id.clone();
        row.artifact_id = Some(component.artifact_id.clone());
        row.subject_id = Some(component.occurrence_id.clone());
        emit!(row);
    }
    for dependency in &store.component_dependencies {
        let mut row = StoredRow::new("component_dependencies", dependency)?;
        row.record_id = dependency.dependency_id.clone();
        row.artifact_id = Some(dependency.artifact_id.clone());
        row.subject_id = dependency.subject_occurrence_id.clone();
        row.object_id = dependency.object_occurrence_id.clone();
        row.predicate = Some(
            match dependency.kind {
                entmoot_contracts::ComponentDependencyKind::DependsOn => "depends_on",
                entmoot_contracts::ComponentDependencyKind::Provides => "provides",
            }
            .into(),
        );
        emit!(row);
    }
    for vulnerability in &store.component_vulnerabilities {
        let mut row = StoredRow::new("component_vulnerabilities", vulnerability)?;
        row.record_id = vulnerability.observation_id.clone();
        row.artifact_id = Some(vulnerability.artifact_id.clone());
        emit!(row);
    }
    for composition in &store.bom_compositions {
        let mut row = StoredRow::new("bom_compositions", composition)?;
        row.record_id = composition.composition_id.clone();
        row.artifact_id = Some(composition.artifact_id.clone());
        emit!(row);
    }
    for capability in &store.capabilities {
        let mut row = StoredRow::new("capabilities", capability)?;
        row.artifact_id = Some(capability.artifact_id.clone());
        emit!(row);
    }
    for finding in &store.findings {
        let mut row = StoredRow::new("findings", finding)?;
        row.record_id = finding.observation_id.clone();
        row.artifact_id = Some(finding.artifact_id.clone());
        emit!(row);
    }
    for resource in &store.resources {
        let mut row = StoredRow::new("resources", resource)?;
        row.record_id = resource.occurrence_id.clone();
        row.subject_id = Some(resource.resource_id.clone());
        if let entmoot_contracts::ResourceOrigin::Artifact { artifact_id, .. } = &resource.origin {
            row.artifact_id = Some(artifact_id.clone());
        }
        emit!(row);
    }
    for location in &store.locations {
        let mut row = StoredRow::new("locations", location)?;
        row.record_id = location.location_id.clone();
        row.artifact_id = Some(location.artifact_id.clone());
        row.location_id = Some(location.location_id.clone());
        emit!(row);
    }
    for trace in &store.traces {
        let mut row = StoredRow::new("traces", trace)?;
        row.record_id = trace.trace_id.clone();
        row.subject_id = Some(trace.finding_observation_id.clone());
        emit!(row);
    }
    for step in &store.trace_steps {
        let mut row = StoredRow::new("trace_steps", step)?;
        row.record_id = step.step_id.clone();
        row.subject_id = Some(step.trace_id.clone());
        row.location_id = Some(step.location_id.clone());
        emit!(row);
    }
    for attachment in &store.attachments {
        let mut row = StoredRow::new("attachments", attachment)?;
        row.record_id = attachment.attachment_id.clone();
        row.artifact_id = Some(attachment.artifact_id.clone());
        row.location_id = Some(attachment.location_id.clone());
        emit!(row);
    }
    for assertion in &store.assertions {
        let mut row = StoredRow::new("assertions", assertion)?;
        row.record_id = assertion.assertion_id.clone();
        row.subject_id = Some(assertion.subject.entity_id.clone());
        row.predicate = Some(assertion.predicate.clone());
        if let Some(artifact_id) = assertion.evidence_refs.first().map(|e| &e.artifact_id) {
            row.artifact_id = Some(artifact_id.clone());
        }
        emit!(row);
    }
    plain_rows!("resolutions", &store.resolutions);
    for coverage in &store.resolver_coverages {
        let mut row = StoredRow::new("resolver_coverages", coverage)?;
        row.artifact_id = Some(coverage.artifact_id.clone());
        emit!(row);
    }
    for record in &store.resolved_identities {
        let mut row = StoredRow::new("resolved_identities", record)?;
        row.record_id = record.record_id();
        row.subject_id = Some(record.address_id.clone());
        row.artifact_id = record
            .binding
            .source()
            .map(|source| source.artifact.artifact_id.to_string());
        emit!(row);
    }
    for proposition in &store.propositions {
        let mut row = StoredRow::new("propositions", proposition)?;
        row.record_id = proposition.id.as_str().to_string();
        row.subject_id = Some(address_identity_key(&proposition.subject));
        row.object_id = Some(address_identity_key(&proposition.object));
        row.proposition_id = Some(proposition.id.as_str().to_string());
        row.predicate = Some(format!("{:?}", proposition.predicate).to_lowercase());
        emit!(row);
    }
    for edge in &store.relation_edges {
        let mut row = StoredRow::new("relation_edges", edge)?;
        row.record_id = edge.id.as_str().to_string();
        row.subject_id = Some(address_identity_key(&edge.subject));
        row.object_id = Some(address_identity_key(&edge.object));
        row.proposition_id = Some(edge.proposition_id.as_str().to_string());
        row.predicate = Some(format!("{:?}", edge.predicate).to_lowercase());
        emit!(row);
    }
    for claim in &store.relation_claims {
        let mut row = StoredRow::new("relation_claims", claim)?;
        row.record_id = claim.id.as_str().to_string();
        row.contradicts = claim.contradicts.iter().map(ToString::to_string).collect();
        row.subject_id = Some(match &claim.subject {
            entmoot_contracts::SubjectId::Address { address } => {
                format!("address:{}", address_identity_key(address))
            }
            entmoot_contracts::SubjectId::Proposition { proposition_id } => {
                format!("proposition:{proposition_id}")
            }
            entmoot_contracts::SubjectId::RelationEdge { edge_id } => {
                format!("relation_edge:{edge_id}")
            }
            entmoot_contracts::SubjectId::RelationClaim { claim_id } => {
                format!("relation_claim:{claim_id}")
            }
            entmoot_contracts::SubjectId::Artifact { artifact_id } => {
                format!("artifact:{artifact_id}")
            }
        });
        if let entmoot_contracts::SubjectId::Proposition { proposition_id } = &claim.subject {
            row.proposition_id = Some(proposition_id.as_str().to_string());
        }
        row.predicate = Some(claim.predicate.clone());
        if let Some(artifact_id) = claim.evidence.first().map(|e| &e.artifact_id) {
            row.artifact_id = Some(artifact_id.clone());
        }
        emit!(row);
    }
    for relation in &store.unresolved_relations {
        let mut row = StoredRow::new("unresolved_relations", relation)?;
        row.record_id = relation.id.as_str().to_string();
        row.subject_id = Some(address_identity_key(&relation.source));
        row.predicate = Some(format!("{:?}", relation.predicate).to_lowercase());
        if let Some(artifact_id) = relation.evidence.first().map(|e| &e.artifact_id) {
            row.artifact_id = Some(artifact_id.clone());
        }
        emit!(row);
    }
    for root in &store.input_roots {
        root.validate()
            .map_err(|error| KnowledgeRepositoryError::InvalidRecord {
                collection: "input_roots",
                reason: error.to_string(),
            })?;
        let mut row = StoredRow::new("input_roots", root)?;
        row.record_id = root.id.as_str().to_string();
        emit!(row);
    }
    for site in &store.call_sites {
        site.validate()
            .map_err(|e| invalid_record("call_sites", e))?;
        let mut row = StoredRow::new("call_sites", site)?;
        row.record_id = site.site_id.clone();
        row.subject_id = Some(site.path.clone());
        emit!(row);
    }
    for record in &store.code_contracts {
        crate::knowledge::validate_code_contract(record)
            .map_err(|e| invalid_record("code_contracts", e))?;
        let mut row = StoredRow::new("code_contracts", record)?;
        row.record_id = record.record_id.clone();
        row.subject_id = Some(record.path.clone());
        emit!(row);
    }
    for site in &store.declaration_sites {
        crate::knowledge::validate_declaration_site(site)
            .map_err(|e| invalid_record("declaration_sites", e))?;
        let mut row = StoredRow::new("declaration_sites", site)?;
        row.record_id = site.site_id.clone();
        row.subject_id = Some(site.path.clone());
        emit!(row);
    }
    for coverage in &store.coverages {
        coverage
            .validate()
            .map_err(|e| invalid_record("coverages", e))?;
        let mut row = StoredRow::new("coverages", coverage)?;
        row.record_id = coverage.coverage_id.clone();
        row.subject_id = Some(coverage.snapshot.repository_id.clone());
        emit!(row);
    }
    plain_rows!("diagnostics", &store.diagnostics);
    plain_rows!("structural_edges", &store.structural_edges);
    Ok(count)
}

fn record_state_in_connection(
    connection: &Connection,
    record: &KnowledgeRecordRef,
) -> Result<Option<KnowledgeRecordState>, KnowledgeRepositoryError> {
    record.validate()?;
    connection
        .query_row(
            "SELECT visible, ownership_managed FROM knowledge_records
                 WHERE collection = ?1 AND record_id = ?2",
            params![record.collection, record.record_id],
            |row| {
                Ok(KnowledgeRecordState {
                    visible: row.get(0)?,
                    ownership_managed: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn prepare_owner_replacements<'a, 'b>(
    publications: &'a [OwnerReplacement<'b>],
) -> Result<Vec<(&'a OwnerReplacement<'b>, String)>, KnowledgeRepositoryError> {
    let context = StoreValidationContext::new(publications.iter().map(|p| p.store));
    let mut owners = BTreeSet::new();
    let mut planned = Vec::with_capacity(publications.len());
    for publication in publications {
        validate_owner(publication.owner_id)?;
        if !owners.insert(publication.owner_id) {
            return Err(KnowledgeRepositoryError::InvalidOwner);
        }
        validate_store_records_in_context(publication.store, &context)?;
        let digest = owner_publication_digest_from_store(
            publication.owner_id,
            publication.store,
            publication.dependencies,
        )?;
        planned.push((publication, digest));
    }

    Ok(planned)
}

fn apply_owner_replacements(
    transaction: &Transaction<'_>,
    planned: &[(&OwnerReplacement<'_>, String)],
) -> Result<Vec<KnowledgeOwnershipStats>, KnowledgeRepositoryError> {
    let mut results = Vec::with_capacity(planned.len());
    for (publication, digest) in planned {
        if owner_publication_digest_matches(transaction, publication.owner_id, digest)? {
            results.push(KnowledgeOwnershipStats::default());
            continue;
        }
        results.push(replace_owner_in_transaction(
            transaction,
            publication.owner_id,
            publication.store,
            publication.dependencies,
            digest,
        )?);
    }
    Ok(results)
}

fn replace_owner_in_transaction(
    transaction: &Transaction<'_>,
    owner_id: &str,
    store: &KnowledgeStore,
    dependencies: &[KnowledgeDependency],
    publication_digest: &str,
) -> Result<KnowledgeOwnershipStats, KnowledgeRepositoryError> {
    validate_owner(owner_id)?;
    // The complete multi-owner schema universe was validated before staging.

    let rows = unique_owned_rows(store)?;
    let dependency_map = validate_dependency_batch(&rows, dependencies)?;
    let old_owned = owner_records(transaction, owner_id)?;

    let mut inserted_records = 0usize;
    for (record, row) in &rows {
        let existing = transaction
            .query_row(
                "SELECT payload FROM knowledge_records
                     WHERE collection = ?1 AND record_id = ?2",
                params![record.collection, record.record_id],
                |result| result.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if !payload_matches(transaction, existing, &row.payload)? {
                return Err(KnowledgeRepositoryError::ImmutableRecordCollision {
                    collection: record.collection.clone(),
                    record_id: record.record_id.clone(),
                });
            }
            transaction.execute(
                "UPDATE scoped_records SET ownership_managed = 1
                     WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND collection = ?1 AND record_id = ?2",
                params![record.collection, record.record_id],
            )?;
        } else {
            insert_owned_row(transaction, row)?;
            inserted_records += 1;
        }
    }

    for inputs in dependency_map.values() {
        for input in inputs {
            let exists = rows.contains_key(input)
                || transaction
                    .query_row(
                        "SELECT 1 FROM knowledge_records
                             WHERE collection = ?1 AND record_id = ?2",
                        params![input.collection, input.record_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
            if !exists {
                return Err(KnowledgeRepositoryError::MissingDependency {
                    collection: input.collection.clone(),
                    record_id: input.record_id.clone(),
                });
            }
        }
    }

    let empty_inputs = BTreeSet::new();
    for dependent in rows.keys() {
        let inputs = dependency_map.get(dependent).unwrap_or(&empty_inputs);
        let desired_digest = dependency_set_digest(inputs);
        let existing_digest = transaction
            .query_row(
                "SELECT dependency_set_digest FROM knowledge_record_dependency_sets
                     WHERE collection = ?1 AND record_id = ?2",
                params![dependent.collection, dependent.record_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_digest
            .as_deref()
            .is_some_and(|existing| existing != desired_digest.as_str())
        {
            return Err(KnowledgeRepositoryError::ImmutableRecordCollision {
                collection: dependent.collection.clone(),
                record_id: dependent.record_id.clone(),
            });
        }
        transaction.execute(
            "INSERT OR IGNORE INTO scoped_record_dependency_sets
                 (view_id, collection, record_id, dependency_set_digest) VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2, ?3)",
            params![dependent.collection, dependent.record_id, desired_digest],
        )?;
        for input in inputs {
            transaction.execute(
                "INSERT OR IGNORE INTO scoped_record_dependencies
                     (view_id, collection, record_id, input_collection, input_record_id)
                     VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2, ?3, ?4)",
                params![
                    dependent.collection,
                    dependent.record_id,
                    input.collection,
                    input.record_id
                ],
            )?;
        }
    }

    transaction.execute(
        "DELETE FROM scoped_record_owners WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND owner_id = ?1",
        [owner_id],
    )?;
    for record in rows.keys() {
        transaction.execute(
            "INSERT INTO scoped_record_owners (view_id, owner_id, collection, record_id)
                 VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2, ?3)",
            params![owner_id, record.collection, record.record_id],
        )?;
    }
    let new_owned = rows.keys().cloned().collect::<BTreeSet<_>>();
    let added_owners = new_owned.difference(&old_owned).count();
    let removed_owners = old_owned.difference(&new_owned).count();
    // Immutable payload/dependency collisions were rejected above. Unchanged
    // membership therefore cannot alter visibility; seed only membership deltas.
    let seeds = old_owned
        .symmetric_difference(&new_owned)
        .cloned()
        .collect();
    let visibility = recompute_visibility(transaction, seeds, Some(owner_id))?;
    set_owner_publication_digest(transaction, owner_id, publication_digest)?;
    set_store_digest(transaction, OWNERSHIP_STORE_MARKER)?;
    Ok(KnowledgeOwnershipStats {
        inserted_records,
        added_owners,
        removed_owners,
        evaluated_records: visibility.evaluated_records,
        became_visible: visibility.became_visible,
        became_inactive: visibility.became_inactive,
    })
}

fn rows_from_store(store: &KnowledgeStore) -> Result<Vec<StoredRow>, KnowledgeRepositoryError> {
    let mut rows = Vec::new();
    for_each_store_row(store, |row| {
        rows.push(row);
        Ok(())
    })?;
    Ok(rows)
}

fn insert_store_rows(
    transaction: &Transaction<'_>,
    store: &KnowledgeStore,
) -> Result<(usize, String), KnowledgeRepositoryError> {
    let mut digest = store_digest_hasher();
    let count = insert_store_chunk(transaction, store, &mut digest)?;
    Ok((count, format!("blake3:{}", digest.finalize().to_hex())))
}

fn insert_store_chunk(
    transaction: &Transaction<'_>,
    store: &KnowledgeStore,
    digest: &mut blake3::Hasher,
) -> Result<usize, KnowledgeRepositoryError> {
    let mut statement = transaction.prepare_cached(
        "INSERT INTO scoped_records
         (view_id, collection, record_id, artifact_id, subject_id, object_id, proposition_id, predicate, location_id, payload)
         VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(view_id, collection, record_id) DO UPDATE SET
           artifact_id=excluded.artifact_id,
           subject_id=excluded.subject_id,
           object_id=excluded.object_id,
           proposition_id=excluded.proposition_id,
           predicate=excluded.predicate,
           location_id=excluded.location_id,
           payload=excluded.payload",
    )?;
    let count = for_each_store_row(store, |row| {
        update_store_digest(digest, &row);
        statement.execute(params![
            row.collection,
            row.record_id,
            row.artifact_id,
            row.subject_id,
            row.object_id,
            row.proposition_id,
            row.predicate,
            row.location_id,
            intern_row_payload(transaction, &row)?,
        ])?;
        Ok(())
    })?;
    Ok(count)
}

fn insert_rows_into_incoming(
    transaction: &Transaction<'_>,
    rows: &[StoredRow],
) -> Result<(), KnowledgeRepositoryError> {
    let mut statement = transaction.prepare_cached(
        "INSERT INTO incoming_knowledge_records
         (collection, record_id, artifact_id, subject_id, object_id, proposition_id, predicate, location_id, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for row in rows {
        statement.execute(params![
            row.collection,
            row.record_id,
            row.artifact_id,
            row.subject_id,
            row.object_id,
            row.proposition_id,
            row.predicate,
            row.location_id,
            intern_row_payload(transaction, row)?,
        ])?;
    }
    Ok(())
}

fn digest_store_rows(store: &KnowledgeStore) -> Result<String, KnowledgeRepositoryError> {
    let mut digest = store_digest_hasher();
    for_each_store_row(store, |row| {
        update_store_digest(&mut digest, &row);
        Ok(())
    })?;
    Ok(format!("blake3:{}", digest.finalize().to_hex()))
}

fn store_digest_hasher() -> blake3::Hasher {
    let mut digest = blake3::Hasher::new();
    // Domain-separate the row-wise representation from prior whole-document
    // digests and from any future synchronization digest representation.
    digest.update(b"entmoot.knowledge-store.rows.v1\0");
    digest
}

fn update_store_digest(digest: &mut blake3::Hasher, row: &StoredRow) {
    fn field(digest: &mut blake3::Hasher, bytes: &[u8]) {
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }

    fn optional_field(digest: &mut blake3::Hasher, value: Option<&str>) {
        match value {
            Some(value) => {
                digest.update(&[1]);
                field(digest, value.as_bytes());
            }
            None => {
                digest.update(&[0]);
            }
        }
    }

    field(digest, row.collection.as_bytes());
    field(digest, row.record_id.as_bytes());
    optional_field(digest, row.artifact_id.as_deref());
    optional_field(digest, row.subject_id.as_deref());
    optional_field(digest, row.object_id.as_deref());
    optional_field(digest, row.proposition_id.as_deref());
    optional_field(digest, row.predicate.as_deref());
    optional_field(digest, row.location_id.as_deref());
    field(digest, &row.payload);
}

fn set_store_digest(transaction: &Transaction<'_>, digest: &str) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO scoped_metadata (view_id, key, value) VALUES ((SELECT id FROM temp.selected_knowledge_view), 'store_digest', ?1)
         ON CONFLICT(view_id, key) DO UPDATE SET value=excluded.value",
        [digest],
    )?;
    Ok(())
}

pub fn remove_store_files(path: &Path) -> Result<(), KnowledgeRepositoryError> {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        match std::fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Marks an ownership-managed store as initialized without claiming a full-store
/// content digest (legacy `synchronize` still computes its own row digest).
const OWNERSHIP_STORE_MARKER: &str = "ownership.v1";

fn owner_publication_metadata_key(owner_id: &str) -> String {
    format!("owner_publication_digest:{owner_id}")
}

fn owner_publication_digest_matches(
    connection: &Connection,
    owner_id: &str,
    digest: &str,
) -> Result<bool, KnowledgeRepositoryError> {
    let key = owner_publication_metadata_key(owner_id);
    let current: Option<String> = connection
        .query_row(
            "SELECT value FROM knowledge_metadata WHERE key = ?1",
            [&key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(current.as_deref() == Some(digest))
}

fn set_owner_publication_digest(
    transaction: &Transaction<'_>,
    owner_id: &str,
    digest: &str,
) -> Result<(), rusqlite::Error> {
    let key = owner_publication_metadata_key(owner_id);
    transaction.execute(
        "INSERT INTO scoped_metadata (view_id, key, value) VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1, ?2)
         ON CONFLICT(view_id, key) DO UPDATE SET value=excluded.value",
        params![key, digest],
    )?;
    Ok(())
}

fn owner_publication_digest(
    owner_id: &str,
    rows: &BTreeMap<KnowledgeRecordRef, StoredRow>,
    dependencies: &BTreeMap<KnowledgeRecordRef, BTreeSet<KnowledgeRecordRef>>,
) -> String {
    owner_publication_digest_from_commitments(
        owner_id,
        rows.iter()
            .map(|(record, row)| (record, entmoot_contracts::blake3_hex(&row.payload))),
        dependencies,
    )
}

fn owner_publication_digest_from_commitments<'a>(
    owner_id: &str,
    rows: impl IntoIterator<Item = (&'a KnowledgeRecordRef, String)>,
    dependencies: &BTreeMap<KnowledgeRecordRef, BTreeSet<KnowledgeRecordRef>>,
) -> String {
    let mut digest = blake3::Hasher::new();
    digest.update(b"entmoot.owner-publication.v1\0");
    update_length_prefixed(&mut digest, owner_id.as_bytes());
    for (record, payload_digest) in rows {
        update_length_prefixed(&mut digest, record.collection.as_bytes());
        update_length_prefixed(&mut digest, record.record_id.as_bytes());
        // Payload content identity — not the SQLite row staging representation.
        update_length_prefixed(&mut digest, payload_digest.as_bytes());
        let empty = BTreeSet::new();
        let inputs = dependencies.get(record).unwrap_or(&empty);
        update_length_prefixed(&mut digest, dependency_set_digest(inputs).as_bytes());
    }
    format!("blake3:{}", digest.finalize().to_hex())
}

fn update_length_prefixed(digest: &mut blake3::Hasher, bytes: &[u8]) {
    digest.update(&(bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn owner_publication_digest_from_store(
    owner_id: &str,
    store: &KnowledgeStore,
    dependencies: &[KnowledgeDependency],
) -> Result<String, KnowledgeRepositoryError> {
    let rows = unique_owned_rows(store)?;
    let dependency_map = validate_dependency_batch(&rows, dependencies)?;
    Ok(owner_publication_digest(owner_id, &rows, &dependency_map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        relation_claim_with_id, relation_edge_with_id, topology_node_address,
        unresolved_relation_with_id, AssertionObject, ClaimScope, EntityKind, EntityRef,
        EpistemicStatus, ProducerCapabilityKind, ProducerRef, RelationClaimValue, RelationKind,
        ResolutionState, Scope, TargetConstraint, UnresolvedReason,
    };

    fn assertion(id: &str, subject: &str) -> Assertion {
        Assertion {
            assertion_id: id.into(),
            subject: EntityRef {
                entity_id: subject.into(),
                entity_kind: EntityKind::Opaque,
            },
            predicate: "entmoot.test".into(),
            object: AssertionObject {
                entity: None,
                value: Some("yes".into()),
            },
            evidence_refs: vec![],
            epistemic_status: EpistemicStatus::Observed,
            derivation: None,
            supports: vec![],
            contradicts: vec![],
            observation_context_id: None,
            valid_from: None,
            valid_to: None,
            claim_scope: Some(ClaimScope::Occurrence),
            subject_address: None,
        }
    }

    #[test]
    fn parallel_views_share_payloads_but_isolate_owners_and_replacements() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.sqlite3");
        let a = entmoot_contracts::blake3_hex(b"revision-a/build-a");
        let b = entmoot_contracts::blake3_hex(b"revision-b/build-b");
        let mut left = SqliteKnowledgeRepository::open_view(&path, &a).unwrap();
        let mut right = SqliteKnowledgeRepository::open_view(&path, &b).unwrap();
        let mut store = KnowledgeStore::default();
        store.assertions.push(assertion("same-id", "subject"));
        left.replace_owner("producer", &store, &[]).unwrap();
        right.replace_owner("producer", &store, &[]).unwrap();
        assert_eq!(left.shared_payload_count().unwrap(), 1);
        left.remove_owner("producer").unwrap();
        assert!(left.load().unwrap().assertions.is_empty());
        assert_eq!(right.load().unwrap().assertions.len(), 1);
        store.assertions[0].object.value = Some("different build result".into());
        left.replace_all(&store).unwrap();
        assert_eq!(
            right.load().unwrap().assertions[0].object.value.as_deref(),
            Some("yes")
        );
        assert_ne!(
            left.read_state_digest().unwrap(),
            right.read_state_digest().unwrap()
        );
        drop(left);
        drop(right);
        let right = SqliteKnowledgeRepository::open_view_read_only(&path, &b).unwrap();
        assert_eq!(
            right.load().unwrap().assertions[0].object.value.as_deref(),
            Some("yes")
        );
    }

    #[test]
    fn indexed_evidence_hydrates_only_selected_payloads_and_interns_metadata() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut store = KnowledgeStore::default();
        let repeated = "repeated subject metadata ".repeat(20);
        store.assertions.push(assertion("a", &repeated));
        store.assertions.push(assertion("b", &repeated));
        store.assertions.push(assertion("unrelated", "another"));
        repository.replace_all(&store).unwrap();
        let count: i64 = repository
            .connection
            .query_row("SELECT COUNT(*) FROM shared_metadata", [], |r| r.get(0))
            .unwrap();
        assert!(count > 0);
        repository.connection.execute("UPDATE shared_payloads SET body=x'00' WHERE payload_id=(SELECT payload FROM knowledge_records WHERE record_id='unrelated')", []).unwrap();
        assert_eq!(
            repository.assertions_for_subject(&repeated).unwrap().len(),
            2
        );
        assert!(repository.assertions_for_subject("another").is_err());
    }

    #[test]
    fn equal_payloads_without_events_still_have_distinct_view_fences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.sqlite3");
        let mut a =
            SqliteKnowledgeRepository::open_view(&path, &entmoot_contracts::blake3_hex(b"a"))
                .unwrap();
        let mut b =
            SqliteKnowledgeRepository::open_view(&path, &entmoot_contracts::blake3_hex(b"b"))
                .unwrap();
        a.replace_all(&KnowledgeStore::default()).unwrap();
        b.replace_all(&KnowledgeStore::default()).unwrap();
        assert_eq!(a.current_visibility_cursor().unwrap(), 0);
        assert_eq!(b.current_visibility_cursor().unwrap(), 0);
        assert_ne!(
            a.read_state_digest().unwrap(),
            b.read_state_digest().unwrap()
        );
    }

    #[test]
    fn atomic_owner_batch_resolves_schema_references_without_adding_context_owners() {
        use entmoot_contracts::{
            ArtifactFormat, ArtifactProducer, ResolvedBindingEvidence, ResolvedIdentityRecord,
            ScopeCompleteness, ScopeUniverse, SourceSnapshot, SourceState,
        };
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "fixture".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            SourceSnapshot {
                estate_id: None,
                repository_id: "fixture".into(),
                revision: "a".repeat(40),
                source_state: SourceState::Committed,
                snapshot_digest: None,
                subject: None,
                subject_aliases: Vec::new(),
            },
            ScopeUniverse::from_selectors(
                "fixture",
                vec![],
                vec!["**".into()],
                vec![],
                ScopeCompleteness::CompleteRelativeToUniverse,
                Some(&[]),
            ),
            entmoot_contracts::blake3_hex(b"fixture"),
            None,
        )
        .unwrap();
        let identity = ResolvedIdentityRecord {
            address_id: "b".repeat(64),
            resolved_hash: entmoot_contracts::blake3_hex(b"bindings"),
            resolution_policy_id: "c".repeat(64),
            binding: ResolvedBindingEvidence::Complete {
                source: entmoot_contracts::ResolverEvidenceRef {
                    producer: entmoot_contracts::ProducerIdentity {
                        producer_id: "fixture".into(),
                        version: Some("1".into()),
                        tool_name: None,
                    },
                    artifact: entmoot_contracts::ArtifactRef::from_manifest(&manifest).unwrap(),
                },
                reference_count: 1,
            },
        };
        let evidence = KnowledgeStore {
            artifacts: vec![manifest.clone()],
            ..Default::default()
        };
        let applicability = KnowledgeStore {
            resolved_identities: vec![identity.clone()],
            ..Default::default()
        };
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        assert!(repository
            .replace_owner("app", &applicability, &[])
            .is_err());
        assert!(!repository.is_initialized().unwrap());
        repository
            .replace_owners(&[
                OwnerReplacement {
                    owner_id: "evidence",
                    store: &evidence,
                    dependencies: &[],
                },
                OwnerReplacement {
                    owner_id: "app",
                    store: &applicability,
                    dependencies: &[],
                },
            ])
            .unwrap();
        let artifact_ref = KnowledgeRecordRef::new("artifacts", &manifest.artifact_id);
        assert_eq!(
            repository.owners_for_record(&artifact_ref, 10).unwrap(),
            vec!["evidence"]
        );
        assert!(
            repository
                .record_state(&KnowledgeRecordRef::new(
                    "resolved_identities",
                    identity.record_id()
                ))
                .unwrap()
                .unwrap()
                .visible
        );
        let cursor = repository.current_visibility_cursor().unwrap();
        for mutation in ["missing_artifact", "mismatched_digest"] {
            let mut invalid = applicability.clone();
            let ResolvedBindingEvidence::Complete { source, .. } =
                &mut invalid.resolved_identities[0].binding
            else {
                panic!("complete fixture")
            };
            if mutation == "missing_artifact" {
                source.artifact.artifact_id =
                    entmoot_contracts::ArtifactId::from_trusted("d".repeat(64));
            } else {
                source.artifact.content_digest = entmoot_contracts::ContentDigest::from_trusted(
                    entmoot_contracts::blake3_hex(b"different bytes"),
                );
            }
            assert!(
                repository
                    .replace_owners(&[
                        OwnerReplacement {
                            owner_id: "evidence",
                            store: &evidence,
                            dependencies: &[]
                        },
                        OwnerReplacement {
                            owner_id: "app",
                            store: &invalid,
                            dependencies: &[]
                        },
                    ])
                    .is_err(),
                "{mutation}"
            );
            assert_eq!(repository.current_visibility_cursor().unwrap(), cursor);
            assert_eq!(
                repository.load().unwrap().resolved_identities,
                vec![identity.clone()]
            );
        }
    }
    #[test]
    fn file_store_survives_reopen_and_supports_indexed_subject_query() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/store/knowledge.sqlite3");
        let expected = KnowledgeStore {
            assertions: vec![assertion("a2", "subject-2"), assertion("a1", "subject-1")],
            ..KnowledgeStore::default()
        };

        SqliteKnowledgeRepository::open(&path)
            .unwrap()
            .replace_all(&expected)
            .unwrap();
        let reopened = SqliteKnowledgeRepository::open(&path).unwrap();
        let actual = reopened.load().unwrap();
        assert_eq!(actual.assertions.len(), 2);
        assert_eq!(
            reopened
                .assertions_for_subject("subject-1")
                .unwrap()
                .iter()
                .map(|value| value.assertion_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a1"]
        );
    }

    #[test]
    fn read_only_open_never_creates_a_missing_store_and_can_query_a_committed_one() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.sqlite3");
        assert!(SqliteKnowledgeRepository::open_read_only(&missing).is_err());
        assert!(!missing.exists());

        let path = directory.path().join("knowledge.sqlite3");
        let store = KnowledgeStore {
            assertions: vec![assertion("a1", "subject-1")],
            ..KnowledgeStore::default()
        };
        SqliteKnowledgeRepository::open(&path)
            .unwrap()
            .replace_all(&store)
            .unwrap();

        let repository = SqliteKnowledgeRepository::open_read_only(&path).unwrap();
        assert!(repository.is_initialized().unwrap());
        assert_eq!(
            repository
                .assertions_for_subject("subject-1")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_read_snapshot_stays_coherent_while_another_connection_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.sqlite3");
        let mut writer = SqliteKnowledgeRepository::open(&path).unwrap();
        writer
            .replace_all(&KnowledgeStore {
                assertions: vec![assertion("old", "subject")],
                ..Default::default()
            })
            .unwrap();
        let reader = SqliteKnowledgeRepository::open_read_snapshot(&path).unwrap();
        writer
            .replace_all(&KnowledgeStore {
                assertions: vec![assertion("new", "subject")],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            reader.assertions_for_subject("subject").unwrap()[0].assertion_id,
            "old"
        );
        let fresh = SqliteKnowledgeRepository::open_read_snapshot(&path).unwrap();
        assert_eq!(
            fresh.assertions_for_subject("subject").unwrap()[0].assertion_id,
            "new"
        );
    }

    #[test]
    fn concurrent_owner_writers_and_readers_observe_atomic_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent.sqlite3");
        let records = |worker: usize, generation: usize| KnowledgeStore {
            assertions: ["left", "right"]
                .into_iter()
                .map(|side| {
                    assertion(
                        &format!("{worker}-{generation}-{side}"),
                        &format!("subject-{worker}"),
                    )
                })
                .collect(),
            ..Default::default()
        };
        let mut initial = SqliteKnowledgeRepository::open(&path).unwrap();
        for worker in 0..4 {
            initial
                .replace_owner(&format!("owner-{worker}"), &records(worker, 0), &[])
                .unwrap();
        }
        drop(initial);
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for worker in 0..4 {
                let (barrier, path, records) = (&barrier, &path, &records);
                scope.spawn(move || {
                    let mut writer = SqliteKnowledgeRepository::open(path).unwrap();
                    barrier.wait();
                    for generation in 1..=20 {
                        if generation % 2 == 0 {
                            writer
                                .replace_owner_buffered(
                                    &format!("owner-{worker}"),
                                    &mut std::iter::once(Ok(OwnerBatch {
                                        store: records(worker, generation),
                                        dependencies: vec![],
                                    })),
                                    PublicationBuffer::default(),
                                )
                                .unwrap();
                        } else {
                            writer
                                .replace_owner(
                                    &format!("owner-{worker}"),
                                    &records(worker, generation),
                                    &[],
                                )
                                .unwrap();
                        }
                    }
                });
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..100 {
                        let reader = SqliteKnowledgeRepository::open_read_snapshot(path).unwrap();
                        let first = reader
                            .assertions_for_subject(&format!("subject-{worker}"))
                            .unwrap();
                        std::thread::yield_now();
                        let second = reader
                            .assertions_for_subject(&format!("subject-{worker}"))
                            .unwrap();
                        assert_eq!(first.len(), 2);
                        assert_eq!(
                            serde_json::to_value(first).unwrap(),
                            serde_json::to_value(second).unwrap()
                        );
                    }
                });
            }
        });
        let reader = SqliteKnowledgeRepository::open_read_snapshot(&path).unwrap();
        for worker in 0..4 {
            let actual = reader
                .assertions_for_subject(&format!("subject-{worker}"))
                .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(records(worker, 20).assertions).unwrap()
            );
        }
    }

    #[test]
    fn greenfield_store_uses_v1_and_preserves_incompatible_layouts() {
        let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let version: i64 = repository
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        fingerprint::verify(&repository.connection).unwrap();
        let directory = tempfile::tempdir().unwrap();
        for version in [0, 3, 6] {
            let path = directory.path().join(format!("old-{version}.sqlite3"));
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("CREATE TABLE retained_evidence (bytes BLOB); INSERT INTO retained_evidence VALUES (x'010203');").unwrap();
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(connection);
            let before = std::fs::read(&path).unwrap();
            assert!(
                matches!(SqliteKnowledgeRepository::open(&path), Err(KnowledgeRepositoryError::UnsupportedSchema { actual, supported: 1 }) if actual == version)
            );
            assert!(
                matches!(SqliteKnowledgeRepository::open_read_only(&path), Err(KnowledgeRepositoryError::UnsupportedSchema { actual, supported: 1 }) if actual == version)
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn failed_replace_rolls_back_previous_knowledge() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut original = KnowledgeStore::default();
        original.assertions.push(assertion("original", "subject"));
        repository.replace_all(&original).unwrap();
        repository
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_replacement BEFORE INSERT ON scoped_records
                 WHEN NEW.record_id != 'blake3:never'
                 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
            )
            .unwrap();

        let mut replacement = KnowledgeStore::default();
        replacement
            .assertions
            .push(assertion("replacement", "subject"));
        assert!(repository.replace_all(&replacement).is_err());
        repository
            .connection
            .execute_batch("DROP TRIGGER reject_replacement")
            .unwrap();
        let loaded = repository.load().unwrap();
        assert_eq!(loaded.assertions[0].assertion_id, "original");
    }

    #[test]
    fn repeated_replace_is_idempotent() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        assert!(!repository.is_initialized().unwrap());
        let mut store = KnowledgeStore::default();
        store.assertions.push(assertion("a1", "subject"));
        repository.replace_all(&store).unwrap();
        assert!(repository.is_initialized().unwrap());
        repository.replace_all(&store).unwrap();
        assert_eq!(repository.load().unwrap().assertions.len(), 1);
    }

    #[test]
    fn buffered_owner_accepts_forward_inputs_and_rolls_back_late_failure() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let make = |id: &str, input: Option<&str>| OwnerBatch {
            store: KnowledgeStore {
                assertions: vec![assertion(id, "subject")],
                ..Default::default()
            },
            dependencies: input
                .map(|input| {
                    KnowledgeDependency::new(
                        KnowledgeRecordRef::new("assertions", id),
                        vec![KnowledgeRecordRef::new("assertions", input)],
                    )
                })
                .into_iter()
                .collect(),
        };
        let limits = PublicationBuffer {
            max_records: 2,
            max_bytes: 4096,
        };
        let receipt = repository
            .replace_owner_buffered(
                "producer",
                &mut vec![
                    Ok(make("dependent", Some("input"))),
                    Ok(make("input", None)),
                ]
                .into_iter(),
                limits,
            )
            .unwrap();
        assert_eq!(receipt.batches, 2);
        assert_eq!(repository.load().unwrap().assertions.len(), 2);
        let before = repository.visibility_changes(None, 100).unwrap();
        assert!(repository
            .replace_owner_buffered(
                "producer",
                &mut vec![
                    Ok(make("new", None)),
                    Err(KnowledgeRepositoryError::InvalidOwner)
                ]
                .into_iter(),
                limits
            )
            .is_err());
        assert_eq!(repository.load().unwrap().assertions.len(), 2);
        assert!(repository
            .record_state(&KnowledgeRecordRef::new("assertions", "new"))
            .unwrap()
            .is_none());
        assert_eq!(repository.visibility_changes(None, 100).unwrap(), before);
        let mut conflicting = make("input", None);
        conflicting.store.assertions[0].predicate = "changed".into();
        assert!(repository
            .replace_owner_buffered(
                "producer",
                &mut vec![Ok(make("new", None)), Ok(conflicting)].into_iter(),
                limits
            )
            .is_err());
        assert!(repository
            .replace_owner_buffered(
                "producer",
                &mut vec![Ok(make("new", Some("absent")))].into_iter(),
                limits
            )
            .is_err());
        assert!(repository
            .record_state(&KnowledgeRecordRef::new("assertions", "new"))
            .unwrap()
            .is_none());
        assert!(repository
            .replace_owner_buffered(
                "producer",
                &mut vec![Ok(make("oversize", None))].into_iter(),
                PublicationBuffer {
                    max_records: 1,
                    max_bytes: 1
                }
            )
            .is_err());
        let replay = repository
            .replace_owner_buffered(
                "producer",
                &mut vec![
                    Ok(make("dependent", Some("input"))),
                    Ok(make("input", None)),
                ]
                .into_iter(),
                limits,
            )
            .unwrap();
        assert!(replay.outcome.is_unchanged());
        assert_eq!(replay.publication.inserted_records, 0);
        assert_eq!(replay.publication.became_visible, 0);
        assert_eq!(replay.publication.evaluated_records, 0);
    }

    #[test]
    fn owner_publication_skips_unchanged_digest() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut store = KnowledgeStore::default();
        store.assertions.push(assertion("a1", "subject"));
        let first = repository
            .replace_owners(&[OwnerReplacement {
                owner_id: "snapshot:repo",
                store: &store,
                dependencies: &[],
            }])
            .unwrap();
        assert_eq!(first[0].inserted_records, 1);

        let second = repository
            .replace_owners(&[OwnerReplacement {
                owner_id: "snapshot:repo",
                store: &store,
                dependencies: &[],
            }])
            .unwrap();
        assert_eq!(second[0], KnowledgeOwnershipStats::default());
    }

    #[test]
    fn second_producer_owner_extends_without_rewriting_foreign_rows() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut inventory = KnowledgeStore::default();
        inventory
            .assertions
            .push(assertion("inv-1", "inventory-subject"));
        repository
            .replace_owners(&[OwnerReplacement {
                owner_id: "snapshot:repo",
                store: &inventory,
                dependencies: &[],
            }])
            .unwrap();

        let mut producer_a = KnowledgeStore::default();
        producer_a
            .assertions
            .push(assertion("prod-a", "producer-a-subject"));
        repository
            .replace_owners(&[OwnerReplacement {
                owner_id: "artifact:producer-a",
                store: &producer_a,
                dependencies: &[],
            }])
            .unwrap();

        let before_a = repository
            .record_state(&KnowledgeRecordRef::new("assertions", "prod-a"))
            .unwrap()
            .expect("producer-a row");
        assert!(before_a.visible);
        assert_eq!(
            repository
                .owners_for_record(&KnowledgeRecordRef::new("assertions", "inv-1"), 8)
                .unwrap(),
            vec!["snapshot:repo".to_string()]
        );

        let mut producer_b = KnowledgeStore::default();
        producer_b
            .assertions
            .push(assertion("prod-b", "producer-b-subject"));
        let extend = repository
            .replace_owners(&[OwnerReplacement {
                owner_id: "artifact:producer-b",
                store: &producer_b,
                dependencies: &[],
            }])
            .unwrap();
        assert_eq!(extend[0].inserted_records, 1);
        assert_eq!(repository.load().unwrap().assertions.len(), 3);

        // Foreign owner rows keep their payload and ownership.
        let after_a = repository
            .record_state(&KnowledgeRecordRef::new("assertions", "prod-a"))
            .unwrap()
            .expect("producer-a still present");
        assert_eq!(before_a.visible, after_a.visible);
        assert_eq!(
            repository
                .owners_for_record(&KnowledgeRecordRef::new("assertions", "prod-a"), 8)
                .unwrap(),
            vec!["artifact:producer-a".to_string()]
        );
        assert_eq!(
            repository
                .owners_for_record(&KnowledgeRecordRef::new("assertions", "prod-b"), 8)
                .unwrap(),
            vec!["artifact:producer-b".to_string()]
        );

        // Same immutable id with different bytes still fails closed across owners.
        let mut colliding = KnowledgeStore::default();
        let mut clash = assertion("prod-a", "producer-a-subject");
        clash.predicate = "mutated-by-foreign-owner".into();
        colliding.assertions.push(clash);
        assert!(matches!(
            repository.replace_owners(&[OwnerReplacement {
                owner_id: "artifact:producer-c",
                store: &colliding,
                dependencies: &[],
            }]),
            Err(KnowledgeRepositoryError::ImmutableRecordCollision { .. })
        ));
        assert_eq!(repository.load().unwrap().assertions.len(), 3);
    }

    #[test]
    fn synchronize_writes_only_the_set_delta() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut store = KnowledgeStore::default();
        store.assertions.push(assertion("a1", "subject"));
        assert_eq!(
            repository.synchronize(&store).unwrap(),
            KnowledgeSyncStats {
                upserted: 1,
                deleted: 0
            }
        );
        assert_eq!(
            repository.synchronize(&store).unwrap(),
            KnowledgeSyncStats::default()
        );

        store.assertions.clear();
        store.assertions.push(assertion("a2", "subject"));
        assert_eq!(
            repository.synchronize(&store).unwrap(),
            KnowledgeSyncStats {
                upserted: 1,
                deleted: 1
            }
        );
        assert_eq!(repository.load().unwrap().assertions[0].assertion_id, "a2");
    }

    #[test]
    fn subject_lookup_uses_covering_prefix_index() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut store = KnowledgeStore::default();
        store.assertions.push(assertion("a1", "subject"));
        repository.replace_all(&store).unwrap();
        let plan: String = repository
            .connection
            .query_row(
                "EXPLAIN QUERY PLAN SELECT payload FROM knowledge_records_by_subject
                 WHERE collection = 'assertions' AND subject_id = ?1",
                ["subject"],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            plan.contains("knowledge_by_subject_predicate"),
            "unexpected query plan: {plan}"
        );
    }

    #[test]
    fn relation_claim_and_unresolved_queries_use_semantic_subject_keys() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let source = topology_node_address("caller");
        let target = topology_node_address("callee");
        let scope = Scope {
            domain: "repo/example".into(),
            snapshot_revision: Some("a".repeat(40)),
            snapshot_digest: None,
            occurrence_id: None,
            context_id: None,
        };
        let producer = ProducerRef {
            id: "resolver".into(),
            name: "resolver".into(),
            version: Some("1".into()),
            config_fingerprint: None,
            environment_fingerprint: None,
            capabilities: vec![ProducerCapabilityKind::Resolver],
        };
        let edge = relation_edge_with_id(
            "source",
            source.clone(),
            RelationKind::Calls,
            target.clone(),
            scope.clone(),
            None,
        );
        let claim = relation_claim_with_id(
            entmoot_contracts::SubjectId::RelationEdge {
                edge_id: edge.id.clone(),
            },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Structural,
            },
            producer.clone(),
            scope.clone(),
            vec![],
            None,
        );
        let proposition_claim = relation_claim_with_id(
            entmoot_contracts::SubjectId::Proposition {
                proposition_id: edge.proposition_id.clone(),
            },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::SemanticResolved,
            },
            producer.clone(),
            scope.clone(),
            vec![],
            None,
        );
        let unresolved = unresolved_relation_with_id(
            source.clone(),
            RelationKind::Calls,
            TargetConstraint::Syntactic {
                callee: "dynamic".into(),
            },
            UnresolvedReason::IndirectCall,
            producer,
            scope,
            Some("site-1".into()),
            vec![],
            None,
            Some(ProducerCapabilityKind::Resolver),
        );
        let store = KnowledgeStore {
            propositions: vec![edge.proposition()],
            relation_edges: vec![edge.clone()],
            relation_claims: vec![claim.clone(), proposition_claim.clone()],
            unresolved_relations: vec![unresolved.clone()],
            ..KnowledgeStore::default()
        };
        repository.replace_all(&store).unwrap();

        assert_eq!(
            repository.relation_edges_for_subject(&source).unwrap(),
            vec![edge.clone()]
        );
        assert_eq!(
            repository
                .relation_claims_for_relation_edge(relation_claim_subject_edge_id(&claim))
                .unwrap(),
            vec![claim]
        );
        assert_eq!(
            repository
                .relation_edges_for_proposition(edge.proposition_id.as_str())
                .unwrap(),
            vec![edge.clone()]
        );
        assert_eq!(
            repository
                .proposition_by_id(edge.proposition_id.as_str())
                .unwrap(),
            Some(edge.proposition())
        );
        assert_eq!(
            repository.propositions_for_subject(&source).unwrap(),
            vec![edge.proposition()]
        );
        assert_eq!(
            repository.propositions_for_object(&target).unwrap(),
            vec![edge.proposition()]
        );
        assert_eq!(
            repository
                .relation_claims_for_proposition(edge.proposition_id.as_str())
                .unwrap(),
            vec![proposition_claim]
        );
        assert_eq!(
            repository.unresolved_for_source(&source).unwrap(),
            vec![unresolved]
        );
        let reverse_plan: String = repository
            .connection
            .query_row(
                "EXPLAIN QUERY PLAN SELECT payload FROM knowledge_records_by_object
                 WHERE collection = 'propositions' AND object_id = ?1",
                [address_identity_key(&target)],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            reverse_plan.contains("knowledge_by_object_predicate"),
            "unexpected reverse query plan: {reverse_plan}"
        );
    }

    #[test]
    fn relation_edge_occurrence_requires_its_materialized_proposition() {
        let edge = relation_edge_with_id(
            "source",
            topology_node_address("caller"),
            RelationKind::Calls,
            topology_node_address("callee"),
            Scope {
                domain: "repo/example".into(),
                snapshot_revision: Some("a".repeat(40)),
                snapshot_digest: None,
                occurrence_id: None,
                context_id: None,
            },
            None,
        );
        let store = KnowledgeStore {
            relation_edges: vec![edge],
            ..KnowledgeStore::default()
        };
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let error = repository.replace_all(&store).unwrap_err();
        assert!(matches!(
            error,
            KnowledgeRepositoryError::InvalidRecord {
                collection: "relation_edges",
                ..
            }
        ));
    }

    #[test]
    fn mutation_and_query_boundaries_reject_invalid_relation_addresses_and_ids() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let source = topology_node_address("caller");
        let mut invalid_edge = relation_edge_with_id(
            "source",
            source,
            RelationKind::Calls,
            topology_node_address("callee"),
            Scope {
                domain: "repo/example".into(),
                snapshot_revision: Some("a".repeat(40)),
                snapshot_digest: None,
                occurrence_id: None,
                context_id: None,
            },
            None,
        );
        invalid_edge.id = entmoot_contracts::RelationEdgeId::from_trusted("not-an-edge-id");
        let invalid_store = KnowledgeStore {
            relation_edges: vec![invalid_edge],
            ..KnowledgeStore::default()
        };
        assert!(repository.replace_all(&invalid_store).is_err());
        assert!(!repository.is_initialized().unwrap());

        let invalid_source = entmoot_contracts::Address::Opaque {
            system: String::new(),
            value: "caller".into(),
        };
        assert!(repository
            .relation_edges_for_subject(&invalid_source)
            .is_err());
        assert!(repository.unresolved_for_source(&invalid_source).is_err());
    }

    #[test]
    fn owner_replacement_uses_or_visibility_and_retains_inactive_payloads() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let shared = KnowledgeStore {
            assertions: vec![assertion("shared", "subject")],
            ..KnowledgeStore::default()
        };

        repository
            .replace_owner("source:a.py", &shared, &[])
            .unwrap();
        repository
            .replace_owner("source:b.py", &shared, &[])
            .unwrap();
        repository.remove_owner("source:a.py").unwrap();
        assert_eq!(repository.load().unwrap().assertions.len(), 1);

        repository.remove_owner("source:b.py").unwrap();
        assert!(repository.load().unwrap().assertions.is_empty());
        assert_eq!(repository.retained_record_count().unwrap(), 1);
    }

    #[test]
    fn dependencies_propagate_inactivity_and_recover_deterministically() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let base = KnowledgeStore {
            assertions: vec![assertion("base", "source")],
            ..KnowledgeStore::default()
        };
        let derived = KnowledgeStore {
            assertions: vec![
                assertion("derived-1", "middle"),
                assertion("derived-2", "sink"),
            ],
            ..KnowledgeStore::default()
        };
        repository
            .replace_owner("source:base.py", &base, &[])
            .unwrap();
        repository
            .replace_owner(
                "resolver:batch",
                &derived,
                &[
                    KnowledgeDependency::new(
                        KnowledgeRecordRef::new("assertions", "derived-1"),
                        vec![KnowledgeRecordRef::new("assertions", "base")],
                    ),
                    KnowledgeDependency::new(
                        KnowledgeRecordRef::new("assertions", "derived-2"),
                        vec![KnowledgeRecordRef::new("assertions", "derived-1")],
                    ),
                ],
            )
            .unwrap();
        assert_eq!(repository.load().unwrap().assertions.len(), 3);

        repository.remove_owner("source:base.py").unwrap();
        assert!(repository.load().unwrap().assertions.is_empty());

        repository
            .replace_owner("source:base.py", &base, &[])
            .unwrap();
        assert_eq!(repository.load().unwrap().assertions.len(), 3);
    }

    #[test]
    fn owner_batch_failure_rolls_back_all_publications() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let original = KnowledgeStore {
            assertions: vec![assertion("stable", "old")],
            ..Default::default()
        };
        repository
            .replace_owner("existing", &original, &[])
            .unwrap();
        let new = KnowledgeStore {
            assertions: vec![assertion("new", "new")],
            ..Default::default()
        };
        let collision = KnowledgeStore {
            assertions: vec![assertion("stable", "different")],
            ..Default::default()
        };
        let cursor = repository
            .visibility_changes(None, 100)
            .unwrap()
            .next_cursor;
        assert!(repository
            .replace_owners(&[
                OwnerReplacement {
                    owner_id: "first",
                    store: &new,
                    dependencies: &[]
                },
                OwnerReplacement {
                    owner_id: "second",
                    store: &collision,
                    dependencies: &[]
                },
            ])
            .is_err());
        assert_eq!(repository.load().unwrap().assertions, original.assertions);
        assert!(repository
            .records_for_owner("first", 100)
            .unwrap()
            .is_empty());
        assert!(repository
            .visibility_changes(Some(cursor), 100)
            .unwrap()
            .events
            .is_empty());
    }

    #[test]
    fn path_summary_invalidation_includes_frontier_roots_and_transitive_summaries() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let facts = KnowledgeStore {
            assertions: vec![assertion("role", "source")],
            ..Default::default()
        };
        let frontier = KnowledgeStore {
            assertions: vec![assertion("frontier-v1", "build-scope")],
            ..Default::default()
        };
        repository
            .replace_owner("provider:roles", &facts, &[])
            .unwrap();
        repository
            .replace_owner("resolver:frontier", &frontier, &[])
            .unwrap();
        let summaries = KnowledgeStore {
            assertions: vec![
                assertion("one-hop", "caller"),
                assertion("two-hop", "outer-caller"),
            ],
            ..Default::default()
        };
        repository
            .replace_owner(
                "consumer:path-membership",
                &summaries,
                &[
                    KnowledgeDependency::new(
                        KnowledgeRecordRef::new("assertions", "one-hop"),
                        vec![
                            KnowledgeRecordRef::new("assertions", "role"),
                            KnowledgeRecordRef::new("assertions", "frontier-v1"),
                        ],
                    ),
                    KnowledgeDependency::new(
                        KnowledgeRecordRef::new("assertions", "two-hop"),
                        vec![KnowledgeRecordRef::new("assertions", "one-hop")],
                    ),
                ],
            )
            .unwrap();
        assert_eq!(repository.load().unwrap().assertions.len(), 4);
        // New possible targets invalidate membership summaries even if the old
        // role and every old positive edge survive. No negative claim is emitted.
        let changed = KnowledgeStore {
            assertions: vec![assertion("frontier-v2", "build-scope")],
            ..Default::default()
        };
        repository
            .replace_owner("resolver:frontier", &changed, &[])
            .unwrap();
        let visible = repository.load().unwrap();
        assert_eq!(visible.assertions.len(), 2);
        assert!(visible
            .assertions
            .iter()
            .any(|fact| fact.assertion_id == "role"));
        assert!(repository
            .assertions_for_subject("caller")
            .unwrap()
            .is_empty());
        assert!(repository
            .assertions_for_subject("outer-caller")
            .unwrap()
            .is_empty());
        assert_eq!(repository.retained_record_count().unwrap(), 5);
        let replay = repository
            .replace_owner("resolver:frontier", &changed, &[])
            .unwrap();
        assert_eq!(replay.became_inactive, 0);
        assert_eq!(replay.became_visible, 0);
    }

    #[test]
    fn expert_owner_and_dependency_lookups_are_exact_and_bounded() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let base = KnowledgeStore {
            assertions: vec![assertion("base", "source")],
            ..KnowledgeStore::default()
        };
        let derived = KnowledgeStore {
            assertions: vec![assertion("derived", "sink")],
            ..KnowledgeStore::default()
        };
        let base_ref = KnowledgeRecordRef::new("assertions", "base");
        let derived_ref = KnowledgeRecordRef::new("assertions", "derived");
        repository.replace_owner("source:a.py", &base, &[]).unwrap();
        repository
            .replace_owner(
                "resolver:batch",
                &derived,
                &[KnowledgeDependency::new(
                    derived_ref.clone(),
                    vec![base_ref.clone()],
                )],
            )
            .unwrap();

        assert_eq!(
            repository.records_for_owner("resolver:batch", 1).unwrap(),
            std::slice::from_ref(&derived_ref)
        );
        assert_eq!(
            repository.owners_for_record(&derived_ref, 1).unwrap(),
            ["resolver:batch"]
        );
        assert_eq!(
            repository.record_state(&derived_ref).unwrap(),
            Some(KnowledgeRecordState {
                visible: true,
                ownership_managed: true,
            })
        );
        assert_eq!(
            repository
                .dependency_inputs_for_record(&derived_ref, 1)
                .unwrap(),
            std::slice::from_ref(&base_ref)
        );
        assert_eq!(
            repository
                .dependent_records_for_input(&base_ref, 1)
                .unwrap(),
            [derived_ref]
        );
        assert!(matches!(
            repository.records_for_owner("resolver:batch", 0),
            Err(KnowledgeRepositoryError::InvalidPageSize)
        ));
    }

    #[test]
    fn trace_lookup_preserves_provider_step_order_by_semantic_ids() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let trace = TraceRecord {
            trace_id: "trace-1".into(),
            finding_observation_id: "finding-1".into(),
            code_flow_index: 0,
            thread_index: 0,
            step_ids: vec!["step-1".into(), "step-2".into()],
        };
        let store = KnowledgeStore {
            traces: vec![trace.clone()],
            trace_steps: vec![
                TraceStepRecord {
                    step_id: "step-2".into(),
                    trace_id: "trace-1".into(),
                    ordinal: 1,
                    location_id: "location-2".into(),
                    provider_importance: None,
                    provider_message: None,
                    provider_kinds: vec![],
                },
                TraceStepRecord {
                    step_id: "step-1".into(),
                    trace_id: "trace-1".into(),
                    ordinal: 0,
                    location_id: "location-1".into(),
                    provider_importance: None,
                    provider_message: None,
                    provider_kinds: vec![],
                },
            ],
            ..KnowledgeStore::default()
        };
        repository.replace_all(&store).unwrap();

        assert_eq!(repository.trace_by_id("trace-1").unwrap(), Some(trace));
        assert_eq!(
            repository
                .trace_steps_for_trace("trace-1", 2)
                .unwrap()
                .iter()
                .map(|step| step.step_id.as_str())
                .collect::<Vec<_>>(),
            ["step-1", "step-2"]
        );
    }

    #[test]
    fn owner_replacement_rejects_immutable_id_collision_without_mutation() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let original = KnowledgeStore {
            assertions: vec![assertion("same-id", "first")],
            ..KnowledgeStore::default()
        };
        repository
            .replace_owner("source:first.py", &original, &[])
            .unwrap();
        let conflicting = KnowledgeStore {
            assertions: vec![assertion("same-id", "second")],
            ..KnowledgeStore::default()
        };

        let error = repository
            .replace_owner("source:second.py", &conflicting, &[])
            .unwrap_err();
        assert!(matches!(
            error,
            KnowledgeRepositoryError::ImmutableRecordCollision { .. }
        ));
        assert_eq!(
            repository.load().unwrap().assertions[0].subject.entity_id,
            "first"
        );
    }

    #[test]
    fn visibility_cursor_pages_have_no_gaps_or_duplicates() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        for (owner, id) in [("source:a.py", "a"), ("source:b.py", "b")] {
            repository
                .replace_owner(
                    owner,
                    &KnowledgeStore {
                        assertions: vec![assertion(id, id)],
                        ..KnowledgeStore::default()
                    },
                    &[],
                )
                .unwrap();
        }

        let first = repository.visibility_changes(None, 1).unwrap();
        assert_eq!(first.events.len(), 1);
        assert!(first.truncated);
        let second = repository
            .visibility_changes(Some(first.next_cursor), 10)
            .unwrap();
        assert_eq!(second.events.len(), 1);
        assert_ne!(first.events[0].record, second.events[0].record);
        assert!(!second.truncated);
    }

    #[test]
    fn owner_replacement_is_idempotent_and_walks_only_its_frontier() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        for index in 0..64 {
            repository
                .replace_owner(
                    &format!("source:{index}.py"),
                    &KnowledgeStore {
                        assertions: vec![assertion(&format!("a-{index}"), "subject")],
                        ..KnowledgeStore::default()
                    },
                    &[],
                )
                .unwrap();
        }
        let isolated = KnowledgeStore {
            assertions: vec![assertion("a-7", "subject")],
            ..KnowledgeStore::default()
        };
        let cursor = repository
            .visibility_changes(None, 100)
            .unwrap()
            .next_cursor;

        let stats = repository
            .replace_owner("source:7.py", &isolated, &[])
            .unwrap();
        assert_eq!(
            stats,
            KnowledgeOwnershipStats {
                evaluated_records: 0,
                ..KnowledgeOwnershipStats::default()
            }
        );
        assert!(repository
            .visibility_changes(Some(cursor), 1)
            .unwrap()
            .events
            .is_empty());
    }

    #[test]
    fn missing_dependency_rolls_back_inserted_payload_and_owner() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let derived = KnowledgeStore {
            assertions: vec![assertion("derived", "subject")],
            ..KnowledgeStore::default()
        };
        let error = repository
            .replace_owner(
                "resolver:batch",
                &derived,
                &[KnowledgeDependency::new(
                    KnowledgeRecordRef::new("assertions", "derived"),
                    vec![KnowledgeRecordRef::new("assertions", "missing")],
                )],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            KnowledgeRepositoryError::MissingDependency { .. }
        ));
        assert_eq!(repository.retained_record_count().unwrap(), 0);
        assert!(!repository.is_initialized().unwrap());
    }

    #[test]
    fn owners_cannot_disagree_about_an_immutable_dependency_set() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let base = KnowledgeStore {
            assertions: vec![assertion("base", "base")],
            ..KnowledgeStore::default()
        };
        let shared = KnowledgeStore {
            assertions: vec![assertion("shared", "shared")],
            ..KnowledgeStore::default()
        };
        repository.replace_owner("base", &base, &[]).unwrap();
        repository.replace_owner("first", &shared, &[]).unwrap();

        let error = repository
            .replace_owner(
                "second",
                &shared,
                &[KnowledgeDependency::new(
                    KnowledgeRecordRef::new("assertions", "shared"),
                    vec![KnowledgeRecordRef::new("assertions", "base")],
                )],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            KnowledgeRepositoryError::ImmutableRecordCollision { .. }
        ));
        assert_eq!(repository.load().unwrap().assertions.len(), 2);
    }

    #[test]
    fn incremental_queries_use_owner_dependency_and_cursor_indexes() {
        let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        for (sql, expected) in [
            (
                "EXPLAIN QUERY PLAN SELECT 1 FROM knowledge_record_owners
                 WHERE collection = 'assertions' AND record_id = 'a' LIMIT 1",
                "knowledge_owners_by_record",
            ),
            (
                "EXPLAIN QUERY PLAN SELECT collection, record_id
                 FROM knowledge_record_dependencies
                 WHERE input_collection = 'assertions' AND input_record_id = 'a'",
                "knowledge_dependencies_by_input",
            ),
            (
                "EXPLAIN QUERY PLAN SELECT cursor FROM knowledge_visibility_events
                 WHERE cursor > 0 ORDER BY cursor LIMIT 10",
                "knowledge_events_by_view",
            ),
        ] {
            let plan: String = repository
                .connection
                .query_row(sql, [], |row| row.get(3))
                .unwrap();
            assert!(plan.contains(expected), "unexpected query plan: {plan}");
        }
    }

    #[test]
    fn incremental_boundary_rejects_ambiguous_owner_and_cursor_inputs() {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        assert!(matches!(
            repository.replace_owner("  ", &KnowledgeStore::default(), &[]),
            Err(KnowledgeRepositoryError::InvalidOwner)
        ));
        assert!(matches!(
            repository.visibility_changes(None, 0),
            Err(KnowledgeRepositoryError::InvalidPageSize)
        ));
        assert!(!repository.is_initialized().unwrap());
    }

    fn relation_claim_subject_edge_id(claim: &RelationClaim) -> &str {
        match &claim.subject {
            entmoot_contracts::SubjectId::RelationEdge { edge_id } => edge_id.as_str(),
            _ => panic!("test claim must target an edge"),
        }
    }
}
