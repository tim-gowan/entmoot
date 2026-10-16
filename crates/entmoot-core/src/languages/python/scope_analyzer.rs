//! Python scope analyzer.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{extract_identifier_text, ScopeAnalyzer};

pub struct PythonScopeAnalyzer;

impl ScopeAnalyzer for PythonScopeAnalyzer {
    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool {
        matches!(node.kind(), "parameters" | "lambda_parameters") && !in_function_body
    }

    fn is_function_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "function_definition" | "lambda")
    }

    fn is_function_body_node(&self, node: &Node) -> bool {
        node.kind() == "block"
    }

    fn is_nested_scope_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "class_definition" | "function_definition" | "lambda")
    }

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for param in node.children(&mut cursor) {
            match param.kind() {
                "identifier" => {
                    if let Some(text) = extract_identifier_text(&param, source) {
                        if text != "self" {
                            local_definitions.insert(text);
                        }
                    }
                }
                "typed_parameter" => {
                    let mut pc = param.walk();
                    for param_child in param.children(&mut pc) {
                        if let Some(text) = extract_identifier_text(&param_child, source) {
                            if text != "self" {
                                local_definitions.insert(text);
                            }
                        }
                    }
                }
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    let mut pc = param.walk();
                    for param_child in param.children(&mut pc) {
                        if param_child.kind() == "identifier" {
                            if let Some(text) = extract_identifier_text(&param_child, source) {
                                local_definitions.insert(text);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn is_global_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "global_statement" | "nonlocal_statement")
    }

    fn extract_global_declarations(&self, node: &Node, source: &[u8], global_declarations: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for global_child in node.children(&mut cursor) {
            if global_child.kind() == "identifier" {
                if let Some(text) = extract_identifier_text(&global_child, source) {
                    global_declarations.insert(text);
                }
            } else if global_child.kind() == "dotted_name" {
                if let Ok(name) = global_child.utf8_text(source) {
                    if let Some(first) = name.split('.').next() {
                        global_declarations.insert(first.to_string());
                    }
                }
            }
        }
    }

    fn extract_assignment_targets(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let skip_self: HashSet<&str> = HashSet::from(["self"]);
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for (i, assign_child) in children.iter().enumerate() {
            if assign_child.kind() == "=" && i > 0 {
                let lhs = &children[i - 1];
                match lhs.kind() {
                    "identifier" => {
                        if let Some(text) = extract_identifier_text(lhs, source) {
                            if text != "self" {
                                local_definitions.insert(text);
                            }
                        }
                    }
                    "tuple" | "pattern_list" | "list" => {
                        crate::scope_analyzer::extract_identifiers_from_children(
                            lhs,
                            source,
                            local_definitions,
                            &HashSet::new(),
                            &skip_self,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn is_loop_node(&self, node: &Node) -> bool {
        node.kind() == "for_in_statement"
    }

    fn extract_loop_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for for_child in node.children(&mut cursor) {
            if for_child.kind() == "identifier" {
                if let Some(text) = extract_identifier_text(&for_child, source) {
                    local_definitions.insert(text);
                }
            } else if matches!(for_child.kind(), "tuple" | "list") {
                crate::scope_analyzer::extract_identifiers_from_children(
                    &for_child,
                    source,
                    local_definitions,
                    &HashSet::new(),
                    &HashSet::new(),
                );
            }
        }
    }

    fn extract_language_specific_definitions(
        &self,
        node: &Node,
        source: &[u8],
        local_definitions: &mut HashSet<String>,
        _global_declarations: &mut HashSet<String>,
    ) {
        match node.kind() {
            "list_comprehension" | "dictionary_comprehension" | "set_comprehension" | "generator_expression" => {
                extract_comprehension_variables(node, source, local_definitions);
            }
            "with_statement" => extract_with_statement_variables(node, source, local_definitions),
            "lambda" => {
                let skip: HashSet<&str> = HashSet::new();
                let mut cursor = node.walk();
                for lambda_child in node.children(&mut cursor) {
                    if lambda_child.kind() == "lambda_parameters" {
                        crate::scope_analyzer::extract_identifiers_from_children(
                            &lambda_child,
                            source,
                            local_definitions,
                            &HashSet::new(),
                            &skip,
                        );
                    }
                }
            }
            "named_expression" => extract_walrus_variables(node, source, local_definitions),
            _ => {}
        }
    }
}

fn extract_comprehension_variables(node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for comp_child in node.children(&mut cursor) {
        if comp_child.kind() == "for_in_clause" {
            let mut cc = comp_child.walk();
            for clause_child in comp_child.children(&mut cc) {
                if clause_child.kind() == "identifier" {
                    if let Some(text) = extract_identifier_text(&clause_child, source) {
                        local_definitions.insert(text);
                    }
                } else if matches!(clause_child.kind(), "tuple" | "list") {
                    crate::scope_analyzer::extract_identifiers_from_children(
                        &clause_child,
                        source,
                        local_definitions,
                        &HashSet::new(),
                        &HashSet::new(),
                    );
                }
            }
        }
    }
}

fn extract_with_statement_variables(node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for with_child in node.children(&mut cursor) {
        if with_child.kind() == "with_item" {
            let mut ic = with_child.walk();
            let items: Vec<Node> = with_child.children(&mut ic).collect();
            for (i, item_child) in items.iter().enumerate() {
                if item_child.kind() == "identifier" && i > 0 && items[i - 1].kind() == "as" {
                    if let Some(text) = extract_identifier_text(item_child, source) {
                        local_definitions.insert(text);
                    }
                }
            }
        }
    }
}

fn extract_walrus_variables(node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for named_child in node.children(&mut cursor) {
        if named_child.kind() == "identifier" {
            if let Some(text) = extract_identifier_text(&named_child, source) {
                if text != "self" {
                    local_definitions.insert(text);
                }
            }
            break;
        }
    }
}

/// Analyze function scope for locally-defined identifiers.
pub fn analyze_function_scope(node: &Node, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let analyzer = PythonScopeAnalyzer;
    let (locals, globals): (Vec<String>, Vec<String>) = analyzer.analyze_scope(node, source);
    (locals.into_iter().collect(), globals.into_iter().collect())
}
