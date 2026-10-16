//! Bounded syntax-pattern fingerprints, deliberately separate from structural identity.
use crate::sound_cst::SoundStructuralProfile;
use entmoot_contracts::{CloneExtent, CloneFingerprints, CodeContract, DeclaredContract, ParameterKind, SiteSpan};
use std::collections::BTreeMap;

mod bindings;
mod fingerprints;
mod summary;
use tree_sitter::Node;

#[derive(Debug)]
pub struct ClonePattern {
    pub owner_start: usize,
    pub span: SiteSpan,
    pub extent: CloneExtent,
    pub token_count: usize,
    pub fingerprints: CloneFingerprints,
}
#[derive(Debug, Default)]
pub struct CloneAnalysis {
    pub patterns: Vec<ClonePattern>,
    pub alpha_unavailable_callables: usize,
    pub truncated: bool,
}

struct Binding {
    declaration: usize,
    visible_after: usize,
    // None keeps an externally bindable parameter's spelling at every use.
    index: Option<usize>,
}

use crate::traversal::preorder as nodes;

fn observes_binding_names(node: Node<'_>, source: &[u8], language: &str) -> bool {
    // References include aliases of these builtins. Shadowed names may make this
    // conservative; syntax alone cannot qualify a runtime reflection target.
    if node.kind() == "identifier" {
        let name = node.utf8_text(source).unwrap_or("");
        return match language {
            "python" => matches!(name, "eval" | "exec" | "locals" | "vars" | "globals"),
            "javascript" | "typescript" | "tsx" => matches!(name, "eval" | "Function"),
            "csharp" => name == "nameof",
            _ => false,
        };
    }
    match language {
        "python" => node.kind() == "=" && node.parent().is_some_and(|parent| parent.kind() == "interpolation"),
        "csharp" => matches!(node.kind(), "nameof" | "nameof_expression"),
        "c" | "cpp" => node.kind().starts_with("preproc_"),
        _ => false,
    }
}

fn bindings(
    root: Node<'_>,
    all: &[Node<'_>],
    source: &[u8],
    contract: &CodeContract,
) -> Option<BTreeMap<String, Binding>> {
    let DeclaredContract::Callable { parameters, .. } = &contract.shape else {
        return None;
    };
    let mut result = BTreeMap::new();
    let identifiers: BTreeMap<_, _> = all
        .iter()
        .filter(|n| n.kind() == "identifier")
        .map(|n| (n.start_byte(), n))
        .collect();
    for parameter in parameters {
        if parameter.kind == ParameterKind::Unsupported {
            return None;
        }
        let name = parameter.name.as_ref()?;
        let matches: Vec<_> = identifiers
            .range(parameter.span.start_byte..parameter.span.end_byte)
            .map(|(_, n)| *n)
            .filter(|n| n.end_byte() <= parameter.span.end_byte && n.utf8_text(source).ok() == Some(name))
            .collect();
        if matches.len() != 1 || result.contains_key(name) {
            return None;
        }
        result.insert(
            name.clone(),
            Binding {
                declaration: matches[0].start_byte(),
                visible_after: parameter.span.end_byte,
                index: match (contract.language.as_str(), parameter.kind) {
                    ("python", ParameterKind::Ordinary | ParameterKind::KeywordOnly) | ("csharp", _) => None,
                    _ => Some(result.len()),
                },
            },
        );
    }
    for node in all {
        // These declarations need their own scope lowering. Never let a local
        // declarator be mistaken for a use of a same-spelled parameter.
        if (matches!(contract.language.as_str(), "c" | "cpp") && node.kind() == "declaration")
            || (contract.language == "go"
                && matches!(
                    node.kind(),
                    "var_declaration" | "const_declaration" | "type_declaration"
                ))
        {
            return None;
        }
        if observes_binding_names(*node, source, &contract.language) {
            return None;
        }
        if node.id() != root.id()
            && matches!(
                node.kind(),
                "function_item"
                    | "function_definition"
                    | "function_declaration"
                    | "function_expression"
                    | "generator_function"
                    | "generator_function_declaration"
                    | "local_function_statement"
                    | "method_definition"
                    | "method_declaration"
                    | "lambda"
                    | "lambda_expression"
                    | "closure_expression"
                    | "arrow_function"
                    | "anonymous_function"
                    | "macro_invocation"
                    | "for_statement"
                    | "for_expression"
                    | "for_in_statement"
                    | "enhanced_for_statement"
                    | "for_each_statement"
                    | "foreach_statement"
                    | "switch_expression"
                    | "is_pattern_expression"
                    | "with_statement"
                    | "catch_clause"
                    | "match_expression"
                    | "list_comprehension"
                    | "dictionary_comprehension"
                    | "set_comprehension"
                    | "generator_expression"
                    | "shorthand_property_identifier"
                    | "shorthand_field_initializer"
                    | "class_definition"
                    | "class_declaration"
                    | "global_statement"
                    | "nonlocal_statement"
                    | "import_statement"
                    | "import_from_statement"
            )
        {
            return None;
        }
        let name = match node.kind() {
            "let_declaration" => node.child_by_field_name("pattern"),
            "variable_declarator" => node.child_by_field_name("name"),
            "assignment" if contract.language == "python" => node.child_by_field_name("left"),
            "short_var_declaration" => return None,
            _ => continue,
        }?;
        // A flat rename map is only valid for declarations in the callable's own body.
        let mut ancestor = node.parent();
        while let Some(parent) = ancestor {
            if parent.id() == root.id() {
                break;
            }
            if matches!(parent.kind(), "block" | "compound_statement" | "statement_block")
                && parent.parent() != Some(root)
            {
                return None;
            }
            ancestor = parent.parent();
        }
        if name.kind() != "identifier" {
            return None;
        }
        let name_text = name.utf8_text(source).ok()?.to_owned();
        if result.contains_key(&name_text) {
            return None;
        }
        result.insert(
            name_text,
            Binding {
                declaration: name.start_byte(),
                visible_after: node.end_byte(),
                index: Some(result.len()),
            },
        );
    }
    if matches!(
        contract.language.as_str(),
        "python" | "javascript" | "typescript" | "tsx" | "java" | "csharp"
    ) {
        // Function-local Python bindings and hoisted/TDZ ECMAScript bindings
        // and Java/C# initializer self-references are not free lookups. Treating them as
        // free names can collapse a local lookup and an unrelated global lookup.
        // Keep this lane unavailable until those scope rules are fully lowered.
        for node in all {
            if identifier_is_reference(*node) {
                if let Some(binding) = node.utf8_text(source).ok().and_then(|name| result.get(name)) {
                    if binding.index.is_some()
                        && node.start_byte() != binding.declaration
                        && node.start_byte() < binding.visible_after
                    {
                        return None;
                    }
                }
            }
        }
    }
    Some(result)
}

fn identifier_is_reference(node: Node<'_>) -> bool {
    if node.kind() != "identifier" {
        return false;
    }
    if let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "scoped_identifier"
                | "scoped_type_identifier"
                | "qualified_identifier"
                | "qualified_name"
                | "namespace_name"
        ) {
            return false;
        }
        for field in ["attribute", "field", "property", "key"] {
            if parent.child_by_field_name(field) == Some(node) {
                return false;
            }
        }
        if matches!(
            parent.kind(),
            "method_invocation" | "keyword_argument" | "named_argument"
        ) && parent.child_by_field_name("name") == Some(node)
        {
            return false;
        }
    }
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if matches!(
            parent.kind(),
            "type_annotation" | "type_arguments" | "type_parameter" | "generic_type" | "scoped_type_identifier"
        ) {
            return false;
        }
        if parent.child_by_field_name("type") == Some(node) {
            return false;
        }
        ancestor = parent.parent();
    }
    true
}

fn literal(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "integer"
            | "integer_literal"
            | "float"
            | "float_literal"
            | "number"
            | "number_literal"
            | "string"
            | "string_literal"
            | "raw_string_literal"
            | "character_literal"
            | "char_literal"
            | "true"
            | "false"
            | "none"
            | "null_literal"
    ) && !crate::traversal::iter(node).any(|n| {
        matches!(
            n.kind(),
            "interpolation" | "interpolation_expression" | "string_interpolation" | "template_substitution"
        )
    })
}

pub(crate) fn analyze(
    source: &[u8],
    index: &crate::sound_cst::CallableNodes<'_>,
    language: &str,
    contracts: &[CodeContract],
    min_tokens: usize,
    statements: bool,
    max_patterns: usize,
) -> CloneAnalysis {
    let mut output = CloneAnalysis::default();
    let profile = SoundStructuralProfile::for_language(language);
    let mut contracts: Vec<_> = contracts
        .iter()
        .filter(|c| matches!(c.shape, DeclaredContract::Callable { .. }))
        .collect();
    contracts.sort_by_key(|c| c.site.span.start_byte);
    let callable_spans: Vec<_> = contracts.iter().map(|c| c.site.span).collect();
    let mut examined = 0usize;
    for contract in contracts {
        if examined >= max_patterns {
            output.truncated = true;
            break;
        }
        let Some(callable) = index.get(contract.site.span.start_byte, contract.site.span.end_byte) else {
            continue;
        };
        // Bound work before walking the callable; large functions remain explicit omissions.
        if callable.end_byte() - callable.start_byte() > 100_000 {
            output.truncated = true;
            continue;
        }
        let all = nodes(callable);
        let bindings = bindings::resolve(callable, &all, source, contract);
        if bindings.is_none() {
            output.alpha_unavailable_callables += 1;
        }
        let name = callable.child_by_field_name("name").or_else(|| {
            callable
                .child_by_field_name("declarator")
                .and_then(crate::languages::c_family::declarator_name)
        });
        let mut extents = vec![(vec![callable], CloneExtent::Callable)];
        if statements {
            'blocks: for block in all.iter().copied().filter(|n| {
                matches!(n.kind(), "block" | "compound_statement" | "statement_block")
                    || (language == "go" && n.kind() == "statement_list")
            }) {
                let nested = callable_spans.partition_point(|span| span.start_byte <= callable.start_byte());
                if callable_spans[nested..]
                    .iter()
                    .take_while(|span| span.start_byte <= block.start_byte())
                    .any(|span| {
                        span.end_byte <= callable.end_byte()
                            && block.start_byte() >= span.start_byte
                            && block.end_byte() <= span.end_byte
                    })
                {
                    continue;
                }
                let mut cursor = block.walk();
                let children: Vec<_> = block
                    .named_children(&mut cursor)
                    .filter(|n| crate::sound_cst::retained(source, *n, &profile))
                    .collect();
                // Fixed-size windows provide bounded repeated-block discovery.
                for length in 3..=6.min(children.len()) {
                    for window in children.windows(length) {
                        if extents.len() >= max_patterns.saturating_sub(examined).max(1) {
                            output.truncated = true;
                            break 'blocks;
                        }
                        extents.push((window.to_vec(), CloneExtent::Statements));
                    }
                }
            }
        }
        let fingerprints = fingerprints::Fingerprints::build(
            source,
            &all,
            name,
            &profile,
            bindings.as_ref(),
            extents
                .iter()
                .flat_map(|(roots, _)| roots.iter().map(|n| n.id()))
                .collect(),
        );
        for (roots, extent) in extents {
            if examined >= max_patterns {
                output.truncated = true;
                return output;
            }
            examined += 1;
            let span = SiteSpan {
                start_byte: roots[0].start_byte(),
                end_byte: roots[roots.len() - 1].end_byte(),
            };
            let (syntax, alpha, parameterized, token_count) = fingerprints.get(&roots);
            if token_count < min_tokens {
                continue;
            }
            output.patterns.push(ClonePattern {
                owner_start: contract.site.span.start_byte,
                span,
                extent,
                token_count,
                fingerprints: CloneFingerprints {
                    exact: crate::location_verify::exact_body_id_of_bytes(&source[span.start_byte..span.end_byte])
                        .into_inner(),
                    syntax,
                    alpha,
                    parameterized,
                },
            });
        }
    }
    output
}
