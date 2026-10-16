//! Atomic policy checkpoints in a private, sealed knowledge view.
use super::*;
use crate::{
    revalidation::{PreparedRevalidation, RevalidationWriter},
    revalidation_operations::{error, ledger_view, transition, Progress},
};
use entmoot_contracts::{OperationKind, RevalidationRequest, RevalidationResult};

struct Writer<'a, 'connection>(&'a Transaction<'connection>);
impl RevalidationWriter for Writer<'_, '_> {
    fn replace(
        &mut self,
        publications: &[OwnerReplacement<'_>],
    ) -> Result<(), KnowledgeRepositoryError> {
        let planned = prepare_owner_replacements(publications)?;
        apply_owner_replacements(self.0, &planned).map(|_| ())
    }
    fn visible(&self, record: &KnowledgeRecordRef) -> Result<bool, KnowledgeRepositoryError> {
        Ok(record_state_in_connection(self.0, record)?.is_some_and(|state| state.visible))
    }
}

fn live(
    tx: &Transaction<'_>,
    id: &str,
    generation: u64,
    now: u64,
    sequence: u64,
) -> Result<(OperationRecord, RevalidationRequest, Progress), OperationError> {
    let item = read_operation(tx, id)?
        .ok_or_else(|| OperationError::Invalid("unknown revalidation operation".into()))?;
    if item.spec.kind != OperationKind::Revalidation
        || item.generation != generation
        || item.state != OperationState::Running
        || item.lease_until_ms.is_none_or(|t| now >= t)
        || item.checkpoint_sequence != sequence
    {
        return Err(OperationError::Conflict(
            "revalidation publication requires a live worker and current checkpoint".into(),
        ));
    }
    let request: RevalidationRequest = serde_json::from_value(item.spec.input.clone())?;
    let progress = Progress::read(&item, request.units.len())?;
    let selected: String =
        tx.query_row("SELECT id FROM temp.selected_knowledge_view", [], |row| {
            row.get(0)
        })?;
    if selected != progress.ledger_view.as_str() {
        return Err(OperationError::Invalid(
            "wrong private revalidation view".into(),
        ));
    }
    let outputs: i64 = tx.query_row(
        "SELECT COUNT(*) FROM local_operation_outputs WHERE operation_id=?1",
        [id],
        |row| row.get(0),
    )?;
    if outputs
        != i64::try_from(progress.completed_units)
            .map_err(|e| OperationError::Invalid(e.to_string()))?
    {
        return Err(OperationError::Invalid(
            "revalidation output and cursor disagree".into(),
        ));
    }
    Ok((item, request, progress))
}

fn unseal(tx: &Transaction<'_>, id: &str, progress: &Progress) -> Result<(), OperationError> {
    let owner: Option<String> = tx
        .query_row(
            "SELECT value FROM knowledge_metadata WHERE key='revalidation_operation'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match owner {
        Some(owner) if owner == id => {
            let sealed = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM local_view_seals WHERE view_id=?1 AND published=0)",
                [progress.ledger_view.as_str()],
                |row| row.get::<_, bool>(0),
            )?;
            if !sealed {
                return Err(OperationError::Conflict(
                    "revalidation prefix is not privately sealed".into(),
                ));
            }
            tx.execute(
                "DELETE FROM local_view_seals WHERE view_id=?1",
                [progress.ledger_view.as_str()],
            )?;
        }
        None if progress.completed_units == 0 => {
            let exists = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM scoped_metadata WHERE view_id=?1
                 UNION ALL SELECT 1 FROM scoped_records WHERE view_id=?1
                 UNION ALL SELECT 1 FROM inventory_headers WHERE view_id=?1
                 UNION ALL SELECT 1 FROM local_view_seals WHERE view_id=?1)",
                [progress.ledger_view.as_str()],
                |row| row.get::<_, bool>(0),
            )?;
            if exists {
                return Err(OperationError::Conflict(
                    "revalidation view already belongs to another publication".into(),
                ));
            }
            set_store_digest(tx, OWNERSHIP_STORE_MARKER)?;
            tx.execute(
                "INSERT INTO scoped_metadata VALUES(?1,'revalidation_operation',?2)",
                params![progress.ledger_view.as_str(), id],
            )?;
            tx.execute(
                "INSERT INTO local_operation_pins VALUES(?1,?2)",
                params![id, progress.ledger_view.as_str()],
            )?;
        }
        _ => {
            return Err(OperationError::Conflict(
                "private revalidation owner mismatch".into(),
            ))
        }
    }
    Ok(())
}

impl SqliteKnowledgeRepository {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn checkpoint_revalidation(
        &mut self,
        id: &str,
        generation: u64,
        now: u64,
        sequence: u64,
        prepared: &PreparedRevalidation<'_>,
        max_records: usize,
    ) -> Result<OperationRecord, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (mut item, request, mut progress) = live(&tx, id, generation, now, sequence)?;
        let frozen = transition(
            &request,
            view_snapshot(&tx, request.base_view.as_str())?,
            view_snapshot(&tx, request.head_view.as_str())?,
        );
        if prepared.ordinal != progress.completed_units
            || prepared.context
                != crate::revalidation::transition_context(&frozen).map_err(error)?
        {
            return Err(OperationError::Invalid(
                "prepared revalidation differs from frozen policy inputs".into(),
            ));
        }
        let required = progress
            .semantic_records
            .saturating_add(prepared.record_count());
        if required > max_records || !(1..=1_000_000).contains(&max_records) {
            return Err(OperationError::Budget(
                "revalidation record allowance exhausted".into(),
            ));
        }
        unseal(&tx, id, &progress)?;
        let receipt = prepared.apply(&mut Writer(&tx)).map_err(error)?;
        progress.completed_units += 1;
        progress.semantic_records = required;
        item.checkpoint_sequence = next(item.checkpoint_sequence)?;
        item.checkpoint = serde_json::to_value(&progress)?;
        tx.execute(
            "INSERT INTO local_operation_outputs VALUES(?1,?2,?3)",
            params![
                id,
                i64::try_from(item.checkpoint_sequence)
                    .map_err(|e| OperationError::Invalid(e.to_string()))?,
                encode(&receipt)?
            ],
        )?;
        tx.execute(
            "INSERT INTO local_view_seals VALUES(?1,0)",
            [progress.ledger_view.as_str()],
        )?;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    pub(crate) fn publish_revalidation(
        &mut self,
        id: &str,
        generation: u64,
        now: u64,
        sequence: u64,
        result: RevalidationResult,
    ) -> Result<OperationRecord, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (mut item, request, progress) = live(&tx, id, generation, now, sequence)?;
        if progress.completed_units != request.units.len()
            || result.ledger_view != progress.ledger_view
            || result.completed_units != progress.completed_units
            || result.semantic_records != progress.semantic_records
            || result.base_snapshot != view_snapshot(&tx, request.base_view.as_str())?
            || result.head_snapshot != view_snapshot(&tx, request.head_view.as_str())?
        {
            return Err(OperationError::Invalid(
                "revalidation result does not match complete frozen prefix".into(),
            ));
        }
        unseal(&tx, id, &progress)?;
        tx.execute(
            "INSERT INTO local_view_seals VALUES(?1,1)",
            [progress.ledger_view.as_str()],
        )?;
        item.checkpoint = serde_json::to_value(progress)?;
        item.checkpoint_sequence = next(item.checkpoint_sequence)?;
        item.result = Some(serde_json::to_value(result)?);
        item.state = OperationState::Completed;
        item.lease_until_ms = None;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    pub(crate) fn require_published_revalidation(&self, id: &str) -> Result<(), OperationError> {
        let view = ledger_view(id);
        let published = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_view_seals s
             JOIN scoped_metadata m ON m.view_id=s.view_id
             WHERE s.view_id=?1 AND s.published=1 AND m.key='revalidation_operation' AND m.value=?2)",
            params![view.as_str(), id],
            |row| row.get::<_, bool>(0),
        )?;
        if !published {
            return Err(OperationError::Unavailable(
                "revalidation ledger missing or no longer retained".into(),
            ));
        }
        Ok(())
    }
}

pub(super) fn cancel(
    connection: &Connection,
    item: &OperationRecord,
) -> Result<(), OperationError> {
    if item.spec.kind != OperationKind::Revalidation {
        return Ok(());
    }
    let view = ledger_view(item.id.as_str());
    let owner: Option<String> = connection
        .query_row(
            "SELECT value FROM scoped_metadata WHERE view_id=?1 AND key='revalidation_operation'",
            [view.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if owner.as_deref() != Some(item.id.as_str()) {
        return Ok(());
    }
    connection.execute(
        "DELETE FROM local_view_seals WHERE view_id=?1",
        [view.as_str()],
    )?;
    for table in [
        "scoped_visibility_events",
        "scoped_record_dependency_sets",
        "scoped_record_dependencies",
        "scoped_record_owners",
        "scoped_records",
        "scoped_metadata",
    ] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE view_id=?1"),
            [view.as_str()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_publication_fences_worker_view_and_snapshot_and_retention_reports_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.py"), "def a(): pass\n").unwrap();
        let graph =
            crate::build_topology_from_repo_with_options(&root, Default::default()).unwrap();
        let store = directory.path().join("store.db");
        let publication =
            crate::collaboration::register_graph(&store, &graph, "main", "agent", None, 0).unwrap();
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let request = RevalidationRequest {
            base_view: publication.view.clone(),
            head_view: publication.view,
            units: vec![],
        };
        let submitted =
            crate::revalidation_operations::submit(&mut db, request, "empty", "agent").unwrap();
        let id = submitted.id.as_str();
        let claim = db.claim_operation(id, "worker", 10, 100).unwrap();
        let result = RevalidationResult {
            ledger_view: ledger_view(id),
            base_snapshot: graph.snapshot.clone(),
            head_snapshot: graph.snapshot,
            completed_units: 0,
            semantic_records: 0,
            unselected_artifact_ids: vec![],
            limitations: vec![],
        };
        assert!(db
            .publish_revalidation(id, claim.generation, 11, 0, result.clone())
            .is_err());
        let mut ledger =
            SqliteKnowledgeRepository::open_view(&store, result.ledger_view.as_str()).unwrap();
        let mut wrong = result.clone();
        wrong.head_snapshot.repository_id = "other".into();
        assert!(ledger
            .publish_revalidation(id, claim.generation, 11, 0, wrong)
            .is_err());
        assert!(ledger
            .publish_revalidation(id, claim.generation, 110, 0, result.clone())
            .is_err());
        db.control_operation(id, OperationControl::Pause).unwrap();
        db.control_operation(id, OperationControl::Resume).unwrap();
        let new = db.claim_operation(id, "new-worker", 20, 100).unwrap();
        assert!(ledger
            .publish_revalidation(id, claim.generation, 21, 0, result.clone())
            .is_err());
        assert!(!ledger.is_initialized().unwrap());
        ledger
            .publish_revalidation(id, new.generation, 21, 0, result.clone())
            .unwrap();
        assert!(ledger.replace_all(&KnowledgeStore::default()).is_err());
        let record = KnowledgeRecordRef::new("findings", "unreported");
        assert_eq!(
            crate::revalidation_operations::record_state(&store, id, record.clone())
                .unwrap()
                .visible,
            None
        );
        drop(ledger);
        drop(db);
        let mut maintenance = crate::knowledge_db::retention::Maintenance::open(&store).unwrap();
        maintenance
            .collect(&BTreeSet::from([result.ledger_view.to_string()]))
            .unwrap();
        drop(maintenance);
        assert!(matches!(
            crate::revalidation_operations::record_state(&store, id, record),
            Err(OperationError::Unavailable(_))
        ));
        assert_eq!(
            SqliteKnowledgeRepository::open(&store)
                .unwrap()
                .operation(id)
                .unwrap()
                .unwrap()
                .state,
            OperationState::Completed
        );
    }
}
