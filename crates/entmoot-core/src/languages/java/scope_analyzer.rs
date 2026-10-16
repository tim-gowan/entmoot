//! Scope analysis for Java AST nodes using manual traversal.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{extract_identifier_text, extract_identifiers_from_children, ScopeAnalyzer};

/// Java-specific scope analyzer.
pub struct JavaScopeAnalyzer;

/// Declared parameter identifier in the qualified Orchard CST. The spread
/// parameter's name belongs to its variable declarator; annotations/type names
/// are not parameter bindings. Unnamed patterns intentionally have no binding.
pub(super) fn parameter_name<'tree>(node: &Node<'tree>) -> Option<Node<'tree>> {
    let name = match node.kind() {
        "formal_parameter" => node.child_by_field_name("name"),
        "spread_parameter" => {
            let mut cursor = node.walk();
            let declarator = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "variable_declarator");
            declarator.and_then(|child| child.child_by_field_name("name"))
        }
        _ => None,
    }?;
    (name.kind() == "identifier").then_some(name)
}

impl ScopeAnalyzer for JavaScopeAnalyzer {
    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool {
        node.kind() == "formal_parameters" && !in_function_body
    }

    fn is_function_declaration_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "method_declaration" | "constructor_declaration")
    }

    fn is_function_body_node(&self, node: &Node) -> bool {
        node.kind() == "block"
    }

    fn is_nested_scope_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "annotation_type_declaration"
                | "method_declaration"
                | "constructor_declaration"
                | "lambda_expression"
        )
    }

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for param in node.children(&mut cursor) {
            if let Some(name) = parameter_name(&param) {
                if let Some(text) = extract_identifier_text(&name, source) {
                    local_definitions.insert(text);
                }
            }
        }
    }

    fn is_variable_declaration_node(&self, node: &Node) -> bool {
        node.kind() == "local_variable_declaration"
    }

    fn extract_variable_declarations(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for decl_child in node.children(&mut cursor) {
            if decl_child.kind() == "variable_declarator" {
                let mut vc = decl_child.walk();
                for var_child in decl_child.children(&mut vc) {
                    if let Some(text) = extract_identifier_text(&var_child, source) {
                        local_definitions.insert(text);
                    }
                }
            }
        }
    }

    fn extract_assignment_targets(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
    }

    fn is_update_expression_node(&self, node: &Node) -> bool {
        node.kind() == "update_expression"
    }

    fn is_loop_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "for_statement" | "enhanced_for_statement")
    }

    fn extract_loop_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        match node.kind() {
            "for_statement" => {
                let mut cursor = node.walk();
                for for_child in node.children(&mut cursor) {
                    if for_child.kind() == "local_variable_declaration" {
                        self.extract_variable_declarations(&for_child, source, local_definitions);
                    }
                }
            }
            "enhanced_for_statement" => {
                extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
            }
            _ => {}
        }
    }

    fn extract_catch_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for catch_child in node.children(&mut cursor) {
            if catch_child.kind() == "catch_formal_parameter" {
                extract_identifiers_from_children(
                    &catch_child,
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
        if node.kind() == "instanceof_expression" {
            let mut found_type = false;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    found_type = true;
                } else if child.kind() == "identifier" && found_type {
                    if let Some(text) = extract_identifier_text(&child, source) {
                        local_definitions.insert(text);
                    }
                }
            }
        }

        if node.kind() == "resource_specification" {
            let mut cursor = node.walk();
            for resource_child in node.children(&mut cursor) {
                if resource_child.kind() == "resource" {
                    extract_identifiers_from_children(
                        &resource_child,
                        source,
                        local_definitions,
                        &HashSet::new(),
                        &HashSet::new(),
                    );
                } else if resource_child.kind() == "local_variable_declaration" {
                    self.extract_variable_declarations(&resource_child, source, local_definitions);
                }
            }
        }
    }
}

/// Analyze method scope to find locally-defined identifiers.
pub fn analyze_method_scope(node: &Node, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let analyzer = JavaScopeAnalyzer;
    let (locals, globals): (Vec<String>, Vec<String>) = analyzer.analyze_scope(node, source);
    (locals.into_iter().collect(), globals.into_iter().collect())
}
