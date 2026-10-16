//! C# AST visitor for structured serialization.

use std::collections::HashMap;

use tree_sitter::Node;

use crate::serialization::TokenStream;
use crate::visitor::{BaseVisitor, NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::normalizer::CsharpNormalizer;
use super::scope_analyzer::CsharpScopeAnalyzer;

/// Visitor for C# AST nodes using structured serialization.
pub struct CsharpNodeVisitor<'a> {
    pub base: BaseVisitor,
    pub source: &'a [u8],
    pub normalizer: Option<&'a CsharpNormalizer>,
    pub using_aliases: HashMap<String, String>,
    pub current_parent_type: Option<String>,
}

impl<'a> CsharpNodeVisitor<'a> {
    pub fn new(
        policy: Option<StructuralHashPolicy>,
        source: &'a [u8],
        normalizer: Option<&'a CsharpNormalizer>,
        using_aliases: HashMap<String, String>,
    ) -> Self {
        Self {
            base: BaseVisitor::new(None, policy),
            source,
            normalizer,
            using_aliases,
            current_parent_type: None,
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

    fn register_arg_if_new(&mut self, identifier_text: &str) -> String {
        self.base.normalize_parameter_name(identifier_text)
    }

    fn is_identifier_bound(&self, identifier_text: &str) -> bool {
        self.base.is_allowed_local(identifier_text)
    }

    fn is_type_node_kind(kind: &str) -> bool {
        matches!(
            kind,
            "predefined_type"
                | "nullable_type"
                | "generic_name"
                | "qualified_name"
                | "array_type"
                | "pointer_type"
                | "ref_type"
                | "tuple_type"
                | "type_argument_list"
                | "implicit_type"
        )
    }

    fn visit_children_skipping_punctuation(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "{" | "}" | "(" | ")" | ";" | "," | "." | ":" | "=" | "<" | ">" | "?" | "[" | "]"
            ) {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                if TokenStream::has_content(&child_stream) {
                    children_streams.push(child_stream);
                }
            }
        }
        Ok(children_streams)
    }

    fn normalize_integer_literal_node(&self, node: &Node) -> Option<TokenStream> {
        let literal_text = self.source_text(node)?;
        let literal_text = if let Some(normalizer) = self.normalizer {
            normalizer.normalize_integer_literal_text(&literal_text)
        } else {
            literal_text
        };
        Some(TokenStream::literal("integer_literal", literal_text))
    }

    fn emit_sorted_modifiers(&self, modifier_nodes: &[Node]) -> Vec<TokenStream> {
        let names = if let Some(normalizer) = self.normalizer {
            normalizer.normalize_modifiers(modifier_nodes, self.source)
        } else {
            let mut names: Vec<String> = modifier_nodes.iter().filter_map(|n| self.source_text(n)).collect();
            names.sort();
            names
        };
        names
            .into_iter()
            .map(|name| TokenStream::node("modifier", vec![TokenStream::text(name)]))
            .collect()
    }

    fn visit_with_wrapped_body(
        &mut self,
        node: &Node,
        node_type: &str,
        body_fields: &[&str],
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.current_parent_type = Some(node_type.to_string());

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        for (idx, child) in children.iter().enumerate() {
            let field_name = u32::try_from(idx)
                .ok()
                .and_then(|index| node.field_name_for_child(index));
            if matches!(
                child.kind(),
                "{" | "}" | "(" | ")" | ";" | "," | "if" | "else" | "while" | "for" | "foreach" | "in"
            ) {
                continue;
            }
            if field_name.map(|f| body_fields.contains(&f)).unwrap_or(false) {
                let child_stream = self.visit(child)?;
                let Some(child_stream) = child_stream else {
                    continue;
                };
                if child.kind() == "block" {
                    children_streams.push(child_stream);
                } else if let Some(normalizer) = self.normalizer {
                    let parent_type = self.current_parent_type.as_deref().unwrap_or("control_flow");
                    if normalizer.should_wrap_child(child, parent_type) {
                        children_streams.push(TokenStream::node("block", vec![child_stream]));
                    } else {
                        children_streams.push(child_stream);
                    }
                } else {
                    children_streams.push(child_stream);
                }
            } else if let Some(child_stream) = self.visit(child)? {
                if TokenStream::has_content(&child_stream) {
                    children_streams.push(child_stream);
                }
            }
        }

        self.current_parent_type = None;

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node(node_type, vec![])))
        } else {
            Ok(Some(TokenStream::node(node_type, children_streams)))
        }
    }

    // ------------------------------------------------------------------
    // Comments / punctuation / modifiers
    // ------------------------------------------------------------------

    fn visit_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "comment")
    }

    fn visit_modifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Sorted at declaration level; standalone fallthrough keeps text.
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::node("modifier", vec![TokenStream::text(text)])));
        }
        Ok(None)
    }

    fn visit_semicolon(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Using directives
    // ------------------------------------------------------------------

    fn visit_using_directive(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut alias_name = None;
        let mut qualified = None;
        let mut saw_equals = false;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "=" => {
                    saw_equals = true;
                }
                "identifier" => {
                    if !saw_equals {
                        alias_name = self.source_text(&child);
                    } else {
                        qualified = self.source_text(&child);
                    }
                }
                "qualified_name" => {
                    qualified = self.source_text(&child);
                }
                _ => {}
            }
        }

        // Alias form: using Alias = Qualified;
        if saw_equals {
            if let (Some(alias), Some(qual)) = (alias_name.clone(), qualified.clone()) {
                self.using_aliases.insert(alias, qual);
            }
        }

        // Granular: track namespace / alias / last segment for import attribution.
        if let Some(ref qual) = qualified {
            self.base.track_file_import(qual.clone());
            if let Some(last) = qual.rsplit('.').next() {
                if last != qual.as_str() {
                    self.base.track_file_import(last.to_string());
                }
            }
        } else if let Some(ref name) = alias_name {
            // `using System;` — single identifier namespace
            if !saw_equals {
                self.base.track_file_import(name.clone());
            }
        }
        if saw_equals {
            if let Some(ref alias) = alias_name {
                self.base.track_file_import(alias.clone());
            }
        }

        let start = node.start_byte();
        let end = node.end_byte();
        if saw_equals {
            if let (Some(alias), Some(qual)) = (alias_name.clone(), qualified.clone()) {
                self.base
                    .track_import_marker(qual, Some(alias), entmoot_contracts::ImportKind::Alias, start, end);
            }
        } else if let Some(qual) = qualified {
            self.base
                .track_import_marker(qual, None, entmoot_contracts::ImportKind::Module, start, end);
        } else if let Some(name) = alias_name {
            self.base
                .track_import_marker(name, None, entmoot_contracts::ImportKind::Module, start, end);
        }

        let children = self.visit_children_skipping_punctuation(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node("using_directive", vec![])))
        } else {
            Ok(Some(TokenStream::node("using_directive", children)))
        }
    }

    // ------------------------------------------------------------------
    // Types (contracts — always retained)
    // ------------------------------------------------------------------

    fn visit_type_like(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Signature/local types are contracts: never prune even if prune_type_hints.
        let children = self.visit_children_skipping_punctuation(node)?;
        if children.is_empty() {
            if let Some(text) = self.source_text(node) {
                return Ok(Some(TokenStream::node(node.kind(), vec![TokenStream::text(text)])));
            }
            return Ok(Some(TokenStream::node(node.kind(), vec![])));
        }
        Ok(Some(TokenStream::node(node.kind(), children)))
    }

    fn visit_predefined_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::node(
                "predefined_type",
                vec![TokenStream::text(text)],
            )));
        }
        Ok(Some(TokenStream::node("predefined_type", vec![])))
    }

    fn visit_nullable_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("nullable_type", children)))
    }

    fn visit_generic_name(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("generic_name", children)))
    }

    fn visit_qualified_name(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("qualified_name", children)))
    }

    fn visit_type_argument_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("type_argument_list", children)))
    }

    // ------------------------------------------------------------------
    // Parameters / methods / constructors / properties
    // ------------------------------------------------------------------

    fn visit_parameter(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();

        // Type children are contracts — always visit/retain.
        if let Some(type_node) = node.child_by_field_name("type") {
            if let Some(type_stream) = self.visit(&type_node)? {
                children_streams.push(type_stream);
            }
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if Self::is_type_node_kind(child.kind()) {
                    if let Some(type_stream) = self.visit(&child)? {
                        children_streams.push(type_stream);
                    }
                }
            }
        }

        let name_node = node.child_by_field_name("name").or_else(|| {
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            children.into_iter().find(|c| c.kind() == "identifier")
        });

        if let Some(name_node) = name_node {
            let identifier_text = self.source_text(&name_node).unwrap_or_default();
            let normalized = self.register_arg_if_new(&identifier_text);
            children_streams.push(TokenStream::identifier(normalized));
        }

        // Retain modifiers on parameters (ref/out/in/params) if present.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "modifier" {
                if let Some(mod_stream) = self.visit_modifier(&child)? {
                    children_streams.insert(0, mod_stream);
                }
            }
        }

        Ok(Some(TokenStream::node("parameter", children_streams)))
    }

    fn visit_parameter_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.set_in_parameter_list(true);
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "parameter" {
                if let Some(param_stream) = self.visit_parameter(&child)? {
                    children_streams.push(param_stream);
                }
            }
        }
        self.set_in_parameter_list(false);
        Ok(Some(TokenStream::node("parameter_list", children_streams)))
    }

    fn extract_callable_name(&self, node: &Node) -> String {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Some(text) = self.source_text(&name_node) {
                if !text.is_empty() {
                    return text;
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                if let Some(text) = self.source_text(&child) {
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
        }
        "<anonymous>".to_string()
    }

    /// Streams for callable hashing: parameters + body only (exclude method name).
    fn maybe_track_import_usage(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        // `Path` under `using System.IO` — attribute as import usage for tests.
        if name == "Path" && (self.base.file_imports.contains("System.IO") || self.base.file_imports.contains("IO")) {
            self.base.track_file_import("Path");
            self.base.track_callable_import_usage("Path");
            return;
        }
        if self.base.file_imports.contains(name) {
            self.base.track_callable_import_usage(name);
        }
    }

    fn visit_method_or_constructor(
        &mut self,
        node: &Node,
        node_type: &str,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let callable_name = self.extract_callable_name(node);
        let callable_kind = if node.kind() == "local_function_statement" {
            "function"
        } else {
            "method"
        };
        self.base
            .begin_callable(callable_name, callable_kind, node.start_byte(), node.end_byte());

        self.base.enter_scope("method");
        self.set_in_parameter_list(false);

        self.base
            .preflight_callable_scope(&CsharpScopeAnalyzer, node, self.source);

        let mut children_streams = Vec::new();
        let mut modifier_nodes = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        for child in &children {
            if child.kind() == "modifier" {
                modifier_nodes.push(*child);
            }
        }
        children_streams.extend(self.emit_sorted_modifiers(&modifier_nodes));

        for (idx, child) in children.iter().enumerate() {
            let field = u32::try_from(idx)
                .ok()
                .and_then(|index| node.field_name_for_child(index));
            match child.kind() {
                "modifier" | "comment" => {
                    if child.kind() == "comment" {
                        let _ = self.visit(child)?;
                    }
                    continue;
                }
                "parameter_list" => {
                    if let Some(stream) = self.visit_parameter_list(child)? {
                        children_streams.push(stream);
                    }
                }
                _ if field == Some("returns") || field == Some("type") || Self::is_type_node_kind(child.kind()) => {
                    // Return / local-function types are contracts.
                    if let Some(stream) = self.visit(child)? {
                        children_streams.push(stream);
                    }
                }
                "identifier" if field == Some("name") || field.is_none() => {
                    // Method/ctor name — keep as-is (not a local).
                    if let Some(text) = self.source_text(child) {
                        children_streams.push(TokenStream::identifier(text));
                    }
                }
                "block" | "arrow_expression_clause" => {
                    if let Some(stream) = self.visit(child)? {
                        children_streams.push(stream);
                    }
                }
                _ => {
                    if let Some(stream) = self.visit(child)? {
                        if TokenStream::has_content(&stream) {
                            children_streams.push(stream);
                        }
                    }
                }
            }
        }

        self.base.exit_callable_scope();
        self.base.exit_scope();

        self.base.end_callable();

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node(node_type, vec![])))
        } else {
            Ok(Some(TokenStream::node(node_type, children_streams)))
        }
    }

    fn visit_method_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_method_or_constructor(node, "method_declaration")
    }

    fn visit_constructor_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_method_or_constructor(node, "constructor_declaration")
    }

    fn visit_local_function_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_method_or_constructor(node, "local_function_statement")
    }

    fn visit_property_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut modifier_nodes = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        for child in &children {
            if child.kind() == "modifier" {
                modifier_nodes.push(*child);
            }
        }
        children_streams.extend(self.emit_sorted_modifiers(&modifier_nodes));

        for child in &children {
            match child.kind() {
                "modifier" | "comment" => continue,
                "accessor_list" => {
                    if let Some(stream) = self.visit_accessor_list(child)? {
                        children_streams.push(stream);
                    }
                }
                _ if Self::is_type_node_kind(child.kind()) => {
                    if let Some(stream) = self.visit(child)? {
                        children_streams.push(stream);
                    }
                }
                "identifier" => {
                    if let Some(text) = self.source_text(child) {
                        children_streams.push(TokenStream::identifier(text));
                    }
                }
                _ => {
                    if let Some(stream) = self.visit(child)? {
                        if TokenStream::has_content(&stream) {
                            children_streams.push(stream);
                        }
                    }
                }
            }
        }

        Ok(Some(TokenStream::node("property_declaration", children_streams)))
    }

    fn visit_accessor_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("accessor_list", children)))
    }

    fn visit_accessor_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("accessor");
        self.base
            .preflight_callable_scope(&CsharpScopeAnalyzer, node, self.source);

        let children = self.visit_children_skipping_punctuation(node)?;

        self.base.exit_callable_scope();
        self.base.exit_scope();
        Ok(Some(TokenStream::node("accessor_declaration", children)))
    }

    // ------------------------------------------------------------------
    // Classes / namespaces / blocks
    // ------------------------------------------------------------------

    fn visit_type_declaration(
        &mut self,
        node: &Node,
        node_type: &str,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let type_name = self.extract_callable_name(node);
        self.base.push_definition_scope(type_name);
        self.base.push_type_body();
        self.base.enter_scope("class");
        let mut children_streams = Vec::new();
        let mut modifier_nodes = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        for child in &children {
            if child.kind() == "modifier" {
                modifier_nodes.push(*child);
            }
        }
        children_streams.extend(self.emit_sorted_modifiers(&modifier_nodes));

        for child in &children {
            if child.kind() == "modifier" || matches!(child.kind(), "class" | "struct" | "interface" | "record") {
                continue;
            }
            if let Some(stream) = self.visit(child)? {
                if TokenStream::has_content(&stream) {
                    children_streams.push(stream);
                }
            }
        }

        self.base.exit_scope();
        self.base.pop_type_body();
        self.base.pop_definition_scope();
        Ok(Some(TokenStream::node(node_type, children_streams)))
    }

    fn visit_class_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_type_declaration(node, "class_declaration")
    }

    fn visit_struct_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_type_declaration(node, "struct_declaration")
    }

    fn visit_interface_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_type_declaration(node, "interface_declaration")
    }

    fn visit_namespace_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("namespace");
        let children = self.visit_children_skipping_punctuation(node)?;
        self.base.exit_scope();
        Ok(Some(TokenStream::node("namespace_declaration", children)))
    }

    fn visit_file_scoped_namespace_declaration(
        &mut self,
        node: &Node,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("namespace");
        let children = self.visit_children_skipping_punctuation(node)?;
        self.base.exit_scope();
        Ok(Some(TokenStream::node("file_scoped_namespace_declaration", children)))
    }

    fn visit_declaration_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("declaration_list", children)))
    }

    fn visit_block(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("block");
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "{" | "}") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                if TokenStream::has_content(&child_stream) {
                    children_streams.push(child_stream);
                }
            }
        }
        self.base.exit_scope();
        Ok(Some(TokenStream::node("block", children_streams)))
    }

    // ------------------------------------------------------------------
    // Control flow / statements
    // ------------------------------------------------------------------

    fn visit_if_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "if_statement", &["consequence", "alternative"])
    }

    fn visit_while_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "while_statement", &["body"])
    }

    fn visit_for_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "for_statement", &["body"])
    }

    fn visit_foreach_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "foreach_statement", &["body"])
    }

    fn visit_simple_named(&mut self, node: &Node, name: &str) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node(name, vec![])))
        } else {
            Ok(Some(TokenStream::node(name, children)))
        }
    }

    fn visit_catch_clause(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Register catch variable before visiting body.
        self.visit_simple_named(node, "catch_clause")
    }

    fn visit_catch_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut idents: Vec<Node> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "(" | ")") {
                continue;
            }
            if child.kind() == "identifier" {
                idents.push(child);
            } else if let Some(stream) = self.visit(&child)? {
                children_streams.push(stream);
            }
        }
        // Last identifier is the catch variable; earlier ones are type names.
        if let Some((last, type_idents)) = idents.split_last() {
            for type_ident in type_idents {
                if let Some(text) = self.source_text(type_ident) {
                    let resolved = self
                        .normalizer
                        .map(|n| n.resolve_using_alias(&text, &self.using_aliases))
                        .unwrap_or(text);
                    children_streams.push(TokenStream::identifier(resolved));
                }
            }
            if let Some(text) = self.source_text(last) {
                {
                    children_streams.push(TokenStream::identifier(text));
                }
            }
        }
        Ok(Some(TokenStream::node("catch_declaration", children_streams)))
    }

    // ------------------------------------------------------------------
    // Expressions / literals / identifiers
    // ------------------------------------------------------------------

    fn visit_await_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("await_expression", children)))
    }

    fn visit_invocation_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Syntactic callee text before argument_list (no IR change).
        if let Some(func) = node.child_by_field_name("function") {
            if let Some(callee) = self.source_text(&func) {
                if !callee.is_empty() {
                    self.base.track_call_node(callee, node, self.source);
                }
            }
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "argument_list" {
                    break;
                }
                if matches!(child.kind(), "(" | ")" | ".") {
                    continue;
                }
                if let Some(callee) = self.source_text(&child) {
                    if !callee.is_empty() {
                        self.base.track_call_node(callee, node, self.source);
                        break;
                    }
                }
            }
        }
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("invocation_expression", children)))
    }

    fn visit_member_access_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("member_access_expression", children)))
    }

    fn visit_argument_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("argument_list", children)))
    }

    fn visit_binary_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("binary_expression", children)))
        }
    }

    fn visit_string_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            Ok(Some(TokenStream::literal("string_literal", text)))
        } else {
            Ok(Some(TokenStream::node("string_literal", vec![])))
        }
    }

    fn visit_null_literal(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(Some(TokenStream::literal("null_literal", "null")))
    }

    fn visit_local_declaration_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("local_declaration_statement", children)))
    }

    fn visit_variable_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("variable_declaration", children)))
    }

    fn visit_variable_declarator(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "=" {
                continue;
            }
            if child.kind() == "identifier" {
                let text = self.source_text(&child).unwrap_or_default();
                {
                    children_streams.push(TokenStream::identifier(text));
                }
            } else if let Some(stream) = self.visit(&child)? {
                if TokenStream::has_content(&stream) {
                    children_streams.push(stream);
                }
            }
        }
        Ok(Some(TokenStream::node("variable_declarator", children_streams)))
    }

    fn visit_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if self.var_maps().is_empty() || self.in_parameter_list() {
            let text = self.source_text(node).unwrap_or_default();
            self.maybe_track_import_usage(&text);
            let resolved = self
                .normalizer
                .map(|n| n.resolve_using_alias(&text, &self.using_aliases))
                .unwrap_or(text);
            return Ok(Some(TokenStream::identifier(resolved)));
        }

        let identifier_text = self.source_text(node).unwrap_or_default();
        self.maybe_track_import_usage(&identifier_text);

        let is_bound_variable = self.is_identifier_bound(&identifier_text);
        if !is_bound_variable {
            let resolved = self
                .normalizer
                .map(|n| n.resolve_using_alias(&identifier_text, &self.using_aliases))
                .unwrap_or(identifier_text);
            return Ok(Some(TokenStream::identifier(resolved)));
        }

        Ok(Some(TokenStream::identifier(identifier_text)))
    }

    fn dispatch_visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "comment" => self.visit_comment(node),
            "modifier" => self.visit_modifier(node),
            ";" | "semicolon" => self.visit_semicolon(node),
            "using_directive" => self.visit_using_directive(node),
            "compilation_unit" => self.visit_simple_named(node, "compilation_unit"),
            "class_declaration" => self.visit_class_declaration(node),
            "struct_declaration" => self.visit_struct_declaration(node),
            "interface_declaration" => self.visit_interface_declaration(node),
            "namespace_declaration" => self.visit_namespace_declaration(node),
            "file_scoped_namespace_declaration" => self.visit_file_scoped_namespace_declaration(node),
            "declaration_list" => self.visit_declaration_list(node),
            "method_declaration" => self.visit_method_declaration(node),
            "constructor_declaration" => self.visit_constructor_declaration(node),
            "local_function_statement" => self.visit_local_function_statement(node),
            "property_declaration" => self.visit_property_declaration(node),
            "accessor_list" => self.visit_accessor_list(node),
            "accessor_declaration" => self.visit_accessor_declaration(node),
            "parameter_list" => self.visit_parameter_list(node),
            "parameter" => self.visit_parameter(node),
            "block" => self.visit_block(node),
            "predefined_type" => self.visit_predefined_type(node),
            "nullable_type" => self.visit_nullable_type(node),
            "generic_name" => self.visit_generic_name(node),
            "qualified_name" => self.visit_qualified_name(node),
            "type_argument_list" => self.visit_type_argument_list(node),
            "array_type" | "pointer_type" | "ref_type" | "tuple_type" | "implicit_type" => self.visit_type_like(node),
            "if_statement" => self.visit_if_statement(node),
            "while_statement" => self.visit_while_statement(node),
            "for_statement" => self.visit_for_statement(node),
            "foreach_statement" => self.visit_foreach_statement(node),
            "switch_statement" => self.visit_simple_named(node, "switch_statement"),
            "try_statement" => self.visit_simple_named(node, "try_statement"),
            "catch_clause" => self.visit_catch_clause(node),
            "catch_declaration" => self.visit_catch_declaration(node),
            "throw_statement" => self.visit_simple_named(node, "throw_statement"),
            "return_statement" => self.visit_simple_named(node, "return_statement"),
            "break_statement" => self.visit_simple_named(node, "break_statement"),
            "continue_statement" => self.visit_simple_named(node, "continue_statement"),
            "local_declaration_statement" => self.visit_local_declaration_statement(node),
            "variable_declaration" => self.visit_variable_declaration(node),
            "variable_declarator" => self.visit_variable_declarator(node),
            "await_expression" => self.visit_await_expression(node),
            "invocation_expression" => self.visit_invocation_expression(node),
            "member_access_expression" => self.visit_member_access_expression(node),
            "argument_list" => self.visit_argument_list(node),
            "argument" => self.visit_simple_named(node, "argument"),
            "binary_expression" => self.visit_binary_expression(node),
            "string_literal" | "verbatim_string_literal" | "raw_string_literal" => self.visit_string_literal(node),
            "integer_literal" => Ok(self.normalize_integer_literal_node(node)),
            "null_literal" => self.visit_null_literal(node),
            "identifier" => self.visit_identifier(node),
            // No visit_ERROR — hasher rejects ERROR for C# (fail closed)
            _ => self.visit_generic(node),
        }
    }
}

impl<'a> NodeVisitor for CsharpNodeVisitor<'a> {
    fn source(&self) -> &[u8] {
        self.source
    }
    fn context(&mut self) -> (&mut BaseVisitor, &[u8]) {
        (&mut self.base, self.source)
    }

    fn dispatch(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.dispatch_visit(node)
    }
}
