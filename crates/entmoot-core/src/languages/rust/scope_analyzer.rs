//! Scope analysis for Rust AST nodes using manual traversal.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{extract_identifier_text, extract_identifiers_from_children, ScopeAnalyzer};

const RESERVED_UNRENAMED: &[&str] = &["self", "Self", "crate", "super", "_"];

/// Rust-specific scope analyzer.
pub struct RustScopeAnalyzer;

impl ScopeAnalyzer for RustScopeAnalyzer {
    fn is_parameter_node(&self, node: &Node, in_function_body: bool) -> bool {
        node.kind() == "parameters" && !in_function_body
    }

    fn is_function_declaration_node(&self, node: &Node) -> bool {
        node.kind() == "function_item"
    }

    fn is_function_body_node(&self, node: &Node) -> bool {
        node.kind() == "block"
    }

    fn is_nested_scope_node(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "impl_item" | "mod_item" | "trait_item" | "function_item" | "closure_expression"
        )
    }

    fn extract_parameters(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "parameter" => {
                    if let Some(name) = child.child_by_field_name("pattern") {
                        collect_pattern_identifiers(&name, source, local_definitions);
                    } else {
                        let mut pc = child.walk();
                        for param_child in child.children(&mut pc) {
                            if param_child.kind() == "identifier" {
                                if let Some(text) = extract_identifier_text(&param_child, source) {
                                    if !RESERVED_UNRENAMED.contains(&text.as_str()) {
                                        local_definitions.insert(text);
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
                "self_parameter" => {
                    // Keep `self` un-renamed — do not register for alpha-rename.
                }
                _ => {}
            }
        }
    }

    fn is_variable_declaration_node(&self, node: &Node) -> bool {
        node.kind() == "let_declaration"
    }

    fn extract_variable_declarations(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        if node.kind() != "let_declaration" {
            return;
        }
        // Pattern is typically the first non-keyword child (identifier or pattern node).
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "let" | "mutable_specifier" | ":" | "=" | ";" => continue,
                "type"
                | "primitive_type"
                | "type_identifier"
                | "reference_type"
                | "generic_type"
                | "array_type"
                | "tuple_type"
                | "scoped_type_identifier"
                | "unit_type" => continue,
                _ => {
                    collect_pattern_identifiers(&child, source, local_definitions);
                    break;
                }
            }
        }
    }

    fn extract_assignment_targets(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        extract_identifiers_from_children(node, source, local_definitions, &HashSet::new(), &HashSet::new());
    }

    fn is_loop_node(&self, node: &Node) -> bool {
        matches!(node.kind(), "for_expression" | "while_expression" | "loop_expression")
    }

    fn extract_loop_variables(&self, node: &Node, source: &[u8], local_definitions: &mut HashSet<String>) {
        if node.kind() != "for_expression" {
            return;
        }
        if let Some(pattern) = node.child_by_field_name("pattern") {
            collect_pattern_identifiers(&pattern, source, local_definitions);
            return;
        }
        // Fallback: first identifier / pattern-like child after `for`.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "for" | "in" => continue,
                "block" => break,
                "identifier" => {
                    collect_pattern_identifiers(&child, source, local_definitions);
                    break;
                }
                k if is_pattern_like(k) => {
                    collect_pattern_identifiers(&child, source, local_definitions);
                    break;
                }
                _ => break,
            }
        }
    }

    fn is_catch_node(&self, _node: &Node) -> bool {
        false
    }

    fn extract_language_specific_definitions(
        &self,
        node: &Node,
        source: &[u8],
        local_definitions: &mut HashSet<String>,
        _global_declarations: &mut HashSet<String>,
    ) {
        // Match arm patterns that bind identifiers are locals.
        if node.kind() == "match_pattern" || node.kind() == "match_arm" {
            if node.kind() == "match_pattern" {
                collect_match_pattern_bindings(node, source, local_definitions);
            } else {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "match_pattern" {
                        collect_match_pattern_bindings(&child, source, local_definitions);
                    }
                }
            }
        }
    }
}

fn is_pattern_like(kind: &str) -> bool {
    matches!(
        kind,
        "tuple_pattern"
            | "slice_pattern"
            | "struct_pattern"
            | "remaining_field_pattern"
            | "mut_pattern"
            | "ref_pattern"
            | "or_pattern"
            | "range_pattern"
            | "identifier"
            | "_"
    )
}

fn collect_pattern_identifiers(node: &Node, source: &[u8], out: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            if let Some(text) = extract_identifier_text(node, source) {
                if !RESERVED_UNRENAMED.contains(&text.as_str()) {
                    out.insert(text);
                }
            }
        }
        "_" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "primitive_type" | "type_identifier" | "scoped_type_identifier" | "generic_type" | "reference_type"
                ) {
                    continue;
                }
                collect_pattern_identifiers(&child, source, out);
            }
        }
    }
}

fn collect_match_pattern_bindings(node: &Node, source: &[u8], out: &mut HashSet<String>) {
    // Skip guard expressions (`if ...`) — only bind from the pattern side.
    let mut cursor = node.walk();
    let mut past_if = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "if" {
            past_if = true;
            continue;
        }
        if past_if {
            continue;
        }
        collect_pattern_identifiers(&child, source, out);
    }
}

/// Analyze function scope to find locally-defined identifiers.
pub fn analyze_function_scope(node: &Node, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let analyzer = RustScopeAnalyzer;
    let (locals, globals): (Vec<String>, Vec<String>) = analyzer.analyze_scope(node, source);
    (locals.into_iter().collect(), globals.into_iter().collect())
}
