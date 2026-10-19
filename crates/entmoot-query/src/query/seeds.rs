//! Seed-role discovery and directed candidate-route queries.

use super::*;
use entmoot_contracts::query::{SeededPathReport, SeededPathRequest, SeededRoute};
use std::collections::{BTreeSet, VecDeque};

impl QueryEngine {
    /// Discover routes from exact provider-role locations through current callable
    /// attachments. Directed multi-target BFS avoids the source × sink product;
    /// each discovered pair is checked by the existing scoped bidirectional query.
    /// Co-location and absent represented routes are not dataflow conclusions.
    pub fn trace_seeds(&self, request: SeededPathRequest) -> Result<SeededPathReport, QueryError> {
        if self.deferred_navigation.is_some() {
            return self.navigation()?.trace_seeds(request);
        }
        let mut admission =
            super::admission::EdgeAdmission::new(self, &request.prune, request.evidence_filter)?;
        let repository = self.repository()?;
        super::validate_walk_budget(request.walk)?;
        if request.from_role.trim().is_empty()
            || request.to_role.trim().is_empty()
            || !(1..=100_000).contains(&request.max_seed_assertions)
            || !(1..=1_000).contains(&request.max_pairs)
            || !(1..=1_000_000).contains(&request.max_discovery_expansions)
            || !(1..=10_000_000).contains(&request.max_discovery_edges)
        {
            return Err(QueryError::InvalidInput(
                "seed roles and budgets are outside supported bounds".into(),
            ));
        }
        let selection = self.indexes.roles.select(
            &self.graph,
            &[&request.from_role, &request.to_role],
            request.max_seed_assertions,
            request.include_hypotheses,
        );
        let mut report = SeededPathReport {
            prune_selection: admission.prune_selection.clone(),
            pruned_edges: 0, evidence_filtered_edges: 0,
            method_id: "entmoot.seeded-call-routes.v2".into(),
            snapshot: self.graph.snapshot.clone().into(),
            matching_assertions: selection.matching,
            excluded_hypotheses: selection.excluded_hypotheses,
            seeds_truncated: selection.truncated,
            request: request.clone(),
            anchors: selection.anchors,
            unmapped_assertion_ids: selection.unmapped,
            colocated: vec![], routes: vec![],
            discovery_expansions: 0, discovery_edges: 0, discovery_truncated: false,
            limitations: vec![
                "Provider roles are location claims; call reachability does not establish dataflow, argument transfer, execution order, sanitizer efficacy or exploitability.".into(),
                "No returned route means no witness found in the represented directed graph under these budgets; implicit, unresolved and runtime edges remain unknown.".into(),
                "Discovery may join incompatible scope candidates; each hydrated path independently checks scope compatibility. Same-callable seeds are co-location only.".into(),
            ],
        };
        let origins: BTreeSet<_> = report
            .anchors
            .iter()
            .filter(|a| a.role == request.from_role)
            .map(|a| a.callable_id.clone())
            .collect();
        let targets: BTreeSet<_> = report
            .anchors
            .iter()
            .filter(|a| a.role == request.to_role)
            .map(|a| a.callable_id.clone())
            .collect();
        report.colocated = origins.intersection(&targets).cloned().collect();
        let mut target_keys: HashMap<String, BTreeSet<String>> = HashMap::new();
        for target in &targets {
            for key in self.walk_endpoint_keys(target) {
                target_keys.entry(key).or_default().insert(target.clone());
            }
        }
        let mut pairs = BTreeSet::new();
        'origins: for origin in origins {
            if targets.is_empty() {
                break;
            }
            let keys: Vec<_> = self
                .walk_endpoint_keys(&origin)
                .into_iter()
                .filter(|key| !admission.excludes_address(key))
                .collect();
            let mut seen: HashSet<_> = keys.iter().cloned().collect();
            let mut queue: VecDeque<_> = keys.into_iter().map(|k| (k, 0usize)).collect();
            while let Some((key, depth)) = queue.pop_front() {
                if let Some(found) = target_keys.get(&key) {
                    for target in found {
                        if target == &origin {
                            continue;
                        }
                        if pairs.len() >= request.max_pairs
                            && !pairs.contains(&(origin.clone(), target.clone()))
                        {
                            report.discovery_truncated = true;
                            break 'origins;
                        }
                        pairs.insert((origin.clone(), target.clone()));
                    }
                }
                if depth >= request.walk.max_hops {
                    continue;
                }
                if report.discovery_expansions >= request.max_discovery_expansions {
                    report.discovery_truncated = true;
                    break 'origins;
                }
                report.discovery_expansions += 1;
                for index in self
                    .indexes
                    .relations
                    .postings(&key, true)
                    .into_iter()
                    .flatten()
                {
                    if report.discovery_edges >= request.max_discovery_edges {
                        report.discovery_truncated = true;
                        break 'origins;
                    }
                    report.discovery_edges += 1;
                    let edge = &self.graph.relation_edges[*index];
                    if edge.predicate != RelationKind::Calls {
                        continue;
                    }
                    if !admission.allows(*index, edge, Some(&repository))? {
                        continue;
                    }
                    let next = address_identity_key(&edge.object);
                    if seen.insert(next.clone()) {
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }
        report.pruned_edges = admission.pruned_edges;
        report.evidence_filtered_edges = admission.evidence_filtered_edges;
        for (from, to) in pairs {
            let query = self.traverse(TraverseQuery::Path {
                evidence_filter: request.evidence_filter,
                prune: request.prune.clone(),
                resumable: false,
                continuation: None,
                start: from.clone(),
                target: to.clone(),
                predicates: vec!["calls".into()],
                budget: request.walk,
            })?;
            report.routes.push(SeededRoute {
                from_callable: from,
                to_callable: to,
                query,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{KnowledgeRepository, SqliteKnowledgeRepository};
    use entmoot_contracts::query::SeededPathRequest;

    #[test]
    fn seeded_routes_follow_calls_and_do_not_turn_colocation_into_flow() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("seed.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&crate::KnowledgeStore::default())
            .unwrap();
        let mut graph = TopologyGraph::new("seed-test");
        for id in ["a", "b", "c"] {
            graph.upsert_node(TopologyNode::shell(id, NodeKind::Callable, id, None));
        }
        for (from, to) in [("a", "b"), ("b", "c")] {
            graph
                .relation_edges
                .push(entmoot_contracts::relation_edge_with_id(
                    "calls",
                    entmoot_contracts::topology_node_address(from),
                    RelationKind::Calls,
                    entmoot_contracts::topology_node_address(to),
                    entmoot_contracts::Scope {
                        domain: "seed-test".into(),
                        snapshot_revision: None,
                        snapshot_digest: None,
                        occurrence_id: None,
                        context_id: None,
                    },
                    None,
                ));
        }
        for (id, role, node, freshness) in [
            ("source-a", "source", "a", "current"),
            ("sink-c", "sink", "c", "current"),
            ("same-a", "sink", "a", "current"),
            ("stale-b", "sink", "b", "stale_body"),
        ] {
            graph.assertions.push(serde_json::from_value(json!({"assertion_id":id,"subject":{"entity_kind":"opaque","entity_id":id},"predicate":"entmoot.provider_role","object":{"value":role},"epistemic_status":"observed"})).unwrap());
            graph.attachments.push(serde_json::from_value(json!({"attachment_id":format!("attach-{id}"),"artifact_id":"producer","location_id":id,"disposition":"attached","freshness":freshness,"selected_target":{"kind":"callable","node_id":node}})).unwrap());
        }
        let engine = QueryEngine::new(graph, store);
        let result = engine.trace_seeds(SeededPathRequest::default()).unwrap();
        assert_eq!(result.routes.len(), 1);
        assert_eq!(result.colocated.len(), 1);
        assert_eq!(result.unmapped_assertion_ids, vec!["stale-b"]);
        let route = serde_json::to_value(&result.routes[0].query).unwrap();
        assert_eq!(route["paths"][0]["hop_count"], 2);
        assert_eq!(route["paths"][0]["classification"], "candidate");
        let pruned = engine
            .trace_seeds(SeededPathRequest {
                prune: entmoot_contracts::query::TraversalPrune {
                    subjects: vec!["b".into()],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        assert!(pruned.routes.is_empty());
        assert_eq!(pruned.pruned_edges, 1);
        let resolved = engine
            .trace_seeds(SeededPathRequest {
                evidence_filter: entmoot_contracts::query::HopEvidenceFilter::ResolvedOnly,
                ..Default::default()
            })
            .unwrap();
        assert!(resolved.routes.is_empty());
        assert_eq!(resolved.evidence_filtered_edges, 1);
        let reverse = engine
            .trace_seeds(SeededPathRequest {
                from_role: "sink".into(),
                to_role: "source".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(reverse.routes.is_empty());
        let limited = engine
            .trace_seeds(SeededPathRequest {
                max_discovery_edges: 1,
                ..Default::default()
            })
            .unwrap();
        assert!(limited.discovery_truncated);
        assert!(limited.routes.is_empty());
        let capped = engine
            .trace_seeds(SeededPathRequest {
                max_seed_assertions: 1,
                ..Default::default()
            })
            .unwrap();
        assert!(capped.seeds_truncated);
        assert_eq!(capped.matching_assertions, 4);
        let mut ambiguous = engine.graph().unwrap().clone();
        let mut alternative = ambiguous.attachments[0].clone();
        alternative.attachment_id = "alternative-owner".into();
        alternative.selected_target.as_mut().unwrap().node_id = Some("b".into());
        ambiguous.attachments.push(alternative);
        let result = QueryEngine::new(ambiguous, directory.path().join("seed.sqlite3"))
            .trace_seeds(SeededPathRequest::default())
            .unwrap();
        assert!(result
            .unmapped_assertion_ids
            .contains(&"source-a".to_owned()));
        assert!(result.routes.is_empty());
    }
}
