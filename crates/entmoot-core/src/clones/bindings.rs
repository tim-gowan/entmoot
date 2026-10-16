//! Binding lowering for clone summaries; never changes structural identities.
use super::{identifier_is_reference, observes_binding_names, summary};
use entmoot_contracts::{CodeContract, DeclaredContract, ParameterKind};
use std::collections::{BTreeMap, HashMap};
use tree_sitter::Node;

#[derive(Clone, Copy)]
pub(super) struct Occurrence {
    pub key: usize,
    pub capture: summary::Digest,
    pub declaration: bool,
}
impl Occurrence {
    fn new(key: usize, name: &str) -> Self {
        // Captures retain spelling and callable-relative declaration order. This
        // conservatively distinguishes two same-spelled shadowed bindings.
        Self {
            key,
            capture: summary::hash(b"capture-name", &[name.as_bytes(), &(key as u64).to_le_bytes()]),
            declaration: true,
        }
    }
    fn reference(self) -> Self {
        Self {
            declaration: false,
            ..self
        }
    }
}

pub(super) fn resolve(
    root: Node<'_>,
    all: &[Node<'_>],
    source: &[u8],
    contract: &CodeContract,
) -> Option<HashMap<usize, Occurrence>> {
    if contract.language == "rust" {
        return rust(root, all, source, contract);
    }
    let bindings = super::bindings(root, all, source, contract)?;
    Some(
        all.iter()
            .filter_map(|node| {
                if !identifier_is_reference(*node) {
                    return None;
                }
                let text = node.utf8_text(source).ok()?;
                let binding = bindings.get(text)?;
                let key = binding.index?;
                if node.start_byte() != binding.declaration && node.start_byte() < binding.visible_after {
                    return None;
                }
                let occurrence = Occurrence::new(key, text);
                Some((
                    node.id(),
                    if node.start_byte() == binding.declaration {
                        occurrence
                    } else {
                        occurrence.reference()
                    },
                ))
            })
            .collect(),
    )
}

fn rust(
    root: Node<'_>,
    all: &[Node<'_>],
    source: &[u8],
    contract: &CodeContract,
) -> Option<HashMap<usize, Occurrence>> {
    let DeclaredContract::Callable { parameters, .. } = &contract.shape else {
        return None;
    };
    let mut occurrences = HashMap::new();
    let mut active: HashMap<&str, Vec<Occurrence>> = HashMap::new();
    let mut key = 0;
    let identifiers: BTreeMap<_, _> = all
        .iter()
        .filter(|n| n.kind() == "identifier")
        .map(|n| (n.start_byte(), n))
        .collect();
    for parameter in parameters {
        if parameter.kind == ParameterKind::Unsupported {
            return None;
        }
        let name = parameter.name.as_deref()?;
        let matches: Vec<_> = identifiers
            .range(parameter.span.start_byte..parameter.span.end_byte)
            .map(|(_, n)| *n)
            .filter(|n| n.end_byte() <= parameter.span.end_byte && n.utf8_text(source).ok() == Some(name))
            .collect();
        if matches.len() != 1 || active.contains_key(name) {
            return None;
        }
        let occurrence = Occurrence::new(key, name);
        key += 1;
        occurrences.insert(matches[0].id(), occurrence);
        active.insert(name, vec![occurrence]);
    }
    enum Event<'a> {
        Enter(Node<'a>),
        Leave,
        Bind(&'a str, Occurrence),
    }
    let mut scopes: Vec<Vec<&str>> = vec![Vec::new()];
    let mut pending = vec![Event::Enter(root)];
    while let Some(event) = pending.pop() {
        let node = match event {
            Event::Leave => {
                for name in scopes.pop()? {
                    active.get_mut(name)?.pop();
                }
                continue;
            }
            Event::Bind(name, occurrence) => {
                active.entry(name).or_default().push(occurrence);
                scopes.last_mut()?.push(name);
                continue;
            }
            Event::Enter(node) => node,
        };
        if node != root
            && matches!(
                node.kind(),
                "function_item"
                    | "closure_expression"
                    | "macro_invocation"
                    | "for_expression"
                    | "match_expression"
                    | "let_condition"
                    | "let_chain"
                    | "shorthand_field_initializer"
                    | "const_item"
                    | "static_item"
                    | "type_item"
                    | "use_declaration"
                    | "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "impl_item"
                    | "mod_item"
            )
        {
            return None;
        }
        if observes_binding_names(node, source, "rust") {
            return None;
        }
        if node.kind() == "block" {
            scopes.push(Vec::new());
            pending.push(Event::Leave);
        }
        if node.kind() == "let_declaration" {
            let name = node.child_by_field_name("pattern")?;
            if name.kind() != "identifier" {
                return None;
            }
            let text = name.utf8_text(source).ok()?;
            let occurrence = Occurrence::new(key, text);
            key += 1;
            occurrences.insert(name.id(), occurrence);
            // Rust's new binding becomes visible only after its initializer.
            pending.push(Event::Bind(text, occurrence));
        }
        if identifier_is_reference(node) && !occurrences.contains_key(&node.id()) {
            if let Some(binding) = active.get(node.utf8_text(source).ok()?).and_then(|stack| stack.last()) {
                occurrences.insert(node.id(), binding.reference());
            }
        }
        let mut cursor = node.walk();
        if cursor.goto_last_child() {
            loop {
                pending.push(Event::Enter(cursor.node()));
                if !cursor.goto_previous_sibling() {
                    break;
                }
            }
        }
    }
    Some(occurrences)
}
