//! Unified AST walk analysis: markers and scope preflight.

pub mod bound_callable;
pub mod context;
pub mod listeners;
pub mod scope;

pub use bound_callable::{
    bound_name_from_ancestors, bound_site_from_ancestors, first_identifier_child, python_simple_assignment_lhs,
    python_simple_assignment_lhs_site, BinderSpec, BoundName, JS_VARIABLE_DECLARATOR, PYTHON_ASSIGNMENT,
};
pub use context::FileAnalysisContext;
pub use listeners::topology::{
    binding_marker_local, binding_marker_parameter, record_call_site, record_callable_begin, record_control_site,
    record_declaration, record_import,
};
pub use scope::preflight_callable;

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::ScopeAnalyzer;

/// Callable scope frame stack used by [`crate::visitor::BaseVisitor`].
#[derive(Default)]
pub struct CallableScopeStack {
    allowed_locals_stack: Vec<HashSet<String>>,
}

impl CallableScopeStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn preflight(&mut self, analyzer: &dyn ScopeAnalyzer, node: &Node, source: &[u8]) {
        self.allowed_locals_stack
            .push(scope::preflight_callable(analyzer, node, source));
    }

    pub fn exit_callable(&mut self) {
        let _ = self.allowed_locals_stack.pop();
    }

    pub fn is_allowed_local(&self, name: &str) -> bool {
        self.allowed_locals_stack
            .iter()
            .rev()
            .any(|allowed| allowed.contains(name))
    }
}
