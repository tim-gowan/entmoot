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
        let limit = request.limit.clamp(1, MAX_INVENTORY_LIMIT);
        let offset = request.offset;
        let mut kinds = request.kinds.clone();
        if kinds.is_empty() {
            kinds.push("callable".into());
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
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_start_matches('/').to_owned());

        let applied = InventoryRequest {
            kinds: kinds.clone(),
            path: path_prefix.clone(),
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
            .filter(|node| match (&path_prefix, &node.path) {
                (None, _) => true,
                (Some(prefix), Some(path)) => {
                    path == prefix || path.starts_with(&format!("{prefix}/"))
                }
                (Some(_), None) => false,
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
            .map(|node| subject_card(node, 0))
            .collect::<Vec<_>>();
        let returned = page.len();
        report.items = page;
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
    match value.trim() {
        "repo" => Some(NodeKind::Repo),
        "file" => Some(NodeKind::File),
        "module_stub" => Some(NodeKind::ModuleStub),
        "callable" => Some(NodeKind::Callable),
        "declaration" => Some(NodeKind::Declaration),
        "call_stub" => Some(NodeKind::CallStub),
        "control_site" => Some(NodeKind::ControlSite),
        "resource" => Some(NodeKind::Resource),
        _ => None,
    }
}

fn subject_card(node: &crate::TopologyNode, match_score: u8) -> SubjectCandidate {
    SubjectCandidate {
        node_id: node.id.clone(),
        kind: kind_name(node.kind).into(),
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

fn kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Repo => "repo",
        NodeKind::File => "file",
        NodeKind::ModuleStub => "module_stub",
        NodeKind::Callable => "callable",
        NodeKind::Declaration => "declaration",
        NodeKind::CallStub => "call_stub",
        NodeKind::ControlSite => "control_site",
        NodeKind::Resource => "resource",
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
