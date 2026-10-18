//! Private file checkpoints. All mutation is fenced by the owning operation.
use super::*;
use crate::checkpoints::canon::{CanonCursor, CanonFileOutcome};
use entmoot_contracts::{OperationKind, ProducerRequest, ProducerTask};
use entmoot_core::scan::{ScanExclusions, ScanInput, ScanManifest, ScanOutcome};

#[derive(serde::Serialize, serde::Deserialize)]
enum FileProgress {
    Inventory(ScanExclusions),
    Canon(CanonCursor),
}
impl FileProgress {
    fn kind(&self) -> OperationKind {
        match self {
            Self::Inventory(_) => OperationKind::Inventory,
            Self::Canon(_) => OperationKind::Canon,
        }
    }
}
trait FileOutcome: serde::Serialize + serde::de::DeserializeOwned {
    const KIND: OperationKind;
    fn input(&self) -> &ScanInput;
    fn valid(&self, request: &ProducerRequest) -> bool;
}
impl FileOutcome for ScanOutcome {
    const KIND: OperationKind = OperationKind::Inventory;
    fn input(&self) -> &ScanInput {
        &self.input
    }
    fn valid(&self, request: &ProducerRequest) -> bool {
        let ProducerTask::Inventory { policy } = &request.task else {
            return false;
        };
        self.record.is_some() != self.skipped.is_some()
            && self.record.as_ref().is_none_or(|r| {
                r.policy_id == policy.fingerprint() && r.resolved_identities.is_empty()
            })
    }
}
impl FileOutcome for CanonFileOutcome {
    const KIND: OperationKind = OperationKind::Canon;
    fn input(&self) -> &ScanInput {
        &self.input
    }
    fn valid(&self, request: &ProducerRequest) -> bool {
        self.valid_for(request)
    }
}

pub(super) const SCHEMA: &str = r#"
CREATE TABLE local_file_progress (
    operation_id TEXT PRIMARY KEY REFERENCES local_operations(operation_id),
    payload INTEGER NOT NULL REFERENCES shared_payloads,
    completed INTEGER NOT NULL CHECK(completed>=0), total INTEGER NOT NULL CHECK(total>=completed),
    syntax_completed INTEGER NOT NULL DEFAULT 0 CHECK(syntax_completed>=0 AND syntax_completed<=completed),
    calls_completed INTEGER NOT NULL DEFAULT 0 CHECK(calls_completed>=0 AND calls_completed<=syntax_completed)
) WITHOUT ROWID;
CREATE TABLE local_file_outcomes (
    operation_id TEXT NOT NULL REFERENCES local_file_progress(operation_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, path TEXT NOT NULL, digest TEXT NOT NULL,
    payload INTEGER REFERENCES shared_payloads,
    PRIMARY KEY(operation_id,ordinal), UNIQUE(operation_id,path)
) WITHOUT ROWID;
"#;

fn invalid(error: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(error.to_string())
}

pub(super) fn require_complete_if_present(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<(), OperationError> {
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM local_file_progress WHERE operation_id=?1 AND completed<>total UNION ALL SELECT 1 FROM local_file_outcomes WHERE operation_id=?1 AND payload IS NULL)", [id], |r|r.get::<_,bool>(0))? {
        return Err(OperationError::Conflict("producer has incomplete file checkpoints".into()));
    }
    Ok(())
}

fn progress(
    item: &mut OperationRecord,
    completed: usize,
    total: usize,
) -> Result<(), OperationError> {
    item.checkpoint_sequence = next(item.checkpoint_sequence)?;
    let name = match item.spec.kind {
        OperationKind::Inventory => "inventory",
        OperationKind::Canon => "canon",
        _ => return Err(invalid("unsupported file producer")),
    };
    item.checkpoint = serde_json::json!({(name):{"completed_files":completed,"total_files":total}});
    Ok(())
}

impl SqliteKnowledgeRepository {
    pub fn begin_inventory(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        manifest: &ScanManifest,
    ) -> Result<OperationRecord, OperationError> {
        self.begin_files(
            id,
            generation,
            now_ms,
            &manifest.inputs,
            FileProgress::Inventory(manifest.exclusions.clone()),
        )
    }

    pub fn begin_canon(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        inputs: &[ScanInput],
        cursor: CanonCursor,
    ) -> Result<OperationRecord, OperationError> {
        if cursor.completed != 0 {
            return Err(invalid("initial canon cursor is not empty"));
        }
        self.begin_files(id, generation, now_ms, inputs, FileProgress::Canon(cursor))
    }

    fn begin_files(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        inputs: &[ScanInput],
        metadata: FileProgress,
    ) -> Result<OperationRecord, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = live_producer(&tx, id, generation, now_ms)?;
        if item.spec.kind != metadata.kind() || !item.checkpoint.is_null() {
            return Err(invalid(
                "file checkpoint already exists or producer differs",
            ));
        }
        let request: ProducerRequest = serde_json::from_value(item.spec.input.clone())?;
        if let FileProgress::Inventory(_) = metadata {
            let files: Vec<_> = inputs
                .iter()
                .map(|f| crate::SnapshotFileDigest {
                    path: f.path.clone(),
                    content_digest: f.content_digest.clone(),
                })
                .collect();
            if entmoot_contracts::blake3_hex(&serde_json::to_vec(&files)?)
                != request.source.files_digest.as_str()
            {
                return Err(OperationError::Conflict(
                    "inventory manifest differs from frozen request".into(),
                ));
            }
        }
        let payload = intern_payload(&tx, &serde_json::to_vec(&metadata)?)?;
        tx.execute(
            "INSERT INTO local_file_progress(operation_id,payload,completed,total) VALUES(?1,?2,0,?3)",
            params![id, payload, i64::try_from(inputs.len()).map_err(invalid)?],
        )?;
        for (ordinal, input) in inputs.iter().enumerate() {
            tx.execute(
                "INSERT INTO local_file_outcomes VALUES(?1,?2,?3,?4,NULL)",
                params![
                    id,
                    i64::try_from(ordinal).map_err(invalid)?,
                    input.path,
                    input.content_digest
                ],
            )?;
        }
        progress(&mut item, 0, inputs.len())?;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    pub fn producer_file_batch(
        &self,
        id: &str,
        limit: usize,
    ) -> Result<Vec<ScanInput>, OperationError> {
        if !(1..=4096).contains(&limit) {
            return Err(invalid("file batch requires 1..4096 files"));
        }
        let mut statement = self.connection.prepare("SELECT path,digest FROM local_file_outcomes WHERE operation_id=?1 AND ordinal>=(SELECT completed FROM local_file_progress WHERE operation_id=?1) ORDER BY ordinal LIMIT ?2")?;
        let rows =
            statement.query_map(params![id, i64::try_from(limit).map_err(invalid)?], |r| {
                Ok(ScanInput {
                    path: r.get(0)?,
                    content_digest: r.get(1)?,
                })
            })?;
        rows.map(|r| r.map_err(OperationError::from)).collect()
    }

    pub fn checkpoint_inventory_batch(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        outcomes: &[ScanOutcome],
        reused: usize,
    ) -> Result<OperationRecord, OperationError> {
        self.checkpoint_files(id, generation, now_ms, outcomes, None, reused)
    }

    pub fn canon_cursor(&self, id: &str) -> Result<CanonCursor, OperationError> {
        let payload: i64 = self.connection.query_row(
            "SELECT payload FROM local_file_progress WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )?;
        match hydrate_record(&self.connection, payload)? {
            FileProgress::Canon(cursor) => Ok(cursor),
            _ => Err(invalid("checkpoint is not canon")),
        }
    }

    pub fn checkpoint_canon_batch(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        outcomes: &[CanonFileOutcome],
        cursor: CanonCursor,
    ) -> Result<OperationRecord, OperationError> {
        self.checkpoint_files(
            id,
            generation,
            now_ms,
            outcomes,
            Some(FileProgress::Canon(cursor)),
            0,
        )
    }

    fn checkpoint_files<T: FileOutcome>(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        outcomes: &[T],
        updated: Option<FileProgress>,
        reused: usize,
    ) -> Result<OperationRecord, OperationError> {
        if outcomes.is_empty() || outcomes.len() > 4096 || reused > outcomes.len() {
            return Err(invalid("file checkpoint requires 1..4096 files"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = live_producer(&tx, id, generation, now_ms)?;
        let (completed, total, prior): (i64, i64, i64) = tx.query_row(
            "SELECT completed,total,payload FROM local_file_progress WHERE operation_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let completed = usize::try_from(completed).map_err(invalid)?;
        let total = usize::try_from(total).map_err(invalid)?;
        let request: entmoot_contracts::ProducerRequest =
            serde_json::from_value(item.spec.input.clone())?;
        let metadata: FileProgress = hydrate_record(&tx, prior)?;
        if item.spec.kind != T::KIND || metadata.kind() != T::KIND {
            return Err(invalid("file producer differs"));
        }
        match (&metadata, &updated) {
            (FileProgress::Inventory(_), None) => {}
            (FileProgress::Canon(prior), Some(FileProgress::Canon(next)))
                if prior.completed == completed
                    && next.completed == completed + outcomes.len()
                    && next.follows(prior) => {}
            _ => return Err(invalid("file cursor transition differs")),
        }
        if outcomes.len()
            > total
                .checked_sub(completed)
                .ok_or_else(|| invalid("file counters are inconsistent"))?
        {
            return Err(invalid("file checkpoint exceeds remaining manifest"));
        }
        for (offset, outcome) in outcomes.iter().enumerate() {
            let ordinal = completed + offset;
            let (path, digest): (String, String) = tx.query_row(
                "SELECT path,digest FROM local_file_outcomes WHERE operation_id=?1 AND ordinal=?2",
                params![id, i64::try_from(ordinal).map_err(invalid)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if outcome.input().path != path
                || outcome.input().content_digest != digest
                || !outcome.valid(&request)
            {
                return Err(invalid(
                    "file outcome does not match the next producer input",
                ));
            }
            let payload = intern_payload(&tx, &serde_json::to_vec(outcome)?)?;
            tx.execute(
                "UPDATE local_file_outcomes SET payload=?3 WHERE operation_id=?1 AND ordinal=?2",
                params![id, i64::try_from(ordinal).map_err(invalid)?, payload],
            )?;
        }
        if let Some(updated) = updated {
            let payload = intern_payload(&tx, &serde_json::to_vec(&updated)?)?;
            tx.execute(
                "UPDATE local_file_progress SET payload=?2 WHERE operation_id=?1",
                params![id, payload],
            )?;
        }
        let completed = completed + outcomes.len();
        tx.execute(
            "UPDATE local_file_progress SET completed=?2 WHERE operation_id=?1",
            params![id, i64::try_from(completed).map_err(invalid)?],
        )?;
        let reused = item.checkpoint["inventory"]["reused_files"]
            .as_u64()
            .unwrap_or(0)
            .checked_add(reused as u64)
            .ok_or_else(|| invalid("reuse counter overflow"))?;
        progress(&mut item, completed, total)?;
        if T::KIND == OperationKind::Inventory {
            item.checkpoint["inventory"]["reused_files"] = reused.into();
        }
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    pub fn completed_inventory(
        &self,
        id: &str,
        root: &Path,
    ) -> Result<entmoot_core::RepoScanResult, OperationError> {
        let (metadata, outcomes) = self.completed_files::<ScanOutcome>(id)?;
        let FileProgress::Inventory(exclusions) = metadata else {
            return Err(invalid("checkpoint is not inventory"));
        };
        Ok(entmoot_core::scan::completed_scan(
            root, exclusions, outcomes,
        ))
    }

    pub fn completed_canon(
        &self,
        id: &str,
    ) -> Result<(CanonCursor, Vec<CanonFileOutcome>), OperationError> {
        let (metadata, outcomes) = self.completed_files::<CanonFileOutcome>(id)?;
        let FileProgress::Canon(cursor) = metadata else {
            return Err(invalid("checkpoint is not canon"));
        };
        Ok((cursor, outcomes))
    }

    fn completed_files<T: FileOutcome>(
        &self,
        id: &str,
    ) -> Result<(FileProgress, Vec<T>), OperationError> {
        let tx = self.connection.unchecked_transaction()?;
        require_complete_if_present(&tx, id)?;
        let (metadata, total): (i64, i64) = tx.query_row(
            "SELECT payload,total FROM local_file_progress WHERE operation_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let total = usize::try_from(total).map_err(invalid)?;
        let metadata: FileProgress = hydrate_record(&tx, metadata)?;
        if metadata.kind() != T::KIND {
            return Err(invalid("file producer differs"));
        }
        let mut statement = tx.prepare("SELECT path,digest,payload FROM local_file_outcomes WHERE operation_id=?1 ORDER BY ordinal")?;
        let rows = statement.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut outcomes = Vec::new();
        for row in rows {
            let (path, digest, payload) = row?;
            let outcome: T = hydrate_record(&tx, payload)?;
            if outcome.input().path != path || outcome.input().content_digest != digest {
                return Err(invalid("file checkpoint membership mismatch"));
            }
            outcomes.push(outcome);
        }
        if outcomes.len() != total {
            return Err(invalid("file checkpoint is missing manifest members"));
        }
        Ok((metadata, outcomes))
    }
}
