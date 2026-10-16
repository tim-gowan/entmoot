//! Private immutable Git deltas. Publication shares the operation checkpoint owner.
use super::*;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE local_integration_packs (
    operation_id TEXT NOT NULL REFERENCES local_operations(operation_id),
    sequence INTEGER NOT NULL, digest TEXT NOT NULL, body BLOB NOT NULL,
    PRIMARY KEY(operation_id,sequence)
) WITHOUT ROWID;
"#;

impl SqliteKnowledgeRepository {
    pub(crate) fn integration_pack_sequences(&self, id: &str) -> Result<Vec<u64>, OperationError> {
        let mut statement = self.connection.prepare(
            "SELECT sequence FROM local_integration_packs WHERE operation_id=?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([id], |row| row.get::<_, i64>(0))?;
        rows.map(|row| u64::try_from(row?).map_err(|e| OperationError::Invalid(e.to_string())))
            .collect()
    }

    pub(crate) fn integration_pack(
        &self,
        id: &str,
        sequence: u64,
    ) -> Result<Vec<u8>, OperationError> {
        let sequence =
            i64::try_from(sequence).map_err(|e| OperationError::Invalid(e.to_string()))?;
        let (digest, bytes): (String, Vec<u8>) = self.connection.query_row(
            "SELECT digest,body FROM local_integration_packs WHERE operation_id=?1 AND sequence=?2",
            params![id, sequence],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if entmoot_contracts::blake3_hex(&bytes) != digest {
            return Err(OperationError::Invalid(
                "integration pack digest mismatch".into(),
            ));
        }
        Ok(bytes)
    }

    pub(crate) fn integration_staged_bytes(&self, id: &str) -> Result<u64, OperationError> {
        let size: i64 = self.connection.query_row(
            "SELECT COALESCE(SUM(length(body)),0) FROM local_integration_packs WHERE operation_id=?1",
            [id],
            |row| row.get(0),
        )?;
        u64::try_from(size).map_err(|e| OperationError::Invalid(e.to_string()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn checkpoint_integration(
        &mut self,
        id: &str,
        generation: u64,
        sequence: u64,
        now_ms: u64,
        checkpoint: serde_json::Value,
        result: Option<serde_json::Value>,
        output: serde_json::Value,
        pack: Option<&[u8]>,
        max_staged_bytes: u64,
    ) -> Result<OperationRecord, OperationError> {
        self.commit_checkpoint(
            id,
            generation,
            sequence,
            now_ms,
            checkpoint,
            result,
            Some(output),
            pack,
            max_staged_bytes,
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use entmoot_contracts::OperationKind;
    use serde_json::json;

    fn running(db: &mut SqliteKnowledgeRepository) -> OperationRecord {
        let spec = OperationSpec {
            repository_id: "fixture/integration-storage".into(),
            idempotency_key: "test".into(),
            owner: "agent".into(),
            kind: OperationKind::Integration,
            views: vec![],
            prerequisites: vec![],
            input: json!({}),
            implementation: crate::implementation_fingerprint(),
        };
        let item = db.submit_operations(&[spec], 10).unwrap().remove(0);
        db.claim_operation(item.id.as_str(), "worker", 1000, 1000)
            .unwrap()
    }

    #[test]
    fn private_pack_cursor_and_output_publish_atomically_and_retries_are_exact() {
        let tmp = tempfile::tempdir().unwrap();
        let mut db = SqliteKnowledgeRepository::open(&tmp.path().join("store.db")).unwrap();
        let item = running(&mut db);
        let id = item.id.as_str();
        // Fail after inserting the pack but before saving the cursor.
        db.connection.execute_batch("CREATE TEMP TRIGGER fail_output BEFORE INSERT ON local_operation_outputs BEGIN SELECT RAISE(ABORT,'injected output failure'); END;").unwrap();
        assert!(db
            .checkpoint_integration(
                id,
                item.generation,
                1,
                1001,
                json!({"next":1}),
                None,
                json!({"step":1}),
                Some(b"pack one"),
                100
            )
            .is_err());
        assert_eq!(db.operation(id).unwrap().unwrap(), item);
        assert_eq!(db.integration_staged_bytes(id).unwrap(), 0);
        assert!(db.operation_outputs(id, 0, 10).unwrap().outputs.is_empty());
        db.connection
            .execute_batch("DROP TRIGGER fail_output")
            .unwrap();
        let first = db
            .checkpoint_integration(
                id,
                item.generation,
                1,
                1001,
                json!({"next":1}),
                None,
                json!({"step":1}),
                Some(b"pack one"),
                100,
            )
            .unwrap();
        assert_eq!(
            db.checkpoint_integration(
                id,
                item.generation,
                1,
                1002,
                json!({"next":1}),
                None,
                json!({"step":1}),
                Some(b"pack one"),
                100
            )
            .unwrap(),
            first
        );
        assert!(db
            .checkpoint_integration(
                id,
                item.generation,
                1,
                1002,
                json!({"next":1}),
                None,
                json!({"step":1}),
                Some(b"different"),
                100
            )
            .is_err());
        assert!(matches!(
            db.checkpoint_integration(
                id,
                item.generation,
                2,
                1002,
                json!({"next":2}),
                None,
                json!({"step":2}),
                Some(b"pack two"),
                10
            ),
            Err(OperationError::Budget(_))
        ));
        assert_eq!(db.operation(id).unwrap().unwrap(), first);
        assert_eq!(db.integration_pack(id, 1).unwrap(), b"pack one");
        assert_eq!(db.operation_outputs(id, 0, 10).unwrap().outputs.len(), 1);
        db.control_operation(id, OperationControl::Pause).unwrap();
        assert!(db
            .checkpoint_integration(
                id,
                item.generation,
                2,
                1003,
                json!({"next":2}),
                None,
                json!({"step":2}),
                Some(b"pack two"),
                100
            )
            .is_err());
        assert_eq!(db.integration_pack_sequences(id).unwrap(), vec![1]);
        db.control_operation(id, OperationControl::Cancel).unwrap();
        assert_eq!(db.integration_staged_bytes(id).unwrap(), 0);
        assert_eq!(db.operation_outputs(id, 0, 10).unwrap().outputs.len(), 1);
    }

    #[test]
    fn collection_keeps_active_objects_and_completion_releases_them_with_idempotent_result() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("store.db");
        let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
        let item = running(&mut db);
        let id = item.id.as_str();
        db.checkpoint_integration(
            id,
            item.generation,
            1,
            1001,
            json!({"next":1}),
            None,
            json!({"step":1}),
            Some(b"retained"),
            100,
        )
        .unwrap();
        drop(db);
        let mut maintenance = crate::knowledge_db::retention::Maintenance::open(&path).unwrap();
        maintenance
            .collect(&std::collections::BTreeSet::new())
            .unwrap();
        drop(maintenance);
        let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
        assert_eq!(db.integration_pack(id, 1).unwrap(), b"retained");
        // Corruption is detected before these bytes can enter Git.
        db.connection
            .execute(
                "UPDATE local_integration_packs SET body=?1 WHERE operation_id=?2",
                params![b"corrupt".as_slice(), id],
            )
            .unwrap();
        assert!(db.integration_pack(id, 1).is_err());
        let done = db
            .checkpoint_integration(
                id,
                item.generation,
                2,
                1002,
                json!({"next":2}),
                Some(json!({"status":"clean"})),
                json!({"step":2}),
                None,
                100,
            )
            .unwrap();
        assert_eq!(done.state, OperationState::Completed);
        assert_eq!(db.integration_staged_bytes(id).unwrap(), 0);
        assert_eq!(
            db.checkpoint_integration(
                id,
                item.generation,
                2,
                3000,
                json!({"next":2}),
                Some(json!({"status":"clean"})),
                json!({"step":2}),
                None,
                100
            )
            .unwrap(),
            done
        );
        assert_eq!(db.operation_outputs(id, 0, 10).unwrap().outputs.len(), 2);
    }
}
