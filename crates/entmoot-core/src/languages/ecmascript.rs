//! Shared ECMAScript visitor mechanics; dialect hooks retain syntax ownership.
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use tree_sitter::Node;

pub(crate) trait EcmaVisitor: NodeVisitor {
    fn type_nodes(&self) -> &[&str];
    fn should_skip_function_child(&self, node: &Node) -> bool;
    fn process_single_parameter_in_visitor(
        &mut self,
        node: &Node,
        skip: bool,
    ) -> Result<(Option<TokenStream>, bool), RecursionDepthError>;
    fn parameter_positions(&mut self) -> &mut std::collections::HashMap<String, usize>;
    fn is_type_only_import(&self, node: &Node) -> bool;
    fn process_import_clause(
        &mut self,
        node: &Node,
        parts: &mut Vec<TokenStream>,
        module: Option<&str>,
        statement: &Node,
    );

    fn find_arrow_body_node<'b>(&self, node: &'b Node) -> (Option<Node<'b>>, bool) {
        let mut body_node = None;
        let mut has_implicit_return = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "=>" | "formal_parameters") {
                continue;
            }
            if self.should_skip_function_child(&child) {
                continue;
            }
            if body_node.is_none() && child.kind() != "statement_block" {
                body_node = Some(child);
                has_implicit_return = true;
                break;
            }
        }
        (body_node, has_implicit_return)
    }

    fn process_arrow_function_body(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let (body_node, has_implicit_return) = self.find_arrow_body_node(node);
        let body_id = body_node.map(|n| n.id());
        let mut children_streams = Vec::new();

        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if self.should_skip_function_child(child) {
                continue;
            }
            if child.kind() == "formal_parameters" {
                self.context().0.state.in_parameter_list = true;
                let param_parts = self.process_formal_parameters_in_visitor(child)?;
                children_streams.extend(param_parts);
                self.context().0.state.in_parameter_list = false;
                continue;
            }
            if child.kind() == "=>" {
                continue;
            }
            if has_implicit_return && body_id == Some(child.id()) {
                if let Some(return_expr) = self.visit(child)? {
                    let return_stmt = TokenStream::node("return_statement", vec![return_expr]);
                    let block_stmt = TokenStream::node("statement_block", vec![return_stmt]);
                    children_streams.push(block_stmt);
                }
                continue;
            }
            if let Some(child_stream) = self.visit(child)? {
                children_streams.push(child_stream);
            }
        }
        Ok(children_streams)
    }

    fn visit_variable_declarator(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if child.kind() == "identifier" && !self.context().0.state.var_maps.is_empty() {
                let identifier_text = self.source_text(child).unwrap_or_default();
                let in_arg = self.context().0.state.current_arg_map().contains_key(&identifier_text);
                let in_var = self
                    .context()
                    .0
                    .state
                    .var_maps
                    .last()
                    .map(|m| m.contains_key(&identifier_text))
                    .unwrap_or(false);

                if !in_arg && !in_var {
                    let normalized = format!("var_{}", self.context().0.state.var_counter);
                    self.context()
                        .0
                        .state
                        .current_var_map_mut()
                        .insert(identifier_text.clone(), normalized);
                    self.context().0.state.var_counter += 1;
                }

                let normalized_name = if let Some(n) = self.context().0.state.current_arg_map().get(&identifier_text) {
                    n.clone()
                } else if let Some(m) = self.context().0.state.var_maps.last() {
                    m.get(&identifier_text)
                        .cloned()
                        .unwrap_or_else(|| identifier_text.clone())
                } else {
                    identifier_text
                };
                children_streams.push(TokenStream::identifier(normalized_name));
            } else if let Some(child_stream) = self.visit(child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("variable_declarator", children_streams)))
        }
    }

    fn process_required_param_in_visitor(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut identifier_found = false;
        let mut normalized_name = None;
        let mut default_value = None;
        let mut found_equals = false;

        let mut cursor = param.walk();
        let children: Vec<Node> = param.children(&mut cursor).collect();
        for param_child in &children {
            if self.type_nodes().contains(&param_child.kind()) {
                let _ = self.visit(param_child)?;
                continue;
            }
            if param_child.kind() == "=" {
                found_equals = true;
                continue;
            }
            if found_equals {
                default_value = self.visit(param_child)?;
                break;
            }
            if param_child.kind() == "identifier" && !self.context().0.state.arg_maps.is_empty() && !identifier_found {
                let identifier_text = self.source_text(param_child).unwrap_or_default();
                normalized_name = Some(self.context().0.normalize_parameter_name(&identifier_text));
                identifier_found = true;
            }
        }

        if let Some(name) = normalized_name {
            if let Some(default_value) = default_value {
                return Ok(Some(TokenStream::node(
                    "default_parameter",
                    vec![TokenStream::literal("parameter", name), default_value],
                )));
            }
            return Ok(Some(TokenStream::literal("parameter", name)));
        }
        Ok(None)
    }

    fn process_identifier_param_with_default(
        &mut self,
        param: &Node,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        if self.context().0.state.arg_maps.is_empty() {
            return Ok(None);
        }

        let identifier_text = self.source_text(param).unwrap_or_default();
        let normalized_name = self.context().0.normalize_parameter_name(&identifier_text);

        if let Some(parent) = param.parent() {
            if parent.kind() == "formal_parameters" {
                let mut cursor = parent.walk();
                let siblings: Vec<Node> = parent.children(&mut cursor).collect();
                if let Some(param_index) = siblings.iter().position(|s| s.id() == param.id()) {
                    if param_index + 1 < siblings.len()
                        && siblings[param_index + 1].kind() == "="
                        && param_index + 2 < siblings.len()
                    {
                        if let Some(default_value) = self.visit(&siblings[param_index + 2])? {
                            return Ok(Some(TokenStream::node(
                                "default_parameter",
                                vec![TokenStream::literal("parameter", normalized_name), default_value],
                            )));
                        }
                    }
                }
            }
        }

        Ok(Some(TokenStream::literal("parameter", normalized_name)))
    }

    fn process_rest_param_in_visitor(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            if param_child.kind() == "identifier" && !self.context().0.state.arg_maps.is_empty() {
                let identifier_text = self.source_text(&param_child).unwrap_or_default();
                let normalized_name = self.context().0.normalize_parameter_name(&identifier_text);
                return Ok(Some(TokenStream::literal("rest_parameter", normalized_name)));
            }
        }
        Ok(None)
    }

    fn visit_throw_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut throw_parts = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for throw_child in &children {
            if throw_child.kind() == "new_expression" {
                let mut new_parts = Vec::new();
                let mut nc = throw_child.walk();
                for new_child in throw_child.children(&mut nc) {
                    if new_child.kind() == "identifier" {
                        let class_name = self.source_text(&new_child).unwrap_or_default();
                        new_parts.push(TokenStream::literal("class_name", class_name));
                    } else if new_child.kind() == "arguments" {
                        if let Some(arg_serialized) = self.visit(&new_child)? {
                            new_parts.push(arg_serialized);
                        }
                    }
                }
                if !new_parts.is_empty() {
                    throw_parts.push(TokenStream::node("new_expression", new_parts));
                }
            } else if let Some(throw_serialized) = self.visit(throw_child)? {
                throw_parts.push(throw_serialized);
            }
        }
        if throw_parts.is_empty() {
            Ok(Some(TokenStream::node("throw_statement", vec![])))
        } else {
            Ok(Some(TokenStream::node("throw_statement", throw_parts)))
        }
    }

    fn process_assignment_pattern(&mut self, param: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut identifier_text = None;
        let mut default_value = None;

        let mut cursor = param.walk();
        for child in param.children(&mut cursor) {
            if child.kind() == "identifier" && !self.context().0.state.arg_maps.is_empty() {
                let text = self.source_text(&child).unwrap_or_default();
                self.context().0.normalize_parameter_name(&text);
                identifier_text = Some(text);
            } else if child.kind() == "=" {
                continue;
            } else {
                default_value = self.visit(&child)?;
            }
        }

        if let Some(identifier_text) = identifier_text {
            if !self.context().0.state.arg_maps.is_empty() {
                let normalized_name = self
                    .context()
                    .0
                    .state
                    .current_arg_map()
                    .get(&identifier_text)
                    .cloned()
                    .unwrap_or(identifier_text);
                if let Some(default_value) = default_value {
                    return Ok(Some(TokenStream::node(
                        "default_parameter",
                        vec![TokenStream::literal("parameter", normalized_name), default_value],
                    )));
                }
                return Ok(Some(TokenStream::literal("parameter", normalized_name)));
            }
        }
        Ok(None)
    }

    fn visit_statement_block(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.context().0.enter_scope("block");
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "{" | "}" | ";") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        self.context().0.exit_scope();
        if children_streams.is_empty() {
            Ok(Some(TokenStream::node("statement_block", vec![])))
        } else {
            Ok(Some(TokenStream::node("statement_block", children_streams)))
        }
    }

    fn visit_template_string(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut template_parts = Vec::new();
        let mut cursor = node.walk();
        for template_child in node.children(&mut cursor) {
            if template_child.kind() == "string_fragment" {
                if let Some(fragment_text) = self.source_text(&template_child) {
                    template_parts.push(TokenStream::literal("string_fragment", fragment_text));
                }
            } else if template_child.kind() == "template_substitution" {
                if let Some(sub) = self.visit(&template_child)? {
                    template_parts.push(sub);
                }
            }
        }
        if !template_parts.is_empty() {
            return Ok(Some(TokenStream::node("template_string", template_parts)));
        }
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("template_string", text)));
        }
        Ok(None)
    }

    fn visit_new_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut new_parts = Vec::new();
        let mut cursor = node.walk();
        for new_child in node.children(&mut cursor) {
            if new_child.kind() == "identifier" {
                let class_name = self.source_text(&new_child).unwrap_or_default();
                new_parts.push(TokenStream::literal("class_name", class_name));
            } else if new_child.kind() == "arguments" {
                if let Some(arg_serialized) = self.visit(&new_child)? {
                    new_parts.push(arg_serialized);
                }
            }
        }
        if new_parts.is_empty() {
            Ok(Some(TokenStream::node("new_expression", vec![])))
        } else {
            Ok(Some(TokenStream::node("new_expression", new_parts)))
        }
    }

    fn visit_type_parameters(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
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
            Ok(Some(TokenStream::node("type_parameters", vec![])))
        } else {
            Ok(Some(TokenStream::node("type_parameters", children_streams)))
        }
    }

    fn visit_variable_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "var" {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("variable_declaration", children_streams)))
        }
    }

    fn visit_lexical_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "let" | "const") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("lexical_declaration", children_streams)))
        }
    }

    fn extract_module_name_from_string(&mut self, arg_child: &Node) -> Option<String> {
        let module_name = self.source_text(arg_child)?;
        if module_name.is_empty() {
            return None;
        }
        if (module_name.starts_with('"') && module_name.ends_with('"'))
            || (module_name.starts_with('\'') && module_name.ends_with('\''))
        {
            return Some(module_name[1..module_name.len() - 1].to_string());
        }
        Some(module_name)
    }

    fn visit_import_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if self.is_type_only_import(node) {
            return Ok(None);
        }

        let mut import_parts = Vec::new();
        let mut module_path = None;
        let mut clause = None;
        let mut cursor = node.walk();
        for import_child in node.children(&mut cursor) {
            if import_child.kind() == "string" {
                if let Some(module_name) = self.extract_module_name_from_string(&import_child) {
                    import_parts.push(TokenStream::literal("import_module", module_name.clone()));
                    module_path = Some(module_name);
                }
            } else if import_child.kind() == "import_clause" {
                clause = Some(import_child);
            }
        }

        if let Some(clause_node) = clause {
            self.process_import_clause(&clause_node, &mut import_parts, module_path.as_deref(), node);
        } else if let Some(path) = module_path {
            self.context().0.track_import_marker(
                path,
                None,
                entmoot_contracts::ImportKind::Module,
                node.start_byte(),
                node.end_byte(),
            );
        }

        if import_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("import_statement", import_parts)))
        }
    }

    fn extract_param_name_from_node(&mut self, param: &Node) -> Option<String> {
        if param.kind() == "identifier" {
            return self.source_text(param);
        }
        let mut cursor = param.walk();
        for child in param.children(&mut cursor) {
            if child.kind() == "identifier" {
                return self.source_text(&child);
            } else if child.kind() == "rest_pattern" {
                let mut rc = child.walk();
                for rest_child in child.children(&mut rc) {
                    if rest_child.kind() == "identifier" {
                        return self.source_text(&rest_child);
                    }
                }
            }
        }
        None
    }

    fn process_formal_parameters_in_visitor(&mut self, child: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        if child.kind() != "formal_parameters" {
            return Ok(Vec::new());
        }

        let mut param_parts = Vec::new();
        let mut skip_next_identifier = false;
        let mut current_param_position = 0usize;

        let mut cursor = child.walk();
        let params: Vec<Node> = child.children(&mut cursor).collect();
        for param in &params {
            let (token, new_skip) = self.process_single_parameter_in_visitor(param, skip_next_identifier)?;
            skip_next_identifier = new_skip;
            if let Some(token) = token {
                if let Some(param_name) = self.extract_param_name_from_node(param) {
                    self.parameter_positions().insert(param_name, current_param_position);
                }
                current_param_position += 1;
                param_parts.push(token);
            }
        }

        Ok(param_parts)
    }

    fn process_function_children(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            if self.should_skip_function_child(child) {
                continue;
            }

            if child.kind() == "formal_parameters" {
                self.context().0.state.in_parameter_list = true;
                let param_parts = self.process_formal_parameters_in_visitor(child)?;
                children_streams.extend(param_parts);
                self.context().0.state.in_parameter_list = false;
                continue;
            }

            if let Some(child_stream) = self.visit(child)? {
                children_streams.push(child_stream);
            }
        }
        Ok(children_streams)
    }
}

pub(crate) fn extract_loop_variables(
    analyzer: &dyn crate::scope_analyzer::ScopeAnalyzer,
    node: &Node,
    source: &[u8],
    local_definitions: &mut std::collections::HashSet<String>,
) {
    match node.kind() {
        "for_statement" => {
            let mut cursor = node.walk();
            for for_child in node.children(&mut cursor) {
                if matches!(for_child.kind(), "lexical_declaration" | "variable_declaration") {
                    analyzer.extract_variable_declarations(&for_child, source, local_definitions);
                } else if for_child.kind() == "identifier" {
                    if let Some(text) = crate::scope_analyzer::extract_identifier_text(&for_child, source) {
                        local_definitions.insert(text);
                    }
                }
            }
        }
        "for_in_statement" | "for_of_statement" => {
            crate::scope_analyzer::extract_identifiers_from_children(
                node,
                source,
                local_definitions,
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new(),
            );
        }
        _ => {}
    }
}

pub(crate) fn extract_catch_variables(
    node: &Node,
    source: &[u8],
    local_definitions: &mut std::collections::HashSet<String>,
) {
    let mut cursor = node.walk();
    for catch_child in node.children(&mut cursor) {
        if catch_child.kind() == "catch_binding" {
            crate::scope_analyzer::extract_identifiers_from_children(
                &catch_child,
                source,
                local_definitions,
                &std::collections::HashSet::new(),
                &std::collections::HashSet::new(),
            );
        } else if catch_child.kind() == "identifier" {
            if let Some(text) = crate::scope_analyzer::extract_identifier_text(&catch_child, source) {
                local_definitions.insert(text);
            }
        }
    }
}
