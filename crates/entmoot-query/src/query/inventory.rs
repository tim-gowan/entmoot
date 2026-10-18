//! Paginated inventory subject listing.

use entmoot_contracts::query::{
    InventoryReport, InventoryRequest, QueryGapCode, ResultBounds, SubjectCandidate,
    MAX_INVENTORY_LIMIT,
};

use super::{QueryEngine, QueryError};
use crate::NodeKind;

impl QueryEngine {
    /// Page materialized inventory subjects for review.
    pub fn inventory(&self, request: InventoryRequest) -> Result<InventoryReport, QueryError> {
        let compiled = request
            .selector
            .as_ref()
            .map(super::selection::CompiledSelector::new)
            .transpose()?;
        let mut selection = entmoot_contracts::query::SelectionReceipt::default();
        let mut explanations = std::collections::HashMap::new();
        let limit = request.limit.clamp(1, MAX_INVENTORY_LIMIT);
        let offset = request.offset;
        let mut kinds = request.kinds.clone();
        if kinds.is_empty() {
            kinds = request
                .selector
                .as_ref()
                .filter(|s| !s.kinds.is_empty())
                .map(|s| s.kinds.iter().map(|k| k.as_str().to_owned()).collect())
                .unwrap_or_else(|| vec!["callable".into()]);
        }
        kinds.sort();
        kinds.dedup();
        let mut unknown = Vec::new();
        let mut allowed = Vec::new();
        for kind in &kinds {
            match parse_kind(kind) {
                Some(value) => allowed.push(value),
                None => unknown.push(kind.clone()),
            }
        }
        let path_prefix = request
            .path
            .as_deref()
            .map(entmoot_contracts::normalize_repo_path)
            .transpose()
            .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
        let path = path_prefix
            .as_ref()
            .map(|p| {
                super::selection::CompiledPath::new(
                    &entmoot_contracts::query::PathSelector::Subtree(p.clone()),
                )
            })
            .transpose()?;

        let applied = InventoryRequest {
            selector: request.selector,
            kinds: kinds.clone(),
            path: path_prefix,
            offset,
            limit,
        };
        let mut report = InventoryReport::new(self.graph.snapshot.clone(), applied);
        if !unknown.is_empty() {
            report.gaps.push(QueryGapCode::NoInventorySubjectMatched);
            report
                .limitations
                .push(format!("unknown_subject_kinds:{}", unknown.join(",")));
        }
        if allowed.is_empty() {
            report.bounds = ResultBounds::window(offset, limit, 0, 0);
            return Ok(report);
        }

        let mut matched: Vec<&crate::TopologyNode> = self
            .graph
            .nodes
            .iter()
            .filter(|node| allowed.contains(&node.kind))
            .filter(|node| {
                path.as_ref()
                    .is_none_or(|p| node.path.as_deref().is_some_and(|value| p.matches(value)))
            })
            .filter(|node| {
                let Some(compiled) = &compiled else {
                    return true;
                };
                selection.examined += 1;
                let (state, explanation) = compiled.matches(node);
                match state {
                    super::selection::MatchState::Yes => {
                        selection.matched += 1;
                        explanations.insert(node.id.as_str(), explanation);
                        true
                    }
                    super::selection::MatchState::Unknown => {
                        selection.unknown += 1;
                        false
                    }
                    super::selection::MatchState::No => false,
                }
            })
            .collect();
        matched.sort_by(|left, right| {
            left.path
                .as_deref()
                .unwrap_or("")
                .cmp(right.path.as_deref().unwrap_or(""))
                .then(left.id.cmp(&right.id))
        });
        let total = matched.len();
        let page = matched
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|node| {
                let mut card = subject_card(node, 0);
                card.selection_match = explanations.remove(node.id.as_str());
                card
            })
            .collect::<Vec<_>>();
        let returned = page.len();
        report.items = page;
        report.selection = compiled.as_ref().map(|_| selection);
        report.bounds = ResultBounds::window(offset, limit, returned, total);
        if allowed.contains(&NodeKind::Declaration) {
            report
                .gaps
                .push(QueryGapCode::DeclarationInventoryIsPartial);
        }
        if total == 0 {
            report.gaps.push(QueryGapCode::NoInventorySubjectMatched);
        }
        Ok(report)
    }
}

fn parse_kind(value: &str) -> Option<NodeKind> {
    serde_json::from_value(serde_json::Value::String(value.trim().to_owned())).ok()
}

pub(super) fn subject_card(node: &crate::TopologyNode, match_score: u8) -> SubjectCandidate {
    SubjectCandidate {
        selection_match: None,
        node_id: node.id.clone(),
        kind: node.kind.as_str().into(),
        label: node.label.clone(),
        path: node.path.clone(),
        start_line: node.start_line,
        display_ref: node.display_ref.clone(),
        fqn: node.fqn.clone(),
        address_id: node.address_id.clone(),
        logic_address: node.logic_address.clone(),
        structural_hash: node.structural_hash.clone(),
        match_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository, TopologyGraph, TopologyNode,
    };
    use entmoot_contracts::query::InventoryRequest;
    use entmoot_contracts::SourceState;

    fn empty_store(path: &std::path::Path) {
        let mut repo = SqliteKnowledgeRepository::open(path).unwrap();
        repo.replace_all(&KnowledgeStore::default()).unwrap();
    }

    #[test]
    fn pages_callables_by_path_and_offset() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("inventory.sqlite3");
        empty_store(&store);
        let mut graph = TopologyGraph::new("fixtures/inventory");
        graph.snapshot.source_state = SourceState::Committed;
        for (id, path) in [("c1", "a.rs"), ("c2", "b.rs")] {
            let mut node = TopologyNode::shell(id, NodeKind::Callable, id, None);
            node.path = Some(path.into());
            graph.upsert_node(node);
        }
        graph.upsert_node(TopologyNode::shell("f1", NodeKind::File, "a.rs", None));
        let engine = QueryEngine::new(graph, store);
        let report = engine
            .inventory(InventoryRequest {
                selector: None,
                kinds: vec!["callable".into()],
                path: Some("a.rs".into()),
                offset: 0,
                limit: 10,
            })
            .unwrap();
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].node_id, "c1");
        assert_eq!(report.bounds.total_matches, 1);

        let page = engine
            .inventory(InventoryRequest {
                selector: None,
                kinds: vec!["callable".into()],
                path: None,
                offset: 1,
                limit: 1,
            })
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].node_id, "c2");
        assert_eq!(page.bounds.total_matches, 2);
        assert_eq!(page.bounds.offset, 1);
    }
}
