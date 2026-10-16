//! Repository-owned operational state, separate from immutable knowledge records.
use super::*;
mod binding;
mod file_progress;
mod git_capture;
mod integration;
mod revalidation;
mod scheduler;
mod stitch;
use entmoot_contracts::{
    ContentDigest, OperationControl, OperationOutput, OperationOutputPage, OperationRecord,
    OperationSpec, OperationState, WorkspacePublication,
};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE local_workspaces (
    workspace_id TEXT PRIMARY KEY, repository_id TEXT NOT NULL, view_id TEXT NOT NULL,
    generation INTEGER NOT NULL, body BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE local_view_seals(
    view_id TEXT PRIMARY KEY, published INTEGER NOT NULL CHECK(published IN(0,1))
) WITHOUT ROWID;
CREATE TABLE local_operation_outputs (
    operation_id TEXT NOT NULL REFERENCES local_operations(operation_id),
    sequence INTEGER NOT NULL, body BLOB NOT NULL,
    PRIMARY KEY(operation_id,sequence)
) WITHOUT ROWID;
CREATE TABLE local_operations (
    ordinal INTEGER PRIMARY KEY, operation_id TEXT NOT NULL UNIQUE,
    repository_id TEXT NOT NULL, idempotency_key TEXT NOT NULL,
    state TEXT NOT NULL, body BLOB NOT NULL,
    UNIQUE(repository_id,idempotency_key)
);
CREATE INDEX local_operations_by_state ON local_operations(state,ordinal);
CREATE TABLE local_operation_pins (
    operation_id TEXT NOT NULL REFERENCES local_operations(operation_id),
    view_id TEXT NOT NULL, PRIMARY KEY(operation_id,view_id)
) WITHOUT ROWID;
CREATE TABLE local_query_checkpoints (
    checkpoint_key TEXT PRIMARY KEY, view_id TEXT NOT NULL, generation TEXT NOT NULL,
    sequence INTEGER NOT NULL, expires_ms INTEGER NOT NULL,
    digest TEXT NOT NULL, body BLOB NOT NULL
) WITHOUT ROWID;
"#;

// Registered views are immutable, including visibility and producer ownership.
// SQL fences cover every mutation API and avoid a check/write race across clients.
pub(super) fn schema() -> String {
    let mut sql = SCHEMA.to_owned();
    sql.push_str(file_progress::SCHEMA);
    sql.push_str(git_capture::SCHEMA);
    sql.push_str(integration::SCHEMA);
    sql.push_str(stitch::SCHEMA);
    sql.push_str(scheduler::SCHEMA);
    for table in [
        "scoped_records",
        "scoped_metadata",
        "scoped_record_owners",
        "scoped_record_dependencies",
        "scoped_record_dependency_sets",
        "scoped_visibility_events",
        "inventory_headers",
        "inventory_nodes",
        "inventory_files",
    ] {
        for (action, condition) in [
            ("INSERT", "view_id=NEW.view_id"),
            ("DELETE", "view_id=OLD.view_id"),
            ("UPDATE", "view_id=OLD.view_id OR view_id=NEW.view_id"),
        ] {
            sql.push_str(&format!("CREATE TRIGGER seal_{table}_{action} BEFORE {action} ON {table} WHEN EXISTS(SELECT 1 FROM local_view_seals WHERE {condition}) BEGIN SELECT RAISE(ABORT,'registered knowledge view is immutable; publish a new view'); END;\n"));
        }
    }
    sql
}

#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error("invalid operation: {0}")]
    Invalid(String),
    #[error("operation conflict: {0}")]
    Conflict(String),
    #[error("operation admission capacity exhausted")]
    Capacity,
    #[error("operation resource budget exhausted: {0}")]
    Budget(String),
    #[error("operation unavailable: {0}")]
    Unavailable(String),
    #[error(transparent)]
    Repository(#[from] KnowledgeRepositoryError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl OperationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::Conflict(_) => "conflict",
            Self::Capacity => "capacity_exhausted",
            Self::Budget(_) => "budget_exhausted",
            Self::Unavailable(_) => "unavailable",
            Self::Repository(_) | Self::Sqlite(_) | Self::Json(_) => "storage_error",
        }
    }
}

const MAX_BODY: usize = 16 * 1024 * 1024;

impl SqliteKnowledgeRepository {
    /// Import and register through a single transaction. Concurrent identical imports
    /// cannot observe half-initialized knowledge or race a newly installed seal.
    pub(crate) fn import_publication(
        &mut self,
        graph: &crate::TopologyGraph,
        publication: WorkspacePublication,
        expected: u64,
    ) -> Result<WorkspacePublication, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        inventory::import_graph_in_transaction(&tx, graph, None)?;
        let publication = publish_workspace_in_transaction(&tx, publication, expected)?;
        tx.commit()?;
        Ok(publication)
    }

    /// This connection selects the result view. Staging, sealing, retention pinning
    /// and the next checkpoint commit together, without making a registry entry.
    pub(crate) fn stage_producer_graph(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        graph: &crate::TopologyGraph,
        source: Option<&dyn crate::SourceStore>,
    ) -> Result<OperationRecord, OperationError> {
        self.stage_producer_graph_with_cache(id, generation, now_ms, graph, source, None)
    }

    pub(crate) fn stage_producer_graph_with_cache(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        graph: &crate::TopologyGraph,
        source: Option<&dyn crate::SourceStore>,
        cache: Option<&syntax_cache::SyntaxCache>,
    ) -> Result<OperationRecord, OperationError> {
        let cache = cache
            .map(syntax_cache::PreparedCache::prepare)
            .transpose()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = live_producer(&tx, id, generation, now_ms)?;
        let request: entmoot_contracts::ProducerRequest =
            serde_json::from_value(item.spec.input.clone())?;
        if matches!(
            request.task,
            entmoot_contracts::ProducerTask::Binding { .. }
        ) {
            return Err(OperationError::Invalid(
                "binding requires its native receipt checkpoint owner".into(),
            ));
        }
        if request.source != crate::producer_operations::graph_source(graph)? {
            return Err(OperationError::Conflict(
                "producer result source differs from request".into(),
            ));
        }
        if !item.checkpoint.is_null()
            && item.checkpoint.get("inventory").is_none()
            && item.checkpoint.get("canon").is_none()
        {
            return Err(OperationError::Conflict(
                "producer result already staged".into(),
            ));
        }
        file_progress::require_complete_if_present(&tx, id)?;
        stitch::require_complete_if_present(&tx, &item)?;
        let view = stage_graph(&tx, id, graph, source)?;
        if let Some(cache) = cache {
            if item.spec.kind != entmoot_contracts::OperationKind::Inventory {
                return Err(OperationError::Invalid(
                    "only inventory may publish syntax cache fragments".into(),
                ));
            }
            cache.store(&tx)?;
        }
        item.checkpoint = serde_json::json!({"view":view});
        item.checkpoint_sequence = next(item.checkpoint_sequence)?;
        tx.execute(
            "DELETE FROM local_file_progress WHERE operation_id=?1",
            [id],
        )?;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    /// Complete a prepared producer and its frozen registry CAS in one commit.
    pub(crate) fn publish_producer(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
    ) -> Result<OperationRecord, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = live_producer(&tx, id, generation, now_ms)?;
        let request: entmoot_contracts::ProducerRequest =
            serde_json::from_value(item.spec.input.clone())?;
        binding::require_complete(&item, &request)?;
        let view = item
            .checkpoint
            .get("view")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OperationError::Invalid("producer result is not staged".into()))?;
        let publication = publish_workspace_in_transaction(
            &tx,
            WorkspacePublication {
                workspace_id: request.publication.workspace_id,
                branch: request.publication.branch,
                snapshot: request.source.snapshot,
                view: ContentDigest::parse(view)
                    .map_err(|e| OperationError::Invalid(e.to_string()))?,
                generation: 0,
                publisher: request.publication.publisher,
                active: true,
                git_capture: None,
            },
            request.publication.expected_generation,
        )?;
        item.state = OperationState::Completed;
        item.lease_until_ms = None;
        item.checkpoint_sequence = next(item.checkpoint_sequence)?;
        item.result = Some(serde_json::to_value(publication)?);
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    pub(crate) fn query_checkpoint(
        &self,
        key: &str,
        generation: &str,
        now_ms: u64,
    ) -> Result<Option<(usize, Vec<u8>)>, OperationError> {
        let row: Option<(String,i64,i64,String,Vec<u8>)> = self.connection.query_row(
            "SELECT generation,sequence,expires_ms,digest,body FROM local_query_checkpoints WHERE checkpoint_key=?1", [key],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((stored, sequence, expires, digest, body)) = row else {
            return Ok(None);
        };
        if stored != generation
            || expires < 0
            || now_ms >= u64::try_from(expires).unwrap_or(u64::MAX)
        {
            return Err(OperationError::Conflict(
                "query checkpoint expired or publication changed".into(),
            ));
        }
        if digest != entmoot_contracts::blake3_hex(&body) {
            return Err(OperationError::Invalid(
                "query checkpoint digest mismatch".into(),
            ));
        }
        let sequence =
            usize::try_from(sequence).map_err(|e| OperationError::Invalid(e.to_string()))?;
        Ok(Some((sequence, body)))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_query_checkpoint(
        &mut self,
        key: &str,
        generation: &str,
        expected: usize,
        sequence: usize,
        expires_ms: u64,
        body: &[u8],
        capacity: usize,
        bytes: usize,
    ) -> Result<(), OperationError> {
        let now = crate::collaboration::now_ms()?;
        let expires =
            i64::try_from(expires_ms).map_err(|e| OperationError::Invalid(e.to_string()))?;
        let sequence =
            i64::try_from(sequence).map_err(|e| OperationError::Invalid(e.to_string()))?;
        if body.len() > bytes {
            return Err(OperationError::Capacity);
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM local_query_checkpoints WHERE expires_ms<=?1",
            [i64::try_from(now).map_err(|e| OperationError::Invalid(e.to_string()))?],
        )?;
        let current: Option<(i64, String)> = tx
            .query_row(
                "SELECT sequence,generation FROM local_query_checkpoints WHERE checkpoint_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if current.as_ref().map_or(0, |r| r.0)
            != i64::try_from(expected).map_err(|e| OperationError::Invalid(e.to_string()))?
            || current.as_ref().is_some_and(|r| r.1 != generation)
        {
            return Err(OperationError::Conflict(
                "query concurrently advanced or checkpoint expired".into(),
            ));
        }
        let (count, retained): (i64,i64) = tx.query_row("SELECT COUNT(*),COALESCE(SUM(length(body)),0) FROM local_query_checkpoints WHERE checkpoint_key<>?1", [key], |r| Ok((r.get(0)?,r.get(1)?)))?;
        if usize::try_from(count).unwrap_or(usize::MAX) >= capacity
            || usize::try_from(retained)
                .unwrap_or(usize::MAX)
                .saturating_add(body.len())
                > bytes
        {
            return Err(OperationError::Capacity);
        }
        tx.execute("INSERT INTO local_query_checkpoints VALUES(?1,(SELECT id FROM temp.selected_knowledge_view),?2,?3,?4,?5,?6) ON CONFLICT(checkpoint_key) DO UPDATE SET sequence=excluded.sequence,expires_ms=excluded.expires_ms,digest=excluded.digest,body=excluded.body",
            params![key,generation,sequence,expires,entmoot_contracts::blake3_hex(body),body])?;
        tx.commit()?;
        Ok(())
    }

    /// Operational reads may precede the first knowledge publication. They must
    /// still verify compatibility and must never create tables or missing stores.
    pub fn open_operations_read_only(path: &Path) -> Result<Self, OperationError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != STORAGE_SCHEMA_VERSION {
            return Err(KnowledgeRepositoryError::UnsupportedSchema {
                actual: version,
                supported: STORAGE_SCHEMA_VERSION,
            }
            .into());
        }
        fingerprint::verify(&connection)?;
        let lease =
            crate::storage_lease::shared(path, false).map_err(KnowledgeRepositoryError::from)?;
        layout::select_view(&connection, "default")?;
        Ok(Self {
            connection,
            lease: Some(lease),
        })
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, OperationError> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_BODY {
        return Err(OperationError::Invalid(
            "operational record exceeds 16 MiB".into(),
        ));
    }
    Ok(bytes)
}

fn next(value: u64) -> Result<u64, OperationError> {
    value
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| OperationError::Invalid("generation exhausted".into()))
}

fn check_text(value: &str) -> Result<(), OperationError> {
    if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
        return Err(OperationError::Invalid(
            "identity must contain 1..4096 nonempty bytes without NUL".into(),
        ));
    }
    Ok(())
}

fn view_snapshot(
    connection: &Connection,
    view: &str,
) -> Result<entmoot_contracts::SourceSnapshot, OperationError> {
    let payload: Option<i64> = connection.query_row(
        "SELECT payload FROM inventory_headers WHERE view_id=?1 AND EXISTS(SELECT 1 FROM scoped_metadata WHERE view_id=?1 AND key='store_digest')",
        [view], |r| r.get(0)).optional()?;
    let payload =
        payload.ok_or_else(|| OperationError::Invalid("view has no published inventory".into()))?;
    let graph: crate::TopologyGraph = hydrate_record(connection, payload)?;
    Ok(graph.snapshot)
}

fn read_operation(
    connection: &Connection,
    id: &str,
) -> Result<Option<OperationRecord>, OperationError> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT body FROM local_operations WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    bytes
        .map(|b| serde_json::from_slice(&b).map_err(Into::into))
        .transpose()
}

fn save(connection: &Connection, item: &OperationRecord) -> Result<(), OperationError> {
    connection.execute(
        "UPDATE local_operations SET state=?2,body=?3 WHERE operation_id=?1",
        params![
            item.id.as_str(),
            serde_json::to_string(&item.state)?,
            encode(item)?
        ],
    )?;
    // A state/cursor/lease transition invalidates any prior waiting interest.
    connection.execute(
        "DELETE FROM local_operation_waiters WHERE operation_id=?1",
        [item.id.as_str()],
    )?;
    if item.state.terminal() {
        if item.state == OperationState::Cancelled {
            revalidation::cancel(connection, item)?;
        }
        connection.execute(
            "DELETE FROM local_integration_packs WHERE operation_id=?1",
            [item.id.as_str()],
        )?;
        connection.execute(
            "DELETE FROM local_file_progress WHERE operation_id=?1",
            [item.id.as_str()],
        )?;
        connection.execute(
            "DELETE FROM local_operation_pins WHERE operation_id=?1",
            [item.id.as_str()],
        )?;
    }
    Ok(())
}

/// Operational extension of the authoritative repository. Implementations must
/// atomically fence state transitions; none of these methods execute producers.
pub trait LocalOperations: KnowledgeRepository {
    fn scheduler_status(
        &self,
        repository: &str,
        now_ms: u64,
    ) -> Result<entmoot_contracts::SchedulerStatus, OperationError>;
    fn configure_scheduler(
        &mut self,
        repository: &str,
        expected: u64,
        policy: entmoot_contracts::SchedulerPolicy,
    ) -> Result<entmoot_contracts::SchedulerConfiguration, OperationError>;
    /// Extend only a live generation. Does not advance its semantic checkpoint.
    fn renew_operation(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<OperationRecord, OperationError>;
    /// Read capture scope without hydrating source bytes during batch admission.
    fn git_capture_receipt(
        &self,
        id: &str,
    ) -> Result<entmoot_contracts::GitTreeCapture, OperationError>;
    fn git_capture(
        &self,
        id: &str,
    ) -> Result<crate::collaboration_git::RetainedGitCapture, OperationError>;
    fn retire_workspace(
        &mut self,
        workspace: &str,
        expected: u64,
    ) -> Result<WorkspacePublication, OperationError>;
    fn retained_graph(&self, view: &str) -> Result<crate::TopologyGraph, OperationError>;
    fn retained_snapshot(
        &self,
        view: &str,
    ) -> Result<entmoot_contracts::SourceSnapshot, OperationError>;
    fn publish_workspace(
        &mut self,
        publication: WorkspacePublication,
        expected: u64,
    ) -> Result<WorkspacePublication, OperationError>;
    fn workspaces(&self, repository: &str) -> Result<Vec<WorkspacePublication>, OperationError>;
    fn submit_operations(
        &mut self,
        specs: &[OperationSpec],
        capacity: usize,
    ) -> Result<Vec<OperationRecord>, OperationError>;
    fn operation(&self, id: &str) -> Result<Option<OperationRecord>, OperationError>;
    fn claim_operation(
        &mut self,
        id: &str,
        worker: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<OperationRecord, OperationError>;
    fn checkpoint_operation(
        &mut self,
        id: &str,
        generation: u64,
        sequence: u64,
        now_ms: u64,
        checkpoint: serde_json::Value,
        result: Option<serde_json::Value>,
    ) -> Result<OperationRecord, OperationError>;
    /// Append one output and commit its cursor in the same transaction.
    #[allow(clippy::too_many_arguments)]
    fn checkpoint_operation_output(
        &mut self,
        id: &str,
        generation: u64,
        sequence: u64,
        now_ms: u64,
        checkpoint: serde_json::Value,
        result: Option<serde_json::Value>,
        output: serde_json::Value,
    ) -> Result<OperationRecord, OperationError>;
    fn operation_outputs(
        &self,
        id: &str,
        after: u64,
        limit: usize,
    ) -> Result<OperationOutputPage, OperationError>;
    /// Worker-initiated handoff, unlike an operator pause, requires a live fence.
    fn yield_operation(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        reason: &str,
    ) -> Result<OperationRecord, OperationError>;
    fn control_operation(
        &mut self,
        id: &str,
        control: OperationControl,
    ) -> Result<OperationRecord, OperationError>;
}

impl LocalOperations for SqliteKnowledgeRepository {
    fn scheduler_status(
        &self,
        repository: &str,
        now_ms: u64,
    ) -> Result<entmoot_contracts::SchedulerStatus, OperationError> {
        scheduler::status(&self.connection, repository, now_ms)
    }
    fn configure_scheduler(
        &mut self,
        repository: &str,
        expected: u64,
        policy: entmoot_contracts::SchedulerPolicy,
    ) -> Result<entmoot_contracts::SchedulerConfiguration, OperationError> {
        scheduler::configure(&mut self.connection, repository, expected, policy)
    }
    fn renew_operation(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<OperationRecord, OperationError> {
        if !(1..=3_600_000).contains(&lease_ms) {
            return Err(OperationError::Invalid(
                "lease requires 1..3600000 ms".into(),
            ));
        }
        let until = now_ms
            .checked_add(lease_ms)
            .filter(|t| *t <= i64::MAX as u64)
            .ok_or_else(|| OperationError::Invalid("lease clock overflow".into()))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = read_operation(&tx, id)?
            .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
        if item.state != OperationState::Running
            || item.generation != generation
            || item.lease_until_ms.is_none_or(|t| t <= now_ms)
        {
            return Err(OperationError::Conflict(
                "renewal requires a live worker generation".into(),
            ));
        }
        item.lease_until_ms = Some(item.lease_until_ms.unwrap_or(until).max(until));
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }
    fn git_capture_receipt(
        &self,
        id: &str,
    ) -> Result<entmoot_contracts::GitTreeCapture, OperationError> {
        git_capture::receipt(&self.connection, id)
    }
    fn git_capture(
        &self,
        id: &str,
    ) -> Result<crate::collaboration_git::RetainedGitCapture, OperationError> {
        git_capture::load(&self.connection, id)
    }
    fn retire_workspace(
        &mut self,
        workspace: &str,
        expected: u64,
    ) -> Result<WorkspacePublication, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let body: Option<Vec<u8>> = tx
            .query_row(
                "SELECT body FROM local_workspaces WHERE workspace_id=?1",
                [workspace],
                |r| r.get(0),
            )
            .optional()?;
        let mut row: WorkspacePublication = serde_json::from_slice(
            &body.ok_or_else(|| OperationError::Invalid("unknown workspace".into()))?,
        )?;
        if !row.active && row.generation == next(expected)? {
            return Ok(row);
        }
        if row.generation != expected {
            return Err(OperationError::Conflict(
                "workspace generation changed".into(),
            ));
        }
        row.generation = next(expected)?;
        row.active = false;
        tx.execute(
            "UPDATE local_workspaces SET view_id='',generation=?2,body=?3 WHERE workspace_id=?1",
            params![
                workspace,
                i64::try_from(row.generation)
                    .map_err(|e| OperationError::Invalid(e.to_string()))?,
                encode(&row)?
            ],
        )?;
        tx.commit()?;
        Ok(row)
    }
    fn retained_snapshot(
        &self,
        view: &str,
    ) -> Result<entmoot_contracts::SourceSnapshot, OperationError> {
        if !self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_view_seals WHERE view_id=?1 AND published=1)",
            [view],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(OperationError::Invalid(
                "view has not been published".into(),
            ));
        }
        view_snapshot(&self.connection, view)
    }
    fn retained_graph(&self, view: &str) -> Result<crate::TopologyGraph, OperationError> {
        self.retained_snapshot(view)?;
        // A separate connection prevents changing the caller's selected view.
        let path: String = self.connection.query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )?;
        if path.is_empty() {
            return Err(OperationError::Invalid(
                "retained graph hydration requires a persistent store".into(),
            ));
        }
        let repository = Self::open_view_read_only(Path::new(&path), view)?;
        repository.connection.execute_batch("BEGIN DEFERRED")?;
        repository
            .load_inventory(false)?
            .ok_or_else(|| OperationError::Invalid("retained view unavailable".into()))
    }
    fn publish_workspace(
        &mut self,
        publication: WorkspacePublication,
        expected: u64,
    ) -> Result<WorkspacePublication, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let publication = publish_workspace_in_transaction(&tx, publication, expected)?;
        tx.commit()?;
        Ok(publication)
    }

    fn workspaces(&self, repository: &str) -> Result<Vec<WorkspacePublication>, OperationError> {
        let mut stmt = self.connection.prepare(
            "SELECT body FROM local_workspaces WHERE repository_id=?1 ORDER BY workspace_id",
        )?;
        let rows = stmt.query_map([repository], |r| r.get::<_, Vec<u8>>(0))?;
        rows.map(|r| Ok(serde_json::from_slice(&r?)?)).collect()
    }

    fn submit_operations(
        &mut self,
        specs: &[OperationSpec],
        capacity: usize,
    ) -> Result<Vec<OperationRecord>, OperationError> {
        if specs.is_empty() || specs.len() > 1000 || capacity == 0 || capacity > 100_000 {
            return Err(OperationError::Invalid(
                "batch requires 1..1000 entries and capacity 1..100000".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut count: i64 = tx.query_row("SELECT COUNT(*) FROM local_operations WHERE state NOT IN ('\"completed\"','\"failed\"','\"cancelled\"')", [], |r| r.get(0))?;
        let mut output = Vec::new();
        for spec in specs {
            check_text(&spec.repository_id)?;
            check_text(&spec.idempotency_key)?;
            check_text(&spec.owner)?;
            if spec.implementation != crate::implementation_fingerprint() {
                return Err(OperationError::Conflict(
                    "implementation fingerprint differs".into(),
                ));
            }
            let id = ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
                &serde_json::to_vec(&(
                    "entmoot.operation.v1",
                    &spec.repository_id,
                    &spec.idempotency_key,
                ))?,
            ));
            if let Some(existing) = read_operation(&tx, id.as_str())? {
                if existing.spec != *spec {
                    return Err(OperationError::Conflict(
                        "idempotency key reused with different inputs".into(),
                    ));
                }
                output.push(existing);
                continue;
            }
            if count
                >= i64::try_from(capacity).map_err(|e| OperationError::Invalid(e.to_string()))?
            {
                return Err(OperationError::Capacity);
            }
            for view in &spec.views {
                if !tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_view_seals WHERE view_id=?1 AND published=1)",
                    [view.as_str()],
                    |r| r.get::<_, bool>(0),
                )? {
                    return Err(OperationError::Invalid(
                        "operation inputs must be registered immutable views".into(),
                    ));
                }
                if view_snapshot(&tx, view.as_str())?.repository_id != spec.repository_id {
                    return Err(OperationError::Invalid(
                        "operation view belongs to another repository".into(),
                    ));
                }
            }
            for dependency in &spec.prerequisites {
                let previous = read_operation(&tx, dependency.as_str())?.ok_or_else(|| {
                    OperationError::Invalid(
                        "prerequisite must already exist (or precede this batch member)".into(),
                    )
                })?;
                if previous.spec.repository_id != spec.repository_id {
                    return Err(OperationError::Invalid(
                        "cross-repository prerequisite".into(),
                    ));
                }
            }
            let item = OperationRecord {
                id,
                spec: spec.clone(),
                state: OperationState::Queued,
                generation: 0,
                worker: None,
                lease_until_ms: None,
                checkpoint_sequence: 0,
                checkpoint: serde_json::Value::Null,
                result: None,
                reason: None,
            };
            tx.execute("INSERT INTO local_operations(operation_id,repository_id,idempotency_key,state,body) VALUES(?1,?2,?3,?4,?5)",
                params![item.id.as_str(), spec.repository_id, spec.idempotency_key, serde_json::to_string(&item.state)?, encode(&item)?])?;
            for view in &spec.views {
                tx.execute(
                    "INSERT OR IGNORE INTO local_operation_pins VALUES(?1,?2)",
                    params![item.id.as_str(), view.as_str()],
                )?;
            }
            count += 1;
            output.push(item);
        }
        tx.commit()?;
        Ok(output)
    }

    fn operation(&self, id: &str) -> Result<Option<OperationRecord>, OperationError> {
        read_operation(&self.connection, id)
    }

    fn claim_operation(
        &mut self,
        id: &str,
        worker: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<OperationRecord, OperationError> {
        check_text(worker)?;
        if !(1..=3_600_000).contains(&lease_ms) {
            return Err(OperationError::Invalid(
                "lease requires 1..3600000 ms".into(),
            ));
        }
        let until = now_ms
            .checked_add(lease_ms)
            .filter(|t| *t <= i64::MAX as u64)
            .ok_or_else(|| OperationError::Invalid("clock overflow".into()))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = read_operation(&tx, id)?
            .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
        if item.spec.implementation != crate::implementation_fingerprint() {
            return Err(OperationError::Conflict(
                "implementation changed; start a new operation".into(),
            ));
        }
        if item.state != OperationState::Queued
            && !(item.state == OperationState::Running
                && item.lease_until_ms.is_some_and(|t| t <= now_ms))
        {
            return Err(OperationError::Conflict("operation is not runnable".into()));
        }
        for dependency in &item.spec.prerequisites {
            if !read_operation(&tx, dependency.as_str())?
                .is_some_and(|d| d.state == OperationState::Completed)
            {
                return Err(OperationError::Conflict(
                    "prerequisite has not completed".into(),
                ));
            }
        }
        // Rejected admission commits only expiring waiting interest. No worker,
        // evidence, cursor or generation advances until resources are granted.
        if !scheduler::admit(&tx, &item, now_ms)? {
            tx.commit()?;
            return Err(OperationError::Capacity);
        }
        item.generation = next(item.generation)?;
        item.state = OperationState::Running;
        item.worker = Some(worker.into());
        item.lease_until_ms = Some(until);
        item.reason = None;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    fn checkpoint_operation(
        &mut self,
        id: &str,
        generation: u64,
        sequence: u64,
        now_ms: u64,
        checkpoint: serde_json::Value,
        result: Option<serde_json::Value>,
    ) -> Result<OperationRecord, OperationError> {
        self.commit_checkpoint(
            id, generation, sequence, now_ms, checkpoint, result, None, None, 0,
        )
    }
    fn checkpoint_operation_output(
        &mut self,
        id: &str,
        generation: u64,
        sequence: u64,
        now_ms: u64,
        checkpoint: serde_json::Value,
        result: Option<serde_json::Value>,
        output: serde_json::Value,
    ) -> Result<OperationRecord, OperationError> {
        self.commit_checkpoint(
            id,
            generation,
            sequence,
            now_ms,
            checkpoint,
            result,
            Some(output),
            None,
            0,
        )
    }
    fn operation_outputs(
        &self,
        id: &str,
        after: u64,
        limit: usize,
    ) -> Result<OperationOutputPage, OperationError> {
        if after > i64::MAX as u64 || !(1..=100).contains(&limit) {
            return Err(OperationError::Invalid(
                "output page requires a valid cursor and limit 1..100".into(),
            ));
        }
        // One snapshot for status and output prevents a false complete page during publication.
        let tx = self.connection.unchecked_transaction()?;
        let item = read_operation(&tx, id)?
            .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
        let mut stmt = tx.prepare("SELECT sequence,body FROM local_operation_outputs WHERE operation_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
        let rows = stmt.query_map(
            params![
                id,
                i64::try_from(after).map_err(|e| OperationError::Invalid(e.to_string()))?,
                i64::try_from(limit + 1).map_err(|e| OperationError::Invalid(e.to_string()))?
            ],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )?;
        let mut outputs = Vec::new();
        let mut bytes = 0usize;
        let mut more = false;
        for row in rows {
            let (sequence, body) = row?;
            if outputs.len() >= limit || bytes.saturating_add(body.len()) > MAX_BODY {
                more = true;
                break;
            }
            bytes += body.len();
            outputs.push(OperationOutput {
                sequence: u64::try_from(sequence)
                    .map_err(|e| OperationError::Invalid(e.to_string()))?,
                value: serde_json::from_slice(&body)?,
            });
        }
        let next_sequence = outputs.last().map_or(after, |row| row.sequence);
        Ok(OperationOutputPage {
            id: item.id,
            outputs,
            next_sequence,
            complete: item.state.terminal() && !more,
        })
    }
    fn yield_operation(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        reason: &str,
    ) -> Result<OperationRecord, OperationError> {
        check_text(reason)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = read_operation(&tx, id)?
            .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
        if item.generation != generation
            || item.state != OperationState::Running
            || item.lease_until_ms.is_none_or(|t| now_ms >= t)
        {
            return Err(OperationError::Conflict(
                "handoff requires a live worker generation".into(),
            ));
        }
        item.generation = next(item.generation)?;
        item.state = OperationState::Paused;
        item.worker = None;
        item.lease_until_ms = None;
        item.reason = Some(reason.into());
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    fn control_operation(
        &mut self,
        id: &str,
        control: OperationControl,
    ) -> Result<OperationRecord, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = read_operation(&tx, id)?
            .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
        let desired = match control {
            OperationControl::Pause => OperationState::Paused,
            OperationControl::Resume => OperationState::Queued,
            OperationControl::Cancel => OperationState::Cancelled,
        };
        if item.state == desired {
            return Ok(item);
        }
        if item.state.terminal()
            || (control == OperationControl::Resume && item.state != OperationState::Paused)
        {
            return Err(OperationError::Conflict(
                "invalid operation control transition".into(),
            ));
        }
        item.generation = next(item.generation)?;
        item.state = desired;
        item.lease_until_ms = None;
        item.worker = None;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }
}

impl SqliteKnowledgeRepository {
    #[allow(clippy::too_many_arguments)]
    fn commit_checkpoint(
        &mut self,
        id: &str,
        generation: u64,
        sequence: u64,
        now_ms: u64,
        checkpoint: serde_json::Value,
        result: Option<serde_json::Value>,
        output: Option<serde_json::Value>,
        pack: Option<&[u8]>,
        max_staged_bytes: u64,
    ) -> Result<OperationRecord, OperationError> {
        let pack_digest = pack.map(entmoot_contracts::blake3_hex);
        let sql_sequence =
            i64::try_from(sequence).map_err(|e| OperationError::Invalid(e.to_string()))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = read_operation(&tx, id)?
            .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
        if item.generation != generation {
            return Err(OperationError::Conflict("stale worker generation".into()));
        }
        if matches!(
            item.spec.kind,
            entmoot_contracts::OperationKind::Inventory
                | entmoot_contracts::OperationKind::Canon
                | entmoot_contracts::OperationKind::Binding
                | entmoot_contracts::OperationKind::Revalidation
        ) {
            return Err(OperationError::Invalid(
                "this operation requires its atomic evidence checkpoint owner".into(),
            ));
        }
        let existing_output: Option<Vec<u8>> = tx
            .query_row(
                "SELECT body FROM local_operation_outputs WHERE operation_id=?1 AND sequence=?2",
                params![id, sql_sequence],
                |r| r.get(0),
            )
            .optional()?;
        let output_bytes = output.as_ref().map(encode).transpose()?;
        let existing_pack: Option<String> =
            if item.spec.kind == entmoot_contracts::OperationKind::Integration {
                tx.query_row(
                "SELECT digest FROM local_integration_packs WHERE operation_id=?1 AND sequence=?2",
                params![id, sql_sequence],
                |r| r.get(0),
            )
            .optional()?
            } else {
                None
            };
        // Exact retry is safe after a lost acknowledgement, including completion.
        if item.checkpoint_sequence == sequence
            && item.checkpoint == checkpoint
            && item.result == result
            && sequence > 0
            && existing_output == output_bytes
            && existing_pack == pack_digest
        {
            return Ok(item);
        }
        if item.state != OperationState::Running
            || item.lease_until_ms.is_none_or(|t| now_ms >= t)
            || sequence != next(item.checkpoint_sequence)?
        {
            return Err(OperationError::Conflict(
                "checkpoint requires a live lease and next sequence".into(),
            ));
        }
        if let Some(bytes) = pack {
            if item.spec.kind != entmoot_contracts::OperationKind::Integration || result.is_some() {
                return Err(OperationError::Invalid(
                    "private Git packs require a nonterminal integration checkpoint".into(),
                ));
            }
            let staged: i64 = tx.query_row(
                "SELECT COALESCE(SUM(length(body)),0) FROM local_integration_packs WHERE operation_id=?1",
                [id],
                |row| row.get(0),
            )?;
            let staged =
                u64::try_from(staged).map_err(|e| OperationError::Invalid(e.to_string()))?;
            if bytes.len() as u64 > max_staged_bytes.saturating_sub(staged)
                || max_staged_bytes > 1_073_741_824
            {
                return Err(OperationError::Budget(
                    "integration staging byte allowance exhausted".into(),
                ));
            }
            tx.execute(
                "INSERT INTO local_integration_packs VALUES(?1,?2,?3,?4)",
                params![id, sql_sequence, pack_digest, bytes],
            )?;
        }
        item.checkpoint_sequence = sequence;
        item.checkpoint = checkpoint;
        item.result = result;
        if item.result.is_some() {
            item.state = OperationState::Completed;
            item.lease_until_ms = None;
        }
        if let Some(body) = output_bytes {
            tx.execute(
                "INSERT INTO local_operation_outputs VALUES(?1,?2,?3)",
                params![id, sql_sequence, body],
            )?;
        }
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }
}

fn publish_workspace_in_transaction(
    tx: &Transaction<'_>,
    mut publication: WorkspacePublication,
    expected: u64,
) -> Result<WorkspacePublication, OperationError> {
    check_text(&publication.workspace_id)?;
    if !publication.active {
        return Err(OperationError::Invalid(
            "use retire to deactivate a workspace".into(),
        ));
    }
    check_text(&publication.publisher)?;
    if let Some(branch) = &publication.branch {
        check_text(branch)?;
    }
    publication
        .snapshot
        .validate()
        .map_err(|e| OperationError::Invalid(e.to_string()))?;
    ContentDigest::parse(publication.view.as_str())
        .map_err(|e| OperationError::Invalid(e.to_string()))?;
    if view_snapshot(tx, publication.view.as_str())? != publication.snapshot {
        return Err(OperationError::Invalid(
            "workspace snapshot does not match retained view".into(),
        ));
    }
    git_capture::validate_publication(tx, &publication)?;
    let previous: Option<Vec<u8>> = tx
        .query_row(
            "SELECT body FROM local_workspaces WHERE workspace_id=?1",
            [&publication.workspace_id],
            |r| r.get(0),
        )
        .optional()?;
    let previous = previous
        .map(|v| serde_json::from_slice::<WorkspacePublication>(&v))
        .transpose()?;
    if previous
        .as_ref()
        .is_some_and(|p| p.snapshot.repository_id != publication.snapshot.repository_id)
    {
        return Err(OperationError::Conflict(
            "workspace belongs to another repository".into(),
        ));
    }
    publication.generation = next(expected)?;
    if previous.as_ref() == Some(&publication) {
        return Ok(publication);
    }
    if previous.as_ref().map_or(0, |p| p.generation) != expected {
        return Err(OperationError::Conflict(
            "workspace generation changed".into(),
        ));
    }
    tx.execute(
        "INSERT INTO local_view_seals VALUES(?1,1) ON CONFLICT(view_id) DO UPDATE SET published=1",
        [publication.view.as_str()],
    )?;
    tx.execute("INSERT INTO local_workspaces VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id) DO UPDATE SET view_id=excluded.view_id,generation=excluded.generation,body=excluded.body",
            params![publication.workspace_id, publication.snapshot.repository_id, publication.view.as_str(), i64::try_from(publication.generation).map_err(|e| OperationError::Invalid(e.to_string()))?, encode(&publication)?])?;
    Ok(publication)
}

fn live_producer(
    tx: &Transaction<'_>,
    id: &str,
    generation: u64,
    now_ms: u64,
) -> Result<OperationRecord, OperationError> {
    let item = read_operation(tx, id)?
        .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
    if item.generation != generation
        || item.state != OperationState::Running
        || item.lease_until_ms.is_none_or(|t| now_ms >= t)
    {
        return Err(OperationError::Conflict(
            "producer requires live worker generation".into(),
        ));
    }
    if !matches!(
        item.spec.kind,
        entmoot_contracts::OperationKind::Inventory
            | entmoot_contracts::OperationKind::Canon
            | entmoot_contracts::OperationKind::Binding
    ) {
        return Err(OperationError::Invalid(
            "operation is not a supported producer".into(),
        ));
    }
    Ok(item)
}

fn stage_graph(
    tx: &Transaction<'_>,
    id: &str,
    graph: &crate::TopologyGraph,
    source: Option<&dyn crate::SourceStore>,
) -> Result<String, OperationError> {
    inventory::import_graph_in_transaction(tx, graph, source)?;
    let view: String = tx.query_row("SELECT id FROM temp.selected_knowledge_view", [], |r| {
        r.get(0)
    })?;
    tx.execute(
        "INSERT OR IGNORE INTO local_view_seals VALUES(?1,0)",
        [&view],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO local_operation_pins VALUES(?1,?2)",
        params![id, view],
    )?;
    Ok(view)
}
