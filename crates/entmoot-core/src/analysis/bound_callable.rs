//! Resolve inventory names for callables bound via assignment / declarator.
//!
//! Only expressions that are the binder's value (modulo transparent wrappers like
//! parentheses) get an LHS name — not arrows nested in arrays, calls, etc.

use tree_sitter::Node;

/// Which ancestor AST kinds count as binders for a bound callable expression.
#[derive(Debug, Clone, Copy)]
pub struct BinderSpec {
    pub binder_kinds: &'static [&'static str],
    /// Hops allowed between the expression and the binder (parentheses, etc.).
    pub transparent_kinds: &'static [&'static str],
    pub max_parent_hops: usize,
}

impl BinderSpec {
    pub const DEFAULT_MAX_HOPS: usize = 8;

    pub const fn new(binder_kinds: &'static [&'static str], transparent_kinds: &'static [&'static str]) -> Self {
        Self {
            binder_kinds,
            transparent_kinds,
            max_parent_hops: Self::DEFAULT_MAX_HOPS,
        }
    }
}

/// JS/TS variable and assignment binders.
pub const JS_VARIABLE_DECLARATOR: BinderSpec = BinderSpec::new(
    &["variable_declarator", "assignment_expression"],
    &[
        "parenthesized_expression",
        "as_expression",
        "satisfies_expression",
        "non_null_expression",
        "type_assertion",
    ],
);

/// Python `name = <expr>`.
pub const PYTHON_ASSIGNMENT: BinderSpec = BinderSpec::new(&["assignment"], &["parenthesized_expression"]);

/// Source identity of a simple binding target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundName {
    pub name: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Name from the grammar's declaration field, never a keyword/spelling filter.
/// Computed/string property names remain outside this identifier-only inventory.
pub fn declared_identifier(node: Node<'_>, source: &[u8]) -> Option<String> {
    let name = node.child_by_field_name("name")?;
    if !matches!(
        name.kind(),
        "identifier" | "property_identifier" | "private_property_identifier" | "type_identifier"
    ) {
        return None;
    }
    let text = name.utf8_text(source).ok()?;
    (!text.is_empty()).then(|| text.to_owned())
}

/// Walk ancestors looking for a binder; return LHS name via `lhs_identifier`.
///
/// Returns `None` if the expression is nested under a non-transparent container
/// before the binder (e.g. array element, call argument).
pub fn bound_name_from_ancestors(
    node: Node<'_>,
    source: &[u8],
    spec: BinderSpec,
    lhs_identifier: impl Fn(Node<'_>, &[u8]) -> Option<String>,
) -> Option<String> {
    let mut current = node;
    for _ in 0..spec.max_parent_hops {
        let parent = current.parent()?;
        if spec.binder_kinds.iter().any(|k| parent.kind() == *k) {
            if parent.kind() == "assignment_expression" && parent.child_by_field_name("right") != Some(current) {
                return None;
            }
            return lhs_identifier(parent, source);
        }
        if !spec.transparent_kinds.iter().any(|k| parent.kind() == *k) {
            return None;
        }
        current = parent;
    }
    None
}

/// Walk ancestors like [`bound_name_from_ancestors`], retaining the exact
/// declaration span returned by `lhs`.
pub fn bound_site_from_ancestors(
    node: Node<'_>,
    source: &[u8],
    spec: BinderSpec,
    lhs: impl Fn(Node<'_>, &[u8]) -> Option<BoundName>,
) -> Option<BoundName> {
    let mut current = node;
    for _ in 0..spec.max_parent_hops {
        let parent = current.parent()?;
        if spec.binder_kinds.iter().any(|kind| parent.kind() == *kind) {
            return lhs(parent, source);
        }
        if !spec.transparent_kinds.iter().any(|kind| parent.kind() == *kind) {
            return None;
        }
        current = parent;
    }
    None
}

/// First simple `identifier` child of a binder (JS/TS `variable_declarator`).
pub fn first_identifier_child(binder: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = binder.walk();
    for child in binder.children(&mut cursor) {
        if child.kind() == "identifier" {
            let text = child.utf8_text(source).ok()?.to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// A static JS/TS assignment spelling is an inventory label, not a dispatch target.
/// Reject computed receivers and calls; the bound expression must be the RHS.
pub fn javascript_binding_lhs(binder: Node<'_>, source: &[u8]) -> Option<String> {
    let field = if binder.kind() == "variable_declarator" {
        "name"
    } else {
        "left"
    };
    let lhs = binder.child_by_field_name(field)?;
    fn static_name(node: Node<'_>, source: &[u8], depth: usize) -> Option<String> {
        if depth >= BinderSpec::DEFAULT_MAX_HOPS {
            return None;
        }
        match node.kind() {
            "identifier" | "this" | "super" => Some(node.utf8_text(source).ok()?.into()),
            "member_expression" => {
                let object = static_name(node.child_by_field_name("object")?, source, depth + 1)?;
                let property = node.child_by_field_name("property")?;
                if property.kind() != "property_identifier" {
                    return None;
                }
                Some(format!("{object}.{}", property.utf8_text(source).ok()?))
            }
            _ => None,
        }
    }
    static_name(lhs, source, 0)
}

/// Python assignment LHS: only a bare identifier (skip attribute / subscript / unpack).
pub fn python_simple_assignment_lhs(binder: Node<'_>, source: &[u8]) -> Option<String> {
    python_simple_assignment_lhs_site(binder, source).map(|site| site.name)
}

/// Python assignment LHS with its exact source span.
pub fn python_simple_assignment_lhs_site(binder: Node<'_>, source: &[u8]) -> Option<BoundName> {
    let mut cursor = binder.walk();
    let children: Vec<_> = binder.children(&mut cursor).collect();
    let left = children.first()?;
    if left.kind() != "identifier" {
        return None;
    }
    let name = left.utf8_text(source).ok()?.to_string();
    if name.is_empty() {
        None
    } else {
        Some(BoundName {
            name,
            start_byte: left.start_byte(),
            end_byte: left.end_byte(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser.set_language(&crate::grammars::JAVASCRIPT.into()).unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_py(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser.set_language(&crate::grammars::PYTHON.into()).unwrap();
        parser.parse(source, None).unwrap()
    }

    fn find_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn js_export_const_arrow_resolves_lhs() {
        let source = "export const handler = (req) => { return req; };\n";
        let tree = parse_js(source);
        let arrow = find_kind(tree.root_node(), "arrow_function").unwrap();
        let name = bound_name_from_ancestors(arrow, source.as_bytes(), JS_VARIABLE_DECLARATOR, first_identifier_child);
        assert_eq!(name.as_deref(), Some("handler"));
    }

    #[test]
    fn js_parenthesized_arrow_resolves_lhs() {
        let source = "const handler = (() => 1);\n";
        let tree = parse_js(source);
        let arrow = find_kind(tree.root_node(), "arrow_function").unwrap();
        let name = bound_name_from_ancestors(arrow, source.as_bytes(), JS_VARIABLE_DECLARATOR, first_identifier_child);
        assert_eq!(name.as_deref(), Some("handler"));
    }

    #[test]
    fn js_unbound_arrow_in_array_has_no_name() {
        let source = "const xs = [() => 1];\n";
        let tree = parse_js(source);
        let arrow = find_kind(tree.root_node(), "arrow_function").unwrap();
        let name = bound_name_from_ancestors(arrow, source.as_bytes(), JS_VARIABLE_DECLARATOR, first_identifier_child);
        assert_eq!(name, None);
    }

    #[test]
    fn python_assigned_lambda_resolves_lhs() {
        let source = "handler = lambda req: req\n";
        let tree = parse_py(source);
        let lambda = find_kind(tree.root_node(), "lambda").unwrap();
        let name = bound_name_from_ancestors(
            lambda,
            source.as_bytes(),
            PYTHON_ASSIGNMENT,
            python_simple_assignment_lhs,
        );
        assert_eq!(name.as_deref(), Some("handler"));
    }

    #[test]
    fn python_attribute_assign_skipped() {
        let source = "obj.fn = lambda x: x\n";
        let tree = parse_py(source);
        let lambda = find_kind(tree.root_node(), "lambda").unwrap();
        let name = bound_name_from_ancestors(
            lambda,
            source.as_bytes(),
            PYTHON_ASSIGNMENT,
            python_simple_assignment_lhs,
        );
        assert_eq!(name, None);
    }
}
