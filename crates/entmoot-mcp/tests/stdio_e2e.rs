#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_engine::{
    build_topology_from_repo_with_options, write_graph_json, KnowledgeRepository, KnowledgeStore,
    SqliteKnowledgeRepository, TopologyBuildOptions,
};
use rmcp::{
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
    ServiceExt,
};

#[test]
fn invalid_storage_configuration_refuses_stdio_startup_without_bootstrapping() {
    for bad in [
        "broken JSON",
        r#"{"schema":"entmoot.config.v1","mode":"remote"}"#,
        r#"{"schema":"entmoot.config.v1","mode":"local","volume":"https://example.invalid/store"}"#,
    ] {
        for global in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let source = fixture.path().join("source");
            let data = fixture.path().join("data");
            std::fs::create_dir(&source).unwrap();
            std::fs::write(source.join("a.py"), "def leaf(): return 1\n").unwrap();
            let namespace = entmoot_engine::workspace::workspace_namespace(&source, &data);
            let config = if global {
                data.join("config.json")
            } else {
                namespace.join("config.json")
            };
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::write(&config, bad).unwrap();
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp"))
                .arg("-C")
                .arg(&source)
                .arg("--data-dir")
                .arg(&data)
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "accepted {bad:?}, global={global}"
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
            assert_eq!(std::fs::read_to_string(config).unwrap(), bad);
            assert!(!namespace.join("store").exists());
            assert_eq!(std::fs::read_dir(source).unwrap().count(), 1);
        }
    }
}

#[tokio::test]
async fn automatic_workspace_and_boundary_query_survive_stdio_restart() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    let data = fixture.path().join("data");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("app.py"),
        "def work(object):\n    object.toString()\n",
    )
    .unwrap();
    let pointer = entmoot_engine::workspace::storage_directory_in(&source, Some(&data))
        .unwrap()
        .join("current.json");
    let mut previous = None;
    for _ in 0..2 {
        let transport = TokioChildProcess::new(
            tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|cmd| {
                cmd.arg("-C").arg(&source).arg("--data-dir").arg(&data);
            }),
        )
        .unwrap();
        let client = ().serve(transport).await.unwrap();
        let response = client.call_tool(CallToolRequestParams::new("boundaries").with_arguments(
            serde_json::from_value(serde_json::json!({"request":{"callee":"*.toString()","outcome":"unreported"}})).unwrap()
        )).await.unwrap();
        assert_ne!(response.is_error, Some(true));
        let value = response.structured_content.unwrap();
        assert_eq!(value["result_bounds"]["total_matches"], 1);
        assert_eq!(
            value["completeness"]["repository"]["status"],
            "not_established"
        );
        assert_eq!(
            value["completeness"]["dependencies"]["status"],
            "not_established"
        );
        client.cancel().await.unwrap();
        let current = std::fs::read(&pointer).unwrap();
        if let Some(previous) = &previous {
            assert_eq!(previous, &current);
        }
        previous = Some(current);
    }
    assert!(!source.join(".entmoot").exists());
}

#[tokio::test]
async fn scheduler_mcp_configuration_is_persistent_and_uses_the_engine_contract() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("main.rs"), "fn leaf() {}\n").unwrap();
    let graph = build_topology_from_repo_with_options(
        directory.path(),
        TopologyBuildOptions {
            repository: Some("mcp/scheduler"),
            ..Default::default()
        },
    )
    .unwrap();
    let graph_path = directory.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let store = directory.path().join("shared.db");
    let transport = || {
        TokioChildProcess::new(
            tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|cmd| {
                cmd.arg("--graph")
                    .arg(&graph_path)
                    .arg("--collaboration-store")
                    .arg(&store)
                    .args(["--operation-workers", "3", "--operation-queue", "0"]);
            }),
        )
        .unwrap()
    };
    let read = || {
        CallToolRequestParams::new("collaboration_read").with_arguments(serde_json::from_value(serde_json::json!({"request":{"operation":"scheduler","repository_id":"mcp/scheduler"}})).unwrap())
    };
    let configure = |generation| {
        CallToolRequestParams::new("collaboration_write").with_arguments(serde_json::from_value(serde_json::json!({"request":{"operation":"configure_scheduler","repository_id":"mcp/scheduler","expected_generation":generation,"policy":{"max_running":3,"max_binding":0,"work_units":5,"costs":{"canon":2}}}})).unwrap())
    };
    let client = ().serve(transport()).await.unwrap();
    assert_eq!(client.call_tool(read()).await.unwrap().is_error, Some(true));
    assert!(!store.exists());
    let configured = client.call_tool(configure(0)).await.unwrap();
    assert_ne!(configured.is_error, Some(true), "{configured:?}");
    let configured = configured.structured_content.unwrap();
    assert_eq!(configured["generation"], 1);
    assert_eq!(
        client.call_tool(configure(0)).await.unwrap().is_error,
        Some(true)
    );
    client.cancel().await.unwrap();
    let client = ().serve(transport()).await.unwrap();
    let observed = client.call_tool(read()).await.unwrap();
    assert_ne!(observed.is_error, Some(true), "{observed:?}");
    let observed = observed.structured_content.unwrap();
    assert_eq!(observed["configuration"], configured);
    assert_eq!(observed["active"], serde_json::json!([]));
    assert_eq!(observed["waiting_total"], 0);
    let shared = entmoot_engine::collaboration::read(
        &store,
        entmoot_contracts::CollaborationRead::Scheduler {
            repository_id: "mcp/scheduler".into(),
        },
    )
    .unwrap();
    assert_eq!(shared["configuration"], configured);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn mcp_binding_submission_and_pause_require_explicit_host_scope_and_survive_restart() {
    use entmoot_contracts::{
        BindingFamily, BindingPlan, ContentDigest, ProducerPublication, ProducerRequest,
        ProducerTask,
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("app.ts"),
        "function leaf() {} function run() { leaf(); }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            repository: Some("mcp/binding"),
            ..Default::default()
        },
    )
    .unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let store = dir.path().join("operations.db");
    let registered =
        entmoot_engine::collaboration::register_graph(&store, &graph, "topic", "agent", None, 0)
            .unwrap();
    let environment =
        ContentDigest::parse(entmoot_contracts::blake3_hex(b"pinned-mcp-environment")).unwrap();
    let request = ProducerRequest {
        source: entmoot_engine::producer_operations::observe_source(&root, "mcp/binding").unwrap(),
        publication: ProducerPublication {
            workspace_id: "topic".into(),
            publisher: "agent".into(),
            branch: None,
            expected_generation: 1,
        },
        task: ProducerTask::Binding {
            view: registered.view,
            plan: BindingPlan {
                families: vec![BindingFamily::Oxc],
                environment: environment.clone(),
                rust_profile: Default::default(),
            },
        },
    };
    let transport = |host_root: bool| {
        TokioChildProcess::new(
            tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|cmd| {
                cmd.arg("--graph")
                    .arg(&graph_path)
                    .arg("--collaboration-store")
                    .arg(&store)
                    .env("ENTMOOT_PRODUCER", dir.path().join("missing-worker"));
                if host_root {
                    cmd.arg("--collaboration-source-root").arg(&root);
                }
            }),
        )
        .unwrap()
    };
    let call = |request: serde_json::Value| {
        CallToolRequestParams::new("collaboration_write")
            .with_arguments(serde_json::from_value(serde_json::json!({"request":request})).unwrap())
    };
    let client = ().serve(transport(false)).await.unwrap();
    let submitted = client.call_tool(call(serde_json::json!({"operation":"submit_producer","request":request,"key":"native","owner":"agent"}))).await.unwrap();
    assert_ne!(submitted.is_error, Some(true), "{submitted:?}");
    let id = submitted.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let step = || {
        call(
            serde_json::json!({"operation":"run_binding","id":id,"worker":"agent","environment":environment,"steps":1,"timeout_seconds":20,"wall_time_ms":0}),
        )
    };
    let unavailable = client.call_tool(step()).await.unwrap();
    assert_eq!(unavailable.is_error, Some(true));
    client.cancel().await.unwrap();
    let client = ().serve(transport(true)).await.unwrap();
    let paused = client.call_tool(step()).await.unwrap();
    assert_ne!(paused.is_error, Some(true), "{paused:?}");
    let paused = paused.structured_content.unwrap();
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["checkpoint_sequence"], 0);
    let cancelled = client
        .call_tool(call(
            serde_json::json!({"operation":"control","id":id,"control":"cancel"}),
        ))
        .await
        .unwrap();
    assert_ne!(cancelled.is_error, Some(true), "{cancelled:?}");
    assert_eq!(cancelled.structured_content.unwrap()["state"], "cancelled");
    client.cancel().await.unwrap();
    use entmoot_engine::LocalOperations;
    assert_eq!(
        SqliteKnowledgeRepository::open_operations_read_only(&store)
            .unwrap()
            .workspaces("mcp/binding")
            .unwrap()[0]
            .generation,
        1
    );
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

#[tokio::test]
async fn mcp_raw_git_capture_uses_shared_generation_fence_and_host_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    std::fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("app.py"), "value = 1\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    std::fs::write(root.join("app.py"), "value = 2\n").unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            repository: Some("mcp/git"),
            ..Default::default()
        },
    )
    .unwrap();
    let graph_path = temp.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let store = temp.path().join("shared.db");
    entmoot_engine::collaboration::register_graph(&store, &graph, "topic", "agent", None, 0)
        .unwrap();
    for name in ["peer", "another"] {
        entmoot_engine::collaboration::register_graph(&store, &graph, name, "agent", None, 0)
            .unwrap();
        entmoot_engine::collaboration_git::capture_workspace(
            &store, &root, name, 1, "agent", 100, 1024,
        )
        .unwrap();
    }
    let transport = || {
        TokioChildProcess::new(
            tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|cmd| {
                cmd.arg("--graph")
                    .arg(&graph_path)
                    .arg("--collaboration-store")
                    .arg(&store)
                    .arg("--collaboration-source-root")
                    .arg(&root);
            }),
        )
        .unwrap()
    };
    let request = || {
        CallToolRequestParams::new("collaboration_write").with_arguments(serde_json::from_value(serde_json::json!({"request":{"operation":"capture_git","workspace_id":"topic","expected_generation":1,"publisher":"agent","max_files":100,"max_bytes":1024}})).unwrap())
    };
    let client = ().serve(transport()).await.unwrap();
    let first = client.call_tool(request()).await.unwrap();
    assert_ne!(first.is_error, Some(true), "{first:?}");
    let first = first.structured_content.unwrap();
    assert_eq!(first["generation"], 2);
    assert!(first["git_capture"].is_string());
    client.cancel().await.unwrap();
    let client = ().serve(transport()).await.unwrap();
    let second = client.call_tool(request()).await.unwrap();
    assert_ne!(second.is_error, Some(true), "{second:?}");
    assert_eq!(second.structured_content.unwrap(), first);
    let call = |tool: &str, request: serde_json::Value| {
        CallToolRequestParams::new(tool.to_owned())
            .with_arguments(serde_json::from_value(serde_json::json!({"request":request})).unwrap())
    };
    let selected=client.call_tool(call("collaboration_read",serde_json::json!({"operation":"capture_integration","repository_id":"mcp/git","workspace":"topic","members":["peer","another"]}))).await.unwrap();
    assert_ne!(selected.is_error, Some(true), "{selected:?}");
    let selected = selected.structured_content.unwrap();
    assert_eq!(selected["members"][0]["workspace_id"], "peer");
    let submitted=client.call_tool(call("collaboration_write",serde_json::json!({"operation":"submit_integration","request":selected,"key":"ordered","owner":"agent"}))).await.unwrap();
    assert_ne!(submitted.is_error, Some(true), "{submitted:?}");
    let id = submitted.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let step = || {
        call(
            "collaboration_write",
            serde_json::json!({"operation":"run_integration","id":id,"worker":"agent","steps":1,"max_staged_bytes":1048576}),
        )
    };
    let partial = client.call_tool(step()).await.unwrap();
    assert_ne!(partial.is_error, Some(true), "{partial:?}");
    assert_eq!(partial.structured_content.unwrap()["state"], "paused");
    client.cancel().await.unwrap();
    let client = ().serve(transport()).await.unwrap();
    let resumed = client
        .call_tool(call(
            "collaboration_write",
            serde_json::json!({"operation":"control","id":id,"control":"resume"}),
        ))
        .await
        .unwrap();
    assert_ne!(resumed.is_error, Some(true), "{resumed:?}");
    let done = client.call_tool(step()).await.unwrap();
    assert_ne!(done.is_error, Some(true), "{done:?}");
    let done = done.structured_content.unwrap();
    assert_eq!(done["state"], "completed");
    assert_eq!(done["result"]["status"], "clean");
    assert_eq!(done["result"]["integrated_members"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn structural_producer_recovers_across_mcp_server_restarts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("app.py"), "def target(): return 7\n").unwrap();
    std::fs::write(root.join("helper.rs"), "fn helper() -> bool { true }\n").unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            repository: Some("mcp/producer"),
            ..Default::default()
        },
    )
    .unwrap();
    let graph_path = tmp.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let store = tmp.path().join("collaboration.db");
    let transport = || {
        TokioChildProcess::new(
            tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|cmd| {
                cmd.arg("--graph")
                    .arg(&graph_path)
                    .arg("--collaboration-store")
                    .arg(&store)
                    .arg("--collaboration-source-root")
                    .arg(&root);
            }),
        )
        .unwrap()
    };
    let client = ().serve(transport()).await.unwrap();
    let source = client.call_tool(CallToolRequestParams::new("collaboration_read").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"source","repository_id":"mcp/producer"}})).unwrap()
    )).await.unwrap();
    assert_ne!(source.is_error, Some(true));
    assert!(!store.exists());
    let request = entmoot_contracts::ProducerRequest {
        source: serde_json::from_value(source.structured_content.unwrap()).unwrap(),
        publication: entmoot_contracts::ProducerPublication {
            workspace_id: "topic".into(),
            branch: None,
            publisher: "mcp".into(),
            expected_generation: 0,
        },
        task: entmoot_contracts::ProducerTask::Inventory {
            policy: Default::default(),
        },
    };
    let submitted = client.call_tool(CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"submit_producer","request":request,"key":"index","owner":"agent"}})).unwrap()
    )).await.unwrap();
    assert_ne!(submitted.is_error, Some(true));
    let id = submitted.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let slice = |worker: &str, steps: usize| {
        CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"run_producer","id":id,"worker":worker,"steps":steps}})).unwrap()
    )
    };
    let staged = client.call_tool(slice("first", 2)).await.unwrap();
    assert_ne!(staged.is_error, Some(true), "{staged:?}");
    let staged = staged.structured_content.unwrap();
    assert_eq!(staged["state"], "paused");
    assert_eq!(staged["checkpoint"]["inventory"]["reused_files"], 0);
    assert_eq!(
        staged["checkpoint"]["inventory"]["reused_extraction_files"],
        0
    );
    client.cancel().await.unwrap();
    let client = ().serve(transport()).await.unwrap();
    let resumed = client.call_tool(CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"control","id":id,"control":"resume"}})).unwrap()
    )).await.unwrap();
    assert_ne!(resumed.is_error, Some(true));
    let done = client.call_tool(slice("replacement", 3)).await.unwrap();
    assert_ne!(done.is_error, Some(true), "{done:?}");
    let done = done.structured_content.unwrap();
    assert_eq!(done["state"], "completed");
    assert_eq!(done["result"]["workspace_id"], "topic");
    assert_eq!(done["result"]["generation"], 1);
    let registry = entmoot_engine::collaboration::read(
        &store,
        entmoot_contracts::CollaborationRead::Workspaces {
            repository_id: "mcp/producer".into(),
        },
    )
    .unwrap();
    assert_eq!(registry, serde_json::json!([done["result"]]));
    let mut canon_request = request;
    canon_request.publication.expected_generation = 1;
    canon_request.task = entmoot_contracts::ProducerTask::Canon {
        view: serde_json::from_value(done["result"]["view"].clone()).unwrap(),
    };
    let submitted = client.call_tool(CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"submit_producer","request":canon_request,"key":"canon","owner":"agent"}})).unwrap()
    )).await.unwrap();
    assert_ne!(submitted.is_error, Some(true), "{submitted:?}");
    let canon_id = submitted.structured_content.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let canon_slice = |steps: usize| {
        CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"run_producer","id":canon_id,"worker":"canon","steps":steps,"files_per_step":1}})).unwrap()
    )
    };
    let partial = client.call_tool(canon_slice(1)).await.unwrap();
    assert_ne!(partial.is_error, Some(true), "{partial:?}");
    assert_eq!(
        partial.structured_content.unwrap()["checkpoint"]["canon"],
        serde_json::json!({"completed_files":1,"total_files":2})
    );
    client.cancel().await.unwrap();
    let client = ().serve(transport()).await.unwrap();
    let resumed = client.call_tool(CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"control","id":canon_id,"control":"resume"}})).unwrap()
    )).await.unwrap();
    assert_ne!(resumed.is_error, Some(true));
    let completed = client.call_tool(canon_slice(2)).await.unwrap();
    assert_ne!(completed.is_error, Some(true), "{completed:?}");
    let completed = completed.structured_content.unwrap();
    assert_eq!(completed["state"], "completed");
    assert_eq!(completed["result"]["generation"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn stdio_server_lists_and_calls_navigation_tools() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("main.rs"),
        "fn callee() {}\nfn caller() { callee(); }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("config.java"),
        "class Config { static final int LIMIT = -1; }",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        directory.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("example/mcp"),
        },
    )
    .unwrap();
    let caller_id = graph
        .nodes
        .iter()
        .find(|node| node.kind == entmoot_engine::NodeKind::Callable && node.label == "caller")
        .unwrap()
        .id
        .clone();
    let callee_id = graph
        .nodes
        .iter()
        .find(|node| node.kind == entmoot_engine::NodeKind::Callable && node.label == "callee")
        .unwrap()
        .id
        .clone();
    let graph_path = directory.path().join("graph.json");
    let store_path = directory.path().join("graph.json.knowledge.sqlite3");
    write_graph_json(&graph, &graph_path).unwrap();
    let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
    repository
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    repository.store_inventory(&graph, None).unwrap();
    let registered = entmoot_engine::collaboration::register_graph(
        &store_path,
        &graph,
        "main",
        "fixture",
        Some("main".into()),
        0,
    )
    .unwrap();

    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|command| {
            command
                .arg("--graph")
                .arg(&graph_path)
                .arg("--compare-graph")
                .arg(&graph_path)
                .arg("--collaboration-store")
                .arg(&store_path);
        }),
    )
    .unwrap();
    let client = ().serve(transport).await.unwrap();
    // Handshake/tools/list must succeed without waiting for publication open to finish.
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    for required in [
        "status",
        "find",
        "query",
        "compatibility",
        "impact",
        "neighborhood",
        "assess_clones",
        "assess_unused",
        "assess_test_impact",
        "collaboration_read",
        "collaboration_write",
    ] {
        assert!(
            names.contains(&required),
            "missing tool {required} in {names:?}"
        );
    }
    let local = client
        .call_tool(
            CallToolRequestParams::new("collaboration_read").with_arguments(
                serde_json::from_value(
                    serde_json::json!({"request":{"operation":"workspaces","repository_id":"example/mcp"}}),
                )
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(local.is_error, Some(true));
    let text = serde_json::to_string(&local).unwrap();
    assert!(text.contains(registered.view.as_str()));
    // Retained-view revalidation needs no source-root producer configuration.
    let call = |tool: &'static str, request: serde_json::Value| {
        CallToolRequestParams::new(tool)
            .with_arguments(serde_json::from_value(serde_json::json!({"request":request})).unwrap())
    };
    let submitted = client.call_tool(call("collaboration_write", serde_json::json!({
        "operation":"submit_revalidation", "request":{"base_view":registered.view,"head_view":registered.view,"units":[]},"key":"empty-revalidation","owner":"agent"
    }))).await.unwrap();
    assert_ne!(submitted.is_error, Some(true), "{submitted:?}");
    let submitted = submitted.structured_content.unwrap();
    let id = submitted["id"].as_str().unwrap();
    let paused = client.call_tool(call("collaboration_write", serde_json::json!({"operation":"run_revalidation","id":id,"worker":"agent","steps":1,"max_records":100,"wall_time_ms":0}))).await.unwrap();
    assert_eq!(paused.structured_content.unwrap()["state"], "paused");
    let resumed = client
        .call_tool(call(
            "collaboration_write",
            serde_json::json!({"operation":"control","id":id,"control":"resume"}),
        ))
        .await
        .unwrap();
    assert_ne!(resumed.is_error, Some(true));
    let completed = client.call_tool(call("collaboration_write", serde_json::json!({"operation":"run_revalidation","id":id,"worker":"agent","steps":1,"max_records":100}))).await.unwrap();
    assert_eq!(completed.structured_content.unwrap()["state"], "completed");
    let record = entmoot_contracts::KnowledgeRecordRef::new("findings", "unreported");
    let read = client
        .call_tool(call(
            "collaboration_read",
            serde_json::json!({"operation":"revalidation_record","id":id,"record":record}),
        ))
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true));
    assert_eq!(
        read.structured_content.unwrap(),
        serde_json::to_value(
            entmoot_engine::revalidation_operations::record_state(&store_path, id, record).unwrap()
        )
        .unwrap()
    );
    let stale = client.call_tool(CallToolRequestParams::new("collaboration_write").with_arguments(
        serde_json::from_value(serde_json::json!({"request":{"operation":"publish","publication":registered,"expected_generation":9}})).unwrap()
    )).await.unwrap();
    assert_eq!(stale.is_error, Some(true));
    let early_status = client
        .call_tool(
            CallToolRequestParams::new("status")
                .with_arguments(serde_json::from_value(serde_json::json!({})).unwrap()),
        )
        .await
        .unwrap();
    assert_ne!(
        early_status.is_error,
        Some(true),
        "tools/call status must succeed once the deferred open completes"
    );
    // Exact payload parity includes evidence receipts, budgets, and unknown outcomes.
    let engine = entmoot_engine::QueryEngine::new(graph.clone(), store_path.clone())
        .with_source_store(std::sync::Arc::new(
            entmoot_engine::GixSourceStore::open(directory.path()).unwrap(),
        ));
    let paired = engine.clone().with_compare_graph(graph.clone());
    for (name, expected) in [
        (
            "assess_clones",
            serde_json::to_value(engine.clones(Default::default()).unwrap()).unwrap(),
        ),
        (
            "assess_unused",
            serde_json::to_value(engine.unused(Default::default()).unwrap()).unwrap(),
        ),
        (
            "assess_test_impact",
            serde_json::to_value(paired.test_impact(Default::default()).unwrap()).unwrap(),
        ),
    ] {
        let result = client
            .call_tool(CallToolRequestParams::new(name).with_arguments(Default::default()))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
        assert_eq!(result.structured_content.unwrap(), expected, "{name}");
    }
    for request in [
        serde_json::json!({"operation":"find","request":{"kind":"subject","query":"caller","limit":5}}),
        serde_json::json!({"operation":"inspect","request":{"kind":"trace","id":"absent"}}),
        serde_json::json!({"operation":"read","request":{"kind":"source","path":"main.rs","start_line":1}}),
        serde_json::json!({"operation":"traverse","request":{"kind":"path","start":caller_id,"target":callee_id,"predicates":["calls"],"budget":entmoot_contracts::query::BoundedWalkBudget::default()}}),
        serde_json::json!({"operation":"find","request":{"kind":"location","path":"main.rs","line":0}}),
    ] {
        for tier in ["compact", "explain", "export"] {
            let mut request = request.clone();
            request["output"] = serde_json::json!({"tier":tier});
            let expected = entmoot_contracts::query::QueryService::execute(
                &engine,
                serde_json::from_value(request.clone()).unwrap(),
            );
            let failed = expected.is_err();
            let mut expected = match expected {
                Ok(value) => value.into_value().unwrap(),
                Err(error) => {
                    serde_json::json!({"schema_version":"entmoot.query.error.v1","status":"error","kind":error.code(),"message":error.to_string()})
                }
            };
            let result = client
                .call_tool(CallToolRequestParams::new("query").with_arguments(
                    serde_json::from_value(serde_json::json!({"request":request})).unwrap(),
                ))
                .await
                .unwrap();
            assert_eq!(result.is_error == Some(true), failed);
            let mut actual = result.structured_content.unwrap();
            // Wall time is execution telemetry; all semantic receipts must still match.
            for value in [&mut actual, &mut expected] {
                without_telemetry(value);
            }
            assert_eq!(actual, expected);
        }
    }
    assert_eq!(
        names,
        [
            "assess_clones",
            "assess_overlay",
            "assess_test_impact",
            "assess_unused",
            "boundaries",
            "call_components",
            "callees",
            "callers",
            "collaboration_read",
            "collaboration_write",
            "compatibility",
            "find",
            "impact",
            "inspect",
            "inventory",
            "neighborhood",
            "query",
            "read_source",
            "relay_impact",
            "status",
            "trace_seeded_paths",
            "traverse",
        ]
    );
    // Omit outputSchema from tools/list: Cursor (and similar hosts) silently drop
    // oversized lists (~1MB with duplicated QueryResponse schemas). Input schemas
    // and structured_content at call time remain; wire truth stays in contracts.
    assert!(tools.iter().all(|tool| tool.output_schema.is_none()));
    let catalog = entmoot_engine::compatibility::CompatibilityCatalog::new(
        entmoot_engine::compatibility::PackagePublication::load(&graph_path, Some(&store_path))
            .unwrap(),
        vec![],
    )
    .unwrap();
    let expected = catalog.assess(catalog.request()).unwrap();
    let compatibility = client
        .call_tool(
            CallToolRequestParams::new("compatibility")
                .with_arguments(serde_json::from_value(serde_json::json!({})).unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        compatibility.structured_content.unwrap(),
        serde_json::to_value(expected).unwrap()
    );

    let arguments =
        serde_json::from_value(serde_json::json!({ "kind": "subject", "query": "callee" }))
            .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("find").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(!result.content.is_empty());
    assert_eq!(
        result.structured_content.as_ref().unwrap()["tier"],
        "compact"
    );

    let status = client
        .call_tool(
            CallToolRequestParams::new("status")
                .with_arguments(serde_json::from_value(serde_json::json!({})).unwrap()),
        )
        .await
        .unwrap();
    assert_ne!(status.is_error, Some(true));
    let status_body = status.structured_content.as_ref().unwrap();
    assert_eq!(status_body["schema_version"], "entmoot.query.status.v1");
    let expected_status =
        entmoot_contracts::query::QueryService::status(&engine, Default::default()).unwrap();
    assert_eq!(status_body, &serde_json::to_value(expected_status).unwrap());

    let initializer_args =
        serde_json::json!({"kind":"initializer","integer":"-1","output":{"tier":"export"}});
    let initializer_result = client
        .call_tool(
            CallToolRequestParams::new("find")
                .with_arguments(serde_json::from_value(initializer_args.clone()).unwrap()),
        )
        .await
        .unwrap();
    assert_ne!(initializer_result.is_error, Some(true));
    let expected = entmoot_contracts::query::QueryService::execute(&engine, serde_json::from_value(serde_json::json!({
        "operation":"find","request":{"kind":"initializer","integer":"-1"},"output":{"tier":"export"}
    })).unwrap()).unwrap().into_value().unwrap();
    assert_eq!(
        initializer_result.structured_content.as_ref().unwrap(),
        &expected
    );
    assert_eq!(expected["result"]["items"].as_array().unwrap().len(), 1);

    let facet = client
        .call_tool(
            CallToolRequestParams::new("find").with_arguments(
                serde_json::from_value(serde_json::json!({
                    "kind": "high_fan_in_or_out",
                    "direction": "either",
                    "min_degree": 1,
                    "limit": 5
                }))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(facet.is_error, Some(true));
    assert_eq!(
        facet.structured_content.as_ref().unwrap()["items"][0]["record_kind"],
        "facet"
    );

    let arguments = serde_json::from_value(serde_json::json!({
        "kind": "location",
        "path": "main.rs",
        "line": 2,
        "column": 15
    }))
    .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("find").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(
        {
            let body = result.structured_content.as_ref().unwrap();
            &body["subjects"][body["items"][0]["subject"].as_str().unwrap()]["kind"]
        },
        "call_stub"
    );

    let arguments = serde_json::from_value(serde_json::json!({
        "path": "main.rs",
        "start_line": 2,
        "context_lines": 1
    }))
    .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("read_source").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.as_ref().unwrap()["items"][0]["kind"],
        "source_line"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["status"],
        "known"
    );
    let arguments = serde_json::from_value(serde_json::json!({
        "kind": "trace",
        "id": "finding-that-does-not-exist"
    }))
    .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("inspect").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.as_ref().unwrap()["tier"],
        "compact"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["status"],
        "unknown"
    );

    let arguments = serde_json::from_value(serde_json::json!({
        "kind": "location",
        "path": "main.rs"
    }))
    .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("find").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.structured_content.as_ref().unwrap()["kind"],
        "invalid_request"
    );

    let arguments =
        serde_json::from_value(serde_json::json!({"kind": "evidence_role", "role": "sink"}))
            .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("find").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.as_ref().unwrap()["tier"],
        "compact"
    );

    let arguments = serde_json::from_value(serde_json::json!({
        "start": caller_id,
        "target": callee_id,
        "predicates": ["calls"],
        "max_hops": 1,
        "max_evidence_per_hop": 0
    }))
    .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("traverse").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result.structured_content.as_ref().unwrap()["tier"],
        "compact"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["status"],
        "known"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["items"][0]["classification"],
        "candidate"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["query"]["request"]["budget"]
            ["max_evidence_per_hop"],
        0
    );
    let body = result.structured_content.as_ref().unwrap();
    let hop = &body["edges"][body["items"][0]["edges"][0].as_str().unwrap()];
    assert!(hop["claim_count"].as_u64().unwrap() > 0);
    assert_eq!(body["details_deferred"], true);
    assert_eq!(body["search_truncated"], false);

    let arguments =
        serde_json::from_value(serde_json::json!({"subject": caller_id, "limit": 10})).unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("callees").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let calls = result.structured_content.as_ref().unwrap();
    assert_eq!(calls["tier"], "compact");
    assert_eq!(calls["query"]["request"]["kind"], "callees");
    assert_eq!(
        calls["edges"][calls["items"][0]["edge"].as_str().unwrap()]["object"],
        callee_id
    );
    assert_eq!(
        calls["gaps"][0],
        "repository_call_graph_completeness_not_established"
    );

    let arguments =
        serde_json::from_value(serde_json::json!({"subject": callee_id, "limit": 10})).unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("callers").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result.structured_content.as_ref().unwrap()["query"]["request"]["kind"],
        "callers"
    );

    let arguments = serde_json::from_value(serde_json::json!({
        "request": {
            "operation": "find",
            "request": { "kind": "subject", "query": "caller", "limit": 5 }
        }
    }))
    .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("query").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result.structured_content.as_ref().unwrap()["tier"],
        "compact"
    );

    let arguments = serde_json::from_value(serde_json::json!({})).unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("trace_seeded_paths").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result.structured_content.as_ref().unwrap()["tier"],
        "compact"
    );
    assert_eq!(
        result.structured_content.unwrap(),
        serde_json::to_value(
            entmoot_contracts::query::QueryService::trace_seeds(&engine, Default::default())
                .unwrap()
        )
        .unwrap()
    );
    let arguments = serde_json::from_value(serde_json::json!({"max_pairs": 0})).unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("trace_seeded_paths").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.structured_content.as_ref().unwrap()["kind"],
        "invalid_request"
    );

    let result = client
        .call_tool(CallToolRequestParams::new("impact"))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.unwrap(),
        serde_json::to_value(entmoot_engine::compare_revision_impact(&graph, &graph).unwrap())
            .unwrap()
    );
    let request = serde_json::json!({
        "start":caller_id,"direction":"callers","predicates":["calls"],"evidence_filter":"resolved_only",
        "budget":entmoot_contracts::query::BoundedWalkBudget::default()
    });
    let mut typed = request.clone();
    typed
        .as_object_mut()
        .unwrap()
        .insert("kind".into(), "neighborhood".into());
    let expected = entmoot_contracts::query::QueryService::execute(
        &engine,
        serde_json::from_value(serde_json::json!({"operation":"traverse", "request":typed}))
            .unwrap(),
    )
    .unwrap()
    .into_value()
    .unwrap();
    let actual = client
        .call_tool(
            CallToolRequestParams::new("neighborhood")
                .with_arguments(serde_json::from_value(request).unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let mut actual = actual;
    let mut expected = expected;
    without_telemetry(&mut actual);
    without_telemetry(&mut expected);
    assert_eq!(actual, expected);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn impact_with_publication_null_base_uses_working_tree_not_compare_graph_error() {
    let directory = tempfile::tempdir().unwrap();
    let source_dir = directory.path().join("source");
    std::fs::create_dir(&source_dir).unwrap();
    std::fs::write(source_dir.join("lib.rs"), "fn leaf() -> i32 { 1 }\n").unwrap();
    let graph = build_topology_from_repo_with_options(&source_dir, Default::default()).unwrap();
    let generation = directory.path().join("generation");
    std::fs::create_dir(&generation).unwrap();
    let graph_path = generation.join("graph.json");
    let store_path = generation.join("graph.json.knowledge.sqlite3");
    write_graph_json(&graph, &graph_path).unwrap();
    SqliteKnowledgeRepository::open(&store_path)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let summary_path = generation.join("summary.json");
    std::fs::write(&summary_path, "{}").unwrap();
    let publication_path = directory.path().join("current.json");
    let publication = serde_json::json!({
        "schema_version": "entmoot.workspace_publication.v1",
        "current": {
            "graph": graph_path,
            "store": store_path,
            "source_root": source_dir,
        },
        "base": null,
        "snapshot": graph.snapshot,
        "summary": summary_path,
    });
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&publication).unwrap(),
    )
    .unwrap();

    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|command| {
            command
                .arg("--publication")
                .arg(&publication_path)
                .arg("--source-root")
                .arg(&source_dir);
        }),
    )
    .unwrap();
    let client = ().serve(transport).await.unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("impact"))
        .await
        .unwrap();
    assert_ne!(
        result.is_error,
        Some(true),
        "impact under --publication with base null must not fail closed: {result:?}"
    );
    let content = result.structured_content.as_ref().unwrap();
    let message = content
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !message.contains("requires --compare-graph"),
        "unexpected compare-graph error: {content}"
    );
    assert_eq!(
        content.get("schema_version").and_then(|v| v.as_str()),
        Some("entmoot.revision_impact.v1"),
        "{content}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn impact_relay_matches_shared_service_for_pages_and_refusals() {
    use entmoot_contracts::query::{ImpactRelayRequest, QueryService};
    let directory = tempfile::tempdir().unwrap();
    let source_dir = directory.path().join("source");
    std::fs::create_dir(&source_dir).unwrap();
    let source = "fn leaf() -> i32 { 1 }\nfn first() { leaf(); }\nfn second() { leaf(); }";
    std::fs::write(source_dir.join("lib.rs"), source).unwrap();
    let base = build_topology_from_repo_with_options(&source_dir, Default::default()).unwrap();
    std::fs::write(source_dir.join("lib.rs"), source.replace("{ 1 }", "{ 2 }")).unwrap();
    let head = build_topology_from_repo_with_options(&source_dir, Default::default()).unwrap();
    let base_path = directory.path().join("base.json");
    let head_path = directory.path().join("head.json");
    write_graph_json(&base, &base_path).unwrap();
    write_graph_json(&head, &head_path).unwrap();
    let store = entmoot_engine::workspace::default_store_path(&head_path);
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&head))
        .unwrap();
    let engine = entmoot_engine::QueryEngine::new(head, store).with_compare_graph(base);
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp")).configure(|command| {
            command
                .arg("--graph")
                .arg(&head_path)
                .arg("--compare-graph")
                .arg(&base_path);
        }),
    )
    .unwrap();
    let client = ().serve(transport).await.unwrap();
    for value in [
        serde_json::json!({"evidence_filter":"any","callers_output":{"limit":1}}),
        serde_json::json!({"output":{"max_bytes":128}}),
    ] {
        let request: ImpactRelayRequest = serde_json::from_value(value.clone()).unwrap();
        let expected = engine.relay_impact(request);
        let actual = client
            .call_tool(CallToolRequestParams::new("relay_impact").with_arguments(
                serde_json::from_value(serde_json::json!({"request":value})).unwrap(),
            ))
            .await
            .unwrap();
        match expected {
            Ok(result) => {
                assert_ne!(actual.is_error, Some(true));
                let mut actual = actual.structured_content.unwrap();
                let mut expected = serde_json::to_value(result).unwrap();
                without_telemetry(&mut actual);
                without_telemetry(&mut expected);
                assert_eq!(actual, expected);
            }
            Err(error) => {
                assert_eq!(actual.is_error, Some(true));
                assert_eq!(actual.structured_content.unwrap()["kind"], error.code());
            }
        }
    }
    client.cancel().await.unwrap();
}
