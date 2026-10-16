//! Real source inventories, independently authored branch outcomes.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::{
    ConflictBatchRequest, ConflictFindingKind, OperationControl, OperationState,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, collaboration::*, LocalOperations,
    SqliteKnowledgeRepository, TopologyBuildOptions, TopologyGraph,
};

#[path = "branch_conflicts/git_capture.rs"]
mod git_capture;

fn graph(root: &std::path::Path, source: &str) -> TopologyGraph {
    std::fs::write(root.join("app.py"), source).unwrap();
    build_topology_from_repo_with_options(
        root,
        TopologyBuildOptions {
            repository: Some("fixture/branches"),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn different_edits_to_one_occurrence_are_overlap_not_a_verdict() {
    let temp = tempfile::tempdir().unwrap();
    let base = graph(temp.path(), "def add(x):\n    return x + 1\n");
    let left = graph(temp.path(), "def add(x):\n    return x + 2\n");
    let right = graph(temp.path(), "def add(x):\n    return x + 3\n");
    let report = compare_branch_conflicts(&base, &left, &right).unwrap();
    assert!(report
        .findings
        .iter()
        .any(|f| f.kind == ConflictFindingKind::StructuralOverlap && !f.subjects.is_empty()));
    assert!(report
        .limitations
        .iter()
        .any(|l| l.contains("not semantic")));
    let unchanged = compare_branch_conflicts(&base, &left, &base).unwrap();
    assert!(unchanged.findings.is_empty());
    assert!(!unchanged.limitations.is_empty());
}

#[test]
fn registry_restart_cas_and_frozen_conflict_batch() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let store = temp.path().join("shared.db");
    let base = graph(&source, "def add(x):\n    return x + 1\n");
    let left = graph(&source, "def add(x):\n    return x + 2\n");
    let right = graph(&source, "def add(x):\n    return x + 3\n");
    let base_row = register_graph(&store, &base, "base", "agent", Some("main".into()), 0).unwrap();
    let left_row = register_graph(&store, &left, "left", "agent", Some("topic".into()), 0).unwrap();
    let right_row =
        register_graph(&store, &right, "right", "agent", Some("peer".into()), 0).unwrap();
    assert_eq!(
        register_graph(&store, &right, "right", "agent", Some("peer".into()), 0).unwrap(),
        right_row
    );
    assert!(register_graph(&store, &left, "right", "agent", None, 0).is_err());
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let request = ConflictBatchRequest {
        git_merge: false,
        base_view: base_row.view.clone(),
        left: left_row,
        peers: vec![right_row, base_row],
    };
    let job = submit_conflicts(&mut db, request, "comparison", "agent").unwrap();
    let first = run_conflicts(&mut db, job.id.as_str(), "worker", 1).unwrap();
    assert_eq!(first.state, OperationState::Paused);
    assert_eq!(first.checkpoint, serde_json::json!(1));
    drop(db);
    // New peer publication must not change the already accepted job's inputs.
    register_graph(&store, &left, "right", "agent", Some("peer".into()), 1).unwrap();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    db.control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    let done = run_conflicts(&mut db, job.id.as_str(), "worker", 1).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let reports: Vec<entmoot_contracts::BranchConflictReport> = serde_json::from_value(
        serde_json::to_value(
            db.operation_outputs(job.id.as_str(), 0, 100)
                .unwrap()
                .outputs
                .into_iter()
                .map(|o| o.value)
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(reports[0].right, right.snapshot);
    assert_eq!(
        reports[0],
        compare_branch_conflicts(&base, &left, &right).unwrap()
    );
    assert!(reports[1].findings.is_empty());
    assert_eq!(db.workspaces("fixture/branches").unwrap().len(), 3);
    assert!(db.workspaces("other").unwrap().is_empty());
    let retired = db.retire_workspace("right", 2).unwrap();
    assert!(!retired.active);
    assert_eq!(retired.generation, 3);
    assert_eq!(db.retire_workspace("right", 2).unwrap(), retired);
    assert!(register_graph(&store, &right, "right", "agent", Some("peer".into()), 0).is_err());
    assert!(
        register_graph(&store, &right, "right", "agent", Some("peer".into()), 3)
            .unwrap()
            .active
    );
}

#[test]
fn rejects_cross_repository_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let base = graph(temp.path(), "def f():\n    return 1\n");
    let mut other = base.clone();
    other.snapshot.repository_id = "other".into();
    assert!(compare_branch_conflicts(&base, &base, &other).is_err());
}

#[test]
fn conflict_worker_rejects_low_level_jobs_with_missing_pins_or_wrong_repository() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    std::fs::create_dir(&root).unwrap();
    let store = temp.path().join("shared.db");
    let graph = graph(&root, "def f(): return 1\n");
    let row = register_graph(&store, &graph, "w", "agent", None, 0).unwrap();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let valid = submit_conflicts(
        &mut db,
        ConflictBatchRequest {
            git_merge: false,
            base_view: row.view.clone(),
            left: row.clone(),
            peers: vec![row],
        },
        "valid",
        "agent",
    )
    .unwrap();
    for wrong_repo in [false, true] {
        let mut spec = valid.spec.clone();
        spec.idempotency_key = format!("malformed-{wrong_repo}");
        spec.views.clear();
        if wrong_repo {
            spec.repository_id = "another/repository".into();
        }
        let item = db.submit_operations(&[spec], 100).unwrap().remove(0);
        let result = run_conflicts(&mut db, item.id.as_str(), "worker", 1);
        assert!(
            result.is_err(),
            "unfenced worker accepted {wrong_repo}: {result:?}"
        );
        let saved = db.operation(item.id.as_str()).unwrap().unwrap();
        assert_eq!(saved.state, OperationState::Queued);
        assert_eq!(saved.generation, 0);
    }
}

#[test]
fn independent_occurrences_stay_distinct_across_languages_and_source_encodings() {
    for (language, file, source) in [
        ("rust", "app.rs", "fn a()->i32 { 11 } fn b()->i32 { 22 }"),
        (
            "python",
            "app.py",
            "def a():\n    return 11\ndef b():\n    return 22\n",
        ),
        (
            "javascript",
            "app.js",
            "function a(){return 11;} function b(){return 22;}",
        ),
        (
            "typescript",
            "app.ts",
            "function a():number{return 11;} function b():number{return 22;}",
        ),
        (
            "tsx",
            "app.tsx",
            "function a():number{return 11;} function b():number{return 22;}",
        ),
        (
            "go",
            "app.go",
            "package p\nfunc a() int {return 11}\nfunc b() int {return 22}\n",
        ),
        (
            "java",
            "App.java",
            "class App { int a(){return 11;} int b(){return 22;} }",
        ),
        (
            "csharp",
            "App.cs",
            "class App { int A(){return 11;} int B(){return 22;} }",
        ),
        ("c", "app.c", "int a(){return 11;} int b(){return 22;}"),
    ] {
        for crlf in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let source = if crlf {
                source.replace('\n', "\r\n")
            } else {
                source.to_owned()
            };
            let build = |text: &str| {
                std::fs::write(temp.path().join(file), text).unwrap();
                build_topology_from_repo_with_options(
                    temp.path(),
                    TopologyBuildOptions {
                        repository: Some("fixture/matrix"),
                        ..Default::default()
                    },
                )
                .unwrap()
            };
            let base = build(&source);
            let left = build(&source.replace("11", "33"));
            let right = build(&source.replace("22", "44"));
            let report = compare_branch_conflicts(&base, &left, &right).unwrap();
            assert!(
                report
                    .findings
                    .iter()
                    .any(|f| f.path.as_deref() == Some(file)),
                "{language}"
            );
            assert!(
                !report.findings.iter().any(|f| !f.subjects.is_empty()),
                "distinct occurrences conflated: {language}"
            );
            let same = compare_branch_conflicts(&base, &left, &left).unwrap();
            assert!(
                same.findings.iter().any(|f| !f.subjects.is_empty()),
                "changed occurrence missing: {language}"
            );
        }
    }
}

#[test]
fn git_conflict_observation_isolated_from_refs_index_and_custom_drivers() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    std::fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let result = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8_lossy(&result.stdout).trim().to_owned()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("app.py"), "value = 1\n").unwrap();
    std::fs::write(root.join(".gitattributes"), "*.py merge=custom\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "left"]);
    std::fs::write(root.join("app.py"), "value = 2\n").unwrap();
    git(&["commit", "-qam", "left"]);
    let left = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "right", &base]);
    std::fs::write(root.join("app.py"), "value = 3\n").unwrap();
    git(&["commit", "-qam", "right"]);
    let right = git(&["rev-parse", "HEAD"]);
    // A configured success-only driver would hide the textual conflict if inherited.
    git(&["config", "merge.custom.driver", "true"]);
    let refs = git(&["show-ref"]);
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let snapshot = |revision: String| entmoot_contracts::SourceSnapshot {
        repository_id: "fixture/git".into(),
        revision,
        source_state: entmoot_contracts::SourceState::Committed,
        snapshot_digest: None,
        estate_id: None,
        subject: None,
        subject_aliases: Vec::new(),
    };
    let observation = entmoot_engine::collaboration_git::observe_merge(
        &root,
        &temp.path().join("scratch"),
        &snapshot(left),
        &snapshot(right),
    )
    .unwrap();
    assert_eq!(observation.status, "conflict");
    assert_eq!(observation.merge_bases, vec![base]);
    assert_eq!(git(&["show-ref"]), refs);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    assert!(git(&["status", "--porcelain"]).is_empty());
}

#[test]
fn registered_evidence_is_immutable_even_through_existing_mutation_apis() {
    use entmoot_engine::{KnowledgeRepository, KnowledgeStore};
    let source = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("shared.db");
    let original = graph(source.path(), "def f():\n    return 1\n");
    let row = register_graph(&store, &original, "work", "agent", None, 0).unwrap();
    let mut db = SqliteKnowledgeRepository::open_view(&store, row.view.as_str()).unwrap();
    assert!(db.replace_all(&KnowledgeStore::default()).is_err());
    assert!(db.synchronize(&KnowledgeStore::default()).is_err());
    let changed = graph(source.path(), "def f():\n    return 2\n");
    assert!(db.store_inventory(&changed, None).is_err());
    assert_eq!(
        db.load_inventory(false).unwrap().unwrap().snapshot,
        original.snapshot
    );
    drop(db);
    assert_eq!(
        register_graph(&store, &original, "work", "agent", None, 0).unwrap(),
        row
    );
    let next = register_graph(&store, &changed, "work", "agent", None, 1).unwrap();
    assert_ne!(next.view, row.view);
}

#[test]
fn simultaneous_branch_jobs_match_serial_reports_with_slice_restarts() {
    use std::sync::{Arc, Barrier};
    let source = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("shared.db");
    let base = graph(source.path(), "def f():\n    return 1\n");
    let left = graph(source.path(), "def f():\n    return 2\n");
    let right = graph(source.path(), "def f():\n    return 3\n");
    let b = register_graph(&store, &base, "base", "a", None, 0).unwrap();
    let l = register_graph(&store, &left, "left", "a", None, 0).unwrap();
    let r = register_graph(&store, &right, "right", "a", None, 0).unwrap();
    let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
    let mut jobs = vec![];
    for (key, left, peer) in [("lr", l.clone(), r.clone()), ("rl", r, l)] {
        jobs.push(
            submit_conflicts(
                &mut db,
                ConflictBatchRequest {
                    git_merge: false,
                    base_view: b.view.clone(),
                    left,
                    peers: vec![peer, b.clone()],
                },
                key,
                "a",
            )
            .unwrap(),
        );
    }
    drop(db);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = jobs
        .into_iter()
        .map(|job| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
                assert_eq!(
                    run_conflicts(&mut db, job.id.as_str(), "worker", 1)
                        .unwrap()
                        .state,
                    OperationState::Paused
                );
                drop(db);
                let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
                db.control_operation(job.id.as_str(), OperationControl::Resume)
                    .unwrap();
                assert_eq!(
                    run_conflicts(&mut db, job.id.as_str(), "worker", 1)
                        .unwrap()
                        .state,
                    OperationState::Completed
                );
                db.operation_outputs(job.id.as_str(), 0, 10).unwrap()
            })
        })
        .collect();
    let expected = [
        compare_branch_conflicts(&base, &left, &right).unwrap(),
        compare_branch_conflicts(&base, &right, &left).unwrap(),
    ];
    for (h, expected) in handles.into_iter().zip(expected) {
        let page = h.join().unwrap();
        assert!(page.complete);
        assert_eq!(page.outputs.len(), 2);
        assert_eq!(
            page.outputs[0].value,
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn changed_caller_and_callee_on_opposite_branches_surface_a_dependency() {
    let source = tempfile::tempdir().unwrap();
    let build = |text: &str| {
        std::fs::write(source.path().join("app.rs"), text).unwrap();
        build_topology_from_repo_with_options(
            source.path(),
            TopologyBuildOptions {
                repository: Some("fixture/dependencies"),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let base = build("fn leaf(x:i32)->i32{x+1} fn caller()->i32{leaf(2)}");
    let left = build("fn leaf(x:i32)->i32{x+3} fn caller()->i32{leaf(2)}");
    let right = build("fn leaf(x:i32)->i32{x+1} fn caller()->i32{leaf(4)}");
    let report = compare_branch_conflicts(&base, &left, &right).unwrap();
    let interactions: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.kind == ConflictFindingKind::PotentialInteraction)
        .collect();
    assert_eq!(interactions.len(), 1, "{report:?}");
    for name in ["caller", "leaf"] {
        let address = base
            .nodes
            .iter()
            .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == name)
            .unwrap()
            .address_id
            .as_ref()
            .unwrap();
        assert!(interactions[0].subjects.contains(address));
    }
}

#[test]
#[ignore = "explicit local collaboration performance probe; prints absolute timings"]
fn collaboration_workflow_measurements() {
    use std::time::Instant;
    let source = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("shared.db");
    for file in 0..20 {
        let text: String = (0..10)
            .map(|n| format!("def f_{file}_{n}(x):\n    return x + {n}\n"))
            .collect();
        std::fs::write(source.path().join(format!("module_{file}.py")), text).unwrap();
    }
    let started = Instant::now();
    let base = graph(source.path(), "def leaf(x):\n    return x + 1\n");
    let left = graph(source.path(), "def leaf(x):\n    return x + 2\n");
    let right = graph(source.path(), "def leaf(x):\n    return x + 3\n");
    let inventory_us = started.elapsed().as_micros();
    let started = Instant::now();
    let b = register_graph(&store, &base, "base", "a", None, 0).unwrap();
    let l = register_graph(&store, &left, "left", "a", None, 0).unwrap();
    let r = register_graph(&store, &right, "right", "a", None, 0).unwrap();
    let registration_us = started.elapsed().as_micros();
    let started = Instant::now();
    let expected = compare_branch_conflicts(&base, &left, &right).unwrap();
    let direct_us = started.elapsed().as_micros();
    let mut timings = vec![];
    for peers in [1, 8, 32] {
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let started = Instant::now();
        let job = submit_conflicts(
            &mut db,
            ConflictBatchRequest {
                git_merge: false,
                base_view: b.view.clone(),
                left: l.clone(),
                peers: vec![r.clone(); peers],
            },
            &format!("probe-{peers}"),
            "a",
        )
        .unwrap();
        drop(db);
        for slice in 0..peers {
            let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
            if slice > 0 {
                db.control_operation(job.id.as_str(), OperationControl::Resume)
                    .unwrap();
            }
            run_conflicts(&mut db, job.id.as_str(), "worker", 1).unwrap();
        }
        let duration_us = started.elapsed().as_micros();
        let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
        let page = db.operation_outputs(job.id.as_str(), 0, 100).unwrap();
        assert!(page.complete);
        assert_eq!(page.outputs.len(), peers);
        for row in page.outputs {
            assert_eq!(row.value, serde_json::to_value(&expected).unwrap());
        }
        timings.push(
            serde_json::json!({"peer_comparisons":peers,"restart_every_pair_us":duration_us}),
        );
    }
    println!(
        "{}",
        serde_json::json!({"nodes":base.nodes.len(),"three_inventories_us":inventory_us,"three_registrations_us":registration_us,"in_memory_pair_us":direct_us,"slices":timings,"store_bytes":std::fs::metadata(store).unwrap().len()})
    );
}

#[test]
#[ignore = "profile an existing workflow store via ENTMOOT_WORKFLOW_PROFILE_STORE"]
fn retained_workflow_phase_measurements() {
    use std::time::Instant;
    let path = std::env::var_os("ENTMOOT_WORKFLOW_PROFILE_STORE").unwrap();
    for repeat in 0..3 {
        let started = Instant::now();
        let db = SqliteKnowledgeRepository::open_operations_read_only(std::path::Path::new(&path))
            .unwrap();
        let publications = db.workspaces("workflow-probe").unwrap();
        let open_us = started.elapsed().as_micros();
        let mut graphs = vec![];
        let mut loads = vec![];
        let mut validations = vec![];
        let mut knowledge_loads = vec![];
        let mut projections = vec![];
        for name in ["base", "topic", "peer-overlap", "peer-dependent"] {
            let publication = publications
                .iter()
                .find(|p| p.workspace_id == name)
                .unwrap();
            let started = Instant::now();
            let graph = db.retained_graph(publication.view.as_str()).unwrap();
            loads.push(started.elapsed().as_micros());
            assert_eq!(graph.snapshot, publication.snapshot);
            let started = Instant::now();
            graph.validate().unwrap();
            validations.push(started.elapsed().as_micros());
            let selected = SqliteKnowledgeRepository::open_view_read_only(
                std::path::Path::new(&path),
                publication.view.as_str(),
            )
            .unwrap();
            let started = Instant::now();
            let knowledge = entmoot_engine::KnowledgeRepository::load(&selected).unwrap();
            knowledge_loads.push(started.elapsed().as_micros());
            let inventory = graph.clone();
            let started = Instant::now();
            let projected = knowledge.apply_to_inventory(inventory);
            projections.push(started.elapsed().as_micros());
            assert_eq!(projected.snapshot, graph.snapshot);
            graphs.push(graph);
        }
        let mut impacts = vec![];
        for head in &graphs[1..] {
            let started = Instant::now();
            let impact = entmoot_engine::compare_revision_impact(&graphs[0], head).unwrap();
            impacts.push(started.elapsed().as_micros());
            assert!(!impact.files.is_empty());
        }
        let mut pairs = vec![];
        for peer in &graphs[2..] {
            let started = Instant::now();
            let _report = compare_branch_conflicts(&graphs[0], &graphs[1], peer).unwrap();
            pairs.push(started.elapsed().as_micros());
        }
        println!(
            "{}",
            serde_json::json!({"repeat":repeat,"open_us":open_us,
            "nodes":graphs[0].nodes.len(),"load_us":loads,"validation_us":validations,
            "knowledge_load_us":knowledge_loads,"projection_us":projections,
            "impact_us":impacts,"pair_us":pairs})
        );
    }
}
