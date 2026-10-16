#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Consumer contracts: display budgets cannot alter evidence or search meaning.
use entmoot_contracts::query::{QueryRequest, QueryService};
use entmoot_engine::{
    build_topology_from_repo_with_options, KnowledgeRepository, KnowledgeStore, QueryEngine,
    SqliteKnowledgeRepository, TopologyBuildOptions,
};
use serde_json::{json, Value};

fn fixture() -> (tempfile::TempDir, QueryEngine) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.rs"),
        "fn leaf() {}\nfn root() { leaf(); missing(); }\nfn extra() { leaf(); }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        dir.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("test/tiers"),
        },
    )
    .unwrap();
    let path = dir.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&path)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    (dir, QueryEngine::new(graph, path))
}

fn execute(engine: &QueryEngine, request: Value) -> Value {
    QueryService::execute(
        engine,
        serde_json::from_value::<QueryRequest>(request).unwrap(),
    )
    .unwrap()
    .into_value()
    .unwrap()
}

#[test]
fn full_store_replacement_invalidates_compact_continuations_even_without_visibility_events() {
    let (dir, engine) = fixture();
    let mut request = json!({"operation":"find","request":{"kind":"subject","path":"main.rs","limit":100},"output":{"limit":1}});
    let first = execute(&engine, request.clone());
    request["output"]["after"] = first["page"]["next_cursor"].clone();
    SqliteKnowledgeRepository::open(&dir.path().join("knowledge.sqlite3"))
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    assert!(engine
        .execute(serde_json::from_value(request).unwrap())
        .is_err());
}

#[test]
fn compact_is_default_and_pages_are_query_and_snapshot_pinned() {
    let (_dir, engine) = fixture();
    let request = json!({"operation":"find","request":{"kind":"subject","path":"main.rs","limit":100},"output":{"limit":1}});
    let first = execute(&engine, request.clone());
    assert_eq!(first["tier"], "compact");
    assert_eq!(first["page"]["returned"], 1);
    assert_eq!(first["details_deferred"], true);
    let cursor = first["page"]["next_cursor"].clone();
    assert!(cursor.is_object());
    let mut next = request.clone();
    next["output"]["after"] = cursor.clone();
    let second = execute(&engine, next);
    assert_ne!(first["items"], second["items"]);
    for field in [
        "visibility_cursor",
        "generation_digest",
        "query_digest",
        "snapshot",
        "offset",
    ] {
        let mut tampered = request.clone();
        tampered["output"]["after"] = cursor.clone();
        match field {
            "visibility_cursor" => tampered["output"]["after"][field] = json!(999999),
            "offset" => tampered["output"]["after"][field] = json!(999999),
            "snapshot" => tampered["output"]["after"][field]["revision"] = json!("changed"),
            _ => tampered["output"]["after"][field] = json!(format!("blake3:{}", "0".repeat(64))),
        }
        assert!(
            QueryService::execute(&engine, serde_json::from_value(tampered).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut wrong = request;
    wrong["request"]["query"] = json!("leaf");
    wrong["output"]["after"] = cursor;
    assert_eq!(
        QueryService::execute(&engine, serde_json::from_value(wrong).unwrap())
            .unwrap_err()
            .code(),
        "invalid_request"
    );
}

#[test]
fn tiers_preserve_candidate_path_and_do_not_call_deferred_details_search_truncation() {
    let (_dir, engine) = fixture();
    let request = json!({"operation":"traverse","request":{"kind":"neighborhood","start":"main.rs::root","direction":"callees","predicates":["calls"],"evidence_filter":"any","budget":{"max_hops":6,"max_expansions":1000,"max_examined_edges":10000,"max_edges_per_node":0,"max_results":10,"max_evidence_per_hop":20}}});
    let compact = execute(&engine, request.clone());
    let mut expanded = request.clone();
    expanded["output"] = json!({"tier":"export"});
    let export = execute(&engine, expanded);
    assert_eq!(compact["status"], export["result"]["status"]);
    assert_eq!(compact["search_truncated"], false);
    assert_eq!(
        compact["items"][0]["classification"],
        export["result"]["items"][0]["witness"]["classification"]
    );
    assert_eq!(compact["items"][0]["classification"], "candidate");
    assert!(compact["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("repository_call_graph_completeness_not_established")));
    assert!(serde_json::to_vec(&compact).unwrap().len() <= 16 * 1024);
    assert!(compact["search_receipt"]["elapsed_micros"].is_u64());
    assert_eq!(
        compact["search_receipt"]["examined_edges"],
        export["result"]["search_receipt"]["examined_edges"]
    );
    assert!(!serde_json::to_string(&compact)
        .unwrap()
        .contains("provider_pointer"));
    let mut explain = request;
    explain["output"] = json!({"tier":"explain"});
    assert_eq!(
        execute(&engine, explain)["result"]["status"],
        compact["status"]
    );
}

#[test]
fn invalid_output_options_and_too_small_envelopes_fail_explicitly() {
    let (_dir, engine) = fixture();
    for output in [
        json!({"max_bytes":0}),
        json!({"limit":0}),
        json!({"max_bytes":128}),
        json!({"tier":"explain","max_bytes":128}),
        json!({"tier":"export","max_bytes":128}),
    ] {
        let request =
            json!({"operation":"find","request":{"kind":"subject","query":"root"},"output":output});
        assert!(QueryService::execute(&engine, serde_json::from_value(request).unwrap()).is_err());
    }
    assert!(serde_json::from_value::<QueryRequest>(
        json!({"operation":"find","request":{"kind":"subject"},"output":{"tier":"invented"}})
    )
    .is_err());
}

#[test]
fn compact_pages_cover_all_bounded_hits_and_reject_rebuilt_publications() {
    let (dir, engine) = fixture();
    let mut request = json!({"operation":"find","request":{"kind":"subject","path":"main.rs","limit":100},"output":{"limit":1}});
    let first = execute(&engine, request.clone());
    let cursor = first["page"]["next_cursor"].clone();
    let mut changed = engine.graph().clone();
    changed.nodes[0].label.push_str("-changed");
    let rebuilt = QueryEngine::new(changed, dir.path().join("knowledge.sqlite3"));
    let mut continued = request.clone();
    continued["output"]["after"] = cursor;
    assert!(QueryService::execute(&rebuilt, serde_json::from_value(continued).unwrap()).is_err());
    let mut seen = Vec::new();
    loop {
        let page = execute(&engine, request.clone());
        assert_eq!(page["search_truncated"], false);
        assert!(serde_json::to_vec(&page).unwrap().len() <= 16384);
        seen.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["subject"].as_str().unwrap().to_owned()),
        );
        if page["page"]["next_cursor"].is_null() {
            break;
        }
        request["output"]["after"] = page["page"]["next_cursor"].clone();
    }
    request["output"] = json!({"tier":"export"});
    let full = execute(&engine, request);
    assert_eq!(
        seen,
        full["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["node_id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
}

#[test]
fn seeded_compact_keeps_original_roles_colocation_and_unmapped_counts() {
    let (dir, engine) = fixture();
    let mut graph = engine.graph().clone();
    let root = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == "root")
        .unwrap()
        .id
        .clone();
    let leaf = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == "leaf")
        .unwrap()
        .id
        .clone();
    let extra = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == "extra")
        .unwrap()
        .id
        .clone();
    for (id, role, node, freshness) in [
        ("source-root", "source", &root, "current"),
        ("source-extra", "source", &extra, "current"),
        ("sink-leaf", "sink", &leaf, "current"),
        ("same-root", "sink", &root, "current"),
        ("stale-root", "sink", &root, "stale_body"),
    ] {
        graph.assertions.push(serde_json::from_value(json!({"assertion_id":id,"subject":{"entity_kind":"opaque","entity_id":id},"predicate":"entmoot.provider_role","object":{"value":role},"epistemic_status":"observed"})).unwrap());
        graph.attachments.push(serde_json::from_value(json!({"attachment_id":format!("attach-{id}"),"artifact_id":"fixture","location_id":id,"disposition":"attached","freshness":freshness,"selected_target":{"kind":"callable","node_id":node}})).unwrap());
    }
    let engine = QueryEngine::new(graph, dir.path().join("knowledge.sqlite3"));
    let result =
        serde_json::to_value(QueryService::trace_seeds(&engine, Default::default()).unwrap())
            .unwrap();
    assert_eq!(result["tier"], "compact");
    assert_eq!(result["colocated_count"], 1);
    assert_eq!(result["unmapped_assertions"], 1);
    assert_eq!(result["routes"][0]["to_anchors"][0]["role"], "sink");
    assert_eq!(
        result["routes"][0]["to_anchors"][0]["epistemic_status"],
        "observed"
    );
    let mut seed_request = entmoot_contracts::query::SeededPathRequest::default();
    seed_request.output.limit = 1;
    let first =
        serde_json::to_value(QueryService::trace_seeds(&engine, seed_request.clone()).unwrap())
            .unwrap();
    assert_eq!(first["page"]["returned"], 1);
    assert_eq!(first["page"]["total_items"], 2);
    let first_bytes = serde_json::to_vec(&first).unwrap().len();
    seed_request.output.limit = 1000;
    // Leave room for the variable-width timing receipt while still fitting only one route.
    seed_request.output.max_bytes = Some(first_bytes + 64);
    let byte_page =
        serde_json::to_value(QueryService::trace_seeds(&engine, seed_request.clone()).unwrap())
            .unwrap();
    let without_timing = |mut routes: serde_json::Value| {
        for route in routes.as_array_mut().unwrap() {
            let receipt = route["query"]["search_receipt"].as_object_mut().unwrap();
            for field in ["elapsed_micros", "index_micros", "evidence_micros"] {
                receipt.remove(field);
            }
        }
        routes
    };
    assert_eq!(
        without_timing(byte_page["routes"].clone()),
        without_timing(first["routes"].clone())
    );
    assert!(serde_json::to_vec(&byte_page).unwrap().len() <= first_bytes + 64);
    seed_request.output.after =
        Some(serde_json::from_value(first["page"]["next_cursor"].clone()).unwrap());
    let next =
        serde_json::to_value(QueryService::trace_seeds(&engine, seed_request.clone()).unwrap())
            .unwrap();
    assert_ne!(first["routes"], next["routes"]);
    assert!(next["page"].get("next_cursor").is_none());
    seed_request.output.tier = entmoot_contracts::query::InformationTier::Export;
    assert!(QueryService::trace_seeds(&engine, seed_request).is_err());
    let neighborhood = execute(
        &engine,
        json!({"operation":"traverse","request":{"kind":"neighborhood","start":root,"direction":"callees","predicates":["calls"],"evidence_filter":"any","endpoint_role":{"role":"sink"},"budget":entmoot_contracts::query::BoundedWalkBudget::default()}}),
    );
    assert_eq!(neighborhood["items"][0]["roles"][0]["role"], "sink");
    assert_eq!(neighborhood["role_selection"]["unmapped_assertions"], 1);
}

#[test]
fn oversized_single_subject_explain_refuses_until_bytes_are_explicitly_raised() {
    let (dir, engine) = fixture();
    let mut graph = engine.graph().clone();
    let node = graph
        .nodes
        .iter_mut()
        .find(|node| node.label == "root")
        .unwrap();
    let id = node.id.clone();
    node.label = "large retained provider label ".repeat(3000);
    let engine = QueryEngine::new(graph, dir.path().join("knowledge.sqlite3"));
    let mut request = json!({"operation":"inspect","request":{"kind":"subject","id":id},"output":{"tier":"explain"}});
    let error = engine
        .execute(serde_json::from_value(request.clone()).unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "invalid_request");
    assert!(error.to_string().contains("65536"));
    request["output"]["max_bytes"] = json!(1024 * 1024);
    let response = execute(&engine, request);
    assert!(serde_json::to_vec(&response).unwrap().len() > 65536);
}
