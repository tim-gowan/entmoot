//! Python AST visitor for structured serialization.

use std::collections::HashMap;

use tree_sitter::Node;

use crate::serialization::TokenStream;
use crate::visitor::{BaseVisitor, NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::normalizer::PythonNormalizer;
use super::scope_analyzer::PythonScopeAnalyzer;

/// Visitor for Python AST nodes using structured serialization.
pub struct PythonNodeVisitor<'a> {
    pub base: BaseVisitor,
    pub source: &'a [u8],
    pub normalizer: Option<&'a PythonNormalizer>,
}

impl<'a> PythonNodeVisitor<'a> {
    pub fn new(
        policy: Option<StructuralHashPolicy>,
        source: &'a [u8],
        normalizer: Option<&'a PythonNormalizer>,
    ) -> Self {
        Self {
            base: BaseVisitor::new(None, policy),
            source,
            normalizer,
        }
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

    fn current_arg_map(&self) -> &HashMap<String, String> {
        self.base.state.current_arg_map()
    }

    fn visit_module(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("module", vec![])))
        } else {
            Ok(Some(TokenStream::node("module", children_streams)))
        }
    }

    fn visit_block(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }

        if !children_streams.is_empty() {
            return Ok(Some(TokenStream::node("block", children_streams)));
        }

        Ok(Some(TokenStream::node("block", vec![])))
    }

    fn visit_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(self.source_text(node).map(|text| TokenStream::literal("comment", text)))
    }

    fn visit_decorator(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("decorator", vec![])))
        } else {
            Ok(Some(TokenStream::node("decorator", children_streams)))
        }
    }

    fn visit_decorated_definition(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut inner_definition = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "decorator" => {
                    if let Some(decorator_stream) = self.visit(&child)? {
                        children_streams.push(decorator_stream);
                    }
                }
                "decorator_list" => {
                    let mut dc = child.walk();
                    for decorator_child in child.children(&mut dc) {
                        if decorator_child.kind() == "decorator" {
                            if let Some(decorator_stream) = self.visit(&decorator_child)? {
                                children_streams.push(decorator_stream);
                            }
                        }
                    }
                }
                _ => {
                    inner_definition = self.visit(&child)?;
                }
            }
        }

        if let Some(inner) = inner_definition {
            children_streams.push(inner);
        }

        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("decorated_definition", children_streams)))
        }
    }

    fn process_typed_parameter(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut param_name = None;
        let mut type_annotation = None;

        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            match param_child.kind() {
                "identifier" => {
                    let identifier_text = self.source_text(&param_child).unwrap_or_default();
                    if identifier_text == "self" {
                        param_name = Some("self".to_string());
                    } else if !self.var_maps().is_empty() {
                        param_name = Some(
                            self.base
                                .normalize_parameter_name_at(&identifier_text, Some(param_child.start_byte())),
                        );
                    } else {
                        param_name = Some(identifier_text);
                    }
                }
                "type" if true => {
                    type_annotation = self.visit(&param_child)?;
                }
                _ => {}
            }
        }

        if let Some(name) = param_name {
            if let Some(ref annotation) = type_annotation {
                {
                    return Ok(Some(TokenStream::node(
                        "typed_parameter",
                        vec![TokenStream::literal("parameter", name), annotation.clone()],
                    )));
                }
            }
            return Ok(Some(TokenStream::literal("parameter", name)));
        }

        Ok(None)
    }

    fn process_parameters_in_visitor(&mut self, child: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        if child.kind() != "parameters" {
            return Ok(vec![]);
        }

        let mut param_parts = Vec::new();
        let mut cursor = child.walk();
        for param in child.children(&mut cursor) {
            if let Some(param_stream) = self.process_single_parameter(&param)? {
                param_parts.push(param_stream);
            }
        }
        Ok(param_parts)
    }

    fn process_single_parameter(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match param.kind() {
            "identifier" => self.process_identifier_parameter(param),
            "typed_parameter" => self.process_typed_parameter(param),
            "default_parameter" => self.process_default_parameter(param),
            "list_splat_pattern" => self.process_list_splat_pattern(param),
            "dictionary_splat_pattern" => self.process_dictionary_splat_pattern(param),
            // Positional-only `/` and keyword-only `*` separators change calling convention.
            "positional_separator" => Ok(Some(TokenStream::literal("positional_separator", "/"))),
            "keyword_separator" => Ok(Some(TokenStream::literal("keyword_separator", "*"))),
            _ => Ok(None),
        }
    }

    fn process_identifier_parameter(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let identifier_text = self.source_text(param).unwrap_or_default();
        if identifier_text == "self" {
            return Ok(Some(TokenStream::identifier("self")));
        }

        if !self.arg_maps().is_empty() {
            let normalized = self
                .base
                .normalize_parameter_name_at(&identifier_text, Some(param.start_byte()));
            return Ok(Some(TokenStream::literal("parameter", normalized)));
        }

        Ok(Some(TokenStream::literal("parameter", identifier_text)))
    }

    fn process_default_parameter(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let param_name = self.extract_default_parameter_name(param);
        let default_value = self.extract_default_parameter_value(param)?;

        if let Some(name) = param_name {
            if let Some(value) = default_value {
                return Ok(Some(TokenStream::node(
                    "default_parameter",
                    vec![TokenStream::literal("parameter", name), value],
                )));
            }
            return Ok(Some(TokenStream::literal("parameter", name)));
        }

        Ok(None)
    }

    fn extract_default_parameter_name(&mut self, param: &Node) -> Option<String> {
        let mut found_equals = false;
        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            if param_child.kind() == "=" {
                found_equals = true;
                continue;
            }
            if !found_equals {
                match param_child.kind() {
                    "identifier" => {
                        let identifier_text = self.source_text(&param_child).unwrap_or_default();
                        if identifier_text == "self" {
                            return Some("self".to_string());
                        }
                        if !self.arg_maps().is_empty() {
                            return Some(
                                self.base
                                    .normalize_parameter_name_at(&identifier_text, Some(param_child.start_byte())),
                            );
                        }
                        return Some(identifier_text);
                    }
                    "typed_parameter" => {
                        if let Ok(Some(typed)) = self.process_typed_parameter(&param_child) {
                            if let Some(ref value) = typed.literal_value {
                                return Some(value.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn extract_default_parameter_value(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut found_equals = false;
        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            if param_child.kind() == "=" {
                found_equals = true;
                continue;
            }
            if found_equals {
                return self.visit(&param_child);
            }
        }
        Ok(None)
    }

    fn process_list_splat_pattern(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            if param_child.kind() == "identifier" {
                let identifier_text = self.source_text(&param_child).unwrap_or_default();
                if !self.arg_maps().is_empty() {
                    let normalized = self
                        .base
                        .normalize_parameter_name_at(&identifier_text, Some(param_child.start_byte()));
                    return Ok(Some(TokenStream::literal("rest_parameter", normalized)));
                }
                return Ok(Some(TokenStream::literal("rest_parameter", identifier_text)));
            }
        }
        Ok(None)
    }

    fn process_dictionary_splat_pattern(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            if param_child.kind() == "identifier" {
                let identifier_text = self.source_text(&param_child).unwrap_or_default();
                if !self.arg_maps().is_empty() {
                    let normalized = self
                        .base
                        .normalize_parameter_name_at(&identifier_text, Some(param_child.start_byte()));
                    return Ok(Some(TokenStream::literal("keyword_parameter", normalized)));
                }
                return Ok(Some(TokenStream::literal("keyword_parameter", identifier_text)));
            }
        }
        Ok(None)
    }

    fn visit_function_definition(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let function_name = self.extract_function_name(node);

        self.base.begin_callable(
            function_name.unwrap_or_else(|| "anonymous".to_string()),
            "function",
            node.start_byte(),
            node.end_byte(),
        );

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.base
            .preflight_callable_scope(&PythonScopeAnalyzer, node, self.source);

        let children_streams = self.process_function_children_py(node)?;

        self.base.end_callable();

        self.base.exit_callable_scope();
        self.base.exit_scope();

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("function_definition", vec![])))
        } else {
            Ok(Some(TokenStream::node("function_definition", children_streams)))
        }
    }

    fn extract_function_name(&self, node: &Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                return self.source_text(&child);
            }
        }
        None
    }

    fn process_function_children_py(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "comment" | "decorator" | "decorator_list") {
                continue;
            }

            if matches!(child.kind(), "type" | "->") {
                if let Some(type_stream) = self.visit(&child)? {
                    children_streams.push(type_stream);
                }
                continue;
            }

            if child.kind() == "parameters" {
                self.set_in_parameter_list(true);
                let param_parts = self.process_parameters_in_visitor(&child)?;
                children_streams.extend(param_parts);
                self.set_in_parameter_list(false);
                continue;
            }

            if child.kind() == "identifier" {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }

        Ok(children_streams)
    }

    fn visit_class_definition(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let class_name = self.extract_function_name(node).unwrap_or_else(|| "@anon".to_string());
        self.base
            .record_declaration(class_name.clone(), "class", node.start_byte(), node.end_byte(), None);
        self.base.push_definition_scope(class_name);
        self.base.push_type_body();
        self.base.enter_scope("class");

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "decorator" | "decorator_list") {
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
            Ok(Some(TokenStream::node("class_definition", vec![])))
        } else {
            Ok(Some(TokenStream::node("class_definition", children_streams)))
        }
    }

    fn visit_lambda(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let bound_site = crate::analysis::bound_site_from_ancestors(
            *node,
            self.source,
            crate::analysis::PYTHON_ASSIGNMENT,
            crate::analysis::python_simple_assignment_lhs_site,
        );
        let track_bound = bound_site.is_some() && !self.base.is_inside_callable();
        if track_bound {
            if let Some(site) = bound_site.as_ref() {
                self.base.record_declaration(
                    site.name.clone(),
                    "assigned_callable",
                    site.start_byte,
                    site.end_byte,
                    Some(node.start_byte()),
                );
                self.base
                    .begin_callable(site.name.clone(), "function", node.start_byte(), node.end_byte());
            }
        }

        // Nested lambda scope: preflight params here so they do not leak into the outer function.
        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.base
            .preflight_callable_scope(&PythonScopeAnalyzer, node, self.source);

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "lambda_parameters" {
                self.set_in_parameter_list(true);
                let mut pc = child.walk();
                let mut param_children = Vec::new();
                for param in child.children(&mut pc) {
                    if let Some(stream) = self.process_single_parameter(&param)? {
                        param_children.push(stream);
                    } else if let Some(stream) = self.visit(&param)? {
                        param_children.push(stream);
                    }
                }
                children_streams.push(TokenStream::node("lambda_parameters", param_children));
                self.set_in_parameter_list(false);
                continue;
            }
            if let Some(stream) = self.visit(&child)? {
                children_streams.push(stream);
            }
        }

        self.base.exit_callable_scope();
        self.base.exit_scope();

        if track_bound {
            self.base.end_callable();
        }

        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("lambda", vec![])))
        } else {
            Ok(Some(TokenStream::node("lambda", children_streams)))
        }
    }

    fn visit_parameters(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    fn visit_assignment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if !self.var_maps().is_empty() {
            let mut cursor = node.walk();
            for assign_child in node.children(&mut cursor) {
                if assign_child.kind() == "identifier" {
                    let identifier_text = self.source_text(&assign_child).unwrap_or_default();
                    if !self.current_arg_map().contains_key(&identifier_text) {
                        self.base
                            .register_variable_if_new_at(&identifier_text, Some(assign_child.start_byte()));
                    }
                } else if assign_child.kind() == "attribute" {
                    break;
                }
            }
        }

        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(stream) = self.visit(&child)? {
                children_streams.push(stream);
            }
        }
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("assignment", vec![])))
        } else {
            Ok(Some(TokenStream::node("assignment", children_streams)))
        }
    }

    fn extract_function_name_from_call(&self, call_child: &Node) -> Option<String> {
        if call_child.kind() != "identifier" {
            return None;
        }
        let function_name = self.source_text(call_child)?;
        Some(function_name)
    }

    fn extract_qualified_name_from_call(&self, call_child: &Node) -> Option<String> {
        if !matches!(call_child.kind(), "attribute" | "call" | "subscript") {
            return None;
        }
        self.normalizer.map(|normalizer| {
            normalizer.extract_qualified_name_python_normalized(
                call_child,
                self.source,
                &self.base.state.var_maps,
                &self.base.state.arg_maps,
            )
        })
    }

    fn visit_call(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut call_parts = Vec::new();
        let mut normalized_callee = None;
        let mut syntactic_callee = None;

        if let Some(function) = node.child_by_field_name("function") {
            syntactic_callee = self.source_text(&function);
            normalized_callee = match function.kind() {
                "identifier" => self.extract_function_name_from_call(&function),
                "attribute" | "call" | "subscript" => self.extract_qualified_name_from_call(&function),
                _ => self.source_text(&function),
            };
            if let Some(function_stream) = self.visit(&function)? {
                call_parts.push(TokenStream::node("callee_expression", vec![function_stream]));
            }
        }

        let mut cursor = node.walk();
        for call_child in node.children(&mut cursor) {
            if call_child.kind() == "argument_list" {
                if let Some(arg_serialized) = self.visit(&call_child)? {
                    call_parts.push(arg_serialized);
                }
            }
        }

        if let Some(name) = normalized_callee {
            call_parts.push(TokenStream::literal("call", name));
        }
        if let Some(name) = syntactic_callee {
            self.base.track_call_node(name, node, self.source);
        }

        if call_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("call", call_parts)))
        }
    }

    fn visit_string(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let Some(text) = self.source_text(node) else {
            return Ok(None);
        };

        {
            Ok(Some(TokenStream::literal("string", text)))
        }
    }

    fn visit_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        {
            let result = self.visit_generic(node)?;
            if let Some(ref stream) = result {
                if TokenStream::has_content(stream) {
                    if let Some(identifier_text) = self.source_text(node) {
                        if self.is_imported_identifier(&identifier_text) {
                            self.track_import_usage(&identifier_text);
                        }
                    }
                }
            }
            Ok(result)
        }
    }

    fn is_imported_identifier(&self, identifier_text: &str) -> bool {
        if self.base.file_imports.contains(identifier_text) {
            if self.base.is_allowed_local(identifier_text) {
                return false;
            }
            return true;
        }

        for import_name in &self.base.file_imports {
            if import_name.ends_with(&format!(".{identifier_text}")) || import_name == identifier_text {
                if self.base.is_allowed_local(identifier_text) {
                    return false;
                }
                return true;
            }
        }

        false
    }

    fn track_import_usage(&mut self, identifier_text: &str) {
        if let Some(callable) = self.base.current_callable.as_mut() {
            callable.imports.insert(identifier_text.to_string());
        }
    }

    fn visit_attribute(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_generic(node)
    }

    fn visit_if_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("if_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("if_statement", children_streams)))
        }
    }

    fn visit_while_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("while_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("while_statement", children_streams)))
        }
    }

    fn visit_for_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("for_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("for_statement", children_streams)))
        }
    }

    fn visit_for_in_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("for_in_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("for_in_statement", children_streams)))
        }
    }

    fn visit_return_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("return_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("return_statement", children_streams)))
        }
    }

    fn visit_break_statement(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(Some(TokenStream::node("break_statement", vec![])))
    }

    fn visit_continue_statement(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(Some(TokenStream::node("continue_statement", vec![])))
    }

    fn visit_try_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("try_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("try_statement", children_streams)))
        }
    }

    fn visit_with_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("with_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("with_statement", children_streams)))
        }
    }

    fn visit_raise_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("raise_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("raise_statement", children_streams)))
        }
    }

    fn visit_assert_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children_streams = self.visit_children(node)?;
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("assert_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("assert_statement", children_streams)))
        }
    }

    fn visit_integer(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let Some(text) = self.source_text(node) else {
            return Ok(None);
        };
        Ok(Some(TokenStream::literal("integer", text)))
    }

    fn visit_float(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let Some(text) = self.source_text(node) else {
            return Ok(None);
        };
        Ok(Some(TokenStream::literal("float", text)))
    }

    fn visit_import_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut import_parts = Vec::new();
        let mut import_names = Vec::new();
        let start = node.start_byte();
        let end = node.end_byte();

        let mut cursor = node.walk();
        for import_child in node.children(&mut cursor) {
            match import_child.kind() {
                "dotted_name" => {
                    if let Some(module_name) = self.source_text(&import_child) {
                        import_parts.push(TokenStream::literal("import_module", module_name.clone()));
                        import_names.push(module_name.clone());
                        let local_binding = module_name
                            .split_once('.')
                            .map(|(first_segment, _)| first_segment.to_string());
                        self.base.track_import_marker(
                            module_name,
                            local_binding,
                            entmoot_contracts::ImportKind::Module,
                            start,
                            end,
                        );
                    }
                }
                "aliased_import" => {
                    self.process_aliased_import(&import_child, &mut import_parts);
                    let (path, alias) = self.aliased_import_path_and_alias(&import_child);
                    if let Some(module_name) = path.clone() {
                        import_names.push(module_name);
                    }
                    if let Some(path) = path {
                        self.base
                            .track_import_marker(path, alias, entmoot_contracts::ImportKind::Alias, start, end);
                    }
                }
                "identifier" => {
                    self.process_simple_import(&import_child, &mut import_parts);
                    if let Some(module_name) = self.source_text(&import_child) {
                        import_names.push(module_name.clone());
                        self.base.track_import_marker(
                            module_name,
                            None,
                            entmoot_contracts::ImportKind::Module,
                            start,
                            end,
                        );
                    }
                }
                _ => {}
            }
        }

        for import_name in import_names {
            self.base.track_file_import(import_name);
        }

        if import_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("import_statement", import_parts)))
        }
    }

    fn aliased_import_path_and_alias(&self, import_child: &Node) -> (Option<String>, Option<String>) {
        let mut path = None;
        let mut alias = None;
        let mut saw_as = false;
        let mut cursor = import_child.walk();
        for child in import_child.children(&mut cursor) {
            match child.kind() {
                "as" => saw_as = true,
                "dotted_name" | "identifier" if !saw_as => path = self.source_text(&child),
                "identifier" if saw_as => alias = self.source_text(&child),
                _ => {}
            }
        }
        (path, alias)
    }

    fn process_aliased_import(&self, import_child: &Node, import_parts: &mut Vec<TokenStream>) {
        let mut cursor = import_child.walk();
        let children: Vec<Node> = import_child.children(&mut cursor).collect();
        for alias_child in &children {
            if alias_child.kind() == "dotted_name" {
                if let Some(module_name) = self.source_text(alias_child) {
                    import_parts.push(TokenStream::literal("import_module", module_name));
                }
            }
        }
        if children.len() > 2 {
            for alias_child in &children {
                if alias_child.kind() == "as" {
                    if let Some(last) = children.last() {
                        if let Some(alias_name) = self.source_text(last) {
                            import_parts.push(TokenStream::literal("import_alias", alias_name));
                        }
                    }
                    break;
                }
            }
        }
    }

    fn process_simple_import(&self, node: &Node, import_parts: &mut Vec<TokenStream>) {
        if let Some(module_name) = self.source_text(node) {
            import_parts.push(TokenStream::literal("import_module", module_name));
        }
    }

    fn visit_import_from_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut import_parts = Vec::new();
        let mut module_path = None;
        let mut imported_names = Vec::new();

        let mut cursor = node.walk();
        for import_child in node.children(&mut cursor) {
            module_path =
                self.process_import_from_child(&import_child, &mut import_parts, &mut imported_names, module_path)?;
        }

        if let Some(path) = &module_path {
            import_parts.push(TokenStream::literal("import_module", path.clone()));
        }

        let imported_names_for_tracking = imported_names.clone();
        for name in imported_names {
            import_parts.push(TokenStream::literal("import_name", name));
        }

        // Analysis uses the grammar's module field even when normalization did
        // not encounter a bare dotted name (relative imports with aliases).
        if let Some(path) = node
            .child_by_field_name("module_name")
            .and_then(|module| self.source_text(&module))
            .or_else(|| module_path.clone())
        {
            self.emit_import_from_markers(node, &path);
        }

        if let Some(path) = module_path {
            self.base.track_file_import(path.clone());
            for name in &imported_names_for_tracking {
                self.base.track_file_import(name.clone());
                self.base.track_file_import(format!("{path}.{name}"));
            }
        } else {
            for name in imported_names_for_tracking {
                self.base.track_file_import(name);
            }
        }

        if import_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("import_from_statement", import_parts)))
        }
    }

    /// Emit ImportMarker(s) for `from … import …` using the statement node spans.
    fn emit_import_from_markers(&mut self, node: &Node, module_path: &str) {
        let start = node.start_byte();
        let end = node.end_byte();

        // Prefer grammar fields: module_name / name (avoids confusing two dotted_name children).
        let path = node
            .child_by_field_name("module_name")
            .and_then(|n| self.source_text(&n))
            .unwrap_or_else(|| module_path.to_string());

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "wildcard_import" {
                self.base
                    .track_import_marker(path, None, entmoot_contracts::ImportKind::Star, start, end);
                return;
            }
        }

        let names = node
            .children_by_field_name("name", &mut node.walk())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            for name_node in names {
                match name_node.kind() {
                    "aliased_import" | "dotted_as_name" => {
                        self.emit_named_from_import_marker(&name_node, &path, start, end);
                    }
                    "dotted_name" | "identifier" => {
                        if let Some(local) = self.source_text(&name_node) {
                            self.emit_plain_named_from_import_marker(&path, local, start, end);
                        }
                    }
                    "dotted_as_names" => {
                        let mut nc = name_node.walk();
                        for name_child in name_node.children(&mut nc) {
                            match name_child.kind() {
                                "aliased_import" | "dotted_as_name" => {
                                    self.emit_named_from_import_marker(&name_child, &path, start, end);
                                }
                                "dotted_name" | "identifier" => {
                                    if let Some(local) = self.source_text(&name_child) {
                                        self.emit_plain_named_from_import_marker(&path, local, start, end);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            return;
        }

        // Fallback for older/alternate shapes without a `name` field.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "aliased_import" | "dotted_as_name" => {
                    self.emit_named_from_import_marker(&child, &path, start, end);
                }
                "dotted_as_names" => {
                    let mut nc = child.walk();
                    for name_child in child.children(&mut nc) {
                        match name_child.kind() {
                            "aliased_import" | "dotted_as_name" => {
                                self.emit_named_from_import_marker(&name_child, &path, start, end);
                            }
                            "dotted_name" | "identifier" => {
                                if let Some(local) = self.source_text(&name_child) {
                                    self.emit_plain_named_from_import_marker(&path, local, start, end);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn emit_plain_named_from_import_marker(&mut self, module_path: &str, local: String, start: usize, end: usize) {
        let target = format!("{module_path}.{local}");
        self.base
            .track_import_marker(target, Some(local), entmoot_contracts::ImportKind::Named, start, end);
    }

    fn emit_named_from_import_marker(&mut self, node: &Node, module_path: &str, start: usize, end: usize) {
        let mut name = None;
        let mut alias = None;
        let mut saw_as = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "as" => saw_as = true,
                "dotted_name" | "identifier" if !saw_as => name = self.source_text(&child),
                "identifier" if saw_as => alias = self.source_text(&child),
                _ => {}
            }
        }
        if let Some(imported_name) = name {
            let target = format!("{module_path}.{imported_name}");
            let local = alias.unwrap_or(imported_name);
            self.base
                .track_import_marker(target, Some(local), entmoot_contracts::ImportKind::Named, start, end);
        }
    }

    fn process_import_from_child(
        &mut self,
        import_child: &Node,
        import_parts: &mut Vec<TokenStream>,
        imported_names: &mut Vec<String>,
        module_path: Option<String>,
    ) -> Result<Option<String>, RecursionDepthError> {
        match import_child.kind() {
            "relative_import" => {
                self.process_relative_import(import_child, import_parts);
                Ok(module_path)
            }
            "dotted_name" => Ok(self.source_text(import_child)),
            "import_from_statement" => {
                if let Some(nested) = self.visit_import_from_statement(import_child)? {
                    import_parts.push(nested);
                }
                Ok(module_path)
            }
            "dotted_as_names" => {
                self.process_dotted_as_names(import_child, imported_names);
                Ok(module_path)
            }
            "dotted_as_name" => {
                self.process_dotted_as_name(import_child, imported_names);
                Ok(module_path)
            }
            "wildcard_import" => {
                import_parts.push(TokenStream::literal("import_wildcard", "*"));
                Ok(module_path)
            }
            _ => Ok(module_path),
        }
    }

    fn process_relative_import(&self, node: &Node, import_parts: &mut Vec<TokenStream>) {
        if let Some(relative_path) = self.source_text(node) {
            import_parts.push(TokenStream::literal("import_relative", relative_path));
        }
    }

    fn process_dotted_as_names(&self, import_child: &Node, imported_names: &mut Vec<String>) {
        let mut cursor = import_child.walk();
        for name_child in import_child.children(&mut cursor) {
            match name_child.kind() {
                "dotted_as_name" => self.process_dotted_as_name(&name_child, imported_names),
                "dotted_name" | "identifier" => {
                    if let Some(name) = self.source_text(&name_child) {
                        imported_names.push(name);
                    }
                }
                _ => {}
            }
        }
    }

    fn process_dotted_as_name(&self, import_child: &Node, imported_names: &mut Vec<String>) {
        let mut cursor = import_child.walk();
        let children: Vec<Node> = import_child.children(&mut cursor).collect();
        for name_part in &children {
            match name_part.kind() {
                "dotted_name" | "identifier" => {
                    if let Some(name) = self.source_text(name_part) {
                        imported_names.push(name);
                    }
                }
                "as" if children.len() > 2 => {
                    if let Some(last) = children.last() {
                        if let Some(alias) = self.source_text(last) {
                            imported_names.push(alias);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl NodeVisitor for PythonNodeVisitor<'_> {
    fn source(&self) -> &[u8] {
        self.source
    }
    fn context(&mut self) -> (&mut BaseVisitor, &[u8]) {
        (&mut self.base, self.source)
    }

    fn dispatch(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.dispatch_visit(node)
    }

    fn prepare_node(&mut self, _node: &Node) {}
}

impl<'a> PythonNodeVisitor<'a> {
    fn dispatch_visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "module" => self.visit_module(node),
            "block" => self.visit_block(node),
            "comment" => self.visit_comment(node),
            "decorator" => self.visit_decorator(node),
            "decorated_definition" => self.visit_decorated_definition(node),
            "function_definition" => self.visit_function_definition(node),
            "class_definition" => self.visit_class_definition(node),
            "lambda" => self.visit_lambda(node),
            "parameters" => self.visit_parameters(node),
            "assignment" => self.visit_assignment(node),
            "call" => self.visit_call(node),
            "string" => self.visit_string(node),
            "identifier" => self.visit_identifier(node),
            "attribute" => self.visit_attribute(node),
            "if_statement" => self.visit_if_statement(node),
            "while_statement" => self.visit_while_statement(node),
            "for_statement" => self.visit_for_statement(node),
            "for_in_statement" => self.visit_for_in_statement(node),
            "return_statement" => self.visit_return_statement(node),
            "break_statement" => self.visit_break_statement(node),
            "continue_statement" => self.visit_continue_statement(node),
            "try_statement" => self.visit_try_statement(node),
            "with_statement" => self.visit_with_statement(node),
            "raise_statement" => self.visit_raise_statement(node),
            "assert_statement" => self.visit_assert_statement(node),
            "integer" => self.visit_integer(node),
            "float" => self.visit_float(node),
            "import_statement" => self.visit_import_statement(node),
            "import_from_statement" => self.visit_import_from_statement(node),
            _ => self.visit_generic(node),
        }
    }
}
