#[cfg(test)]
mod tests {
    use entmoot_contracts::*;
    use entmoot_ops::checkpoints::revalidation::ledger_view;
    use entmoot_ops::*;
    use std::collections::BTreeSet;

    #[test]
    fn final_publication_fences_worker_view_and_snapshot_and_retention_reports_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.py"), "def a(): pass\n").unwrap();
        let graph =
            entmoot_ops::build_topology_from_repo_with_options(&root, Default::default()).unwrap();
        let store = directory.path().join("store.db");
        let publication =
            entmoot_ops::collaboration::register_graph(&store, &graph, "main", "agent", None, 0)
                .unwrap();
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let request = RevalidationRequest {
            base_view: publication.view.clone(),
            head_view: publication.view,
            units: vec![],
        };
        let submitted =
            entmoot_ops::revalidation_operations::submit(&mut db, request, "empty", "agent")
                .unwrap();
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
            entmoot_ops::revalidation_operations::record_state(&store, id, record.clone())
                .unwrap()
                .visible,
            None
        );
        drop(ledger);
        drop(db);
        let mut maintenance =
            entmoot_ops::knowledge_db::retention::Maintenance::open(&store).unwrap();
        maintenance
            .collect(&BTreeSet::from([result.ledger_view.to_string()]))
            .unwrap();
        drop(maintenance);
        assert!(matches!(
            entmoot_ops::revalidation_operations::record_state(&store, id, record),
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
