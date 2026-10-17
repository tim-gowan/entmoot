#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::query::QueryService;
use entmoot_engine::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

fn stable(value: &mut Value) {
    match value {
        Value::Object(values) => {
            values.remove("elapsed_micros");
            values.remove("index_micros");
            values.remove("evidence_micros");
            for value in values.values_mut() {
                stable(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(stable),
        _ => {}
    }
}

#[tokio::test]
async fn loopback_auth_shared_queries_and_resumable_paging() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.rs"),
        "fn a(){} fn b(){} fn c(){} fn root(){a();b();c();}",
    )
    .unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(dir.path(), Default::default())
            .unwrap();
    let root = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == "root")
        .unwrap()
        .id
        .clone();
    let graph_path = dir.path().join("graph.json");
    let store = dir.path().join("graph.json.knowledge.sqlite3");
    entmoot_engine::write_graph_json(&graph, &graph_path).unwrap();
    let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
    repository
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    repository.store_inventory(&graph, None).unwrap();
    drop(repository);
    let token = "fixture-local-backend-credential-123456789";
    // Workspace integration runs already build the CLI. Isolated MCP runs must
    // prepare it explicitly; tests never switch feature/profile graphs or
    // replace a binary that another test may currently be using.
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_entmoot-mcp"));
    let profile = executable.parent().unwrap();
    let cli = profile.join(format!("entmoot{}", std::env::consts::EXE_SUFFIX));
    let fingerprint = std::process::Command::new(&cli)
        .arg("fingerprint")
        .output()
        .expect("prepare the matching CLI first: cargo build --locked --profile test -p entmoot");
    assert!(
        fingerprint.status.success(),
        "{}",
        String::from_utf8_lossy(&fingerprint.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fingerprint.stdout).unwrap(),
        serde_json::to_value(entmoot_engine::storage_fingerprint()).unwrap(),
        "CLI/library fingerprints differ; rebuild both with the same source inputs"
    );
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_entmoot-mcp"))
        .args(["--listen", "127.0.0.1:0", "--graph"])
        .arg(&graph_path)
        .env("ENTMOOT_QUERY_TOKEN", token)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    let url = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let line = stderr
                .next_line()
                .await
                .unwrap()
                .expect("backend exited before listening");
            if let Some(url) = line.strip_prefix("Entmoot query backend: ") {
                break url.to_owned();
            }
        }
    })
    .await
    .unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(http.get(&url).send().await.unwrap().status(), 401);
    assert_eq!(
        http.get(&url)
            .bearer_auth(token)
            .header("Origin", "http://example.invalid")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone()).auth_header(token);
    config.allow_stateless = true;
    let client = ().serve(StreamableHttpClientTransport::with_client(http, config)).await.unwrap();
    let engine =
        entmoot_engine::workspace::open_query_bundle_paths(&graph_path, &store, None, None, false)
            .unwrap()
            .engine;
    let request = json!({"operation":"find","request":{"kind":"subject","query":"root"}});
    let mut expected = engine
        .execute(serde_json::from_value(request.clone()).unwrap())
        .unwrap()
        .into_value()
        .unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("query")
                .with_arguments(json!({"request":request}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let mut actual = result.structured_content.unwrap();
    stable(&mut actual);
    stable(&mut expected);
    assert_eq!(actual, expected);

    let mut request = json!({"operation":"traverse","request":{"kind":"neighborhood","start":root,"direction":"callees","predicates":["calls"],"evidence_filter":"any","resumable":true,"budget":{"max_hops":2,"max_expansions":1,"max_examined_edges":100,"max_edges_per_node":0,"max_results":3}},"output":{"limit":1}});
    let call = |request: Value| {
        CallToolRequestParams::new("query")
            .with_arguments(json!({"request":request}).as_object().unwrap().clone())
    };
    let first = client.call_tool(call(request.clone())).await.unwrap();
    assert_ne!(first.is_error, Some(true), "{first:?}");
    let first = first.structured_content.unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert!(first["search_receipt"]["slice_token"].is_string());
    assert!(first["page"]["next_cursor"].is_object());
    request["output"]["after"] = first["page"]["next_cursor"].clone();
    let second = client
        .call_tool(call(request.clone()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(second["search_receipt"], first["search_receipt"]);
    assert_ne!(second["items"], first["items"]);
    let retry = client
        .call_tool(call(request.clone()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(retry, second);

    // Exercise the actual CLI frontend through a new stateless MCP connection,
    // including a replay token created by a different client.
    let request_path = dir.path().join("replay.json");
    std::fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = std::process::Command::new(cli)
        .args(["--backend", &url, "request", "--request"])
        .arg(&request_path)
        .env("ENTMOOT_QUERY_TOKEN", token)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(cli, second);
    client.cancel().await.unwrap();
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
