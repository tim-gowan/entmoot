//! Scope analysis for C# AST nodes using manual traversal.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{extract_identifier_text, extract_identifiers_from_children, ScopeAnalyzer};

/// C#-specific scope analyzer.
pub struct CsharpScopeAnalyzer;

impl ScopeAnalyzer for CsharpScopeAnalyzer {
    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool {
        node.kind() == "parameter_list" && !in_function_body
    }

    fn is_function_declaration_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration" | "accessor_declaration" | "local_function_statement"
        )
    }

    fn is_function_body_node(&self, node: &Node) -> bool {
        node.kind() == "block"
    }

    fn is_nested_scope_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "class_declaration"
                | "struct_declaration"
                | "interface_declaration"
                | "namespace_declaration"
                | "file_scoped_namespace_declaration"
                | "method_declaration"
                | "constructor_declaration"
                | "accessor_declaration"
                | "local_function_statement"
                | "lambda_expression"
                | "anonymous_method_expression"
        )
    }

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for param in node.children(&mut cursor) {
            if param.kind() != "parameter" {
                continue;
            }
            if let Some(name) = param.child_by_field_name("name") {
                if let Some(text) = extract_identifier_text(&name, source) {
                    local_definitions.insert(text);
                }
            } else {
                let mut pc = param.walk();
                for param_child in param.children(&mut pc) {
                    if param_child.kind() == "identifier" {
                        if let Some(text) = extract_identifier_text(&param_child, source) {
                            local_definitions.insert(text);
                        }
                        break;
                    }
                }
            }
        }
    }

    fn is_variable_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "local_declaration_statement" | "variable_declaration")
    }

    fn extract_variable_declarations(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        match node.kind() {
            "local_declaration_statement" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "variable_declaration" {
                        self.extract_variable_declarations(&child, source, local_definitions);
                    }
                }
            }
            "variable_declaration" => {
                let mut cursor = node.walk();
                for decl_child in node.children(&mut cursor) {
                    if decl_child.kind() == "variable_declarator" {
                        if let Some(name) = decl_child.child_by_field_name("name") {
                            if let Some(text) = extract_identifier_text(&name, source) {
                                local_definitions.insert(text);
                            }
                        } else {
                            let mut vc = decl_child.walk();
                            for var_child in decl_child.children(&mut vc) {
                                if var_child.kind() == "identifier" {
                                    if let Some(text) = extract_identifier_text(&var_child, source) {
                                        local_definitions.insert(text);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_assignment_targets(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
    }

    fn is_loop_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "for_statement" | "foreach_statement")
    }

    fn extract_loop_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        match node.kind() {
            "for_statement" => {
                let mut cursor = node.walk();
                for for_child in node.children(&mut cursor) {
                    if matches!(for_child.kind(), "local_declaration_statement" | "variable_declaration") {
                        self.extract_variable_declarations(&for_child, source, local_definitions);
                    }
                }
            }
            "foreach_statement" => {
                if let Some(left) = node.child_by_field_name("left") {
                    if let Some(text) = extract_identifier_text(&left, source) {
                        local_definitions.insert(text);
                    }
                } else {
                    extract_identifiers_from_children(
                        node,
                        source,
                        local_definitions,
                        &HashSet::new(),
                        &HashSet::new(),
                    );
                }
            }
            _ => {}
        }
    }

    fn is_catch_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "catch_clause" | "catch_declaration")
    }

    fn extract_catch_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        match node.kind() {
            "catch_declaration" => {
                // Catch variable is typically the last identifier child.
                let mut last_ident = None;
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        last_ident = extract_identifier_text(&child, source);
                    }
                }
                if let Some(text) = last_ident {
                    local_definitions.insert(text);
                }
            }
            "catch_clause" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "catch_declaration" {
                        self.extract_catch_variables(&child, source, local_definitions);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Analyze method/constructor scope to find locally-defined identifiers.
pub fn analyze_method_scope(node: &Node, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let analyzer = CsharpScopeAnalyzer;
    let (locals, globals): (Vec<String>, Vec<String>) = analyzer.analyze_scope(node, source);
    (locals.into_iter().collect(), globals.into_iter().collect())
}
