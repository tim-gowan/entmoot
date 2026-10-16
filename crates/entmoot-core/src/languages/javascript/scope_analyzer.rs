//! Scope analysis for JavaScript AST nodes using manual traversal.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{extract_identifier_text, extract_identifiers_from_children, ScopeAnalyzer};

const TYPE_IDENTIFIERS: &[&str] = &["number", "string", "boolean", "any", "void", "object"];

/// JavaScript-specific scope analyzer.
pub struct JavaScriptScopeAnalyzer;

impl ScopeAnalyzer for JavaScriptScopeAnalyzer {
    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool {
        matches!(node.kind(), "formal_parameters" | "parameters") && !in_function_body
    }

    fn is_function_declaration_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "function_declaration"
                | "function"
                | "arrow_function"
                | "method_definition"
                | "generator_function"
                | "generator_function_declaration"
        )
    }

    fn is_function_body_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "statement_block" | "expression")
    }

    fn is_nested_scope_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "class_declaration"
                | "function_declaration"
                | "function"
                | "arrow_function"
                | "method_definition"
                | "generator_function"
                | "generator_function_declaration"
        )
    }

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for param in node.children(&mut cursor) {
            if should_skip_parameter_node(&param, source) {
                continue;
            }

            match param.kind() {
                "identifier" => {
                    if let Some(text) = extract_identifier_text(&param, source) {
                        local_definitions.insert(text);
                    }
                }
                "ERROR" => extract_error_parameter(&param, source, local_definitions),
                "required_parameter" | "optional_parameter" | "rest_parameter" | "pattern" => {
                    extract_structured_parameter(&param, source, local_definitions);
                }
                _ => {}
            }
        }
    }

    fn is_variable_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "lexical_declaration" | "variable_declaration")
    }

    fn extract_variable_declarations(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for decl_child in node.children(&mut cursor) {
            match decl_child.kind() {
                "variable_declarator" => {
                    let mut vc = decl_child.walk();
                    for var_child in decl_child.children(&mut vc) {
                        if var_child.kind() == "identifier" {
                            if let Some(text) = extract_identifier_text(&var_child, source) {
                                local_definitions.insert(text);
                            }
                        } else if matches!(var_child.kind(), "object_pattern" | "array_pattern") {
                            extract_identifiers_from_children(
                                &var_child,
                                source,
                                local_definitions,
                                &HashSet::new(),
                                &HashSet::new(),
                            );
                        }
                    }
                }
                "type_annotation" | "type_identifier" => {}
                _ => {}
            }
        }
    }

    fn extract_assignment_targets(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
    }

    fn is_update_expression_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "update_expression"
                | "pre_increment_expression"
                | "post_increment_expression"
                | "pre_decrement_expression"
                | "post_decrement_expression"
        )
    }

    fn is_loop_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "for_statement" | "for_in_statement" | "for_of_statement")
    }

    fn extract_loop_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        crate::languages::ecmascript::extract_loop_variables(self, node, source, local_definitions);
    }

    fn extract_catch_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        crate::languages::ecmascript::extract_catch_variables(node, source, local_definitions);
    }
}

fn should_skip_parameter_node(param: &Node, source: &[u8]) -> bool {
    if matches!(param.kind(), "(" | ")" | ",") {
        return true;
    }

    if param.kind() == "identifier" {
        if let Ok(text) = param.utf8_text(source) {
            if TYPE_IDENTIFIERS.contains(&text) {
                return true;
            }
        }
    }

    false
}

fn extract_error_parameter(param: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut found_colon = false;
    let mut cursor = param.walk();
    for error_child in param.children(&mut cursor) {
        if error_child.kind() == ":" {
            found_colon = true;
        } else if error_child.kind() == "identifier" && !found_colon {
            if let Some(text) = extract_identifier_text(&error_child, source) {
                local_definitions.insert(text);
            }
        }
    }
}

fn extract_structured_parameter(param: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
    let mut cursor = param.walk();
    for param_child in param.children(&mut cursor) {
        if param_child.kind() == "identifier" {
            if let Some(text) = extract_identifier_text(&param_child, source) {
                local_definitions.insert(text);
            }
        } else if matches!(param_child.kind(), "object_pattern" | "array_pattern") {
            extract_identifiers_from_children(
                &param_child,
                source,
                local_definitions,
                &HashSet::new(),
                &HashSet::new(),
            );
        }
    }
}

/// Analyze function scope to find locally-defined identifiers.
pub fn analyze_function_scope(node: &Node, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let analyzer = JavaScriptScopeAnalyzer;
    let (locals, globals): (Vec<String>, Vec<String>) = analyzer.analyze_scope(node, source);
    (locals.into_iter().collect(), globals.into_iter().collect())
}
