#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Independent signature near misses across production extraction and stored reload.
use entmoot_contracts::query::{FindQuery, InventoryRequest};
use entmoot_engine::{
    build_topology_from_repo_with_options, KnowledgeRepository, KnowledgeStore, QueryEngine,
    SqliteKnowledgeRepository, TopologyBuildOptions,
};
use serde_json::json;

#[test]
fn signature_selection_correlates_one_parameter_and_survives_storage() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("main.rs"),
        r#"
struct Request;
fn handle_good(request: Request, count: usize) {}
fn handle_cross(request: usize, other: Request) {}
fn handle_other(value: Request) {}
fn helper(request: Request) {}
"#,
    )
    .unwrap();
    std::fs::write(
        source.join("main.py"),
        "def handle_unknown(request):\n    return request\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        &source,
        TopologyBuildOptions {
            policy: None,
            repository: Some("test/selection"),
        },
    )
    .unwrap();
    let knowledge = KnowledgeStore::extract_from_graph(&graph);
    let original = knowledge
        .code_contracts
        .iter()
        .find(|c| c.contract.name == "handle_good")
        .unwrap();
    let mut relocated = original.clone();
    let entmoot_contracts::DeclaredContract::Callable { parameters, .. } =
        &mut relocated.contract.shape
    else {
        panic!("expected callable")
    };
    parameters[0].span.start_byte += 1;
    assert_eq!(
        original.contract.boundary_key(),
        relocated.contract.boundary_key()
    );
    assert_ne!(
        original.derive_id(),
        relocated.derive_id(),
        "durable identity retains source extents"
    );
    relocated.record_id = relocated.derive_id();
    relocated.validate().unwrap();
    let mut conflicting = knowledge.clone();
    conflicting.code_contracts.push(relocated);
    let projected = conflicting.apply_to_inventory(graph.clone());
    assert!(projected
        .nodes
        .iter()
        .find(|n| n.label == "handle_good")
        .unwrap()
        .code_contract
        .is_none());
    let mut changed_source = graph.clone();
    for file in &mut changed_source.snapshot_files {
        file.content_digest = entmoot_contracts::blake3_hex(b"different source bytes");
    }
    assert!(knowledge
        .apply_to_inventory(changed_source)
        .nodes
        .iter()
        .all(|n| n.code_contract.is_none()));
    assert!(KnowledgeStore::default()
        .apply_to_inventory(graph.clone())
        .nodes
        .iter()
        .all(|n| n.code_contract.is_none()));
    let store = dir.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&knowledge)
        .unwrap();
    // Export/reload is the same published inventory contract used by frontends.
    let mut graph: entmoot_engine::TopologyGraph =
        serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
    for node in &mut graph.nodes {
        node.code_contract = None;
    }
    let graph = SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .load()
        .unwrap()
        .apply_to_inventory(graph);
    let engine = QueryEngine::new(graph, store);
    let selector = json!({
        "kinds":["callable"], "languages":["rust"],
        "name":{"mode":"contains","value":"handle"},
        "callable":{"parameters":[{
            "name":{"mode":"exact","value":"request"},
            "declared_type_syntax":{"mode":"exact","value":"Request"}
        }]}
    });
    let result = engine
        .find(
            serde_json::from_value::<FindQuery>(json!({
                "kind":"subject", "selector":selector
            }))
            .unwrap(),
        )
        .unwrap()
        .into_value()
        .unwrap();
    let labels: Vec<_> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["handle_good"]);
    assert_eq!(
        result["items"][0]["selection_match"]["parameter_positions"],
        json!([[0]])
    );
    let inventory = engine
        .inventory(
            serde_json::from_value::<InventoryRequest>(json!({
                "selector":selector
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(inventory.items.len(), 1);
    assert_eq!(inventory.items[0].node_id, result["items"][0]["node_id"]);
    let mut all_languages = selector;
    all_languages.as_object_mut().unwrap().remove("languages");
    let response = entmoot_contracts::query::QueryService::execute(
        &engine,
        serde_json::from_value(json!({
            "operation":"find","request":{"kind":"subject","selector":all_languages}
        }))
        .unwrap(),
    )
    .unwrap();
    let compact = serde_json::to_value(response).unwrap();
    assert_eq!(compact["selection"]["unknown"], 1);
    assert_eq!(compact["selection"]["matched"], 1);
    let key = compact["items"][0]["subject"].as_str().unwrap();
    assert_eq!(
        compact["subjects"][key]["selection_match"]["parameter_positions"],
        json!([[0]])
    );
    let objects = engine
        .inventory(
            serde_json::from_value(json!({
                "selector":{"kinds":["declaration"],"name":{"mode":"exact","value":"Request"}}
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        objects.items.len(),
        1,
        "selector kinds must not intersect a hidden callable default"
    );
}

#[test]
fn explicit_match_modes_reject_invalid_requests_before_empty_inventory() {
    let graph = entmoot_engine::TopologyGraph::new("test/empty");
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    let engine = QueryEngine::new(graph, store);
    for selector in [
        json!({"name":{"mode":"contains","value":""}}),
        json!({"name":{"mode":"levenshtein","value":"handle","max_distance":99}}),
        json!({"path":{"mode":"glob","value":"[unclosed"}}),
        json!({"path":{"mode":"subtree","value":"../outside"}}),
    ] {
        let request: FindQuery =
            serde_json::from_value(json!({"kind":"subject","selector":selector})).unwrap();
        assert!(engine.find(request).is_err(), "{selector}");
    }
}

#[test]
fn pruning_and_evidence_admission_find_the_longer_route_and_fence_resume() {
    use entmoot_contracts::*;
    use entmoot_engine::{NodeKind, TopologyGraph, TopologyNode};
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("knowledge.sqlite3");
    let mut graph = TopologyGraph::new("test/admission");
    for id in ["origin", "shortcut", "middle", "detour", "target"] {
        graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
    }
    let producer = ProducerRef {
        id: "fixture".into(),
        name: "fixture".into(),
        version: None,
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![],
    };
    for (from, to, state) in [
        ("origin", "shortcut", ResolutionState::Candidate),
        ("shortcut", "target", ResolutionState::Validated),
        ("origin", "middle", ResolutionState::Validated),
        ("middle", "detour", ResolutionState::Validated),
        ("detour", "target", ResolutionState::Validated),
    ] {
        let scope = Scope {
            domain: "source".into(),
            snapshot_revision: None,
            snapshot_digest: None,
            occurrence_id: Some(format!("{from}/{to}")),
            context_id: None,
        };
        let edge = relation_edge_with_id(
            "source",
            topology_node_address(from),
            RelationKind::Calls,
            topology_node_address(to),
            scope.clone(),
            scope.occurrence_id.clone(),
        );
        graph.relation_claims.push(relation_claim_with_id(
            SubjectId::RelationEdge {
                edge_id: edge.id.clone(),
            },
            "resolution_state",
            RelationClaimValue::ResolutionState { state },
            producer.clone(),
            scope,
            vec![],
            None,
        ));
        graph.relation_edges.push(edge);
    }
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let engine = QueryEngine::new(graph, &store);
    let query = json!({"kind":"path","start":"origin","target":"target",
        "predicates":["calls"],"budget":{"max_edges_per_node":0}});
    let run = |value: serde_json::Value| {
        engine
            .traverse(serde_json::from_value(value).unwrap())
            .unwrap()
            .into_value()
            .unwrap()
    };
    let ordinary = run(query.clone());
    assert_eq!(ordinary["paths"][0]["hop_count"], 2);
    let mut unknown_prune = query.clone();
    unknown_prune["prune"] = json!({"matching_subjects":{"callable":{"parameters":[{
        "declared_type_syntax":{"mode":"exact","value":"Request"}
    }]}}});
    let uncertain = run(unknown_prune);
    assert_eq!(uncertain["paths"][0]["hop_count"], 2);
    assert_eq!(uncertain["search_receipt"]["prune_selection"]["unknown"], 5);

    let mut pruned = query.clone();
    pruned["prune"] = json!({"subjects":["shortcut"]});
    let result = run(pruned.clone());
    assert_eq!(result["paths"][0]["hop_count"], 3);
    assert!(result["search_receipt"]["pruned_edges"].as_u64().unwrap() > 0);
    let mut resolved = query;
    resolved["evidence_filter"] = json!("resolved_only");
    let result = run(resolved);
    assert_eq!(result["paths"][0]["hop_count"], 3);
    assert_eq!(result["paths"][0]["classification"], "supported");
    let neighborhood = run(
        json!({"kind":"neighborhood","start":"origin","direction":"callees",
        "predicates":["calls"],"evidence_filter":"resolved_only",
        "endpoint_selector":{"name":{"mode":"exact","value":"target"}},
        "budget":{"max_edges_per_node":0}}),
    );
    assert_eq!(neighborhood["items"].as_array().unwrap().len(), 1);
    assert_eq!(neighborhood["items"][0]["witness"]["hop_count"], 3);
    assert_eq!(neighborhood["endpoint_selection"]["matched"], 1);
    // Evidence admission is cached only within the current knowledge generation.
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    let after_removal = run(json!({"kind":"path","start":"origin","target":"target",
        "predicates":["calls"],"evidence_filter":"resolved_only","budget":{"max_edges_per_node":0}}));
    assert!(after_removal["paths"].as_array().unwrap().is_empty());
    pruned["resumable"] = json!(true);
    pruned["budget"]["max_expansions"] = json!(1);
    let first = run(pruned.clone());
    let token = first["search_receipt"]["continuation"].clone();
    assert!(token.is_string());
    pruned["continuation"] = token;
    let mut changed = pruned.clone();
    changed["prune"] = json!({});
    assert!(engine
        .traverse(serde_json::from_value(changed).unwrap())
        .is_err());
    for _ in 0..20 {
        let page = run(pruned.clone());
        if page["search_receipt"]["continuation"].is_null() {
            assert_eq!(page["paths"][0]["hop_count"], 3);
            return;
        }
        pruned["continuation"] = page["search_receipt"]["continuation"].clone();
    }
    panic!("bounded fixture did not complete");
}

#[test]
fn text_modes_are_explicit_and_unicode_distance_is_not_byte_distance() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("knowledge.sqlite3");
    let mut graph = entmoot_engine::TopologyGraph::new("test/text");
    for name in ["café", "cafe", "prefix_cafe_suffix"] {
        graph.upsert_node(entmoot_engine::TopologyNode::shell(
            name,
            entmoot_engine::NodeKind::Callable,
            name,
            None,
        ));
    }
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    let engine = QueryEngine::new(graph, store);
    for (name, expected) in [
        (json!({"mode":"exact","value":"cafe"}), 1),
        (json!({"mode":"contains","value":"cafe"}), 2),
        (
            json!({"mode":"levenshtein","value":"cafe","max_distance":1}),
            2,
        ),
    ] {
        let result = engine
            .find(
                serde_json::from_value(json!({"kind":"subject","selector":{"name":name}})).unwrap(),
            )
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), expected);
    }
}

#[test]
fn explicit_paths_distinguish_files_subtrees_and_glob_segments() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    let mut graph = entmoot_engine::TopologyGraph::new("test/paths");
    for path in [
        "src/api.rs",
        "src/api/client.rs",
        "src/apricot.rs",
        "test/api.rs",
    ] {
        let mut node = entmoot_engine::TopologyNode::shell(
            path,
            entmoot_engine::NodeKind::Callable,
            path,
            None,
        );
        node.path = Some(path.into());
        graph.upsert_node(node);
    }
    let engine = QueryEngine::new(graph, store);
    for (selector, expected) in [
        (
            json!({"mode":"exact","value":"src/api.rs"}),
            vec!["src/api.rs"],
        ),
        (
            json!({"mode":"subtree","value":"src/api"}),
            vec!["src/api/client.rs"],
        ),
        (
            json!({"mode":"glob","value":"src/*.rs"}),
            vec!["src/api.rs", "src/apricot.rs"],
        ),
        (
            json!({"mode":"glob","value":"src/**/*.rs"}),
            vec!["src/api.rs", "src/api/client.rs", "src/apricot.rs"],
        ),
    ] {
        let result = engine
            .find(
                serde_json::from_value(json!({"kind":"subject","selector":{"path":selector}}))
                    .unwrap(),
            )
            .unwrap()
            .into_value()
            .unwrap();
        let paths: Vec<_> = result["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, expected);
    }
}

#[test]
fn excluding_one_occurrence_keeps_parallel_evidence_in_both_directions() {
    use entmoot_contracts::*;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::default())
        .unwrap();
    let mut graph = entmoot_engine::TopologyGraph::new("test/parallel");
    for name in ["caller", "callee"] {
        graph.upsert_node(entmoot_engine::TopologyNode::shell(
            name,
            entmoot_engine::NodeKind::Callable,
            name,
            None,
        ));
    }
    for occurrence in ["one", "two"] {
        graph.relation_edges.push(relation_edge_with_id(
            "source",
            topology_node_address("caller"),
            RelationKind::Calls,
            topology_node_address("callee"),
            Scope {
                domain: "source".into(),
                snapshot_revision: None,
                snapshot_digest: None,
                context_id: None,
                occurrence_id: Some(occurrence.into()),
            },
            Some(occurrence.into()),
        ));
    }
    let survivor = graph.relation_edges[1].id.to_string();
    let engine = QueryEngine::new(graph, store);
    for query in [
        json!({"kind":"path","start":"caller","target":"callee","predicates":["calls"],"prune":{"occurrences":["one"]},"budget":{}}),
        json!({"kind":"neighborhood","start":"callee","direction":"callers","predicates":["calls"],"evidence_filter":"any",
            "prune":{"occurrences":["one"]},"budget":{}}),
    ] {
        let response = entmoot_contracts::query::QueryService::execute(
            &engine,
            serde_json::from_value(json!({"operation":"traverse","request":query})).unwrap(),
        )
        .unwrap();
        let result = serde_json::to_value(response).unwrap();
        assert_eq!(result["items"][0]["edges"], json!([survivor]));
    }
}
