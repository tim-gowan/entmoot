//! Fenced, durable handoff to external analysis producers.
use crate::{plan_analysis, ManifestVerified};
use entmoot_contracts::{
    AnalysisAction, AnalysisDecision, AnalysisWorkItem, AnalysisWorkState, ContentDigest,
    ProducerRef,
};
use rusqlite::{params, Connection, OptionalExtension};
#[cfg(test)]
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum AnalysisWorkError {
    #[error("invalid analysis work: {0}")]
    Invalid(String),
    #[error("analysis work transition conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl AnalysisWorkError {
    /// Stable operation category shared by frontends.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::Conflict(_) => "conflict",
            Self::Sqlite(_) | Self::Json(_) => "storage_error",
        }
    }
}

/// Replaceable local/external queue boundary. Implementations persist the full
/// decision receipt; they do not execute compilers or publish knowledge.
pub trait AnalysisWorkQueue {
    fn enqueue(
        &mut self,
        decision: AnalysisDecision,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError>;
    fn claim(
        &mut self,
        work_id: &str,
        worker: ProducerRef,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError>;
    fn artifact_ready(
        &mut self,
        work_id: &str,
        fencing_token: &str,
        artifact: &ManifestVerified,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError>;
    fn ingested(
        &mut self,
        work_id: &str,
        fencing_token: &str,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError>;
    fn fail(
        &mut self,
        work_id: &str,
        fencing_token: &str,
        reason: String,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError>;
    fn cancel(&mut self, work_id: &str) -> Result<AnalysisWorkItem, AnalysisWorkError>;
    fn get(&self, work_id: &str) -> Result<Option<AnalysisWorkItem>, AnalysisWorkError>;
}

fn work_id(decision: &AnalysisDecision) -> Result<ContentDigest, AnalysisWorkError> {
    Ok(ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
        &serde_json::to_vec(decision)?,
    )))
}

fn new_item(decision: AnalysisDecision) -> Result<AnalysisWorkItem, AnalysisWorkError> {
    if decision.action != AnalysisAction::RequestArtifact
        || plan_analysis(decision.demand.clone(), decision.mode)
            .map_err(|error| AnalysisWorkError::Invalid(error.to_string()))?
            != decision
    {
        return Err(AnalysisWorkError::Invalid(
            "queue accepts only an untampered request_artifact decision".into(),
        ));
    }
    Ok(AnalysisWorkItem {
        work_id: work_id(&decision)?,
        limitations: decision.limitations.clone(),
        decision,
        state: AnalysisWorkState::Queued,
        attempt: 0,
        worker: None,
        fencing_token: None,
        artifact_id: None,
        failure_reason: None,
    })
}

fn claim_item(item: &mut AnalysisWorkItem, worker: ProducerRef) -> Result<(), AnalysisWorkError> {
    worker
        .validate()
        .map_err(|error| AnalysisWorkError::Invalid(error.to_string()))?;
    if !matches!(
        item.state,
        AnalysisWorkState::Queued | AnalysisWorkState::Failed
    ) {
        return Err(AnalysisWorkError::Conflict(format!(
            "cannot claim {:?} work",
            item.state
        )));
    }
    item.attempt = item
        .attempt
        .checked_add(1)
        .ok_or_else(|| AnalysisWorkError::Invalid("attempt counter overflow".into()))?;
    item.fencing_token = Some(ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
        &serde_json::to_vec(&(item.work_id.as_str(), item.attempt, &worker))?,
    )));
    item.worker = Some(worker);
    item.artifact_id = None;
    item.failure_reason = None;
    item.state = AnalysisWorkState::Claimed;
    Ok(())
}

fn check_fence(item: &AnalysisWorkItem, fencing_token: &str) -> Result<(), AnalysisWorkError> {
    if item.fencing_token.as_ref().map(ContentDigest::as_str) != Some(fencing_token) {
        return Err(AnalysisWorkError::Conflict("stale fencing token".into()));
    }
    Ok(())
}

fn ready_item(
    item: &mut AnalysisWorkItem,
    fencing_token: &str,
    artifact: &ManifestVerified,
) -> Result<(), AnalysisWorkError> {
    check_fence(item, fencing_token)?;
    if item.state != AnalysisWorkState::Claimed {
        return Err(AnalysisWorkError::Conflict(
            "artifact requires claimed work".into(),
        ));
    }
    if artifact.manifest().snapshot != item.decision.demand.snapshot {
        return Err(AnalysisWorkError::Invalid(
            "verified artifact snapshot does not match the work decision".into(),
        ));
    }
    item.artifact_id = Some(
        entmoot_contracts::ArtifactId::parse(artifact.manifest().artifact_id.clone())
            .map_err(|error| AnalysisWorkError::Invalid(error.to_string()))?,
    );
    item.state = AnalysisWorkState::ArtifactReady;
    Ok(())
}

fn ingested_item(
    item: &mut AnalysisWorkItem,
    fencing_token: &str,
) -> Result<(), AnalysisWorkError> {
    check_fence(item, fencing_token)?;
    if item.state != AnalysisWorkState::ArtifactReady || item.artifact_id.is_none() {
        return Err(AnalysisWorkError::Conflict(
            "ingestion acknowledgement requires a verified ready artifact".into(),
        ));
    }
    item.state = AnalysisWorkState::Ingested;
    Ok(())
}

fn fail_item(
    item: &mut AnalysisWorkItem,
    fencing_token: &str,
    reason: String,
) -> Result<(), AnalysisWorkError> {
    check_fence(item, fencing_token)?;
    if item.state != AnalysisWorkState::Claimed || reason.trim().is_empty() {
        return Err(AnalysisWorkError::Conflict(
            "only claimed work can fail with a nonempty reason".into(),
        ));
    }
    item.failure_reason = Some(reason);
    item.state = AnalysisWorkState::Failed;
    Ok(())
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemoryAnalysisWorkQueue {
    items: BTreeMap<String, AnalysisWorkItem>,
}

#[cfg(test)]
impl AnalysisWorkQueue for MemoryAnalysisWorkQueue {
    fn enqueue(
        &mut self,
        decision: AnalysisDecision,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = new_item(decision)?;
        if let Some(existing) = self.items.get(item.work_id.as_str()) {
            return Ok(existing.clone());
        }
        self.items
            .insert(item.work_id.as_str().to_owned(), item.clone());
        Ok(item)
    }

    fn claim(
        &mut self,
        work_id: &str,
        worker: ProducerRef,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = self
            .items
            .get_mut(work_id)
            .ok_or_else(|| AnalysisWorkError::Invalid("unknown work id".into()))?;
        claim_item(item, worker)?;
        Ok(item.clone())
    }

    fn artifact_ready(
        &mut self,
        work_id: &str,
        fencing_token: &str,
        artifact: &ManifestVerified,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = self
            .items
            .get_mut(work_id)
            .ok_or_else(|| AnalysisWorkError::Invalid("unknown work id".into()))?;
        ready_item(item, fencing_token, artifact)?;
        Ok(item.clone())
    }

    fn ingested(
        &mut self,
        work_id: &str,
        fencing_token: &str,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = self
            .items
            .get_mut(work_id)
            .ok_or_else(|| AnalysisWorkError::Invalid("unknown work id".into()))?;
        ingested_item(item, fencing_token)?;
        Ok(item.clone())
    }

    fn fail(
        &mut self,
        work_id: &str,
        fencing_token: &str,
        reason: String,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = self
            .items
            .get_mut(work_id)
            .ok_or_else(|| AnalysisWorkError::Invalid("unknown work id".into()))?;
        fail_item(item, fencing_token, reason)?;
        Ok(item.clone())
    }

    fn cancel(&mut self, work_id: &str) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = self
            .items
            .get_mut(work_id)
            .ok_or_else(|| AnalysisWorkError::Invalid("unknown work id".into()))?;
        if item.state != AnalysisWorkState::Queued {
            return Err(AnalysisWorkError::Conflict(
                "only unclaimed work can be cancelled".into(),
            ));
        }
        item.state = AnalysisWorkState::Cancelled;
        Ok(item.clone())
    }

    fn get(&self, work_id: &str) -> Result<Option<AnalysisWorkItem>, AnalysisWorkError> {
        Ok(self.items.get(work_id).cloned())
    }
}

pub struct SqliteAnalysisWorkQueue {
    connection: Connection,
}

impl SqliteAnalysisWorkQueue {
    pub fn open(path: &Path) -> Result<Self, AnalysisWorkError> {
        Self::from_connection(Connection::open(crate::sqlite_startup::file_path(path))?)
    }

    /// Read existing work without initializing or modifying a queue.
    pub fn open_read_only(path: &Path) -> Result<Self, AnalysisWorkError> {
        let connection = Connection::open_with_flags(
            crate::sqlite_startup::file_path(path),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.prepare("SELECT work_id FROM analysis_work_items LIMIT 0")?;
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> Result<Self, AnalysisWorkError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, AnalysisWorkError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        crate::sqlite_startup::enable_wal(&connection)?;
        connection.execute_batch(
            "PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS analysis_work_items (
               work_id TEXT PRIMARY KEY, state TEXT NOT NULL, payload BLOB NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS analysis_work_by_state
               ON analysis_work_items(state, work_id);",
        )?;
        Ok(Self { connection })
    }

    fn update(
        &mut self,
        work_id: &str,
        change: impl FnOnce(&mut AnalysisWorkItem) -> Result<(), AnalysisWorkError>,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let payload = transaction
            .query_row(
                "SELECT payload FROM analysis_work_items WHERE work_id=?1",
                [work_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .ok_or_else(|| AnalysisWorkError::Invalid("unknown work id".into()))?;
        let mut item: AnalysisWorkItem = serde_json::from_slice(&payload)?;
        change(&mut item)?;
        transaction.execute(
            "UPDATE analysis_work_items SET state=?2,payload=?3 WHERE work_id=?1",
            params![
                work_id,
                format!("{:?}", item.state),
                serde_json::to_vec(&item)?
            ],
        )?;
        transaction.commit()?;
        Ok(item)
    }
}

impl AnalysisWorkQueue for SqliteAnalysisWorkQueue {
    fn enqueue(
        &mut self,
        decision: AnalysisDecision,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        let item = new_item(decision)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT payload FROM analysis_work_items WHERE work_id=?1",
                [item.work_id.as_str()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|bytes| serde_json::from_slice::<AnalysisWorkItem>(&bytes))
            .transpose()?;
        if let Some(existing) = existing {
            if existing.decision != item.decision {
                return Err(AnalysisWorkError::Conflict("work id collision".into()));
            }
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO analysis_work_items(work_id,state,payload) VALUES(?1,?2,?3)",
            params![item.work_id.as_str(), "Queued", serde_json::to_vec(&item)?],
        )?;
        transaction.commit()?;
        Ok(item)
    }

    fn claim(
        &mut self,
        work_id: &str,
        worker: ProducerRef,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        self.update(work_id, |item| claim_item(item, worker))
    }

    fn artifact_ready(
        &mut self,
        work_id: &str,
        fencing_token: &str,
        artifact: &ManifestVerified,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        self.update(work_id, |item| ready_item(item, fencing_token, artifact))
    }

    fn ingested(
        &mut self,
        work_id: &str,
        fencing_token: &str,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        self.update(work_id, |item| ingested_item(item, fencing_token))
    }

    fn fail(
        &mut self,
        work_id: &str,
        fencing_token: &str,
        reason: String,
    ) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        self.update(work_id, |item| fail_item(item, fencing_token, reason))
    }

    fn cancel(&mut self, work_id: &str) -> Result<AnalysisWorkItem, AnalysisWorkError> {
        self.update(work_id, |item| {
            if item.state != AnalysisWorkState::Queued {
                return Err(AnalysisWorkError::Conflict(
                    "only unclaimed work can be cancelled".into(),
                ));
            }
            item.state = AnalysisWorkState::Cancelled;
            Ok(())
        })
    }

    fn get(&self, work_id: &str) -> Result<Option<AnalysisWorkItem>, AnalysisWorkError> {
        self.connection
            .query_row(
                "SELECT payload FROM analysis_work_items WHERE work_id=?1",
                [work_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|payload| serde_json::from_slice(&payload))
            .transpose()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManifestUnverified;
    use entmoot_contracts::{
        AnalysisPassMode, ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness,
        ScopeUniverse,
    };

    #[test]
    fn read_only_queue_never_creates_or_changes_storage() {
        let directory = tempfile::tempdir().unwrap();
        let mut parent = directory.path().to_path_buf();
        while parent.as_os_str().len() < 280 {
            parent.push("nested-queue-storage");
        }
        let path = parent.join("queue.sqlite3");
        assert!(SqliteAnalysisWorkQueue::open_read_only(&path).is_err());
        assert!(
            !parent.exists(),
            "read-only open created missing directories"
        );
        std::fs::create_dir_all(&parent).unwrap();
        let item = {
            let mut queue = SqliteAnalysisWorkQueue::open(&path).unwrap();
            queue.enqueue(decision()).unwrap()
        };
        let before = std::fs::read(&path).unwrap();
        {
            let queue = SqliteAnalysisWorkQueue::open_read_only(&path).unwrap();
            assert_eq!(queue.get(item.work_id.as_str()).unwrap(), Some(item));
            assert_eq!(queue.get("absent").unwrap(), None);
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    fn decision() -> AnalysisDecision {
        let demand = serde_json::from_value(serde_json::json!({
          "policy_producer":{"id":"fixture.plan","name":"Fixture plan","capabilities":["deriver"]},
          "snapshot":{"repository_id":"fixture","revision":"a".repeat(40),"source_state":"committed"},
          "subject":{"scheme":"opaque","system":"fixture","value":"task"},
          "method_id":"fixture.analysis.v1","input_root":entmoot_contracts::blake3_hex(b"inputs"),
          "prior_input_root":null,"current_artifact":null,"relevant_unknown":true,
          "assumptions":["fixture inputs are complete"],
          "evidence_refs":[{"artifact_id":"b".repeat(64),"provider_pointer":"/demand"}],
          "gates":[
            {"kind":"qualification","disposition":"accepted","claimant":{"id":"fixture.q","name":"Q","capabilities":["deriver"]},"method_id":"q.v1","assumptions":["fixture"],"limitations":["fixture only"],"evidence_refs":[{"artifact_id":"c".repeat(64),"provider_pointer":"/q"}]},
            {"kind":"environment","disposition":"accepted","claimant":{"id":"fixture.e","name":"E","capabilities":["deriver"]},"method_id":"e.v1","assumptions":["fixture"],"limitations":["fixture only"],"evidence_refs":[{"artifact_id":"d".repeat(64),"provider_pointer":"/e"}]},
            {"kind":"marginal_value","disposition":"accepted","claimant":{"id":"fixture.v","name":"V","capabilities":["deriver"]},"method_id":"v.v1","assumptions":["fixture"],"limitations":["fixture only"],"evidence_refs":[{"artifact_id":"e".repeat(64),"provider_pointer":"/v"}]}
          ]
        }))
        .unwrap();
        plan_analysis(demand, AnalysisPassMode::Auto).unwrap()
    }

    fn open_with_lock_retry(
        path: &std::path::Path,
    ) -> Result<SqliteAnalysisWorkQueue, AnalysisWorkError> {
        let mut last = None;
        for _ in 0..50 {
            match SqliteAnalysisWorkQueue::open(path) {
                Ok(queue) => return Ok(queue),
                Err(error) => {
                    let transient = matches!(
                        &error,
                        AnalysisWorkError::Sqlite(rusqlite::Error::SqliteFailure(code, _))
                            if code.code == rusqlite::ErrorCode::FileLockingProtocolFailed
                                || code.code == rusqlite::ErrorCode::DatabaseBusy
                                || code.code == rusqlite::ErrorCode::DatabaseLocked
                    );
                    last = Some(error);
                    if !transient {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        Err(last.expect("open retried without an error"))
    }

    #[test]
    fn concurrent_first_open_of_the_legacy_queue_is_safe() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new-queue.sqlite3");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    // Windows can refuse the first concurrent creates with
                    // FileLockingProtocolFailed even after busy_timeout.
                    open_with_lock_retry(&path).map(|_| ())
                })
            })
            .collect();
        let rows: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(rows.iter().all(Result::is_ok), "{rows:?}");
    }

    #[test]
    fn simultaneous_duplicate_submissions_all_return_the_same_work() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue.sqlite3");
        drop(SqliteAnalysisWorkQueue::open(&path).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(12));
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    open_with_lock_retry(&path).and_then(|mut queue| queue.enqueue(decision()))
                })
            })
            .collect();
        let rows: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(rows.iter().all(Result::is_ok), "{rows:?}");
        assert!(rows
            .windows(2)
            .all(|r| r[0].as_ref().unwrap() == r[1].as_ref().unwrap()));
    }

    fn worker() -> ProducerRef {
        ProducerRef {
            id: "fixture.worker".into(),
            name: "Fixture worker".into(),
            version: Some("1".into()),
            config_fingerprint: None,
            environment_fingerprint: None,
            capabilities: vec![entmoot_contracts::ProducerCapabilityKind::Resolver],
        }
    }

    fn verified(decision: &AnalysisDecision) -> ManifestVerified {
        let bytes = b"resolver artifact";
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "fixture".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            decision.demand.snapshot.clone(),
            ScopeUniverse::from_selectors(
                "fixture",
                vec![],
                vec![],
                vec![],
                ScopeCompleteness::Partial,
                None,
            ),
            entmoot_contracts::blake3_hex(bytes),
            None,
        )
        .unwrap();
        ManifestUnverified::new(manifest)
            .verify(bytes, &decision.demand.snapshot)
            .unwrap()
    }

    fn exercises_fencing(queue: &mut dyn AnalysisWorkQueue) {
        let decision = decision();
        let queued = queue.enqueue(decision.clone()).unwrap();
        assert_eq!(queue.enqueue(decision.clone()).unwrap(), queued);
        let first = queue.claim(queued.work_id.as_str(), worker()).unwrap();
        let old_fence = first.fencing_token.as_ref().unwrap().clone();
        queue
            .fail(
                queued.work_id.as_str(),
                old_fence.as_str(),
                "worker stopped".into(),
            )
            .unwrap();
        let second = queue.claim(queued.work_id.as_str(), worker()).unwrap();
        let fence = second.fencing_token.as_ref().unwrap();
        assert_ne!(fence, &old_fence);
        assert!(queue
            .artifact_ready(
                queued.work_id.as_str(),
                old_fence.as_str(),
                &verified(&decision)
            )
            .is_err());
        let ready = queue
            .artifact_ready(
                queued.work_id.as_str(),
                fence.as_str(),
                &verified(&decision),
            )
            .unwrap();
        assert_eq!(ready.state, AnalysisWorkState::ArtifactReady);
        assert_eq!(
            queue
                .ingested(queued.work_id.as_str(), fence.as_str())
                .unwrap()
                .state,
            AnalysisWorkState::Ingested
        );
    }

    #[test]
    fn memory_and_sqlite_queues_enforce_the_same_fenced_transitions() {
        exercises_fencing(&mut MemoryAnalysisWorkQueue::default());
        exercises_fencing(&mut SqliteAnalysisWorkQueue::open_in_memory().unwrap());
    }

    #[test]
    fn sqlite_queue_survives_restart_without_losing_claim() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("work.sqlite3");
        let mut queue = SqliteAnalysisWorkQueue::open(&path).unwrap();
        let item = queue.enqueue(decision()).unwrap();
        let claimed = queue.claim(item.work_id.as_str(), worker()).unwrap();
        drop(queue);
        let queue = SqliteAnalysisWorkQueue::open(&path).unwrap();
        assert_eq!(queue.get(item.work_id.as_str()).unwrap(), Some(claimed));
    }
}
