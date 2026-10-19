use super::*;
use crate::{KnowledgeRead, KnowledgeRepository, SqliteKnowledgeRepository};
use crate::{KnowledgeStore, NodeKind, TopologyNode};
use entmoot_contracts::query::FindQuery;
use entmoot_contracts::*;

fn scope() -> Scope {
    Scope {
        domain: "fixture".into(),
        snapshot_revision: None,
        snapshot_digest: None,
        occurrence_id: None,
        context_id: None,
    }
}

fn producer() -> ProducerRef {
    ProducerRef {
        id: "fixture".into(),
        name: "fixture".into(),
        version: None,
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![],
    }
}

fn graph() -> TopologyGraph {
    let mut graph = TopologyGraph::new("fixture");
    for id in ["a", "b", "c"] {
        graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
    }
    for target in ["b", "c"] {
        graph.relation_edges.push(relation_edge_with_id(
            "source",
            topology_node_address("a"),
            RelationKind::Calls,
            topology_node_address(target),
            scope(),
            None,
        ));
    }
    graph
}

fn publish_and_reload(graph: &mut TopologyGraph, path: &std::path::Path) {
    {
        let mut repository = SqliteKnowledgeRepository::open(path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(graph))
            .unwrap();
    }
    let retained = SqliteKnowledgeRepository::open(path)
        .unwrap()
        .load()
        .unwrap();
    graph.relation_edges = retained.relation_edges;
    graph.relation_claims = retained.relation_claims;
    graph.unresolved_relations = retained.unresolved_relations;
}

#[test]
fn cross_edge_disagreement_survives_reload_and_query_without_endpoint_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let mut graph = graph();
    graph.root = dir.path().to_string_lossy().into_owned();
    let fixture_scope = crate::relation_claims::graph_relation_scope(&graph);
    graph.relation_edges = graph
        .relation_edges
        .iter()
        .map(|edge| {
            relation_edge_with_id(
                &edge.schema_domain,
                edge.subject.clone(),
                edge.predicate,
                edge.object.clone(),
                fixture_scope.clone(),
                edge.occurrence_id.clone(),
            )
        })
        .collect();
    let expected = graph.relation_edges.clone();
    graph.relation_claims = expected
        .iter()
        .map(|edge| {
            relation_claim_with_id(
                SubjectId::RelationEdge {
                    edge_id: edge.id.clone(),
                },
                "resolution_state",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Candidate,
                },
                producer(),
                fixture_scope.clone(),
                vec![],
                None,
            )
        })
        .collect();
    let ids: Vec<_> = graph
        .relation_claims
        .iter()
        .map(|claim| claim.id.clone())
        .collect();
    graph.relation_claims[0].contradicts.push(ids[1].clone());
    graph.relation_claims[1].contradicts.push(ids[0].clone());
    let expected_claims = graph.relation_claims.clone();
    publish_and_reload(&mut graph, &path);
    for edge in &expected {
        assert!(graph.relation_edges.contains(edge));
    }
    let engine = QueryEngine::new(graph.clone(), path.clone());
    SqliteKnowledgeRepository::open(&path)
        .unwrap()
        .store_inventory(&graph, None)
        .unwrap();
    let retained = QueryEngine::from_retained(&path, Default::default()).unwrap();
    // One-edge pages must include disagreements originating outside that page.
    // Exercise each direction of the sparse reverse-claim lookup after restart.
    for edge in &expected {
        let entmoot_contracts::Address::Opaque { value: target, .. } = &edge.object else {
            panic!("fixture target must be a topology address");
        };
        let request = InspectQuery::Callers {
            id: target.clone(),
            after: None,
            limit: Some(1),
        };
        let actual = retained
            .inspect(request.clone())
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(
            actual,
            engine.inspect(request).unwrap().into_value().unwrap()
        );
        assert_eq!(
            actual["receipt"]["relations"]["conflicted_relation_edge_ids"],
            json!([edge.id])
        );
    }
    assert!(retained
        .deferred_navigation
        .as_ref()
        .unwrap()
        .get()
        .is_none());
    let calls = engine
        .inspect(InspectQuery::Callees {
            id: "a".into(),
            after: None,
            limit: Some(10),
        })
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!(calls["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        calls["receipt"]["relations"]["conflicted_relation_edge_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    for edge in &expected {
        let hit = calls["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|hit| hit["relation"]["id"] == edge.id.as_str())
            .unwrap();
        assert_eq!(
            serde_json::from_value::<RelationEdge>(hit["relation"].clone()).unwrap(),
            *edge
        );
        let expected_claim = expected_claims
            .iter()
            .find(|claim| {
                claim.subject
                    == (SubjectId::RelationEdge {
                        edge_id: edge.id.clone(),
                    })
            })
            .unwrap();
        assert_eq!(hit["claims"], serde_json::json!([expected_claim]));
    }
    assert!(calls["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap == "repository_call_graph_completeness_not_established"));
    assert_eq!(
        engine
            .status(Default::default())
            .unwrap()
            .conflicted_relations
            .value,
        Some(2)
    );
    let conflicts = engine
        .find(FindQuery::ConflictedRelation {
            path: None,
            limit: Some(10),
        })
        .unwrap()
        .into_value()
        .unwrap();
    let actual: HashSet<_> = conflicts["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        actual,
        expected.iter().map(|edge| edge.id.as_str()).collect()
    );
    // A link to an inactive or absent claim is retained evidence, not an active disagreement.
    graph.relation_claims[1].state = RelationClaimState::Superseded;
    publish_and_reload(&mut graph, &path);
    assert_eq!(
        QueryEngine::new(graph, path)
            .status(Default::default())
            .unwrap()
            .conflicted_relations
            .value,
        Some(0)
    );
}

#[test]
fn unresolved_frontier_exposes_continuation_after_concrete_edges_are_exhausted() {
    for concrete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.sqlite3");
        let mut graph = graph();
        graph.relation_edges.truncate(usize::from(concrete));
        for i in 0..3 {
            graph.unresolved_relations.push(unresolved_relation_with_id(
                topology_node_address("a"),
                RelationKind::Calls,
                TargetConstraint::Syntactic {
                    callee: format!("unknown_{i}"),
                },
                UnresolvedReason::UnresolvedBinding,
                producer(),
                scope(),
                Some(format!("site-{i}")),
                vec![],
                None,
                None,
            ));
        }
        let expected: HashSet<_> = graph
            .unresolved_relations
            .iter()
            .map(|attempt| attempt.id.to_string())
            .collect();
        publish_and_reload(&mut graph, &path);
        let engine = QueryEngine::new(graph, path);
        let calls = engine
            .inspect(InspectQuery::Callees {
                id: "a".into(),
                after: None,
                limit: Some(1),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(calls["status"], "truncated");
        assert!(calls["next_cursor"].is_null());
        assert_eq!(
            calls["items"].as_array().unwrap().len(),
            usize::from(concrete)
        );
        let mut seen = HashSet::from([calls["unresolved_relations"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned()]);
        let mut request: InspectQuery =
            serde_json::from_value(calls["unresolved_continuation"].clone())
                .expect("actionable unresolved continuation");
        loop {
            let page = engine.inspect(request).unwrap().into_value().unwrap();
            for attempt in page["items"].as_array().unwrap() {
                assert!(
                    seen.insert(attempt["id"].as_str().unwrap().to_owned()),
                    "attempt repeated"
                );
            }
            if page["next_cursor"].is_null() {
                break;
            }
            request = InspectQuery::Unresolved {
                id: "a".into(),
                after: Some(serde_json::from_value(page["next_cursor"].clone()).unwrap()),
                limit: Some(1),
            };
        }
        assert_eq!(seen, expected);
        let compact = engine
            .execute(entmoot_contracts::query::QueryRequest::Inspect {
                request: InspectQuery::Callees {
                    id: "a".into(),
                    after: None,
                    limit: Some(1),
                },
                output: Default::default(),
            })
            .unwrap();
        let entmoot_contracts::query::QueryResponse::Compact(compact) = compact else {
            panic!("compact")
        };
        let mut request = compact
            .unresolved_continuation
            .expect("compact must expose deferred attempts");
        let mut seen = HashSet::new();
        loop {
            let entmoot_contracts::query::QueryResponse::Compact(page) = engine
                .execute(entmoot_contracts::query::QueryRequest::Inspect {
                    request: *request,
                    output: Default::default(),
                })
                .unwrap()
            else {
                panic!("compact")
            };
            for item in page.items {
                let value = serde_json::to_value(item).unwrap();
                assert!(seen.insert(value["id"].as_str().unwrap().to_owned()));
            }
            let Some(next) = page.unresolved_continuation else {
                break;
            };
            request = next;
        }
        assert_eq!(
            seen, expected,
            "compact continuation must include the deferred first page"
        );
    }
}

#[test]
fn ambiguous_address_keys_and_scopes_remain_distinct_in_durable_joins() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("addresses.sqlite3");
    let a = Address::Opaque {
        system: "a:b".into(),
        value: "c".into(),
    };
    let b = Address::Opaque {
        system: "a".into(),
        value: "b:c".into(),
    };
    let mut graph = graph();
    graph.relation_edges.clear();
    for address in [&a, &b] {
        for scoped in [
            Scope {
                snapshot_revision: Some("abc".into()),
                ..scope()
            },
            Scope {
                context_id: Some("abc".into()),
                ..scope()
            },
        ] {
            graph.relation_edges.push(relation_edge_with_id(
                "source",
                address.clone(),
                RelationKind::Calls,
                address.clone(),
                scoped.clone(),
                None,
            ));
            graph.unresolved_relations.push(unresolved_relation_with_id(
                address.clone(),
                RelationKind::Calls,
                TargetConstraint::Candidates {
                    addresses: vec![b.clone(), a.clone(), b.clone()],
                },
                UnresolvedReason::AmbiguousTarget,
                producer(),
                scoped,
                None,
                vec![],
                None,
                None,
            ));
        }
    }
    publish_and_reload(&mut graph, &path);
    assert_eq!(graph.relation_edges.len(), 4);
    let repository = SqliteKnowledgeRepository::open(&path).unwrap();
    for address in [&a, &b] {
        let edges = repository.relation_edges_for_subject(address).unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges
            .iter()
            .all(|edge| &edge.subject == address && &edge.object == address));
        let propositions = repository.propositions_for_object(address).unwrap();
        assert_eq!(propositions.len(), 1);
        let engine = QueryEngine::new(graph.clone(), path.clone());
        let result = engine
            .inspect(InspectQuery::Proposition {
                id: propositions[0].id.to_string(),
            })
            .unwrap()
            .into_value()
            .unwrap();
        let occurrences: Vec<RelationEdge> =
            serde_json::from_value(result["occurrences"].clone()).unwrap();
        assert_eq!(occurrences.len(), 2);
        for edge in edges {
            assert!(occurrences.contains(&edge));
        }
        let attempts = repository.unresolved_for_source(address).unwrap();
        assert_eq!(attempts.len(), 2);
        for attempt in attempts {
            assert_eq!(&attempt.source, address);
            let TargetConstraint::Candidates { addresses } = &attempt.target_constraint else {
                panic!("candidates")
            };
            assert_eq!(addresses.len(), 2);
            assert!(addresses.contains(&a) && addresses.contains(&b));
            let result = engine
                .inspect(InspectQuery::Unresolved {
                    id: attempt.id.to_string(),
                    after: None,
                    limit: None,
                })
                .unwrap()
                .into_value()
                .unwrap();
            assert_eq!(
                serde_json::from_value::<UnresolvedRelation>(result["items"][0].clone()).unwrap(),
                attempt
            );
        }
    }
}
