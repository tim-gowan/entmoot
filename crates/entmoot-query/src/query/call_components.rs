//! Weakly connected call-relation components among callables.

use std::collections::{BTreeMap, HashMap};

use entmoot_contracts::query::{
    CallComponent, CallComponentAnchorKind, CallComponentsReport, CallComponentsRequest,
    QueryGapCode, ResultBounds, DEFAULT_CALL_COMPONENTS_LIMIT, MAX_CALL_COMPONENTS_LIMIT,
};
use entmoot_contracts::RelationKind;

use super::{QueryEngine, QueryError};
use crate::NodeKind;

impl QueryEngine {
    /// Page weakly connected `calls` components among callables.
    pub fn call_components(
        &self,
        request: CallComponentsRequest,
    ) -> Result<CallComponentsReport, QueryError> {
        if self.deferred_navigation.is_some() {
            return self.navigation()?.call_components(request);
        }
        let predicate = request.predicate.trim().to_ascii_lowercase();
        if predicate != "calls" {
            return Err(QueryError::InvalidInput(format!(
                "call_components predicate must be calls; got {predicate}"
            )));
        }
        let limit = request.limit.clamp(1, MAX_CALL_COMPONENTS_LIMIT);
        let sample_limit = request
            .sample_limit
            .clamp(1, DEFAULT_CALL_COMPONENTS_LIMIT.max(64));
        let offset = request.offset;
        let applied = CallComponentsRequest {
            predicate: "calls".into(),
            offset,
            limit,
            sample_limit,
            max_nodes: request.max_nodes.clamp(1, 200_000),
            max_edges: request.max_edges.clamp(1, 1_000_000),
        };
        let mut report = CallComponentsReport::new(self.graph.snapshot.clone(), applied);
        report
            .gaps
            .push(QueryGapCode::CallGraphCompletenessNotEstablished);

        let mut callable_ids = Vec::new();
        let mut by_id: HashMap<String, &crate::TopologyNode> = HashMap::new();
        for node in &self.graph.nodes {
            if node.kind != NodeKind::Callable {
                continue;
            }
            if callable_ids.len() >= report.request.max_nodes {
                report.gaps.push(QueryGapCode::BudgetExceeded);
                report
                    .limitations
                    .push("callable_node_budget_exceeded".into());
                break;
            }
            callable_ids.push(node.id.clone());
            by_id.insert(node.id.clone(), node);
        }
        callable_ids.sort();

        let dense: HashMap<_, _> = callable_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut components = super::adjacency::DisjointSet::new(callable_ids.len());

        let mut out_degree: HashMap<String, usize> = HashMap::new();
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for (examined, edge) in self.graph.relation_edges.iter().enumerate() {
            if examined >= report.request.max_edges {
                report.gaps.push(QueryGapCode::BudgetExceeded);
                report.limitations.push("call_edge_budget_exceeded".into());
                break;
            }
            if edge.predicate != RelationKind::Calls {
                continue;
            }
            let Some(subject) = self.node_for_address(&edge.subject) else {
                continue;
            };
            let Some(object) = self.node_for_address(&edge.object) else {
                continue;
            };
            if subject.kind != NodeKind::Callable || object.kind != NodeKind::Callable {
                continue;
            }
            if !by_id.contains_key(&subject.id) || !by_id.contains_key(&object.id) {
                continue;
            }
            *out_degree.entry(subject.id.clone()).or_default() += 1;
            *in_degree.entry(object.id.clone()).or_default() += 1;
            components.union(dense[subject.id.as_str()], dense[object.id.as_str()]);
        }

        let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for id in &callable_ids {
            let root = components.root(dense[id.as_str()]);
            groups.entry(root).or_default().push(id.clone());
        }

        let mut components = Vec::new();
        for mut members in groups.into_values() {
            members.sort();
            let roots: Vec<&String> = members
                .iter()
                .filter(|id| *in_degree.get(*id).unwrap_or(&0) == 0)
                .collect();
            let (anchor, anchor_kind) = if roots.len() == 1 {
                (roots[0].clone(), CallComponentAnchorKind::Root)
            } else {
                let mut best = members[0].clone();
                let mut best_degree = 0usize;
                for id in &members {
                    let degree = in_degree.get(id).copied().unwrap_or(0)
                        + out_degree.get(id).copied().unwrap_or(0);
                    if degree > best_degree || (degree == best_degree && id < &best) {
                        best_degree = degree;
                        best = id.clone();
                    }
                }
                (best, CallComponentAnchorKind::Hub)
            };
            let path_hint = by_id.get(&anchor).and_then(|node| node.path.clone());
            let sample_members = members.iter().take(sample_limit).cloned().collect();
            components.push(CallComponent {
                anchor,
                anchor_kind,
                member_count: members.len() as u64,
                sample_members,
                path_hint,
            });
        }
        components.sort_by(|left, right| {
            right
                .member_count
                .cmp(&left.member_count)
                .then(left.anchor.cmp(&right.anchor))
        });
        let total = components.len();
        let page: Vec<_> = components.into_iter().skip(offset).take(limit).collect();
        let returned = page.len();
        report.components = page;
        report.bounds = ResultBounds::window(offset, limit, returned, total);
        report.bounds.truncated |= report.gaps.contains(&QueryGapCode::BudgetExceeded);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository, TopologyGraph, TopologyNode,
    };
    use entmoot_contracts::{
        relation_edge_with_id, topology_node_address, RelationKind, Scope, SourceState,
    };

    fn empty_store(path: &std::path::Path) {
        let mut repo = SqliteKnowledgeRepository::open(path).unwrap();
        repo.replace_all(&KnowledgeStore::default()).unwrap();
    }

    fn scope(domain: &str) -> Scope {
        Scope {
            domain: domain.into(),
            snapshot_revision: Some("rev".into()),
            snapshot_digest: None,
            occurrence_id: None,
            context_id: None,
        }
    }

    #[test]
    fn roots_and_hubs_name_disconnected_call_islands() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("components.sqlite3");
        empty_store(&store);
        let mut graph = TopologyGraph::new("fixtures/call-components");
        graph.snapshot.source_state = SourceState::Committed;
        for (id, path) in [
            ("root", "a.rs"),
            ("leaf", "a.rs"),
            ("x", "b.rs"),
            ("y", "b.rs"),
        ] {
            let mut node = TopologyNode::shell(id, NodeKind::Callable, id, None);
            node.path = Some(path.into());
            graph.upsert_node(node);
        }
        let scope = scope("fixtures/call-components");
        graph.relation_edges.push(relation_edge_with_id(
            "source",
            topology_node_address("root"),
            RelationKind::Calls,
            topology_node_address("leaf"),
            scope.clone(),
            None,
        ));
        graph.relation_edges.push(relation_edge_with_id(
            "source",
            topology_node_address("x"),
            RelationKind::Calls,
            topology_node_address("y"),
            scope.clone(),
            None,
        ));
        graph.relation_edges.push(relation_edge_with_id(
            "source",
            topology_node_address("y"),
            RelationKind::Calls,
            topology_node_address("x"),
            scope,
            None,
        ));
        let engine = QueryEngine::new(graph, store);
        let report = engine
            .call_components(CallComponentsRequest::default())
            .unwrap();
        assert_eq!(report.components.len(), 2);
        let rooted = report
            .components
            .iter()
            .find(|row| row.anchor == "root")
            .expect("root component");
        assert_eq!(rooted.anchor_kind, CallComponentAnchorKind::Root);
        assert_eq!(rooted.member_count, 2);
        let cycled = report
            .components
            .iter()
            .find(|row| row.anchor == "x" || row.anchor == "y")
            .expect("cycle component");
        assert_eq!(cycled.anchor_kind, CallComponentAnchorKind::Hub);
        assert_eq!(cycled.member_count, 2);
    }
}
