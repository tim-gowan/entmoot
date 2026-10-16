//! Independently authored same-repository integration outcomes and restart fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::{
    IntegrationRequest, OperationControl, OperationState, WorkspacePublication,
};
use entmoot_engine::{
    collaboration, integration_operations as integration, LocalOperations,
    SqliteKnowledgeRepository,
};
use std::{path::PathBuf, process::Command};

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    store: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let f = Self {
            root,
            store: temp.path().join("shared.db"),
            temp,
        };
        f.git(&["init", "-q"]);
        for (k, v) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            f.git(&["config", k, v]);
        }
        for (p, v) in [
            ("app.py", "value = 1\n"),
            ("peer.py", "peer = 1\n"),
            ("tail.py", "tail = 1\n"),
            (".gitignore", ".tmp/\n"),
        ] {
            f.write(p, v);
        }
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "base"]);
        f
    }
    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.root.join(path), text).unwrap();
    }
    fn git(&self, args: &[&str]) -> String {
        let r = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            r.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&r.stderr)
        );
        String::from_utf8(r.stdout).unwrap().trim().into()
    }
    fn capture(&self, name: &str) -> WorkspacePublication {
        let graph = entmoot_engine::build_topology_from_repo_with_options(
            &self.root,
            entmoot_engine::TopologyBuildOptions {
                repository: Some("fixture/integration"),
                ..Default::default()
            },
        )
        .unwrap();
        collaboration::register_graph(&self.store, &graph, name, "agent", None, 0).unwrap();
        entmoot_engine::collaboration_git::capture_workspace(
            &self.store,
            &self.root,
            name,
            1,
            "agent",
            1000,
            1024 * 1024,
        )
        .unwrap()
    }
    fn request(&self, conflict: bool) -> IntegrationRequest {
        self.write("app.py", "value = 2\n");
        let initial = self.capture("initial");
        self.write("app.py", "value = 1\n");
        self.write("peer.py", "peer = 2\n");
        let first = self.capture("z-first");
        self.write(
            "peer.py",
            if conflict { "peer = 3\n" } else { "peer = 1\n" },
        );
        self.write("tail.py", "tail = 2\n");
        let second = self.capture("a-second");
        IntegrationRequest {
            initial,
            members: vec![first, second],
        }
    }
    fn submit(&self, request: IntegrationRequest, key: &str) -> String {
        integration::submit(
            &mut SqliteKnowledgeRepository::open(&self.store).unwrap(),
            request,
            key,
            "agent",
        )
        .unwrap()
        .id
        .to_string()
    }
    fn run(&self, id: &str, steps: usize) -> entmoot_contracts::OperationRecord {
        integration::run(
            &self.store,
            &self.root,
            id,
            "worker",
            steps,
            64 * 1024 * 1024,
            None,
        )
        .unwrap()
    }
    fn resume(&self, id: &str) {
        SqliteKnowledgeRepository::open(&self.store)
            .unwrap()
            .control_operation(id, OperationControl::Resume)
            .unwrap();
    }
    fn outputs(&self, id: &str) -> Vec<serde_json::Value> {
        SqliteKnowledgeRepository::open(&self.store)
            .unwrap()
            .operation_outputs(id, 0, 100)
            .unwrap()
            .outputs
            .into_iter()
            .map(|r| r.value)
            .collect()
    }
    fn packs(&self, id: &str) -> i64 {
        rusqlite::Connection::open(&self.store)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM local_integration_packs WHERE operation_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[test]
fn cumulative_prefix_finds_conflict_missed_by_initial_vs_each_member_and_stops() {
    let f = Fixture::new();
    let mut request = f.request(true);
    let db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let left = db
        .git_capture(request.initial.git_capture.as_ref().unwrap().as_str())
        .unwrap();
    for member in &request.members {
        let right = db
            .git_capture(member.git_capture.as_ref().unwrap().as_str())
            .unwrap();
        let result = entmoot_engine::collaboration_git::observe_captured_merge(
            &f.root,
            &f.temp.path().join("pairs"),
            &request.initial.snapshot,
            &member.snapshot,
            Some(&left),
            Some(&right),
        )
        .unwrap();
        assert_eq!(result.status, "clean");
    }
    drop(db);
    f.write("tail.py", "tail = 3\n");
    request.members.push(f.capture("untested"));
    let refs = f.git(&["show-ref"]);
    let index = std::fs::read(f.root.join(".git/index")).unwrap();
    let serial = f.submit(request.clone(), "serial");
    let sliced = f.submit(request, "sliced");
    let done = f.run(&serial, 10);
    assert_eq!(done.state, OperationState::Completed);
    let result = done.result.unwrap();
    assert_eq!(result["status"], "conflict");
    assert_eq!(result["integrated_members"], 1);
    assert_eq!(result["attempted_members"], 2);
    assert_eq!(result["untested_members"], 1);
    let partial = f.run(&sliced, 1);
    assert_eq!(partial.state, OperationState::Paused);
    assert_eq!(f.packs(&sliced), 1);
    f.write("app.py", "unrelated = 99\n");
    f.resume(&sliced);
    assert_eq!(f.run(&sliced, 10).state, OperationState::Completed);
    assert_eq!(f.outputs(&serial), f.outputs(&sliced));
    assert_eq!(f.packs(&sliced), 0);
    assert_eq!(f.outputs(&sliced)[1]["member"]["workspace_id"], "a-second");
    assert_eq!(f.git(&["show-ref"]), refs);
    assert_eq!(std::fs::read(f.root.join(".git/index")).unwrap(), index);
}

#[test]
fn clean_serial_and_sliced_integrations_match_an_independently_staged_expected_tree() {
    let f = Fixture::new();
    let request = f.request(false);
    let a = f.submit(request.clone(), "serial");
    let b = f.submit(request, "sliced");
    let done = f.run(&a, 10);
    assert_eq!(done.result.as_ref().unwrap()["status"], "clean");
    f.run(&b, 1);
    f.resume(&b);
    let restarted = f.run(&b, 1);
    assert_eq!(done.result, restarted.result);
    assert_eq!(f.outputs(&a), f.outputs(&b));
    let reports = f.outputs(&b);
    // Expected union is authored independently from the integration implementation.
    f.write("app.py", "value = 2\n");
    f.write("peer.py", "peer = 2\n");
    f.write("tail.py", "tail = 2\n");
    f.git(&["add", "."]);
    let expected = f.git(&["write-tree"]);
    assert_eq!(
        reports[1]["git"]["details"]
            .as_str()
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        expected
    );
    assert_eq!(f.packs(&b), 0);
}

#[test]
fn budget_can_be_raised_after_pause_and_zero_time_never_advances_the_prefix() {
    let f = Fixture::new();
    let id = f.submit(f.request(false), "budget");
    let before = integration::run(&f.store, &f.root, &id, "worker", 1, 1, None).unwrap();
    assert_eq!(before.state, OperationState::Paused);
    assert!(before.reason.unwrap().contains("budget"));
    assert_eq!(before.checkpoint_sequence, 0);
    assert_eq!(f.packs(&id), 0);
    f.resume(&id);
    let partial = f.run(&id, 1);
    assert_eq!(partial.checkpoint["next_member"], 1);
    assert_eq!(f.packs(&id), 1);
    f.resume(&id);
    let timed = integration::run(
        &f.store,
        &f.root,
        &id,
        "worker",
        1,
        64 * 1024 * 1024,
        Some(0),
    )
    .unwrap();
    assert_eq!(timed.state, OperationState::Paused);
    assert_eq!(timed.checkpoint, partial.checkpoint);
    assert_eq!(timed.checkpoint_sequence, partial.checkpoint_sequence);
    f.resume(&id);
    assert_eq!(f.run(&id, 1).result.unwrap()["status"], "clean");
}

#[test]
fn source_history_failure_preserves_a_resumable_prefix_and_cancellation_releases_objects() {
    let f = Fixture::new();
    let id = f.submit(f.request(false), "unavailable");
    f.run(&id, 1);
    f.resume(&id);
    assert!(integration::run(
        &f.store,
        f.temp.path(),
        &id,
        "worker",
        1,
        64 * 1024 * 1024,
        None
    )
    .is_err());
    let mut db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let item = db.operation(&id).unwrap().unwrap();
    assert_eq!(item.state, OperationState::Paused);
    assert_eq!(item.checkpoint["next_member"], 1);
    assert_eq!(f.packs(&id), 1);
    db.control_operation(&id, OperationControl::Cancel).unwrap();
    assert_eq!(f.packs(&id), 0);
    assert_eq!(f.outputs(&id).len(), 1);
}

#[test]
fn captured_order_and_low_level_envelope_are_checked_before_execution() {
    let f = Fixture::new();
    let request = f.request(false);
    let captured = collaboration::read(
        &f.store,
        entmoot_contracts::CollaborationRead::CaptureIntegration {
            repository_id: "fixture/integration".into(),
            workspace: "initial".into(),
            members: vec!["z-first".into(), "a-second".into()],
        },
    )
    .unwrap();
    assert_eq!(captured, serde_json::to_value(&request).unwrap());
    let mut db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let mut duplicate = request.clone();
    duplicate.members.push(duplicate.members[0].clone());
    assert!(integration::submit(&mut db, duplicate, "duplicate", "agent").is_err());
    let mut uncaptured = request.clone();
    uncaptured.members[0].git_capture = None;
    assert!(integration::submit(&mut db, uncaptured, "uncaptured", "agent").is_err());
    let valid = integration::submit(&mut db, request, "valid", "agent").unwrap();
    let mut bad = valid.spec;
    bad.idempotency_key = "bad".into();
    bad.views.clear();
    let item = db.submit_operations(&[bad], 100).unwrap().remove(0);
    drop(db);
    assert!(integration::run(
        &f.store,
        &f.root,
        item.id.as_str(),
        "worker",
        1,
        64 * 1024 * 1024,
        None
    )
    .is_err());
    assert_eq!(
        SqliteKnowledgeRepository::open(&f.store)
            .unwrap()
            .operation(item.id.as_str())
            .unwrap()
            .unwrap()
            .generation,
        0
    );
}

#[test]
fn merged_prefix_preserves_ancestry_for_a_later_descendant() {
    let f = Fixture::new();
    let initial = f.capture("initial");
    f.write("peer.py", "peer = 2\n");
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "first"]);
    let ancestor = f.git(&["rev-parse", "HEAD"]);
    let first = f.capture("first");
    f.write("peer.py", "peer = 3\n");
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "descendant"]);
    let second = f.capture("second");
    let id = f.submit(
        IntegrationRequest {
            initial,
            members: vec![first, second],
        },
        "ancestry",
    );
    f.run(&id, 1);
    f.resume(&id);
    assert_eq!(f.run(&id, 1).result.unwrap()["status"], "clean");
    assert_eq!(
        f.outputs(&id)[1]["git"]["merge_bases"],
        serde_json::json!([ancestor])
    );
}

#[test]
fn unrelated_histories_remain_unknown_and_retry_does_not_require_a_missing_synthetic_head() {
    let f = Fixture::new();
    let initial = f.capture("initial");
    f.git(&["checkout", "--orphan", "unrelated"]);
    f.write("peer.py", "peer = 99\n");
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "unrelated root"]);
    let member = f.capture("unrelated");
    let id = f.submit(
        IntegrationRequest {
            initial,
            members: vec![member],
        },
        "unknown",
    );
    for _ in 0..2 {
        let item = f.run(&id, 1);
        assert_eq!(item.state, OperationState::Paused);
        assert_eq!(item.checkpoint["next_member"], 0);
        assert!(item.checkpoint["head"].is_null());
        assert!(item.result.is_none());
        assert_eq!(f.packs(&id), 0);
        f.resume(&id);
    }
    let reports = f.outputs(&id);
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0]["git"]["status"], "unavailable");
    assert_eq!(reports[0], reports[1]);
}

#[test]
fn concurrent_integrations_match_serial_without_sharing_mutable_git_state() {
    let f = Fixture::new();
    let request = f.request(false);
    let ids = [
        f.submit(request.clone(), "parallel-a"),
        f.submit(request.clone(), "parallel-b"),
    ];
    let gate = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let jobs = ids
            .iter()
            .map(|id| {
                scope.spawn(|| {
                    gate.wait();
                    assert_eq!(f.run(id, 10).result.unwrap()["status"], "clean");
                })
            })
            .collect::<Vec<_>>();
        for job in jobs {
            job.join().unwrap();
        }
    });
    let serial = f.submit(request, "serial");
    f.run(&serial, 10);
    for id in ids {
        assert_eq!(f.outputs(&id), f.outputs(&serial));
    }
}

#[test]
fn tool_version_drift_and_corrupted_private_bytes_preserve_the_last_good_cursor() {
    let f = Fixture::new();
    let id = f.submit(f.request(false), "drift");
    let partial = f.run(&id, 1);
    let db = rusqlite::Connection::open(&f.store).unwrap();
    let mut changed = partial.clone();
    changed.checkpoint["git_version"] = serde_json::json!("different Git version");
    db.execute(
        "UPDATE local_operations SET body=?1 WHERE operation_id=?2",
        rusqlite::params![serde_json::to_vec(&changed).unwrap(), id],
    )
    .unwrap();
    f.resume(&id);
    assert!(
        integration::run(&f.store, &f.root, &id, "worker", 1, 64 * 1024 * 1024, None)
            .unwrap_err()
            .to_string()
            .contains("Git version changed")
    );
    let mut changed = SqliteKnowledgeRepository::open(&f.store)
        .unwrap()
        .operation(&id)
        .unwrap()
        .unwrap();
    assert_eq!(changed.checkpoint_sequence, 1);
    changed.checkpoint = partial.checkpoint;
    db.execute(
        "UPDATE local_operations SET body=?1 WHERE operation_id=?2",
        rusqlite::params![serde_json::to_vec(&changed).unwrap(), id],
    )
    .unwrap();
    db.execute(
        "UPDATE local_integration_packs SET body=?1 WHERE operation_id=?2",
        rusqlite::params![b"corrupt".as_slice(), id],
    )
    .unwrap();
    f.resume(&id);
    assert!(
        integration::run(&f.store, &f.root, &id, "worker", 1, 64 * 1024 * 1024, None)
            .unwrap_err()
            .to_string()
            .contains("pack digest mismatch")
    );
    assert_eq!(f.outputs(&id).len(), 1);
    assert_eq!(f.packs(&id), 1);
}

#[test]
fn abrupt_worker_exit_retains_the_next_prefix_without_replaying_completed_merges() {
    if let Ok(store) = std::env::var("ENTMOOT_INTEGRATION_TEST_STORE") {
        let root = std::env::var("ENTMOOT_INTEGRATION_TEST_ROOT").unwrap();
        let id = std::env::var("ENTMOOT_INTEGRATION_TEST_ID").unwrap();
        integration::run(
            std::path::Path::new(&store),
            std::path::Path::new(&root),
            &id,
            "child",
            1,
            64 * 1024 * 1024,
            None,
        )
        .unwrap();
        std::process::exit(0);
    }
    let f = Fixture::new();
    let request = f.request(false);
    let id = f.submit(request.clone(), "child");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "abrupt_worker_exit_retains_the_next_prefix_without_replaying_completed_merges",
            "--nocapture",
        ])
        .env("ENTMOOT_INTEGRATION_TEST_STORE", &f.store)
        .env("ENTMOOT_INTEGRATION_TEST_ROOT", &f.root)
        .env("ENTMOOT_INTEGRATION_TEST_ID", &id)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(f.packs(&id), 1);
    let first = f.outputs(&id);
    assert_eq!(first.len(), 1);
    f.resume(&id);
    assert_eq!(f.run(&id, 1).result.unwrap()["status"], "clean");
    assert_eq!(f.outputs(&id)[0], first[0]);
    let serial = f.submit(request, "serial");
    f.run(&serial, 10);
    assert_eq!(f.outputs(&id), f.outputs(&serial));
}

#[test]
#[ignore = "opt-in local Git process/storage cost probe; not a scale qualification"]
fn integration_restart_cost_probe() {
    let f = Fixture::new();
    for i in 0..8 {
        f.write(&format!("member_{i}.py"), "value = 0\n");
    }
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "eight independent inputs"]);
    let capture_started = std::time::Instant::now();
    let initial = f.capture("initial");
    let mut members = Vec::new();
    for i in 0..8 {
        let path = format!("member_{i}.py");
        f.write(&path, "value = 1\n");
        members.push(f.capture(&format!("member-{i}")));
        f.write(&path, "value = 0\n");
    }
    let capture_ms = capture_started.elapsed().as_millis();
    let request = IntegrationRequest { initial, members };
    let mut samples = Vec::new();
    for round in 0..3 {
        let mut elapsed = [0; 2];
        let mut ids = [String::new(), String::new()];
        let mut peak_bytes = 0;
        for mode in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
            let start = std::time::Instant::now();
            ids[mode] = f.submit(request.clone(), &format!("probe-{round}-{mode}"));
            loop {
                let item = f.run(&ids[mode], if mode == 0 { 10 } else { 1 });
                let db = rusqlite::Connection::open(&f.store).unwrap();
                let bytes:i64=db.query_row("SELECT COALESCE(SUM(length(body)),0) FROM local_integration_packs WHERE operation_id=?1",[&ids[mode]],|r|r.get(0)).unwrap();
                peak_bytes = peak_bytes.max(bytes);
                if item.state == OperationState::Completed {
                    assert_eq!(item.result.unwrap()["status"], "clean");
                    break;
                }
                f.resume(&ids[mode]);
            }
            elapsed[mode] = start.elapsed().as_millis();
        }
        assert_eq!(f.outputs(&ids[0]), f.outputs(&ids[1]));
        samples.push(serde_json::json!({"serial_ms":elapsed[0],"eight_slices_ms":elapsed[1],"peak_saved_pack_bytes":peak_bytes}));
    }
    println!(
        "{}",
        serde_json::json!({"members":8,"captured_workspaces":9,"capture_ms":capture_ms,"git":f.git(&["--version"]),"samples":samples,"store_bytes":std::fs::metadata(&f.store).unwrap().len()})
    );
}
