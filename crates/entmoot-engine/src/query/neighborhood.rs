//! Resumable scoped breadth-first neighborhoods over original relation edges.
use super::*;
use entmoot_contracts::query::{HopEvidenceFilter, ProviderRoleSelector, RoleSelectionReceipt};
use std::collections::{BTreeSet, VecDeque};

use super::adjacency::ScopedState;
#[derive(serde::Serialize, serde::Deserialize)]
struct FrontierState {
    state: ScopedState,
    parent: Option<(usize, usize)>,
    depth: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NeighborhoodSearch {
    start: String,
    direction: CallDirection,
    predicates: Vec<String>,
    evidence_filter: HopEvidenceFilter,
    endpoint_role: Option<ProviderRoleSelector>,
    roots: usize,
    root_keys: Vec<String>,
    selection: Option<super::roles::RoleSelection>,
    anchors_by_key: HashMap<String, Vec<usize>>,
    states: Vec<FrontierState>,
    queue: VecDeque<usize>,
    visited: HashSet<ScopedState>,
    discovered: HashSet<usize>,
    pub counters: WalkCounters,
    predicate_filtered: usize,
    evidence_filtered: usize,
    scope_filtered: usize,
    depth_frontier: HashSet<usize>,
    qualified_edges: HashMap<usize, bool>,
    pending: Option<(usize, usize, usize)>,
    pub done: bool,
    fixed_bytes: usize,
    unresolved: BTreeSet<(String, usize)>,
    delivered: usize,
}

impl NeighborhoodSearch {
    pub fn new(
        engine: &QueryEngine,
        start: &str,
        direction: CallDirection,
        predicates: &[String],
        evidence_filter: HopEvidenceFilter,
        endpoint_role: Option<&ProviderRoleSelector>,
    ) -> Result<Self, QueryError> {
        parse_walk_predicates(predicates)?;
        let selection = endpoint_role
            .map(|selector| {
                if selector.role.trim().is_empty()
                    || !(1..=100_000).contains(&selector.max_assertions)
                {
                    return Err(QueryError::InvalidInput(
                        "endpoint role and assertion budget are outside supported bounds".into(),
                    ));
                }
                Ok(engine.indexes.roles.select(
                    &engine.graph,
                    &[&selector.role],
                    selector.max_assertions,
                    selector.include_hypotheses,
                ))
            })
            .transpose()?;
        let mut anchors_by_key: HashMap<String, Vec<usize>> = HashMap::new();
        if let Some(selection) = &selection {
            for (index, anchor) in selection.anchors.iter().enumerate() {
                for key in engine.walk_endpoint_keys(&anchor.callable_id) {
                    anchors_by_key.entry(key).or_default().push(index);
                }
            }
        }
        let roots = engine.exact_subject_nodes(start);
        let root_keys = if roots.len() == 1 {
            engine.walk_endpoint_keys(start)
        } else {
            vec![]
        };
        let states: Vec<_> = root_keys
            .iter()
            .filter_map(|key| engine.indexes.relations.ids.get(key))
            .map(|&node| FrontierState {
                state: ScopedState {
                    node,
                    scope: None,
                    boundary: false,
                },
                parent: None,
                depth: 0,
            })
            .collect();
        let queue: VecDeque<usize> = (0..states.len()).collect();
        let visited = states.iter().map(|s| s.state).collect();
        let discovered: HashSet<_> = states.iter().map(|s| s.state.node).collect();
        let unresolved = discovered
            .iter()
            .flat_map(|&node| {
                engine
                    .indexes
                    .unresolved_calls_by_address
                    .get(&engine.indexes.relations.keys[node])
                    .into_iter()
                    .flatten()
            })
            .map(|&index| {
                (
                    engine.graph.unresolved_relations[index].id.to_string(),
                    index,
                )
            })
            .collect();
        let fixed_bytes = selection
            .as_ref()
            .map(|s| super::output::byte_len(&s.anchors))
            .transpose()?
            .unwrap_or(0)
            .saturating_mul(8)
            + anchors_by_key
                .iter()
                .map(|(k, v)| k.capacity() + v.capacity() * 8 + 64)
                .sum::<usize>();
        let fixed_bytes = fixed_bytes
            + start.len() * 2
            + root_keys
                .iter()
                .map(|k| k.capacity() * 2 + 32)
                .sum::<usize>()
            + predicates
                .iter()
                .map(|p| p.capacity() * 2 + 32)
                .sum::<usize>()
            + endpoint_role
                .map(super::output::byte_len)
                .transpose()?
                .unwrap_or(0)
                * 8;
        Ok(Self {
            start: start.into(),
            direction,
            predicates: predicates.to_vec(),
            evidence_filter,
            endpoint_role: endpoint_role.cloned(),
            roots: roots.len(),
            root_keys,
            selection,
            anchors_by_key,
            states,
            queue,
            visited,
            discovered,
            counters: WalkCounters::default(),
            predicate_filtered: 0,
            evidence_filtered: 0,
            scope_filtered: 0,
            depth_frontier: HashSet::new(),
            qualified_edges: HashMap::new(),
            pending: None,
            done: false,
            fixed_bytes,
            unresolved,
            delivered: 0,
        })
    }

    pub fn retained_bytes(&self) -> usize {
        self.fixed_bytes
            + 1024
            + self.states.len() * 512
            + self.unresolved.len() * 256
            + self.qualified_edges.len() * 64
    }

    pub fn advance(
        &mut self,
        engine: &QueryEngine,
        repository: &SqliteKnowledgeRepository,
        budget: BoundedWalkBudget,
        memory_limit: usize,
    ) -> Result<Value, QueryError> {
        let started = std::time::Instant::now();
        let allowed = parse_walk_predicates(&self.predicates)?;
        let forward = self.direction == CallDirection::Callees;
        let mut evidence_micros = 0u64;
        let mut evidence_claims_examined = 0usize;
        let mut reconstructed_edges = 0usize;
        let mut hits = Vec::new();
        let mut counters = WalkCounters::default();
        let mut result_budget_exhausted = false;
        'search: while !self.done {
            if self.fixed_bytes
                + 2048
                + self.states.len() * 512
                + self.unresolved.len() * 256
                + self.qualified_edges.len() * 64
                > memory_limit
            {
                return Err(QueryError::Unavailable("neighborhood continuation exceeded retained memory; narrow the selector or depth".into()));
            }
            if self.pending.is_none() {
                let Some(&state_index) = self.queue.front() else {
                    self.done = true;
                    break;
                };
                let state = &self.states[state_index];
                let neighbors = engine.indexes.relations.edges(state.state.node, forward);
                if neighbors.is_empty() {
                    self.queue.pop_front();
                    continue;
                }
                if state.depth == budget.max_hops {
                    self.depth_frontier.insert(state.state.node);
                    self.queue.pop_front();
                    continue;
                }
                if counters.expanded_nodes >= budget.max_expansions {
                    counters.expansion_budget_exhausted = true;
                    break;
                }
                counters.expanded_nodes += 1;
                self.queue.pop_front();
                self.pending = Some((state_index, 0, 0));
            }
            let Some((state_index, offset, fanout)) = self.pending else {
                continue;
            };
            let state = &self.states[state_index];
            let depth = state.depth;
            let path_scope = state.state.scope;
            let neighbors = engine.indexes.relations.edges(state.state.node, forward);
            let Some(&index) = neighbors.get(offset) else {
                self.pending = None;
                continue;
            };
            if counters.examined_edges >= budget.max_examined_edges {
                counters.edge_budget_exhausted = true;
                break;
            }
            counters.examined_edges += 1;
            self.pending = Some((state_index, offset + 1, fanout));
            let edge = &engine.graph.relation_edges[index];
            if !allowed.is_empty() && !allowed.contains(&edge.predicate) {
                self.predicate_filtered += 1;
                continue;
            }
            if budget.max_edges_per_node > 0 && fanout == budget.max_edges_per_node {
                counters.fanout_truncated = true;
                self.pending = None;
                continue;
            }
            self.pending = Some((state_index, offset + 1, fanout + 1));
            let previous = self.states[state_index].parent.map(|(_, edge)| edge);
            let Some(scope_key) = engine.transition_scope(path_scope, previous, index, forward)
            else {
                self.scope_filtered += 1;
                continue;
            };
            if self.evidence_filter == HopEvidenceFilter::ResolvedOnly {
                let qualifies = if let Some(qualifies) = self.qualified_edges.get(&index) {
                    *qualifies
                } else {
                    let evidence_started = std::time::Instant::now();
                    let claims = repository
                        .relation_claims_for_edges(&[edge])?
                        .pop()
                        .unwrap_or_default();
                    evidence_micros = evidence_micros.saturating_add(
                        u64::try_from(evidence_started.elapsed().as_micros()).unwrap_or(u64::MAX),
                    );
                    evidence_claims_examined += claims.len();
                    let resolution_states: Vec<_> = claims
                        .iter()
                        .filter_map(|claim| applicable_resolution_state(claim, edge))
                        .collect();
                    let qualifies = !resolution_states.is_empty()
                        && resolution_states.iter().all(|state| {
                            matches!(
                                state,
                                ResolutionState::ArtifactResolved
                                    | ResolutionState::SemanticResolved
                                    | ResolutionState::Validated
                            )
                        });
                    self.qualified_edges.insert(index, qualifies);
                    qualifies
                };
                if !qualifies {
                    self.evidence_filtered += 1;
                    continue;
                }
            }
            let endpoint = if self.direction == CallDirection::Callers {
                &edge.subject
            } else {
                &edge.object
            };
            let pair = engine.indexes.relations.endpoints[index];
            let node = if forward { pair.1 } else { pair.0 };
            let endpoint_key = &engine.indexes.relations.keys[node];
            // Reverse walks must distinguish arriving behind an explicit
            // observation bridge from an ordinary edge in the same scope.
            let boundary =
                self.direction == CallDirection::Callers && is_observation_boundary(edge);
            let state = ScopedState {
                node,
                scope: Some(scope_key),
                boundary,
            };
            if self.visited.insert(state) {
                let next_index = self.states.len();
                self.states.push(FrontierState {
                    state,
                    parent: Some((state_index, index)),
                    depth: depth + 1,
                });
                self.queue.push_back(next_index);
            }
            let fresh = self.discovered.insert(node);
            if fresh {
                for &index in engine
                    .indexes
                    .unresolved_calls_by_address
                    .get(endpoint_key)
                    .into_iter()
                    .flatten()
                {
                    self.unresolved.insert((
                        engine.graph.unresolved_relations[index].id.to_string(),
                        index,
                    ));
                }
            }
            let anchor_indexes = self
                .anchors_by_key
                .get(endpoint_key)
                .map_or(&[][..], Vec::as_slice);
            if fresh && (self.selection.is_none() || !anchor_indexes.is_empty()) {
                let mut next_path = vec![index];
                let mut cursor = state_index;
                while let Some((parent, edge)) = self.states[cursor].parent {
                    next_path.push(edge);
                    cursor = parent;
                }
                if self.direction == CallDirection::Callees {
                    next_path.reverse();
                }
                let evidence_started = std::time::Instant::now();
                let witness = engine.walk_path_receipt(
                    repository,
                    next_path.len(),
                    endpoint_key,
                    &next_path,
                    budget.max_evidence_per_hop,
                )?;
                evidence_micros = evidence_micros.saturating_add(
                    u64::try_from(evidence_started.elapsed().as_micros()).unwrap_or(u64::MAX),
                );
                reconstructed_edges += next_path.len();
                evidence_claims_examined += witness["hops"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|hop| hop["relation_claim_count"].as_u64())
                    .filter_map(|n| usize::try_from(n).ok())
                    .sum::<usize>();
                let role_bounds = self.selection.as_ref().map(|_| {
                    ResultBounds::page(
                        budget.max_evidence_per_hop,
                        anchor_indexes.len().min(budget.max_evidence_per_hop),
                        anchor_indexes.len(),
                    )
                });
                let role_anchors: Vec<_> = self
                    .selection
                    .as_ref()
                    .into_iter()
                    .flat_map(|s| {
                        anchor_indexes
                            .iter()
                            .take(budget.max_evidence_per_hop)
                            .map(|&i| &s.anchors[i])
                    })
                    .collect();
                let role_summaries: Vec<_> = self
                    .selection
                    .as_ref()
                    .into_iter()
                    .flat_map(|s| {
                        anchor_indexes
                            .iter()
                            .map(|&i| super::output::compact_role(&s.anchors[i]))
                    })
                    .collect();
                hits.push(json!({"subject": endpoint, "witness": witness, "role_summaries": role_summaries,
                        "role_anchors": role_anchors, "role_anchor_bounds": role_bounds}));
                if hits.len() == budget.max_results {
                    // Conservative: remaining work is deliberately not scanned
                    // to turn a budget stop into a spurious completeness claim.
                    result_budget_exhausted = true;
                    break 'search;
                }
            }
        }
        self.counters.expanded_nodes += counters.expanded_nodes;
        self.counters.examined_edges += counters.examined_edges;
        self.counters.fanout_truncated |= counters.fanout_truncated;
        counters.fanout_truncated = self.counters.fanout_truncated;
        self.delivered += hits.len();
        let unresolved_bounds = ResultBounds::page(
            budget.max_evidence_per_hop,
            self.unresolved.len().min(budget.max_evidence_per_hop),
            self.unresolved.len(),
        );
        let role_selection = self
            .endpoint_role
            .as_ref()
            .zip(self.selection.as_ref())
            .map(|(selector, selected)| {
                let colocated: BTreeSet<_> = self
                    .root_keys
                    .iter()
                    .flat_map(|key| self.anchors_by_key.get(key).into_iter().flatten().copied())
                    .collect();
                let bounds = ResultBounds::page(
                    budget.max_evidence_per_hop,
                    colocated.len().min(budget.max_evidence_per_hop),
                    colocated.len(),
                );
                RoleSelectionReceipt {
                    selector: selector.clone(),
                    matching_assertions: selected.matching,
                    excluded_hypotheses: selected.excluded_hypotheses,
                    unmapped_assertion_ids: selected.unmapped.clone(),
                    truncated: selected.truncated,
                    colocated: colocated
                        .into_iter()
                        .take(budget.max_evidence_per_hop)
                        .map(|i| selected.anchors[i].clone())
                        .collect(),
                    colocated_bounds: bounds,
                }
            });
        let truncated = counters.expansion_budget_exhausted
            || counters.edge_budget_exhausted
            || counters.fanout_truncated
            || result_budget_exhausted
            || !self.depth_frontier.is_empty()
            || self.selection.as_ref().is_some_and(|s| s.truncated);
        let mut gaps = vec![
            "repository_call_graph_completeness_not_established",
            "dependency_call_graph_completeness_not_established",
            "native_library_call_graph_completeness_not_established",
        ];
        if self.roots == 0 {
            gaps.push("start_not_materialized");
        } else if self.roots > 1 {
            gaps.push("subject_is_ambiguous");
        }
        if truncated {
            gaps.push("budget_exceeded");
        }
        if self.scope_filtered > 0 {
            gaps.push("scope_composition_not_established");
        }
        if let Some(selection) = &self.selection {
            gaps.push("dataflow_not_established");
            if selection.matching == 0 {
                gaps.push("no_provider_stated_role_matched");
            }
            if !selection.unmapped.is_empty() {
                gaps.push("role_mapping_not_established");
            }
        }
        let conflicted = hits
            .iter()
            .any(|hit| hit["witness"]["classification"] == "conflicted");
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA, "operation": "traverse", "view": "neighborhood",
            "snapshot": engine.graph.snapshot, "start": self.start, "direction": self.direction,
            "predicates": self.predicates, "evidence_filter": self.evidence_filter,
            "status": if truncated { "truncated" } else if conflicted { "conflict" } else if hits.is_empty() { "unknown" } else { "known" },
            "bounds": ResultBounds { offset: self.delivered - hits.len(), limit: budget.max_results, total_matches: self.delivered, truncated },
            "items": hits, "role_selection": role_selection,
            "unresolved_calls": self.unresolved.iter().take(budget.max_evidence_per_hop).map(|(_,i)| &engine.graph.unresolved_relations[*i]).collect::<Vec<_>>(),
            "unresolved_call_bounds": unresolved_bounds,
            "gaps": gaps,
            "predicate_filtered_edges": self.predicate_filtered,
            "evidence_filtered_edges": self.evidence_filtered,
            "scope_filtered_edges": self.scope_filtered,
            "depth_frontier_count": self.depth_frontier.len(),
            "result_budget_exhausted": result_budget_exhausted,
            "search_receipt": {
                "evidence_micros":evidence_micros,"evidence_claims_examined":evidence_claims_examined,"reconstructed_edges":reconstructed_edges,
                "expanded_nodes": counters.expanded_nodes, "examined_edges": counters.examined_edges,
                "cumulative_expanded_nodes": self.counters.expanded_nodes,
                "cumulative_examined_edges": self.counters.examined_edges,
                "cumulative_examined_shortcuts": 0,
                "elapsed_micros": u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                "budget": budget, "fanout_truncated": counters.fanout_truncated,
                "expansion_budget_exhausted": counters.expansion_budget_exhausted,
                "edge_budget_exhausted": counters.edge_budget_exhausted
            }
        }))
    }
}

impl QueryEngine {
    pub(super) fn bounded_neighborhood(
        &self,
        start: &str,
        direction: CallDirection,
        predicates: &[String],
        evidence_filter: HopEvidenceFilter,
        endpoint_role: Option<&ProviderRoleSelector>,
        budget: BoundedWalkBudget,
    ) -> Result<Value, QueryError> {
        let budget = validate_walk_budget(budget)?;
        NeighborhoodSearch::new(
            self,
            start,
            direction,
            predicates,
            evidence_filter,
            endpoint_role,
        )?
        .advance(self, &self.repository()?, budget, usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeStore;
    use entmoot_contracts::{
        relation_claim_with_id, relation_edge_with_id, topology_node_address, Scope, SubjectId,
    };

    #[test]
    fn neighborhood_filters_preserve_unknowns_and_bound_work_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("knowledge.sqlite3");
        let mut graph = TopologyGraph::new("neighborhood");
        for id in ["a", "b", "c", "d"] {
            graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
        }
        let scope = Scope {
            domain: "source".into(),
            snapshot_revision: None,
            snapshot_digest: None,
            occurrence_id: None,
            context_id: None,
        };
        let producer = entmoot_contracts::ProducerRef {
            id: "fixture".into(),
            name: "fixture".into(),
            version: Some("1".into()),
            config_fingerprint: None,
            environment_fingerprint: None,
            capabilities: vec![],
        };
        for (a, b, states) in [
            ("a", "b", vec![ResolutionState::SemanticResolved]),
            ("b", "c", vec![ResolutionState::ArtifactResolved]),
            (
                "c",
                "d",
                vec![ResolutionState::SemanticResolved, ResolutionState::Refuted],
            ),
        ] {
            let mut site_scope = scope.clone();
            site_scope.occurrence_id = Some(format!("{a}-{b}"));
            let edge = relation_edge_with_id(
                "source",
                topology_node_address(a),
                RelationKind::Calls,
                topology_node_address(b),
                site_scope.clone(),
                None,
            );
            for state in states {
                graph.relation_claims.push(relation_claim_with_id(
                    SubjectId::RelationEdge {
                        edge_id: edge.id.clone(),
                    },
                    "resolution_state",
                    RelationClaimValue::ResolutionState { state },
                    producer.clone(),
                    site_scope.clone(),
                    vec![],
                    None,
                ));
            }
            graph.relation_edges.push(edge);
        }
        let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        drop(repository);
        let engine = QueryEngine::new(graph.clone(), store.clone());
        let query = |root: &str, direction, filter, budget| {
            engine
                .traverse(TraverseQuery::Neighborhood {
                    resumable: false,
                    continuation: None,
                    start: root.into(),
                    direction,
                    predicates: vec!["calls".into()],
                    evidence_filter: filter,
                    endpoint_role: None,
                    budget,
                })
                .unwrap()
                .into_value()
                .unwrap()
        };
        let budget = BoundedWalkBudget {
            max_hops: 4,
            max_edges_per_node: 0,
            ..Default::default()
        };
        let all = query("a", CallDirection::Callees, HopEvidenceFilter::Any, budget);
        assert_eq!(all["items"].as_array().unwrap().len(), 3);
        assert_eq!(all["status"], "conflict");
        assert_eq!(all["items"][2]["witness"]["classification"], "conflicted");
        let resolved = query(
            "a",
            CallDirection::Callees,
            HopEvidenceFilter::ResolvedOnly,
            budget,
        );
        assert_eq!(resolved["items"].as_array().unwrap().len(), 2);
        assert_eq!(resolved["evidence_filtered_edges"], 1);
        assert!(resolved["gaps"]
            .as_array()
            .unwrap()
            .contains(&json!("repository_call_graph_completeness_not_established")));
        let callers = query(
            "c",
            CallDirection::Callers,
            HopEvidenceFilter::ResolvedOnly,
            budget,
        );
        assert_eq!(callers["items"].as_array().unwrap().len(), 2);
        assert_eq!(
            callers["items"][1]["witness"]["hops"][0]["relation_edge"]["subject"],
            serde_json::to_value(topology_node_address("a")).unwrap()
        );
        for limited in [
            BoundedWalkBudget {
                max_hops: 1,
                ..budget
            },
            BoundedWalkBudget {
                max_expansions: 1,
                ..budget
            },
            BoundedWalkBudget {
                max_examined_edges: 1,
                ..budget
            },
            BoundedWalkBudget {
                max_results: 1,
                ..budget
            },
        ] {
            let result = query("a", CallDirection::Callees, HopEvidenceFilter::Any, limited);
            assert_eq!(result["status"], "truncated");
            assert!(result["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!("budget_exceeded")));
            assert!(
                result["search_receipt"]["examined_edges"].as_u64().unwrap()
                    <= limited.max_examined_edges as u64
            );
        }
        assert_eq!(
            query(
                "absent",
                CallDirection::Callees,
                HopEvidenceFilter::Any,
                budget
            )["status"],
            "unknown"
        );
        let deferred = query(
            "a",
            CallDirection::Callees,
            HopEvidenceFilter::Any,
            BoundedWalkBudget {
                max_evidence_per_hop: 0,
                ..budget
            },
        );
        assert_eq!(deferred["status"], "conflict");
        assert_eq!(deferred["bounds"]["truncated"], false);
        assert_eq!(
            deferred["items"][0]["witness"]["hops"][0]["relation_claims_truncated"],
            true
        );
        // Foreign build context cannot silently compose into a deeper witness.
        graph.relation_edges[1].scope.context_id = Some("other-build".into());
        let scoped = QueryEngine::new(graph, store)
            .traverse(TraverseQuery::Neighborhood {
                resumable: false,
                continuation: None,
                start: "a".into(),
                direction: CallDirection::Callees,
                predicates: vec!["calls".into()],
                evidence_filter: HopEvidenceFilter::Any,
                endpoint_role: None,
                budget,
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(scoped["items"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["scope_filtered_edges"], 1);
    }
}
