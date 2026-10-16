//! Rooted impact/role routes are evidence navigation, never value-flow inference.
#![allow(clippy::unwrap_used)]
use entmoot_contracts::query::{BoundedWalkBudget, SeededPathRequest, TraverseQuery};
use entmoot_contracts::{relation_edge_with_id, topology_node_address, RelationKind, Scope};
use entmoot_engine::{
    KnowledgeRepository, KnowledgeStore, NodeKind, QueryEngine, SqliteKnowledgeRepository,
    TopologyGraph, TopologyNode,
};
use serde_json::{json, Value};

fn fixture() -> TopologyGraph {
    let mut graph = TopologyGraph::new("rooted");
    for id in ["origin", "middle", "target", "hypothesis", "unlabeled"] {
        graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
    }
    for (from, to) in [
        ("origin", "middle"),
        ("middle", "target"),
        ("middle", "hypothesis"),
        ("middle", "unlabeled"),
    ] {
        edge(&mut graph, from, to);
    }
    role(&mut graph, "source", "source", "origin", "observed");
    role(&mut graph, "sink", "sink", "target", "observed");
    role(
        &mut graph,
        "guess",
        "sink",
        "hypothesis",
        "heuristically_derived",
    );
    graph
}
fn edge(graph: &mut TopologyGraph, from: &str, to: &str) {
    graph.relation_edges.push(relation_edge_with_id(
        "calls",
        topology_node_address(from),
        RelationKind::Calls,
        topology_node_address(to),
        Scope {
            domain: "rooted".into(),
            snapshot_revision: None,
            snapshot_digest: None,
            occurrence_id: Some(format!("{from}-{to}")),
            context_id: Some("build-a".into()),
        },
        None,
    ));
}
fn role(graph: &mut TopologyGraph, id: &str, role: &str, node: &str, status: &str) {
    graph.assertions.push(serde_json::from_value(json!({
        "assertion_id":id, "subject":{"entity_kind":"opaque","entity_id":id},
        "predicate":"entmoot.provider_role", "object":{"value":role},
        "epistemic_status":status,
        "evidence_refs":[{"artifact_id":"provider","provider_pointer":id}],
        "derivation": if status == "observed" { Value::Null } else {
            json!({"method_id":"fixture.role.v1","policy_version":"1","limitations":"candidate"})
        }
    })).unwrap());
    graph.attachments.push(
        serde_json::from_value(json!({
            "attachment_id":format!("attach-{id}"),"artifact_id":"provider","location_id":id,
            "disposition":"attached","freshness":"current",
            "selected_target":{"kind":"callable","node_id":node}
        }))
        .unwrap(),
    );
}
fn open(graph: TopologyGraph, directory: &tempfile::TempDir) -> QueryEngine {
    let store = directory.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    QueryEngine::new(graph, store)
}
fn request(start: &str, direction: &str, role: &str, hypotheses: bool) -> Value {
    json!({"kind":"neighborhood","start":start,"direction":direction,
        "predicates":["calls"],"evidence_filter":"any",
        "endpoint_role":{"role":role,"include_hypotheses":hypotheses,"max_assertions":20000},
        "budget":BoundedWalkBudget { max_hops:12,max_edges_per_node:0,max_results:20,..Default::default() }})
}
fn run(engine: &QueryEngine, request: Value) -> Value {
    serde_json::to_value(
        engine
            .traverse(serde_json::from_value(request).unwrap())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn rooted_routes_filter_terminals_preserve_direction_and_original_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(fixture(), &dir);
    let before = serde_json::to_vec(engine.graph()).unwrap();
    let result = run(&engine, request("origin", "callees", "sink", false));
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    let hit = &result["items"][0];
    assert_eq!(hit["witness"]["hop_count"], 2);
    assert_eq!(hit["witness"]["classification"], "candidate");
    assert_eq!(hit["role_anchors"][0]["assertion_id"], "sink");
    assert_eq!(
        hit["role_anchors"][0]["assertion"]["epistemic_status"],
        "observed"
    );
    assert_eq!(result["role_selection"]["excluded_hypotheses"], 1);
    assert_eq!(result["receipt"]["attachments"]["attachment_count"], 1);
    assert_eq!(
        hit["role_anchors"][0]["attachments"][0]["attachment_id"],
        "attach-sink"
    );
    assert!(result["receipt"].to_string().contains("provider"));
    let reverse = run(&engine, request("target", "callers", "source", false));
    let edges = reverse["items"][0]["witness"]["hops"].as_array().unwrap();
    assert_eq!(
        edges[0]["relation_edge"]["subject"],
        serde_json::to_value(topology_node_address("origin")).unwrap()
    );
    assert_eq!(
        edges[1]["relation_edge"]["object"],
        serde_json::to_value(topology_node_address("target")).unwrap()
    );
    assert!(
        run(&engine, request("target", "callees", "source", false))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut capped = request("origin", "callees", "sink", false);
    capped["budget"]["max_results"] = json!(1);
    assert_eq!(run(&engine, capped)["items"][0]["witness"]["hop_count"], 2);
    let own = run(&engine, request("target", "callers", "sink", false));
    assert!(own["items"].as_array().unwrap().is_empty());
    assert_eq!(
        own["role_selection"]["colocated"][0]["assertion_id"],
        "sink"
    );
    assert!(own["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("dataflow_not_established")));
    assert_eq!(serde_json::to_vec(engine.graph()).unwrap(), before);
}

#[test]
fn rooted_roles_and_all_seed_routes_share_explicit_hypothesis_admission() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(fixture(), &dir);
    let result = run(&engine, request("origin", "callees", "sink", true));
    assert_eq!(result["items"].as_array().unwrap().len(), 2);
    let guessed = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|hit| hit["role_anchors"].as_array().unwrap())
        .find(|a| a["assertion_id"] == "guess")
        .unwrap();
    assert_eq!(
        guessed["assertion"]["epistemic_status"],
        "heuristically_derived"
    );
    assert_eq!(
        guessed["assertion"]["derivation"]["method_id"],
        "fixture.role.v1"
    );
    assert_eq!(
        engine
            .trace_seeds(SeededPathRequest::default())
            .unwrap()
            .routes
            .len(),
        1
    );
    let mut all = serde_json::to_value(SeededPathRequest::default()).unwrap();
    all["include_hypotheses"] = json!(true);
    assert_eq!(
        engine
            .trace_seeds(serde_json::from_value(all).unwrap())
            .unwrap()
            .routes
            .len(),
        2
    );
}

#[test]
fn rooted_role_mapping_rejects_stale_ambiguous_wrong_artifact_and_invalid_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = fixture();
    for (id, status) in [
        ("stale", "observed"),
        ("ambiguous", "observed"),
        ("foreign", "observed"),
        ("contradicted", "contradicted"),
        ("unknown", "unknown"),
        ("invalid-derived", "deterministically_derived"),
    ] {
        role(&mut graph, id, "sink", "target", status);
    }
    graph
        .attachments
        .iter_mut()
        .find(|a| a.location_id == "stale")
        .unwrap()
        .freshness = entmoot_contracts::AttachmentFreshness::StaleBody;
    let mut duplicate = graph
        .attachments
        .iter()
        .find(|a| a.location_id == "ambiguous")
        .unwrap()
        .clone();
    duplicate.attachment_id = "other".into();
    duplicate.selected_target.as_mut().unwrap().node_id = Some("middle".into());
    graph.attachments.push(duplicate);
    graph
        .attachments
        .iter_mut()
        .find(|a| a.location_id == "foreign")
        .unwrap()
        .artifact_id = "other-provider".into();
    graph
        .assertions
        .iter_mut()
        .find(|a| a.assertion_id == "invalid-derived")
        .unwrap()
        .derivation = None;
    let engine = open(graph, &dir);
    let result = run(&engine, request("origin", "callees", "sink", true));
    let ids: Vec<_> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|hit| hit["role_anchors"].as_array().unwrap())
        .map(|a| a["assertion_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"sink") && ids.contains(&"guess"));
    let unmapped = result["role_selection"]["unmapped_assertion_ids"]
        .as_array()
        .unwrap();
    for id in [
        "stale",
        "ambiguous",
        "foreign",
        "contradicted",
        "unknown",
        "invalid-derived",
    ] {
        assert!(unmapped.contains(&json!(id)), "{id}");
    }
}

#[test]
fn rooted_budgets_and_input_permutations_remain_explicit_and_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = fixture();
    role(&mut graph, "sink-two", "sink", "target", "observed");
    edge(&mut graph, "target", "origin");
    let engine = open(graph.clone(), &dir);
    let mut query = request("origin", "callees", "sink", false);
    query["budget"]["max_evidence_per_hop"] = json!(1);
    let mut result = run(&engine, query.clone());
    assert_eq!(result["status"], "known");
    assert_eq!(result["bounds"]["truncated"], false);
    assert_eq!(result["items"][0]["role_anchor_bounds"]["truncated"], true);
    assert_eq!(
        result["items"][0]["role_summaries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(result["items"][0]["role_anchor_bounds"]["total_matches"], 2);
    graph.relation_edges.reverse();
    graph.assertions.reverse();
    graph.attachments.reverse();
    let mut reordered = run(&open(graph, &dir), query);
    for value in [&mut result, &mut reordered] {
        for field in ["index_micros", "evidence_micros"] {
            value["search_receipt"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
    }
    result["search_receipt"]
        .as_object_mut()
        .unwrap()
        .remove("elapsed_micros");
    reordered["search_receipt"]
        .as_object_mut()
        .unwrap()
        .remove("elapsed_micros");
    assert_eq!(result, reordered);
    let mut query = request("origin", "callees", "sink", false);
    query["endpoint_role"]["max_assertions"] = json!(1);
    assert_eq!(run(&engine, query)["role_selection"]["truncated"], true);
    let mut query = request("origin", "callees", "sink", false);
    query["budget"]["max_examined_edges"] = json!(1);
    let result = run(&engine, query);
    assert_eq!(result["status"], "truncated");
    assert!(result["items"].as_array().unwrap().is_empty());
    let mut query = request("origin", "callees", "sink", false);
    query["endpoint_role"]["role"] = json!(" ");
    assert!(engine
        .traverse(serde_json::from_value(query).unwrap())
        .is_err());
    let missing = run(&engine, request("origin", "callees", "absent-role", false));
    assert_eq!(missing["status"], "unknown");
    assert!(missing["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("no_provider_stated_role_matched")));
}

#[test]
fn pairwise_and_rooted_routes_reject_cross_context_domain_and_snapshot_composition() {
    for mutation in ["context", "domain", "revision", "digest", "missing-context"] {
        let dir = tempfile::tempdir().unwrap();
        let mut graph = fixture();
        let scope = &mut graph.relation_edges[1].scope;
        match mutation {
            "context" => scope.context_id = Some("build-b".into()),
            "domain" => scope.domain = "other-repository".into(),
            "revision" => scope.snapshot_revision = Some("foreign-revision".into()),
            "digest" => scope.snapshot_digest = Some(format!("blake3:{}", "a".repeat(64))),
            _ => scope.context_id = None,
        }
        let engine = open(graph, &dir);
        let result = serde_json::to_value(
            engine
                .traverse(TraverseQuery::Path {
                    resumable: false,
                    continuation: None,
                    start: "origin".into(),
                    target: "target".into(),
                    predicates: vec!["calls".into()],
                    budget: BoundedWalkBudget::default(),
                })
                .unwrap(),
        )
        .unwrap();
        assert!(
            result["paths"].as_array().unwrap().is_empty(),
            "{mutation}: {result}"
        );
        assert!(result["gaps"]
            .as_array()
            .unwrap()
            .contains(&json!("scope_composition_not_established")));
        let result = run(&engine, request("origin", "callees", "sink", false));
        assert!(result["items"].as_array().unwrap().is_empty(), "{mutation}");
        assert!(result["scope_filtered_edges"].as_u64().unwrap() > 0);
    }
}

#[test]
fn rooted_search_keeps_alternative_build_states_at_the_same_address() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = fixture();
    graph.relation_edges[1].scope.context_id = Some("build-b".into());
    edge(&mut graph, "origin", "unlabeled");
    edge(&mut graph, "unlabeled", "middle");
    for edge in graph.relation_edges.iter_mut().rev().take(2) {
        edge.scope.context_id = Some("build-b".into());
    }
    let engine = open(graph, &dir);
    let result = run(&engine, request("origin", "callees", "sink", false));
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["witness"]["hop_count"], 3);
    assert_eq!(
        result["items"][0]["witness"]["context_ids"],
        json!(["build-b"])
    );
}

#[test]
fn rooted_sparse_role_search_has_linear_edge_work_and_explicit_caps() {
    let directory = tempfile::tempdir().unwrap();
    let mut graph = TopologyGraph::new("rooted");
    for id in ["origin", "target"] {
        graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
    }
    const BRANCHES: usize = 10_000;
    for i in 0..BRANCHES {
        let id = format!("branch-{i:05}");
        graph.upsert_node(TopologyNode::shell(&id, NodeKind::Callable, &id, None));
        edge(&mut graph, "origin", &id);
        edge(&mut graph, &id, "target");
    }
    role(&mut graph, "sink", "sink", "target", "observed");
    let engine = open(graph, &directory);
    let mut query = request("origin", "callees", "sink", false);
    query["budget"]["max_expansions"] = json!(BRANCHES + 1);
    let result = run(&engine, query.clone());
    assert_eq!(result["search_receipt"]["expanded_nodes"], BRANCHES + 1);
    assert_eq!(result["search_receipt"]["examined_edges"], BRANCHES * 2);
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["witness"]["hop_count"], 2);
    eprintln!(
        "rooted breadth: {} edges, {} expansions, {} microseconds, {} output bytes",
        BRANCHES * 2,
        BRANCHES + 1,
        result["search_receipt"]["elapsed_micros"],
        serde_json::to_vec(&result).unwrap().len()
    );
    query["budget"]["max_examined_edges"] = json!(BRANCHES + 10);
    let capped = run(&engine, query);
    assert_eq!(capped["search_receipt"]["examined_edges"], BRANCHES + 10);
    assert_eq!(capped["status"], "truncated");
    assert_eq!(capped["search_receipt"]["edge_budget_exhausted"], true);
}

#[test]
fn rooted_roles_require_valid_grain_and_versioned_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = fixture();
    role(&mut graph, "wrong-grain", "sink", "target", "observed");
    graph
        .attachments
        .last_mut()
        .unwrap()
        .selected_target
        .as_mut()
        .unwrap()
        .kind = entmoot_contracts::AttachmentTargetKind::File;
    role(
        &mut graph,
        "unversioned",
        "sink",
        "target",
        "heuristically_derived",
    );
    graph
        .assertions
        .last_mut()
        .unwrap()
        .derivation
        .as_mut()
        .unwrap()
        .policy_version
        .clear();
    let engine = open(graph, &dir);
    let result = run(&engine, request("origin", "callees", "sink", true));
    let unmapped = result["role_selection"]["unmapped_assertion_ids"]
        .as_array()
        .unwrap();
    assert!(unmapped.contains(&json!("wrong-grain")));
    assert!(unmapped.contains(&json!("unversioned")));
}
