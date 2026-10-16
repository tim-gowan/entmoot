#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Accelerators preserve typed read semantics across every claimed syntax family.
use entmoot_contracts::query::{QueryRequest, QueryService};
use entmoot_engine::{
    build_topology_from_repo_with_options, query::QueryEngineOptions, KnowledgeRepository,
    KnowledgeStore, NodeKind, QueryEngine, SqliteKnowledgeRepository, TopologyBuildOptions,
};
use serde_json::{json, Value};

fn answer(engine: &QueryEngine, request: &Value) -> Value {
    let request: QueryRequest = serde_json::from_value(request.clone()).unwrap();
    let mut value = engine.execute(request).unwrap().into_value().unwrap();
    fn stable(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if map.contains_key("budget") && map.contains_key("elapsed_micros") {
                    for field in [
                        "elapsed_micros",
                        "index_micros",
                        "evidence_micros",
                        "expanded_nodes",
                        "examined_edges",
                        "reused_discovery",
                        "examined_shortcuts",
                        "index_examined_edges",
                        "cumulative_expanded_nodes",
                        "cumulative_examined_edges",
                        "cumulative_examined_shortcuts",
                    ] {
                        map.remove(field);
                    }
                }
                for child in map.values_mut() {
                    stable(child);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(stable),
            _ => {}
        }
    }
    stable(&mut value);
    value
}

#[test]
fn all_families_match_reference_search_pages_and_warm_witnesses() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("case.rs", "fn leaf() {} fn root() { leaf(); }"),
        (
            "case.py",
            "def leaf():\n    return 1\ndef root():\n    return leaf()\n",
        ),
        ("case.js", "function leaf() {} function root() { leaf(); }"),
        (
            "case.ts",
            "function leaf(): void {} function root(): void { leaf(); }",
        ),
        (
            "case.tsx",
            "function leaf(): void {} function root(): void { leaf(); }",
        ),
        (
            "case.go",
            "package sample\nfunc leaf() {}\nfunc root() { leaf() }\n",
        ),
        (
            "Case.java",
            "class Case { static void leaf() {} static void root() { leaf(); } }",
        ),
        (
            "Case.cs",
            "class Case { static void leaf() {} static void root() { leaf(); } }",
        ),
        ("case.c", "void leaf(void) {} void root(void) { leaf(); }"),
        ("experimental.cpp", "void leaf() {} void root() { leaf(); }"),
    ];
    for (path, source) in files {
        std::fs::write(dir.path().join(path), source).unwrap();
    }
    let graph = build_topology_from_repo_with_options(
        dir.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("test/query-acceleration"),
        },
    )
    .unwrap();
    let store = dir.path().join("knowledge.sqlite3");
    let mut writer = SqliteKnowledgeRepository::open(&store).unwrap();
    writer
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    writer.store_inventory(&graph, None).unwrap();
    let lazy = QueryEngine::new(writer.load_inventory(true).unwrap().unwrap(), &store);
    let reference = QueryEngine::with_options(
        graph.clone(),
        &store,
        QueryEngineOptions {
            substring_postings_budget: 0,
            cache_walk_discovery: false,
            contract_paths: false,
            ..Default::default()
        },
    );
    let indexed = QueryEngine::new(graph.clone(), &store);
    let mut requests = Vec::new();
    for (path, _) in files {
        let nodes: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Callable && n.path.as_deref() == Some(path))
            .collect();
        assert!(nodes.len() >= 2, "{path}");
        for query in ["leaf", "lea", "root", "r", "missing", "::", path] {
            for filter in [None, Some(path)] {
                requests.push(json!({"operation":"find","request":{"kind":"subject","query":query,"path":filter,"limit":50}}));
            }
        }
        for node in &nodes {
            for kind in ["callers", "callees"] {
                requests.push(
                    json!({"operation":"inspect","request":{"kind":kind,"id":node.id,"limit":1}}),
                );
            }
        }
        requests.push(json!({"operation":"traverse","request":{"kind":"path","start":nodes[0].id,"target":nodes[1].id,"predicates":["calls"],"budget":{"max_hops":6,"max_expansions":100,"max_examined_edges":1000,"max_edges_per_node":0,"max_results":10,"max_evidence_per_hop":3}},"output":{"tier":"export"}}));
    }
    for request in &requests {
        let expected = answer(&reference, request);
        assert_eq!(expected, answer(&lazy, request), "lazy {request}");
        assert_eq!(expected, answer(&indexed, request), "cold {request}");
        assert_eq!(
            expected,
            answer(&indexed.clone(), request),
            "warm {request}"
        );
    }
    // Concurrent readers share only disposable state. Durable evidence changes
    // must be rehydrated even when graph-only discovery is already warm.
    writer.replace_all(&KnowledgeStore::default()).unwrap();
    std::thread::scope(|scope| {
        for chunk in requests.chunks(20) {
            let (reference, indexed) = (&reference, &indexed);
            scope.spawn(move || {
                for request in chunk {
                    assert_eq!(answer(reference, request), answer(indexed, request));
                }
            });
        }
    });
}

#[test]
fn contracted_original_edges_rehydrate_changed_claims() {
    use entmoot_contracts::{
        relation_claim_with_id, relation_edge_with_id, topology_node_address, ProducerRef,
        RelationClaimValue, RelationKind, ResolutionState, SubjectId,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("chain.rs"),
        "fn a() {} fn b() { a(); } fn c() { b(); }",
    )
    .unwrap();
    let mut graph = build_topology_from_repo_with_options(dir.path(), Default::default()).unwrap();
    let node = |label: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Callable && n.label == label)
            .unwrap()
            .id
            .clone()
    };
    let (a, b, c) = (node("a"), node("b"), node("c"));
    let mut scope = graph
        .relation_edges
        .iter()
        .find(|e| e.predicate == RelationKind::Calls)
        .unwrap()
        .scope
        .clone();
    scope.occurrence_id = None;
    let producer = ProducerRef {
        id: "fixture".into(),
        name: "fixture".into(),
        version: Some("1".into()),
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![],
    };
    let mut changed = None;
    for (from, to) in [(&c, &b), (&b, &a)] {
        let edge = relation_edge_with_id(
            "fixture",
            topology_node_address(from),
            RelationKind::DependsOn,
            topology_node_address(to),
            scope.clone(),
            None,
        );
        let claim = relation_claim_with_id(
            SubjectId::Proposition {
                proposition_id: edge.proposition_id.clone(),
            },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Validated,
            },
            producer.clone(),
            scope.clone(),
            vec![],
            None,
        );
        changed = Some(claim.clone());
        graph.relation_claims.push(claim);
        graph.relation_edges.push(edge);
    }
    let store = dir.path().join("knowledge.sqlite3");
    let mut writer = SqliteKnowledgeRepository::open(&store).unwrap();
    writer
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let engine = QueryEngine::new(graph.clone(), &store);
    let request = json!({"operation":"traverse","request":{"kind":"path","start":c,"target":a,"predicates":["depends_on"],"budget":entmoot_contracts::query::BoundedWalkBudget::default()},"output":{"tier":"export"}});
    let cold = engine
        .execute(serde_json::from_value(request.clone()).unwrap())
        .unwrap()
        .into_value()
        .unwrap();
    let cold = &cold["result"];
    assert_eq!(cold["paths"][0]["classification"], "supported");
    assert!(
        cold["search_receipt"]["examined_shortcuts"]
            .as_u64()
            .unwrap()
            > 0
    );
    let previous = changed.unwrap();
    graph
        .relation_claims
        .retain(|claim| claim.id != previous.id);
    graph.relation_claims.push(relation_claim_with_id(
        previous.subject,
        "resolution_state",
        RelationClaimValue::ResolutionState {
            state: ResolutionState::Refuted,
        },
        producer,
        scope,
        vec![],
        None,
    ));
    writer
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let warm = engine
        .execute(serde_json::from_value(request.clone()).unwrap())
        .unwrap()
        .into_value()
        .unwrap();
    let warm = &warm["result"];
    assert_eq!(warm["search_receipt"]["reused_discovery"], true);
    assert_eq!(warm["paths"][0]["classification"], "conflicted");
    assert_eq!(warm["paths"][0]["hop_count"], 2);
    let reference = QueryEngine::with_options(
        graph,
        store,
        QueryEngineOptions {
            contract_paths: false,
            ..Default::default()
        },
    );
    assert_eq!(answer(&engine, &request), answer(&reference, &request));
}
