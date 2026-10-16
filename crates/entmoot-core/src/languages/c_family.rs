//! Syntax inventory for C/C++; compiler binding stays exclusively in Resolver.
//! Only explicit function bodies are callable inventory. Macro-generated bodies,
//! implicit constructors and template instantiations require compiler evidence.
use std::collections::HashSet;
use tree_sitter::{Node, Tree};

use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::sound_cst::{serialize_sound_span, SoundStructuralProfile};
use crate::visitor::BaseVisitor;
use entmoot_contracts::StructuralHashPolicy;

/// Shared syntax walker; the grammar is selected by the language registry.
pub struct CFamilyNormalizer;

/// Follow declarator fields, never parameter identifiers, to the declared name.
/// Qualified names remain one syntax-owned spelling without semantic resolution.
pub(crate) fn declarator_name(mut node: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..MAX_RECURSIVE_DEPTH {
        if matches!(
            node.kind(),
            "identifier"
                | "field_identifier"
                | "qualified_identifier"
                | "destructor_name"
                | "operator_name"
                | "operator_cast"
        ) {
            return Some(node);
        }
        node = node.child_by_field_name("declarator").or_else(|| {
            // Parenthesized pointer declarators have no named declarator field.
            let mut cursor = node.walk();
            let child = node.named_children(&mut cursor).next();
            child
        })?;
    }
    None
}

fn text(node: Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or_default().into()
}

fn walk(node: Node<'_>, source: &[u8], base: &mut BaseVisitor, depth: usize) -> Result<(), &'static str> {
    if depth > MAX_RECURSIVE_DEPTH {
        return Err("C/C++ inventory exceeded syntax depth");
    }
    let mut entered_scope = false;
    base.record_named_declaration(&node, source);
    let is_type = matches!(node.kind(), "class_specifier" | "struct_specifier" | "union_specifier");
    if is_type || node.kind() == "namespace_definition" {
        if let Some(name) = node.child_by_field_name("name") {
            base.definition_scope.push(text(name, source));
            entered_scope = true;
        }
        if is_type {
            base.type_body_depth += 1;
        }
    }
    let mut callable = false;
    if node.kind() == "function_definition" {
        if let Some(name) = node.child_by_field_name("declarator").and_then(declarator_name) {
            let name_kind = name.kind();
            let name = text(name, source);
            if node.child_by_field_name("type").is_none() {
                let pieces: Vec<_> = name.split("::").collect();
                let constructor = if pieces.len() >= 2 {
                    pieces[pieces.len() - 2] == pieces[pieces.len() - 1].trim_start_matches('~')
                } else {
                    base.type_body_depth > 0
                        && base
                            .definition_scope
                            .last()
                            .is_some_and(|scope| scope == name.trim_start_matches('~'))
                };
                // tree-sitter accepts TEST_F(Suite, Test) { ... } as an untyped
                // function definition. It is a macro invocation, not an owned
                // callable declaration. Reject the file rather than fabricate one.
                if !constructor && name_kind != "operator_cast" {
                    return Err("untyped or macro-shaped C/C++ function; file inventory unavailable");
                }
            }
            base.begin_callable(name, "function", node.start_byte(), node.end_byte());
            if let Some(context) = base.current_callable.as_mut() {
                let mut ancestor = node.parent();
                while let Some(parent) = ancestor.filter(|p| p.kind() == "template_declaration") {
                    let mut cursor = parent.walk();
                    for boundary in parent
                        .children(&mut cursor)
                        .filter(|n| n.end_byte() <= node.start_byte())
                    {
                        context.imports.insert(format!(
                            "template_boundary:{}",
                            serialize_sound_span(source, boundary, &SoundStructuralProfile::for_language("cpp"),)
                        ));
                    }
                    ancestor = parent.parent();
                }
            }
            callable = true;
        } else {
            // Do not silently omit a body that this inventory cannot name.
            return Err("unsupported C/C++ function declarator; file inventory unavailable");
        }
    }
    if node.kind() == "lambda_expression" {
        // A nested lambda must not attribute its calls to the enclosing function.
        // Its source span disambiguates the syntax-owned anonymous occurrence.
        base.begin_callable("<lambda>", "function", node.start_byte(), node.end_byte());
        callable = true;
    }
    if node.kind() == "call_expression" {
        if let Some(function) = node.child_by_field_name("function") {
            base.track_call_node(text(function, source), &node, source);
        }
    }
    if node.kind() == "preproc_include" {
        // Preserve every include token as a syntactic dependency. Macro includes
        // are never converted into a resolved file edge.
        if let Some(path) = node.child_by_field_name("path") {
            base.track_file_import(text(path, source));
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, source, base, depth + 1)?;
    }
    if callable {
        let profile = SoundStructuralProfile::for_language("c");
        let mut cursor = node.walk();
        let signature = node
            .children(&mut cursor)
            .filter(|child| Some(*child) != node.child_by_field_name("body"))
            .map(|child| serialize_sound_span(source, child, &profile))
            .collect::<Vec<_>>()
            .join(" ");
        base.end_callable();
        if let Some(context) = base.callables.last_mut() {
            context.signature_hash = crate::hash::hash_structural_ir(&signature);
            context.signature = format!("({signature})");
        }
    }
    if entered_scope {
        base.definition_scope.pop();
    }
    if is_type {
        base.type_body_depth -= 1;
    }
    Ok(())
}

impl ASTNormalizer for CFamilyNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let mut base = BaseVisitor::new(None, policy.cloned());
        walk(tree.root_node(), source, &mut base, 0)?;
        let (mut callables, imports, markers) = base.take_granular_outputs();
        // All syntactic includes are dependencies; do not guess which body uses
        // a macro or declaration from an include.
        for callable in &mut callables {
            callable.imports.extend(imports.iter().cloned());
        }
        Ok(VisitorOutput {
            callables,
            imports: imports.into_iter().collect::<HashSet<_>>(),
            markers,
        })
    }
}
