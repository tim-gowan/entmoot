//! AST normalization utilities for structural hashing.

use std::collections::HashSet;

use crate::analysis::context::UnsealedMarkers;
use tree_sitter::Tree;

use crate::callable_context::CallableContext;
use entmoot_contracts::StructuralHashPolicy;

/// Maximum recursive depth for language visitors.
///
/// Keep this below ordinary worker-thread stack exhaustion so adversarial or
/// generated syntax becomes a per-file `HashError` instead of aborting the
/// repository scan. Deep generic serialization has a separate iterative path.
pub const MAX_RECURSIVE_DEPTH: usize = 512;

/// Syntax facts from one language visitor. Identity is derived from the same CST.
pub struct VisitorOutput {
    pub callables: Vec<CallableContext>,
    pub imports: HashSet<String>,
    pub markers: UnsealedMarkers,
}

pub type VisitorResult = Result<VisitorOutput, Box<dyn std::error::Error>>;

/// Language visitor over the original parsed tree and source bytes.
///
/// Implementations emit inventory and syntax observations; they do not mutate
/// or clone the parse tree. StructuralHasher owns canonical identity policy.
pub trait ASTNormalizer: Send + Sync {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult;
}
