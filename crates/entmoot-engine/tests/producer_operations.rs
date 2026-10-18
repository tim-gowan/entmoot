//! Independent source fixtures exercise public producer submission and recovery.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::{
    OperationControl, OperationState, ProducerPublication, ProducerRequest, ProducerTask,
};
use entmoot_engine::{
    collaboration, producer_operations::*, LocalOperations, SqliteKnowledgeRepository,
};
use std::path::Path;

const REPOSITORY: &str = "fixture/producers";

fn resume(store: &Path, id: &str) {
    SqliteKnowledgeRepository::open(store)
        .unwrap()
        .control_operation(id, OperationControl::Resume)
        .unwrap();
}

#[test]
fn graph_frontiers_preserve_cross_file_targets_ambiguity_and_source_order() {
    let (_tmp, root, store) = fixture();
    for (path, source) in [
        ("client.py", "from .target import leaf as remote\ndef leaf():\n    return 2\ndef caller():\n    return remote()\n"),
        ("target.py", "def leaf():\n    return 1\n"),
        ("src/lib.rs", "mod target; use crate::target::leaf; pub fn caller()->i32 { leaf() }"),
        ("src/target.rs", "pub fn leaf()->i32 { 1 }"),
        ("a.ts", "import {leaf} from './a/target'; export function caller(){ return leaf(); }"),
        ("a/target.ts", "export function leaf(){ return 1; }"),
        ("ambiguous.py", "class A:\n    def same(self): pass\nclass B:\n    def same(self): pass\ndef caller():\n    return same()\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let spec = request(&root, "topic");
    let job = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        spec.clone(),
        "index",
        "agent",
    )
    .unwrap();
    let mut slices = 0;
    let done = loop {
        let item = run_bounded(&store, &root, job.id.as_str(), "replacement", 1, 2, None).unwrap();
        slices += 1;
        assert!(slices < 30);
        if item.state == OperationState::Completed {
            break item;
        }
        assert!(SqliteKnowledgeRepository::open_operations_read_only(&store)
            .unwrap()
            .workspaces(REPOSITORY)
            .unwrap()
            .is_empty());
        resume(&store, job.id.as_str());
    };
    assert_eq!(slices, 14); // Four batches per phase, then seal and publish.
    let graph = SqliteKnowledgeRepository::open_operations_read_only(&store)
        .unwrap()
        .retained_graph(done.result.as_ref().unwrap()["view"].as_str().unwrap())
        .unwrap();
    for (caller_path, target_path) in [
        ("client.py", "target.py"),
        ("src/lib.rs", "src/target.rs"),
        ("a.ts", "a/target.ts"),
    ] {
        let caller = graph
            .nodes
            .iter()
            .find(|n| n.path.as_deref() == Some(caller_path) && n.label == "caller")
            .unwrap();
        let target = graph
            .nodes
            .iter()
            .find(|n| n.path.as_deref() == Some(target_path) && n.label == "leaf")
            .unwrap();
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.resolved && e.from == caller.id && e.to == target.id),
            "{caller_path}: {:?}; unresolved {:?}",
            graph
                .edges
                .iter()
                .filter(|e| e.kind == entmoot_engine::EdgeKind::Calls && e.from == caller.id)
                .collect::<Vec<_>>(),
            graph.unresolved_relations
        );
    }
    assert!(graph
        .unresolved_relations
        .iter()
        .any(|u| u.reason == entmoot_contracts::UnresolvedReason::AmbiguousTarget));
    let expected = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    // The storage projection has its own record ordering; all content must match.
    assert_eq!(inventory_value(graph), inventory_value(expected));
    let serial_store = store.with_file_name("serial.sqlite3");
    let serial = submit(
        &mut SqliteKnowledgeRepository::open(&serial_store).unwrap(),
        spec,
        "index",
        "agent",
    )
    .unwrap();
    assert_eq!(
        run_bounded(
            &serial_store,
            &root,
            serial.id.as_str(),
            "serial",
            30,
            3,
            None
        )
        .unwrap()
        .result,
        done.result
    );
}

#[test]
fn failed_graph_part_insert_rolls_back_frontier_and_resume_keeps_the_saved_prefix() {
    let (_tmp, root, store) = fixture();
    std::fs::write(root.join("second.py"), "def other(): pass\n").unwrap();
    let job = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        request(&root, "topic"),
        "index",
        "agent",
    )
    .unwrap();
    // Parse both files, but no extraction has been admitted.
    run_bounded(&store, &root, job.id.as_str(), "parse", 1, 2, None).unwrap();
    let connection = rusqlite::Connection::open(&store).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_graph_part BEFORE INSERT ON local_inventory_parts WHEN NEW.ordinal=1 BEGIN SELECT RAISE(ABORT,'injected graph checkpoint failure'); END;").unwrap();
    resume(&store, job.id.as_str());
    assert!(run_bounded(&store, &root, job.id.as_str(), "failed", 1, 2, None).is_err());
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM local_inventory_parts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT syntax_completed FROM local_file_progress",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    connection
        .execute_batch("DROP TRIGGER reject_graph_part")
        .unwrap();
    resume(&store, job.id.as_str());
    let prefix = run_bounded(&store, &root, job.id.as_str(), "first", 1, 1, None).unwrap();
    assert_eq!(prefix.checkpoint["inventory"]["syntax_files"], 1);
    // A completed part must not be inserted again after restart.
    connection.execute_batch("CREATE TRIGGER reject_replay BEFORE INSERT ON local_inventory_parts WHEN NEW.phase='syntax' AND NEW.ordinal=0 BEGIN SELECT RAISE(ABORT,'completed graph work replayed'); END;").unwrap();
    resume(&store, job.id.as_str());
    assert_eq!(
        run_bounded(&store, &root, job.id.as_str(), "restored", 5, 1, None)
            .unwrap()
            .state,
        OperationState::Completed
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM local_inventory_parts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
#[ignore = "explicit inventory graph-frontier time/storage probe"]
fn inventory_frontier_cost_probe() {
    use std::time::Instant;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    std::fs::create_dir(&root).unwrap();
    for n in 0..192 {
        std::fs::write(root.join(format!("file{n:03}.py")),format!("from .file{:03} import leaf as remote\ndef local(value):\n    return value + {n}\ndef leaf():\n    return local({n})\ndef caller():\n    return remote()\n",(n+1)%192)).unwrap();
    }
    let mut times = std::collections::BTreeMap::<&str, Vec<u128>>::new();
    for repeat in 0..3 {
        for mode in ["ordinary", "durable", "resumed"] {
            let store = tmp.path().join(format!("{mode}-{repeat}.db"));
            let started = Instant::now();
            let (publication, peak) = if mode == "ordinary" {
                let graph = entmoot_engine::build_topology_from_repo_with_options(
                    &root,
                    entmoot_engine::TopologyBuildOptions {
                        repository: Some(REPOSITORY),
                        ..Default::default()
                    },
                )
                .unwrap();
                let publication =
                    collaboration::register_graph(&store, &graph, "topic", "agent", None, 0)
                        .unwrap();
                (publication, 0)
            } else {
                let job = submit(
                    &mut SqliteKnowledgeRepository::open(&store).unwrap(),
                    request(&root, "topic"),
                    "index",
                    "agent",
                )
                .unwrap();
                let mut peak = 0;
                let done = loop {
                    let item = run_bounded(
                        &store,
                        &root,
                        job.id.as_str(),
                        "worker",
                        if mode == "resumed" { 1 } else { 100 },
                        32,
                        None,
                    )
                    .unwrap();
                    peak = peak.max(std::fs::metadata(&store).unwrap().len());
                    if item.state == OperationState::Completed {
                        break item;
                    }
                    resume(&store, job.id.as_str());
                };
                (serde_json::from_value(done.result.unwrap()).unwrap(), peak)
            };
            let elapsed = started.elapsed().as_millis();
            times.entry(mode).or_default().push(elapsed);
            let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
            let graph = db.retained_graph(publication.view.as_str()).unwrap();
            assert_eq!(
                graph
                    .nodes
                    .iter()
                    .filter(|n| n.kind == entmoot_engine::NodeKind::Callable)
                    .count(),
                576
            );
            let callers = graph.nodes.iter().filter(|n| n.label == "caller");
            for caller in callers {
                let target = graph
                    .edges
                    .iter()
                    .find(|e| {
                        e.kind == entmoot_engine::EdgeKind::Calls
                            && e.resolved
                            && e.from == caller.id
                    })
                    .unwrap();
                let callee = graph.nodes.iter().find(|n| n.id == target.to).unwrap();
                assert_ne!(
                    caller.path, callee.path,
                    "{caller:?} -> {callee:?}: {target:?}"
                );
                assert_eq!(callee.label, "leaf");
            }
            eprintln!("mode={mode} repeat={repeat} elapsed_ms={elapsed} final_store_bytes={} peak_store_bytes={peak}",std::fs::metadata(&store).unwrap().len());
        }
    }
    for (mode, mut samples) in times {
        samples.sort();
        eprintln!("mode={mode} median_ms={} samples={samples:?}", samples[1]);
    }
}

// Repository hydration orders collections by record ID; source emission uses
// traversal order. Compare complete records without assigning meaning to collection order.
fn inventory_value(graph: entmoot_engine::TopologyGraph) -> serde_json::Value {
    let mut value = serde_json::to_value(graph).unwrap();
    for collection in value.as_object_mut().unwrap().values_mut() {
        if let Some(rows) = collection.as_array_mut() {
            rows.sort_by_cached_key(|row| serde_json::to_string(row).unwrap());
        }
    }
    value
}

fn request(root: &Path, workspace: &str) -> ProducerRequest {
    ProducerRequest {
        source: observe_source(root, REPOSITORY).unwrap(),
        publication: ProducerPublication {
            workspace_id: workspace.into(),
            branch: Some(workspace.into()),
            publisher: "agent".into(),
            expected_generation: 0,
        },
        task: ProducerTask::Inventory {
            policy: Default::default(),
        },
    }
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("app.py"),
        "def target(x):\n    return x + 7\n\ndef caller():\n    return target(4)\n",
    )
    .unwrap();
    let store = tmp.path().join("shared.sqlite3");
    (tmp, root, store)
}

#[test]
fn shared_worktree_cache_replays_fresh_scopes_across_restarts_and_contexts() {
    let (tmp, root, store) = fixture();
    std::fs::write(
        root.join("client.ts"),
        "import {leaf} from './leaf'; function call(){ return leaf(); }",
    )
    .unwrap();
    std::fs::write(root.join("leaf.ts"), "export function leaf(){ return 1; }").unwrap();
    let first = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        request(&root, "base"),
        "base",
        "agent",
    )
    .unwrap();
    let done = run(&store, &root, first.id.as_str(), "base", 5).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let original = done.result.unwrap()["view"].as_str().unwrap().to_owned();
    let peer = tmp.path().join("peer");
    std::fs::create_dir(&peer).unwrap();
    for file in ["app.py", "client.ts", "leaf.ts"] {
        std::fs::copy(root.join(file), peer.join(file)).unwrap();
    }
    for (round, (parse, calls)) in [(3, 3), (3, 2), (0, 0)].into_iter().enumerate() {
        if round == 1 {
            std::fs::write(peer.join("leaf.js"), "export function leaf(){ return 2; }").unwrap();
        }
        let name = format!("peer-{round}");
        let mut spec = request(&peer, &name);
        if round == 2 {
            spec.source = observe_source(&peer, "different/repository").unwrap();
        }
        let repo = spec.source.snapshot.repository_id.clone();
        let job = submit(
            &mut SqliteKnowledgeRepository::open(&store).unwrap(),
            spec,
            &name,
            "agent",
        )
        .unwrap();
        let mut item = run_bounded(&store, &peer, job.id.as_str(), "parse", 1, 8, None).unwrap();
        assert_eq!(item.checkpoint["inventory"]["reused_files"], parse);
        for (phase, counter, expected) in [
            ("syntax", "reused_extraction_files", parse),
            ("calls", "reused_stitching_files", calls),
        ] {
            resume(&store, job.id.as_str());
            item = run_bounded(&store, &peer, job.id.as_str(), phase, 1, 8, None).unwrap();
            assert_eq!(
                item.checkpoint["inventory"][counter], expected,
                "round {round}: {phase}"
            );
        }
        resume(&store, job.id.as_str());
        let done = run(&store, &peer, job.id.as_str(), "publish", 2).unwrap();
        assert_eq!(done.state, OperationState::Completed);
        let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
        let graph = db
            .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
            .unwrap();
        let expected = entmoot_engine::build_topology_from_repo_with_options(
            &peer,
            entmoot_engine::TopologyBuildOptions {
                repository: Some(&repo),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(inventory_value(graph), inventory_value(expected));
        assert_eq!(
            db.retained_graph(&original).unwrap().snapshot.repository_id,
            REPOSITORY
        );
    }
}

#[test]
fn failed_cache_publication_rolls_back_graph_seal_and_preserves_complete_frontiers() {
    let (_tmp, root, store) = fixture();
    let base = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        request(&root, "base"),
        "base",
        "agent",
    )
    .unwrap();
    assert_eq!(
        run(&store, &root, base.id.as_str(), "base", 5)
            .unwrap()
            .state,
        OperationState::Completed
    );
    let connection = rusqlite::Connection::open(&store).unwrap();
    let old_digest: String = connection
        .query_row("SELECT digest FROM syntax_cache_header", [], |r| r.get(0))
        .unwrap();
    std::fs::write(root.join("new.py"), "def addition(): return 1\n").unwrap();
    let job = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        request(&root, "topic"),
        "topic",
        "agent",
    )
    .unwrap();
    let ready = run_bounded(&store, &root, job.id.as_str(), "frontiers", 3, 8, None).unwrap();
    assert_eq!(ready.checkpoint["inventory"]["stitched_files"], 2);
    connection.execute_batch("CREATE TRIGGER fail_cache BEFORE INSERT ON file_syntax_cache WHEN NEW.path='new.py' BEGIN SELECT RAISE(ABORT,'cache write failed'); END;").unwrap();
    resume(&store, job.id.as_str());
    assert!(run(&store, &root, job.id.as_str(), "failed", 1).is_err());
    assert_eq!(
        connection
            .query_row("SELECT digest FROM syntax_cache_header", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        old_digest
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM local_inventory_parts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM local_view_seals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        SqliteKnowledgeRepository::open_operations_read_only(&store)
            .unwrap()
            .workspaces(REPOSITORY)
            .unwrap()
            .len(),
        1
    );
    connection.execute_batch("DROP TRIGGER fail_cache; CREATE TRIGGER fail_replay BEFORE INSERT ON local_inventory_parts BEGIN SELECT RAISE(ABORT,'completed frontier replayed'); END;").unwrap();
    resume(&store, job.id.as_str());
    assert_eq!(
        run(&store, &root, job.id.as_str(), "restored", 2)
            .unwrap()
            .state,
        OperationState::Completed
    );
    assert_ne!(
        connection
            .query_row("SELECT digest FROM syntax_cache_header", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        old_digest
    );
}

#[test]
fn inventory_restart_keeps_prepared_output_private_and_matches_uninterrupted() {
    let (_tmp, root, store) = fixture();
    let spec = request(&root, "topic");
    let job = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        spec.clone(),
        "index",
        "agent",
    )
    .unwrap();
    let paused = run(&store, &root, job.id.as_str(), "first-worker", 4).unwrap();
    assert_eq!(paused.state, OperationState::Paused);
    let staged_view = paused.checkpoint["view"].as_str().unwrap();
    {
        let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
        assert!(db.workspaces(REPOSITORY).unwrap().is_empty());
        assert!(
            db.retained_graph(staged_view).is_err(),
            "staging is not public evidence"
        );
    }
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    let done = run(&store, &root, job.id.as_str(), "new-process", 1).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.result.as_ref().unwrap()["view"], staged_view);
    let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
    let graph = db.retained_graph(staged_view).unwrap();
    let names: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == entmoot_engine::NodeKind::Callable)
        .map(|n| n.label.as_str())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"target") && names.contains(&"caller"));
    let independent = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(inventory_value(graph), inventory_value(independent));
    let serial_store = store.with_file_name("serial.sqlite3");
    let serial = submit(
        &mut SqliteKnowledgeRepository::open(&serial_store).unwrap(),
        spec,
        "index",
        "agent",
    )
    .unwrap();
    let serial = run(&serial_store, &root, serial.id.as_str(), "worker", 5).unwrap();
    assert_eq!(serial.result, done.result);
    // A lost final acknowledgement can be replayed without even needing source files.
    assert_eq!(
        run(&store, &root.join("missing"), job.id.as_str(), "retry", 5).unwrap(),
        done
    );
}

#[test]
fn source_drift_new_paths_and_ignore_changes_never_publish_a_prepared_result() {
    for (name, bytes) in [
        ("app.py", "def changed(): pass\n"),
        ("new.txt", "new input"),
        (".gitignore", "app.py\n"),
    ] {
        let (_tmp, root, store) = fixture();
        let job = submit(
            &mut SqliteKnowledgeRepository::open(&store).unwrap(),
            request(&root, "topic"),
            "index",
            "agent",
        )
        .unwrap();
        run(&store, &root, job.id.as_str(), "worker", 4).unwrap();
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .control_operation(job.id.as_str(), OperationControl::Resume)
            .unwrap();
        std::fs::write(root.join(name), bytes).unwrap();
        assert!(run(&store, &root, job.id.as_str(), "new-worker", 5)
            .unwrap_err()
            .to_string()
            .contains("source changed"));
        assert!(SqliteKnowledgeRepository::open_operations_read_only(&store)
            .unwrap()
            .workspaces(REPOSITORY)
            .unwrap()
            .is_empty());
    }
}

#[test]
fn publication_cas_failure_keeps_the_job_and_previous_registry_atomic() {
    let (_tmp, root, store) = fixture();
    let job = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        request(&root, "topic"),
        "index",
        "agent",
    )
    .unwrap();
    run(&store, &root, job.id.as_str(), "worker", 4).unwrap();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    let competitor =
        collaboration::register_graph(&store, &graph, "topic", "different-agent", None, 0).unwrap();
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    assert!(run(&store, &root, job.id.as_str(), "worker", 1)
        .unwrap_err()
        .to_string()
        .contains("generation changed"));
    let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
    let failed = db.operation(job.id.as_str()).unwrap().unwrap();
    assert_eq!(failed.state, OperationState::Paused);
    assert!(failed.result.is_none());
    assert_eq!(db.workspaces(REPOSITORY).unwrap(), vec![competitor]);
}

#[test]
fn cancelled_output_cannot_be_published_by_a_resumed_worker() {
    let (_tmp, root, store) = fixture();
    let job = submit(
        &mut SqliteKnowledgeRepository::open(&store).unwrap(),
        request(&root, "topic"),
        "index",
        "agent",
    )
    .unwrap();
    run(&store, &root, job.id.as_str(), "worker", 4).unwrap();
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .control_operation(job.id.as_str(), OperationControl::Cancel)
        .unwrap();
    assert!(run(&store, &root, job.id.as_str(), "worker", 1).is_err());
    assert!(SqliteKnowledgeRepository::open_operations_read_only(&store)
        .unwrap()
        .workspaces(REPOSITORY)
        .unwrap()
        .is_empty());
}

#[test]
fn canon_restart_preserves_structural_ids_and_has_attributable_claims() {
    let (_tmp, root, store) = fixture();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let job = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
    run(&store, &root, job.id.as_str(), "worker", 5).unwrap();
    let registered = db.workspaces(REPOSITORY).unwrap().remove(0);
    let before = db.retained_graph(registered.view.as_str()).unwrap();
    let mut spec = request(&root, "topic");
    spec.publication.expected_generation = 1;
    spec.task = ProducerTask::Canon {
        view: registered.view.clone(),
    };
    let job = submit(&mut db, spec, "canon", "agent").unwrap();
    let staged = run(&store, &root, job.id.as_str(), "worker", 1).unwrap();
    assert_eq!(db.workspaces(REPOSITORY).unwrap(), vec![registered]);
    db.control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    drop(db);
    let done = run(&store, &root, job.id.as_str(), "next-worker", 1).unwrap();
    assert_eq!(done.result.as_ref().unwrap()["generation"], 2);
    let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
    let after = db
        .retained_graph(staged.checkpoint["view"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(&before.nodes).unwrap(),
        serde_json::to_value(&after.nodes).unwrap()
    );
    assert!(after
        .relation_claims
        .iter()
        .any(|c| c.predicate == "canon.file"));
    assert!(after
        .relation_claims
        .iter()
        .any(|c| c.predicate == "canon.graph"));
}

#[test]
fn concurrent_identical_registration_is_one_atomic_idempotent_publication() {
    let (_tmp, root, store) = fixture();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    let barrier = std::sync::Barrier::new(6);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let (barrier, store, graph) = (&barrier, &store, &graph);
                scope.spawn(move || {
                    barrier.wait();
                    collaboration::register_graph(store, graph, "topic", "agent", None, 0)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(results.iter().all(|r| *r == results[0]));
    let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
    assert_eq!(db.workspaces(REPOSITORY).unwrap().len(), 1);
    assert_eq!(
        inventory_value(db.retained_graph(results[0].view.as_str()).unwrap()),
        inventory_value(graph)
    );
}

#[test]
fn request_collision_and_invalid_source_identity_are_rejected_before_work() {
    let (_tmp, root, store) = fixture();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let spec = request(&root, "topic");
    let job = submit(&mut db, spec.clone(), "key", "agent").unwrap();
    assert_eq!(submit(&mut db, spec, "key", "agent").unwrap(), job);
    assert!(submit(&mut db, request(&root, "other"), "key", "agent").is_err());
    assert!(run(&store, &root, job.id.as_str(), "worker", 0).is_err());
    assert!(collaboration::write(
        &store,
        entmoot_contracts::CollaborationWrite::RunProducer {
            id: job.id,
            worker: "worker".into(),
            steps: 2,
            files_per_step: 128,
            wall_time_ms: None,
        }
    )
    .is_err());
    assert!(db.workspaces(REPOSITORY).unwrap().is_empty());
}

#[test]
fn failed_registration_rolls_back_the_new_inventory_and_knowledge() {
    use entmoot_engine::KnowledgeRead;
    let (_tmp, root, store) = fixture();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let job = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
    let done = run(&store, &root, job.id.as_str(), "worker", 5).unwrap();
    std::fs::write(root.join("app.py"), "def replacement(): return 12\n").unwrap();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    let digest = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph).unwrap());
    assert!(collaboration::register_graph(&store, &graph, "topic", "agent", None, 0).is_err());
    assert_eq!(
        serde_json::to_value(db.workspaces(REPOSITORY).unwrap()).unwrap(),
        serde_json::json!([done.result.unwrap()])
    );
    assert!(db.retained_graph(&digest).is_err());
    // No half-imported knowledge survives the failed registry transaction.
    let orphan = SqliteKnowledgeRepository::open_view(&store, &digest).unwrap();
    assert!(!orphan.is_initialized().unwrap());
    assert!(orphan.load_inventory(false).unwrap().is_none());
}

#[test]
fn simultaneous_producers_share_a_result_without_publishing_each_others_workspace() {
    let (_tmp, root, store) = fixture();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let first = submit(&mut db, request(&root, "alice"), "alice", "agent").unwrap();
    let second = submit(&mut db, request(&root, "bob"), "bob", "agent").unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = [&first, &second]
            .into_iter()
            .map(|job| {
                let (barrier, store, root) = (&barrier, &store, &root);
                scope.spawn(move || {
                    barrier.wait();
                    run(store, root, job.id.as_str(), &job.spec.owner, 5)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(results.iter().all(|r| r.state == OperationState::Completed));
    let registered = db.workspaces(REPOSITORY).unwrap();
    assert_eq!(
        registered
            .iter()
            .map(|p| p.workspace_id.as_str())
            .collect::<Vec<_>>(),
        vec!["alice", "bob"]
    );
    assert_eq!(registered[0].view, registered[1].view);
    assert!(registered.iter().all(|p| p.generation == 1));
}

#[test]
fn producer_retains_working_tree_source_even_if_an_equal_import_was_already_sealed() {
    let (_tmp, root, store) = fixture();
    let original = std::fs::read(root.join("app.py")).unwrap();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    collaboration::register_graph(&store, &graph, "imported", "agent", None, 0).unwrap();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let job = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
    let done = run(&store, &root, job.id.as_str(), "worker", 5).unwrap();
    let publication: entmoot_contracts::WorkspacePublication =
        serde_json::from_value(done.result.unwrap()).unwrap();
    std::fs::write(root.join("app.py"), "def changed(): return 90\n").unwrap();
    let retained =
        SqliteKnowledgeRepository::open_view_read_only(&store, publication.view.as_str()).unwrap();
    let bytes = retained
        .retained_source(entmoot_engine::SourceReadRequest {
            snapshot: &graph.snapshot,
            path: "app.py",
            expected_content_digest: &graph.snapshot_files[0].content_digest,
        })
        .unwrap()
        .unwrap();
    assert_eq!(bytes.bytes, original);
}

#[test]
fn bounded_file_restarts_match_full_inventory_across_languages_and_skip_outcomes() {
    let (_tmp, root, store) = fixture();
    let sources = [
        ("00.rs", "fn leaf()->i32 {7} fn caller()->i32 {leaf()}"),
        ("01.py", "def leaf():\n    return 7\n"),
        ("02.js", "export const leaf = () => 7;"),
        (
            "03.ts",
            "export function leaf(x: number): number { return x + 7; }",
        ),
        (
            "04.tsx",
            "export function Leaf() {return <div>seven</div>;}",
        ),
        ("05.go", "package leaf\nfunc Leaf() int { return 7 }"),
        ("06.java", "class Leaf { static int leaf() { return 7; } }"),
        ("07.cs", "class Leaf { static int Get() { return 7; } }"),
        ("08.c", "int leaf(void) { return 7; }"),
        ("09.txt", "unsupported documentation\n"),
    ];
    for (path, source) in sources {
        std::fs::write(root.join(path), source.replace('\n', "\r\n")).unwrap();
    }
    std::fs::write(root.join("10.py"), [0xff, 0xfe, 0xff]).unwrap();
    // 11 added files plus the fixture's app.py; no budget changes the denominator.
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let job = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
    drop(db);
    for completed in [3, 6, 9] {
        let partial = run_bounded(&store, &root, job.id.as_str(), "worker", 1, 3, None).unwrap();
        assert_eq!(
            partial.checkpoint,
            serde_json::json!({"inventory":{"completed_files":completed,"total_files":12,"reused_files":0}})
        );
        assert_eq!(partial.state, OperationState::Paused);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        assert!(db.workspaces(REPOSITORY).unwrap().is_empty());
        db.control_operation(job.id.as_str(), OperationControl::Resume)
            .unwrap();
    }
    let parsed = run_bounded(&store, &root, job.id.as_str(), "worker", 1, 3, None).unwrap();
    assert_eq!(parsed.checkpoint["inventory"]["completed_files"], 12);
    for phase in ["syntax_files", "stitched_files"] {
        for completed in [3, 6, 9, 12] {
            let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
            db.control_operation(job.id.as_str(), OperationControl::Resume)
                .unwrap();
            drop(db);
            let partial =
                run_bounded(&store, &root, job.id.as_str(), "replacement", 1, 3, None).unwrap();
            assert_eq!(partial.checkpoint["inventory"][phase], completed);
            assert!(partial.checkpoint.get("view").is_none());
            assert!(SqliteKnowledgeRepository::open_operations_read_only(&store)
                .unwrap()
                .workspaces(REPOSITORY)
                .unwrap()
                .is_empty());
        }
    }
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    let prepared = run_bounded(&store, &root, job.id.as_str(), "worker", 1, 3, None).unwrap();
    assert!(prepared.checkpoint["view"].is_string());
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    db.control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    let done = run_bounded(&store, &root, job.id.as_str(), "last-worker", 1, 1, None).unwrap();
    let published = db
        .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
        .unwrap();
    let measurement = serde_json::to_value(&published.coverages[0].measurement).unwrap();
    assert_eq!(measurement["observed_files"], 12);
    assert_eq!(measurement["parsed"], 10);
    assert_eq!(measurement["unsupported"], 1);
    assert_eq!(measurement["failed"], 1);
    let expected = entmoot_engine::build_topology_from_repo_with_options(
        &root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some(REPOSITORY),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(inventory_value(published), inventory_value(expected));
}

#[test]
fn zero_wall_budget_and_source_drift_preserve_partial_progress() {
    // Two files: stop during parsing, syntax extraction, and call stitching.
    for steps in [1, 3, 5] {
        let (_tmp, root, store) = fixture();
        std::fs::write(root.join("00.rs"), "fn leaf() {}\n").unwrap();
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let job = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
        let stopped =
            run_bounded(&store, &root, job.id.as_str(), "worker", 100, 1, Some(0)).unwrap();
        assert_eq!(stopped.checkpoint["inventory"]["completed_files"], 0);
        db.control_operation(job.id.as_str(), OperationControl::Resume)
            .unwrap();
        let partial =
            run_bounded(&store, &root, job.id.as_str(), "worker", steps, 1, None).unwrap();
        assert_eq!(
            partial.checkpoint["inventory"]["completed_files"],
            steps.min(2)
        );
        db.control_operation(job.id.as_str(), OperationControl::Resume)
            .unwrap();
        let stopped =
            run_bounded(&store, &root, job.id.as_str(), "worker", 100, 1, Some(0)).unwrap();
        assert_eq!(stopped.checkpoint, partial.checkpoint);
        std::fs::write(root.join("00.rs"), "fn changed() {}\n").unwrap();
        db.control_operation(job.id.as_str(), OperationControl::Resume)
            .unwrap();
        assert!(run_bounded(&store, &root, job.id.as_str(), "worker", 100, 1, None).is_err());
        assert_eq!(
            db.operation(job.id.as_str()).unwrap().unwrap().checkpoint,
            partial.checkpoint
        );
        assert!(db.workspaces(REPOSITORY).unwrap().is_empty());
    }
}

#[test]
fn empty_and_unparseable_manifests_never_invent_parsed_files() {
    for with_skips in [false, true] {
        let (_tmp, root, store) = fixture();
        std::fs::remove_file(root.join("app.py")).unwrap();
        if with_skips {
            std::fs::write(root.join("notes.txt"), "unsupported source\n").unwrap();
            std::fs::write(root.join("broken.py"), [0xff]).unwrap();
        }
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let job = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
        let done = run_bounded(&store, &root, job.id.as_str(), "worker", 8, 1, None).unwrap();
        assert_eq!(done.state, OperationState::Completed);
        let graph = db
            .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
            .unwrap();
        assert!(!graph
            .nodes
            .iter()
            .any(|n| n.kind == entmoot_engine::NodeKind::Callable));
        let measurement = serde_json::to_value(&graph.coverages[0].measurement).unwrap();
        assert_eq!(
            measurement["observed_files"],
            if with_skips { 2 } else { 0 }
        );
        assert_eq!(measurement["parsed"], 0);
        assert_eq!(measurement["failed"], usize::from(with_skips));
        assert_eq!(measurement["unsupported"], usize::from(with_skips));
    }
}

#[test]
fn canon_file_restarts_match_uninterrupted_claims_and_keep_partial_folds_private() {
    let (_tmp, root, store) = fixture();
    std::fs::create_dir(root.join("nested")).unwrap();
    for (path, source) in [
        ("a.rs", "fn first() -> bool {true}"),
        ("nested/b.py", "def second():\n    return True\n"),
        ("nested/c.ts", "export function third() {return true;}"),
    ] {
        std::fs::write(root.join(path), source).unwrap();
    }
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let inventory = submit(&mut db, request(&root, "topic"), "index", "agent").unwrap();
    let indexed = run(&store, &root, inventory.id.as_str(), "inventory", 5).unwrap();
    let before = db
        .retained_graph(indexed.result.as_ref().unwrap()["view"].as_str().unwrap())
        .unwrap();
    let publication = db.workspaces(REPOSITORY).unwrap()[0].clone();
    let mut spec = request(&root, "topic");
    spec.publication.expected_generation = 1;
    spec.task = ProducerTask::Canon {
        view: publication.view.clone(),
    };
    let canon = submit(&mut db, spec, "canon", "agent").unwrap();
    let zero = run_bounded(&store, &root, canon.id.as_str(), "zero", 10, 1, Some(0)).unwrap();
    assert_eq!(
        zero.checkpoint["canon"],
        serde_json::json!({"completed_files":0,"total_files":4})
    );
    for completed in 1..4 {
        db.control_operation(canon.id.as_str(), OperationControl::Resume)
            .unwrap();
        let partial = run_bounded(&store, &root, canon.id.as_str(), "fold", 1, 1, None).unwrap();
        assert_eq!(partial.checkpoint["canon"]["completed_files"], completed);
        assert_eq!(
            db.workspaces(REPOSITORY).unwrap().as_slice(),
            std::slice::from_ref(&publication)
        );
        assert!(db
            .operation_outputs(canon.id.as_str(), 0, 10)
            .unwrap()
            .outputs
            .is_empty());
        assert_eq!(
            db.retained_graph(publication.view.as_str())
                .unwrap()
                .relation_claims,
            before.relation_claims
        );
        drop(db);
        db = SqliteKnowledgeRepository::open(&store).unwrap();
    }
    db.control_operation(canon.id.as_str(), OperationControl::Resume)
        .unwrap();
    let done = run_bounded(&store, &root, canon.id.as_str(), "finish", 2, 1, None).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let after = db
        .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
        .unwrap();
    let mut expected = before.clone();
    let previous =
        SqliteKnowledgeRepository::open_view_read_only(&store, publication.view.as_str()).unwrap();
    entmoot_engine::canon::produce(&mut expected, &root, &[], &[], Some(&previous)).unwrap();
    assert_eq!(inventory_value(after.clone()), inventory_value(expected));
    assert_eq!(
        serde_json::to_value(&after.nodes).unwrap(),
        serde_json::to_value(&before.nodes).unwrap()
    );
    assert_eq!(
        after
            .relation_claims
            .iter()
            .filter(|c| c.predicate == "canon.file")
            .count(),
        4
    );
}

#[test]
fn configured_three_worktree_inventory_matches_serial_views_without_cross_publication() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("concurrent.db");
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    db.configure_scheduler(
        REPOSITORY,
        0,
        entmoot_contracts::SchedulerPolicy {
            max_running: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let branches: Vec<_> = ["alice", "bob", "carol"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let root = temp.path().join(name);
            std::fs::create_dir(&root).unwrap();
            std::fs::write(
                root.join("app.py"),
                format!("from .target import leaf\ndef {name}():\n    return leaf() + {i}\n"),
            )
            .unwrap();
            std::fs::write(
                root.join("target.py"),
                format!("def leaf():\n    return {i}\n"),
            )
            .unwrap();
            let spec = request(&root, name);
            let job = submit(&mut db, spec.clone(), name, name).unwrap();
            (root, spec, job)
        })
        .collect();
    let barrier = std::sync::Barrier::new(3);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = branches
            .iter()
            .map(|(root, _, job)| {
                let (barrier, store) = (&barrier, &store);
                scope.spawn(move || {
                    barrier.wait();
                    run_bounded(store, root, job.id.as_str(), &job.spec.owner, 20, 1, None).unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let serial_store = temp.path().join("serial.db");
    let mut serial_db = SqliteKnowledgeRepository::open(&serial_store).unwrap();
    for ((root, spec, submitted), done) in branches.iter().zip(&results) {
        assert_eq!(done.state, OperationState::Completed);
        let serial = submit(
            &mut serial_db,
            spec.clone(),
            &submitted.spec.idempotency_key,
            &submitted.spec.owner,
        )
        .unwrap();
        let serial = run_bounded(
            &serial_store,
            root,
            serial.id.as_str(),
            "serial",
            20,
            1,
            None,
        )
        .unwrap();
        assert_eq!(done.result, serial.result);
        let view = done.result.as_ref().unwrap()["view"].as_str().unwrap();
        let graph = db.retained_graph(view).unwrap();
        assert_eq!(
            inventory_value(graph.clone()),
            inventory_value(serial_db.retained_graph(view).unwrap())
        );
        let caller = graph
            .nodes
            .iter()
            .find(|n| n.label == submitted.spec.owner)
            .unwrap();
        let leaf = graph.nodes.iter().find(|n| n.label == "leaf").unwrap();
        assert!(graph
            .edges
            .iter()
            .any(|e| e.resolved && e.from == caller.id && e.to == leaf.id));
        for other in ["alice", "bob", "carol"]
            .into_iter()
            .filter(|n| *n != submitted.spec.owner)
        {
            assert!(graph.nodes.iter().all(|n| n.label != other));
        }
    }
    assert_eq!(db.workspaces(REPOSITORY).unwrap().len(), 3);
    assert!(db
        .scheduler_status(REPOSITORY, u64::MAX / 2)
        .unwrap()
        .active
        .is_empty());
}

#[test]
fn concurrent_canon_jobs_keep_independent_cursors_and_publish_equivalent_results() {
    let (_tmp, root, store) = fixture();
    std::fs::write(root.join("second.rs"), "fn second() -> bool { true }").unwrap();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    db.configure_scheduler(
        REPOSITORY,
        0,
        entmoot_contracts::SchedulerPolicy {
            max_running: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let inventory = submit(&mut db, request(&root, "input"), "index", "agent").unwrap();
    run(&store, &root, inventory.id.as_str(), "index", 5).unwrap();
    let input = db.workspaces(REPOSITORY).unwrap()[0].clone();
    let input_claims = db
        .retained_graph(input.view.as_str())
        .unwrap()
        .relation_claims;
    assert!(input_claims
        .iter()
        .all(|c| c.producer.id != "entmoot.canon"));
    let jobs: Vec<_> = ["alice", "bob", "carol"]
        .into_iter()
        .map(|name| {
            let mut spec = request(&root, name);
            spec.task = ProducerTask::Canon {
                view: input.view.clone(),
            };
            submit(&mut db, spec, name, name).unwrap()
        })
        .collect();
    let barrier = std::sync::Barrier::new(3);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|job| {
                let (barrier, store, root) = (&barrier, &store, &root);
                scope.spawn(move || {
                    barrier.wait();
                    run_bounded(store, root, job.id.as_str(), &job.spec.owner, 3, 1, None).unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(results.iter().all(|r| r.state == OperationState::Completed));
    assert_eq!(
        results[0].result.as_ref().unwrap()["view"],
        results[1].result.as_ref().unwrap()["view"]
    );
    assert_eq!(
        results[0].result.as_ref().unwrap()["view"],
        results[2].result.as_ref().unwrap()["view"]
    );
    assert_eq!(
        db.workspaces(REPOSITORY)
            .unwrap()
            .iter()
            .map(|w| w.workspace_id.as_str())
            .collect::<Vec<_>>(),
        ["alice", "bob", "carol", "input"]
    );
    assert_eq!(
        db.retained_graph(input.view.as_str())
            .unwrap()
            .relation_claims,
        input_claims
    );
}
