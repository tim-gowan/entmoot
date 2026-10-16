//! Compiler-free lowering over portable, source-verified semantic evidence.
//! Language families select syntax; term storage and binding state are shared.
use entmoot_contracts::*;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::Node;

pub struct CanonAnalysis {
    pub clones: crate::clones::CloneAnalysis,
    pub forms: Vec<CanonForm>,
}

#[derive(Clone, PartialEq, Eq)]
struct Value {
    digest: ContentDigest,
    sort: CanonSort,
}

struct Fold<'a> {
    source: &'a [u8],
    language: &'a str,
    facts: &'a BTreeMap<(usize, usize), &'a CanonExpressionFact>,
    terms: Vec<CanonTerm>,
    interned: BTreeSet<String>,
    bindings: BTreeMap<String, Value>,
    conditions: Vec<ContentDigest>,
    visited: usize,
}

impl Fold<'_> {
    fn emit(&mut self, op: CanonOp, sort: CanonSort, children: Vec<ContentDigest>) -> Result<Value, CanonLimit> {
        let term = CanonTerm::new(op, sort.clone(), children);
        if self.interned.insert(term.digest.to_string()) {
            if self.terms.len() >= 4095 {
                return Err(CanonLimit::BudgetExceeded);
            }
            self.terms.push(term.clone());
        }
        Ok(Value {
            digest: term.digest,
            sort,
        })
    }
    fn text<'a>(&self, node: Node<'a>) -> Result<&str, CanonLimit> {
        node.utf8_text(self.source)
            .map_err(|_| CanonLimit::UnsupportedConstruct)
    }
    fn fact(&self, node: Node<'_>) -> Result<&CanonExpressionFact, CanonLimit> {
        self.facts
            .get(&(node.start_byte(), node.end_byte()))
            .copied()
            .filter(|f| f.limitations.is_empty() && f.sort != CanonSort::Unknown)
            .ok_or(CanonLimit::UnresolvedBinding)
    }
    fn expression(&mut self, node: Node<'_>, depth: usize) -> Result<Value, CanonLimit> {
        self.visited += 1;
        if depth > 128 || self.visited > 16384 {
            return Err(CanonLimit::BudgetExceeded);
        }
        let text = node
            .utf8_text(self.source)
            .map_err(|_| CanonLimit::UnsupportedConstruct)?;
        match node.kind() {
            "true" | "false" | "boolean_literal" if matches!(text, "true" | "false" | "True" | "False") => {
                // Go's predeclared constants can be shadowed; a type alone cannot
                // qualify their value. A local parameter binding is unambiguous.
                if self.language == "go" {
                    return self.bindings.get(text).cloned().ok_or(CanonLimit::UnresolvedBinding);
                }
                self.emit(
                    CanonOp::Bool {
                        value: matches!(text, "true" | "True"),
                    },
                    CanonSort::Bool,
                    vec![],
                )
            }
            "identifier" => self.bindings.get(text).cloned().ok_or(CanonLimit::UnresolvedBinding),
            "integer" | "integer_literal" | "number" | "number_literal" | "decimal_integer_literal" => {
                let sort = self.fact(node)?.sort.clone();
                if !matches!(sort, CanonSort::Integer { .. }) || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                if text.len() > 1 && text.starts_with('0') {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                let normalized = text.trim_start_matches('0');
                let value = if normalized.is_empty() { "0" } else { normalized };
                self.emit(CanonOp::Integer { value: value.into() }, sort, vec![])
            }
            "parenthesized_expression" | "parenthesized_expression_list" | "condition_clause" | "expression_list" => {
                if node.named_child_count() != 1 {
                    return Err(CanonLimit::UnsupportedConstruct);
                }
                let child = node.named_child(0).ok_or(CanonLimit::UnsupportedConstruct)?;
                self.expression(child, depth + 1)
            }
            "binary_expression" | "binary_operator" | "comparison_operator" | "boolean_operator" => {
                let left = node
                    .child_by_field_name("left")
                    .or_else(|| node.named_child(0))
                    .ok_or(CanonLimit::UnsupportedConstruct)?;
                let right = node
                    .child_by_field_name("right")
                    .or_else(|| node.named_child(1))
                    .ok_or(CanonLimit::UnsupportedConstruct)?;
                let operator = self
                    .source
                    .get(left.end_byte()..right.start_byte())
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let a = self.expression(left, depth + 1)?;
                let b = self.expression(right, depth + 1)?;
                if a.sort != b.sort {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                let (op, sort) = match operator.as_str() {
                    "==" | "===" if matches!(a.sort, CanonSort::Bool | CanonSort::Integer { .. }) => {
                        (CanonOp::Equal, CanonSort::Bool)
                    }
                    "<" if matches!(a.sort, CanonSort::Integer { .. }) => (CanonOp::Less, CanonSort::Bool),
                    "&&" | "and" if a.sort == CanonSort::Bool => (CanonOp::And, CanonSort::Bool),
                    "||" | "or" if a.sort == CanonSort::Bool => (CanonOp::Or, CanonSort::Bool),
                    "+" | "-" | "*" if matches!(a.sort, CanonSort::Integer { .. }) => {
                        let fact = self.fact(node)?;
                        if fact.sort != a.sort {
                            return Err(CanonLimit::UnsupportedSemantics);
                        }
                        let overflow = fact.overflow.ok_or(CanonLimit::UnsupportedSemantics)?;
                        // Wrapping/unbounded arithmetic is total. Checked/undefined arithmetic
                        // needs explicit overflow obligations before it can be qualified.
                        if matches!(overflow, CanonOverflow::Checked | CanonOverflow::Undefined) {
                            return Err(CanonLimit::UnsupportedSemantics);
                        }
                        let op = match operator.as_str() {
                            "+" => CanonOp::Add { overflow },
                            "-" => CanonOp::Subtract { overflow },
                            _ => CanonOp::Multiply { overflow },
                        };
                        (op, a.sort)
                    }
                    _ => return Err(CanonLimit::UnsupportedSemantics),
                };
                self.emit(op, sort, vec![a.digest, b.digest])
            }
            "unary_expression" | "prefix_unary_expression" | "not_operator" => {
                let child = node
                    .child_by_field_name("argument")
                    .or_else(|| node.named_child(0))
                    .ok_or(CanonLimit::UnsupportedConstruct)?;
                let value = self.expression(child, depth + 1)?;
                let operator = self
                    .source
                    .get(node.start_byte()..child.start_byte())
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .unwrap_or("")
                    .trim();
                if !matches!(operator, "!" | "not") || value.sort != CanonSort::Bool {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                self.emit(CanonOp::Not, CanonSort::Bool, vec![value.digest])
            }
            "if_expression" | "conditional_expression" | "ternary_expression" => {
                let condition = node
                    .child_by_field_name("condition")
                    .ok_or(CanonLimit::UnsupportedConstruct)?;
                let yes = node
                    .child_by_field_name("consequence")
                    .ok_or(CanonLimit::UnsupportedConstruct)?;
                let no = node
                    .child_by_field_name("alternative")
                    .ok_or(CanonLimit::UnsupportedConstruct)?;
                let c = self.expression(condition, depth + 1)?;
                let before = self.bindings.clone();
                let a = self.value(yes, depth + 1)?;
                let after_yes = self.bindings.clone();
                self.bindings = before.clone();
                let b = self.value(no, depth + 1)?;
                // Branch-local mutations must not escape without explicit SSA merging.
                if after_yes != before || self.bindings != before {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                if c.sort != CanonSort::Bool || a.sort != b.sort {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                self.emit(CanonOp::Ite, a.sort, vec![c.digest, a.digest, b.digest])
            }
            "call" | "call_expression" | "method_invocation" | "invocation_expression" => {
                // Call lowering is unsupported. Syntax and callee spellings
                // cannot establish a native dispatch mechanism.
                Err(CanonLimit::UnsupportedSemantics)
            }
            _ => Err(CanonLimit::UnsupportedConstruct),
        }
    }
    fn value(&mut self, node: Node<'_>, depth: usize) -> Result<Value, CanonLimit> {
        if matches!(
            node.kind(),
            "block" | "statement_block" | "compound_statement" | "else_clause"
        ) {
            self.block(node, depth + 1)
        } else {
            self.expression(node, depth + 1)
        }
    }
    fn assignment(&mut self, node: Node<'_>, depth: usize) -> Result<(), CanonLimit> {
        let name = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("pattern"))
            .or_else(|| node.child_by_field_name("left"))
            .ok_or(CanonLimit::UnsupportedConstruct)?;
        let value = node
            .child_by_field_name("value")
            .or_else(|| node.child_by_field_name("right"))
            .ok_or(CanonLimit::UnsupportedConstruct)?;
        if name.kind() != "identifier" {
            return Err(CanonLimit::UnsupportedConstruct);
        }
        let spelling = self.text(name)?.to_owned();
        if matches!(node.kind(), "assignment" | "assignment_expression") {
            if self.language != "python" && !self.bindings.contains_key(&spelling) {
                return Err(CanonLimit::UnresolvedBinding);
            }
            let operator = self
                .source
                .get(name.end_byte()..value.start_byte())
                .and_then(|v| std::str::from_utf8(v).ok())
                .unwrap_or("")
                .trim();
            if operator != "=" {
                return Err(CanonLimit::UnsupportedConstruct);
            }
        }
        let result = self.expression(value, depth + 1)?;
        if node.child_by_field_name("type").is_some() {
            // A written annotation cannot qualify an implicit conversion.
            if !self.fact(name).is_ok_and(|fact| fact.sort == result.sort) {
                return Err(CanonLimit::UnsupportedSemantics);
            }
        }
        if let Some(previous) = self.bindings.get(&spelling) {
            if previous.sort != result.sort {
                return Err(CanonLimit::UnsupportedSemantics);
            }
        }
        self.bindings.insert(spelling, result);
        Ok(())
    }
    fn block(&mut self, node: Node<'_>, depth: usize) -> Result<Value, CanonLimit> {
        self.statements(node, depth)?.ok_or(CanonLimit::UnsupportedConstruct)
    }
    fn statements(&mut self, node: Node<'_>, depth: usize) -> Result<Option<Value>, CanonLimit> {
        if depth > 128 {
            return Err(CanonLimit::BudgetExceeded);
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.kind().contains("comment"))
            .collect();
        for (i, child) in children.iter().copied().enumerate() {
            self.visited += 1;
            if self.visited > 16384 {
                return Err(CanonLimit::BudgetExceeded);
            }
            let last = i + 1 == children.len();
            match child.kind() {
                "statement_list" if last => return self.statements(child, depth + 1),
                "return_statement" if last => {
                    return self
                        .expression(child.named_child(0).ok_or(CanonLimit::UnsupportedConstruct)?, depth + 1)
                        .map(Some)
                }
                "let_declaration" | "variable_declarator" | "assignment" => self.assignment(child, depth + 1)?,
                "lexical_declaration" | "variable_declaration" | "local_variable_declaration" => {
                    let mut walk = child.walk();
                    for declaration in child.named_children(&mut walk) {
                        self.assignment(declaration, depth + 1)?;
                    }
                }
                "expression_statement" => {
                    let expression = child.named_child(0).ok_or(CanonLimit::UnsupportedConstruct)?;
                    if matches!(expression.kind(), "assignment" | "assignment_expression") {
                        self.assignment(expression, depth + 1)?;
                    } else if last && self.language == "rust" {
                        return self.expression(expression, depth + 1).map(Some);
                    } else {
                        return Err(CanonLimit::UnsupportedSemantics);
                    }
                }
                "if_statement" => {
                    let condition = child
                        .child_by_field_name("condition")
                        .ok_or(CanonLimit::UnsupportedConstruct)?;
                    let consequence = child
                        .child_by_field_name("consequence")
                        .or_else(|| child.child_by_field_name("body"))
                        .ok_or(CanonLimit::UnsupportedConstruct)?;
                    let c = self.expression(condition, depth + 1)?;
                    if c.sort != CanonSort::Bool {
                        return Err(CanonLimit::UnsupportedSemantics);
                    }
                    let before = self.bindings.clone();
                    let yes = self.statements(consequence, depth + 1)?;
                    let after_yes = self.bindings.clone();
                    self.bindings = before.clone();
                    let no = match child.child_by_field_name("alternative") {
                        Some(alt) if alt.kind() == "else_clause" => self.statements(
                            alt.child_by_field_name("body")
                                .or_else(|| alt.named_child(0))
                                .ok_or(CanonLimit::UnsupportedConstruct)?,
                            depth + 1,
                        )?,
                        Some(alt) => self.statements(alt, depth + 1)?,
                        None => None,
                    };
                    match (yes, no) {
                        (Some(a), Some(b)) if last && a.sort == b.sort => {
                            return self
                                .emit(CanonOp::Ite, a.sort, vec![c.digest, a.digest, b.digest])
                                .map(Some)
                        }
                        (None, None) => {
                            if before.keys().ne(after_yes.keys()) || before.keys().ne(self.bindings.keys()) {
                                return Err(CanonLimit::UnsupportedSemantics);
                            }
                            let after_no = self.bindings.clone();
                            for (name, a) in after_yes {
                                let b = after_no.get(&name).ok_or(CanonLimit::UnsupportedSemantics)?;
                                if a.sort != b.sort {
                                    return Err(CanonLimit::UnsupportedSemantics);
                                }
                                let merged = if &a == b {
                                    a
                                } else {
                                    self.emit(
                                        CanonOp::Ite,
                                        a.sort.clone(),
                                        vec![c.digest.clone(), a.digest, b.digest.clone()],
                                    )?
                                };
                                self.bindings.insert(name, merged);
                            }
                        }
                        _ => return Err(CanonLimit::UnsupportedSemantics),
                    }
                }
                _ if last && self.language == "rust" => return self.value(child, depth + 1).map(Some),
                _ => return Err(CanonLimit::UnsupportedConstruct),
            }
        }
        Ok(None)
    }
}

pub(crate) fn lower(
    source: &[u8],
    language: &str,
    index: &crate::sound_cst::CallableNodes<'_>,
    contracts: &[CodeContract],
    facts: &[CanonExpressionFact],
    max_forms: usize,
) -> Vec<CanonForm> {
    let source_digest = format!("blake3:{}", blake3::hash(source).to_hex());
    let mut facts_by_span = BTreeMap::new();
    let mut conflicts = false;
    for fact in facts {
        if fact.source_digest.as_str() != source_digest
            || fact.site.span.start_byte >= fact.site.span.end_byte
            || fact.site.span.end_byte > source.len()
            || fact.sort.validate().is_err()
        {
            conflicts = true;
            continue;
        }
        if let Some(previous) = facts_by_span.insert((fact.site.span.start_byte, fact.site.span.end_byte), fact) {
            if previous != fact {
                conflicts = true;
            }
        }
    }
    let mut forms = Vec::new();
    let mut emitted_terms = 0;
    for contract in contracts.iter().take(max_forms) {
        if emitted_terms >= 16_384 {
            break;
        }
        let DeclaredContract::Callable {
            parameters,
            result: declared_result,
        } = &contract.shape
        else {
            continue;
        };
        let Some(node) = index.get(contract.site.span.start_byte, contract.site.span.end_byte) else {
            continue;
        };
        let mut fold = Fold {
            source,
            language,
            facts: &facts_by_span,
            terms: vec![],
            interned: BTreeSet::new(),
            bindings: BTreeMap::new(),
            conditions: vec![],
            visited: 0,
        };
        let result = (|| {
            if conflicts {
                return Err(CanonLimit::ConflictingEvidence);
            }
            if node.end_byte() - node.start_byte() > 100_000 {
                return Err(CanonLimit::BudgetExceeded);
            }
            if !contract.limitations.is_empty() {
                return Err(CanonLimit::UnsupportedConstruct);
            }
            // No lowering for nominal result conversions, boxing, or ABI wrappers.
            if language == "cpp" || matches!(declared_result, DeclaredType::Unsupported { .. }) {
                return Err(CanonLimit::UnsupportedSemantics);
            }
            if !matches!(language, "python" | "javascript" | "typescript" | "tsx") {
                if let DeclaredType::Explicit { syntax, .. } = declared_result {
                    if !matches!(
                        syntax.trim(),
                        "bool"
                            | "boolean"
                            | "_Bool"
                            | "int"
                            | "int8"
                            | "int16"
                            | "int32"
                            | "int64"
                            | "uint"
                            | "uint8"
                            | "uint16"
                            | "uint32"
                            | "uint64"
                            | "byte"
                            | "sbyte"
                            | "short"
                            | "ushort"
                            | "long"
                            | "ulong"
                            | "i8"
                            | "i16"
                            | "i32"
                            | "i64"
                            | "i128"
                            | "u8"
                            | "u16"
                            | "u32"
                            | "u64"
                            | "u128"
                    ) {
                        return Err(CanonLimit::UnsupportedSemantics);
                    }
                }
            }
            let body = node
                .child_by_field_name("body")
                .ok_or(CanonLimit::UnsupportedConstruct)?;
            if node.parent().is_some_and(|p| p.kind() == "decorated_definition")
                || crate::traversal::iter(node)
                    .take_while(|n| n.start_byte() < body.start_byte())
                    .any(|n| {
                        n.end_byte() <= body.start_byte()
                            && matches!(
                                n.kind(),
                                "async" | "decorator" | "attribute_item" | "attribute_list" | "*" | "ref"
                            )
                    })
            {
                return Err(CanonLimit::UnsupportedSemantics);
            }
            for (slot, parameter) in parameters.iter().enumerate() {
                let name = parameter.name.as_ref().ok_or(CanonLimit::UnsupportedConstruct)?;
                if parameter.default.is_some()
                    || parameter.kind != ParameterKind::Ordinary
                    || !parameter.modifiers.is_empty()
                {
                    return Err(CanonLimit::UnsupportedSemantics);
                }
                let fact = fold
                    .facts
                    .get(&(parameter.span.start_byte, parameter.span.end_byte))
                    .copied()
                    .or_else(|| {
                        fold.facts
                            .range((parameter.span.start_byte, 0)..=(parameter.span.end_byte, usize::MAX))
                            .find_map(|(&(start, end), fact)| {
                                (end <= parameter.span.end_byte && source.get(start..end) == Some(name.as_bytes()))
                                    .then_some(*fact)
                            })
                    })
                    .ok_or(CanonLimit::UnresolvedBinding)?;
                if !fact.limitations.is_empty() || fact.sort == CanonSort::Unknown {
                    return Err(CanonLimit::UnresolvedBinding);
                }
                let slot = u32::try_from(slot).map_err(|_| CanonLimit::UnresolvedBinding)?;
                let input = fold.emit(CanonOp::Input { slot }, fact.sort.clone(), vec![])?;
                fold.bindings.insert(name.clone(), input);
            }
            fold.value(body, 0)
        })();
        let mut limitations = Vec::new();
        let value = match result {
            Ok(value) => value,
            Err(limit) => {
                limitations.push(limit);
                let bytes = &source[node.start_byte()..node.end_byte()];
                let term = CanonTerm::new(
                    CanonOp::Opaque {
                        source: ContentDigest::from_trusted(format!("blake3:{}", blake3::hash(bytes).to_hex())),
                    },
                    CanonSort::Unknown,
                    vec![],
                );
                let value = Value {
                    digest: term.digest.clone(),
                    sort: CanonSort::Unknown,
                };
                if !fold.terms.iter().any(|t| t.digest == term.digest) {
                    fold.terms.push(term);
                }
                value
            }
        };
        emitted_terms += fold.terms.len();
        forms.push(CanonForm {
            owner: contract.site.occurrence.address_id.to_string(),
            span: contract.site.span,
            language: language.into(),
            result_boundary: match declared_result {
                DeclaredType::Explicit { syntax, .. } => syntax.clone(),
                DeclaredType::Unspecified => "unspecified".into(),
                DeclaredType::Unsupported { syntax, .. } => format!("unsupported:{syntax}"),
            },
            root: value.digest.clone(),
            terms: fold.terms,
            limitations,
            obligation: CanonObligation::new(
                value.digest,
                if facts.is_empty() {
                    vec![CanonAssumption::ScalarValueObservations]
                } else if matches!(language, "python" | "javascript" | "typescript" | "tsx") {
                    vec![
                        CanonAssumption::ScalarValueObservations,
                        CanonAssumption::ValidNativeProgram,
                        CanonAssumption::InputsRespectNativeTypes,
                    ]
                } else {
                    vec![
                        CanonAssumption::ScalarValueObservations,
                        CanonAssumption::ValidNativeProgram,
                    ]
                },
                fold.conditions,
            ),
            inputs: None,
        });
    }
    forms
}
