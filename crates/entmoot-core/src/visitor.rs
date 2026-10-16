//! AST visitor pattern for tree traversal and serialization state.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::analysis::{binding_marker_local, binding_marker_parameter};
use crate::analysis::{
    record_call_site, record_callable_begin, record_control_site, record_declaration, record_import,
    CallableScopeStack, FileAnalysisContext,
};
use crate::callable_context::CallableContext;
use crate::scope_analyzer::ScopeAnalyzer;
use crate::serialization::TokenStream;
use entmoot_contracts::StructuralHashPolicy;

/// State maintained during AST traversal (extractable for copying).
#[derive(Debug, Clone, Default)]
pub struct VisitorState {
    pub scope_stack: Vec<String>,
    pub var_maps: Vec<HashMap<String, String>>,
    pub arg_maps: Vec<HashMap<String, String>>,
    pub var_counter: usize,
    pub arg_counter: usize,
    pub depth: usize,
    pub in_parameter_list: bool,
}

impl VisitorState {
    pub fn enter_scope(&mut self, scope_type: impl Into<String>, _policy: &StructuralHashPolicy) {
        self.scope_stack.push(scope_type.into());
        // Always track scopes for structural binding markers; alpha-rename only affects
        // the normalized spelling written into those markers / IR.
        self.var_maps.push(HashMap::new());
        self.arg_maps.push(HashMap::new());
        self.var_counter = 0;
        self.arg_counter = 0;
    }

    pub fn exit_scope(&mut self) {
        if !self.scope_stack.is_empty() {
            self.scope_stack.pop();
        }
        if !self.var_maps.is_empty() {
            self.var_maps.pop();
        }
        if !self.arg_maps.is_empty() {
            self.arg_maps.pop();
        }
        self.var_counter = self.var_maps.last().map(|m| m.len()).unwrap_or(0);
        self.arg_counter = self.arg_maps.last().map(|m| m.len()).unwrap_or(0);
    }

    pub fn current_var_map(&self) -> &HashMap<String, String> {
        static EMPTY: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
        self.var_maps.last().unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
    }

    pub fn current_var_map_mut(&mut self) -> &mut HashMap<String, String> {
        if self.var_maps.is_empty() {
            self.var_maps.push(HashMap::new());
        }
        let idx = self.var_maps.len() - 1;
        &mut self.var_maps[idx]
    }

    pub fn current_arg_map(&self) -> &HashMap<String, String> {
        static EMPTY: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
        self.arg_maps.last().unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
    }

    pub fn current_arg_map_mut(&mut self) -> &mut HashMap<String, String> {
        if self.arg_maps.is_empty() {
            self.arg_maps.push(HashMap::new());
        }
        let idx = self.arg_maps.len() - 1;
        &mut self.arg_maps[idx]
    }
}

/// Base visitor holding scope, alpha-rename, granular hashing, and analysis state.
pub struct BaseVisitor {
    pub state: VisitorState,
    pub policy: StructuralHashPolicy,
    max_depth: usize,
    pub current_callable: Option<CallableContext>,
    pub callable_stack: Vec<CallableContext>,
    pub callables: Vec<CallableContext>,
    pub file_imports: HashSet<String>,
    /// Declared container names (class/type/module) for logic addresses.
    pub definition_scope: Vec<String>,
    /// Nesting depth of type-like bodies (class/impl/trait/interface/…).
    /// Used for `function` → `method` kind; not incremented for `mod`/namespaces.
    pub type_body_depth: usize,
    pub analysis: FileAnalysisContext,
    callable_scope: CallableScopeStack,
}

impl BaseVisitor {
    pub fn new(max_depth: Option<usize>, policy: Option<StructuralHashPolicy>) -> Self {
        Self::with_options(max_depth, policy, true)
    }

    /// `collect_markers`: when false, hash-only path (markers discarded by hasher).
    pub fn with_options(max_depth: Option<usize>, policy: Option<StructuralHashPolicy>, collect_markers: bool) -> Self {
        Self {
            state: VisitorState::default(),
            policy: policy.unwrap_or_else(StructuralHashPolicy::v1),
            max_depth: max_depth.unwrap_or(1000),
            current_callable: None,
            callable_stack: Vec::new(),
            callables: Vec::new(),
            file_imports: HashSet::new(),
            definition_scope: Vec::new(),
            type_body_depth: 0,
            analysis: FileAnalysisContext::new(collect_markers),
            callable_scope: CallableScopeStack::new(),
        }
    }

    /// Extract granular outputs after serialization (avoids partial-move borrow issues).
    pub fn take_granular_outputs(
        &mut self,
    ) -> (
        Vec<CallableContext>,
        HashSet<String>,
        crate::analysis::context::UnsealedMarkers,
    ) {
        (
            std::mem::take(&mut self.callables),
            std::mem::take(&mut self.file_imports),
            self.analysis.take_unsealed_markers(),
        )
    }

    pub fn preflight_callable_scope(&mut self, analyzer: &dyn ScopeAnalyzer, node: &Node, source: &[u8]) {
        self.callable_scope.preflight(analyzer, node, source);
    }

    pub fn exit_callable_scope(&mut self) {
        self.callable_scope.exit_callable();
    }

    pub fn is_allowed_local(&self, name: &str) -> bool {
        self.callable_scope.is_allowed_local(name)
    }

    pub fn push_definition_scope(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !name.is_empty() {
            self.definition_scope.push(name);
        }
    }

    pub fn pop_definition_scope(&mut self) {
        let _ = self.definition_scope.pop();
    }

    /// Enter a type-like body (class / impl / trait / interface / struct / enum).
    pub fn push_type_body(&mut self) {
        self.type_body_depth += 1;
    }

    /// Leave a type-like body.
    pub fn pop_type_body(&mut self) {
        self.type_body_depth = self.type_body_depth.saturating_sub(1);
    }

    pub fn set_max_depth(&mut self, max_depth: usize) {
        self.max_depth = max_depth;
    }

    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub fn enter_scope(&mut self, scope_type: impl Into<String>) {
        let policy = self.policy.clone();
        self.state.enter_scope(scope_type, &policy);
    }

    pub fn exit_scope(&mut self) {
        self.state.exit_scope();
    }

    pub fn register_variable_if_new(&mut self, identifier_text: &str) -> String {
        self.register_variable_if_new_at(identifier_text, None)
    }

    pub fn register_variable_if_new_at(&mut self, identifier_text: &str, start_byte: Option<usize>) -> String {
        if let Some(map) = self.state.var_maps.last_mut() {
            let is_new = !map.contains_key(identifier_text);
            let normalized = {
                if is_new {
                    map.insert(identifier_text.to_string(), identifier_text.to_string());
                }
                identifier_text.to_string()
            };
            if is_new {
                crate::analysis::listeners::topology::record_binding(
                    &mut self.analysis,
                    binding_marker_local(identifier_text, &normalized, start_byte),
                );
            }
            return normalized;
        }

        identifier_text.to_string()
    }

    pub fn normalize_parameter_name(&mut self, identifier_text: &str) -> String {
        self.normalize_parameter_name_at(identifier_text, None)
    }

    pub fn normalize_parameter_name_at(&mut self, identifier_text: &str, start_byte: Option<usize>) -> String {
        if self.state.arg_maps.is_empty() {
            return identifier_text.to_string();
        }

        let idx = self.state.arg_maps.len() - 1;
        let map = &mut self.state.arg_maps[idx];
        let is_new = !map.contains_key(identifier_text);
        let normalized = {
            if is_new {
                map.insert(identifier_text.to_string(), identifier_text.to_string());
            }
            identifier_text.to_string()
        };
        if is_new {
            crate::analysis::listeners::topology::record_binding(
                &mut self.analysis,
                binding_marker_parameter(identifier_text, &normalized, start_byte),
            );
        }
        normalized
    }

    /// Push current callable (if any) and start a new one.
    pub fn begin_callable(
        &mut self,
        name: impl Into<String>,
        node_type: impl Into<String>,
        start_byte: usize,
        end_byte: usize,
    ) {
        if let Some(current) = self.current_callable.take() {
            self.callable_stack.push(current);
        }
        // Nested defs under a method: stack non-empty → not a method.
        let inside_type = self.type_body_depth > 0 && self.callable_stack.is_empty();
        self.current_callable = Some(CallableContext::with_scope(
            name,
            node_type,
            self.definition_scope.clone(),
            inside_type,
            start_byte,
            end_byte,
        ));
        if let Some(callable) = self.current_callable.as_ref() {
            record_callable_begin(&mut self.analysis, callable);
        }
    }

    /// Record a syntax-owned declaration, optionally linked to a callable body
    /// whose start byte is already known from this same AST walk.
    pub fn record_declaration(
        &mut self,
        name: impl Into<String>,
        kind: impl Into<String>,
        start_byte: usize,
        end_byte: usize,
        callable_start_byte: Option<usize>,
    ) {
        record_declaration(
            &mut self.analysis,
            name,
            kind,
            self.definition_scope.clone(),
            start_byte,
            end_byte,
            callable_start_byte,
        );
    }

    /// Record a named syntax declaration without manufacturing a callable.
    /// Native producers may attach to these syntax-owned declarations without
    /// inventing callable bodies or changing structural hashes.
    pub fn record_named_declaration(&mut self, node: &tree_sitter::Node<'_>, source: &[u8]) {
        let kind = match node.kind() {
            "class_declaration" | "record_declaration" | "class_specifier" => "class",
            "struct_item" | "struct_declaration" | "struct_specifier" | "union_item" | "union_specifier" => "struct",
            "enum_item" | "enum_declaration" | "enum_specifier" => "enum",
            "interface_declaration" => "interface",
            "trait_item" => "trait",
            "type_item" | "type_alias" | "type_alias_declaration" | "alias_declaration" | "type_definition" => {
                "type_alias"
            }
            "macro_definition" => "macro",
            "function_signature_item" => "method",
            "type_spec" => match node.child_by_field_name("type").map(|n| n.kind()) {
                Some("struct_type") => "struct",
                Some("interface_type") => "interface",
                _ => "type",
            },
            _ => return,
        };
        let Some(mut name) = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("declarator"))
        else {
            return;
        };
        while let Some(inner) = name.child_by_field_name("declarator") {
            name = inner;
        }
        let Ok(name) = name.utf8_text(source) else { return };
        if !name.is_empty() {
            self.record_declaration(name, kind, node.start_byte(), node.end_byte(), None);
        }
    }

    /// True when visiting under an open callable (nested binders stay hash-only).
    pub fn is_inside_callable(&self) -> bool {
        self.current_callable.is_some() || !self.callable_stack.is_empty()
    }

    /// Finish syntax inventory for this callable and restore its enclosing scope.
    pub fn end_callable(&mut self) {
        if let Some(callable) = self.current_callable.take() {
            self.callables.push(callable);
        }
        self.current_callable = self.callable_stack.pop();
    }

    /// Record a Merkle import name (hashing). Does not emit an [`entmoot_contracts::ImportMarker`].
    pub fn track_file_import(&mut self, name: impl Into<String>) {
        self.file_imports.insert(name.into());
    }

    /// Record an analysis [`entmoot_contracts::ImportMarker`] with kind, alias, and real byte spans.
    /// Does not modify the Merkle import set — call [`Self::track_file_import`] for hashing names.
    pub fn track_import_marker(
        &mut self,
        path: impl Into<String>,
        alias: Option<String>,
        kind: entmoot_contracts::ImportKind,
        start_byte: usize,
        end_byte: usize,
    ) {
        record_import(&mut self.analysis, path, alias, kind, start_byte, end_byte);
    }

    pub fn track_callable_import_usage(&mut self, name: &str) {
        if let Some(callable) = self.current_callable.as_mut() {
            callable.imports.insert(name.to_string());
        }
    }

    /// Record a call-site marker for topology (intra-file call edges in Phase 2 POC).
    pub fn track_call_site(&mut self, callee: impl Into<String>, start_byte: usize, end_byte: usize) {
        record_call_site(
            &mut self.analysis,
            self.current_callable.as_ref(),
            callee,
            start_byte,
            end_byte,
        );
    }

    /// Record exact syntax ports from the same AST and callable stack as inventory.
    pub fn track_call_node(&mut self, callee: impl Into<String>, node: &Node, source: &[u8]) {
        if !self.analysis.collect_markers {
            return;
        }
        self.track_call_site(callee, node.start_byte(), node.end_byte());
        if let Some(call) = self.analysis.markers.call_sites.last_mut() {
            call.ports = crate::call_ports::extract(node, source);
        }
    }

    /// Record a provider-independent control site from this grammar's node kind.
    /// This is marker-only metadata and never contributes to structural hashes.
    pub fn track_control_site(&mut self, kind: &str, start_byte: usize, end_byte: usize) {
        record_control_site(&mut self.analysis, kind, start_byte, end_byte);
    }

    pub fn check_depth(&self) -> Result<(), RecursionDepthError> {
        if self.state.depth >= self.max_depth {
            Err(RecursionDepthError)
        } else {
            Ok(())
        }
    }

    pub fn increment_depth(&mut self) {
        self.state.depth += 1;
    }

    pub fn decrement_depth(&mut self) {
        if self.state.depth > 0 {
            self.state.depth -= 1;
        }
    }
}

/// Language-specific AST node serialization.
pub trait NodeVisitor {
    /// Shared source access; dialects own interpretation, not byte decoding.
    fn source_text(&self, node: &Node) -> Option<String> {
        node.utf8_text(self.source()).ok().map(str::to_owned)
    }

    fn source(&self) -> &[u8];
    fn context(&mut self) -> (&mut BaseVisitor, &[u8]);
    fn dispatch(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError>;

    fn prepare_node(&mut self, node: &Node) {
        let (base, source) = self.context();
        base.record_named_declaration(node, source);
    }

    fn retain_empty_node(&self, _kind: &str) -> bool {
        false
    }

    fn visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.context().0.check_depth()?;
        self.prepare_node(node);
        let (base, _) = self.context();
        base.increment_depth();
        base.track_control_site(node.kind(), node.start_byte(), node.end_byte());
        let result = self.dispatch(node);
        self.context().0.decrement_depth();
        result
    }

    fn visit_children(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut children = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(stream) = self.visit(&child)? {
                if stream.has_content() {
                    children.push(stream);
                }
            }
        }
        Ok(children)
    }

    fn visit_generic(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children(node)?;
        if !children.is_empty() || self.retain_empty_node(node.kind()) {
            return Ok(Some(TokenStream::node(node.kind(), children)));
        }
        Ok(Some(match node.utf8_text(self.source()) {
            Ok(text) => TokenStream::text(text),
            Err(_) => TokenStream::node(node.kind(), vec![]),
        }))
    }

    fn visit_comment_kind(&mut self, node: &Node, kind: &str) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(Some(match node.utf8_text(self.source()) {
            Ok(text) => TokenStream::literal(kind, text),
            Err(_) => TokenStream::node(kind, vec![]),
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecursionDepthError;

impl std::fmt::Display for RecursionDepthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Maximum recursion depth exceeded during serialization")
    }
}

impl std::error::Error for RecursionDepthError {}

#[cfg(test)]
mod tests {
    use super::BaseVisitor;
    use entmoot_contracts::StructuralHashPolicy;

    #[test]
    fn enter_exit_scope_pushes_maps() {
        let mut visitor = BaseVisitor::new(None, None);
        visitor.enter_scope("function");
        assert_eq!(visitor.state.scope_stack, vec!["function"]);
        assert_eq!(visitor.state.var_maps.len(), 1);
        assert_eq!(visitor.state.arg_maps.len(), 1);
        visitor.exit_scope();
        assert!(visitor.state.scope_stack.is_empty());
    }

    #[test]
    fn parameters_keep_source_names_and_deduplicate_within_scope() {
        let mut visitor = BaseVisitor::new(None, Some(StructuralHashPolicy::v1()));
        visitor.enter_scope("function");
        assert_eq!(visitor.normalize_parameter_name("foo"), "foo");
        assert_eq!(visitor.normalize_parameter_name("foo"), "foo");
        assert_eq!(visitor.normalize_parameter_name("bar"), "bar");
        assert_eq!(visitor.state.arg_maps.last().unwrap().len(), 2);
    }

    #[test]
    fn variables_keep_source_names_and_deduplicate_within_scope() {
        let mut visitor = BaseVisitor::new(None, Some(StructuralHashPolicy::v1()));
        visitor.enter_scope("function");
        assert_eq!(visitor.register_variable_if_new("value"), "value");
        assert_eq!(visitor.register_variable_if_new("value"), "value");
        assert_eq!(visitor.register_variable_if_new("next"), "next");
        assert_eq!(visitor.state.var_maps.last().unwrap().len(), 2);
    }

    #[test]
    fn preflight_and_is_allowed_local() {
        use tree_sitter::Parser;

        use crate::languages::python::scope_analyzer::PythonScopeAnalyzer;

        let source = b"def f(x):\n    y = 1\n    return x\n";
        let mut parser = Parser::new();
        parser.set_language(&crate::grammars::PYTHON.into()).unwrap();
        let tree = parser.parse(source.as_slice(), None).unwrap();
        let func = tree.root_node().named_child(0).unwrap();

        let mut visitor = BaseVisitor::new(None, None);
        visitor.preflight_callable_scope(&PythonScopeAnalyzer, &func, source.as_slice());
        assert!(visitor.is_allowed_local("y"));
        visitor.exit_callable_scope();
        assert!(!visitor.is_allowed_local("y"));
    }
}
