//! Scope analysis for Go AST nodes using manual traversal.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{extract_identifier_text, extract_identifiers_from_children, ScopeAnalyzer};

/// Go-specific scope analyzer.
pub struct GoScopeAnalyzer;

impl ScopeAnalyzer for GoScopeAnalyzer {
    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool {
        node.kind() == "parameter_list" && !in_function_body
    }

    fn is_function_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "function_declaration" | "method_declaration")
    }

    fn is_function_body_node(&self, node: &Node) -> bool {
        node.kind() == "block"
    }

    fn is_nested_scope_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "type_declaration" | "package_clause" | "function_declaration" | "method_declaration" | "func_literal"
        )
    }

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for param in node.children(&mut cursor) {
            match param.kind() {
                "parameter_declaration" => {
                    let mut pc = param.walk();
                    for param_child in param.children(&mut pc) {
                        if let Some(text) = extract_identifier_text(&param_child, source) {
                            local_definitions.insert(text);
                        }
                    }
                }
                "variadic_parameter_declaration" => {
                    if let Some(name_node) = param.child_by_field_name("name") {
                        if let Some(text) = extract_identifier_text(&name_node, source) {
                            local_definitions.insert(text);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn is_variable_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "short_var_declaration" | "var_declaration")
    }

    fn extract_variable_declarations(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        match node.kind() {
            "short_var_declaration" => extract_short_var_declaration(node, source, local_definitions),
            "var_declaration" => extract_var_declaration(node, source, local_definitions),
            _ => {}
        }
    }

    fn is_loop_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "for_statement" | "range_clause")
    }

    fn extract_loop_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        match node.kind() {
            "range_clause" => {
                let mut cursor = node.walk();
                for range_child in node.children(&mut cursor) {
                    if range_child.kind() == "expression_list" {
                        extract_identifiers_from_children(
                            &range_child,
                            source,
                            local_definitions,
                            &HashSet::new(),
                            &HashSet::new(),
                        );
                    } else if range_child.kind() == "identifier" {
                        if let Some(text) = extract_identifier_text(&range_child, source) {
                            local_definitions.insert(text);
                        }
                    }
                }
            }
            "for_statement" => {
                let mut cursor = node.walk();
                for for_child in node.children(&mut cursor) {
                    if for_child.kind() == "short_var_declaration" {
                        self.extract_variable_declarations(&for_child, source, local_definitions);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_short_var_declaration(node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for decl_child in node.children(&mut cursor) {
        if decl_child.kind() == "identifier" {
            if let Some(text) = extract_identifier_text(&decl_child, source) {
                local_definitions.insert(text);
            }
        } else if decl_child.kind() == "expression_list" {
            extract_identifiers_from_children(&decl_child, source, local_definitions, &HashSet::new(), &HashSet::new());
        }
    }
}

fn extract_var_declaration(node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for decl_child in node.children(&mut cursor) {
        if decl_child.kind() == "var_spec" {
            extract_var_spec(&decl_child, source, local_definitions);
        }
    }
}

fn extract_var_spec(node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for spec_child in node.children(&mut cursor) {
        if spec_child.kind() == "identifier" {
            if let Some(text) = extract_identifier_text(&spec_child, source) {
                local_definitions.insert(text);
            }
        } else if spec_child.kind() == "identifier_list" {
            extract_identifiers_from_children(&spec_child, source, local_definitions, &HashSet::new(), &HashSet::new());
        }
    }
}

/// Analyze function scope to find locally-defined identifiers.
pub fn analyze_function_scope(node: &Node, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let analyzer = GoScopeAnalyzer;
    let (locals, globals): (Vec<String>, Vec<String>) = analyzer.analyze_scope(node, source);
    (locals.into_iter().collect(), globals.into_iter().collect())
}
