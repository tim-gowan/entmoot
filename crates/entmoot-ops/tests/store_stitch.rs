#[cfg(test)]
mod tests {
    use entmoot_contracts::*;
    use entmoot_contracts::{ProducerPublication, ProducerRequest, ProducerTask};
    use entmoot_ops::inventory_stitch::Part;
    use entmoot_ops::inventory_stitch::{CallPart, SyntaxPart};
    use entmoot_ops::*;
    use std::path::Path;

    fn setup(root: &Path, store: &Path) -> (SqliteKnowledgeRepository, OperationRecord) {
        std::fs::write(
            root.join("a.py"),
            "from .b import leaf\ndef caller():\n    return leaf()\n",
        )
        .unwrap();
        std::fs::write(root.join("b.py"), "def leaf():\n    return 1\n").unwrap();
        let mut db = SqliteKnowledgeRepository::open(store).unwrap();
        let request = ProducerRequest {
            source: entmoot_ops::producer_operations::observe_source(root, "frontier/recovery")
                .unwrap(),
            publication: ProducerPublication {
                workspace_id: "topic".into(),
                publisher: "agent".into(),
                branch: None,
                expected_generation: 0,
            },
            task: ProducerTask::Inventory {
                policy: Default::default(),
            },
        };
        let job =
            entmoot_ops::producer_operations::submit(&mut db, request, "index", "agent").unwrap();
        let job = db.claim_operation(job.id.as_str(), "child", 0, 1).unwrap();
        let manifest = entmoot_core::scan::source_manifest(root).unwrap();
        db.begin_inventory(job.id.as_str(), job.generation, 0, &manifest)
            .unwrap();
        let outcomes = entmoot_core::scan::ScanWorkers::new()
            .unwrap()
            .scan(
                root,
                "frontier/recovery",
                &Default::default(),
                &manifest.inputs,
            )
            .unwrap();
        db.checkpoint_inventory_batch(job.id.as_str(), job.generation, 0, &outcomes, 0)
            .unwrap();
        (db, job)
    }

    fn syntax(
        db: &mut SqliteKnowledgeRepository,
        job: &OperationRecord,
        root: &Path,
        count: usize,
    ) {
        let frontier = db
            .inventory_frontier(job.id.as_str(), count)
            .unwrap()
            .unwrap();
        let parts = frontier
            .outcomes
            .iter()
            .map(|o| Part::Syntax(SyntaxPart::build(root, o).unwrap()))
            .collect::<Vec<_>>();
        db.checkpoint_inventory_frontier(job.id.as_str(), job.generation, 0, &frontier, &parts, 0)
            .unwrap();
    }

    #[test]
    fn frontier_crash_child() {
        let Some(root) = std::env::var_os("ENTMOOT_TEST_STITCH_CRASH") else {
            return;
        };
        let root = Path::new(&root);
        let store = root.parent().unwrap().join("shared.db");
        let (mut db, job) = setup(root, &store);
        if std::env::var("ENTMOOT_TEST_STITCH_PHASE").unwrap() == "syntax" {
            syntax(&mut db, &job, root, 1);
        } else {
            syntax(&mut db, &job, root, 2);
            let request: ProducerRequest = serde_json::from_value(job.spec.input.clone()).unwrap();
            let graph = db
                .inventory_syntax_graph(job.id.as_str(), root, &request.source.snapshot)
                .unwrap();
            let index = entmoot_ops::resolve_edges::StructuralCallIndex::new(&graph);
            let frontier = db.inventory_frontier(job.id.as_str(), 1).unwrap().unwrap();
            let parts = frontier
                .outcomes
                .iter()
                .map(|o| Part::Calls(CallPart::build(&index, o)))
                .collect::<Vec<_>>();
            db.checkpoint_inventory_frontier(
                job.id.as_str(),
                job.generation,
                0,
                &frontier,
                &parts,
                0,
            )
            .unwrap();
        }
        std::fs::write(root.parent().unwrap().join("operation"), job.id.as_str()).unwrap();
        // Release the lifecycle lease before aborting so the parent does not wait on
        // process-exit FD cleanup under parallel CI load.
        drop(db);
        std::process::exit(0);
    }

    #[test]
    fn graph_frontiers_survive_process_exit_collection_and_new_worker_generation() {
        for phase in ["syntax", "calls"] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("source");
            std::fs::create_dir(&root).unwrap();
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::frontier_crash_child"])
                .env("ENTMOOT_TEST_STITCH_CRASH", &root)
                .env("ENTMOOT_TEST_STITCH_PHASE", phase)
                .status()
                .unwrap()
                .success());
            let store = tmp.path().join("shared.db");
            let id = std::fs::read_to_string(tmp.path().join("operation")).unwrap();
            let before = {
                let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
                db.operation(&id).unwrap().unwrap()
            };
            assert_eq!(before.state, OperationState::Running);
            {
                let mut maintenance =
                    entmoot_ops::knowledge_db::retention::Maintenance::open(&store).unwrap();
                maintenance
                    .collect(&std::collections::BTreeSet::new())
                    .unwrap();
            }
            let done = entmoot_ops::producer_operations::run_bounded(
                &store,
                &root,
                &id,
                "replacement",
                10,
                1,
                None,
            )
            .unwrap();
            assert_eq!(done.state, OperationState::Completed);
            assert!(done.generation > before.generation);
            let graph = SqliteKnowledgeRepository::open_operations_read_only(&store)
                .unwrap()
                .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
                .unwrap();
            assert!(graph.edges.iter().any(|e| {
                e.kind == entmoot_ops::EdgeKind::Calls
                    && e.resolved
                    && graph
                        .node(&e.from)
                        .is_some_and(|n| n.path.as_deref() == Some("a.py"))
                    && graph
                        .node(&e.to)
                        .is_some_and(|n| n.path.as_deref() == Some("b.py"))
            }));
        }
    }

    #[test]
    fn incomplete_stitch_cannot_stage_and_stale_or_replayed_frontiers_cannot_advance() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let store = tmp.path().join("shared.db");
        let (mut db, old) = setup(&root, &store);
        let graph = entmoot_ops::build_topology_from_repo_with_options(
            &root,
            entmoot_ops::TopologyBuildOptions {
                repository: Some("frontier/recovery"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(db
            .stage_producer_graph(old.id.as_str(), old.generation, 0, &graph, None)
            .unwrap_err()
            .to_string()
            .contains("incomplete graph frontiers"));
        let frontier = db.inventory_frontier(old.id.as_str(), 1).unwrap().unwrap();
        let parts = frontier
            .outcomes
            .iter()
            .map(|o| Part::Syntax(SyntaxPart::build(&root, o).unwrap()))
            .collect::<Vec<_>>();
        let new = db.claim_operation(old.id.as_str(), "new", 1, 100).unwrap();
        assert!(db
            .checkpoint_inventory_frontier(old.id.as_str(), old.generation, 1, &frontier, &parts, 0)
            .is_err());
        db.checkpoint_inventory_frontier(old.id.as_str(), new.generation, 1, &frontier, &parts, 0)
            .unwrap();
        assert!(db
            .checkpoint_inventory_frontier(old.id.as_str(), new.generation, 1, &frontier, &parts, 0)
            .is_err());
        let request: ProducerRequest = serde_json::from_value(old.spec.input.clone()).unwrap();
        assert!(db
            .inventory_syntax_graph(old.id.as_str(), &root, &request.source.snapshot)
            .is_err());
        db.control_operation(old.id.as_str(), entmoot_contracts::OperationControl::Cancel)
            .unwrap();
        assert_eq!(
            rusqlite::Connection::open(&store)
                .unwrap()
                .query_row("SELECT count(*) FROM local_inventory_parts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
