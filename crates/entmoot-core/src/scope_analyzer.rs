//! Shared scope analysis for language plugins.

use std::collections::HashSet;

use tree_sitter::Node;

/// Language-specific scope analysis hooks.
pub trait ScopeAnalyzer: Send + Sync {
    fn analyze_scope(&self, node: &Node, source: &[u8]) -> (Vec<String>, Vec<String>)
    where
        Self: Sized,
    {
        analyze_scope_for_node(self, node, source)
    }

    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool;

    fn is_function_declaration_node(&self, node: &Node) -> bool;

    fn is_function_body_node(&self, node: &Node) -> bool;

    fn is_nested_scope_node(&self, node: &Node) -> bool;

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>);

    fn is_global_declaration_node(&self, _node: &Node) -> bool {
        false
    }

    fn extract_global_declarations(&self, _node: &Node, _source: &[u8], _global_declarations: &mut HashSet<String>) {}

    fn is_assignment_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "assignment" | "assignment_expression")
    }

    fn extract_assignment_targets(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        if let Some(first) = node.child(0) {
            if let Some(text) = extract_identifier_text(&first, source) {
                local_definitions.insert(text);
            }
        }
    }

    fn is_variable_declaration_node(&self, _node: &Node) -> bool {
        false
    }

    fn extract_variable_declarations(&self, _node: &Node, _source: &[u8], _local_definitions: &mut HashSet<String>) {}

    fn is_loop_node(&self, node: &Node) -> bool {
        let _ = node;
        false
    }

    fn extract_loop_variables(&self, _node: &Node, _source: &[u8], _local_definitions: &mut HashSet<String>) {}

    fn is_catch_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "catch_clause" | "except_clause")
    }

    fn extract_catch_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
    }

    fn is_update_expression_node(&self, _node: &Node) -> bool {
        false
    }

    fn extract_update_expression_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
    }

    fn extract_language_specific_definitions(
        &self,
        _node: &Node,
        _source: &[u8],
        _local_definitions: &mut HashSet<String>,
        _global_declarations: &mut HashSet<String>,
    ) {
    }
}

/// Run scope analysis through a trait object (for centralized callable preflight).
pub fn analyze_scope_for_node(analyzer: &dyn ScopeAnalyzer, node: &Node, source: &[u8]) -> (Vec<String>, Vec<String>) {
    let mut local_definitions = HashSet::new();
    let mut global_declarations = HashSet::new();
    analyze_scope_recursive(
        analyzer,
        node,
        source,
        false,
        &mut local_definitions,
        &mut global_declarations,
    );
    (
        local_definitions.into_iter().collect(),
        global_declarations.into_iter().collect(),
    )
}

pub(crate) fn extract_identifier_text(node: &Node, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return node.utf8_text(source).ok().map(str::to_string);
    }
    None
}

fn analyze_scope_recursive(
    analyzer: &dyn ScopeAnalyzer,
    node: &Node,
    source: &[u8],
    in_function_body: bool,
    local_definitions: &mut HashSet<String>,
    global_declarations: &mut HashSet<String>,
) {
    // Nested functions/classes must not merge params/locals into the outer scope.
    // Check before any extraction so lambdas/nested defs cannot leak identifiers.
    if in_function_body && analyzer.is_nested_scope_node(node) {
        return;
    }

    if analyzer.is_parameter_node(node, in_function_body) {
        analyzer.extract_parameters(node, source, local_definitions);
    }

    if analyzer.is_global_declaration_node(node) {
        analyzer.extract_global_declarations(node, source, global_declarations);
    }

    if in_function_body {
        process_function_body_variables(analyzer, node, source, local_definitions);
        analyzer.extract_language_specific_definitions(node, source, local_definitions, global_declarations);
    }

    if analyzer.is_function_declaration_node(node) {
        process_function_declaration(analyzer, node, source, local_definitions, global_declarations);
        return;
    }

    if analyzer.is_nested_scope_node(node) {
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        analyze_scope_recursive(
            analyzer,
            &child,
            source,
            in_function_body,
            local_definitions,
            global_declarations,
        );
    }
}

fn process_function_body_variables(
    analyzer: &dyn ScopeAnalyzer,
    node: &Node,
    source: &[u8],
    local_definitions: &mut HashSet<String>,
) {
    if analyzer.is_assignment_node(node) {
        analyzer.extract_assignment_targets(node, source, local_definitions);
    } else if analyzer.is_variable_declaration_node(node) {
        analyzer.extract_variable_declarations(node, source, local_definitions);
    } else if analyzer.is_loop_node(node) {
        analyzer.extract_loop_variables(node, source, local_definitions);
    } else if analyzer.is_catch_node(node) {
        analyzer.extract_catch_variables(node, source, local_definitions);
    } else if analyzer.is_update_expression_node(node) {
        analyzer.extract_update_expression_variables(node, source, local_definitions);
    }
}

fn process_function_declaration(
    analyzer: &dyn ScopeAnalyzer,
    node: &Node,
    source: &[u8],
    local_definitions: &mut HashSet<String>,
    global_declarations: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if analyzer.is_parameter_node(&child, false) {
            analyze_scope_recursive(analyzer, &child, source, false, local_definitions, global_declarations);
        } else if analyzer.is_function_body_node(&child) {
            analyze_scope_recursive(analyzer, &child, source, true, local_definitions, global_declarations);
        } else {
            analyze_scope_recursive(analyzer, &child, source, false, local_definitions, global_declarations);
        }
    }
}

pub(crate) fn extract_identifiers_from_children(
    node: &Node,
    source: &[u8],
    target_set: &mut HashSet<String>,
    skip_types: &HashSet<&str>,
    skip_values: &HashSet<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(text) = extract_identifier_text(&child, source) {
            if !skip_values.contains(text.as_str()) {
                target_set.insert(text);
            }
        } else if !skip_types.contains(child.kind()) {
            extract_identifiers_from_children(&child, source, target_set, skip_types, skip_values);
        }
    }
}
