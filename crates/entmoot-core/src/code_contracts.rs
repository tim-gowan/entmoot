//! Bounded declarations over the already-parsed tree. Never resolves types.
use std::collections::BTreeMap;

use entmoot_contracts::{
    CodeContract, DeclaredContract, DeclaredExpression, DeclaredMember, DeclaredParameter, DeclaredType, FileMarkers,
    MemberKind, OccurrenceRef, ParameterKind, SiteSpan, SourceOccurrenceRef,
};
use tree_sitter::Node;

use crate::granular::CallableHash;

fn span(node: Node<'_>) -> SiteSpan {
    SiteSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}
fn text(node: Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or_default().to_owned()
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    node.named_children(&mut node.walk())
        .filter(|n| !n.is_extra())
        .collect()
}
fn named(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier"
            | "field_identifier"
            | "property_identifier"
            | "private_property_identifier"
            | "type_identifier"
            | "self"
    )
}
fn type_ref(node: Option<Node<'_>>, source: &[u8]) -> DeclaredType {
    let Some(mut node) = node else {
        return DeclaredType::Unspecified;
    };
    if node.kind() == "type_annotation" {
        if let Some(inner) = node.named_child(0) {
            node = inner;
        }
    }
    DeclaredType::Explicit {
        syntax: text(node, source),
        span: span(node),
    }
}
fn unsupported_type(node: Node<'_>, source: &[u8]) -> DeclaredType {
    DeclaredType::Unsupported {
        syntax: text(node, source),
        span: span(node),
    }
}

fn modifiers(node: Node<'_>, source: &[u8]) -> Vec<String> {
    node.children(&mut node.walk())
        .filter(|n| {
            matches!(
                n.kind(),
                "modifier"
                    | "modifiers"
                    | "visibility_modifier"
                    | "accessibility_modifier"
                    | "?"
                    | "readonly"
                    | "static"
                    | "abstract"
                    | "override"
                    | "ref"
                    | "out"
                    | "in"
                    | "params"
                    | "this"
            )
        })
        .map(|n| text(n, source))
        .collect()
}

fn trailing_dimensions(node: Node<'_>) -> Option<Node<'_>> {
    children(node).into_iter().find(|n| n.kind() == "dimensions")
}

fn parameters(node: Node<'_>, language: &str, source: &[u8], gaps: &mut Vec<String>) -> Vec<DeclaredParameter> {
    let list = if matches!(language, "c" | "cpp") {
        let mut declarator = node.child_by_field_name("declarator");
        let mut found = None;
        while let Some(d) = declarator {
            if d.kind() == "function_declarator" {
                found = d.child_by_field_name("parameters");
                break;
            }
            declarator = d.child_by_field_name("declarator");
        }
        found
    } else {
        node.child_by_field_name("parameters")
            .or_else(|| node.child_by_field_name("parameter"))
    };
    let Some(list) = list else {
        gaps.push("parameter_list_not_inventoried".into());
        return vec![];
    };
    let items = if named(list) { vec![list] } else { children(list) };
    let mut output: Vec<DeclaredParameter> = vec![];
    let mut keyword_only = false;
    for p in items {
        if p.kind() == "positional_separator" {
            for p in &mut output {
                p.kind = ParameterKind::PositionalOnly;
            }
            continue;
        }
        if p.kind() == "keyword_separator" {
            keyword_only = true;
            continue;
        }
        let mut names: Vec<Node<'_>> = p.children_by_field_name("name", &mut p.walk()).collect();
        if names.is_empty() {
            if let Some(n) = p
                .child_by_field_name("pattern")
                .or_else(|| p.child_by_field_name("declarator"))
                .or_else(|| p.child_by_field_name("left"))
            {
                names.push(n);
            }
        }
        if names.is_empty() && named(p) {
            names.push(p);
        }
        if names.is_empty() && language == "python" {
            if let Some(n) = children(p)
                .into_iter()
                .find(|n| named(*n) || matches!(n.kind(), "list_splat_pattern" | "dictionary_splat_pattern"))
            {
                names.push(n);
            }
        }
        if names.is_empty() && matches!(p.kind(), "rest_pattern" | "self_parameter") {
            if let Some(n) = children(p).into_iter().find(|n| named(*n)) {
                names.push(n);
            }
        }
        let splat = names.first().copied().filter(|n| {
            matches!(
                n.kind(),
                "list_splat_pattern" | "dictionary_splat_pattern" | "rest_pattern"
            )
        });
        if let Some(n) = splat {
            names = children(n).into_iter().filter(|n| named(*n)).collect();
        }
        let mut kind = if keyword_only {
            ParameterKind::KeywordOnly
        } else {
            ParameterKind::Ordinary
        };
        if p.kind() == "self_parameter" {
            kind = ParameterKind::Receiver;
        }
        let splat_kind = splat.map(|n| n.kind()).unwrap_or(p.kind());
        if matches!(
            splat_kind,
            "list_splat_pattern"
                | "rest_pattern"
                | "variadic_parameter_declaration"
                | "variadic_parameter"
                | "spread_parameter"
        ) {
            kind = ParameterKind::Variadic;
            if language == "python" {
                keyword_only = true;
            }
        } else if splat_kind == "dictionary_splat_pattern" {
            kind = ParameterKind::KeywordVariadic;
        }
        let mut ty = type_ref(p.child_by_field_name("type"), source);
        if language == "java" && trailing_dimensions(p).is_some() {
            ty = unsupported_type(p, source);
            gaps.push("trailing_array_type_not_reconstructed".into());
        }
        if matches!(language, "c" | "cpp") {
            if text(p, source) == "void" && names.is_empty() {
                continue;
            }
            if let Some(d) = names.first().copied().filter(|n| !named(*n)) {
                ty = unsupported_type(p, source);
                names = crate::languages::c_family::declarator_name(d).into_iter().collect();
                gaps.push("compound_c_declarator_type_not_resolved".into());
            }
        }
        if names.is_empty() {
            names.push(p);
        }
        for n in names {
            let name = named(n).then(|| text(n, source));
            if name.is_none() && kind != ParameterKind::Variadic {
                kind = ParameterKind::Unsupported;
                gaps.push("parameter_pattern_not_inventoried".into());
            }
            output.push(DeclaredParameter {
                modifiers: modifiers(p, source),
                name,
                kind,
                span: span(p),
                declared_type: ty.clone(),
                default: p
                    .child_by_field_name("value")
                    .or_else(|| p.child_by_field_name("right"))
                    .map(|n| DeclaredExpression {
                        span: span(n),
                        syntax: text(n, source),
                    }),
            });
        }
    }
    if language == "go" {
        if let Some(receiver) = node.child_by_field_name("receiver") {
            for p in children(receiver) {
                output.insert(
                    0,
                    DeclaredParameter {
                        modifiers: modifiers(p, source),
                        name: p.child_by_field_name("name").map(|n| text(n, source)),
                        kind: ParameterKind::Receiver,
                        span: span(p),
                        declared_type: type_ref(p.child_by_field_name("type"), source),
                        default: None,
                    },
                );
            }
        }
    }
    output
}

fn callable(node: Node<'_>, language: &str, source: &[u8], gaps: &mut Vec<String>) -> DeclaredContract {
    let parameters = parameters(node, language, source, gaps);
    let result = node
        .child_by_field_name("return_type")
        .or_else(|| node.child_by_field_name("result"))
        .or_else(|| node.child_by_field_name("returns"))
        .or_else(|| {
            matches!(language, "java" | "csharp" | "c" | "cpp")
                .then(|| node.child_by_field_name("type"))
                .flatten()
        });
    let result = if let Some(dimensions) = (language == "java").then(|| trailing_dimensions(node)).flatten() {
        gaps.push("trailing_array_result_type_not_reconstructed".into());
        unsupported_type(dimensions, source)
    } else if matches!(language, "c" | "cpp")
        && node
            .child_by_field_name("declarator")
            .is_some_and(|n| n.kind() != "function_declarator")
    {
        gaps.push("compound_c_result_type_not_resolved".into());
        unsupported_type(node.child_by_field_name("declarator").unwrap_or(node), source)
    } else {
        type_ref(result, source)
    };
    DeclaredContract::Callable { parameters, result }
}

fn object_body(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "class"
        | "class_definition"
        | "class_declaration"
        | "class_specifier"
        | "struct_item"
        | "union_item"
        | "struct_declaration"
        | "struct_specifier"
        | "union_specifier"
        | "interface_declaration"
        | "trait_item"
        | "record_declaration" => node.child_by_field_name("body"),
        "type_spec" => node
            .child_by_field_name("type")
            .filter(|n| matches!(n.kind(), "struct_type" | "interface_type")),
        _ => None,
    }
}

fn members(node: Node<'_>, language: &str, source: &[u8], gaps: &mut Vec<String>) -> Vec<DeclaredMember> {
    let mut body = object_body(node).unwrap_or(node);
    if let Some(list) = children(body)
        .into_iter()
        .find(|n| n.kind() == "field_declaration_list")
    {
        body = list;
    }
    let mut result = vec![];
    for mut member in children(body) {
        if member.kind() == "expression_statement" {
            if let Some(child) = member.named_child(0) {
                member = child;
            }
        }
        if member.kind() == "decorated_definition" {
            if let Some(child) = member.child_by_field_name("definition") {
                member = child;
            }
        }
        if matches!(
            member.kind(),
            "function_definition"
                | "function_item"
                | "function_signature_item"
                | "method_declaration"
                | "method_definition"
                | "method_signature"
                | "constructor_declaration"
        ) {
            result.push(DeclaredMember {
                modifiers: modifiers(member, source),
                name: member.child_by_field_name("name").map(|n| text(n, source)),
                kind: MemberKind::Method,
                span: span(member),
                declared_type: DeclaredType::Unspecified,
            });
            continue;
        }
        let field = matches!(
            member.kind(),
            "field_declaration"
                | "field_definition"
                | "public_field_definition"
                | "property_signature"
                | "property_declaration"
                | "assignment"
        );
        if !field {
            if !matches!(member.kind(), "pass_statement" | "empty_statement" | "access_specifier") {
                gaps.push(format!("member_form_not_inventoried:{}", member.kind()));
            }
            continue;
        }
        let owner = children(member)
            .into_iter()
            .find(|n| n.kind() == "variable_declaration")
            .unwrap_or(member);
        let ty = type_ref(owner.child_by_field_name("type"), source);
        let mut names: Vec<_> = owner.children_by_field_name("name", &mut owner.walk()).collect();
        names.extend(owner.children_by_field_name("declarator", &mut owner.walk()));
        if names.is_empty() {
            names.extend(
                owner
                    .child_by_field_name("left")
                    .or_else(|| owner.child_by_field_name("property")),
            );
        }
        if names.is_empty() {
            names.extend(
                children(owner)
                    .into_iter()
                    .filter(|n| n.kind() == "variable_declarator"),
            );
        }
        if names.is_empty() {
            gaps.push("member_name_not_inventoried".into());
        }
        for mut name in names {
            let mut member_ty = ty.clone();
            if language == "java" && trailing_dimensions(name).is_some() {
                member_ty = unsupported_type(member, source);
                gaps.push("trailing_array_member_type_not_reconstructed".into());
            }
            if name.kind() == "variable_declarator" {
                name = name.child_by_field_name("name").unwrap_or(name);
            }
            if matches!(language, "c" | "cpp") && !named(name) {
                member_ty = unsupported_type(owner, source);
                name = crate::languages::c_family::declarator_name(name).unwrap_or(name);
                gaps.push("compound_c_member_type_not_resolved".into());
            }
            if !named(name) {
                gaps.push("member_pattern_not_inventoried".into());
            }
            result.push(DeclaredMember {
                modifiers: modifiers(member, source),
                name: named(name).then(|| text(name, source)),
                kind: if member.kind() == "property_declaration" {
                    MemberKind::Property
                } else {
                    MemberKind::Field
                },
                span: span(member),
                declared_type: member_ty,
            });
        }
    }
    gaps.push("direct_members_only; inheritance_and_runtime_shape_not_inventoried".into());
    result
}

/// One tree traversal indexes requested declaration spans; no parse or root search
/// per callable. Existing visitor identities are the only attachment targets.
pub(crate) fn collect(
    nodes: &crate::sound_cst::CallableNodes<'_>,
    language: &str,
    source: &[u8],
    callables: &[CallableHash],
    markers: &mut FileMarkers,
) -> Result<(), String> {
    let mut targets = BTreeMap::new();
    for callable in callables {
        let site = SourceOccurrenceRef {
            occurrence: OccurrenceRef::new(callable.logic_address.clone())?,
            span: SiteSpan {
                start_byte: callable.start_byte,
                end_byte: callable.end_byte,
            },
        };
        targets.insert((callable.start_byte, callable.end_byte), (callable.name.clone(), site));
    }
    for declaration in &markers.declarations {
        if matches!(
            declaration.kind.as_str(),
            "class" | "struct" | "interface" | "trait" | "enum" | "type" | "type_alias" | "method"
        ) {
            targets.insert(
                (declaration.site.span.start_byte, declaration.site.span.end_byte),
                (declaration.name.clone(), declaration.site.clone()),
            );
        }
    }
    for ((start, end), (name, site)) in targets {
        let node = nodes
            .get(start, end)
            .ok_or("contract occurrence has no exact CST node")?;
        let mut limitations = vec![];
        let shape = if object_body(node).is_some() {
            DeclaredContract::Object {
                members: members(node, language, source, &mut limitations),
            }
        } else if matches!(
            node.kind(),
            "type_alias"
                | "type_alias_declaration"
                | "type_item"
                | "type_spec"
                | "enum_item"
                | "enum_declaration"
                | "enum_specifier"
        ) {
            limitations.push(format!("declaration_form_not_inventoried:{}", node.kind()));
            DeclaredContract::Unsupported
        } else {
            callable(node, language, source, &mut limitations)
        };
        limitations.sort();
        limitations.dedup();
        let record = CodeContract {
            method: CodeContract::METHOD.into(),
            language: language.into(),
            name,
            site,
            shape,
            limitations,
        };
        record.validate()?;
        markers.code_contracts.push(record);
    }
    markers
        .code_contracts
        .sort_by(|a, b| a.site.occurrence.address_id.cmp(&b.site.occurrence.address_id));
    Ok(())
}
