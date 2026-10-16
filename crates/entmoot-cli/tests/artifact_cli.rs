use std::process::Command;

use entmoot_contracts::{
    blake3_hex, ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness,
    ScopeUniverse,
};
use entmoot_engine::{
    load_graph_json, GraphSummary, KnowledgeRepository, SqliteKnowledgeRepository,
};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_entmoot"))
}

#[test]
fn fingerprint_command_emits_the_rust_reader_contract_without_creating_a_store() {
    let directory = tempfile::tempdir().unwrap();
    let output = cli()
        .current_dir(directory.path())
        .arg("fingerprint")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: entmoot_contracts::StorageFingerprint =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, entmoot_engine::storage_fingerprint());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn retention_cli_plans_without_deleting_then_applies_a_fenced_plan() {
    // Durable state is home-scoped (`--data-dir`), never `<source>/.entmoot`.
    let home = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let storage =
        entmoot_engine::workspace::storage_directory_in(directory.path(), Some(home.path()));
    let retention = |args: &[&str]| {
        let mut cmd = cli();
        cmd.args(["--data-dir"]).arg(home.path()).args(args);
        cmd.output().unwrap()
    };
    for value in 0..4 {
        std::fs::write(
            directory.path().join("a.py"),
            format!("VALUE = {value}\ndef get():\n    return VALUE\n"),
        )
        .unwrap();
        let output = retention(&[
            "init",
            "--root",
            directory.path().to_str().unwrap(),
            "--structural-only",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let plan = home.path().join("plan.json");
    let output = retention(&[
        "storage-plan",
        "--root",
        directory.path().to_str().unwrap(),
        "--out",
        plan.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["remove_views"].as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read_dir(storage.join("current")).unwrap().count(), 4);
    let output = retention(&[
        "storage-apply",
        "--root",
        directory.path().to_str().unwrap(),
        "--plan",
        plan.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_dir(storage.join("current")).unwrap().count(), 2);
    let output = retention(&[
        "storage-apply",
        "--root",
        directory.path().to_str().unwrap(),
        "--plan",
        plan.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stale"));
}

fn without_telemetry(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("elapsed_micros");
            object.remove("index_micros");
            object.remove("evidence_micros");
            for child in object.values_mut() {
                without_telemetry(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                without_telemetry(child);
            }
        }
        _ => {}
    }
}

#[test]
fn typed_request_frontend_matches_service_and_preserves_publication() {
    use entmoot_contracts::query::{BoundedWalkBudget, QueryRequest, QueryService};
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("a.py"),
        "def leaf():\n    pass\nLIMIT = -1\n",
    )
    .unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(directory.path(), Default::default())
            .unwrap();
    let graph_path = directory.path().join("graph.json");
    let store_path = directory.path().join("graph.json.knowledge.sqlite3");
    entmoot_engine::write_graph_json(&graph, &graph_path).unwrap();
    SqliteKnowledgeRepository::open(&store_path)
        .unwrap()
        .replace_all(&entmoot_engine::KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let request_path = directory.path().join("request.json");
    std::fs::write(
        &request_path,
        br#"{"operation":"find","request":{"kind":"subject","query":"leaf","limit":5}}"#,
    )
    .unwrap();
    let before = std::fs::read(&graph_path).unwrap();
    let engine = entmoot_engine::QueryEngine::new(graph, store_path).with_source_store(
        std::sync::Arc::new(entmoot_engine::GixSourceStore::open(directory.path()).unwrap()),
    );
    for request in [
        serde_json::json!({"operation":"find","request":{"kind":"initializer","integer":"-1"}}),
        serde_json::json!({"operation":"find","request":{"kind":"initializer","integer":"01"}}),
        serde_json::json!({"operation":"find","request":{"kind":"subject","query":"leaf","limit":5}}),
        serde_json::json!({"operation":"inspect","request":{"kind":"trace","id":"absent"}}),
        serde_json::json!({"operation":"read","request":{"kind":"source","path":"a.py","start_line":1}}),
        serde_json::json!({"operation":"traverse","request":{"kind":"path","start":"absent","target":"also-absent","predicates":["calls"],"budget":BoundedWalkBudget::default()}}),
        serde_json::json!({"operation":"find","request":{"kind":"location","path":"a.py","line":0}}),
    ] {
        for tier in ["compact", "explain", "export"] {
            let mut request = request.clone();
            request["output"] = serde_json::json!({"tier":tier});
            std::fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
            let result = QueryService::execute(
                &engine,
                serde_json::from_value::<QueryRequest>(request.clone()).unwrap(),
            );
            let success = result.is_ok();
            let mut expected = match result {
                Ok(value) => value.into_value().unwrap(),
                Err(error) => {
                    serde_json::json!({"schema_version":"entmoot.query.error.v1","status":"error","kind":error.code(),"message":error.to_string()})
                }
            };
            let output = cli()
                .arg("request")
                .arg("--graph")
                .arg(&graph_path)
                .arg("--request")
                .arg(&request_path)
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                success,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let mut actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            // Wall time is execution telemetry; all semantic receipts must still match.
            for value in [&mut actual, &mut expected] {
                without_telemetry(value);
            }
            assert_eq!(actual, expected);
            std::fs::write(
                &request_path,
                serde_json::to_vec(&vec![request.clone(), request]).unwrap(),
            )
            .unwrap();
            let batch = cli()
                .arg("request")
                .arg("--batch")
                .arg("--graph")
                .arg(&graph_path)
                .arg("--request")
                .arg(&request_path)
                .output()
                .unwrap();
            assert_eq!(batch.status.success(), success);
            let batch_value: serde_json::Value = serde_json::from_slice(&batch.stdout).unwrap();
            if success {
                let rows = batch_value.as_array().unwrap();
                assert_eq!(rows.len(), 2);
                for row in rows {
                    let mut row = row.clone();
                    without_telemetry(&mut row);
                    assert_eq!(row, expected);
                }
            } else {
                assert_eq!(batch_value, expected);
            }
        }
    }
    std::fs::write(
        &request_path,
        br#"{"operation":"find","request":{"kind":"subject","path":"a.py","limit":100},"output":{"tier":"export"}}"#,
    )
    .unwrap();
    let override_result = cli()
        .args(["request", "--detail", "compact", "--output-limit", "1"])
        .arg("--graph")
        .arg(&graph_path)
        .arg("--request")
        .arg(&request_path)
        .output()
        .unwrap();
    assert!(
        override_result.status.success(),
        "{}",
        String::from_utf8_lossy(&override_result.stderr)
    );
    let page: serde_json::Value = serde_json::from_slice(&override_result.stdout).unwrap();
    assert_eq!(page["tier"], "compact");
    assert_eq!(page["page"]["returned"], 1);
    assert!(override_result.stdout.len() <= 16385);
    let cursor_path = directory.path().join("cursor.json");
    std::fs::write(
        &cursor_path,
        serde_json::to_vec(&page["page"]["next_cursor"]).unwrap(),
    )
    .unwrap();
    let continuation = cli()
        .args([
            "request",
            "--detail",
            "compact",
            "--output-limit",
            "1",
            "--output-after",
        ])
        .arg(cursor_path)
        .arg("--graph")
        .arg(&graph_path)
        .arg("--request")
        .arg(&request_path)
        .output()
        .unwrap();
    assert!(
        continuation.status.success(),
        "{}",
        String::from_utf8_lossy(&continuation.stderr)
    );
    let next: serde_json::Value = serde_json::from_slice(&continuation.stdout).unwrap();
    assert_ne!(page["items"], next["items"]);
    std::fs::write(&request_path, b"[]").unwrap();
    assert!(!cli()
        .arg("request")
        .arg("--batch")
        .arg("--graph")
        .arg(&graph_path)
        .arg("--request")
        .arg(&request_path)
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(std::fs::read(&graph_path).unwrap(), before);

    let expected_status =
        QueryService::status(&engine, entmoot_contracts::query::StatusRequest::default()).unwrap();
    let status_output = cli()
        .arg("status")
        .arg("--graph")
        .arg(&graph_path)
        .output()
        .unwrap();
    assert!(
        status_output.status.success(),
        "{}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let actual_status: serde_json::Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(
        actual_status,
        serde_json::to_value(expected_status).unwrap()
    );

    // Malformed input must be rejected before even trying to open publication paths.
    std::fs::write(
        &request_path,
        br#"{"operation":"find","request":{"kind":"subject"},"unexpected":true}"#,
    )
    .unwrap();
    let missing = directory.path().join("missing.sqlite3");
    let output = cli()
        .arg("request")
        .arg("--graph")
        .arg("missing.json")
        .arg("--store")
        .arg(&missing)
        .arg("--request")
        .arg(&request_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
    assert!(!missing.exists());
}

#[test]
fn native_call_audit_emits_accounting_before_failing_the_gate() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("a.py"),
        "def leaf():\n    pass\ndef caller():\n    leaf()\n",
    )
    .unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(directory.path(), Default::default())
            .unwrap();
    let file = directory.path().join("graph.json");
    entmoot_engine::write_graph_json(&graph, &file).unwrap();
    let before = std::fs::read(&file).unwrap();
    let output = cli()
        .args(["audit-calls", "--graph"])
        .arg(&file)
        .arg("--require-accounted")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: entmoot_contracts::ResolverAccounting =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.callables.len(), 2);
    assert_eq!(report.static_work_remaining, 1);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert!(cli()
        .args(["audit-calls", "--graph"])
        .arg(&file)
        .output()
        .unwrap()
        .status
        .success());
}

fn analysis_input(marginal_value: &str) -> serde_json::Value {
    serde_json::json!({
        "policy_producer": {"id":"fixture.planner", "name":"Fixture planner"},
        "snapshot": {"repository_id":"fixture", "revision":"a".repeat(40), "source_state":"committed"},
        "subject": {"scheme":"opaque", "system":"fixture", "value":"a"},
        "method_id":"fixture.native.v1", "input_root":blake3_hex(b"inputs"), "prior_input_root":null, "current_artifact":null,
        "gates":[
          {"kind":"qualification","disposition":"accepted","claimant":{"id":"fixture.qualifier","name":"Fixture qualifier","capabilities":["deriver"]},"method_id":"fixture.qualification.v1","assumptions":["fixture corpus applies"],"limitations":["not ecosystem-wide"],"evidence_refs":[{"artifact_id":"c".repeat(64),"provider_pointer":"/qualification"}]},
          {"kind":"environment","disposition":"accepted","claimant":{"id":"fixture.environment","name":"Fixture environment","capabilities":["deriver"]},"method_id":"fixture.environment.v1","assumptions":["pinned toolchain"],"limitations":["fixture host only"],"evidence_refs":[{"artifact_id":"d".repeat(64),"provider_pointer":"/environment"}]},
          {"kind":"marginal_value","disposition":marginal_value,"claimant":{"id":"fixture.value","name":"Fixture value gate","capabilities":["deriver"]},"method_id":"fixture.value.v1","assumptions":["matched corpus required"],"limitations":["measurement pending"],"evidence_refs":[{"artifact_id":"e".repeat(64),"provider_pointer":"/measurement"}]}
        ], "relevant_unknown":true,
        "assumptions":["input root commits the declared method, task, source, and build inputs"],
        "evidence_refs":[{"artifact_id":"b".repeat(64),"provider_pointer":"/"}]
    })
}

#[test]
fn analysis_plan_is_explicit_and_does_not_publish_knowledge() {
    let directory = tempfile::tempdir().unwrap();
    let request = directory.path().join("demand.json");
    let input = analysis_input("unknown");
    std::fs::write(&request, serde_json::to_vec(&input).unwrap()).unwrap();
    for (mode, action) in [
        ("auto", "deferred"),
        ("always", "request_artifact"),
        ("never", "deferred"),
    ] {
        let result = cli()
            .args(["plan-analysis", "--request"])
            .arg(&request)
            .args(["--adapter-pass", mode])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let plan: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(plan["action"], action);
        assert!(plan["limitations"]
            .as_array()
            .is_some_and(|v| !v.is_empty()));
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn external_analysis_work_is_durable_and_fenced() {
    let directory = tempfile::tempdir().unwrap();
    let request = directory.path().join("demand.json");
    std::fs::write(
        &request,
        serde_json::to_vec(&analysis_input("accepted")).unwrap(),
    )
    .unwrap();
    let planned = cli()
        .args(["plan-analysis", "--request"])
        .arg(&request)
        .output()
        .unwrap();
    assert!(planned.status.success());
    let decision = directory.path().join("decision.json");
    std::fs::write(&decision, &planned.stdout).unwrap();
    let queue = directory.path().join("work.sqlite3");
    let enqueued = cli()
        .args(["work", "--queue"])
        .arg(&queue)
        .args(["enqueue", "--decision"])
        .arg(&decision)
        .output()
        .unwrap();
    assert!(
        enqueued.status.success(),
        "{}",
        String::from_utf8_lossy(&enqueued.stderr)
    );
    let item: serde_json::Value = serde_json::from_slice(&enqueued.stdout).unwrap();
    let id = item["work_id"].as_str().unwrap();
    let worker = directory.path().join("worker.json");
    std::fs::write(
        &worker,
        br#"{"id":"fixture.worker","name":"Fixture worker","capabilities":["resolver"]}"#,
    )
    .unwrap();
    let claimed = cli()
        .args(["work", "--queue"])
        .arg(&queue)
        .args(["claim", "--id", id, "--worker"])
        .arg(&worker)
        .output()
        .unwrap();
    assert!(
        claimed.status.success(),
        "{}",
        String::from_utf8_lossy(&claimed.stderr)
    );
    let claimed: serde_json::Value = serde_json::from_slice(&claimed.stdout).unwrap();
    assert_eq!(claimed["state"], "claimed");
    let reopened = cli()
        .args(["work", "--queue"])
        .arg(&queue)
        .args(["get", "--id", id])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&reopened.stdout).unwrap(),
        claimed
    );
}

#[test]
fn policy_audit_round_trips_v1_and_rejects_retired_modes_and_controls() {
    let output = cli().args(["policy", "--config", "v1"]).output().unwrap();
    assert!(output.status.success());
    let audit: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!audit["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "runtime_documentation"));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("policy.json");
    std::fs::write(&path, serde_json::to_vec(&audit["policy"]).unwrap()).unwrap();
    let selector = format!("@{}", path.display());
    let custom = cli()
        .args(["policy", "--config", &selector])
        .output()
        .unwrap();
    assert!(custom.status.success());
    assert_eq!(output.stdout, custom.stdout);
    assert_eq!(
        audit["policy"],
        serde_json::json!({"series":"entmoot.structural.v1"})
    );
    for field in ["preserve_semantics", "alpha_rename", "prune_type_hints"] {
        let mut invalid = audit["policy"].clone();
        invalid[field] = false.into();
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let rejected = cli()
            .args(["policy", "--config", &selector])
            .output()
            .unwrap();
        assert!(!rejected.status.success(), "{field}");
        assert!(rejected.stdout.is_empty());
    }
    for retired in ["minimal", "standard", "sound", "v2", "legacy", "aggressive"] {
        let rejected = cli()
            .args(["policy", "--config", retired])
            .output()
            .unwrap();
        assert!(!rejected.status.success(), "{retired}");
        assert!(rejected.stdout.is_empty());
    }
}

fn write_sarif_manifest(
    directory: &std::path::Path,
    graph: &entmoot_engine::TopologyGraph,
) -> Result<(std::path::PathBuf, String), Box<dyn std::error::Error>> {
    let artifact = directory.join("scanner.sarif");
    let bytes = br#"{
      "version":"2.1.0",
      "runs":[{
        "tool":{"driver":{"name":"integration-scanner","version":"1"}},
        "results":[{
          "ruleId":"TEST001",
          "message":{"text":"test finding"},
          "locations":[{"physicalLocation":{
            "artifactLocation":{"uri":"main.rs"},
            "region":{"startLine":1,"startColumn":1}
          }}],
          "codeFlows":[{"threadFlows":[{"locations":[{"location":{"physicalLocation":{
            "artifactLocation":{"uri":"main.rs"},
            "region":{"startLine":1,"startColumn":1}
          }}}]}]}]
        }]
      }]
    }"#;
    std::fs::write(&artifact, bytes)?;
    let digest = blake3_hex(bytes);
    let scope = ScopeUniverse::from_selectors(
        "integration",
        vec!["rust".into()],
        vec!["main.rs".into()],
        vec![],
        ScopeCompleteness::Partial,
        Some(&["main.rs".into()]),
    );
    let mut manifest = ArtifactManifest::build(
        ArtifactFormat::Sarif21,
        ArtifactProducer {
            name: "integration-scanner".into(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: Some("integration".into()),
            config_class: None,
        },
        graph.snapshot.clone(),
        scope,
        digest.clone(),
        None,
    )?;
    manifest.artifact_path = Some("scanner.sarif".into());
    let manifest_path = directory.join("scanner.sarif.manifest.json");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok((manifest_path, digest))
}

#[test]
fn index_rejects_a_malformed_manifest_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
    let manifest = temp.path().join("bad.manifest.json");
    std::fs::write(&manifest, "{not json}").unwrap();
    let output_path = temp.path().join("graph.json");
    let output = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/repo",
            "--out",
            output_path.to_str().unwrap(),
            "--artifact",
            manifest.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!output_path.exists());
    assert!(!temp.path().join("graph.json.knowledge.sqlite3").exists());
}

#[test]
fn attach_rejects_obsolete_topology_graph_v1_without_modifying_it() {
    let temp = tempfile::tempdir().unwrap();
    let graph = temp.path().join("graph.json");
    let original = br#"{"schema_version":"topology.graph.v1"}"#;
    std::fs::write(&graph, original).unwrap();
    let manifest = temp.path().join("artifact.manifest.json");
    std::fs::write(&manifest, "{}").unwrap();
    let output = cli()
        .args([
            "attach",
            "--graph",
            graph.to_str().unwrap(),
            "--artifact",
            manifest.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("regenerate"));
    assert_eq!(std::fs::read(&graph).unwrap(), original);
}

#[test]
fn index_publishes_reopenable_knowledge_store_and_graph_projection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
    let graph_path = temp.path().join("graph.json");
    let summary_path = temp.path().join("graph.summary.json");
    let store_path = temp.path().join("knowledge.sqlite3");

    let output = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/repo",
            "--out",
            graph_path.to_str().unwrap(),
            "--summary-out",
            summary_path.to_str().unwrap(),
            "--store",
            store_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let graph = load_graph_json(&graph_path).unwrap();
    let summary: GraphSummary =
        serde_json::from_slice(&std::fs::read(&summary_path).unwrap()).unwrap();
    let knowledge = SqliteKnowledgeRepository::open(&store_path)
        .unwrap()
        .load()
        .unwrap();
    assert!(!graph.nodes.is_empty());
    assert_eq!(summary.nodes, graph.nodes.len());
    assert_eq!(summary.policy_ids, graph.policy_ids);
    assert!(!summary.policy_ids.is_empty());
    assert!(!knowledge.structural_edges.is_empty());
    assert!(!SqliteKnowledgeRepository::open(&store_path)
        .unwrap()
        .records_for_owner("snapshot:example/repo", 10_000)
        .unwrap()
        .is_empty());
}

#[test]
fn attach_uses_graph_fallback_for_an_uninitialized_store_and_archives_artifact() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
    let graph_path = temp.path().join("graph.json");
    let initial_store = temp.path().join("initial.sqlite3");
    let index = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/repo",
            "--out",
            graph_path.to_str().unwrap(),
            "--store",
            initial_store.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        index.status.success(),
        "{}",
        String::from_utf8_lossy(&index.stderr)
    );

    let graph = load_graph_json(&graph_path).unwrap();
    let initial_structural_edges = graph.edges.len();
    let (manifest_path, digest) = write_sarif_manifest(temp.path(), &graph).unwrap();
    let new_store = temp.path().join("uninitialized.sqlite3");
    let repository = SqliteKnowledgeRepository::open(&new_store).unwrap();
    assert!(!repository.is_initialized().unwrap());
    drop(repository);
    let object_store = temp.path().join("objects");

    let attach = cli()
        .args([
            "attach",
            "--graph",
            graph_path.to_str().unwrap(),
            "--store",
            new_store.to_str().unwrap(),
            "--artifact-store",
            object_store.to_str().unwrap(),
            "--artifact",
            manifest_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        attach.status.success(),
        "{}",
        String::from_utf8_lossy(&attach.stderr)
    );

    let attached = load_graph_json(&graph_path).unwrap();
    let receipt: entmoot_contracts::ArtifactAttachReceipt =
        serde_json::from_slice(&attach.stdout).unwrap();
    assert_eq!(receipt.schema_version, "entmoot.attach.v1");
    assert_eq!(receipt.snapshot, attached.snapshot);
    assert_eq!(receipt.artifacts.len(), 1);
    assert_eq!(receipt.artifacts[0].artifact_digest, digest);
    assert!(!receipt.artifacts[0].already_present);
    assert_eq!(receipt.artifacts[0].parsed, 1);
    let durable = SqliteKnowledgeRepository::open(&new_store)
        .unwrap()
        .load()
        .unwrap();
    assert_eq!(attached.findings.len(), 1);
    assert_eq!(attached.edges.len(), initial_structural_edges);
    assert_eq!(durable.findings.len(), 1);
    assert_eq!(durable.traces.len(), 1);
    assert_eq!(durable.structural_edges.len(), initial_structural_edges);
    let retry = cli()
        .arg("attach")
        .arg("--graph")
        .arg(&graph_path)
        .arg("--store")
        .arg(&new_store)
        .arg("--artifact")
        .arg(&manifest_path)
        .output()
        .unwrap();
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let retry_receipt: entmoot_contracts::ArtifactAttachReceipt =
        serde_json::from_slice(&retry.stdout).unwrap();
    assert!(retry_receipt.artifacts[0].already_present);
    assert_eq!(
        retry_receipt.artifacts[0].artifact_id,
        receipt.artifacts[0].artifact_id
    );
    let hex = digest.strip_prefix("blake3:").unwrap();
    assert!(object_store
        .join("blake3")
        .join(&hex[..2])
        .join(&hex[2..])
        .exists());

    let inspect = cli()
        .args([
            "artifact",
            "inspect",
            "--artifact",
            temp.path().join("scanner.sarif").to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8(inspect.stdout).unwrap();
    assert!(inspect_stdout.contains("format_hint: sarif_2_1"));
    assert!(inspect_stdout.contains("normalized_findings: 1"));

    for (kind, id) in [
        ("artifact", attached.artifacts[0].artifact_id.as_str()),
        ("trace", attached.traces[0].trace_id.as_str()),
    ] {
        let query = cli()
            .args([
                "query",
                "--store",
                new_store.to_str().unwrap(),
                kind,
                "--id",
                id,
            ])
            .output()
            .unwrap();
        assert!(
            query.status.success(),
            "{}",
            String::from_utf8_lossy(&query.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&query.stdout).unwrap();
        assert_eq!(result["query_kind"], kind);
        assert_eq!(result["status"], "known");
    }

    let dependency = cli()
        .args([
            "query",
            "--store",
            new_store.to_str().unwrap(),
            "dependency",
            "--collection",
            "artifacts",
            "--record",
            attached.artifacts[0].artifact_id.as_str(),
        ])
        .output()
        .unwrap();
    assert!(dependency.status.success());
    let result: serde_json::Value = serde_json::from_slice(&dependency.stdout).unwrap();
    assert_eq!(result["query_kind"], "dependency");
    assert_eq!(result["status"], "known");
    assert_eq!(result["state"]["visible"], true);
    assert_eq!(result["state"]["ownership_managed"], true);
}

#[test]
fn exact_unresolved_expert_query_uses_the_semantic_attempt_id() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.rs"),
        "fn caller() { unresolved_target(); }\n",
    )
    .unwrap();
    let graph_path = temp.path().join("graph.json");
    let store_path = temp.path().join("knowledge.sqlite3");
    let index = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/expert-query",
            "--out",
            graph_path.to_str().unwrap(),
            "--store",
            store_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(index.status.success());
    let graph = load_graph_json(&graph_path).unwrap();
    let unresolved_id = graph.unresolved_relations[0].id.as_str();
    let query = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "unresolved",
            "--id",
            unresolved_id,
        ])
        .output()
        .unwrap();
    assert!(query.status.success());
    let result: serde_json::Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(result["query_kind"], "unresolved");
    assert_eq!(result["status"], "known");
    assert_eq!(result["unresolved"]["id"], unresolved_id);
}

#[test]
fn hash_command_emits_a_typed_digest_and_rejects_unknown_config() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.py");
    std::fs::write(&source, "def add(a, b):\n    return a + b\n").unwrap();
    let success = cli()
        .args([
            "hash",
            "--lang",
            "python",
            "--file",
            source.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(success.status.success());
    let digest = String::from_utf8(success.stdout).unwrap();
    assert!(digest.starts_with("blake3:"));
    assert_eq!(digest.len(), 71);

    let inventory = cli()
        .args([
            "hash",
            "--lang",
            "python",
            "--file",
            source.to_str().unwrap(),
            "--callable-inventory",
            "--path",
            "src/sample.py",
        ])
        .output()
        .unwrap();
    assert!(inventory.status.success());
    let record: serde_json::Value = serde_json::from_slice(&inventory.stdout).unwrap();
    assert_eq!(record["occurrences"].as_array().unwrap().len(), 1);
    assert!(record["occurrences"][0]["structural_hash"]
        .as_str()
        .unwrap()
        .starts_with("blake3:"));
    assert!(record["occurrences"][0]["logic_address"]
        .as_str()
        .unwrap()
        .contains("src/sample.py"));

    let failure = cli()
        .args([
            "hash",
            "--lang",
            "python",
            "--config",
            "unknown",
            "--file",
            source.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!failure.status.success());
}

#[test]
fn ground_truth_command_publishes_bounded_v1_dataset_results() {
    let temp = tempfile::tempdir().unwrap();
    let report_path = temp.path().join("report.json");
    let output = cli()
        .args([
            "validate-ground-truth",
            "--config",
            "v1",
            "--json-out",
            report_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: entmoot_contracts::NormalizationValidationReport =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    report
        .validate(&entmoot_engine::implementation_fingerprint())
        .unwrap();
    assert!(report.passes());
    assert_eq!(report.cases.len(), 586);
    assert_eq!(
        report
            .cases
            .iter()
            .filter(
                |case| case.outcome() == entmoot_contracts::NormalizationCaseOutcome::Unqualified
            )
            .count(),
        0
    );
    let stdout: entmoot_contracts::NormalizationValidationReport =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout, report);
}

#[test]
fn ground_truth_errors_and_empty_sets_cannot_pass_or_become_hash_comparisons() {
    use entmoot_contracts::{NormalizationCaseOutcome, NormalizationValidationReport};
    let temp = tempfile::tempdir().unwrap();
    for (name, cases, expected) in [
        ("empty", serde_json::json!([]), None),
        (
            "parse",
            serde_json::json!([{
                "id":"malformed", "language":"javascript",
                "before":"function f() { return 1; }", "after":"function f( {",
                "expectation":{"kind":"hash","relation":"different_hash"}
            }]),
            Some(NormalizationCaseOutcome::NotComparable),
        ),
        (
            "unknown",
            serde_json::json!([{
                "id":"unknown", "language":"unknown-language", "before":"x", "after":"y",
                "expectation":{"kind":"syntax","expected":{"before":"rejected","after":"rejected"}}
            }]),
            Some(NormalizationCaseOutcome::NotComparable),
        ),
    ] {
        let dataset = temp.path().join(format!("{name}.json"));
        std::fs::write(
            &dataset,
            serde_json::to_vec(&serde_json::json!({"cases":cases})).unwrap(),
        )
        .unwrap();
        let output = cli()
            .args([
                "validate-ground-truth",
                "--config",
                "v1",
                "--dataset",
                dataset.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{name}: {:?}", output);
        let report: NormalizationValidationReport = serde_json::from_slice(&output.stdout).unwrap();
        report
            .validate(&entmoot_engine::implementation_fingerprint())
            .unwrap();
        assert!(!report.passes());
        assert_eq!(report.cases.first().map(|case| case.outcome()), expected);
    }
}

#[test]
fn index_revision_mismatch_publishes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();
    let graph_path = temp.path().join("graph.json");
    let output = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/repo",
            "--revision",
            "0000000000000000000000000000000000000000",
            "--out",
            graph_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("revision mismatch"));
    assert!(!graph_path.exists());
    assert!(!temp.path().join("graph.json.knowledge.sqlite3").exists());
}

#[test]
fn query_reads_relations_and_claims_from_the_durable_store() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("main.rs"),
        "fn callee() {}\nfn caller() { callee(); }\n",
    )
    .unwrap();
    let graph_path = temp.path().join("graph.json");
    let store_path = temp.path().join("knowledge.sqlite3");
    let indexed = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/repo",
            "--out",
            graph_path.to_str().unwrap(),
            "--store",
            store_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let graph = load_graph_json(&graph_path).unwrap();
    let edge = graph
        .relation_edges
        .first()
        .expect("resolved relation edge");
    let address_path = temp.path().join("address.json");
    std::fs::write(
        &address_path,
        serde_json::to_vec_pretty(&edge.subject).unwrap(),
    )
    .unwrap();

    let relations = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "relations",
            "--address",
            address_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        relations.status.success(),
        "{}",
        String::from_utf8_lossy(&relations.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&relations.stdout).unwrap();
    assert_eq!(
        result["schema_version"],
        "entmoot.knowledge.query.result.v1"
    );
    let actual: std::collections::BTreeSet<_> = result["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["id"].as_str().unwrap())
        .collect();
    let expected: std::collections::BTreeSet<_> = graph
        .relation_edges
        .iter()
        .filter(|relation| relation.subject == edge.subject)
        .map(|relation| relation.id.as_str())
        .collect();
    assert_eq!(actual, expected);
    assert!(!result["relation_claims"].as_array().unwrap().is_empty());

    let relation_claims = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "relation-claims",
            "--relation-edge",
            edge.id.as_str(),
        ])
        .output()
        .unwrap();
    assert!(relation_claims.status.success());
    let result: serde_json::Value = serde_json::from_slice(&relation_claims.stdout).unwrap();
    assert_eq!(result["relation_edge_id"], edge.id.as_str());
    assert!(!result["relation_claims"].as_array().unwrap().is_empty());

    let proposition = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "proposition",
            "--proposition",
            edge.proposition_id.as_str(),
        ])
        .output()
        .unwrap();
    assert!(proposition.status.success());
    let result: serde_json::Value = serde_json::from_slice(&proposition.stdout).unwrap();
    assert_eq!(result["status"], "known");
    assert_eq!(result["proposition_id"], edge.proposition_id.as_str());
    assert_eq!(result["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(result["proposition"]["id"], edge.proposition_id.as_str());
    assert!(result["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .any(|occurrence| occurrence["id"] == edge.id.as_str()));

    let malformed_edge = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "relation-claims",
            "--relation-edge",
            "not-an-edge-id",
        ])
        .output()
        .unwrap();
    assert!(!malformed_edge.status.success());
    assert!(String::from_utf8_lossy(&malformed_edge.stderr)
        .contains("query relation claims by relation edge"));

    let assertions = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "assertions",
            "--subject",
            "not-present",
        ])
        .output()
        .unwrap();
    assert!(assertions.status.success());
    let result: serde_json::Value = serde_json::from_slice(&assertions.stdout).unwrap();
    assert_eq!(result["count"], 0);

    std::fs::write(
        &address_path,
        r#"{"scheme":"opaque","system":"","value":"x"}"#,
    )
    .unwrap();
    let invalid = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "relations",
            "--address",
            address_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("validate query Address"));
}

#[test]
fn query_reads_resource_occurrences_from_the_durable_store() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("deployment.yaml"),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\n",
    )
    .unwrap();
    let graph_path = temp.path().join("graph.json");
    let store_path = temp.path().join("knowledge.sqlite3");
    let indexed = cli()
        .args([
            "index",
            "--root",
            temp.path().to_str().unwrap(),
            "--repo",
            "example/resources",
            "--out",
            graph_path.to_str().unwrap(),
            "--store",
            store_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let graph = load_graph_json(&graph_path).unwrap();
    let resource = graph.resources.first().expect("configuration resource");

    let queried = cli()
        .args([
            "query",
            "--store",
            store_path.to_str().unwrap(),
            "resources",
            "--resource",
            &resource.resource_id,
        ])
        .output()
        .unwrap();
    assert!(
        queried.status.success(),
        "{}",
        String::from_utf8_lossy(&queried.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&queried.stdout).unwrap();
    assert_eq!(
        result["schema_version"],
        "entmoot.knowledge.query.result.v1"
    );
    assert_eq!(result["query_kind"], "resources");
    assert_eq!(result["count"], 1);
    assert_eq!(result["resources"][0]["resource_id"], resource.resource_id);
}

#[test]
fn transition_revalidation_is_atomic_replayable_and_keeps_historical_evidence() {
    use entmoot_contracts::{
        AnalysisAction, AnalysisPassMode, ArtifactId, EvidenceRef, RevalidationDeclaration,
        RevalidationUnit, TransitionRevalidationReceipt, TransitionRevalidationRequest,
    };
    let directory = tempfile::tempdir().unwrap();
    let build = |name: &str, source: &str| {
        let root = directory.path().join(name);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("main.rs"), source).unwrap();
        entmoot_engine::build_topology_from_repo_with_options(
            &root,
            entmoot_engine::TopologyBuildOptions {
                repository: Some("fixture"),
                policy: None,
            },
        )
        .unwrap()
    };
    let mut base = build("base", "fn main() {}\n");
    let head = build("head", "fn main() { let _x = 1; }\n");
    let (artifact_path, _) = write_sarif_manifest(directory.path(), &base).unwrap();
    entmoot_engine::ingest_artifact_manifest(&mut base, &artifact_path, false).unwrap();
    let artifact = &base.artifacts[0];
    let demand = analysis_input("accepted");
    let declaration = RevalidationDeclaration {
        policy_producer: serde_json::from_value(demand["policy_producer"].clone()).unwrap(),
        method_id: "fixture.complete-source.v1".into(),
        input_scope: ScopeUniverse::from_selectors(
            "fixture",
            vec![],
            vec!["main.rs".into()],
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&["main.rs".into()]),
        ),
        assumptions: vec!["fixed fixture scanner and build environment".into()],
        evidence_refs: vec![EvidenceRef {
            artifact_id: artifact.artifact_id.clone(),
            provider_pointer: "/runs/0".into(),
            record_id: None,
        }],
    };
    let mut request = TransitionRevalidationRequest {
        base_snapshot: base.snapshot.clone(),
        head_snapshot: head.snapshot.clone(),
        units: vec![RevalidationUnit {
            base_artifact_id: ArtifactId::parse(&artifact.artifact_id).unwrap(),
            head_artifact_id: None,
            declaration,
            subject: serde_json::from_value(demand["subject"].clone()).unwrap(),
            gates: serde_json::from_value(demand["gates"].clone()).unwrap(),
            mode: AnalysisPassMode::Auto,
        }],
        max_records: 1000,
    };
    let bp = directory.path().join("base.json");
    let hp = directory.path().join("head.json");
    entmoot_engine::write_graph_json(&base, &bp).unwrap();
    entmoot_engine::write_graph_json(&head, &hp).unwrap();
    let before = (std::fs::read(&bp).unwrap(), std::fs::read(&hp).unwrap());
    let rp = directory.path().join("request.json");
    let execute = |request: &TransitionRevalidationRequest, ledger: &std::path::Path| {
        std::fs::write(&rp, serde_json::to_vec(request).unwrap()).unwrap();
        cli()
            .args(["revalidate", "--base-graph"])
            .arg(&bp)
            .arg("--head-graph")
            .arg(&hp)
            .arg("--request")
            .arg(&rp)
            .arg("--ledger")
            .arg(ledger)
            .output()
            .unwrap()
    };
    let ledger = directory.path().join("ledger.sqlite3");
    let output = execute(&request, &ledger);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: TransitionRevalidationReceipt = serde_json::from_slice(&output.stdout).unwrap();
    let unit = &receipt.units[0];
    assert!(unit.revalidation.inputs_changed);
    assert!(!unit.revalidation.invalidated_applicability.is_empty());
    assert!(unit.revalidation.retained_applicability.is_empty());
    assert_eq!(unit.decision.action, AnalysisAction::RequestArtifact);
    let reopened = SqliteKnowledgeRepository::open_read_only(&ledger).unwrap();
    for record in &unit.publication.retained_evidence {
        assert!(reopened.record_state(record).unwrap().unwrap().visible);
    }
    for record in &unit.revalidation.invalidated_applicability {
        assert!(!reopened.record_state(record).unwrap().unwrap().visible);
    }
    drop(reopened);
    let replay = execute(&request, &directory.path().join("replay.sqlite3"));
    assert!(replay.status.success());
    assert_eq!(output.stdout, replay.stdout);
    let retained = std::fs::read(&ledger).unwrap();
    assert!(!execute(&request, &ledger).status.success());
    assert_eq!(std::fs::read(&ledger).unwrap(), retained);
    request.max_records = 1;
    let rejected_path = directory.path().join("rejected.sqlite3");
    let rejected = execute(&request, &rejected_path);
    assert!(!rejected.status.success());
    assert!(!rejected_path.exists());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&rejected.stdout).unwrap()["kind"],
        "budget_exceeded"
    );
    request.max_records = 1000;
    request.units[0].gates.pop();
    let invalid = execute(&request, &rejected_path);
    assert!(!invalid.status.success());
    assert!(!rejected_path.exists());
    assert_eq!(
        before,
        (std::fs::read(&bp).unwrap(), std::fs::read(&hp).unwrap())
    );
    // A freshly attached artifact can establish head applicability, but its
    // own configuration root must drive the plan, not the old producer root.
    let mut fresh_head = head;
    let (fresh_path, digest) = write_sarif_manifest(directory.path(), &fresh_head).unwrap();
    let original_manifest: ArtifactManifest =
        serde_json::from_slice(&std::fs::read(&fresh_path).unwrap()).unwrap();
    let mut fresh_manifest = ArtifactManifest::build(
        original_manifest.format,
        original_manifest.producer,
        original_manifest.snapshot,
        original_manifest.scope,
        digest,
        Some(blake3_hex(b"new explicit scanner configuration")),
    )
    .unwrap();
    fresh_manifest.artifact_path = Some("scanner.sarif".into());
    std::fs::write(&fresh_path, serde_json::to_vec(&fresh_manifest).unwrap()).unwrap();
    entmoot_engine::ingest_artifact_manifest(&mut fresh_head, &fresh_path, false).unwrap();
    entmoot_engine::write_graph_json(&fresh_head, &hp).unwrap();
    request.units[0].gates = serde_json::from_value(demand["gates"].clone()).unwrap();
    request.units[0].head_artifact_id =
        Some(ArtifactId::parse(&fresh_manifest.artifact_id).unwrap());
    let fresh_output = execute(&request, &directory.path().join("fresh.sqlite3"));
    assert!(
        fresh_output.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh_output.stderr)
    );
    let fresh_receipt: TransitionRevalidationReceipt =
        serde_json::from_slice(&fresh_output.stdout).unwrap();
    let fresh_unit = &fresh_receipt.units[0];
    let fresh_publication = fresh_unit.reingested.as_ref().unwrap();
    assert_eq!(fresh_unit.decision.action, AnalysisAction::ReuseCandidate);
    assert_eq!(
        fresh_unit.decision.demand.input_root,
        fresh_publication.input_root.id
    );
    assert_ne!(
        fresh_unit.decision.demand.input_root,
        fresh_unit.revalidation.current_input_root.id
    );

    // Independent CLI invocations recover the same actual ledger through the queue.
    let shared = directory.path().join("shared.sqlite3");
    let b = entmoot_engine::collaboration::register_graph(&shared, &base, "base", "agent", None, 0)
        .unwrap();
    let h = entmoot_engine::collaboration::register_graph(
        &shared,
        &fresh_head,
        "head",
        "agent",
        None,
        0,
    )
    .unwrap();
    let queued = entmoot_contracts::RevalidationRequest {
        base_view: b.view,
        head_view: h.view,
        units: request.units.clone(),
    };
    std::fs::write(&rp, serde_json::to_vec(&queued).unwrap()).unwrap();
    let invoke = |args: &[&str]| {
        let output = cli()
            .arg("collaborate")
            .arg("--store")
            .arg(&shared)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let submitted = invoke(&[
        "submit-revalidation",
        "--request",
        rp.to_str().unwrap(),
        "--key",
        "cli",
        "--owner",
        "agent",
    ]);
    let id = submitted["id"].as_str().unwrap();
    let paused = invoke(&[
        "run-revalidation",
        "--id",
        id,
        "--worker",
        "agent",
        "--steps",
        "1",
        "--max-records",
        "1",
    ]);
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["checkpoint_sequence"], 0);
    invoke(&["resume", "--id", id]);
    let prefix = invoke(&[
        "run-revalidation",
        "--id",
        id,
        "--worker",
        "agent",
        "--steps",
        "1",
    ]);
    assert_eq!(prefix["checkpoint"]["completed_units"], 1);
    invoke(&["resume", "--id", id]);
    let done = invoke(&[
        "run-revalidation",
        "--id",
        id,
        "--worker",
        "agent",
        "--steps",
        "1",
    ]);
    assert_eq!(done["state"], "completed");
    let outputs = invoke(&["outputs", "--id", id]);
    assert_eq!(
        outputs["outputs"][0]["value"],
        serde_json::to_value(fresh_unit).unwrap()
    );
    let invalidated = &fresh_unit.revalidation.invalidated_applicability[0];
    let state = invoke(&[
        "revalidation-record",
        "--id",
        id,
        "--collection",
        &invalidated.collection,
        "--record-id",
        &invalidated.record_id,
    ]);
    assert_eq!(state["visible"], false);
    let state = invoke(&[
        "revalidation-record",
        "--id",
        id,
        "--collection",
        "findings",
        "--record-id",
        "unreported",
    ]);
    assert!(state["visible"].is_null());
}
