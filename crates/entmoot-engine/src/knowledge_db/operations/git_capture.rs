//! Raw Git inputs share source bytes with inventory, scoped to their owning view.
use super::*;
use crate::collaboration_git::RetainedGitCapture;
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
    let rows = statement
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != receipt.files.len() {
        return Err(OperationError::Unavailable(
            "Git capture member count differs".into(),
        ));
    }
    let mut blobs = std::collections::BTreeMap::new();
    for (file, row) in receipt.files.iter().zip(rows) {
        let (digest, bytes) = row;
        if digest != file.content_digest.as_str() || entmoot_contracts::blake3_hex(&bytes) != digest
        {
            return Err(OperationError::Invalid("Git capture bytes mismatch".into()));
        }
        blobs.insert(digest, bytes);
    }
    let captured = RetainedGitCapture { receipt, blobs };
    captured.validate()?;
    Ok(captured)
}

impl SqliteKnowledgeRepository {
    pub(crate) fn workspace_publication(
        &self,
        id: &str,
    ) -> Result<WorkspacePublication, OperationError> {
        let body: Vec<u8> = self.connection.query_row(
            "SELECT body FROM local_workspaces WHERE workspace_id=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_slice(&body)?)
    }

    pub(crate) fn publish_git_capture(
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
            let bytes = captured
                .blobs
                .get(file.content_digest.as_str())
                .ok_or_else(|| OperationError::Invalid("Git file bytes missing".into()))?;
            let (file_id, retained) =
                inventory::intern_source_digest(&tx, file.content_digest.as_str())?;
            if !retained {
                inventory::retain_source_bytes(&tx, file_id, bytes)?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn captures_share_bytes_and_survive_collection_until_their_pinned_view_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("app.py"), "value = 1\n").unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            &source,
            crate::TopologyBuildOptions {
                repository: Some("gc/git"),
                ..Default::default()
            },
        )
        .unwrap();
        let store = dir.path().join("shared.db");
        let row =
            crate::collaboration::register_graph(&store, &graph, "w", "agent", None, 0).unwrap();
        let bytes = b"retained raw nonstructural input".to_vec();
        let digest = entmoot_contracts::blake3_hex(&bytes);
        let capture = RetainedGitCapture {
            receipt: GitTreeCapture {
                view: row.view.clone(),
                source: crate::producer_operations::graph_source(&graph).unwrap(),
                parent_commit: "0".repeat(40),
                object_format: "sha1".into(),
                publisher: "agent".into(),
                files: vec![entmoot_contracts::GitTreeFile {
                    path: "raw.bin".into(),
                    mode: "100644".into(),
                    content_digest: ContentDigest::from_trusted(digest.clone()),
                }],
                limitations: vec![],
            },
            blobs: BTreeMap::from([(digest.clone(), bytes.clone())]),
        };
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let row = db.publish_git_capture(row, &capture, 1).unwrap();
        let id = row.git_capture.as_ref().unwrap().to_string();
        let job = crate::collaboration::submit_conflicts(
            &mut db,
            entmoot_contracts::ConflictBatchRequest {
                git_merge: true,
                base_view: row.view.clone(),
                left: row.clone(),
                peers: vec![],
            },
            "gc",
            "agent",
        )
        .unwrap();
        db.retire_workspace("w", 2).unwrap();
        drop(db);
        let remove = BTreeSet::from([row.view.to_string()]);
        let mut maintenance = crate::knowledge_db::retention::Maintenance::open(&store).unwrap();
        maintenance.collect(&BTreeSet::new()).unwrap();
        assert!(maintenance.collect(&remove).is_err());
        drop(maintenance);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        assert_eq!(db.git_capture(&id).unwrap().blobs[&digest], bytes);
        db.control_operation(job.id.as_str(), entmoot_contracts::OperationControl::Cancel)
            .unwrap();
        drop(db);
        crate::knowledge_db::retention::Maintenance::open(&store)
            .unwrap()
            .collect(&remove)
            .unwrap();
        let db = SqliteKnowledgeRepository::open(&store).unwrap();
        assert!(db.git_capture(&id).is_err());
        assert_eq!(
            db.connection
                .query_row("SELECT COUNT(*) FROM local_git_files", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.connection
                .query_row(
                    "SELECT COUNT(*) FROM shared_source_files WHERE digest=?1",
                    [digest],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}
