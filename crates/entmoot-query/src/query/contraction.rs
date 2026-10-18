//! Disposable chain contraction; every shortcut retains its original edges.
use super::{walk_cache::RetainedSize, QueryEngine};
use entmoot_contracts::RelationKind;
use std::collections::HashSet;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ShortcutOverlay {
    pub chains: Vec<Vec<usize>>,
    /// Original edge -> chain and offset. Interior query endpoints split ranges.
    pub positions: Vec<Option<(usize, usize)>>,
    pub components: Vec<usize>,
    pub interiors: Vec<Option<(usize, usize)>>,
}

impl RetainedSize for ShortcutOverlay {
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.positions.capacity() * std::mem::size_of::<Option<(usize, usize)>>()
            + self.interiors.capacity() * std::mem::size_of::<Option<(usize, usize)>>()
            + self.components.capacity() * std::mem::size_of::<usize>()
            + self.chains.capacity() * std::mem::size_of::<Vec<usize>>()
            + self
                .chains
                .iter()
                .map(|c| c.capacity() * std::mem::size_of::<usize>())
                .sum::<usize>()
    }
}

impl QueryEngine {
    pub(super) fn shortcut_overlay(
        &self,
        allowed: &HashSet<RelationKind>,
    ) -> (Option<std::sync::Arc<ShortcutOverlay>>, usize) {
        if !self.options.contract_paths || self.options.shortcut_cache_entries == 0 {
            return (None, 0);
        }
        let mut selector: Vec<_> = allowed
            .iter()
            .filter_map(|p| serde_json::to_string(p).ok())
            .collect();
        selector.sort();
        let key = selector.join(",");
        if let Some(overlay) = self.shortcuts.get(&key) {
            return (Some(overlay), 0);
        }
        let Ok(_building) = self.shortcut_build.try_lock() else {
            return (None, 0);
        };
        if let Some(overlay) = self.shortcuts.get(&key) {
            return (Some(overlay), 0);
        }
        let edges = &self.graph.relation_edges;
        let index = &self.indexes.relations;
        let estimate = edges
            .len()
            .saturating_mul(256)
            .saturating_add(index.keys.len().saturating_mul(256));
        if edges.len() > self.options.max_index_edges
            || estimate > self.options.shortcut_cache_bytes
        {
            return (None, 0);
        }
        let mut incoming = vec![Vec::new(); index.keys.len()];
        let mut outgoing = vec![Vec::new(); index.keys.len()];
        let mut components = super::adjacency::DisjointSet::new(index.keys.len());
        let mut work = 0;
        let charge = |work: &mut usize, amount: usize| {
            if amount > self.options.max_index_edges.saturating_sub(*work) {
                return false;
            }
            *work += amount;
            true
        };
        for (e, edge) in edges.iter().enumerate() {
            if !charge(&mut work, 1) {
                return (None, work);
            }
            if !allowed.is_empty() && !allowed.contains(&edge.predicate) {
                continue;
            }
            let (from, to) = index.endpoints[e];
            outgoing[from].push(e);
            incoming[to].push(e);
            components.union(from, to);
        }
        let components = components.labels();
        let Some(strong) = super::adjacency::strong_components(
            &outgoing,
            &incoming,
            |edge, forward| {
                let (a, b) = index.endpoints[edge];
                if forward {
                    b
                } else {
                    a
                }
            },
            || charge(&mut work, 1),
        ) else {
            return (None, work);
        };
        let mut cyclic = vec![false; index.keys.len()];
        for members in strong {
            if members.len() > 1 {
                for node in members {
                    cyclic[node] = true;
                }
            }
        }
        let mut interior = vec![false; index.keys.len()];
        for n in 0..index.keys.len() {
            let ([before], [after]) = (incoming[n].as_slice(), outgoing[n].as_slice()) else {
                continue;
            };
            if !charge(&mut work, 2) {
                return (None, work);
            }
            interior[n] = !cyclic[n]
                && self
                    .indexes
                    .nodes_by_address
                    .get(&index.keys[n])
                    .is_some_and(|nodes| nodes.len() == 1)
                && self.indexes.composition_scopes[*before].is_some()
                && self.indexes.composition_scopes[*before]
                    == self.indexes.composition_scopes[*after]
                && edges[*before].predicate == edges[*after].predicate
                && !super::is_observation_boundary(&edges[*before])
                && !super::is_observation_boundary(&edges[*after]);
        }
        let mut overlay = ShortcutOverlay {
            chains: Vec::new(),
            positions: vec![None; edges.len()],
            interiors: vec![None; index.keys.len()],
            components,
        };
        for start in 0..index.keys.len() {
            if interior[start] {
                continue;
            }
            for &first in &outgoing[start] {
                if !charge(&mut work, 1) {
                    return (None, work);
                }
                if super::is_observation_boundary(&edges[first])
                    || self.indexes.composition_scopes[first].is_none()
                {
                    continue;
                }
                let mut chain = vec![first];
                let mut next = index.endpoints[first].1;
                while interior[next] && next != start {
                    if !charge(&mut work, 1) {
                        return (None, work);
                    }
                    let edge = outgoing[next][0];
                    chain.push(edge);
                    next = index.endpoints[edge].1;
                }
                if next == start || chain.len() < 2 {
                    continue;
                }
                let id = overlay.chains.len();
                for (offset, &e) in chain.iter().enumerate() {
                    overlay.positions[e] = Some((id, offset));
                }
                for (offset, &edge) in chain.iter().enumerate().take(chain.len() - 1) {
                    overlay.interiors[index.endpoints[edge].1] = Some((id, offset + 1));
                }
                overlay.chains.push(chain);
            }
        }
        let overlay = std::sync::Arc::new(overlay);
        if !self.shortcuts.insert(key.clone(), overlay) {
            return (None, work);
        }
        (self.shortcuts.get(&key), work)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        KnowledgeRepository, KnowledgeStore, NodeKind, QueryEngine, SqliteKnowledgeRepository,
        TopologyGraph, TopologyNode,
    };
    use entmoot_contracts::query::BoundedWalkBudget;
    use entmoot_contracts::{relation_edge_with_id, topology_node_address, RelationKind, Scope};

    pub(super) fn chain(length: usize) -> TopologyGraph {
        let mut graph = TopologyGraph::new("fixture/shortcuts");
        for n in 0..=length {
            graph.upsert_node(TopologyNode::shell(
                n.to_string(),
                NodeKind::Callable,
                n.to_string(),
                None,
            ));
            if n > 0 {
                graph.relation_edges.push(relation_edge_with_id(
                    "fixture",
                    topology_node_address(&(n - 1).to_string()),
                    RelationKind::Calls,
                    topology_node_address(&n.to_string()),
                    Scope {
                        domain: "fixture/shortcuts".into(),
                        snapshot_revision: None,
                        snapshot_digest: None,
                        occurrence_id: None,
                        context_id: None,
                    },
                    None,
                ));
            }
        }
        graph
    }

    #[test]
    fn authored_dags_match_original_search_for_every_endpoint() {
        // Enumerate all forward-edge graphs on five vertices. Expected shortest
        // distances come from a separate, simple breadth-first oracle.
        let pairs: Vec<_> = (0..5)
            .flat_map(|a| (a + 1..5).map(move |b| (a, b)))
            .collect();
        for mask in 0..(1 << pairs.len()) {
            let mut graph = chain(4);
            graph.relation_edges.clear();
            let mut adjacency = vec![Vec::new(); 5];
            for (bit, &(a, b)) in pairs.iter().enumerate() {
                if mask & (1 << bit) == 0 {
                    continue;
                }
                adjacency[a].push(b);
                graph.relation_edges.push(relation_edge_with_id(
                    "fixture",
                    topology_node_address(&a.to_string()),
                    RelationKind::Calls,
                    topology_node_address(&b.to_string()),
                    Scope {
                        domain: "fixture/shortcuts".into(),
                        snapshot_revision: None,
                        snapshot_digest: None,
                        occurrence_id: None,
                        context_id: None,
                    },
                    None,
                ));
            }
            let fast = QueryEngine::new(graph.clone(), "absent.sqlite3");
            let slow = QueryEngine::with_options(
                graph,
                "absent.sqlite3",
                super::super::QueryEngineOptions {
                    contract_paths: false,
                    ..Default::default()
                },
            );
            for from in 0..5 {
                let mut distance = [None; 5];
                distance[from] = Some(0);
                let mut queue = std::collections::VecDeque::from([from]);
                while let Some(a) = queue.pop_front() {
                    for &b in &adjacency[a] {
                        if distance[b].is_none() {
                            distance[b] = distance[a].map(|d| d + 1);
                            queue.push_back(b);
                        }
                    }
                }
                // A separate DAG fold gives the lexicographically first shortest
                // witness; agreement between two search modes alone is insufficient.
                let mut witnesses: Vec<Option<Vec<String>>> = vec![None; 5];
                witnesses[from] = Some(vec![]);
                for source in from..5 {
                    for &target in &adjacency[source] {
                        let Some(mut candidate) = witnesses[source].clone() else {
                            continue;
                        };
                        let edge = fast
                            .graph
                            .relation_edges
                            .iter()
                            .find(|edge| {
                                edge.subject == topology_node_address(&source.to_string())
                                    && edge.object == topology_node_address(&target.to_string())
                            })
                            .unwrap();
                        candidate.push(edge.id.to_string());
                        if witnesses[target].as_ref().is_none_or(|prior| {
                            (candidate.len(), &candidate) < (prior.len(), prior)
                        }) {
                            witnesses[target] = Some(candidate);
                        }
                    }
                }
                for (to, expected) in distance.iter().enumerate() {
                    let run = |e: &QueryEngine| {
                        e.discover_walk(
                            &from.to_string(),
                            &to.to_string(),
                            &e.walk_endpoint_keys(&from.to_string()),
                            &e.walk_endpoint_keys(&to.to_string()),
                            &[RelationKind::Calls].into_iter().collect(),
                            BoundedWalkBudget {
                                max_hops: 5,
                                ..Default::default()
                            },
                        )
                        .unwrap()
                    };
                    let a = run(&fast);
                    let b = run(&slow);
                    assert_eq!(a.candidates, b.candidates, "mask={mask} {from}->{to}");
                    assert_eq!(
                        a.candidates.first().map(|c| c
                            .2
                            .iter()
                            .map(|&edge| fast.graph.relation_edges[edge].id.to_string())
                            .collect::<Vec<_>>()),
                        witnesses[to],
                        "witness mask={mask} {from}->{to}"
                    );
                    assert_eq!(
                        a.candidates.first().map(|c| c.0),
                        *expected,
                        "mask={mask} {from}->{to}"
                    );
                }
            }
        }
    }

    #[test]
    fn contraction_matches_original_witnesses_at_branches_and_cycles() {
        // Two equally short routes, a longer route, a back edge, and a disconnected node.
        let routes = [
            (0, 1),
            (1, 2),
            (2, 7),
            (0, 3),
            (3, 4),
            (4, 7),
            (0, 5),
            (5, 6),
            (6, 8),
            (8, 7),
            (4, 3),
        ];
        let mut graph = chain(9);
        graph.relation_edges.clear();
        for (a, b) in routes {
            graph.relation_edges.push(relation_edge_with_id(
                "fixture",
                topology_node_address(&a.to_string()),
                RelationKind::Calls,
                topology_node_address(&b.to_string()),
                Scope {
                    domain: "fixture/shortcuts".into(),
                    snapshot_revision: None,
                    snapshot_digest: None,
                    occurrence_id: None,
                    context_id: None,
                },
                None,
            ));
        }
        let accelerated = QueryEngine::new(graph.clone(), "absent.sqlite3");
        let reference = QueryEngine::with_options(
            graph,
            "absent.sqlite3",
            super::super::QueryEngineOptions {
                contract_paths: false,
                ..Default::default()
            },
        );
        let allowed = [RelationKind::Calls].into_iter().collect();
        let budget = BoundedWalkBudget {
            max_hops: 20,
            ..Default::default()
        };
        for from in 0..10 {
            for to in 0..10 {
                let from = from.to_string();
                let to = to.to_string();
                let keys = |e: &QueryEngine, s: &str| e.walk_endpoint_keys(s);
                let run = |e: &QueryEngine| {
                    e.discover_walk(&from, &to, &keys(e, &from), &keys(e, &to), &allowed, budget)
                        .unwrap()
                };
                let a = run(&accelerated);
                let b = run(&reference);
                assert_eq!(a.candidates, b.candidates, "{from}->{to}");
            }
        }
    }

    #[test]
    fn cached_shortcuts_preserve_hops_and_interior_endpoints() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("knowledge.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        let engine = QueryEngine::new(chain(80), store);
        let budget = BoundedWalkBudget {
            max_hops: 100,
            ..Default::default()
        };
        for (from, to, hops) in [("0", "80", 80), ("17", "62", 45), ("17", "18", 1)] {
            let result = engine
                .bounded_walk(from, to, &["calls".into()], budget)
                .unwrap();
            assert_eq!(result["paths"][0]["hop_count"], hops);
            assert_eq!(result["paths"][0]["hops"].as_array().unwrap().len(), hops);
            assert!(
                result["search_receipt"]["examined_shortcuts"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0
            );
            let too_short = engine.bounded_walk(
                from,
                to,
                &["calls".into()],
                BoundedWalkBudget {
                    max_hops: hops - 1,
                    ..budget
                },
            );
            if hops > 1 {
                assert!(too_short.unwrap()["paths"].as_array().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn index_limits_fall_back_and_warm_chains_reduce_search_work() {
        let graph = chain(120);
        let options = super::super::QueryEngineOptions::default();
        let make = |options| QueryEngine::with_options(graph.clone(), "unused.sqlite3", options);
        let fast = make(options);
        let slow = make(super::super::QueryEngineOptions {
            contract_paths: false,
            ..options
        });
        let exhausted = make(super::super::QueryEngineOptions {
            max_index_edges: 121,
            ..options
        });
        let tiny = make(super::super::QueryEngineOptions {
            shortcut_cache_bytes: 1,
            ..options
        });
        let run = |engine: &QueryEngine| {
            engine
                .discover_walk(
                    "0",
                    "120",
                    &engine.walk_endpoint_keys("0"),
                    &engine.walk_endpoint_keys("120"),
                    &[RelationKind::Calls].into_iter().collect(),
                    BoundedWalkBudget {
                        max_hops: 128,
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        let expected = run(&slow);
        let first = run(&fast);
        assert_eq!(first.candidates, expected.candidates);
        assert!(first.counters.expanded_nodes < expected.counters.expanded_nodes);
        assert!(first.counters.index_examined_edges > 120);
        let warm = run(&fast);
        assert_eq!(warm.counters.index_examined_edges, 0);
        for engine in [exhausted, tiny] {
            let result = run(&engine);
            assert_eq!(result.candidates, expected.candidates);
            assert_eq!(result.counters.examined_shortcuts, 0);
        }
    }

    #[test]
    fn resumed_path_chunks_match_uninterrupted_and_retries_do_not_advance() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("knowledge.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        for contract_paths in [false, true] {
            let engine = QueryEngine::with_options(
                chain(8),
                store.clone(),
                super::super::QueryEngineOptions {
                    contract_paths,
                    ..Default::default()
                },
            );
            let mut request = serde_json::json!({"kind":"path","start":"0","target":"8","predicates":["calls"],"resumable":true,"budget":{"max_hops":10,"max_expansions":1,"max_examined_edges":1,"max_edges_per_node":0,"max_results":1}});
            let mut delivered = Vec::new();
            let mut previous_work = 0;
            for _ in 0..20 {
                let query = serde_json::from_value(request.clone()).unwrap();
                let result = serde_json::to_value(engine.traverse(query).unwrap()).unwrap();
                let work = result["search_receipt"]["cumulative_examined_edges"]
                    .as_u64()
                    .unwrap();
                assert!(work >= previous_work);
                previous_work = work;
                delivered.extend(result["paths"].as_array().unwrap().iter().cloned());
                if request.get("continuation").is_some() {
                    let retry = serde_json::to_value(
                        engine
                            .traverse(serde_json::from_value(request.clone()).unwrap())
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(result, retry);
                }
                let Some(token) = result["search_receipt"]["continuation"].as_str() else {
                    break;
                };
                request["continuation"] = token.into();
            }
            assert_eq!(delivered.len(), 1);
            assert_eq!(delivered[0]["hop_count"], 8);
            assert_eq!(previous_work, if contract_paths { 1 } else { 8 });
        }
    }

    #[test]
    fn neighborhoods_resume_inside_adjacency_without_recounting_expansions() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("knowledge.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&KnowledgeStore::default())
            .unwrap();
        let mut graph = chain(5);
        for edge in &mut graph.relation_edges {
            *edge = relation_edge_with_id(
                "fixture",
                topology_node_address("0"),
                RelationKind::Calls,
                edge.object.clone(),
                edge.scope.clone(),
                None,
            );
        }
        let engine = QueryEngine::new(graph, store);
        let mut request = serde_json::json!({"kind":"neighborhood","start":"0","direction":"callees","predicates":["calls"],"evidence_filter":"any","resumable":true,"budget":{"max_hops":10,"max_expansions":1,"max_examined_edges":1,"max_edges_per_node":0,"max_results":1}});
        let mut items = Vec::new();
        let mut last = serde_json::Value::Null;
        for _ in 0..10 {
            let result = serde_json::to_value(
                engine
                    .traverse(serde_json::from_value(request.clone()).unwrap())
                    .unwrap(),
            )
            .unwrap();
            items.extend(result["items"].as_array().unwrap().iter().cloned());
            last = result;
            let Some(token) = last["search_receipt"]["continuation"].as_str() else {
                break;
            };
            request["continuation"] = token.into();
        }
        assert_eq!(items.len(), 5);
        assert!(items.iter().all(|item| item["witness"]["hop_count"] == 1));
        assert_eq!(last["search_receipt"]["cumulative_examined_edges"], 5);
        assert_eq!(last["search_receipt"]["cumulative_expanded_nodes"], 1);
        assert!(last["search_receipt"]["continuation"].is_null());
    }

    #[test]
    fn parallel_occurrences_cycles_and_scope_changes_are_not_hidden() {
        let allowed = [RelationKind::Calls].into_iter().collect();
        let mut parallel = chain(2);
        let mut extra = parallel.relation_edges[0].clone();
        extra.scope.occurrence_id = Some("other-occurrence".into());
        extra = relation_edge_with_id(
            "fixture",
            extra.subject,
            extra.predicate,
            extra.object,
            extra.scope,
            None,
        );
        parallel.relation_edges.push(extra);
        let engine = QueryEngine::new(parallel, "unused.sqlite3");
        assert!(engine
            .shortcut_overlay(&allowed)
            .0
            .unwrap()
            .chains
            .is_empty());

        let mut cycle = chain(2);
        let mut back = cycle.relation_edges[0].clone();
        back.subject = topology_node_address("2");
        back.object = topology_node_address("0");
        back = relation_edge_with_id(
            "fixture",
            back.subject,
            back.predicate,
            back.object,
            back.scope,
            None,
        );
        cycle.relation_edges.push(back);
        let engine = QueryEngine::new(cycle, "unused.sqlite3");
        assert!(engine
            .shortcut_overlay(&allowed)
            .0
            .unwrap()
            .chains
            .is_empty());

        let mut scoped = chain(2);
        scoped.relation_edges[0].scope.context_id = Some("another-build".into());
        let edge = scoped.relation_edges.remove(0);
        scoped.relation_edges.insert(
            0,
            relation_edge_with_id(
                "fixture",
                edge.subject,
                edge.predicate,
                edge.object,
                edge.scope,
                None,
            ),
        );
        let engine = QueryEngine::new(scoped, "unused.sqlite3");
        assert!(engine
            .shortcut_overlay(&allowed)
            .0
            .unwrap()
            .chains
            .is_empty());
    }

    #[test]
    fn predicate_overlays_are_separate_bounded_and_reusable() {
        let engine = QueryEngine::with_options(
            chain(4),
            "unused.sqlite3",
            super::super::QueryEngineOptions {
                shortcut_cache_entries: 1,
                ..Default::default()
            },
        );
        let calls = [RelationKind::Calls].into_iter().collect();
        let imports = [RelationKind::Imports].into_iter().collect();
        let first = engine.shortcut_overlay(&calls).0.unwrap();
        // In-use overlays cannot be evicted to evade aggregate retention limits.
        assert!(engine.shortcut_overlay(&imports).0.is_none());
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &engine.shortcut_overlay(&calls).0.unwrap()
        ));
        drop(first);
        assert!(engine
            .shortcut_overlay(&imports)
            .0
            .unwrap()
            .chains
            .is_empty());
        assert!(engine.shortcut_overlay(&calls).1 > 0);
    }
}
