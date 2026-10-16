//! Rust AST visitor for structured serialization.

use std::collections::HashMap;

use tree_sitter::Node;

use crate::serialization::TokenStream;
use crate::visitor::{BaseVisitor, NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::normalizer::RustNormalizer;
use super::scope_analyzer::RustScopeAnalyzer;

const RESERVED_UNRENAMED: &[&str] = &["self", "Self", "crate", "super", "_"];

const TYPE_NODE_KINDS: &[&str] = &[
    "primitive_type",
    "type_identifier",
    "scoped_type_identifier",
    "generic_type",
    "reference_type",
    "array_type",
    "tuple_type",
    "unit_type",
    "pointer_type",
    "function_type",
    "dyn_type",
    "bounded_type",
    "type_arguments",
    "lifetime",
    "lifetime_parameter",
    "type_parameters",
    "type_parameter",
    "mutable_specifier",
];

/// Visitor for Rust AST nodes using structured serialization.
pub struct RustNodeVisitor<'a> {
    pub base: BaseVisitor,
    pub source: &'a [u8],
    pub normalizer: Option<&'a RustNormalizer>,
    pub use_aliases: HashMap<String, String>,
}

impl<'a> RustNodeVisitor<'a> {
    pub fn new(
        policy: Option<StructuralHashPolicy>,
        source: &'a [u8],
        normalizer: Option<&'a RustNormalizer>,
        use_aliases: HashMap<String, String>,
    ) -> Self {
        Self {
            base: BaseVisitor::new(None, policy),
            source,
            normalizer,
            use_aliases,
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
        if RESERVED_UNRENAMED.contains(&identifier_text) {
            return identifier_text.to_string();
        }
        self.base.normalize_parameter_name(identifier_text)
    }

    fn is_identifier_bound(&self, identifier_text: &str) -> bool {
        self.base.is_allowed_local(identifier_text)
    }

    fn is_reserved_unrenamed(text: &str) -> bool {
        RESERVED_UNRENAMED.contains(&text)
    }

    fn is_type_node_kind(kind: &str) -> bool {
        TYPE_NODE_KINDS.contains(&kind)
    }

    fn resolve_alias(&self, identifier_text: &str) -> String {
        self.normalizer
            .map(|n| n.resolve_use_alias(identifier_text, &self.use_aliases))
            .unwrap_or_else(|| identifier_text.to_string())
    }

    fn should_skip_child(kind: &str) -> bool {
        matches!(
            kind,
            "{" | "}"
                | "("
                | ")"
                | ";"
                | ","
                | "."
                | ":"
                | "::"
                | "="
                | "<"
                | ">"
                | "?"
                | "["
                | "]"
                | "->"
                | "=>"
                | "#"
                | "'"
                | ".."
                | "fn"
                | "let"
                | "use"
                | "as"
                | "impl"
                | "mod"
                | "trait"
                | "match"
                | "for"
                | "in"
                | "while"
                | "loop"
                | "if"
                | "else"
                | "async"
                | "unsafe"
                | "pub"
        )
    }

    fn visit_children_skipping_punctuation(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Self::should_skip_child(child.kind()) {
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

    fn visit_simple_named(&mut self, node: &Node, name: &str) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node(name, vec![])))
        } else {
            Ok(Some(TokenStream::node(name, children)))
        }
    }

    // ------------------------------------------------------------------
    // Comments / attributes
    // ------------------------------------------------------------------

    fn visit_line_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "line_comment")
    }

    fn visit_block_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "block_comment")
    }

    fn visit_attribute_item(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Attributes are KEPT by default.
        self.visit_generic(node)
    }

    fn visit_attribute(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_generic(node)
    }

    // ------------------------------------------------------------------
    // Use declarations
    // ------------------------------------------------------------------

    fn visit_use_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Track `use path as Alias`
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "use_as_clause" {
                self.capture_use_as_clause(&child);
            }
        }
        for name in self.extract_use_import_binding_names(node) {
            self.base.track_file_import(name);
        }
        self.emit_use_import_markers(node);
        let children = self.visit_children_skipping_punctuation(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node("use_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("use_declaration", children)))
        }
    }

    fn emit_use_import_markers(&mut self, node: &Node) {
        let start = node.start_byte();
        let end = node.end_byte();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "use_as_clause" => {
                    if let (Some(path), Some(alias)) =
                        (self.use_as_clause_path(&child), self.use_as_clause_alias(&child))
                    {
                        self.base.track_import_marker(
                            path,
                            Some(alias),
                            entmoot_contracts::ImportKind::Alias,
                            start,
                            end,
                        );
                    }
                }
                "use_wildcard" => {
                    let path = self.use_wildcard_path(&child).unwrap_or_default();
                    self.base
                        .track_import_marker(path, None, entmoot_contracts::ImportKind::Star, start, end);
                }
                "scoped_use_list" => {
                    self.emit_scoped_use_list_markers(&child, start, end);
                }
                "use_list" => {
                    self.emit_use_list_markers(&child, None, start, end);
                }
                "scoped_identifier" | "identifier" => {
                    if let Some(path) = self.source_text(&child) {
                        self.base
                            .track_import_marker(path, None, entmoot_contracts::ImportKind::Named, start, end);
                    }
                }
                _ => {}
            }
        }
    }

    fn emit_scoped_use_list_markers(&mut self, node: &Node, start: usize, end: usize) {
        let prefix = node.child_by_field_name("path").and_then(|p| self.source_text(&p));
        if let Some(list) = node.child_by_field_name("list") {
            self.emit_use_list_markers(&list, prefix.as_deref(), start, end);
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "use_list" {
                    self.emit_use_list_markers(&child, prefix.as_deref(), start, end);
                }
            }
        }
    }

    fn emit_use_list_markers(&mut self, node: &Node, prefix: Option<&str>, start: usize, end: usize) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "use_as_clause" => {
                    if let (Some(path), Some(alias)) =
                        (self.use_as_clause_path(&child), self.use_as_clause_alias(&child))
                    {
                        let full = match prefix {
                            Some(p) => format!("{p}::{path}"),
                            None => path,
                        };
                        self.base.track_import_marker(
                            full,
                            Some(alias),
                            entmoot_contracts::ImportKind::Alias,
                            start,
                            end,
                        );
                    }
                }
                "use_wildcard" => {
                    let path = prefix
                        .map(str::to_string)
                        .or_else(|| self.use_wildcard_path(&child))
                        .unwrap_or_default();
                    self.base
                        .track_import_marker(path, None, entmoot_contracts::ImportKind::Star, start, end);
                }
                "scoped_identifier" | "identifier" => {
                    if let Some(seg) = self.source_text(&child) {
                        let path = match prefix {
                            Some(p) => format!("{p}::{seg}"),
                            None => seg,
                        };
                        self.base
                            .track_import_marker(path, None, entmoot_contracts::ImportKind::Named, start, end);
                    }
                }
                "use_list" | "scoped_use_list" => {
                    if child.kind() == "scoped_use_list" {
                        self.emit_scoped_use_list_markers(&child, start, end);
                    } else {
                        self.emit_use_list_markers(&child, prefix, start, end);
                    }
                }
                _ => {}
            }
        }
    }

    fn use_wildcard_path(&self, node: &Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "identifier" | "scoped_identifier" | "crate" | "super" | "self"
            ) {
                return self.source_text(&child);
            }
        }
        None
    }

    fn use_as_clause_path(&self, node: &Node) -> Option<String> {
        let mut path_text = None;
        let mut saw_as = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "as" => saw_as = true,
                "scoped_identifier" | "identifier" if !saw_as => {
                    path_text = self.source_text(&child);
                }
                _ => {
                    if !saw_as && path_text.is_none() {
                        if let Some(t) = self.source_text(&child) {
                            if child.kind() != "as" {
                                path_text = Some(t);
                            }
                        }
                    }
                }
            }
        }
        path_text
    }

    /// Binding name(s) for granular import tracking: last path segment, or `as` alias.
    fn extract_use_import_binding_names(&self, node: &Node) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "use" | ";" | "visibility_modifier" => {}
                "use_as_clause" => {
                    if let Some(alias) = self.use_as_clause_alias(&child) {
                        names.push(alias);
                    }
                }
                "use_list" | "scoped_use_list" => {
                    names.extend(self.extract_use_list_binding_names(&child));
                }
                "scoped_identifier" | "identifier" => {
                    if let Some(seg) = self.last_path_segment(&child) {
                        names.push(seg);
                    }
                }
                _ => {}
            }
        }
        names
    }

    fn extract_use_list_binding_names(&self, node: &Node) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "use_as_clause" => {
                    if let Some(alias) = self.use_as_clause_alias(&child) {
                        names.push(alias);
                    }
                }
                "use_list" | "scoped_use_list" => {
                    names.extend(self.extract_use_list_binding_names(&child));
                }
                "scoped_identifier" | "identifier" => {
                    if let Some(seg) = self.last_path_segment(&child) {
                        names.push(seg);
                    }
                }
                _ => {}
            }
        }
        names
    }

    fn use_as_clause_alias(&self, node: &Node) -> Option<String> {
        let mut saw_as = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "as" => saw_as = true,
                "identifier" if saw_as => return self.source_text(&child),
                _ => {}
            }
        }
        None
    }

    fn last_path_segment(&self, node: &Node) -> Option<String> {
        match node.kind() {
            "identifier" => self.source_text(node),
            "scoped_identifier" => {
                if let Some(name) = node.child_by_field_name("name") {
                    return self.source_text(&name);
                }
                let mut last = None;
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        last = self.source_text(&child);
                    } else if child.kind() == "scoped_identifier" {
                        last = self.last_path_segment(&child);
                    }
                }
                last
            }
            _ => None,
        }
    }

    fn first_path_segment(&self, node: &Node) -> Option<String> {
        match node.kind() {
            "identifier" | "crate" | "super" | "self" => self.source_text(node),
            "scoped_identifier" => {
                if let Some(path) = node.child_by_field_name("path") {
                    return self.first_path_segment(&path);
                }
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if matches!(
                        child.kind(),
                        "identifier" | "scoped_identifier" | "crate" | "super" | "self"
                    ) {
                        return self.first_path_segment(&child);
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn track_import_usage_if_imported(&mut self, name: &str) {
        if self.base.file_imports.contains(name) {
            self.base.track_callable_import_usage(name);
        }
    }

    fn capture_use_as_clause(&mut self, node: &Node) {
        let mut path_text = None;
        let mut alias = None;
        let mut saw_as = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "as" => saw_as = true,
                "identifier" if saw_as => {
                    alias = self.source_text(&child);
                }
                "scoped_identifier" | "identifier" if !saw_as => {
                    path_text = self.source_text(&child);
                }
                _ => {
                    if !saw_as && path_text.is_none() {
                        if let Some(t) = self.source_text(&child) {
                            if child.kind() != "as" {
                                path_text = Some(t);
                            }
                        }
                    }
                }
            }
        }
        if let (Some(alias), Some(path)) = (alias, path_text) {
            self.use_aliases.insert(alias, path);
        }
    }

    // ------------------------------------------------------------------
    // Types (contracts — always retained)
    // ------------------------------------------------------------------

    fn visit_type_like(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        if children.is_empty() {
            if let Some(text) = self.source_text(node) {
                return Ok(Some(TokenStream::node(node.kind(), vec![TokenStream::text(text)])));
            }
            return Ok(Some(TokenStream::node(node.kind(), vec![])));
        }
        Ok(Some(TokenStream::node(node.kind(), children)))
    }

    fn visit_primitive_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::node("primitive_type", vec![TokenStream::text(text)])));
        }
        Ok(Some(TokenStream::node("primitive_type", vec![])))
    }

    fn visit_mutable_specifier(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(Some(TokenStream::node(
            "mutable_specifier",
            vec![TokenStream::text("mut".to_string())],
        )))
    }

    fn visit_lifetime(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Keep lifetime as written (e.g. 'a) — contract.
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::node("lifetime", vec![TokenStream::text(text)])));
        }
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("lifetime", children)))
    }

    // ------------------------------------------------------------------
    // Parameters / functions
    // ------------------------------------------------------------------

    fn visit_parameter(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();

        // Retain mutable_specifier and type children as contracts.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "mutable_specifier" => {
                    if let Some(stream) = self.visit_mutable_specifier(&child)? {
                        children_streams.push(stream);
                    }
                }
                k if Self::is_type_node_kind(k) || k == "reference_type" => {
                    if let Some(stream) = self.visit(&child)? {
                        children_streams.push(stream);
                    }
                }
                "identifier" => {
                    let text = self.source_text(&child).unwrap_or_default();
                    let normalized = self.register_arg_if_new(&text);
                    children_streams.push(TokenStream::identifier(normalized));
                }
                ":" | "," => {}
                _ => {
                    // Patterns, etc.
                    if child.kind() != "identifier" {
                        if let Some(stream) = self.visit(&child)? {
                            if TokenStream::has_content(&stream) {
                                children_streams.push(stream);
                            }
                        }
                    }
                }
            }
        }

        Ok(Some(TokenStream::node("parameter", children_streams)))
    }

    fn visit_self_parameter(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Keep self/&self/&mut self structure; never rename `self`.
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("self_parameter", children)))
    }

    fn visit_parameters(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.set_in_parameter_list(true);
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "parameter" => {
                    if let Some(stream) = self.visit_parameter(&child)? {
                        children_streams.push(stream);
                    }
                }
                "self_parameter" => {
                    if let Some(stream) = self.visit_self_parameter(&child)? {
                        children_streams.push(stream);
                    }
                }
                _ => {}
            }
        }
        self.set_in_parameter_list(false);
        Ok(Some(TokenStream::node("parameters", children_streams)))
    }

    fn visit_function_item(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_function_name(node);
        let node_type = if self.is_inside_impl() { "method" } else { "function" };
        self.base
            .begin_callable(name, node_type, node.start_byte(), node.end_byte());

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);

        self.base
            .preflight_callable_scope(&RustScopeAnalyzer, node, self.source);

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        for child in &children {
            match child.kind() {
                "fn" => continue,
                "line_comment" | "block_comment" => {
                    let _ = self.visit(child)?;
                    continue;
                }
                "visibility_modifier" | "function_modifiers" => {
                    if let Some(stream) = self.visit(child)? {
                        children_streams.push(stream);
                    }
                }
                "identifier" => {
                    // Function name — keep as-is.
                    if let Some(text) = self.source_text(child) {
                        children_streams.push(TokenStream::identifier(text));
                    }
                }
                "type_parameters" => {
                    if let Some(stream) = self.visit_type_like(child)? {
                        children_streams.push(stream);
                    }
                }
                "parameters" => {
                    if let Some(stream) = self.visit_parameters(child)? {
                        children_streams.push(stream);
                    }
                }
                "->" => continue,
                k if Self::is_type_node_kind(k) || k == "reference_type" => {
                    // Return type — contract.
                    if let Some(stream) = self.visit(child)? {
                        children_streams.push(stream);
                    }
                }
                "block" => {
                    if let Some(stream) = self.visit_block(child)? {
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
            Ok(Some(TokenStream::node("function_item", vec![])))
        } else {
            Ok(Some(TokenStream::node("function_item", children_streams)))
        }
    }

    fn extract_function_name(&self, node: &Node) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                if let Some(text) = self.source_text(&child) {
                    return text;
                }
            }
        }
        "<anonymous>".to_string()
    }

    fn is_inside_impl(&self) -> bool {
        self.base.state.scope_stack.iter().any(|s| s == "impl")
    }

    /// Parameters + body block only — exclude the fn name so alpha-renamed bodies match.
    fn visit_function_modifiers(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Retain async/const/unsafe modifiers as written.
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(text) = self.source_text(&child) {
                if !text.is_empty() {
                    names.push(text);
                }
            }
        }
        names.sort();
        let children: Vec<_> = names
            .into_iter()
            .map(|n| TokenStream::node("modifier", vec![TokenStream::text(n)]))
            .collect();
        Ok(Some(TokenStream::node("function_modifiers", children)))
    }

    fn visit_visibility_modifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::node(
                "visibility_modifier",
                vec![TokenStream::text(text)],
            )));
        }
        Ok(Some(TokenStream::node("visibility_modifier", vec![])))
    }

    // ------------------------------------------------------------------
    // Impl / mod / trait / blocks
    // ------------------------------------------------------------------

    fn visit_impl_item(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let impl_name = self.extract_impl_type_name(node).unwrap_or_else(|| "@impl".to_string());
        self.base.push_definition_scope(impl_name);
        self.base.push_type_body();
        self.base.enter_scope("impl");
        let children = self.visit_children_skipping_punctuation(node)?;
        self.base.exit_scope();
        self.base.pop_type_body();
        self.base.pop_definition_scope();
        Ok(Some(TokenStream::node("impl_item", children)))
    }

    fn visit_mod_item(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mod_name = self.extract_function_name(node);
        self.base.push_definition_scope(mod_name);
        // mod is namespace scope for FQN, not a type body for LogicKind.
        self.base.enter_scope("mod");
        let children = self.visit_children_skipping_punctuation(node)?;
        self.base.exit_scope();
        self.base.pop_definition_scope();
        Ok(Some(TokenStream::node("mod_item", children)))
    }

    fn visit_trait_item(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let trait_name = self.extract_function_name(node);
        self.base.push_definition_scope(trait_name);
        self.base.push_type_body();
        self.base.enter_scope("trait");
        let children = self.visit_children_skipping_punctuation(node)?;
        self.base.exit_scope();
        self.base.pop_type_body();
        self.base.pop_definition_scope();
        Ok(Some(TokenStream::node("trait_item", children)))
    }

    fn extract_impl_type_name(&self, node: &Node) -> Option<String> {
        // Prefer `type` field (impl Type / impl Trait for Type).
        if let Some(ty) = node.child_by_field_name("type") {
            let mut cursor = ty.walk();
            for child in ty.children(&mut cursor) {
                if matches!(child.kind(), "type_identifier" | "identifier") {
                    if let Some(t) = self.source_text(&child) {
                        if !t.is_empty() {
                            return Some(t);
                        }
                    }
                }
            }
            if let Some(t) = self.source_text(&ty) {
                let cleaned = t
                    .split_whitespace()
                    .next()?
                    .trim_matches(|c: char| c == '&' || c == '<');
                if !cleaned.is_empty() {
                    return Some(cleaned.to_string());
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "type_identifier" | "generic_type") {
                if let Some(t) = self.source_text(&child) {
                    let cleaned = t.split('<').next().unwrap_or(&t).trim();
                    if !cleaned.is_empty() {
                        return Some(cleaned.to_string());
                    }
                }
            }
        }
        None
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

    fn visit_unsafe_block(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Retain unsafe — structural difference from plain block.
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("unsafe_block", children)))
    }

    // ------------------------------------------------------------------
    // Let / match / control flow
    // ------------------------------------------------------------------

    fn visit_let_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "let" | "=" | ";" | ":") {
                continue;
            }
            if child.kind() == "identifier" {
                let text = self.source_text(&child).unwrap_or_default();
                children_streams.push(TokenStream::identifier(text));
            } else if child.kind() == "mutable_specifier" {
                if let Some(stream) = self.visit_mutable_specifier(&child)? {
                    children_streams.push(stream);
                }
            } else if let Some(stream) = self.visit(&child)? {
                if TokenStream::has_content(&stream) {
                    children_streams.push(stream);
                }
            }
        }
        Ok(Some(TokenStream::node("let_declaration", children_streams)))
    }

    fn visit_match_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("match_expression", children)))
    }

    fn visit_match_arm(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Bind pattern identifiers as vars before arm body.
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("match_arm", children)))
    }

    fn visit_for_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Register loop variable before visiting body.
        self.visit_generic(node)
    }

    // ------------------------------------------------------------------
    // Expressions / literals / identifiers
    // ------------------------------------------------------------------

    fn visit_binary_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Keep operator tokens so + vs - differ.
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("binary_expression", children)))
        }
    }

    fn visit_call_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Syntactic callee text of the function child (no IR change).
        if let Some(func) = node.child_by_field_name("function") {
            if let Some(callee) = self.source_text(&func) {
                if !callee.is_empty() {
                    self.base.track_call_node(callee, node, self.source);
                }
            }
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "arguments" {
                    break;
                }
                if matches!(child.kind(), "(" | ")" | "::" | ".") {
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
        Ok(Some(TokenStream::node("call_expression", children)))
    }

    fn visit_field_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("field_expression", children)))
    }

    fn visit_arguments(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children_skipping_punctuation(node)?;
        Ok(Some(TokenStream::node("arguments", children)))
    }

    fn visit_string_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            Ok(Some(TokenStream::literal("string_literal", text)))
        } else {
            Ok(Some(TokenStream::node("string_literal", vec![])))
        }
    }

    fn visit_field_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Field names are not locals — keep as-is (unless somehow bound, which is rare).
        let text = self.source_text(node).unwrap_or_default();
        Ok(Some(TokenStream::identifier(text)))
    }

    fn visit_type_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let text = self.source_text(node).unwrap_or_default();
        if text == "Self" {
            return Ok(Some(TokenStream::identifier("Self".to_string())));
        }
        let resolved = self.resolve_alias(&text);
        Ok(Some(TokenStream::identifier(resolved)))
    }

    fn visit_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let identifier_text = self.source_text(node).unwrap_or_default();

        if Self::is_reserved_unrenamed(&identifier_text) {
            return Ok(Some(TokenStream::identifier(identifier_text)));
        }

        if self.var_maps().is_empty() || self.in_parameter_list() {
            self.track_import_usage_if_imported(&identifier_text);
            let resolved = self.resolve_alias(&identifier_text);
            return Ok(Some(TokenStream::identifier(resolved)));
        }

        let is_bound_variable = self.is_identifier_bound(&identifier_text);
        if !is_bound_variable {
            self.track_import_usage_if_imported(&identifier_text);
            let resolved = self.resolve_alias(&identifier_text);
            return Ok(Some(TokenStream::identifier(resolved)));
        }

        Ok(Some(TokenStream::identifier(identifier_text)))
    }

    fn visit_scoped_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(first) = self.first_path_segment(node) {
            self.track_import_usage_if_imported(&first);
        }
        self.visit_generic(node)
    }

    fn visit_self_token(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let text = self.source_text(node).unwrap_or_else(|| "self".to_string());
        Ok(Some(TokenStream::identifier(text)))
    }

    fn dispatch_visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "line_comment" => self.visit_line_comment(node),
            "block_comment" => self.visit_block_comment(node),
            "attribute_item" => self.visit_attribute_item(node),
            "attribute" => self.visit_attribute(node),
            "use_declaration" => self.visit_use_declaration(node),
            "source_file" => self.visit_simple_named(node, "source_file"),
            "function_item" => self.visit_function_item(node),
            "function_modifiers" => self.visit_function_modifiers(node),
            "visibility_modifier" => self.visit_visibility_modifier(node),
            "parameters" => self.visit_parameters(node),
            "parameter" => self.visit_parameter(node),
            "self_parameter" => self.visit_self_parameter(node),
            "impl_item" => self.visit_impl_item(node),
            "mod_item" => self.visit_mod_item(node),
            "trait_item" => self.visit_trait_item(node),
            "declaration_list" => self.visit_declaration_list(node),
            "block" => self.visit_block(node),
            "unsafe_block" => self.visit_unsafe_block(node),
            "let_declaration" => self.visit_let_declaration(node),
            "match_expression" => self.visit_match_expression(node),
            "match_block" => self.visit_simple_named(node, "match_block"),
            "match_arm" => self.visit_match_arm(node),
            "match_pattern" => self.visit_simple_named(node, "match_pattern"),
            "for_expression" => self.visit_for_expression(node),
            "while_expression" | "loop_expression" | "if_expression" => self.visit_generic(node),
            "primitive_type" => self.visit_primitive_type(node),
            "mutable_specifier" => self.visit_mutable_specifier(node),
            "lifetime" => self.visit_lifetime(node),
            "lifetime_parameter"
            | "type_parameters"
            | "type_parameter"
            | "type_arguments"
            | "reference_type"
            | "array_type"
            | "generic_type"
            | "tuple_type"
            | "unit_type"
            | "pointer_type"
            | "function_type"
            | "dyn_type"
            | "bounded_type"
            | "scoped_type_identifier" => self.visit_type_like(node),
            "type_identifier" => self.visit_type_identifier(node),
            "call_expression" => self.visit_call_expression(node),
            "field_expression" => self.visit_field_expression(node),
            "arguments" => self.visit_arguments(node),
            "binary_expression" => self.visit_binary_expression(node),
            "type_cast_expression" | "compound_assignment_expr" => self.visit_generic(node),
            "scoped_identifier" => self.visit_scoped_identifier(node),
            "string_literal" | "raw_string_literal" => self.visit_string_literal(node),
            "integer_literal" => Ok(self.normalize_integer_literal_node(node)),
            "field_identifier" => self.visit_field_identifier(node),
            "identifier" => self.visit_identifier(node),
            "self" => self.visit_self_token(node),
            // No visit_ERROR — hasher rejects ERROR for Rust (fail closed)
            _ => self.visit_generic(node),
        }
    }
}

impl<'a> NodeVisitor for RustNodeVisitor<'a> {
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
