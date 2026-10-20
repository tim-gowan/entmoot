//! CLI smoke gates for explicit effectful defaults and read-only discovery.
#![allow(clippy::unwrap_used)]
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Own both analyzed source and its sibling data root through one temporary lifetime.
struct WorkspaceFixture {
    _directory: tempfile::TempDir,
    source: PathBuf,
}

impl WorkspaceFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        Self {
            _directory: directory,
            source,
        }
    }

    fn path(&self) -> &Path {
        &self.source
    }
}

/// Isolated data root beside the analyzed source (never `<source>/.entmoot`).
fn data_dir(root: &Path) -> PathBuf {
    root.parent().unwrap_or(root).join(format!(
        ".entmoot-home-{}",
        root.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
    ))
}

fn storage(root: &Path) -> PathBuf {
    entmoot_engine::workspace::storage_directory_in(root, Some(&data_dir(root))).unwrap()
}

fn entmoot_cmd(root: &Path) -> Command {
    let data = data_dir(root);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_entmoot"));
    cmd.current_dir(root)
        .arg("-C")
        .arg(root)
        .arg("--data-dir")
        .arg(&data);
    cmd
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    entmoot_cmd(root).args(args).output().unwrap()
}
fn json(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.starts_with(&[239, 187, 191]));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn commit_all(root: &Path) {
    let add = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["add", "."])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "revise",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_tree(root: &Path) {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "snapshot",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn contract_find_and_path_walk_stored_edges() {
    let fixture = WorkspaceFixture::new();
    let root = fixture.path();
    fs::write(
        root.join("walk.py"),
        "def target():\n    return 1\n\ndef middle():\n    return target()\n\ndef start():\n    middle()\n    missing()\n",
    )
    .unwrap();
    commit_tree(root);
    let receipt = json(root, &["init"]);
    assert!(receipt["snapshot_id"].as_str().unwrap().len() > 8);
    assert!(receipt["callable_count"].as_u64().unwrap() >= 3);
    let typed = run(
        root,
        &[
            "find",
            "--name-contains",
            "start",
            "--parameter-type-contains",
            "Request",
        ],
    );
    assert!(!typed.status.success());
    assert!(String::from_utf8_lossy(&typed.stderr).contains("contract snapshot does not store"));
    let found = json(root, &["find", "--name-exact", "start"]);
    assert_eq!(found.as_array().unwrap().len(), 1);
    let start = found[0]["occurrence_key"].as_str().unwrap();
    let walked = json(root, &["path", start, "target", "--max-hops", "4"]);
    assert!(walked["routes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|route| route.as_array().unwrap().len() == 3));
    assert!(walked["unbound"]
        .as_array()
        .unwrap()
        .iter()
        .any(|edge| edge["target"] == "missing" && edge["status"] == "unbound"));
    let callers = json(root, &["callers", "target"]);
    assert!(callers["set"]
        .as_array()
        .unwrap()
        .iter()
        .any(|key| key.as_str().unwrap().contains("middle")));
    let source = run(root, &["read-source", "walk.py"]);
    assert!(!source.status.success());
    assert!(String::from_utf8_lossy(&source.stderr).contains("source text"));
    let audit = run(root, &["audit-calls", "--graph", "missing.json"]);
    assert!(!audit.status.success());
    assert!(String::from_utf8_lossy(&audit.stderr).contains("native dispatch"));
    let trace = run(root, &["trace-seeded-paths"]);
    assert!(!trace.status.success());
    assert!(String::from_utf8_lossy(&trace.stderr).contains("provider-role seeds"));
}

#[test]
fn scheduler_configuration_survives_cli_restart_and_matches_shared_reads() {
    let temp = WorkspaceFixture::new();
    let root = temp.path();
    let store = root.join("shared.db");
    let path = store.to_str().unwrap();
    let read = [
        "collaborate",
        "--store",
        path,
        "scheduler",
        "--repository",
        "cli/scheduler",
    ];
    assert!(!run(root, &read).status.success());
    assert!(
        !store.exists(),
        "a read cannot initialize operational storage"
    );
    let policy = root.join("policy.json");
    fs::write(
        &policy,
        r#"{"max_running":4,"max_binding":2,"work_units":8,"costs":{"binding":4,"canon":2}}"#,
    )
    .unwrap();
    let configure = [
        "collaborate",
        "--store",
        path,
        "configure-scheduler",
        "--repository",
        "cli/scheduler",
        "--expected-generation",
        "0",
        "--policy",
        policy.to_str().unwrap(),
    ];
    let configured = json(root, &configure);
    assert_eq!(configured["generation"], 1);
    assert_eq!(configured["policy"]["max_waiting"], 1024);
    assert_eq!(configured["policy"]["costs"]["binding"], 4);
    let observed = json(root, &read);
    assert_eq!(observed["configuration"], configured);
    assert_eq!(observed["active"], serde_json::json!([]));
    assert_eq!(observed["waiting_total"], 0);
    let shared = entmoot_engine::collaboration::read(
        &store,
        entmoot_contracts::CollaborationRead::Scheduler {
            repository_id: "cli/scheduler".into(),
        },
    )
    .unwrap();
    assert_eq!(shared["configuration"], configured);
    assert!(
        !run(root, &configure).status.success(),
        "stale policy writes must fail"
    );
    fs::write(&policy, r#"{"max_running":0}"#).unwrap();
    assert!(!run(root, &configure).status.success());
    fs::write(&policy, r#"{"max_runing":4}"#).unwrap();
    assert!(
        !run(root, &configure).status.success(),
        "a misspelled field cannot silently use defaults"
    );
    assert_eq!(json(root, &read)["configuration"], configured);
}

#[test]
fn raw_git_capture_cli_freezes_precommit_inputs_for_a_durable_batch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    fs::write(root.join("app.py"), "value = 1\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let store = temp.path().join("shared.db");
    let path = store.to_str().unwrap();
    let register = |name: &str| {
        let graph = entmoot_engine::build_topology_from_repo_with_options(
            &root,
            entmoot_engine::TopologyBuildOptions {
                repository: Some("cli/git"),
                ..Default::default()
            },
        )
        .unwrap();
        entmoot_engine::collaboration::register_graph(&store, &graph, name, "agent", None, 0)
            .unwrap();
    };
    register("base");
    json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "capture-git",
            "--workspace",
            "base",
            "--publisher",
            "agent",
            "--expected-generation",
            "1",
            "--source-root",
            ".",
        ],
    );
    for (name, text) in [("left", "value = 2\n"), ("right", "value = 3\n")] {
        fs::write(root.join("app.py"), text).unwrap();
        register(name);
        let row = json(
            &root,
            &[
                "collaborate",
                "--store",
                path,
                "capture-git",
                "--workspace",
                name,
                "--publisher",
                "agent",
                "--expected-generation",
                "1",
                "--source-root",
                ".",
            ],
        );
        assert_eq!(row["generation"], 2);
        assert!(row["git_capture"].as_str().unwrap().starts_with("blake3:"));
    }
    let batch = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "capture",
            "--repository",
            "cli/git",
            "--base-workspace",
            "base",
            "--workspace",
            "left",
            "--peer",
            "right",
            "--git-merge",
        ],
    );
    assert_eq!(batch["git_merge"], true);
    let request = temp.path().join("request.json");
    fs::write(&request, serde_json::to_vec(&batch).unwrap()).unwrap();
    let job = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "submit",
            "--request",
            request.to_str().unwrap(),
            "--key",
            "git",
            "--owner",
            "agent",
        ],
    );
    let id = job["id"].as_str().unwrap();
    fs::write(root.join("app.py"), "drift = 99\n").unwrap();
    assert_eq!(
        json(
            &root,
            &[
                "collaborate",
                "--store",
                path,
                "run",
                "--id",
                id,
                "--worker",
                "agent",
                "--git-root",
                "."
            ]
        )["state"],
        "completed"
    );
    let outputs = json(
        &root,
        &["collaborate", "--store", path, "outputs", "--id", id],
    );
    assert_eq!(outputs["outputs"][0]["value"]["git"]["status"], "conflict");
    let ordered = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "capture-integration",
            "--repository",
            "cli/git",
            "--workspace",
            "base",
            "--member",
            "left",
            "--member",
            "right",
        ],
    );
    assert_eq!(ordered["members"][0]["workspace_id"], "left");
    fs::write(&request, serde_json::to_vec(&ordered).unwrap()).unwrap();
    let job = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "submit-integration",
            "--request",
            request.to_str().unwrap(),
            "--key",
            "ordered",
            "--owner",
            "agent",
        ],
    );
    let id = job["id"].as_str().unwrap();
    let step = || {
        json(
            &root,
            &[
                "collaborate",
                "--store",
                path,
                "run-integration",
                "--id",
                id,
                "--worker",
                "agent",
                "--git-root",
                ".",
                "--steps",
                "1",
                "--max-staged-bytes",
                "1048576",
            ],
        )
    };
    assert_eq!(step()["state"], "paused");
    json(
        &root,
        &["collaborate", "--store", path, "resume", "--id", id],
    );
    let done = step();
    assert_eq!(done["state"], "completed");
    assert_eq!(done["result"]["status"], "conflict");
    assert_eq!(done["result"]["integrated_members"], 1);
}

#[test]
fn collaboration_producer_cli_resumes_a_sealed_result_in_a_new_process() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("app.py"), "def target(): return 7\n").unwrap();
    fs::write(root.join("helper.rs"), "fn helper() {}\n").unwrap();
    let store = tmp.path().join("shared.db");
    let path = store.to_str().unwrap();
    let source = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "source",
            "--source-root",
            ".",
            "--repository",
            "cli/producer",
        ],
    );
    assert!(
        !store.exists(),
        "digest observation must not initialize a store"
    );
    let spec = serde_json::json!({
        "source":source,
        "publication":{"workspace_id":"topic","branch":"topic","publisher":"cli","expected_generation":0},
        "task":{"producer":"inventory","policy":{"series":"entmoot.structural.v1"}}
    });
    let request = tmp.path().join("request.json");
    fs::write(&request, serde_json::to_vec(&spec).unwrap()).unwrap();
    let job = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "submit-producer",
            "--request",
            request.to_str().unwrap(),
            "--key",
            "index",
            "--owner",
            "agent",
        ],
    );
    let id = job["id"].as_str().unwrap();
    let staged = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "run-producer",
            "--id",
            id,
            "--worker",
            "first",
            "--source-root",
            ".",
            "--steps",
            "1",
            "--files-per-step",
            "1",
        ],
    );
    assert_eq!(staged["state"], "paused");
    assert_eq!(staged["checkpoint"]["inventory"]["reused_files"], 0);
    assert_eq!(
        json(
            &root,
            &[
                "collaborate",
                "--store",
                path,
                "list",
                "--repository",
                "cli/producer"
            ]
        ),
        serde_json::json!([])
    );
    json(
        &root,
        &["collaborate", "--store", path, "resume", "--id", id],
    );
    let done = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "run-producer",
            "--id",
            id,
            "--worker",
            "second",
            "--source-root",
            ".",
            "--steps",
            "7",
            "--files-per-step",
            "1",
        ],
    );
    assert_eq!(done["state"], "completed");
    assert_eq!(staged["checkpoint"]["inventory"]["completed_files"], 1);
    assert_eq!(staged["checkpoint"]["inventory"]["total_files"], 2);
    assert!(done["result"]["view"].is_string());
    assert_eq!(
        json(
            &root,
            &[
                "collaborate",
                "--store",
                path,
                "list",
                "--repository",
                "cli/producer"
            ]
        ),
        serde_json::json!([done["result"]])
    );
    let mut canon_spec = spec;
    canon_spec["publication"]["expected_generation"] = serde_json::json!(1);
    canon_spec["task"] = serde_json::json!({"producer":"canon","view":done["result"]["view"]});
    fs::write(&request, serde_json::to_vec(&canon_spec).unwrap()).unwrap();
    let job = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "submit-producer",
            "--request",
            request.to_str().unwrap(),
            "--key",
            "canon",
            "--owner",
            "agent",
        ],
    );
    let id = job["id"].as_str().unwrap();
    let partial = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "run-producer",
            "--id",
            id,
            "--worker",
            "first",
            "--source-root",
            ".",
            "--steps",
            "1",
            "--files-per-step",
            "1",
        ],
    );
    assert_eq!(
        partial["checkpoint"]["canon"],
        serde_json::json!({"completed_files":1,"total_files":2})
    );
    json(
        &root,
        &["collaborate", "--store", path, "resume", "--id", id],
    );
    let done = json(
        &root,
        &[
            "collaborate",
            "--store",
            path,
            "run-producer",
            "--id",
            id,
            "--worker",
            "second",
            "--source-root",
            ".",
            "--steps",
            "2",
            "--files-per-step",
            "1",
        ],
    );
    assert_eq!(done["state"], "completed");
    assert_eq!(done["result"]["generation"], 2);
    assert!(!root.join(".gitignore").exists());
    assert!(!root.join(".entmoot").exists());
}

#[test]
fn native_budget_defaults_to_one_hour_and_accepts_explicit_override() {
    let root = WorkspaceFixture::new();
    fs::write(root.path().join("api.py"), "def leaf(): return 1\n").unwrap();
    let indexed = json(root.path(), &["bind", "--structural-only"]);
    assert_eq!(indexed["budget_seconds"], 3600);
    let bounded = json(
        root.path(),
        &["bind", "--plan-only", "--timeout-seconds", "17"],
    );
    assert_eq!(bounded["budget_seconds"], 17);
}

#[test]
#[cfg(feature = "producer")]
fn experimental_canon_is_required_and_published_only_by_explicit_bind() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='canon-cli'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("src/lib.rs"),
        "fn a()->bool { true } fn b()->bool { true }",
    )
    .unwrap();
    fs::write(
        root.path().join("request.json"),
        br#"{"min_tokens":5,"canonical":true}"#,
    )
    .unwrap();
    json(root.path(), &["bind", "--structural-only"]);
    let missing = json(
        root.path(),
        &["assess", "clones", "--request", "request.json"],
    );
    assert_eq!(missing["canon"], "missing");
    // Require native for RA; bare bind defaults to required-native.
    json(root.path(), &["bind", "--experimental", "canon"]);
    let pointer = fs::read(storage(root.path()).join("current.json")).unwrap();
    let retained = json(
        root.path(),
        &["assess", "clones", "--request", "request.json"],
    );
    assert_eq!(retained["canon"], "available");
    assert!(retained["groups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["level"] == "canonical"));
    assert_eq!(
        pointer,
        fs::read(storage(root.path()).join("current.json")).unwrap()
    );
    let alias = run(root.path(), &["bind", "--experimental", "canonicalization"]);
    assert!(!alias.status.success());
}

#[test]
fn explicit_test_validation_writes_ingestible_status_without_changing_publication() {
    let root = WorkspaceFixture::new();
    let output = tempfile::tempdir().unwrap();
    fs::write(root.path().join("lib.rs"), "fn value() {}\n").unwrap();
    json(root.path(), &["bind", "--structural-only"]);
    let before = fs::read(storage(root.path()).join("current.json")).unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    let graph_path = output.path().join("graph.json");
    entmoot_engine::export::write_graph_json(&graph, &graph_path).unwrap();
    let request = output.path().join("request.json");
    fs::write(&request, serde_json::to_vec(&serde_json::json!({
        "targets":[{"id":"process-control", "runner":"explicit", "command":[env!("CARGO_BIN_EXE_entmoot"), "fingerprint"], "owners":["lib.rs"]}],
        "build_context": entmoot_contracts::blake3_hex(b"authored-cli-control"), "wall_seconds":10
    })).unwrap()).unwrap();
    let evidence = json(
        root.path(),
        &[
            "validate-tests",
            "--graph",
            graph_path.to_str().unwrap(),
            "--request",
            request.to_str().unwrap(),
            "--out",
            output.path().to_str().unwrap(),
        ],
    );
    assert_eq!(evidence["observations"][0]["outcome"], "passed");
    assert_eq!(
        evidence["observations"][0]["covered"],
        serde_json::json!([])
    );
    let mut joined = graph;
    entmoot_engine::ingest_artifact_manifest(
        &mut joined,
        Path::new(evidence["manifest"].as_str().unwrap()),
        false,
    )
    .unwrap();
    assert!(joined.relation_claims.iter().any(|c| matches!(
        c.value,
        entmoot_contracts::RelationClaimValue::Assessment { .. }
    )));
    assert_eq!(
        fs::read(storage(root.path()).join("current.json")).unwrap(),
        before
    );
}

#[test]
fn assessment_commands_are_read_only_and_refuse_unqualified_absence() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("lib.rs"),
        "fn first(x:i32)->i32 {x+x} fn second(y:i32)->i32 {y+y}",
    )
    .unwrap();
    fs::write(
        root.path().join("targets.json"),
        serde_json::to_vec(&serde_json::json!({"targets":[{
            "id":"unit", "runner":"cargo", "command":["cargo","test","--lib"], "owners":["lib.rs"]
        }]}))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.path().join("clones.json"),
        br#"{"min_tokens":5,"include_statements":false}"#,
    )
    .unwrap();
    json(root.path(), &["bind", "--structural-only"]);
    let pointer = storage(root.path()).join("current.json");
    let before = fs::read(&pointer).unwrap();
    let clones = json(
        root.path(),
        &["assess", "clones", "--request", "clones.json"],
    );
    assert_eq!(clones["canon"], "missing");
    assert!(clones["nudge"]
        .as_str()
        .unwrap()
        .contains("bind --experimental canon"));
    assert_eq!(fs::read(&pointer).unwrap(), before);
    let unused = run(root.path(), &["assess", "unused", "--require-complete"]);
    assert!(!unused.status.success());
    let report: Value = serde_json::from_slice(&unused.stdout).unwrap();
    assert_eq!(report["summary"], "completeness not established");
    fs::write(
        root.path().join("lib.rs"),
        "\nfn first(x:i32)->i32 {x+x} fn second(y:i32)->i32 {y+y}",
    )
    .unwrap();
    let impact = json(
        root.path(),
        &[
            "assess",
            "impact",
            "--tests",
            "--test-targets",
            "targets.json",
        ],
    );
    assert_eq!(impact["selected"][0]["target"]["id"], "unit");
    assert_eq!(impact["full_suite_required"], false);
    assert_eq!(fs::read(&pointer).unwrap(), before);
}

#[test]
#[cfg(feature = "producer")]
fn rust_profile_reaches_automatic_production_and_fences_planned_inputs() {
    let root = WorkspaceFixture::new();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='profile-cli'\nversion='0.1.0'\nedition='2021'\n[features]\na=[]\nb=[]\n",
    )
    .unwrap();
    fs::write(
        root.path().join("src/lib.rs"),
        "pub fn leaf() {}\n#[test] fn check() { leaf(); }\n",
    )
    .unwrap();
    let initialized = json(
        root.path(),
        &[
            "bind",
            "--cargo-features",
            "b,a",
            "--rust-test-root",
            "src/lib.rs",
        ],
    );
    let stage = initialized["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["family"] == "rust")
        .unwrap();
    assert_eq!(stage["stage"], "produced", "{stage}");
    let raw_path = Path::new(stage["manifest"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("native.raw.json");
    let raw: Value = serde_json::from_slice(&fs::read(raw_path).unwrap()).unwrap();
    assert_eq!(
        raw["build_context"]["rust_profile"]["analysis"],
        serde_json::json!({"kind":"test","crate_root":"src/lib.rs"})
    );
    assert_eq!(
        raw["build_context"]["rust_profile"]["features"]["names"],
        serde_json::json!(["a", "b"])
    );
    assert!(raw["calls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|call| !call["targets"].as_array().unwrap().is_empty()));
    let same = json(
        root.path(),
        &[
            "bind",
            "--plan-only",
            "--cargo-features",
            "a,b",
            "--rust-test-root",
            "src/lib.rs",
        ],
    );
    let default = json(root.path(), &["bind", "--plan-only"]);
    assert_eq!(same["stages"][0]["input_key"], stage["input_key"]);
    assert_ne!(default["stages"][0]["input_key"], stage["input_key"]);
    let pointer = storage(root.path()).join("current.json");
    let before = fs::read(&pointer).unwrap();
    for args in [
        vec!["bind", "--rust-test-root", "../escape.rs"],
        vec!["bind", "--cargo-features", "all", "--no-default-features"],
        vec!["bind", "--sysroot", "relative"],
    ] {
        assert!(!run(root.path(), &args).status.success());
        assert_eq!(fs::read(&pointer).unwrap(), before);
    }
}

#[test]
#[cfg(feature = "producer")]
fn native_cache_fingerprints_are_checked_before_reuse_or_publication() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("app.js"),
        "function leaf() { return 1; } function caller() { return leaf(); }\n",
    )
    .unwrap();
    let first = json(root.path(), &["bind"]);
    let reused = json(root.path(), &["bind"]);
    let stage = reused["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["family"] == "oxc")
        .unwrap();
    assert_eq!(stage["stage"], "reused");
    assert_eq!(
        stage["native_receipt"]["implementation"],
        first["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["family"] == "oxc")
            .unwrap()["native_receipt"]["implementation"]
    );
    let pointer = storage(root.path()).join("current.json");
    let pointer_before = fs::read(&pointer).unwrap();
    let publication: Value = serde_json::from_slice(&pointer_before).unwrap();
    let summary = Path::new(publication["summary"].as_str().unwrap());
    let summary_before = fs::read(summary).unwrap();
    let graph = Path::new(reused["publication"]["graph"].as_str().unwrap());
    let graph_before = fs::read(graph).unwrap();
    for mutation in ["missing", "contracts", "derivation", "status"] {
        let mut value: Value = serde_json::from_slice(&summary_before).unwrap();
        let receipt = &mut value["stages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|stage| stage["family"] == "oxc")
            .unwrap()["native_receipt"];
        if mutation == "missing" {
            receipt.as_object_mut().unwrap().remove("implementation");
        } else if mutation == "status" {
            receipt["semantic_status"] = "complete".into();
        } else {
            receipt["implementation"][mutation] =
                entmoot_contracts::blake3_hex(b"stale native receipt").into();
        }
        fs::write(summary, serde_json::to_vec(&value).unwrap()).unwrap();
        let rejected = run(root.path(), &["bind"]);
        assert!(
            !rejected.status.success(),
            "cached receipt accepted: {mutation}"
        );
        let error = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            error.contains("fingerprint")
                || error.contains("implementation")
                || error.contains("unknown variant"),
            "{error}"
        );
        assert_eq!(fs::read(&pointer).unwrap(), pointer_before, "{mutation}");
        assert_eq!(fs::read(graph).unwrap(), graph_before, "{mutation}");
        fs::write(summary, &summary_before).unwrap();
    }

    // Change the raw capture while keeping its enclosing digest chain consistent.
    // Compatibility must refuse independently of ordinary digest validation.
    let manifest_path = Path::new(stage["manifest"].as_str().unwrap());
    let parent = manifest_path.parent().unwrap();
    let raw_path = parent.join("native.raw.json");
    let artifact_path = parent.join("resolver.json");
    let raw_before = fs::read(&raw_path).unwrap();
    let artifact_before = fs::read(&artifact_path).unwrap();
    let manifest_before = fs::read(manifest_path).unwrap();
    let mut raw: Value = serde_json::from_slice(&raw_before).unwrap();
    raw["implementation"]["derivation"] =
        entmoot_contracts::blake3_hex(b"stale native capture").into();
    let raw_bytes = serde_json::to_vec(&raw).unwrap();
    let mut artifact: Value = serde_json::from_slice(&artifact_before).unwrap();
    artifact["build_context_digest"] = entmoot_contracts::blake3_hex(&raw_bytes).into();
    let artifact_bytes = serde_json::to_vec(&artifact).unwrap();
    let mut manifest: Value = serde_json::from_slice(&manifest_before).unwrap();
    manifest["content_digest"] = entmoot_contracts::blake3_hex(&artifact_bytes).into();
    fs::write(&raw_path, raw_bytes).unwrap();
    fs::write(&artifact_path, artifact_bytes).unwrap();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let rejected = run(root.path(), &["bind"]);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("cached native capture implementation fingerprint mismatch"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert_eq!(fs::read(&pointer).unwrap(), pointer_before);
    assert_eq!(fs::read(graph).unwrap(), graph_before);
    fs::write(&raw_path, raw_before).unwrap();
    fs::write(&artifact_path, artifact_before).unwrap();
    fs::write(manifest_path, manifest_before).unwrap();
    let restored = json(root.path(), &["bind"]);
    assert_eq!(
        restored["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["family"] == "oxc")
            .unwrap()["stage"],
        "reused"
    );
}
#[test]
fn defaults_examples_and_read_discovery() {
    let root = WorkspaceFixture::new();
    for example in ["find", "path", "neighborhood", "boundaries", "status"] {
        let request = json(root.path(), &["request", "--example", example]);
        assert_eq!(request["operation"], example);
    }
    assert!(!root.path().join(".entmoot").exists());
    fs::write(root.path().join("app.py"), "def work():\n    return 1\n").unwrap();
    commit_tree(root.path());
    let report = json(root.path(), &["init"]);
    assert!(report["snapshot_id"].as_str().unwrap().len() > 8);
    assert!(!storage(root.path()).join("current.json").exists());
    let status = json(root.path(), &["status"]);
    assert_eq!(status["snapshot_id"], report["snapshot_id"]);
    let request = storage(root.path()).join("find.json");
    fs::write(
        &request,
        br#"{"operation":"find","query":"work","limit":1}"#,
    )
    .unwrap();
    let result = json(
        root.path(),
        &["request", "--request", request.to_str().unwrap()],
    );
    assert_eq!(result.as_array().unwrap().len(), 1);
    fs::write(
        &request,
        br#"{"operation":"find","request":{"kind":"subject","query":"work"}}"#,
    )
    .unwrap();
    let publication = run(
        root.path(),
        &["request", "--request", request.to_str().unwrap()],
    );
    assert!(!publication.status.success());
    assert!(String::from_utf8_lossy(&publication.stderr).contains("graph publication"));
    json(root.path(), &["bind", "--structural-only"]);
    let pointer = storage(root.path()).join("current.json");
    let before = fs::read(&pointer).unwrap();
    let fail = entmoot_cmd(root.path())
        .env(
            "ENTMOOT_PYTHON_ENVIRONMENT",
            root.path().join("absent-environment.json"),
        )
        .args(["bind"])
        .output()
        .unwrap();
    assert!(!fail.status.success());
    assert!(String::from_utf8_lossy(&fail.stderr).contains("invalid_environment"));
    assert_eq!(fs::read(&pointer).unwrap(), before);
}

#[test]
fn bind_requires_native_by_default_and_structural_only_waives() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("main.go"),
        "package main\nfunc helper() {}\nfunc main() { helper() }\n",
    )
    .unwrap();
    let empty_adapters = root.path().join("empty-adapters");
    fs::create_dir_all(&empty_adapters).unwrap();
    let init = entmoot_cmd(root.path())
        .env_remove("ENTMOOT_GO_ADAPTER")
        .env_remove("ENTMOOT_GO_VIA_DOCKER")
        .env("ENTMOOT_ADAPTERS_ROOT", &empty_adapters)
        .args(["bind", "--structural-only"])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "structural bind records the adapter gap: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let profile: Value = serde_json::from_slice(
        &fs::read(storage(root.path()).join("adapter-profile.json")).unwrap(),
    )
    .unwrap();
    assert!(
        profile["families"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["family"] == "go" && row["hard"] == true),
        "profile must record go as a hard gap: {profile}"
    );
    let pointer = storage(root.path()).join("current.json");
    let before = fs::read(&pointer).unwrap();
    let bare = entmoot_cmd(root.path())
        .env_remove("ENTMOOT_GO_ADAPTER")
        .env_remove("ENTMOOT_GO_VIA_DOCKER")
        .env("ENTMOOT_ADAPTERS_ROOT", &empty_adapters)
        .args(["bind"])
        .output()
        .unwrap();
    assert!(
        !bare.status.success(),
        "bare bind must require native: {}",
        String::from_utf8_lossy(&bare.stderr)
    );
    let stderr = String::from_utf8_lossy(&bare.stderr);
    assert!(
        stderr.contains("invalid_environment") || stderr.contains("required go"),
        "expected require-native failure: {stderr}"
    );
    assert_eq!(
        fs::read(&pointer).unwrap(),
        before,
        "failed bind must not move publication pointer"
    );
    let waived = entmoot_cmd(root.path())
        .env_remove("ENTMOOT_GO_ADAPTER")
        .env_remove("ENTMOOT_GO_VIA_DOCKER")
        .env("ENTMOOT_ADAPTERS_ROOT", &empty_adapters)
        .args(["bind", "--structural-only"])
        .output()
        .unwrap();
    assert!(
        waived.status.success(),
        "--structural-only waives native: {}",
        String::from_utf8_lossy(&waived.stderr)
    );
}

#[test]
fn assess_alias_and_compatibility_reads_preserve_publications() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("lib.rs"),
        "pub fn api(x: i32) -> i32 { x }\n",
    )
    .unwrap();
    let initialized = json(root.path(), &["bind", "--structural-only"]);
    let graph = initialized["publication"]["graph"].as_str().unwrap();
    let graph_before = fs::read(graph).unwrap();
    let pointer = storage(root.path()).join("current.json");
    let pointer_before = fs::read(&pointer).unwrap();
    assert_eq!(
        json(root.path(), &["impact"]),
        json(root.path(), &["assess", "impact"])
    );
    let result = json(
        root.path(),
        &[
            "assess",
            "compatibility",
            "--consumer",
            graph,
            "--provider",
            graph,
        ],
    );
    assert_eq!(result["verdict"], "incomplete_bind");
    assert!(!run(
        root.path(),
        &[
            "assess",
            "compatibility",
            "--consumer",
            graph,
            "--provider",
            graph,
            "--out",
            graph
        ]
    )
    .status
    .success());
    assert_eq!(fs::read(graph).unwrap(), graph_before);
    assert_eq!(fs::read(pointer).unwrap(), pointer_before);
    let discovered = json(root.path(), &["assess", "compatibility"]);
    assert_eq!(discovered["verdict"], "incomplete_bind");
    assert_eq!(discovered["providers"], serde_json::json!([]));
    assert_eq!(
        discovered,
        json(
            root.path(),
            &["assess", "compatibility", "--consumer", graph]
        )
    );
    assert_eq!(fs::read(graph).unwrap(), graph_before);
}

#[test]
#[cfg(feature = "producer")]
fn collapsed_impact_and_bind_need_no_graph_flags() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("app.js"),
        "function target() { return 1; }\nfunction caller() { return target(); }\n",
    )
    .unwrap();
    let base = json(root.path(), &["bind"]);
    fs::write(
        root.path().join("app.js"),
        "function target() { return 2; }\nfunction caller() { return target(); }\n",
    )
    .unwrap();
    let head = json(root.path(), &["bind", "--structural-only"]);
    let base_graph = base["publication"]["graph"].as_str().unwrap();
    let head_graph = head["publication"]["graph"].as_str().unwrap();
    let package_path = storage(root.path()).join("impact.json");
    let package = json(
        root.path(),
        &[
            "impact",
            "--base-graph",
            base_graph,
            "--head-graph",
            head_graph,
            "--revalidate",
            "--out",
            package_path.to_str().unwrap(),
        ],
    );
    assert_eq!(
        package["work_hints"][0]["decision"]["action"],
        "request_artifact"
    );
    let bound = json(
        root.path(),
        &["bind", "--from-impact", package_path.to_str().unwrap()],
    );
    assert_eq!(
        bound["publication"]["stages"][0]["work"]["state"],
        "ingested"
    );
    let noop = json(
        root.path(),
        &["bind", "--from-impact", package_path.to_str().unwrap()],
    );
    assert_eq!(noop["status"], "no_work");
    let portable = json(
        root.path(),
        &[
            "impact",
            "--base-graph",
            base_graph,
            "--head-graph",
            head_graph,
            "--no-revalidate",
        ],
    );
    assert_eq!(portable["schema_version"], "entmoot.revision_impact.v1");

    fs::write(
        root.path().join("app.js"),
        "function target() { return 3; }
function caller() { return sanitize(target()); }
",
    )
    .unwrap();
    let pointer_before = fs::read(storage(root.path()).join("current.json")).unwrap();
    let working = json(root.path(), &["impact"]);
    assert_eq!(working["schema_version"], "entmoot.revision_impact.v1");
    assert_ne!(
        working["base_snapshot"]["snapshot_digest"],
        working["head_snapshot"]["snapshot_digest"]
    );
    assert!(working["callables"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
    assert_eq!(
        fs::read(storage(root.path()).join("current.json")).unwrap(),
        pointer_before
    );
    let incomplete_pair = run(root.path(), &["impact", "--base-graph", base_graph]);
    assert!(!incomplete_pair.status.success());
}

#[test]
fn relay_impact_reports_callers_of_a_changed_set() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("lib.rs"),
        "fn leaf() {}\nfn first() { leaf(); }\nfn second() { leaf(); }\n",
    )
    .unwrap();
    commit_tree(root.path());
    json(root.path(), &["init"]);
    fs::write(
        root.path().join("lib.rs"),
        "fn leaf() {}\nfn first() {}\nfn second() { leaf(); }\n",
    )
    .unwrap();
    commit_all(root.path());
    json(root.path(), &["init"]);
    let relay = json(root.path(), &["relay-impact"]);
    let leaf = relay["diff"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|change| {
            change["occurrence_key"]
                .as_str()
                .unwrap()
                .contains("|leaf|")
        })
        .expect("leaf caller set changed");
    assert!(leaf["callers_removed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|key| key.as_str().unwrap().contains("|first|")));
    assert!(relay["affected"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["occurrence_key"] == leaf["occurrence_key"])
        .unwrap()["set"]
        .as_array()
        .unwrap()
        .iter()
        .any(|key| key.as_str().unwrap().contains("|second|")));
}

#[test]
fn java_adapter_pin_is_exclusive_in_plan_summaries() {
    let root = WorkspaceFixture::new();
    fs::write(
        root.path().join("Api.java"),
        "class Api { int api() { return 1; } }",
    )
    .unwrap();
    json(root.path(), &["bind", "--structural-only"]);
    let adapters = tempfile::tempdir().unwrap();
    let fallback = adapters.path().join("java");
    fs::create_dir(&fallback).unwrap();
    fs::write(fallback.join("entmoot-java-resolver.jar"), b"not executed").unwrap();
    let pinned = adapters.path().join("pinned.jar");
    fs::write(&pinned, b"not executed").unwrap();
    for (path, available) in [(pinned, true), (adapters.path().join("missing.jar"), false)] {
        let output = entmoot_cmd(root.path())
            .args(["bind", "--plan-only"])
            .env("ENTMOOT_JAVA_ADAPTER", &path)
            .env("ENTMOOT_ADAPTERS_ROOT", adapters.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let stage = value["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["family"] == "java")
            .unwrap();
        if available {
            let expected = fs::canonicalize(&path).unwrap();
            assert_eq!(
                stage["adapter_path"].as_str().unwrap(),
                expected.to_str().unwrap()
            );
        } else {
            assert!(stage["adapter_path"].is_null());
            assert_eq!(stage["stage"], "skipped");
        }
    }
}

#[test]
#[cfg(feature = "producer")]
fn native_sessions_are_serial_attach_in_order_and_fail_atomically() {
    let root = WorkspaceFixture::new();
    let adapters = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("main.go"),
        "package example\nfunc Main() {}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("go.mod"),
        "module example.test/fixture\ngo 1.23\n",
    )
    .unwrap();
    fs::write(root.path().join("main.c"), "int main(void) { return 0; }\n").unwrap();
    fs::write(root.path().join("compile_commands.json"), "[]").unwrap();
    for family in ["clang", "go"] {
        fs::write(
            adapters.path().join(format!("{family}.py")),
            format!(
                r#"import json, sys, pathlib, time
request = json.load(sys.stdin)
markers = pathlib.Path(__file__).resolve().parent / 'markers'
markers.mkdir(exist_ok=True)
active = markers / 'active'
try:
    active.mkdir()
except FileExistsError:
    print(json.dumps({{'invalid_environment': True}}))
else:
    try:
        (markers / '{family}.started').write_text('started')
        time.sleep(0.1)
        (markers / '{family}.finished').write_text('finished')
        print(json.dumps({{'invalid_environment': False,
            'compiler_version': 'fixture', 'build_context': {{'module_scope':'single_module.v1'}}}}))
    finally:
        active.rmdir()
"#
            ),
        )
        .unwrap();
    }
    let invoke = |args: &[&str]| {
        entmoot_cmd(root.path())
            .args(args)
            .env("ENTMOOT_GO_ADAPTER", adapters.path().join("go.py"))
            .env("ENTMOOT_CLANG_ADAPTER", adapters.path().join("clang.py"))
            .output()
            .unwrap()
    };
    let first = invoke(&["bind"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    for family in ["clang", "go"] {
        assert!(adapters
            .path()
            .join(format!("markers/{family}.finished"))
            .is_file());
    }
    assert!(!adapters.path().join("markers/active").exists());
    let stages = report["stages"].as_array().unwrap();
    assert_eq!(
        stages
            .iter()
            .map(|stage| stage["family"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["clang", "go"]
    );
    assert!(stages.iter().all(|stage| stage["events"]
        .as_array()
        .unwrap()
        .contains(&Value::from("attached"))));
    assert_eq!(
        fs::read_dir(root.path()).unwrap().count(),
        4,
        "adapter synchronization must stay outside analyzed source"
    );
    let pointer = storage(root.path()).join("current.json");
    let before = fs::read(&pointer).unwrap();
    fs::write(
        adapters.path().join("clang.py"),
        "import json,sys\njson.load(sys.stdin)\nprint('{\"invalid_environment\":true}')\n",
    )
    .unwrap();
    let failed = invoke(&["bind", "--full"]);
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("invalid_environment"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert_eq!(fs::read(pointer).unwrap(), before);
}

#[test]
fn automatic_storage_is_reused_and_can_live_outside_source() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    let data = fixture.path().join("home-data");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("app.py"),
        "def entmoot_smoke_fn(object):\n    object.toString()\n",
    )
    .unwrap();
    let invoke = |args: &[&str]| -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_entmoot"))
            .env("RUST_LOG", "warn")
            .args([
                "--data-dir",
                data.to_str().unwrap(),
                "-C",
                source.to_str().unwrap(),
                "--json",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("WARN"),
            "normal structural preparation is not a native failure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    commit_tree(&source);
    let receipt = invoke(&["init"]);
    assert!(receipt["snapshot_id"].as_str().unwrap().len() > 8);
    assert!(!source.join(".entmoot").exists());
    let found = invoke(&["find", "entmoot_smoke_fn"]);
    assert_eq!(found.as_array().unwrap().len(), 1);
    let native = Command::new(env!("CARGO_BIN_EXE_entmoot"))
        .env("RUST_LOG", "warn")
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "-C",
            source.to_str().unwrap(),
            "--json",
            "boundaries",
            "--outcome",
            "unreported",
        ])
        .output()
        .unwrap();
    assert!(!native.status.success());
    assert!(String::from_utf8_lossy(&native.stderr).contains("native dispatch"));
    let status = invoke(&["status"]);
    assert_eq!(status["snapshot_id"], receipt["snapshot_id"]);
    fs::write(source.join("app.py"), "def changed(): pass\n").unwrap();
    let still = invoke(&["find", "entmoot_smoke_fn"]);
    assert_eq!(still.as_array().unwrap().len(), 1);
    let dirty = Command::new(env!("CARGO_BIN_EXE_entmoot"))
        .env("RUST_LOG", "warn")
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "-C",
            source.to_str().unwrap(),
            "init",
        ])
        .output()
        .unwrap();
    assert!(!dirty.status.success());
    assert!(String::from_utf8_lossy(&dirty.stderr).contains("clean tree"));
}

#[test]
fn invalid_storage_configuration_rejects_reads_and_writes_without_redirecting_publication() {
    for bad in [
        "broken JSON",
        r#"{"schema":"entmoot.config.v1","mode":"remote"}"#,
        r#"{"schema":"entmoot.config.v1","mode":"local","volume":"https://example.invalid/store"}"#,
    ] {
        for global in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let source = fixture.path().join("source");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("a.py"), "def leaf(): return 1\n").unwrap();
            let namespace =
                entmoot_engine::workspace::workspace_namespace(&source, &data_dir(&source));
            fs::create_dir_all(&namespace).unwrap();
            fs::write(namespace.join("config.json"), serde_json::to_vec(&serde_json::json!({
                "schema":"entmoot.config.v1", "mode":"local", "volume":fixture.path().join("custom-volume")
            })).unwrap()).unwrap();
            json(&source, &["bind", "--structural-only"]);
            let volume = storage(&source);
            let pointer = volume.join("current.json");
            let before = fs::read(&pointer).unwrap();
            let config = if global {
                data_dir(&source).join("config.json")
            } else {
                namespace.join("config.json")
            };
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, bad).unwrap();
            let plan = fixture.path().join("plan.json");
            for command in [
                vec!["bind", "--structural-only"],
                vec!["bind", "--structural-only"],
                vec!["find", "leaf"],
                vec![
                    "storage-plan",
                    "--root",
                    source.to_str().unwrap(),
                    "--out",
                    plan.to_str().unwrap(),
                ],
            ] {
                let output = run(&source, &command);
                assert!(
                    !output.status.success(),
                    "accepted {bad:?}, global={global}, command={command:?}"
                );
                let error = String::from_utf8_lossy(&output.stderr);
                assert!(
                    error.contains(if bad == "broken JSON" {
                        "invalid"
                    } else {
                        "remote storage is not implemented"
                    }),
                    "{error}"
                );
                assert_eq!(
                    fs::read(&pointer).unwrap(),
                    before,
                    "failed command replaced publication"
                );
                assert!(
                    !namespace.join("store").exists(),
                    "invalid config redirected a write to default storage"
                );
                assert_eq!(fs::read_to_string(&config).unwrap(), bad);
            }
            assert!(!source.join(".entmoot").exists());
        }
    }
}

#[test]
fn native_archive_is_readable_after_restart_in_default_and_relocated_volumes() {
    for relocated in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source");
        fs::create_dir(&source).unwrap();
        let code = "int leaf(void) { return 1; }\nint main(void) { return leaf(); }\n";
        fs::write(source.join("main.c"), code).unwrap();
        fs::write(source.join("compile_commands.json"), "[]").unwrap();
        if relocated {
            let namespace =
                entmoot_engine::workspace::workspace_namespace(&source, &data_dir(&source));
            fs::create_dir_all(&namespace).unwrap();
            fs::write(
                namespace.join("config.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema":"entmoot.config.v1", "mode":"local", "volume":fixture.path().join("custom-volume")
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let adapter = fixture.path().join("authored-clang.py");
        fs::write(&adapter, "import json,sys\njson.load(sys.stdin)\nprint(json.dumps({'compiler_version':'authored-storage-fixture'}))\n").unwrap();
        let invoke = |args: &[&str]| {
            let output = entmoot_cmd(&source)
                .env("ENTMOOT_CLANG_ADAPTER", &adapter)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        };
        let first = invoke(&["bind"]);
        assert_eq!(first["stages"][0]["stage"], "produced");
        assert!(storage(&source).join("knowledge.sqlite3.objects").is_dir());
        invoke(&["bind", "--full"]);
        assert!(storage(&source).join("knowledge.sqlite3.objects").is_dir());
        let request = fixture.path().join("boundaries.json");
        fs::write(
            &request,
            br#"{"operation":"find","request":{"kind":"call_boundary","request":{}}}"#,
        )
        .unwrap();
        let historical = entmoot_cmd(&source)
            .args(["request", "--request", request.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!historical.status.success());
        assert!(String::from_utf8_lossy(&historical.stderr).contains("graph publication"));
        assert_eq!(fs::read_to_string(source.join("main.c")).unwrap(), code);
        assert_eq!(
            fs::read_dir(&source).unwrap().count(),
            2,
            "native writes leaked into source"
        );
    }
}
