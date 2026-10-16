//! Java AST visitor for structured serialization.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::serialization::TokenStream;
use crate::visitor::{BaseVisitor, NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::normalizer::JavaNormalizer;
use super::scope_analyzer::JavaScopeAnalyzer;

const JAVA_FIELD_PREFIXES_TO_STRIP: &[&str] = &["this", "super"];

/// Visitor for Java AST nodes using structured serialization.
pub struct JavaNodeVisitor<'a> {
    pub base: BaseVisitor,
    pub source: &'a [u8],
    pub normalizer: Option<&'a JavaNormalizer>,
    pub static_imports: HashMap<String, String>,
    pub regular_imports: HashMap<String, String>,
    pub field_maps: Vec<HashSet<String>>,
    pub current_parent_type: Option<String>,
    type_param_map: HashMap<String, String>,
    type_param_map_stack: Vec<HashMap<String, String>>,
}

impl<'a> JavaNodeVisitor<'a> {
    pub fn new(
        policy: Option<StructuralHashPolicy>,
        source: &'a [u8],
        normalizer: Option<&'a JavaNormalizer>,
        static_imports: HashMap<String, String>,
        regular_imports: HashMap<String, String>,
    ) -> Self {
        Self {
            base: BaseVisitor::new(None, policy),
            source,
            normalizer,
            static_imports,
            regular_imports,
            field_maps: Vec::new(),
            current_parent_type: None,
            type_param_map: HashMap::new(),
            type_param_map_stack: Vec::new(),
        }
    }

    fn node_text(&self, node: &Node) -> Option<String> {
        node.utf8_text(self.source).ok().map(str::to_string)
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

    fn current_arg_map_mut(&mut self) -> &mut HashMap<String, String> {
        self.base.state.current_arg_map_mut()
    }

    fn should_strip_prefix(&self, identifier_text: &str) -> bool {
        JAVA_FIELD_PREFIXES_TO_STRIP.contains(&identifier_text)
    }

    fn qualify_first_segment(&self, parts: &mut [String]) {
        if let Some(first) = parts.first_mut() {
            if let Some(normalizer) = self.normalizer {
                *first = normalizer.qualify_java_lang_identifier(first, &self.regular_imports);
            }
        }
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
            if field_name.map(|f| body_fields.contains(&f)).unwrap_or(false) {
                if let Some(normalizer) = self.normalizer {
                    // Safety: normalizer is separate from visitor borrow via reborrow pattern
                    let wrapped = {
                        let norm = normalizer;
                        // Can't call wrap_block_child with self while holding normalizer easily;
                        // inline the wrap logic here instead.
                        let child_stream = self.visit(child)?;
                        let Some(child_stream) = child_stream else {
                            continue;
                        };
                        if child.kind() == "block" {
                            Some(child_stream)
                        } else {
                            let parent_type = self.current_parent_type.as_deref().unwrap_or("control_flow");
                            if norm.should_wrap_child(child, parent_type) {
                                Some(TokenStream::node("block", vec![child_stream]))
                            } else {
                                Some(child_stream)
                            }
                        }
                    };
                    if let Some(wrapped) = wrapped {
                        children_streams.push(wrapped);
                    }
                }
            } else if let Some(child_stream) = self.visit(child)? {
                children_streams.push(child_stream);
            }
        }

        self.current_parent_type = None;

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node(node_type, vec![])))
        } else {
            Ok(Some(TokenStream::node(node_type, children_streams)))
        }
    }

    fn extract_field_names_from_declaration(&self, node: &Node) -> HashSet<String> {
        let mut names = HashSet::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let mut dc = child.walk();
                for declarator_child in child.children(&mut dc) {
                    if declarator_child.kind() == "identifier" {
                        if let Some(text) = self.source_text(&declarator_child) {
                            names.insert(text);
                        }
                        break;
                    }
                }
            }
        }
        names
    }

    fn collect_class_field_names(&self, class_node: &Node) -> HashSet<String> {
        let mut names = HashSet::new();
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() == "class_body" {
                let mut bc = child.walk();
                for body_child in child.children(&mut bc) {
                    if body_child.kind() == "field_declaration" {
                        names.extend(self.extract_field_names_from_declaration(&body_child));
                    }
                }
            }
        }
        names
    }

    // ------------------------------------------------------------------
    // Comments / annotations / modifiers
    // ------------------------------------------------------------------

    fn visit_line_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "line_comment")
    }

    fn visit_block_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "block_comment")
    }

    fn visit_marker_annotation(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Prefer miss: keep annotations even when prune_type_hints (runtime-affecting).
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node("marker_annotation", vec![])))
        } else {
            Ok(Some(TokenStream::node("marker_annotation", children)))
        }
    }

    fn visit_annotation(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Prefer miss: keep annotations even when prune_type_hints (runtime-affecting).
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node("annotation", vec![])))
        } else {
            Ok(Some(TokenStream::node("annotation", children)))
        }
    }

    fn visit_modifiers(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        let mut streams = Vec::new();
        let mut modifier_nodes = Vec::new();

        for child in children {
            match child.kind() {
                "marker_annotation" | "annotation" => {
                    if let Some(stream) = self.visit(&child)? {
                        streams.push(stream);
                    }
                }
                _ => modifier_nodes.push(child),
            }
        }

        let modifier_names = if let Some(normalizer) = self.normalizer {
            normalizer.normalize_modifiers(&modifier_nodes, self.source)
        } else {
            Vec::new()
        };

        streams.extend(modifier_names.into_iter().map(TokenStream::text));

        if streams.is_empty() {
            return Ok(None);
        }

        Ok(Some(TokenStream::node("modifiers", streams)))
    }

    fn visit_dimensions(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    fn visit_semicolon(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Types
    // ------------------------------------------------------------------

    fn count_array_dimensions(&self, type_node: &Node) -> usize {
        let mut dimension_count = 0;
        let mut cursor = type_node.walk();
        for child in type_node.children(&mut cursor) {
            if child.kind() == "dimensions" {
                let mut dc = child.walk();
                for dim_child in child.children(&mut dc) {
                    if dim_child.kind() == "[" {
                        dimension_count += 1;
                    } else if let Some(text) = self.source_text(&dim_child) {
                        if text == "[" {
                            dimension_count += 1;
                        }
                    }
                }
            } else if child.kind() == "[" {
                dimension_count += 1;
            }
        }
        dimension_count
    }

    fn qualify_type_identifier(&self, node: &Node) -> Option<String> {
        let type_text = self.source_text(node)?;
        if type_text.is_empty() {
            return None;
        }
        if let Some(normalizer) = self.normalizer {
            Some(normalizer.qualify_java_lang_identifier(&type_text, &self.regular_imports))
        } else {
            Some(type_text)
        }
    }

    fn extract_type_from_type_argument(&self, node: &Node) -> Option<String> {
        let mut cursor = node.walk();
        for arg_child in node.children(&mut cursor) {
            match arg_child.kind() {
                "type_identifier" => return self.qualify_type_identifier(&arg_child),
                "scoped_type_identifier" => return self.source_text(&arg_child),
                "generic_type" => return self.extract_generic_type_with_arguments(&arg_child),
                "integral_type" => return self.source_text(&arg_child),
                _ => {}
            }
        }
        None
    }

    fn extract_type_arguments(&self, node: &Node) -> Vec<String> {
        let mut type_args = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_argument" => {
                    if let Some(arg) = self.extract_type_from_type_argument(&child) {
                        type_args.push(arg);
                    }
                }
                "type_identifier" => {
                    if let Some(arg) = self.qualify_type_identifier(&child) {
                        type_args.push(arg);
                    }
                }
                "<" | ">" | "," => {}
                "scoped_type_identifier" => {
                    if let Some(text) = self.source_text(&child) {
                        type_args.push(text);
                    }
                }
                "generic_type" => {
                    if let Some(nested) = self.extract_generic_type_with_arguments(&child) {
                        type_args.push(nested);
                    }
                }
                _ => {}
            }
        }
        type_args
    }

    fn extract_generic_type_with_arguments(&self, node: &Node) -> Option<String> {
        let mut base_type = None;
        let mut type_arguments = Vec::new();

        let mut cursor = node.walk();
        for generic_child in node.children(&mut cursor) {
            match generic_child.kind() {
                "type_identifier" => {
                    base_type = self.qualify_type_identifier(&generic_child);
                }
                "scoped_type_identifier" => {
                    let mut type_text = self.source_text(&generic_child).unwrap_or_default();
                    if !type_text.is_empty() {
                        if let Some(normalizer) = self.normalizer {
                            let mut parts: Vec<String> = type_text.split('.').map(str::to_string).collect();
                            if let Some(first) = parts.first_mut() {
                                *first = normalizer.qualify_java_lang_identifier(first, &self.regular_imports);
                            }
                            type_text = parts.join(".");
                        }
                        base_type = Some(type_text);
                    }
                    let mut sc = generic_child.walk();
                    for scoped_child in generic_child.children(&mut sc) {
                        if scoped_child.kind() == "type_arguments" {
                            let mut ac = scoped_child.walk();
                            for arg_child in scoped_child.children(&mut ac) {
                                if arg_child.kind() == "type_argument" {
                                    if let Some(arg) = self.extract_type_from_type_argument(&arg_child) {
                                        type_arguments.push(arg);
                                    }
                                }
                            }
                        }
                    }
                }
                "type_arguments" => {
                    type_arguments.extend(self.extract_type_arguments(&generic_child));
                }
                _ => {}
            }
        }

        let base_type = base_type?;
        if type_arguments.is_empty() {
            return Some(base_type);
        }

        let qualified_args: Vec<String> = type_arguments
            .into_iter()
            .map(|arg| {
                if !arg.contains('.') {
                    if let Some(normalizer) = self.normalizer {
                        return normalizer.qualify_java_lang_identifier(&arg, &self.regular_imports);
                    }
                }
                arg
            })
            .collect();
        Some(format!("{base_type}<{}>", qualified_args.join(",")))
    }

    fn extract_type_from_node(&self, node: &Node) -> Option<String> {
        match node.kind() {
            "type_identifier" => {
                let identifier_text = self.source_text(node).unwrap_or_default();
                if let Some(normalized) = self.type_param_map.get(&identifier_text) {
                    return Some(normalized.clone());
                }
                self.qualify_type_identifier(node)
            }
            "integral_type" | "floating_point_type" => self.source_text(node),
            "scoped_type_identifier" => {
                let type_text = self.source_text(node)?;
                if let Some(parent) = node.parent() {
                    if parent.kind() == "generic_type" {
                        return self.extract_generic_type_with_arguments(&parent);
                    }
                }
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "type_arguments" {
                        let type_args = self.extract_type_arguments(&child);
                        if !type_args.is_empty() {
                            return Some(format!("{type_text}<{}>", type_args.join(",")));
                        }
                    }
                }
                Some(type_text)
            }
            "generic_type" => self.extract_generic_type_with_arguments(node),
            _ => None,
        }
    }

    fn extract_parameter_type(&self, param: &Node) -> Option<String> {
        let mut type_node = None;
        let mut type_arguments_node = None;

        let mut cursor = param.walk();
        let children: Vec<Node> = param.children(&mut cursor).collect();

        for param_child in &children {
            match param_child.kind() {
                "generic_type" => {
                    let base_type = self.extract_generic_type_with_arguments(param_child)?;
                    let dimensions = self.count_array_dimensions(param);
                    if dimensions > 0 {
                        return Some(format!("{base_type}{}", "[]".repeat(dimensions)));
                    }
                    return Some(base_type);
                }
                "array_type" => {
                    let mut base_type = None;
                    let mut ac = param_child.walk();
                    for array_child in param_child.children(&mut ac) {
                        if array_child.kind() == "generic_type" {
                            base_type = self.extract_generic_type_with_arguments(&array_child);
                            break;
                        } else if matches!(
                            array_child.kind(),
                            "type_identifier"
                                | "scoped_type_identifier"
                                | "integral_type"
                                | "floating_point_type"
                                | "boolean_type"
                                | "void_type"
                        ) {
                            base_type = self.extract_type_from_node(&array_child);
                            break;
                        }
                    }
                    if let Some(base_type) = base_type {
                        let dimensions = self.count_array_dimensions(param_child);
                        if dimensions > 0 {
                            return Some(format!("{base_type}{}", "[]".repeat(dimensions)));
                        }
                        return Some(base_type);
                    }
                }
                "type_identifier"
                | "scoped_type_identifier"
                | "integral_type"
                | "floating_point_type"
                | "boolean_type"
                | "void_type" => {
                    type_node = Some(*param_child);
                }
                "type_arguments" => {
                    type_arguments_node = Some(*param_child);
                }
                _ => {}
            }
        }

        if let (Some(type_node), Some(type_arguments_node)) = (type_node, type_arguments_node) {
            if let Some(base_type) = self.extract_type_from_node(&type_node) {
                let type_args = self.extract_type_arguments(&type_arguments_node);
                let result_type = if type_args.is_empty() {
                    base_type
                } else {
                    let qualified_args: Vec<String> = type_args
                        .into_iter()
                        .map(|arg| {
                            if !arg.contains('.') {
                                if let Some(normalizer) = self.normalizer {
                                    return normalizer.qualify_java_lang_identifier(&arg, &self.regular_imports);
                                }
                            }
                            arg
                        })
                        .collect();
                    format!("{base_type}<{}>", qualified_args.join(","))
                };
                let dimensions = self.count_array_dimensions(param);
                if dimensions > 0 {
                    return Some(format!("{result_type}{}", "[]".repeat(dimensions)));
                }
                return Some(result_type);
            }
        }

        if let Some(type_node) = type_node {
            if type_node.kind() == "scoped_type_identifier" {
                let mut sc = type_node.walk();
                for scoped_child in type_node.children(&mut sc) {
                    if scoped_child.kind() == "type_arguments" {
                        let base_type = self.source_text(&type_node).unwrap_or_default();
                        if !base_type.is_empty() {
                            let type_args = self.extract_type_arguments(&scoped_child);
                            let result_type = if type_args.is_empty() {
                                base_type
                            } else {
                                format!("{base_type}<{}>", type_args.join(","))
                            };
                            let dimensions = self.count_array_dimensions(param);
                            if dimensions > 0 {
                                return Some(format!("{result_type}{}", "[]".repeat(dimensions)));
                            }
                            return Some(result_type);
                        }
                    }
                }
            }

            if type_arguments_node.is_some() {
                return None;
            }

            if let Some(base_type) = self.extract_type_from_node(&type_node) {
                let dimensions = self.count_array_dimensions(param);
                if dimensions > 0 {
                    return Some(format!("{base_type}{}", "[]".repeat(dimensions)));
                }
                return Some(base_type);
            }
        }

        for param_child in &children {
            if matches!(
                param_child.kind(),
                "integral_type" | "floating_point_type" | "boolean_type" | "void_type"
            ) {
                if let Some(base_type) = self.source_text(param_child) {
                    let dimensions = self.count_array_dimensions(param);
                    if dimensions > 0 {
                        return Some(format!("{base_type}{}", "[]".repeat(dimensions)));
                    }
                    return Some(base_type);
                }
            }
        }

        let mut type_parts = Vec::new();
        for param_child in &children {
            if matches!(
                param_child.kind(),
                "identifier" | "modifiers" | "dimensions" | "annotation" | "marker_annotation" | "type_arguments"
            ) {
                continue;
            }
            if let Some(part) = self.extract_type_from_node(param_child) {
                type_parts.push(part);
            }
        }

        if type_parts.is_empty() {
            return None;
        }

        let result_type = match type_parts.as_slice() {
            [only] => only.clone(),
            _ => type_parts.join("."),
        };
        let dimensions = self.count_array_dimensions(param);
        if dimensions > 0 {
            return Some(format!("{result_type}{}", "[]".repeat(dimensions)));
        }
        Some(result_type)
    }

    fn visit_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Types are logical contracts — always include
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node("type", vec![])))
        } else {
            Ok(Some(TokenStream::node("type", children)))
        }
    }

    fn visit_return_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut return_type_info = self.extract_type_from_node(node);
        if let Some(ref mut info) = return_type_info {
            let dimensions = self.count_array_dimensions(node);
            if dimensions > 0 {
                *info = format!("{info}{}", "[]".repeat(dimensions));
            }
            return Ok(Some(TokenStream::literal("return_type", info.clone())));
        }
        Ok(None)
    }

    fn visit_type_parameters(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut type_param_counter = 0usize;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_parameter" {
                let mut pc = child.walk();
                for param_child in child.children(&mut pc) {
                    if param_child.kind() == "type_identifier" {
                        if let Some(param_name) = self.source_text(&param_child) {
                            if !param_name.is_empty() {
                                let normalized = format!("type_param_{type_param_counter}");
                                self.type_param_map.insert(param_name, normalized);
                                type_param_counter += 1;
                            }
                        }
                        break;
                    }
                }
            }
        }
        // Type parameters are pruned (cosmetic)
        Ok(None)
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

    fn infer_type_from_initializer(&self, initializer: &Node) -> Option<&'static str> {
        match initializer.kind() {
            "decimal_integer_literal" | "hex_integer_literal" | "octal_integer_literal" | "binary_integer_literal" => {
                Some("int")
            }
            "floating_point_literal" => {
                let text = self.source_text(initializer).unwrap_or_default();
                if text.ends_with('f') || text.ends_with('F') {
                    Some("float")
                } else {
                    Some("double")
                }
            }
            "character_literal" => Some("char"),
            "string_literal" => Some("String"),
            "boolean_literal" => Some("boolean"),
            _ => None,
        }
    }

    fn visit_type_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let identifier_text = self.source_text(node).unwrap_or_default();
        if identifier_text == "var" {
            if let Some(parent) = node.parent() {
                if parent.kind() == "local_variable_declaration" {
                    let mut cursor = parent.walk();
                    for sibling in parent.children(&mut cursor) {
                        if sibling.kind() == "variable_declarator" {
                            let mut dc = sibling.walk();
                            let decl_children: Vec<Node> = sibling.children(&mut dc).collect();
                            for (i, declarator_child) in decl_children.iter().enumerate() {
                                if declarator_child.kind() == "=" && i + 1 < decl_children.len() {
                                    let initializer = &decl_children[i + 1];
                                    if let Some(inferred) = self.infer_type_from_initializer(initializer) {
                                        return Ok(Some(TokenStream::literal("integral_type", inferred)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return Ok(None);
        }

        if let Some(normalized) = self.type_param_map.get(&identifier_text) {
            return Ok(Some(TokenStream::identifier(normalized.clone())));
        }

        // Types in parameters handled specially; skip elsewhere
        Ok(None)
    }

    fn visit_integral_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("integral_type", text)));
        }
        Ok(Some(TokenStream::node("integral_type", vec![])))
    }

    fn visit_floating_point_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("floating_point_type", text)));
        }
        Ok(Some(TokenStream::node("floating_point_type", vec![])))
    }

    fn visit_array_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "dimensions" {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else if children_streams.len() == 1 {
            Ok(Some(children_streams.remove(0)))
        } else {
            Ok(Some(TokenStream::node("array_type", children_streams)))
        }
    }

    fn visit_scoped_type_identifier(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Catch / object creation / imports
    // ------------------------------------------------------------------

    fn extract_exception_type(&self, param_child: &Node) -> Option<TokenStream> {
        match param_child.kind() {
            "catch_type" => {
                let mut cursor = param_child.walk();
                for type_child in param_child.children(&mut cursor) {
                    if type_child.kind() == "type_identifier" {
                        let type_text = self.source_text(&type_child).unwrap_or_default();
                        return Some(TokenStream::literal("exception_type", type_text));
                    }
                }
                None
            }
            "type_identifier" => {
                let type_text = self.source_text(param_child).unwrap_or_default();
                Some(TokenStream::literal("exception_type", type_text))
            }
            _ => None,
        }
    }

    fn visit_catch_clause(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut catch_parts = Vec::new();
        let mut cursor = node.walk();
        for catch_child in node.children(&mut cursor) {
            if catch_child.kind() == "catch_formal_parameter" {
                let mut pc = catch_child.walk();
                for param_child in catch_child.children(&mut pc) {
                    if let Some(exception_type) = self.extract_exception_type(&param_child) {
                        catch_parts.push(exception_type);
                    } else if !matches!(param_child.kind(), "modifiers" | "dimensions" | "identifier") {
                        if let Some(param_serialized) = self.visit(&param_child)? {
                            catch_parts.push(param_serialized);
                        }
                    }
                }
            } else if let Some(catch_serialized) = self.visit(&catch_child)? {
                catch_parts.push(catch_serialized);
            }
        }
        if catch_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("catch_clause", catch_parts)))
        }
    }

    fn visit_catch_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = node.walk();
        for type_child in node.children(&mut cursor) {
            if type_child.kind() == "type_identifier" {
                let type_text = self.source_text(&type_child).unwrap_or_default();
                return Ok(Some(TokenStream::literal("exception_type", type_text)));
            }
        }
        Ok(None)
    }

    fn visit_object_creation_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut obj_parts = Vec::new();
        let mut cursor = node.walk();
        for obj_child in node.children(&mut cursor) {
            if obj_child.kind() == "type_identifier" {
                let mut type_text = self.source_text(&obj_child).unwrap_or_default();
                if let Some(normalizer) = self.normalizer {
                    type_text = normalizer.qualify_java_lang_identifier(&type_text, &self.regular_imports);
                }
                obj_parts.push(TokenStream::literal("class_name", type_text));
            } else if !matches!(obj_child.kind(), "type_arguments" | "argument_list") {
                if let Some(obj_serialized) = self.visit(&obj_child)? {
                    obj_parts.push(obj_serialized);
                }
            }
        }
        if obj_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("object_creation_expression", obj_parts)))
        }
    }

    fn extract_qualified_from_scoped(&self, n: &Node) -> Vec<String> {
        let mut parts = Vec::new();
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "identifier" {
                if let Some(text) = self.source_text(&child) {
                    parts.push(text);
                }
            } else if child.kind() == "scoped_identifier" {
                parts.extend(self.extract_qualified_from_scoped(&child));
            }
        }
        parts
    }

    fn track_import_names(&mut self, qualified_parts: &[String]) {
        if let Some(simple) = qualified_parts.last() {
            if !simple.is_empty() {
                self.base.track_file_import(simple.clone());
            }
        }
        if !qualified_parts.is_empty() {
            self.base.track_file_import(qualified_parts.join("."));
        }
    }

    fn process_static_import(&mut self, scoped_id_node: &Node) {
        let qualified_parts = self.extract_qualified_from_scoped(scoped_id_node);
        if let Some(identifier) = qualified_parts.last() {
            if !identifier.is_empty() {
                let qualified_path = if qualified_parts.len() > 1 {
                    qualified_parts[..qualified_parts.len() - 1].join(".")
                } else {
                    String::new()
                };
                let value = if qualified_path.is_empty() {
                    identifier.clone()
                } else {
                    format!("{qualified_path}.{identifier}")
                };
                self.static_imports.insert(identifier.clone(), value);
            }
        }
    }

    fn process_regular_import(&mut self, scoped_id_node: &Node) {
        let qualified_parts = self.extract_qualified_from_scoped(scoped_id_node);
        if let Some(identifier) = qualified_parts.last() {
            if !identifier.is_empty() {
                let qualified_path = if qualified_parts.len() > 1 {
                    qualified_parts[..qualified_parts.len() - 1].join(".")
                } else {
                    String::new()
                };
                let value = if qualified_path.is_empty() {
                    identifier.clone()
                } else {
                    format!("{qualified_path}.{identifier}")
                };
                self.regular_imports.insert(identifier.clone(), value);
                self.track_import_names(&qualified_parts);
            }
        }
    }

    fn visit_import_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut is_static = false;
        let mut is_star = false;
        let mut scoped_id_node = None;
        let mut identifier_node = None;
        let mut cursor = node.walk();
        for import_child in node.children(&mut cursor) {
            match import_child.kind() {
                "static" => is_static = true,
                "scoped_identifier" => scoped_id_node = Some(import_child),
                "identifier" => identifier_node = Some(import_child),
                "asterisk" | "*" => is_star = true,
                _ => {}
            }
        }
        // Grammar versions may represent `*` anonymously; source text is reliable.
        if !is_star {
            if let Some(text) = self.source_text(node) {
                is_star = text.contains(".*");
            }
        }

        let start = node.start_byte();
        let end = node.end_byte();

        if let Some(scoped_id_node) = scoped_id_node {
            let qualified_parts = self.extract_qualified_from_scoped(&scoped_id_node);
            let qualified_name = qualified_parts.join(".");

            if is_static {
                self.process_static_import(&scoped_id_node);
                let kind = if is_star {
                    entmoot_contracts::ImportKind::Star
                } else {
                    entmoot_contracts::ImportKind::Static
                };
                if !qualified_name.is_empty() {
                    self.base.track_import_marker(qualified_name, None, kind, start, end);
                }
                return Ok(None);
            }

            self.process_regular_import(&scoped_id_node);
            if !qualified_name.is_empty() {
                let kind = if is_star {
                    entmoot_contracts::ImportKind::Star
                } else {
                    entmoot_contracts::ImportKind::Named
                };
                self.base
                    .track_import_marker(qualified_name.clone(), None, kind, start, end);
                let import_token = TokenStream::literal("import", qualified_name);
                return Ok(Some(TokenStream::node("import_declaration", vec![import_token])));
            }
        } else if is_star {
            // `import pkg.*;` — package is a bare identifier, not scoped_identifier.
            if let Some(pkg) = identifier_node.and_then(|n| self.source_text(&n)) {
                let kind = if is_static {
                    entmoot_contracts::ImportKind::Static
                } else {
                    entmoot_contracts::ImportKind::Star
                };
                self.base.track_import_marker(pkg, None, kind, start, end);
            }
        }
        Ok(Some(TokenStream::node("import_declaration", vec![])))
    }

    // ------------------------------------------------------------------
    // Method / class / record / block / lambda
    // ------------------------------------------------------------------

    fn extract_callable_name(&self, node: &Node) -> String {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Some(t) = self.source_text(&name_node) {
                if !t.is_empty() {
                    return t;
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                if let Some(t) = self.source_text(&child) {
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
        }
        if node.kind() == "constructor_declaration" {
            return "<init>".to_string();
        }
        "<anonymous>".to_string()
    }

    /// Streams for callable hashing: parameters + body block only (exclude name / return type).
    fn process_formal_parameters_java(&mut self, child: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        if child.kind() != "formal_parameters" {
            return Ok(Vec::new());
        }

        let mut param_parts = Vec::new();
        let mut cursor = child.walk();
        for param in child.children(&mut cursor) {
            let Some(name) = super::scope_analyzer::parameter_name(&param) else {
                continue;
            };
            let param_type = self.extract_parameter_type(&param);
            let Some(identifier_text) = self.source_text(&name) else {
                continue;
            };
            if !self.arg_maps().is_empty() {
                let normalized_name = self
                    .base
                    .normalize_parameter_name_at(&identifier_text, Some(name.start_byte()));
                let param_value = if let Some(ref param_type) = param_type {
                    format!("{normalized_name}:{param_type}")
                } else {
                    normalized_name
                };
                param_parts.push(TokenStream::literal("parameter", param_value));
            }
        }
        Ok(param_parts)
    }

    fn visit_method_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_callable_name(node);
        self.base
            .begin_callable(name, "method", node.start_byte(), node.end_byte());

        self.base.enter_scope("method");
        self.set_in_parameter_list(false);

        self.base
            .preflight_callable_scope(&JavaScopeAnalyzer, node, self.source);

        let mut children_streams = Vec::new();
        let mut seen_return_type = false;

        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        for (child_index, child) in children.iter().enumerate() {
            if matches!(child.kind(), "line_comment" | "block_comment") {
                let _ = self.visit(child)?;
                continue;
            }

            if !seen_return_type
                && matches!(
                    child.kind(),
                    "return_type"
                        | "type"
                        | "type_identifier"
                        | "scoped_type_identifier"
                        | "integral_type"
                        | "floating_point_type"
                        | "void_type"
                        | "boolean_type"
                        | "array_type"
                )
                && child_index + 1 < children.len()
                && children[child_index + 1].kind() == "identifier"
            {
                let return_type_stream = match child.kind() {
                    "return_type" => self.visit_return_type(child)?,
                    "array_type" => {
                        let mut base_type_node = None;
                        let mut ac = child.walk();
                        for array_child in child.children(&mut ac) {
                            if matches!(
                                array_child.kind(),
                                "type_identifier"
                                    | "scoped_type_identifier"
                                    | "integral_type"
                                    | "floating_point_type"
                                    | "boolean_type"
                                    | "void_type"
                            ) {
                                base_type_node = Some(array_child);
                                break;
                            }
                        }
                        if let Some(base_type_node) = base_type_node {
                            if let Some(mut base_type) = self.extract_type_from_node(&base_type_node) {
                                let dimensions = self.count_array_dimensions(child);
                                if dimensions > 0 {
                                    base_type = format!("{base_type}{}", "[]".repeat(dimensions));
                                }
                                Some(TokenStream::literal("return_type", base_type))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => {
                        if let Some(mut return_type_info) = self.extract_type_from_node(child) {
                            let dimensions = self.count_array_dimensions(child);
                            if dimensions > 0 {
                                return_type_info = format!("{return_type_info}{}", "[]".repeat(dimensions));
                            }
                            Some(TokenStream::literal("return_type", return_type_info))
                        } else {
                            None
                        }
                    }
                };
                seen_return_type = true;
                if let Some(return_type_stream) = return_type_stream {
                    children_streams.push(return_type_stream);
                }
                continue;
            }

            if child.kind() == "formal_parameters" {
                self.set_in_parameter_list(true);
                let param_parts = self.process_formal_parameters_java(child)?;
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
            Ok(Some(TokenStream::node("method_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("method_declaration", children_streams)))
        }
    }

    fn visit_constructor_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_callable_name(node);
        self.base
            .begin_callable(name, "method", node.start_byte(), node.end_byte());

        self.base.enter_scope("method");
        self.set_in_parameter_list(false);

        self.base
            .preflight_callable_scope(&JavaScopeAnalyzer, node, self.source);

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "line_comment" | "block_comment") {
                let _ = self.visit(&child)?;
                continue;
            }

            if child.kind() == "formal_parameters" {
                self.set_in_parameter_list(true);
                let param_parts = self.process_formal_parameters_java(&child)?;
                children_streams.extend(param_parts);
                self.set_in_parameter_list(false);
                continue;
            }

            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }

        self.base.exit_callable_scope();
        self.base.exit_scope();

        self.base.end_callable();

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("constructor_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("constructor_declaration", children_streams)))
        }
    }

    fn visit_class_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if !self.type_param_map.is_empty() {
            self.type_param_map_stack.push(self.type_param_map.clone());
        }
        self.type_param_map.clear();

        let class_name = self.extract_callable_name(node);
        self.base.push_definition_scope(class_name);
        self.base.push_type_body();
        self.field_maps.push(self.collect_class_field_names(node));
        self.base.enter_scope("class");

        let children_streams = self.visit_children(node)?;
        self.base.exit_scope();
        self.field_maps.pop();
        self.base.pop_type_body();
        self.base.pop_definition_scope();

        if let Some(prev) = self.type_param_map_stack.pop() {
            self.type_param_map = prev;
        }

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("class_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("class_declaration", children_streams)))
        }
    }

    fn visit_record_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("record");
        let children_streams = self.visit_children(node)?;
        self.base.exit_scope();
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("record_declaration", vec![])))
        } else {
            Ok(Some(TokenStream::node("record_declaration", children_streams)))
        }
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
                children_streams.push(child_stream);
            }
        }
        self.base.exit_scope();

        if !children_streams.is_empty() {
            return Ok(Some(TokenStream::node("block", children_streams)));
        }
        Ok(Some(TokenStream::node("block", vec![])))
    }

    fn visit_lambda_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("lambda");
        let mut lambda_parts = Vec::new();
        let mut cursor = node.walk();
        for lambda_child in node.children(&mut cursor) {
            if lambda_child.kind() != "lambda_parameters" {
                if let Some(lambda_serialized) = self.visit(&lambda_child)? {
                    lambda_parts.push(lambda_serialized);
                }
            } else {
                let mut pc = lambda_child.walk();
                for lambda_param in lambda_child.children(&mut pc) {
                    if lambda_param.kind() == "identifier" && !self.arg_maps().is_empty() {
                        let identifier_text = self.source_text(&lambda_param).unwrap_or_default();
                        let normalized_name = {
                            if !self.current_arg_map_mut().contains_key(&identifier_text) {
                                let counter = self.base.state.arg_counter;
                                self.current_arg_map_mut()
                                    .insert(identifier_text.clone(), format!("arg_{counter}"));
                                self.base.state.arg_counter += 1;
                            }
                            self.base
                                .state
                                .current_arg_map()
                                .get(&identifier_text)
                                .cloned()
                                .unwrap_or(identifier_text)
                        };
                        lambda_parts.push(TokenStream::literal("parameter", normalized_name));
                    }
                }
            }
        }
        self.base.exit_scope();
        if lambda_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("lambda_expression", lambda_parts)))
        }
    }

    fn visit_formal_parameter(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    fn visit_record_components(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Mirror Python: process for side effects on arg maps, return None
        let mut cursor = node.walk();
        for component in node.children(&mut cursor) {
            if component.kind() != "record_component" {
                continue;
            }
            let mut cc = component.walk();
            for comp_child in component.children(&mut cc) {
                if comp_child.kind() == "identifier" && !self.arg_maps().is_empty() {
                    let identifier_text = self.source_text(&comp_child).unwrap_or_default();
                    let counter = self.base.state.arg_counter;
                    let map = self.current_arg_map_mut();
                    if let std::collections::hash_map::Entry::Vacant(e) = map.entry(identifier_text) {
                        e.insert(format!("arg_{counter}"));
                        self.base.state.arg_counter += 1;
                    }
                } else {
                    let _ = self.visit(&comp_child)?;
                }
            }
        }
        Ok(None)
    }

    fn visit_record_component(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Method invocation / references / field access
    // ------------------------------------------------------------------

    fn visit_method_reference(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut class_name = None;
        let mut method_name = None;
        let mut cursor = node.walk();
        for ref_child in node.children(&mut cursor) {
            match ref_child.kind() {
                "type_identifier" => {
                    let mut name = self.source_text(&ref_child).unwrap_or_default();
                    if !name.is_empty() {
                        if let Some(normalizer) = self.normalizer {
                            name = normalizer.qualify_java_lang_identifier(&name, &self.regular_imports);
                        }
                        class_name = Some(name);
                    }
                }
                "identifier" => {
                    method_name = self.source_text(&ref_child);
                }
                "::" => {}
                _ => {}
            }
        }

        match (class_name, method_name) {
            (Some(class_name), Some(method_name)) => Ok(Some(TokenStream::node(
                "method_reference",
                vec![TokenStream::text(class_name), TokenStream::text(method_name)],
            ))),
            (None, Some(method_name)) => Ok(Some(TokenStream::node(
                "method_reference",
                vec![TokenStream::text(method_name)],
            ))),
            _ => Ok(None),
        }
    }

    fn is_identifier_bound(&self, identifier_text: &str) -> bool {
        self.base.is_allowed_local(identifier_text)
    }

    fn build_call_signature(
        &self,
        qualified_name: Option<&str>,
        object_identifier: Option<&str>,
        method_name: Option<&str>,
    ) -> Option<TokenStream> {
        if let (Some(qualified_name), Some(method_name)) = (qualified_name, method_name) {
            return Some(TokenStream::literal("call", format!("{qualified_name}.{method_name}")));
        }
        if let (Some(object_identifier), Some(method_name)) = (object_identifier, method_name) {
            return Some(TokenStream::literal(
                "call",
                format!("{object_identifier}.{method_name}"),
            ));
        }
        method_name.map(|m| TokenStream::literal("call", m))
    }

    fn extract_identifier_name(&self, child: &Node, is_first: bool) -> Option<String> {
        let mut text = self.source_text(child)?;
        if text.is_empty() || self.should_strip_prefix(&text) {
            return None;
        }
        if is_first {
            if let Some(normalizer) = self.normalizer {
                text = normalizer.qualify_java_lang_identifier(&text, &self.regular_imports);
            }
        }
        Some(text)
    }

    fn extract_scoped_identifier_name(&self, child: &Node, is_first: bool) -> Option<String> {
        let mut text = self.source_text(child)?;
        if text.is_empty() || self.should_strip_prefix(&text) {
            return None;
        }
        if is_first {
            if let Some(normalizer) = self.normalizer {
                text = normalizer.qualify_java_lang_identifier(&text, &self.regular_imports);
            }
        }
        Some(text)
    }

    fn extract_name_part(&self, child: &Node, is_first: bool) -> Option<String> {
        match child.kind() {
            "identifier" => self.extract_identifier_name(child, is_first),
            "scoped_identifier" => self.extract_scoped_identifier_name(child, is_first),
            "field_access" => {
                let nested = self.extract_qualified_name(child);
                if nested.is_empty() {
                    None
                } else {
                    Some(nested)
                }
            }
            "." | "this" | "super" => None,
            _ => {
                let nested = self.extract_qualified_name(child);
                if nested.is_empty() {
                    None
                } else {
                    Some(nested)
                }
            }
        }
    }

    fn extract_qualified_name(&self, node: &Node) -> String {
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(part) = self.extract_name_part(&child, parts.is_empty()) {
                parts.push(part);
            }
        }
        if parts.len() == 1 {
            if let Some(fields) = self.field_maps.last() {
                if fields.contains(&parts[0]) {
                    return parts[0].clone();
                }
            }
        }
        parts.join(".")
    }

    fn visit_method_invocation(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut call_parts = Vec::new();
        let mut method_name = None;
        let mut qualified_name = None;
        let mut object_identifier = None;
        let mut seen_dot = false;

        let mut cursor = node.walk();
        for call_child in node.children(&mut cursor) {
            if call_child.kind() == "argument_list" {
                if let Some(arg_serialized) = self.visit(&call_child)? {
                    call_parts.push(arg_serialized);
                }
                continue;
            }

            match call_child.kind() {
                "identifier" => {
                    if seen_dot {
                        method_name = self.source_text(&call_child);
                    } else {
                        let identifier_text = self.source_text(&call_child).unwrap_or_default();
                        let object_stream = if self.is_identifier_bound(&identifier_text) {
                            self.visit(&call_child)?
                        } else {
                            Some(TokenStream::identifier(identifier_text))
                        };
                        if let Some(object_stream) = object_stream {
                            if let Some(ref id) = object_stream.identifier {
                                object_identifier = Some(id.clone());
                            }
                            call_parts.push(object_stream);
                        }
                    }
                }
                "." => {
                    seen_dot = true;
                }
                "object" | "primary" | "field_access" | "scoped_identifier" => {
                    let qn = self.extract_qualified_name(&call_child);
                    if !qn.is_empty() {
                        qualified_name = Some(qn);
                    }
                }
                "method_invocation" => {
                    // Chained calls use the preceding invocation as the object
                    // of the outer call.  Preserve its structured stream so
                    // receiver identity and inner arguments contribute to the
                    // callable body hash.
                    if let Some(receiver) = self.visit(&call_child)? {
                        call_parts.push(TokenStream::node("callee_expression", vec![receiver]));
                    }
                }
                "string_literal" | "text_block" => {}
                _ => {}
            }
        }

        if method_name.is_none() && !seen_dot {
            // Simple call `foo()` — identifier was captured as object, not method name.
            method_name = object_identifier.take();
        }

        if let Some(call_sig) = self.build_call_signature(
            qualified_name.as_deref(),
            object_identifier.as_deref(),
            method_name.as_deref(),
        ) {
            if let Some(ref callee) = call_sig.literal_value {
                self.base.track_call_node(callee.clone(), node, self.source);
            }
            call_parts.push(call_sig);
        }

        if call_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("method_invocation", call_parts)))
        }
    }

    fn visit_string_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("string", text)));
        }
        Ok(None)
    }

    fn visit_text_block(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("string", text)));
        }
        Ok(None)
    }

    fn visit_scoped_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        fn extract(visitor: &JavaNodeVisitor<'_>, n: &Node) -> Vec<String> {
            let mut parts = Vec::new();
            let mut cursor = n.walk();
            for scoped_child in n.children(&mut cursor) {
                if scoped_child.kind() == "identifier" {
                    if let Some(text) = visitor.node_text(&scoped_child) {
                        if !text.is_empty() && !visitor.should_strip_prefix(&text) {
                            parts.push(text);
                        }
                    }
                } else if scoped_child.kind() == "scoped_identifier" {
                    parts.extend(extract(visitor, &scoped_child));
                }
            }
            parts
        }

        let mut qualified_parts = extract(self, node);
        if !qualified_parts.is_empty() {
            self.qualify_first_segment(&mut qualified_parts);
            return Ok(Some(TokenStream::identifier(qualified_parts.join("."))));
        }
        Ok(Some(TokenStream::node("scoped_identifier", vec![])))
    }

    fn visit_field_access(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        fn extract(visitor: &mut JavaNodeVisitor<'_>, n: &Node) -> Result<Vec<String>, RecursionDepthError> {
            let mut parts = Vec::new();
            let mut cursor = n.walk();
            let children: Vec<Node> = n.children(&mut cursor).collect();
            for field_child in &children {
                if field_child.kind() == "identifier" {
                    let identifier_stream = visitor.visit_identifier(field_child)?;
                    let text = if let Some(ref stream) = identifier_stream {
                        stream
                            .identifier
                            .clone()
                            .unwrap_or_else(|| visitor.node_text(field_child).unwrap_or_default())
                    } else {
                        visitor.node_text(field_child).unwrap_or_default()
                    };
                    if !text.is_empty() && !visitor.should_strip_prefix(&text) {
                        parts.push(text);
                    }
                } else if field_child.kind() == "field_access" {
                    parts.extend(extract(visitor, field_child)?);
                }
            }
            Ok(parts)
        }

        let mut qualified_parts = extract(self, node)?;
        while qualified_parts
            .first()
            .map(|p| self.should_strip_prefix(p))
            .unwrap_or(false)
        {
            qualified_parts.remove(0);
        }
        if qualified_parts.is_empty() {
            return Ok(None);
        }
        if qualified_parts.len() == 1 {
            if let Some(fields) = self.field_maps.last() {
                if fields.contains(&qualified_parts[0]) {
                    return Ok(Some(TokenStream::identifier(qualified_parts[0].clone())));
                }
            }
        }
        self.qualify_first_segment(&mut qualified_parts);
        Ok(Some(TokenStream::identifier(qualified_parts.join("."))))
    }

    fn visit_instanceof_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut left_operand = None;
        let mut type_identifier = None;
        let mut cursor = node.walk();
        for instanceof_child in node.children(&mut cursor) {
            if instanceof_child.kind() == "type_identifier" {
                let mut ti = self.source_text(&instanceof_child).unwrap_or_default();
                if !ti.is_empty() {
                    if let Some(normalizer) = self.normalizer {
                        ti = normalizer.qualify_java_lang_identifier(&ti, &self.regular_imports);
                    }
                    type_identifier = Some(ti);
                }
            } else if instanceof_child.kind() == "identifier" && type_identifier.is_some() {
                continue;
            } else if instanceof_child.kind() != "instanceof" {
                if let Some(left_serialized) = self.visit(&instanceof_child)? {
                    left_operand = Some(left_serialized);
                }
            }
        }

        let mut instanceof_parts = Vec::new();
        if let Some(left_operand) = left_operand {
            instanceof_parts.push(left_operand);
        }
        if let Some(type_identifier) = type_identifier {
            instanceof_parts.push(TokenStream::text(type_identifier));
            return Ok(Some(TokenStream::node("instanceof_expression", instanceof_parts)));
        }
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Control flow / statements / literals / identifier
    // ------------------------------------------------------------------

    fn visit_simple_named(&mut self, node: &Node, name: &str) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(Some(TokenStream::node(name, vec![])))
        } else {
            Ok(Some(TokenStream::node(name, children)))
        }
    }

    fn visit_if_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "if_statement", &["consequence", "alternative"])
    }

    fn visit_while_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "while_statement", &["body"])
    }

    fn visit_for_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "for_statement", &["body"])
    }

    fn visit_enhanced_for_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_with_wrapped_body(node, "enhanced_for_statement", &["body"])
    }

    fn visit_parenthesized_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !matches!(child.kind(), "(" | ")") {
                return self.visit(&child);
            }
        }
        Ok(None)
    }

    fn visit_binary_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children(node)?;
        if children.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("binary_expression", children)))
        }
    }

    fn visit_null_literal(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(Some(TokenStream::literal("null_literal", "null")))
    }

    fn visit_floating_point_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("floating_point_literal", text)));
        }
        Ok(None)
    }

    fn visit_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let identifier_text = self.source_text(node).unwrap_or_default();
        if self.base.file_imports.contains(&identifier_text) {
            self.base.track_callable_import_usage(&identifier_text);
        }

        if self.var_maps().is_empty() || self.in_parameter_list() {
            return self.visit_generic(node);
        }

        if let Some(qualified) = self.static_imports.get(&identifier_text) {
            return Ok(Some(TokenStream::identifier(qualified.clone())));
        }

        if let Some(fields) = self.field_maps.last() {
            if fields.contains(&identifier_text) {
                return Ok(Some(TokenStream::identifier(identifier_text)));
            }
        }

        let is_bound_variable = self.is_identifier_bound(&identifier_text);
        if !is_bound_variable {
            return Ok(Some(TokenStream::identifier(identifier_text)));
        }

        Ok(Some(TokenStream::identifier(identifier_text)))
    }

    fn dispatch_visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "line_comment" => self.visit_line_comment(node),
            "block_comment" => self.visit_block_comment(node),
            "catch_clause" => self.visit_catch_clause(node),
            "catch_type" => self.visit_catch_type(node),
            "object_creation_expression" => self.visit_object_creation_expression(node),
            "type" => self.visit_type(node),
            "return_type" => self.visit_return_type(node),
            "type_parameters" => self.visit_type_parameters(node),
            "type_arguments" => self.visit_type_arguments(node),
            "type_identifier" => self.visit_type_identifier(node),
            "integral_type" => self.visit_integral_type(node),
            "floating_point_type" => self.visit_floating_point_type(node),
            ";" | "semicolon" => self.visit_semicolon(node),
            "method_declaration" => self.visit_method_declaration(node),
            "constructor_declaration" => self.visit_constructor_declaration(node),
            "class_declaration" => self.visit_class_declaration(node),
            "record_declaration" => self.visit_record_declaration(node),
            "block" => self.visit_block(node),
            "lambda_expression" => self.visit_lambda_expression(node),
            "import_declaration" => self.visit_import_declaration(node),
            "modifiers" => self.visit_modifiers(node),
            "marker_annotation" => self.visit_marker_annotation(node),
            "annotation" => self.visit_annotation(node),
            "dimensions" => self.visit_dimensions(node),
            "array_type" => self.visit_array_type(node),
            "formal_parameter" => self.visit_formal_parameter(node),
            "record_components" => self.visit_record_components(node),
            "record_component" => self.visit_record_component(node),
            "method_reference" => self.visit_method_reference(node),
            "method_invocation" => self.visit_method_invocation(node),
            "string_literal" => self.visit_string_literal(node),
            "text_block" => self.visit_text_block(node),
            "scoped_identifier" => self.visit_scoped_identifier(node),
            "scoped_type_identifier" => self.visit_scoped_type_identifier(node),
            "field_access" => self.visit_field_access(node),
            "instanceof_expression" => self.visit_instanceof_expression(node),
            "if_statement" => self.visit_if_statement(node),
            "while_statement" => self.visit_while_statement(node),
            "for_statement" => self.visit_for_statement(node),
            "enhanced_for_statement" => self.visit_enhanced_for_statement(node),
            "return_statement" => self.visit_simple_named(node, "return_statement"),
            "break_statement" => self.visit_simple_named(node, "break_statement"),
            "continue_statement" => self.visit_simple_named(node, "continue_statement"),
            "switch_statement" => self.visit_simple_named(node, "switch_statement"),
            "assert_statement" => self.visit_simple_named(node, "assert_statement"),
            "try_statement" => self.visit_simple_named(node, "try_statement"),
            "throw_statement" => self.visit_simple_named(node, "throw_statement"),
            "identifier" => self.visit_identifier(node),
            "decimal_integer_literal" | "hex_integer_literal" | "octal_integer_literal" | "binary_integer_literal" => {
                Ok(self.normalize_integer_literal_node(node))
            }
            "floating_point_literal" => self.visit_floating_point_literal(node),
            "parenthesized_expression" => self.visit_parenthesized_expression(node),
            "binary_expression" => self.visit_binary_expression(node),
            "null_literal" => self.visit_null_literal(node),
            // No visit_ERROR — hasher rejects ERROR for Java
            _ => self.visit_generic(node),
        }
    }
}

impl<'a> NodeVisitor for JavaNodeVisitor<'a> {
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
