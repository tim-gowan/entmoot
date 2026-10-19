//! Raw Git inputs share source bytes with inventory, scoped to their owning view.
use super::*;
use crate::checkpoints::git::RetainedGitCapture;
use entmoot_contracts::GitTreeCapture;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE local_git_captures (
    capture_id TEXT PRIMARY KEY, view_id TEXT NOT NULL,
    payload INTEGER NOT NULL REFERENCES shared_payloads
) WITHOUT ROWID;
CREATE INDEX local_git_captures_by_view ON local_git_captures(view_id);
CREATE TABLE local_git_files (
    capture_id TEXT NOT NULL REFERENCES local_git_captures(capture_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, file_id INTEGER NOT NULL REFERENCES shared_source_files,
    PRIMARY KEY(capture_id,ordinal)
) WITHOUT ROWID;
"#;

pub(super) fn validate_publication(
    tx: &Connection,
    row: &WorkspacePublication,
) -> Result<(), OperationError> {
    if let Some(id) = &row.git_capture {
        let receipt = receipt(tx, id.as_str())?;
        if receipt.view != row.view || receipt.source.snapshot != row.snapshot {
            return Err(OperationError::Invalid(
                "Git capture snapshot differs".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn receipt(connection: &Connection, id: &str) -> Result<GitTreeCapture, OperationError> {
    let payload: i64 = connection.query_row(
        "SELECT payload FROM local_git_captures WHERE capture_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let receipt: GitTreeCapture = hydrate_record(connection, payload)?;
    if entmoot_contracts::blake3_hex(&serde_json::to_vec(&receipt)?) != id {
        return Err(OperationError::Invalid(
            "Git capture digest mismatch".into(),
        ));
    }
    Ok(receipt)
}

pub(super) fn load(
    connection: &Connection,
    id: &str,
) -> Result<RetainedGitCapture, OperationError> {
    let tx = connection.unchecked_transaction()?;
    let receipt = receipt(&tx, id)?;
    let mut statement = tx.prepare("SELECT f.digest,f.bytes FROM local_git_files c JOIN shared_source_files f ON f.file_id=c.file_id WHERE c.capture_id=?1 ORDER BY c.ordinal")?;
    let parent = connection
        .path()
        .filter(|path| !path.is_empty())
        .and_then(|path| Path::new(path).parent())
        .filter(|path| !path.as_os_str().is_empty());
    let mut blobs = match parent {
        Some(parent) => crate::checkpoints::git::CapturedBlobs::new_in(parent)?,
        None => crate::checkpoints::git::CapturedBlobs::temporary()?,
    };
    let mut rows = statement.query([id])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let file = receipt.files.get(count).ok_or_else(|| {
            OperationError::Unavailable("Git capture member count differs".into())
        })?;
        let digest: String = row.get(0)?;
        let bytes: Vec<u8> = row.get(1)?;
        if digest != file.content_digest.as_str() {
            return Err(OperationError::Invalid("Git capture bytes mismatch".into()));
        }
        blobs.insert(&digest, &bytes)?;
        count += 1;
    }
    if count != receipt.files.len() {
        return Err(OperationError::Unavailable(
            "Git capture member count differs".into(),
        ));
    }
    let captured = RetainedGitCapture { receipt, blobs };
    captured.validate()?;
    Ok(captured)
}

impl SqliteKnowledgeRepository {
    pub fn workspace_publication(&self, id: &str) -> Result<WorkspacePublication, OperationError> {
        let body: Vec<u8> = self.connection.query_row(
            "SELECT body FROM local_workspaces WHERE workspace_id=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn publish_git_capture(
        &mut self,
        mut publication: WorkspacePublication,
        captured: &RetainedGitCapture,
        expected: u64,
    ) -> Result<WorkspacePublication, OperationError> {
        captured.validate()?;
        if captured.receipt.view != publication.view
            || captured.receipt.source.snapshot != publication.snapshot
        {
            return Err(OperationError::Conflict(
                "Git capture and publication source differ".into(),
            ));
        }
        let encoded = serde_json::to_vec(&captured.receipt)?;
        let id = entmoot_contracts::blake3_hex(&encoded);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let payload = intern_payload(&tx, &encoded)?;
        tx.execute(
            "INSERT OR IGNORE INTO local_git_captures VALUES(?1,?2,?3)",
            params![id, publication.view.as_str(), payload],
        )?;
        for (ordinal, file) in captured.receipt.files.iter().enumerate() {
            let bytes = captured.blobs.read(file.content_digest.as_str())?;
            let (file_id, retained) =
                inventory::intern_source_digest(&tx, file.content_digest.as_str())?;
            if !retained {
                inventory::retain_source_bytes(&tx, file_id, &bytes)?;
            }
            tx.execute(
                "INSERT OR IGNORE INTO local_git_files VALUES(?1,?2,?3)",
                params![
                    id,
                    i64::try_from(ordinal).map_err(|e| OperationError::Invalid(e.to_string()))?,
                    file_id
                ],
            )?;
        }
        publication.git_capture = Some(ContentDigest::from_trusted(id));
        publication.publisher = captured.receipt.publisher.clone();
        let result = publish_workspace_in_transaction(&tx, publication, expected)?;
        tx.commit()?;
        Ok(result)
    }
}
