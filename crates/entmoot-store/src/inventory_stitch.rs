//! File-local inventory construction and cross-file call stitching. Parts are
//! private producer checkpoints; only their complete graph can be published.
use crate::{graph::StructuralEdge, OperationError, TopologyGraph, TopologyNode};
use entmoot_contracts::FileIndexRecord;
use entmoot_core::scan::ScanOutcome;
use std::path::Path;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SyntaxPart {
    nodes: Vec<TopologyNode>,
    edges: Vec<StructuralEdge>,
    policies: Vec<String>,
}
impl SyntaxPart {
    pub fn build(root: &Path, outcome: &ScanOutcome) -> Result<Self, OperationError> {
        Self::build_record(
            root,
            &outcome.input.path,
            &outcome.input.content_digest,
            outcome.record.as_ref(),
        )
    }
    pub fn build_record(
        root: &Path,
        path: &str,
        digest: &str,
        record: Option<&FileIndexRecord>,
    ) -> Result<Self, OperationError> {
        let mut extractor = crate::extractor::MarkerTopologyExtractor::new(root.to_string_lossy());
        // The complete graph owns its repository node. File parts must not
        // retain a host-root label that prevents sharing across worktrees.
        if let Some(record) = record {
            let relative = crate::validated_graph_relative_path(path)
                .map_err(|e| OperationError::Invalid(e.to_string()))?;
            let bytes = std::fs::read(root.join(relative))
                .map_err(|e| OperationError::Unavailable(e.to_string()))?;
            if entmoot_contracts::blake3_hex(&bytes) != digest {
                return Err(OperationError::Conflict(
                    "inventory syntax source changed".into(),
                ));
            }
            // Keep the ordinary extractor's UTF-8 display-source convention.
            let source = std::str::from_utf8(&bytes).ok();
            extractor.on_file(path, record, source);
            extractor.on_file_end(&crate::graph::file_node_id(path));
        }
        let (graph, _) = extractor.finish_syntax();
        Ok(Self {
            nodes: graph.nodes.into_inner(),
            edges: graph.edges.into_inner(),
            policies: graph.policy_ids,
        })
    }
    pub fn apply(self, graph: &mut TopologyGraph) {
        for policy in self.policies {
            graph.record_policy_id(&policy);
        }
        for node in self.nodes {
            graph.upsert_node(node);
        }
        for edge in self.edges {
            graph.try_add_edge(edge);
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CallPart(crate::resolve_edges::CallSummary);
impl CallPart {
    pub fn build(index: &crate::resolve_edges::StructuralCallIndex, outcome: &ScanOutcome) -> Self {
        Self::build_record(index, &outcome.input.path, outcome.record.as_ref())
    }
    pub fn build_record(
        index: &crate::resolve_edges::StructuralCallIndex,
        path: &str,
        record: Option<&FileIndexRecord>,
    ) -> Self {
        Self(crate::resolve_edges::CallSummary::build(
            index, path, record,
        ))
    }
    pub fn matches(&self, index: &crate::resolve_edges::StructuralCallIndex) -> bool {
        self.0.matches(index)
    }
    pub fn apply(&self, graph: &mut TopologyGraph) {
        self.0.apply(graph);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Syntax,
    Calls,
}
impl Phase {
    pub fn key(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::Calls => "calls",
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Part {
    Syntax(SyntaxPart),
    Calls(CallPart),
}
impl Part {
    pub fn phase(&self) -> Phase {
        match self {
            Self::Syntax(_) => Phase::Syntax,
            Self::Calls(_) => Phase::Calls,
        }
    }
}

pub struct Frontier {
    pub phase: Phase,
    pub start: usize,
    pub outcomes: Vec<ScanOutcome>,
}
