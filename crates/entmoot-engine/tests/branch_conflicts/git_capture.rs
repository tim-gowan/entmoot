use super::*;
use entmoot_contracts::{CollaborationWrite, GitTreeCapture, WorkspacePublication};
use entmoot_engine::collaboration_git::{capture_workspace, observe_captured_merge};
use std::path::PathBuf;

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
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            f.git(&["config", key, value]);
        }
        f.write("app.py", b"value = 1\n");
        f.write("other.py", b"other = 1\n");
        f.write(".gitignore", b"ignored/\n.tmp/\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "base"]);
        f
    }
    fn git(&self, args: &[&str]) -> String {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap().trim().into()
    }
    fn write(&self, path: &str, bytes: &[u8]) {
        std::fs::write(self.root.join(path), bytes).unwrap();
    }
    fn register(&self, name: &str) -> WorkspacePublication {
        let g = build_topology_from_repo_with_options(
            &self.root,
            TopologyBuildOptions {
                repository: Some("fixture/git"),
                ..Default::default()
            },
        )
        .unwrap();
        register_graph(&self.store, &g, name, "agent", None, 0).unwrap()
    }
    fn capture(&self, name: &str) -> WorkspacePublication {
        self.register(name);
        capture_workspace(&self.store, &self.root, name, 1, "agent", 100, 1024 * 1024).unwrap()
    }
    fn receipt(&self, row: &WorkspacePublication) -> GitTreeCapture {
        SqliteKnowledgeRepository::open(&self.store)
            .unwrap()
            .git_capture(row.git_capture.as_ref().unwrap().as_str())
            .unwrap()
            .receipt
    }
    fn compare(
        &self,
        left: &WorkspacePublication,
        right: &WorkspacePublication,
    ) -> entmoot_contracts::GitMergeObservation {
        let db = SqliteKnowledgeRepository::open(&self.store).unwrap();
        let l = db
            .git_capture(left.git_capture.as_ref().unwrap().as_str())
            .unwrap();
        let r = db
            .git_capture(right.git_capture.as_ref().unwrap().as_str())
            .unwrap();
        observe_captured_merge(
            &self.root,
            &self.temp.path().join("scratch"),
            &left.snapshot,
            &right.snapshot,
            Some(&l),
            Some(&r),
        )
        .unwrap()
    }
}

#[test]
fn uncommitted_conflicts_survive_further_edits_and_do_not_touch_index_or_refs() {
    let f = Fixture::new();
    f.write(".gitattributes", b"*.py merge=custom filter=custom\n");
    f.git(&["add", ".gitattributes"]);
    f.git(&["commit", "-qm", "attributes"]);
    f.git(&[
        "config",
        "merge.custom.driver",
        "echo executed > driver-ran",
    ]);
    f.git(&[
        "config",
        "filter.custom.clean",
        "echo executed > filter-ran",
    ]);
    f.write("app.py", b"value = 2\n");
    let refs = f.git(&["show-ref"]);
    let index = std::fs::read(f.root.join(".git/index")).unwrap();
    let left = f.capture("left");
    f.write("app.py", b"value = 3\n");
    let right = f.capture("right");
    f.write("app.py", b"completely different now\n");
    let first = f.compare(&left, &right);
    assert_eq!(first.status, "conflict", "{first:?}");
    assert_eq!(first, f.compare(&left, &right));
    assert!(first.details.contains("raw working-tree"));
    assert_eq!(refs, f.git(&["show-ref"]));
    assert_eq!(index, std::fs::read(f.root.join(".git/index")).unwrap());
    assert!(!f.root.join("driver-ran").exists());
    assert!(!f.root.join("filter-ran").exists());
}

#[test]
fn captured_add_delete_binary_unicode_crlf_and_ignored_tracked_inputs_are_exact() {
    let f = Fixture::new();
    std::fs::create_dir(f.root.join("ignored")).unwrap();
    f.write("ignored/retained.bin", &[0, 255, 0, 1]);
    f.git(&["add", "-f", "ignored/retained.bin"]);
    f.git(&["commit", "-qm", "ignored tracked"]);
    f.write("ignored/untracked.bin", &[9]);
    f.write("new ü file.bin", &[0, 128, 255]);
    f.write("app.py", b"value = 2\r\n");
    std::fs::remove_file(f.root.join("other.py")).unwrap();
    let left = f.capture("left");
    let db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let c = db
        .git_capture(left.git_capture.as_ref().unwrap().as_str())
        .unwrap();
    for (path, bytes) in [
        ("ignored/retained.bin", vec![0, 255, 0, 1]),
        ("new ü file.bin", vec![0, 128, 255]),
        ("app.py", b"value = 2\r\n".to_vec()),
    ] {
        let file = c.receipt.files.iter().find(|v| v.path == path).unwrap();
        assert_eq!(c.blobs.read(file.content_digest.as_str()).unwrap(), bytes);
    }
    assert!(!c
        .receipt
        .files
        .iter()
        .any(|v| v.path == "other.py" || v.path == "ignored/untracked.bin"));
    // A delete/modify conflict remains visible through quoted-path replay.
    f.write("app.py", b"value = 1\n");
    f.write("other.py", b"other = 2\n");
    std::fs::remove_file(f.root.join("new ü file.bin")).unwrap();
    let right = f.capture("right");
    assert_eq!(f.compare(&left, &right).status, "conflict"); // delete/modify conflict
}

#[test]
fn independent_raw_working_tree_edits_merge_cleanly() {
    let f = Fixture::new();
    f.write("app.py", b"value = 2\n");
    let l = f.capture("l");
    f.write("app.py", b"value = 1\n");
    f.write("other.py", b"other = 2\n");
    let r = f.capture("r");
    assert_eq!(f.compare(&l, &r).status, "clean");
}

#[test]
fn source_drift_budgets_stale_cas_and_sparse_inputs_never_publish() {
    let f = Fixture::new();
    let original = f.register("w");
    let call = |files, bytes| capture_workspace(&f.store, &f.root, "w", 1, "agent", files, bytes);
    assert_eq!(call(1, 1024).unwrap_err().code(), "budget_exhausted");
    assert_eq!(call(100, 1).unwrap_err().code(), "budget_exhausted");
    f.write("app.py", b"value = 7\n");
    assert_eq!(call(100, 1024).unwrap_err().code(), "conflict");
    f.write("app.py", b"value = 1\n");
    f.git(&["update-index", "--skip-worktree", "app.py"]);
    assert_eq!(call(100, 1024).unwrap_err().code(), "unavailable");
    f.git(&["update-index", "--no-skip-worktree", "app.py"]);
    assert_eq!(
        SqliteKnowledgeRepository::open(&f.store)
            .unwrap()
            .workspaces("fixture/git")
            .unwrap(),
        vec![original]
    );
    let captured = call(100, 1024).unwrap();
    assert_eq!(captured, call(100, 1024).unwrap());
    assert!(capture_workspace(&f.store, &f.root, "w", 1, "other agent", 100, 1024).is_err());
    let db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    assert_eq!(db.workspaces("fixture/git").unwrap(), vec![captured]);
    let sql = rusqlite::Connection::open(&f.store).unwrap();
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM local_git_captures", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn shared_dispatch_and_durable_batch_replay_freeze_git_inputs_and_mode() {
    let f = Fixture::new();
    let base = f.register("base");
    f.write("app.py", b"value = 2\n");
    f.register("left");
    let request = || CollaborationWrite::CaptureGit {
        workspace_id: "left".into(),
        expected_generation: 1,
        publisher: "agent".into(),
        max_files: 100,
        max_bytes: 1024,
    };
    assert!(write(&f.store, request()).is_err());
    let left: WorkspacePublication =
        serde_json::from_value(write_with_source(&f.store, request(), Some(&f.root)).unwrap())
            .unwrap();
    f.write("app.py", b"value = 3\n");
    let right = f.capture("right");
    let mut db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let job = submit_conflicts(
        &mut db,
        ConflictBatchRequest {
            git_merge: true,
            base_view: base.view,
            left: left.clone(),
            peers: vec![right.clone(), right],
        },
        "git-batch",
        "agent",
    )
    .unwrap();
    assert_eq!(
        run_conflicts(&mut db, job.id.as_str(), "worker", 1)
            .unwrap_err()
            .code(),
        "unavailable"
    );
    let first =
        run_conflicts_with_git(&mut db, job.id.as_str(), "worker", 1, Some(&f.root)).unwrap();
    assert_eq!(first.state, OperationState::Paused);
    drop(db);
    f.write("app.py", b"value = 99\n");
    let mut db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    db.control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    let done =
        run_conflicts_with_git(&mut db, job.id.as_str(), "worker", 1, Some(&f.root)).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    let reports = db.operation_outputs(job.id.as_str(), 0, 10).unwrap();
    assert_eq!(reports.outputs[0].value, reports.outputs[1].value);
    assert_eq!(reports.outputs[0].value["git"]["status"], "conflict");
    assert!(!f.receipt(&left).files.is_empty());
}

#[test]
fn simultaneous_identical_captures_converge_on_one_generation() {
    let f = Fixture::new();
    f.register("w");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (root, store, barrier) = (f.root.clone(), f.store.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                capture_workspace(&store, &root, "w", 1, "agent", 100, 1024).unwrap()
            })
        })
        .collect();
    let rows: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[0].generation, 2);
}

#[test]
fn mismatched_capture_cannot_attach_to_another_view() {
    let f = Fixture::new();
    let left = f.capture("left");
    f.write("app.py", b"value = 2\n");
    let mut right = f.register("right");
    right.git_capture = left.git_capture;
    let mut db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    assert!(db.publish_workspace(right, 1).is_err());
}

#[test]
fn unmerged_index_and_gitlinks_remain_explicitly_unavailable() {
    use std::io::Write;
    let f = Fixture::new();
    let blob = f.git(&["rev-parse", "HEAD:app.py"]);
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(&f.root)
        .args(["update-index", "--index-info"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        child.stdin.take().unwrap(),
        "0 {}\tapp.py\n100644 {blob} 1\tapp.py\n100644 {blob} 2\tapp.py\n",
        "0".repeat(40)
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    f.register("unmerged");
    let error =
        capture_workspace(&f.store, &f.root, "unmerged", 1, "agent", 100, 1024).unwrap_err();
    assert!(error.to_string().contains("unmerged"), "{error}");
    f.git(&["reset", "--mixed", "HEAD"]);
    let parent = f.git(&["rev-parse", "HEAD"]);
    f.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{parent},submodule"),
    ]);
    f.register("submodule");
    let error =
        capture_workspace(&f.store, &f.root, "submodule", 1, "agent", 100, 1024).unwrap_err();
    assert!(error.to_string().contains("submodule"), "{error}");
}

#[test]
fn missing_parent_history_reports_unknown_without_losing_structural_output() {
    let f = Fixture::new();
    let row = f.capture("w");
    let db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let c = db
        .git_capture(row.git_capture.as_ref().unwrap().as_str())
        .unwrap();
    drop(db);
    let other = Fixture::new();
    // An unavailable source object database cannot be interpreted as clean.
    assert!(observe_captured_merge(
        other.temp.path(),
        &other.temp.path().join("scratch"),
        &row.snapshot,
        &row.snapshot,
        Some(&c),
        Some(&c)
    )
    .is_err());
    let mut db = SqliteKnowledgeRepository::open(&f.store).unwrap();
    let job = submit_conflicts(
        &mut db,
        ConflictBatchRequest {
            git_merge: true,
            base_view: row.view.clone(),
            left: row.clone(),
            peers: vec![row.clone()],
        },
        "missing-history",
        "agent",
    )
    .unwrap();
    run_conflicts_with_git(
        &mut db,
        job.id.as_str(),
        "agent",
        1,
        Some(other.temp.path()),
    )
    .unwrap();
    let output = db.operation_outputs(job.id.as_str(), 0, 10).unwrap();
    assert_eq!(output.outputs[0].value["git"]["status"], "unavailable");
    assert!(output.outputs[0].value["findings"].is_array());
    let mut forged = c;
    forged.receipt.source.snapshot.repository_id = "different".into();
    assert!(observe_captured_merge(
        &f.root,
        &f.temp.path().join("scratch"),
        &row.snapshot,
        &row.snapshot,
        Some(&forged),
        None
    )
    .is_err());
}

#[test]
fn regular_link_representation_obeys_git_symlink_setting_without_conflating_captures() {
    let f = Fixture::new();
    f.write("link", b"outside-target");
    let blob = f.git(&["hash-object", "-w", "link"]);
    f.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("120000,{blob},link"),
    ]);
    f.git(&["config", "core.symlinks", "false"]);
    let encoded = f.capture("encoded");
    assert_eq!(
        f.receipt(&encoded)
            .files
            .iter()
            .find(|v| v.path == "link")
            .unwrap()
            .mode,
        "120000"
    );
    f.git(&["config", "core.symlinks", "true"]);
    let replaced = f.capture("replaced");
    assert_eq!(
        f.receipt(&replaced)
            .files
            .iter()
            .find(|v| v.path == "link")
            .unwrap()
            .mode,
        "100644"
    );
    assert_ne!(encoded.git_capture, replaced.git_capture);
}

#[test]
#[ignore = "explicit local raw capture and precommit conflict cost probe"]
fn raw_capture_workflow_measurements() {
    let f = Fixture::new();
    for n in 0..64 {
        f.write(
            &format!("module_{n}.py"),
            format!("def f_{n}():\n    return {n}\n").as_bytes(),
        );
    }
    f.git(&["add", "."]);
    f.git(&["commit", "-qm", "modules"]);
    f.write("app.py", b"value = 2\n");
    f.register("left");
    let start = std::time::Instant::now();
    let left = capture_workspace(&f.store, &f.root, "left", 1, "agent", 1000, 1024 * 1024).unwrap();
    let capture_ms = start.elapsed().as_millis();
    f.write("app.py", b"value = 3\n");
    let right = f.capture("right");
    let start = std::time::Instant::now();
    let report = f.compare(&left, &right);
    println!(
        "{}",
        serde_json::json!({"files":f.receipt(&left).files.len(),"capture_ms":capture_ms,"replay_merge_ms":start.elapsed().as_millis(),"status":report.status,"store_bytes":std::fs::metadata(&f.store).unwrap().len()})
    );
    assert_eq!(report.status, "conflict");
}

#[cfg(unix)]
#[test]
fn symlinks_capture_target_without_reading_target_and_keep_executable_mode() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let f = Fixture::new();
    symlink("/outside/absent-secret", f.root.join("link")).unwrap();
    std::fs::set_permissions(
        f.root.join("app.py"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let row = f.capture("w");
    let receipt = f.receipt(&row);
    assert_eq!(
        receipt
            .files
            .iter()
            .find(|v| v.path == "app.py")
            .unwrap()
            .mode,
        "100755"
    );
    assert_eq!(
        receipt
            .files
            .iter()
            .find(|v| v.path == "link")
            .unwrap()
            .mode,
        "120000"
    );
}
