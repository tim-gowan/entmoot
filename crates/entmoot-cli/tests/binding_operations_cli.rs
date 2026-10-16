//! Real producer processes against independently authored source and recovery expectations.
#![cfg(feature = "producer")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::{
    BindingFamily, BindingPlan, ContentDigest, OperationControl, OperationRecord, OperationState,
    ProducerPublication, ProducerRequest, ProducerTask,
};
use entmoot_engine::{
    collaboration, producer_operations, LocalOperations, SqliteKnowledgeRepository,
};
use std::{
    path::PathBuf,
    process::{Command, Output},
};

const REPO: &str = "fixture/binding";
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: PathBuf,
    graph: entmoot_engine::TopologyGraph,
    input: ContentDigest,
    environment: ContentDigest,
}
impl Fixture {
    fn new() -> Self {
        Self::with_files(1)
    }
    fn with_files(files: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("main.ts"), "export function leaf(x: number) { return x + 7; }\nexport function run() { return leaf(4); }\n").unwrap();
        std::fs::write(
            root.join("main.go"),
            "package main\nfunc help() {}\nfunc main() { help() }\n",
        )
        .unwrap();
        for i in 1..files {
            std::fs::write(root.join(format!("module{i}.ts")), format!("export function leaf{i}(x: number) {{ return x + {i}; }}\nexport function run{i}() {{ return leaf{i}(4); }}\n")).unwrap();
        }
        let graph = entmoot_engine::build_topology_from_repo_with_options(
            &root,
            entmoot_engine::TopologyBuildOptions {
                repository: Some(REPO),
                ..Default::default()
            },
        )
        .unwrap();
        let store = dir.path().join("shared.db");
        let registration =
            collaboration::register_graph(&store, &graph, "topic", "agent", None, 0).unwrap();
        Self {
            _dir: dir,
            root,
            store,
            graph,
            input: registration.view,
            environment: ContentDigest::parse(entmoot_contracts::blake3_hex(
                b"host-pinned-test-environment",
            ))
            .unwrap(),
        }
    }
    fn request(&self, families: Vec<BindingFamily>) -> ProducerRequest {
        ProducerRequest {
            source: producer_operations::observe_source(&self.root, REPO).unwrap(),
            publication: ProducerPublication {
                workspace_id: "topic".into(),
                branch: None,
                publisher: "agent".into(),
                expected_generation: 1,
            },
            task: ProducerTask::Binding {
                view: self.input.clone(),
                plan: BindingPlan {
                    families,
                    environment: self.environment.clone(),
                    rust_profile: Default::default(),
                },
            },
        }
    }
    fn submit(&self, families: Vec<BindingFamily>) -> OperationRecord {
        producer_operations::submit(
            &mut SqliteKnowledgeRepository::open(&self.store).unwrap(),
            self.request(families),
            "bind",
            "agent",
        )
        .unwrap()
    }
    fn command(&self, id: &str, steps: usize) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_entmoot"));
        command
            .args(["collaborate", "--store"])
            .arg(&self.store)
            .args([
                "run-binding",
                "--id",
                id,
                "--worker",
                "test-worker",
                "--environment",
                self.environment.as_str(),
                "--source-root",
            ])
            .arg(&self.root)
            .arg("--steps")
            .arg(steps.to_string())
            .args(["--timeout-seconds", "20"])
            .env("ENTMOOT_PRODUCER", env!("CARGO_BIN_EXE_entmoot-producer"))
            .env("ENTMOOT_GO_ADAPTER", self.root.join("missing-go-adapter"));
        command
    }
    fn run(&self, id: &str, steps: usize) -> OperationRecord {
        decode(self.command(id, steps).output().unwrap())
    }
    fn control(&self, id: &str, control: OperationControl) {
        SqliteKnowledgeRepository::open(&self.store)
            .unwrap()
            .control_operation(id, control)
            .unwrap();
    }
    fn db(&self) -> SqliteKnowledgeRepository {
        SqliteKnowledgeRepository::open_operations_read_only(&self.store).unwrap()
    }
}
fn decode(output: Output) -> OperationRecord {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn sorted(graph: entmoot_engine::TopologyGraph) -> serde_json::Value {
    let mut value = serde_json::to_value(graph).unwrap();
    for rows in value
        .as_object_mut()
        .unwrap()
        .values_mut()
        .filter_map(|v| v.as_array_mut())
    {
        rows.sort_by_cached_key(|r| r.to_string());
    }
    value
}

#[test]
fn native_restart_publishes_saved_prefix_without_reexecuting_and_matches_ordinary_binding() {
    let f = Fixture::new();
    let job = f.submit(vec![BindingFamily::Oxc]);
    let paused = f.run(job.id.as_str(), 1);
    assert_eq!(paused.state, OperationState::Paused);
    assert_eq!(paused.checkpoint["binding"]["completed_families"], 1);
    assert_eq!(paused.checkpoint_sequence, 1);
    let view = paused.checkpoint["view"].as_str().unwrap();
    assert!(f.db().retained_graph(view).is_err());
    assert_eq!(f.db().workspaces(REPO).unwrap()[0].generation, 1);
    f.control(job.id.as_str(), OperationControl::Resume);
    let done = decode(
        f.command(job.id.as_str(), 1)
            .env("ENTMOOT_PRODUCER", f.root.join("no-longer-installed"))
            .output()
            .unwrap(),
    );
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.checkpoint_sequence, 2);
    assert_eq!(f.db().workspaces(REPO).unwrap()[0].generation, 2);
    let bound = f.db().retained_graph(view).unwrap();
    let caller = bound
        .nodes
        .iter()
        .find(|n| n.label == "run" && n.kind == entmoot_engine::NodeKind::Callable)
        .unwrap();
    let leaf = bound
        .nodes
        .iter()
        .find(|n| n.label == "leaf" && n.kind == entmoot_engine::NodeKind::Callable)
        .unwrap();
    assert!(
        bound.relation_edges.iter().any(|edge| edge.subject
            == entmoot_contracts::topology_node_address(&caller.id)
            && edge.object == entmoot_contracts::topology_node_address(&leaf.id)
            && edge.predicate == entmoot_contracts::RelationKind::Calls),
        "authored run -> leaf target must survive retained-view hydration"
    );
    let structural_ids = |g: &entmoot_engine::TopologyGraph| {
        g.nodes
            .iter()
            .map(|n| n.id.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(structural_ids(&bound), structural_ids(&f.graph));
    assert!(
        !bound.resolutions.is_empty(),
        "authored leaf/run fixture must gain lexical evidence"
    );
    let path = f._dir.path().join("ordinary.json");
    entmoot_engine::write_graph_json(&f.graph, &path).unwrap();
    let output = entmoot_engine::produce_oxc_resolver(entmoot_engine::NativeProduceRequest {
        graph_path: path,
        source_root: f.root.clone(),
        output_dir: f._dir.path().join("ordinary"),
        canon: false,
        budget: Default::default(),
        execution_policy: entmoot_contracts::ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let mut ordinary = f.graph.clone();
    entmoot_engine::ingest_artifact_manifest(&mut ordinary, &output.manifest_path, true).unwrap();
    assert_eq!(sorted(bound), sorted(ordinary));
    assert_eq!(
        f.run(job.id.as_str(), 2),
        done,
        "completed acknowledgement is idempotent"
    );
}

#[test]
fn failed_family_retains_attempts_without_advancing_or_repeating_completed_families() {
    let f = Fixture::new();
    let job = f.submit(vec![BindingFamily::Oxc, BindingFamily::Go]);
    let first = f.run(job.id.as_str(), 3);
    assert_eq!(first.state, OperationState::Paused);
    assert_eq!(first.checkpoint["binding"]["completed_families"], 1);
    assert_eq!(first.checkpoint_sequence, 2);
    let rows = f.db().operation_outputs(job.id.as_str(), 0, 10).unwrap();
    assert_eq!(rows.outputs.len(), 2);
    assert_eq!(rows.outputs[0].value["family"], "oxc");
    assert_eq!(rows.outputs[1].value["family"], "go");
    assert_eq!(rows.outputs[1].value["completed"], false);
    assert_eq!(
        rows.outputs[1].value["receipt"]["semantic_status"],
        "failed"
    );
    f.control(job.id.as_str(), OperationControl::Resume);
    let second = f.run(job.id.as_str(), 3);
    assert_eq!(second.checkpoint_sequence, 3);
    assert_eq!(second.checkpoint["binding"]["completed_families"], 1);
    assert_eq!(
        f.db()
            .operation_outputs(job.id.as_str(), 2, 10)
            .unwrap()
            .outputs[0]
            .value["family"],
        "go"
    );
    f.control(job.id.as_str(), OperationControl::Cancel);
    assert!(!f
        .command(job.id.as_str(), 2)
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(f.db().workspaces(REPO).unwrap()[0].generation, 1);
    assert_eq!(
        f.db()
            .operation_outputs(job.id.as_str(), 0, 10)
            .unwrap()
            .outputs
            .len(),
        3
    );
}

#[test]
fn environment_source_and_workspace_generation_fence_native_publication() {
    for condition in ["environment", "source", "workspace"] {
        let f = Fixture::new();
        let job = f.submit(vec![BindingFamily::Oxc]);
        f.run(job.id.as_str(), 1);
        f.control(job.id.as_str(), OperationControl::Resume);
        let mut command = f.command(job.id.as_str(), 1);
        match condition {
            "environment" => {
                let changed =
                    ContentDigest::parse(entmoot_contracts::blake3_hex(b"different-host")).unwrap();
                assert!(entmoot_engine::binding_operations::run(
                    &f.store,
                    &f.root,
                    job.id.as_str(),
                    "other",
                    &changed,
                    1,
                    20,
                    None
                )
                .unwrap_err()
                .to_string()
                .contains("environment differs"));
                continue;
            }
            "source" => std::fs::write(f.root.join("new.txt"), b"newly appearing input").unwrap(),
            "workspace" => {
                collaboration::register_graph(&f.store, &f.graph, "topic", "other-agent", None, 1)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let result = command.output().unwrap();
        assert!(!result.status.success(), "{condition}");
        let job = f.db().operation(job.id.as_str()).unwrap().unwrap();
        assert_ne!(job.state, OperationState::Completed);
        assert!(job.result.is_none());
        assert_eq!(job.checkpoint_sequence, 1);
    }
}

#[test]
fn receipt_write_failure_rolls_back_graph_prefix_and_cursor() {
    let f = Fixture::new();
    let job = f.submit(vec![BindingFamily::Oxc]);
    let sql = rusqlite::Connection::open(&f.store).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_binding_report BEFORE INSERT ON local_operation_outputs BEGIN SELECT RAISE(ABORT, 'injected binding report failure'); END;").unwrap();
    let result = f.command(job.id.as_str(), 2).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("injected binding report failure"));
    let failed = f.db().operation(job.id.as_str()).unwrap().unwrap();
    assert_eq!(failed.state, OperationState::Paused);
    assert_eq!(failed.checkpoint_sequence, 0);
    assert!(failed.checkpoint.is_null());
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM local_view_seals WHERE published=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(f
        .db()
        .operation_outputs(job.id.as_str(), 0, 10)
        .unwrap()
        .outputs
        .is_empty());
    sql.execute_batch("DROP TRIGGER reject_binding_report")
        .unwrap();
    f.control(job.id.as_str(), OperationControl::Resume);
    assert_eq!(f.run(job.id.as_str(), 2).state, OperationState::Completed);
}

#[test]
fn explicit_missing_worker_and_zero_allowance_do_not_stage_or_launch() {
    let f = Fixture::new();
    let job = f.submit(vec![BindingFamily::Oxc]);
    let paused = decode(
        f.command(job.id.as_str(), 2)
            .args(["--wall-time-ms", "0"])
            .env("ENTMOOT_PRODUCER", f.root.join("missing"))
            .output()
            .unwrap(),
    );
    assert_eq!(paused.state, OperationState::Paused);
    assert_eq!(paused.checkpoint_sequence, 0);
    f.control(job.id.as_str(), OperationControl::Resume);
    let failed = f
        .command(job.id.as_str(), 2)
        .env("ENTMOOT_PRODUCER", f.root.join("missing"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("native worker unavailable"));
    assert_eq!(
        f.db()
            .operation(job.id.as_str())
            .unwrap()
            .unwrap()
            .checkpoint_sequence,
        0
    );
}

#[test]
fn binding_rejects_empty_duplicate_and_host_path_plans_before_admission() {
    let f = Fixture::new();
    for families in [vec![], vec![BindingFamily::Oxc, BindingFamily::Oxc]] {
        let error = producer_operations::submit(
            &mut SqliteKnowledgeRepository::open(&f.store).unwrap(),
            f.request(families),
            "invalid",
            "agent",
        )
        .unwrap_err();
        assert!(error.to_string().contains("distinct explicit families"));
    }
    let mut request = f.request(vec![BindingFamily::Rust]);
    let ProducerTask::Binding { plan, .. } = &mut request.task else {
        unreachable!()
    };
    plan.rust_profile.sysroot = Some(f.root.clone());
    assert!(producer_operations::submit(
        &mut SqliteKnowledgeRepository::open(&f.store).unwrap(),
        request,
        "invalid",
        "agent"
    )
    .unwrap_err()
    .to_string()
    .contains("sysroot"));
    let job = f.submit(vec![BindingFamily::Oxc]);
    for (steps, timeout) in [(0, 20), (1001, 20), (1, 0), (1, 3601)] {
        assert!(entmoot_engine::binding_operations::run(
            &f.store,
            &f.root,
            job.id.as_str(),
            "agent",
            &f.environment,
            steps,
            timeout,
            None
        )
        .is_err());
        assert_eq!(f.db().operation(job.id.as_str()).unwrap().unwrap(), job);
    }
}

#[test]
fn concurrent_branch_workers_publish_the_same_evidence_as_serial_execution() {
    let f = Fixture::new();
    collaboration::register_graph(&f.store, &f.graph, "peer", "peer-agent", None, 0).unwrap();
    let first = f.submit(vec![BindingFamily::Oxc]);
    let mut request = f.request(vec![BindingFamily::Oxc]);
    request.publication.workspace_id = "peer".into();
    let second = producer_operations::submit(
        &mut SqliteKnowledgeRepository::open(&f.store).unwrap(),
        request,
        "peer-bind",
        "peer-agent",
    )
    .unwrap();
    let a = f
        .command(first.id.as_str(), 2)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let b = f.command(second.id.as_str(), 2).output().unwrap();
    let a = a.wait_with_output().unwrap();
    for (job, output) in [(&first, a), (&second, b)] {
        if !output.status.success() {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("capacity"),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(f.run(job.id.as_str(), 2).state, OperationState::Completed);
        } else {
            assert_eq!(decode(output).state, OperationState::Completed);
        }
    }
    let rows = f.db().workspaces(REPO).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.generation == 2));
    assert_eq!(rows[0].view, rows[1].view);
    assert_eq!(
        f.db()
            .operation_outputs(first.id.as_str(), 0, 10)
            .unwrap()
            .outputs
            .len(),
        1
    );
    assert_eq!(
        f.db()
            .operation_outputs(second.id.as_str(), 0, 10)
            .unwrap()
            .outputs
            .len(),
        1
    );
}

#[test]
#[ignore = "opt-in process fault control requires installed Node; no adapter or SDK installation"]
fn pause_and_cancel_stop_native_descendants_and_keep_saved_prefix() {
    assert!(Command::new("node")
        .arg("--version")
        .output()
        .unwrap()
        .status
        .success());
    for action in ["pause", "cancel"] {
        let f = Fixture::new();
        let script = f._dir.path().join("blocked-adapter.cjs");
        std::fs::write(&script, r#"
const fs = require('node:fs');
const cp = require('node:child_process');
cp.spawn(process.execPath, ['-e', "setTimeout(()=>require('node:fs').writeFileSync(process.env.ESCAPED,'escaped'),2500)"], {stdio:'ignore'});
fs.writeFileSync(process.env.READY, 'ready');
setInterval(()=>{},1000);
"#).unwrap();
        let ready = f._dir.path().join("ready");
        let escaped = f._dir.path().join("escaped");
        let job = f.submit(vec![BindingFamily::Oxc, BindingFamily::Go]);
        let mut worker = f
            .command(job.id.as_str(), 3)
            .env("ENTMOOT_GO_ADAPTER", script)
            .env("READY", &ready)
            .env("ESCAPED", &escaped)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while !ready.exists() && started.elapsed() < std::time::Duration::from_secs(15) {
            assert!(
                worker.try_wait().unwrap().is_none(),
                "worker exited before sidecar startup"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !ready.exists() {
            worker.kill().unwrap();
            worker.wait().unwrap();
            panic!("sidecar startup deadline");
        }
        assert_eq!(
            f.db()
                .operation(job.id.as_str())
                .unwrap()
                .unwrap()
                .checkpoint_sequence,
            1
        );
        match action {
            "pause" => f.control(job.id.as_str(), OperationControl::Pause),
            "cancel" => f.control(job.id.as_str(), OperationControl::Cancel),
            _ => unreachable!(),
        }
        let stopped = std::time::Instant::now();
        while worker.try_wait().unwrap().is_none()
            && stopped.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if worker.try_wait().unwrap().is_none() {
            worker.kill().unwrap();
            worker.wait().unwrap();
            panic!("revoked worker did not stop");
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(
            !escaped.exists(),
            "{action} left a native descendant running"
        );
        let saved = f.db().operation(job.id.as_str()).unwrap().unwrap();
        assert_eq!(saved.checkpoint_sequence, 1);
        assert_eq!(saved.checkpoint["binding"]["completed_families"], 1);
        assert_eq!(f.db().workspaces(REPO).unwrap()[0].generation, 1);
        if action != "cancel" {
            f.control(job.id.as_str(), OperationControl::Resume);
            let retry = f.run(job.id.as_str(), 2);
            assert_eq!(retry.checkpoint_sequence, 2);
            assert_eq!(retry.checkpoint["binding"]["completed_families"], 1);
            assert_eq!(
                f.db()
                    .operation_outputs(job.id.as_str(), 1, 10)
                    .unwrap()
                    .outputs[0]
                    .value["family"],
                "go"
            );
        }
    }
}

#[test]
#[ignore = "explicit local end-to-end cost probe, not a timing gate"]
fn binding_restart_cost_probe() {
    let setup = std::time::Instant::now();
    let mut f = Fixture::with_files(32);
    let graph_path = f._dir.path().join("graph.json");
    entmoot_engine::write_graph_json(&f.graph, &graph_path).unwrap();
    println!("setup_ms={}", setup.elapsed().as_millis());
    for round in 0..3 {
        let modes = if round % 2 == 0 {
            ["ordinary", "durable", "restart"]
        } else {
            ["restart", "durable", "ordinary"]
        };
        for mode in modes {
            f.store = f._dir.path().join(format!("{round}-{mode}.db"));
            collaboration::register_graph(&f.store, &f.graph, "topic", "agent", None, 0).unwrap();
            let before = std::fs::metadata(&f.store).unwrap().len();
            let started = std::time::Instant::now();
            if mode == "ordinary" {
                let output =
                    entmoot_engine::produce_oxc_resolver(entmoot_engine::NativeProduceRequest {
                        graph_path: graph_path.clone(),
                        source_root: f.root.clone(),
                        output_dir: f._dir.path().join(format!("ordinary-{round}")),
                        canon: false,
                        budget: Default::default(),
                        execution_policy: entmoot_contracts::ExecutionPolicy::Safe,
                        rust_profile: Default::default(),
                    })
                    .unwrap();
                let mut bound = f.graph.clone();
                entmoot_engine::ingest_artifact_manifest(&mut bound, &output.manifest_path, true)
                    .unwrap();
                collaboration::register_graph(&f.store, &bound, "topic", "agent", None, 1).unwrap();
            } else {
                let job = f.submit(vec![BindingFamily::Oxc]);
                if mode == "restart" {
                    assert_eq!(f.run(job.id.as_str(), 1).state, OperationState::Paused);
                    f.control(job.id.as_str(), OperationControl::Resume);
                }
                assert_eq!(f.run(job.id.as_str(), 2).state, OperationState::Completed);
            }
            let elapsed = started.elapsed().as_millis();
            let after = std::fs::metadata(&f.store).unwrap().len();
            println!(
                "round={round} mode={mode} elapsed_ms={elapsed} growth_bytes={}",
                after - before
            );
        }
    }
}
