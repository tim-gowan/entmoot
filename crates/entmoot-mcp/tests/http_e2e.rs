#![allow(clippy::unwrap_used, clippy::expect_used)]
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

#[tokio::test]
async fn loopback_auth_rejects_anonymous_calls_and_graph_queries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.rs"),
        "fn a(){} fn b(){} fn c(){} fn root(){a();b();c();}",
    )
    .unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(dir.path(), Default::default())
            .unwrap();
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
    let request = json!({"operation":"find","request":{"kind":"subject","query":"root"}});
    let result = client
        .call_tool(
            CallToolRequestParams::new("query")
                .with_arguments(json!({"request":request}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(result.structured_content.unwrap()["message"]
        .as_str()
        .unwrap()
        .contains("graph publication"));
    let request_path = dir.path().join("replay.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec(
            &json!({"operation":"find","request":{"kind":"subject","query":"root"}}),
        )
        .unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(cli)
        .arg("-C")
        .arg(dir.path())
        .args(["request", "--request"])
        .arg(&request_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("graph publication"));
    client.cancel().await.unwrap();
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
