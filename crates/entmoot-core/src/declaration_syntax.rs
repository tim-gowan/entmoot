//! Bounded CST observations, separate from callable hashing and native binding.
//! Compound binding patterns, implicit values, macro expansions and assignment
//! targets such as `obj.field` are intentionally not interpreted.

use entmoot_contracts::{DeclarationSyntax, InitializerSyntax, InitializerValue, SiteSpan};
use tree_sitter::Node;

fn span(node: Node<'_>) -> SiteSpan {
    SiteSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

fn simple_name(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier" | "property_identifier" | "private_property_identifier"
    )
}

fn only_expression(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "expression_list" {
        if node.named_child_count() == 1 {
            node.named_child(0)
        } else {
            None
        }
    } else {
        Some(node)
    }
}

fn parts<'a>(node: Node<'a>, language: &str) -> Option<(Node<'a>, Option<Node<'a>>)> {
    let (name, value) = match (language, node.kind()) {
        ("rust", "const_item" | "static_item") => {
            (node.child_by_field_name("name")?, node.child_by_field_name("value"))
        }
        ("rust", "let_declaration") => (node.child_by_field_name("pattern")?, node.child_by_field_name("value")),
        ("python", "assignment") => (node.child_by_field_name("left")?, node.child_by_field_name("right")),
        ("javascript" | "typescript" | "tsx" | "java", "variable_declarator") => {
            (node.child_by_field_name("name")?, node.child_by_field_name("value"))
        }
        ("javascript", "field_definition") => {
            (node.child_by_field_name("property")?, node.child_by_field_name("value"))
        }
        ("typescript" | "tsx", "public_field_definition") => {
            (node.child_by_field_name("name")?, node.child_by_field_name("value"))
        }
        ("go", "const_spec" | "var_spec") => {
            let mut cursor = node.walk();
            let names: Vec<_> = node
                .children_by_field_name("name", &mut cursor)
                .filter(|n| n.is_named())
                .collect();
            if names.len() != 1 {
                return None;
            }
            let value = node.child_by_field_name("value");
            if value.is_some_and(|v| only_expression(v).is_none()) {
                return None;
            }
            (names[0], value.and_then(only_expression))
        }
        ("go", "short_var_declaration") => {
            let name = only_expression(node.child_by_field_name("left")?)?;
            let value = only_expression(node.child_by_field_name("right")?)?;
            (name, Some(value))
        }
        ("csharp", "variable_declarator") => {
            let name = node.child_by_field_name("name")?;
            let mut cursor = node.walk();
            let value = node
                .children(&mut cursor)
                .skip_while(|n| n.kind() != "=")
                .skip(1)
                .find(|n| n.is_named() && !n.is_extra());
            (name, value)
        }
        ("c" | "cpp", "identifier") => {
            let parent = node.parent()?;
            if parent.kind() != "declaration" {
                return None;
            }
            let mut cursor = parent.walk();
            if !parent
                .children_by_field_name("declarator", &mut cursor)
                .any(|n| n.id() == node.id())
            {
                return None;
            }
            (node, None)
        }
        ("c" | "cpp", "init_declarator") => (
            node.child_by_field_name("declarator")?,
            node.child_by_field_name("value"),
        ),
        _ => return None,
    };
    simple_name(name).then_some((name, value))
}

fn literal_integer(node: Node<'_>, source: &[u8]) -> Option<String> {
    // Qualify syntax before interpreting the small decimal spelling. A string,
    // macro, cast, arithmetic expression or call cannot become a value fact.
    let literal = matches!(
        node.kind(),
        "integer_literal" | "decimal_integer_literal" | "int_literal" | "integer" | "number" | "number_literal"
    );
    let unary = matches!(
        node.kind(),
        "unary_expression" | "unary_operator" | "prefix_unary_expression"
    ) && node.named_child_count() == 1
        && node.named_child(0).is_some_and(|n| {
            matches!(
                n.kind(),
                "integer_literal" | "decimal_integer_literal" | "int_literal" | "integer" | "number" | "number_literal"
            )
        });
    if !literal && !unary {
        return None;
    }
    let text = node.utf8_text(source).ok()?;
    entmoot_contracts::decimal_integer(text).map(|_| text.to_owned())
}

pub(crate) fn collect(
    root: Node<'_>,
    language: &str,
    source: &[u8],
    markers: &mut crate::analysis::context::UnsealedMarkers,
) {
    // JS can pass the hasher's separate typed-syntax validator while its original
    // CST is recovered. No declaration/absence observation is qualified there.
    if root.has_error() {
        return;
    }
    let callable_owners: std::collections::HashMap<_, _> = markers
        .markers
        .definitions
        .iter()
        .map(|definition| ((definition.start_byte, definition.end_byte), definition))
        .collect();
    // Iterative walk: deeply nested initializer syntax does not consume the Rust stack.
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if !node.has_error() && !node.is_missing() {
            if let Some((name, value)) = parts(node, language) {
                let name_text = name.utf8_text(source).unwrap_or("");
                // Existing assigned-callable declarations own those binding sites.
                let assigned_callable = value.is_some_and(|v| {
                    matches!(
                        v.kind(),
                        "lambda" | "arrow_function" | "function_expression" | "function"
                    )
                });
                if !name_text.is_empty() && name_text.len() <= 1024 && !assigned_callable {
                    let initializer = value.map(|value| {
                        let literal = literal_integer(value, source);
                        InitializerSyntax {
                            span: span(value),
                            content_digest: entmoot_contracts::ContentDigest::from_trusted(format!(
                                "blake3:{}",
                                blake3::hash(&source[value.byte_range()]).to_hex()
                            )),
                            value: literal.map_or(InitializerValue::Unevaluated {}, |text| {
                                InitializerValue::DecimalInteger { text }
                            }),
                        }
                    });
                    let mut scope = Vec::new();
                    let mut ancestor = node.parent();
                    while let Some(parent) = ancestor {
                        if let Some(owner) = callable_owners.get(&(parent.start_byte(), parent.end_byte())) {
                            scope.push(owner.name.clone());
                            scope.extend(owner.scope.iter().rev().cloned());
                            break;
                        }
                        if matches!(
                            parent.kind(),
                            "class_declaration"
                                | "class_definition"
                                | "class"
                                | "struct_item"
                                | "struct_declaration"
                                | "trait_item"
                                | "impl_item"
                                | "mod_item"
                                | "namespace_declaration"
                                | "namespace_definition"
                                | "function_item"
                                | "function_definition"
                                | "function_declaration"
                                | "method_declaration"
                                | "constructor_declaration"
                                | "local_function_statement"
                        ) {
                            if let Some(owner) = parent.child_by_field_name("name").or_else(|| {
                                (parent.kind() == "impl_item")
                                    .then(|| parent.child_by_field_name("type"))
                                    .flatten()
                            }) {
                                if let Ok(text) = owner.utf8_text(source) {
                                    scope.push(text.to_owned());
                                }
                            }
                        }
                        ancestor = parent.parent();
                    }
                    scope.reverse();
                    markers.declarations.push(crate::analysis::context::PendingDeclaration {
                        name: name_text.to_owned(),
                        kind: "binding".into(),
                        scope,
                        span: span(node),
                        callable_start_byte: None,
                        syntax: Some(DeclarationSyntax {
                            method: DeclarationSyntax::METHOD.into(),
                            name_span: span(name),
                            initializer,
                        }),
                    });
                }
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}
