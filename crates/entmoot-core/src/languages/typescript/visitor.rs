//! TypeScript/TSX AST visitor.

use std::collections::HashMap;

use super::super::ecmascript::EcmaVisitor;
use tree_sitter::Node;

use crate::serialization::TokenStream;
use crate::visitor::{BaseVisitor, NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::normalizer::TypeScriptNormalizer;
use super::scope_analyzer::TypeScriptScopeAnalyzer;

/// Node types treated as comments.
pub const COMMENT_NODES: &[&str] = &["comment", "line_comment", "block_comment", "jsdoc"];

/// Type-related node kinds.
pub const TYPE_RELATED_NODES: &[&str] = &[
    "type_annotation",
    "predefined_type",
    "type_identifier",
    "union_type",
    "intersection_type",
    "type_parameters",
    "type_assertion",
    "as_expression",
    ":",
];

/// Function-like node kinds used for hoisting scope tracking.
pub const FUNCTION_NODES: &[&str] = &[
    "function_declaration",
    "function_expression",
    "function",
    "arrow_function",
];

/// TypeScript type names used for JSX type-assertion recovery.
const TS_TYPE_NAMES: &[&str] = &[
    "number", "string", "boolean", "any", "void", "object", "unknown", "never",
];

/// Visitor for TypeScript AST nodes with hoisting support.
pub struct TypeScriptNodeVisitor<'a> {
    pub base: BaseVisitor,
    pub source: &'a [u8],
    pub normalizer: Option<&'a TypeScriptNormalizer>,
    param_name_to_position: HashMap<String, usize>,
    param_name_to_position_stack: Vec<HashMap<String, usize>>,
}

impl<'a> TypeScriptNodeVisitor<'a> {
    pub fn new(
        policy: Option<StructuralHashPolicy>,
        source: &'a [u8],
        normalizer: Option<&'a TypeScriptNormalizer>,
    ) -> Self {
        Self {
            base: BaseVisitor::new(None, policy),
            source,
            normalizer,
            param_name_to_position: HashMap::new(),
            param_name_to_position_stack: Vec::new(),
        }
    }

    fn in_parameter_list(&self) -> bool {
        self.base.state.in_parameter_list
    }

    fn set_in_parameter_list(&mut self, value: bool) {
        self.base.state.in_parameter_list = value;
    }

    fn var_maps(&self) -> &Vec<HashMap<String, String>> {
        &self.base.state.var_maps
    }

    fn arg_maps(&self) -> &Vec<HashMap<String, String>> {
        &self.base.state.arg_maps
    }

    fn register_variable_if_new(&mut self, identifier_text: &str) -> String {
        self.base.register_variable_if_new(identifier_text)
    }

    // ------------------------------------------------------------------
    // Comments / types
    // ------------------------------------------------------------------

    fn visit_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "comment")
    }

    fn visit_line_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "line_comment")
    }

    fn visit_block_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "block_comment")
    }

    fn visit_jsdoc(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("jsdoc", text)));
        }
        Ok(Some(TokenStream::node("jsdoc", vec![])))
    }

    fn visit_type_annotation(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("type_annotation", vec![])))
        } else {
            Ok(Some(TokenStream::node("type_annotation", children_streams)))
        }
    }

    fn visit_type_arguments(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "<" | ">") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("type_arguments", vec![])))
        } else {
            Ok(Some(TokenStream::node("type_arguments", children_streams)))
        }
    }

    fn visit_type_assertion(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "<" | ">") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("type_assertion", vec![])))
        } else {
            Ok(Some(TokenStream::node("type_assertion", children_streams)))
        }
    }

    fn visit_as_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "as" {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("as_expression", vec![])))
        } else {
            Ok(Some(TokenStream::node("as_expression", children_streams)))
        }
    }

    fn visit_parenthesized_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(result) = self.check_type_assertion_in_parentheses(node)? {
            return Ok(Some(result));
        }
        self.process_regular_parenthesized_expression(node)
    }

    fn check_type_assertion_in_parentheses(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if child.kind() == "type_assertion" {
                return self.visit_type_assertion(child);
            }
            if child.kind() == "as_expression" {
                return self.visit(child);
            }
            if child.kind() == "type_arguments" {
                return self.process_type_arguments_assertion(node);
            }
        }
        Ok(None)
    }

    fn process_type_arguments_assertion(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut type_args_index = None;
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for (i, c) in children.iter().enumerate() {
            if c.kind() == "type_arguments" {
                type_args_index = Some(i);
                break;
            }
        }
        let Some(type_args_index) = type_args_index else {
            return Ok(None);
        };
        for (i, c) in children.iter().enumerate() {
            if i > type_args_index
                && !TYPE_RELATED_NODES.contains(&c.kind())
                && !matches!(c.kind(), "(" | ")" | "<" | ">")
            {
                return self.visit(c);
            }
        }
        Ok(None)
    }

    fn process_regular_parenthesized_expression(
        &mut self,
        node: &Node,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "(" | ")") {
                continue;
            }
            if TYPE_RELATED_NODES.contains(&child.kind()) {
                if let Some(child_stream) = self.visit(&child)? {
                    children_streams.push(child_stream);
                }
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("parenthesized_expression", children_streams)))
        }
    }

    fn visit_interface_declaration(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    fn visit_type_alias_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("type_alias_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("type_alias_declaration", children_streams)))
        }
    }

    fn visit_enum_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Enums emit runtime values — keep members and initializers (prefer miss over false hit).
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("enum_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("enum_declaration", children_streams)))
        }
    }

    fn visit_namespace_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Namespaces emit runtime containers — keep body (prefer miss over false hit).
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("namespace_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("namespace_declaration", children_streams)))
        }
    }

    fn visit_internal_module(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // tree-sitter uses internal_module for `namespace` / `module` declarations.
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("internal_module", vec![])))
        } else {
            Ok(Some(TokenStream::node("internal_module", children_streams)))
        }
    }

    fn visit_decorator(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Decorators are runtime wrappers — keep expression (prefer miss over false hit).
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("decorator", vec![])))
        } else {
            Ok(Some(TokenStream::node("decorator", children_streams)))
        }
    }

    fn should_skip_function_child(&self, child: &Node) -> bool {
        if matches!(child.kind(), "comment" | "line_comment" | "block_comment") {
            return false;
        }
        if child.kind() == "jsdoc" {
            return false;
        }
        if matches!(child.kind(), "type_annotation" | "type_parameters") {
            return false;
        }
        if child.kind() == "ERROR" {
            let error_text = self.source_text(child).unwrap_or_default();
            if error_text.trim().starts_with(':') {
                return true;
            }
            if child.child(0).map(|c| c.kind() == ":").unwrap_or(false) {
                return true;
            }
        }
        false
    }

    // ------------------------------------------------------------------
    // Formal parameters pipeline
    // ------------------------------------------------------------------

    fn process_error_param_in_visitor(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut found_colon = false;
        let mut cursor = param.walk();
        for error_child in param.children(&mut cursor) {
            if error_child.kind() == ":" {
                found_colon = true;
                continue;
            }
            if error_child.kind() == "identifier" && !self.arg_maps().is_empty() && !found_colon {
                let identifier_text = self.source_text(&error_child).unwrap_or_default();
                let normalized_name = self.base.normalize_parameter_name(&identifier_text);
                return Ok(Some(TokenStream::literal("parameter", normalized_name)));
            }
        }
        Ok(None)
    }

    fn process_single_parameter_in_visitor(
        &mut self,
        param: &Node,
        skip_next_identifier: bool,
    ) -> Result<(Option<TokenStream>, bool), RecursionDepthError> {
        if matches!(param.kind(), "(" | ")" | "..." | "?") {
            return Ok((None, skip_next_identifier));
        }

        if matches!(param.kind(), "type_annotation" | "type_parameters") {
            let type_stream = self.visit(param)?;
            return Ok((type_stream, skip_next_identifier));
        }

        if param.kind() == "ERROR" {
            let error_param = self.process_error_param_in_visitor(param)?;
            return Ok((error_param, true));
        }

        if skip_next_identifier && param.kind() == "identifier" {
            return Ok((None, false));
        }

        match param.kind() {
            "required_parameter" => {
                let token = self.process_required_or_optional_parameter(param, true)?;
                Ok((token, skip_next_identifier))
            }
            "optional_parameter" => {
                let token = self.process_required_or_optional_parameter(param, false)?;
                Ok((token, skip_next_identifier))
            }
            "assignment_pattern" => {
                let token = self.process_assignment_pattern(param)?;
                Ok((token, skip_next_identifier))
            }
            "rest_pattern" => {
                let token = self.process_rest_param_in_visitor(param)?;
                Ok((token, skip_next_identifier))
            }
            "identifier" => {
                let token = self.process_identifier_param_with_default(param)?;
                Ok((token, skip_next_identifier))
            }
            _ => Ok((None, skip_next_identifier)),
        }
    }

    fn process_required_or_optional_parameter(
        &mut self,
        param: &Node,
        is_required: bool,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = param.walk();
        for child in param.children(&mut cursor) {
            if child.kind() == "rest_pattern" {
                return self.process_rest_param_in_visitor(&child);
            }
        }
        let token = self.process_required_param_in_visitor(param)?;
        // Optional `?` is a calling-convention marker (arg may be omitted) — keep it.
        if !is_required {
            if let Some(inner) = token {
                return Ok(Some(TokenStream::node("optional_parameter", vec![inner])));
            }
        }
        Ok(token)
    }

    // ------------------------------------------------------------------
    // Functions / scopes
    // ------------------------------------------------------------------

    fn extract_callable_name(&self, node: &Node) -> String {
        crate::analysis::bound_callable::declared_identifier(*node, self.source)
            .unwrap_or_else(|| "<anonymous>".to_owned())
    }

    /// Static variable/property binding label, without asserting property dispatch.
    fn lhs_binding_name(&self, node: &Node) -> Option<String> {
        crate::analysis::bound_name_from_ancestors(
            *node,
            self.source,
            crate::analysis::JS_VARIABLE_DECLARATOR,
            crate::analysis::bound_callable::javascript_binding_lhs,
        )
    }

    /// Streams for callable hashing: parameters + body block only.
    fn visit_function_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let lhs = self.lhs_binding_name(node);
        let track_callable = node.kind() == "function_declaration"
            || (node.kind() == "function_expression"
                && lhs.is_some()
                && (!self.base.is_inside_callable() || lhs.as_ref().is_some_and(|name| name.contains('.'))));
        let name = if node.kind() == "function_expression" {
            lhs.unwrap_or_else(|| self.extract_callable_name(node))
        } else {
            self.extract_callable_name(node)
        };
        if track_callable {
            self.base
                .begin_callable(name, "function", node.start_byte(), node.end_byte());
        }

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);

        if !self.param_name_to_position.is_empty() || !self.param_name_to_position_stack.is_empty() {
            self.param_name_to_position_stack
                .push(self.param_name_to_position.clone());
            self.param_name_to_position = HashMap::new();
        } else {
            self.param_name_to_position = HashMap::new();
        }

        self.base
            .preflight_callable_scope(&TypeScriptScopeAnalyzer, node, self.source);
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "formal_parameters" {
                    self.set_in_parameter_list(true);
                    break;
                }
            }
        }
        let children_streams = self.process_function_children(node)?;

        self.base.exit_callable_scope();
        if let Some(prev) = self.param_name_to_position_stack.pop() {
            self.param_name_to_position = prev;
        }
        self.base.exit_scope();

        if track_callable {
            self.base.end_callable();
        }

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("function_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("function_declaration", children_streams)))
        }
    }

    fn visit_generator_function_declaration(
        &mut self,
        node: &Node,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_callable_name(node);
        self.base
            .begin_callable(name, "function", node.start_byte(), node.end_byte());

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.base
            .preflight_callable_scope(&TypeScriptScopeAnalyzer, node, self.source);
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "formal_parameters" {
                    self.set_in_parameter_list(true);
                    break;
                }
            }
        }

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if child.kind() == "*" {
                children_streams.push(TokenStream::text("*"));
                continue;
            }
            if self.should_skip_function_child(child) {
                continue;
            }
            if child.kind() == "formal_parameters" {
                self.set_in_parameter_list(true);
                let param_parts = self.process_formal_parameters_in_visitor(child)?;
                children_streams.extend(param_parts);
                self.set_in_parameter_list(false);
                continue;
            }
            if let Some(child_stream) = self.visit(child)? {
                children_streams.push(child_stream);
            }
        }

        self.base.exit_callable_scope();
        self.base.exit_scope();

        self.base.end_callable();

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("generator_function_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node(
                "generator_function_declaration",
                children_streams,
            )))
        }
    }

    fn visit_arrow_function(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let bound_name = self.lhs_binding_name(node);
        let track_bound = bound_name.is_some()
            && (!self.base.is_inside_callable() || bound_name.as_ref().is_some_and(|name| name.contains('.')));
        if track_bound {
            if let Some(name) = bound_name.clone() {
                self.base
                    .begin_callable(name, "function", node.start_byte(), node.end_byte());
            }
        }

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.base
            .preflight_callable_scope(&TypeScriptScopeAnalyzer, node, self.source);
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "formal_parameters" {
                    self.set_in_parameter_list(true);
                    break;
                }
            }
        }
        let children_streams = self.process_arrow_function_body(node)?;

        self.base.exit_callable_scope();
        self.base.exit_scope();

        if track_bound && bound_name.is_some() {
            self.base.end_callable();
        }

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("arrow_function", vec![])))
        } else {
            Ok(Some(TokenStream::node("arrow_function", children_streams)))
        }
    }

    fn visit_class_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let class_name = self.extract_callable_name(node);
        self.base.push_definition_scope(class_name);
        self.base.push_type_body();
        self.base.enter_scope("class");
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if TYPE_RELATED_NODES.contains(&child.kind()) {
                if let Some(child_stream) = self.visit(&child)? {
                    children_streams.push(child_stream);
                }
                continue;
            }
            if child.kind() == "abstract" || child.kind() == "modifiers" {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        self.base.exit_scope();
        self.base.pop_type_body();
        self.base.pop_definition_scope();
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("class_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("class_declaration", children_streams)))
        }
    }

    fn visit_abstract_class_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Abstract is cosmetic — emit as class_declaration.
        self.visit_class_declaration(node)
    }

    fn visit_method_definition(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_callable_name(node);
        self.base
            .begin_callable(name, "method", node.start_byte(), node.end_byte());

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.base
            .preflight_callable_scope(&TypeScriptScopeAnalyzer, node, self.source);
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "formal_parameters" {
                    self.set_in_parameter_list(true);
                    break;
                }
            }
        }

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if self.should_skip_function_child(child) {
                continue;
            }
            if matches!(child.kind(), "abstract" | "modifiers") {
                continue;
            }
            if child.kind() == "formal_parameters" {
                self.set_in_parameter_list(true);
                let param_parts = self.process_formal_parameters_in_visitor(child)?;
                children_streams.extend(param_parts);
                self.set_in_parameter_list(false);
                continue;
            }
            if let Some(child_stream) = self.visit(child)? {
                children_streams.push(child_stream);
            }
        }

        self.base.exit_callable_scope();
        self.base.exit_scope();

        self.base.end_callable();

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("method_definition", vec![])))
        } else {
            Ok(Some(TokenStream::node("method_definition", children_streams)))
        }
    }

    fn visit_formal_parameters(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut skip_next_identifier = false;
        let mut cursor = node.walk();
        let params: Vec<Node> = node.children(&mut cursor).collect();
        for param in &params {
            if param.kind() == "..." {
                continue;
            }
            let (_token, new_skip) = self.process_single_parameter_in_visitor(param, skip_next_identifier)?;
            skip_next_identifier = new_skip;
        }
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Literals / calls / identifiers
    // ------------------------------------------------------------------

    fn visit_string(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            Ok(Some(TokenStream::literal("string", text)))
        } else {
            Ok(None)
        }
    }

    fn process_require_arguments(&mut self, call_child: &Node, call_parts: &mut Vec<TokenStream>) {
        let mut cursor = call_child.walk();
        for arg_child in call_child.children(&mut cursor) {
            if arg_child.kind() == "string" {
                if let Some(module_name) = self.extract_module_name_from_string(&arg_child) {
                    call_parts.push(TokenStream::literal("require_module", module_name));
                }
            }
        }
    }

    fn visit_call_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut call_parts = Vec::new();
        let mut is_require = false;
        let mut normalized_callee = None;
        let mut syntactic_callee = None;

        if let Some(function) = node.child_by_field_name("function") {
            syntactic_callee = self.source_text(&function);
            is_require = syntactic_callee.as_deref() == Some("require");
            if !is_require {
                if let Some(function_stream) = self.visit(&function)? {
                    call_parts.push(TokenStream::node("callee_expression", vec![function_stream]));
                }
                normalized_callee = match function.kind() {
                    "identifier" => {
                        let name = self.source_text(&function).unwrap_or_default();
                        Some(name)
                    }
                    "member_expression" | "call_expression" | "subscript_expression" => {
                        self.normalizer.map(|normalizer| {
                            normalizer.extract_qualified_name_ts_normalized(
                                &function,
                                self.source,
                                self.var_maps(),
                                self.arg_maps(),
                            )
                        })
                    }
                    _ => self.source_text(&function),
                };
            }
        }

        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for call_child in &children {
            if call_child.kind() == "arguments" {
                if is_require {
                    self.process_require_arguments(call_child, &mut call_parts);
                } else if let Some(arg_serialized) = self.visit(call_child)? {
                    call_parts.push(arg_serialized);
                }
            }
        }

        if is_require && !call_parts.is_empty() {
            return Ok(Some(TokenStream::node("require_call", call_parts)));
        }

        if let Some(name) = normalized_callee.filter(|name| !name.is_empty()) {
            call_parts.push(TokenStream::literal("call", name));
        }
        if let Some(name) = syntactic_callee {
            self.base.track_call_node(name, node, self.source);
        }

        if call_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("call_expression", call_parts)))
        }
    }

    fn is_identifier_bound_ts(&self, identifier_text: &str) -> bool {
        self.base.is_allowed_local(identifier_text)
    }

    fn normalize_bound_identifier_ts(&mut self, identifier_text: &str) -> TokenStream {
        let normalized_name = self.register_variable_if_new(identifier_text);
        TokenStream::identifier(normalized_name)
    }

    fn visit_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if self.var_maps().is_empty() || self.in_parameter_list() {
            let result = self.visit_generic(node)?;
            if let Some(identifier_text) = self.source_text(node) {
                if self.base.file_imports.contains(&identifier_text) {
                    self.base.track_callable_import_usage(&identifier_text);
                }
            }
            return Ok(result);
        }

        let identifier_text = self.source_text(node).unwrap_or_default();
        if self.base.file_imports.contains(&identifier_text) {
            self.base.track_callable_import_usage(&identifier_text);
        }
        if !self.is_identifier_bound_ts(&identifier_text) {
            return Ok(Some(TokenStream::identifier(identifier_text)));
        }

        Ok(Some(self.normalize_bound_identifier_ts(&identifier_text)))
    }

    // ------------------------------------------------------------------
    // Control flow
    // ------------------------------------------------------------------

    fn visit_simple_named(&mut self, node: &Node, name: &str) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node(name, vec![])))
        } else {
            Ok(Some(TokenStream::node(name, children_streams)))
        }
    }

    fn visit_for_in_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_simple_named(node, "for_in_statement")
    }

    fn visit_for_of_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_simple_named(node, "for_of_statement")
    }

    fn visit_return_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "return" | ";") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("return_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("return_statement", children_streams)))
        }
    }

    fn visit_number(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(mut text) = self.source_text(node) {
            if let Some(normalizer) = self.normalizer {
                text = normalizer.normalize_integer_literal_text(&text);
            }
            Ok(Some(TokenStream::literal("number", text)))
        } else {
            Ok(None)
        }
    }

    fn visit_member_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut has_optional_chaining = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "?." {
                has_optional_chaining = true;
                break;
            }
            if let Some(text) = self.source_text(&child) {
                if text == "?." {
                    has_optional_chaining = true;
                    break;
                }
            }
        }

        // Preserve nested receiver expressions (including their arguments)
        // instead of reducing the entire chain to a qualified-name literal.
        // Simple member access keeps the established compact representation.
        let receiver_has_structure = node
            .child_by_field_name("object")
            .is_some_and(|object| !matches!(object.kind(), "identifier" | "this" | "super" | "member_expression"));

        if !receiver_has_structure {
            if let Some(normalizer) = self.normalizer {
                let qualified_name = normalizer.extract_qualified_name_ts_normalized(
                    node,
                    self.source,
                    self.var_maps(),
                    self.arg_maps(),
                );
                if !qualified_name.is_empty() {
                    if has_optional_chaining {
                        return Ok(Some(TokenStream::literal("optional_member_expression", qualified_name)));
                    }
                    return Ok(Some(TokenStream::literal("member_expression", qualified_name)));
                }
            }
        }

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "?." {
                children_streams.push(TokenStream::literal("optional_chain", "?."));
            } else if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("member_expression", vec![])))
        } else {
            Ok(Some(TokenStream::node("member_expression", children_streams)))
        }
    }

    // ------------------------------------------------------------------
    // Variables / imports / JSX
    // ------------------------------------------------------------------

    fn is_type_only_import(&self, node: &Node) -> bool {
        let mut cursor = node.walk();
        for import_child in node.children(&mut cursor) {
            // tree-sitter-typescript emits a dedicated `type` keyword node
            if import_child.kind() == "type" {
                return true;
            }
            if import_child.kind() == "identifier" {
                if let Some(text) = self.source_text(&import_child) {
                    if text == "type" {
                        return true;
                    }
                }
            }
            if import_child.kind() == "ERROR" {
                let error_text = self.source_text(&import_child).unwrap_or_default();
                if error_text.to_lowercase().contains("type") {
                    return true;
                }
            }
        }
        false
    }

    fn process_import_clause(
        &mut self,
        import_child: &Node,
        import_parts: &mut Vec<TokenStream>,
        module_path: Option<&str>,
        import_stmt: &Node,
    ) {
        let mut cursor = import_child.walk();
        for clause_child in import_child.children(&mut cursor) {
            match clause_child.kind() {
                "identifier" => {
                    if let Some(import_name) = self.source_text(&clause_child) {
                        if !import_name.is_empty() {
                            import_parts.push(TokenStream::literal("import_default", import_name.clone()));
                            self.base.track_file_import(import_name.clone());
                            if let Some(path) = module_path {
                                self.base.track_import_marker(
                                    path.to_string(),
                                    Some(import_name),
                                    entmoot_contracts::ImportKind::Module,
                                    import_stmt.start_byte(),
                                    import_stmt.end_byte(),
                                );
                            }
                        }
                    }
                }
                "namespace_import" => {
                    let mut ns_alias = None;
                    let mut nc = clause_child.walk();
                    for ns_child in clause_child.children(&mut nc) {
                        if ns_child.kind() == "identifier" {
                            if let Some(ns_name) = self.source_text(&ns_child) {
                                if !ns_name.is_empty() {
                                    import_parts.push(TokenStream::literal("import_namespace", ns_name.clone()));
                                    self.base.track_file_import(ns_name.clone());
                                    ns_alias = Some(ns_name);
                                }
                            }
                        }
                    }
                    if let Some(alias) = ns_alias {
                        let path = module_path.unwrap_or("*").to_string();
                        self.base.track_import_marker(
                            path,
                            Some(alias),
                            entmoot_contracts::ImportKind::Alias,
                            clause_child.start_byte(),
                            clause_child.end_byte(),
                        );
                    }
                }
                "named_imports" => {
                    let mut nc = clause_child.walk();
                    for named_child in clause_child.children(&mut nc) {
                        if named_child.kind() == "import_specifier" {
                            // `import { type Foo }` carries no runtime binding. Recording it
                            // as a value import could turn syntactically invalid `Foo()` into
                            // a fabricated repository-local call edge.
                            if self.is_type_only_import(&named_child) {
                                continue;
                            }
                            let mut sc = named_child.walk();
                            for spec_child in named_child.children(&mut sc) {
                                if spec_child.kind() == "identifier" {
                                    if let Some(spec_name) = self.source_text(&spec_child) {
                                        if !spec_name.is_empty() {
                                            import_parts.push(TokenStream::literal("import_name", spec_name.clone()));
                                            self.base.track_file_import(spec_name);
                                        }
                                    }
                                }
                            }
                            let name = named_child
                                .child_by_field_name("name")
                                .and_then(|n| self.source_text(&n))
                                .filter(|s| !s.is_empty());
                            let alias = named_child
                                .child_by_field_name("alias")
                                .and_then(|n| self.source_text(&n))
                                .filter(|s| !s.is_empty());
                            if let Some(path_name) = name {
                                let target = module_path
                                    .map(|module| format!("{module}.{path_name}"))
                                    .unwrap_or_else(|| path_name.clone());
                                self.base.track_import_marker(
                                    target,
                                    alias,
                                    entmoot_contracts::ImportKind::Named,
                                    named_child.start_byte(),
                                    named_child.end_byte(),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn visit_jsx_element(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_simple_named(node, "jsx_element")
    }

    fn is_typescript_type_name(&self, name: &str) -> bool {
        TS_TYPE_NAMES.contains(&name)
    }

    fn extract_jsx_element_name(&self, node: &Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "jsx_namespace_name") {
                return self.source_text(&child);
            }
        }
        None
    }

    fn process_type_assertion_in_jsx(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if child.kind() == "jsx_text" {
                return self.visit(child);
            }
        }
        let mut found_type = false;
        for child in &children {
            if found_type && child.kind() == "identifier" {
                return self.visit(child);
            }
            if matches!(child.kind(), "identifier" | "jsx_namespace_name") {
                if let Some(text) = self.source_text(child) {
                    if self.is_typescript_type_name(&text) {
                        found_type = true;
                    }
                }
            }
        }
        Ok(None)
    }

    fn process_regular_jsx_opening_element(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "jsx_namespace_name") {
                if let Some(element_name) = self.source_text(&child) {
                    if !element_name.is_empty() {
                        children_streams.push(TokenStream::literal("jsx_element_name", element_name));
                    }
                }
            } else if TYPE_RELATED_NODES.contains(&child.kind()) {
                if let Some(child_stream) = self.visit(&child)? {
                    children_streams.push(child_stream);
                }
            } else if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("jsx_opening_element", vec![])))
        } else {
            Ok(Some(TokenStream::node("jsx_opening_element", children_streams)))
        }
    }

    fn visit_jsx_opening_element(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(element_name) = self.extract_jsx_element_name(node) {
            if self.is_typescript_type_name(&element_name) {
                return self.process_type_assertion_in_jsx(node);
            }
        }
        self.process_regular_jsx_opening_element(node)
    }

    fn visit_jsx_attribute(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "property_identifier" {
                if let Some(attr_name) = self.source_text(&child) {
                    if !attr_name.is_empty() {
                        children_streams.push(TokenStream::literal("jsx_attribute_name", attr_name));
                    }
                }
            } else if TYPE_RELATED_NODES.contains(&child.kind()) {
                if let Some(child_stream) = self.visit(&child)? {
                    children_streams.push(child_stream);
                }
            } else if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("jsx_attribute", vec![])))
        } else {
            Ok(Some(TokenStream::node("jsx_attribute", children_streams)))
        }
    }

    fn visit_jsx_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "{" | "}") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("jsx_expression", vec![])))
        } else {
            Ok(Some(TokenStream::node("jsx_expression", children_streams)))
        }
    }

    fn visit_jsx_text(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            Ok(Some(TokenStream::literal("jsx_text", text)))
        } else {
            Ok(None)
        }
    }

    fn visit_expression_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if self.is_export_type_declaration(node) {
            return Ok(None);
        }
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("expression_statement", children_streams)))
        }
    }

    fn is_export_type_declaration(&self, node: &Node) -> bool {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "assignment_expression" && self.check_export_type_pattern(&child) {
                return true;
            }
        }
        false
    }

    fn check_export_type_pattern(&self, assignment_node: &Node) -> bool {
        let mut has_export = false;
        let mut has_type_error = false;
        let mut cursor = assignment_node.walk();
        for assign_child in assignment_node.children(&mut cursor) {
            if assign_child.kind() == "identifier" {
                if let Some(text) = self.source_text(&assign_child) {
                    if text == "export" {
                        has_export = true;
                    }
                }
            } else if assign_child.kind() == "ERROR" && self.check_type_error(&assign_child) {
                has_type_error = true;
            }
        }
        has_export && has_type_error
    }

    fn check_type_error(&self, error_node: &Node) -> bool {
        let error_text = self.source_text(error_node).unwrap_or_default();
        if error_text.to_lowercase().contains("type") {
            let mut cursor = error_node.walk();
            for error_child in error_node.children(&mut cursor) {
                if error_child.kind() == "identifier" {
                    return true;
                }
            }
        }
        false
    }

    fn visit_export_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "type" | "type_alias_declaration" | "interface_declaration"
            ) {
                // Hash pruning must not prune the syntax-owned declaration inventory.
                self.base.record_named_declaration(&child, self.source);
                return Ok(None);
            }
        }
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("export_statement", children_streams)))
        }
    }

    // ------------------------------------------------------------------
    // ERROR nodes
    // ------------------------------------------------------------------

    fn visit_error(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let (has_return, expression_parts) = self.process_error_node_children(node)?;
        if has_return && !expression_parts.is_empty() {
            return Ok(Some(self.construct_return_statement(expression_parts)));
        } else if !expression_parts.is_empty() {
            return Ok(Some(TokenStream::node("expression_statement", expression_parts)));
        }
        Ok(None)
    }

    fn process_error_node_children(&mut self, node: &Node) -> Result<(bool, Vec<TokenStream>), RecursionDepthError> {
        let mut has_return = false;
        let mut expression_parts = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if self.check_return_keyword(child) {
                has_return = true;
                continue;
            }
            if matches!(child.kind(), "(" | ")" | "<" | ">") {
                continue;
            }
            if child.kind() == "jsx_text" {
                self.process_jsx_text_in_error(child, &mut expression_parts)?;
                continue;
            }
            if child.kind() == "jsx_opening_element" {
                self.process_jsx_opening_element_in_error(child, &mut expression_parts)?;
                continue;
            }
            if TYPE_RELATED_NODES.contains(&child.kind()) {
                if let Some(child_stream) = self.visit(child)? {
                    expression_parts.push(child_stream);
                }
                continue;
            }
            if matches!(
                child.kind(),
                "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">" | "<=" | ">="
            ) {
                if let Some(text) = self.source_text(child) {
                    expression_parts.push(TokenStream::text(text));
                }
            } else if child.kind() == "number" {
                if let Some(child_stream) = self.visit(child)? {
                    expression_parts.push(child_stream);
                }
            } else if let Some(child_stream) = self.visit(child)? {
                expression_parts.push(child_stream);
            }
        }
        Ok((has_return, expression_parts))
    }

    fn process_jsx_text_in_error(
        &mut self,
        child: &Node,
        expression_parts: &mut Vec<TokenStream>,
    ) -> Result<(), RecursionDepthError> {
        let Some(raw) = self.source_text(child) else {
            if let Some(child_stream) = self.visit(child)? {
                expression_parts.push(child_stream);
            }
            return Ok(());
        };

        let text = raw.trim();
        // Match identifier followed by ')' — common type-assertion misparse residue.
        let ident_end = text
            .char_indices()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map(|(i, c)| i + c.len_utf8());

        if let Some(end) = ident_end {
            let identifier_text = &text[..end];
            let rest = text[end..].trim_start();
            if let Some(after_paren) = rest.strip_prefix(')') {
                if self.is_identifier_bound_ts(identifier_text) {
                    expression_parts.push(self.normalize_bound_identifier_ts(identifier_text));
                } else {
                    expression_parts.push(TokenStream::identifier(identifier_text.to_string()));
                }
                let after = after_paren.trim_start();
                if after.starts_with('+') {
                    expression_parts.push(TokenStream::text("+"));
                    let digits: String = after
                        .chars()
                        .skip(1)
                        .skip_while(|c| c.is_whitespace())
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if !digits.is_empty() {
                        expression_parts.push(TokenStream::literal("number", digits));
                    }
                }
                return Ok(());
            }
        }

        if !text.is_empty()
            && text.chars().all(|c| c.is_alphanumeric() || c == '_')
            && text
                .chars()
                .next()
                .map(|c| c.is_alphabetic() || c == '_')
                .unwrap_or(false)
        {
            if self.is_identifier_bound_ts(text) {
                expression_parts.push(self.normalize_bound_identifier_ts(text));
            } else {
                expression_parts.push(TokenStream::identifier(text.to_string()));
            }
            return Ok(());
        }

        if let Some(child_stream) = self.visit(child)? {
            expression_parts.push(child_stream);
        }
        Ok(())
    }

    fn process_jsx_opening_element_in_error(
        &mut self,
        child: &Node,
        expression_parts: &mut Vec<TokenStream>,
    ) -> Result<(), RecursionDepthError> {
        if self.is_type_assertion_jsx_element(child) {
            return Ok(());
        }
        if let Some(child_stream) = self.visit(child)? {
            expression_parts.push(child_stream);
        }
        Ok(())
    }

    fn is_type_assertion_jsx_element(&self, child: &Node) -> bool {
        let mut cursor = child.walk();
        for jsx_child in child.children(&mut cursor) {
            if jsx_child.kind() == "identifier" {
                if let Some(type_name) = self.source_text(&jsx_child) {
                    if self.is_typescript_type_name(&type_name) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn check_return_keyword(&self, child: &Node) -> bool {
        if child.kind() == "return" {
            return true;
        }
        if child.kind() == "identifier" {
            if let Some(text) = self.source_text(child) {
                return text == "return";
            }
        }
        false
    }

    fn construct_return_statement(&self, expression_parts: Vec<TokenStream>) -> TokenStream {
        if expression_parts.len() >= 3 {
            let binary_expr = TokenStream::node("binary_expression", expression_parts);
            TokenStream::node("return_statement", vec![binary_expr])
        } else {
            TokenStream::node("return_statement", expression_parts)
        }
    }

    fn dispatch_visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "comment" => self.visit_comment(node),
            "line_comment" => self.visit_line_comment(node),
            "block_comment" => self.visit_block_comment(node),
            "jsdoc" => self.visit_jsdoc(node),
            "type_annotation" => self.visit_type_annotation(node),
            "type_parameters" => self.visit_type_parameters(node),
            "type_arguments" => self.visit_type_arguments(node),
            "type_assertion" => self.visit_type_assertion(node),
            "as_expression" => self.visit_as_expression(node),
            "parenthesized_expression" => self.visit_parenthesized_expression(node),
            "interface_declaration" => self.visit_interface_declaration(node),
            "type_alias_declaration" => self.visit_type_alias_declaration(node),
            "enum_declaration" => self.visit_enum_declaration(node),
            "namespace_declaration" => self.visit_namespace_declaration(node),
            "internal_module" => self.visit_internal_module(node),
            "decorator" => self.visit_decorator(node),
            "function_declaration" => self.visit_function_declaration(node),
            "generator_function_declaration" => self.visit_generator_function_declaration(node),
            "function_expression" | "function" => self.visit_function_declaration(node),
            "method_definition" => self.visit_method_definition(node),
            "arrow_function" => self.visit_arrow_function(node),
            "class_declaration" => self.visit_class_declaration(node),
            "abstract_class_declaration" => self.visit_abstract_class_declaration(node),
            "statement_block" => self.visit_statement_block(node),
            "formal_parameters" => self.visit_formal_parameters(node),
            "string" => self.visit_string(node),
            "template_string" => self.visit_template_string(node),
            "call_expression" => self.visit_call_expression(node),
            "identifier" => self.visit_identifier(node),
            "if_statement" => self.visit_simple_named(node, "if_statement"),
            "while_statement" => self.visit_simple_named(node, "while_statement"),
            "for_statement" => self.visit_simple_named(node, "for_statement"),
            "for_in_statement" => self.visit_for_in_statement(node),
            "for_of_statement" => self.visit_for_of_statement(node),
            "return_statement" => self.visit_return_statement(node),
            "break_statement" => Ok(Some(TokenStream::node("break_statement", vec![]))),
            "continue_statement" => Ok(Some(TokenStream::node("continue_statement", vec![]))),
            "switch_statement" => self.visit_simple_named(node, "switch_statement"),
            "try_statement" => self.visit_simple_named(node, "try_statement"),
            "with_statement" => self.visit_simple_named(node, "with_statement"),
            "throw_statement" => self.visit_throw_statement(node),
            "number" => self.visit_number(node),
            "member_expression" => self.visit_member_expression(node),
            "new_expression" => self.visit_new_expression(node),
            "variable_declaration" => self.visit_variable_declaration(node),
            "lexical_declaration" => self.visit_lexical_declaration(node),
            "variable_declarator" => self.visit_variable_declarator(node),
            "import_statement" => self.visit_import_statement(node),
            "export_statement" => self.visit_export_statement(node),
            "jsx_element" => self.visit_jsx_element(node),
            "jsx_opening_element" => self.visit_jsx_opening_element(node),
            "jsx_attribute" => self.visit_jsx_attribute(node),
            "jsx_expression" => self.visit_jsx_expression(node),
            "jsx_text" => self.visit_jsx_text(node),
            "semicolon" | ";" => Ok(None),
            "expression_statement" => self.visit_expression_statement(node),
            "ERROR" => self.visit_error(node),
            _ => self.visit_generic(node),
        }
    }
}

impl<'a> NodeVisitor for TypeScriptNodeVisitor<'a> {
    fn source(&self) -> &[u8] {
        self.source
    }
    fn context(&mut self) -> (&mut BaseVisitor, &[u8]) {
        (&mut self.base, self.source)
    }

    fn dispatch(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.dispatch_visit(node)
    }

    fn retain_empty_node(&self, kind: &str) -> bool {
        kind == "program"
    }
}

impl EcmaVisitor for TypeScriptNodeVisitor<'_> {
    fn type_nodes(&self) -> &[&str] {
        TYPE_RELATED_NODES
    }
    fn should_skip_function_child(&self, node: &Node) -> bool {
        TypeScriptNodeVisitor::should_skip_function_child(self, node)
    }
    fn process_single_parameter_in_visitor(
        &mut self,
        node: &Node,
        skip: bool,
    ) -> Result<(Option<TokenStream>, bool), RecursionDepthError> {
        TypeScriptNodeVisitor::process_single_parameter_in_visitor(self, node, skip)
    }
    fn parameter_positions(&mut self) -> &mut std::collections::HashMap<String, usize> {
        &mut self.param_name_to_position
    }
    fn is_type_only_import(&self, node: &Node) -> bool {
        TypeScriptNodeVisitor::is_type_only_import(self, node)
    }
    fn process_import_clause(
        &mut self,
        node: &Node,
        parts: &mut Vec<TokenStream>,
        module: Option<&str>,
        statement: &Node,
    ) {
        TypeScriptNodeVisitor::process_import_clause(self, node, parts, module, statement)
    }
}
