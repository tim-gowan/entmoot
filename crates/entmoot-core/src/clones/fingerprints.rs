use super::{bindings::Occurrence, summary::Summary};
use crate::sound_cst::SoundStructuralProfile;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

#[derive(Clone)]
struct Lanes {
    syntax: Summary,
    alpha: Option<Summary>,
    parameterized: Option<Summary>,
    tokens: usize,
}
impl Lanes {
    fn node(kind: &str, children: &[Self], available: bool) -> Self {
        let syntax = Summary::node(kind, children.iter().map(|c| &c.syntax));
        let alpha = available.then(|| {
            if children
                .iter()
                .all(|c| c.alpha.as_ref().is_some_and(|a| a.same(&c.syntax)))
            {
                syntax.clone()
            } else {
                Summary::node(kind, children.iter().filter_map(|c| c.alpha.as_ref()))
            }
        });
        let parameterized = if available
            && children.iter().all(|c| {
                c.parameterized
                    .as_ref()
                    .zip(c.alpha.as_ref())
                    .is_some_and(|(p, a)| p.same(a))
            }) {
            alpha.clone()
        } else {
            available.then(|| Summary::node(kind, children.iter().filter_map(|c| c.parameterized.as_ref())))
        };
        Self {
            syntax,
            alpha,
            parameterized,
            tokens: children.iter().map(|c| c.tokens).sum(),
        }
    }
    fn append(&mut self, child: &Self) {
        let alpha_is_syntax = self
            .alpha
            .as_ref()
            .zip(child.alpha.as_ref())
            .is_some_and(|(a, b)| a.same(&self.syntax) && b.same(&child.syntax));
        let parameter_is_alpha = self
            .parameterized
            .as_ref()
            .zip(child.parameterized.as_ref())
            .zip(self.alpha.as_ref().zip(child.alpha.as_ref()))
            .is_some_and(|((a, b), (c, d))| a.same(c) && b.same(d));
        self.syntax = self.syntax.combine(&child.syntax);
        self.alpha = if alpha_is_syntax {
            Some(self.syntax.clone())
        } else {
            self.alpha.as_ref().zip(child.alpha.as_ref()).map(|(a, b)| a.combine(b))
        };
        self.parameterized = if parameter_is_alpha {
            self.alpha.clone()
        } else {
            self.parameterized
                .as_ref()
                .zip(child.parameterized.as_ref())
                .map(|(a, b)| a.combine(b))
        };
        self.tokens += child.tokens;
    }
}

pub(super) struct Fingerprints {
    saved: HashMap<usize, Lanes>,
    available: bool,
}
impl Fingerprints {
    pub(super) fn build(
        source: &[u8],
        all: &[Node<'_>],
        omitted: Option<Node<'_>>,
        profile: &SoundStructuralProfile,
        bindings: Option<&HashMap<usize, Occurrence>>,
        selected: HashSet<usize>,
    ) -> Self {
        let mut pending = HashMap::<usize, Lanes>::new();
        let mut saved = HashMap::new();
        let mut literals = HashMap::new();
        let mut binding_summaries = HashMap::new();
        // Literal keys occupy a disjoint namespace from binding IDs.
        let literal_base = all.len();
        for node in all.iter().rev().copied() {
            let retained = Some(node) != omitted && crate::sound_cst::retained(source, node, profile);
            let text = node.utf8_text(source).unwrap_or("");
            let mut lanes = if node.child_count() == 0 {
                let syntax = Summary::atom(node.kind(), text.as_bytes());
                Lanes {
                    alpha: bindings.map(|_| syntax.clone()),
                    parameterized: bindings.map(|_| syntax.clone()),
                    syntax,
                    tokens: 1,
                }
            } else {
                let mut cursor = node.walk();
                let children: Vec<_> = node
                    .children(&mut cursor)
                    .filter_map(|child| pending.remove(&child.id()))
                    .collect();
                Lanes::node(node.kind(), &children, bindings.is_some())
            };
            if !retained {
                continue;
            }
            if let Some(binding) = bindings.and_then(|b| b.get(&node.id())) {
                lanes.alpha = Some(
                    binding_summaries
                        .entry((binding.key, binding.declaration, node.kind()))
                        .or_insert_with(|| {
                            Summary::binding(node.kind(), binding.key, binding.capture, binding.declaration)
                        })
                        .clone(),
                );
                lanes.parameterized = lanes.alpha.clone();
            }
            if bindings.is_some() && super::literal(node) {
                let next = literal_base + literals.len();
                let key = *literals.entry((node.kind(), text)).or_insert(next);
                lanes.parameterized = Some(Summary::literal(node.kind(), key));
            }
            if selected.contains(&node.id()) {
                saved.insert(node.id(), lanes.clone());
            }
            pending.insert(node.id(), lanes);
        }
        Self {
            saved,
            available: bindings.is_some(),
        }
    }
    pub(super) fn get(&self, roots: &[Node<'_>]) -> (String, Option<String>, Option<String>, usize) {
        let empty = Summary::atom("forest", &[]);
        let mut result = Lanes {
            syntax: empty.clone(),
            alpha: self.available.then(|| empty.clone()),
            parameterized: self.available.then_some(empty),
            tokens: 0,
        };
        for root in roots {
            if let Some(lanes) = self.saved.get(&root.id()) {
                result.append(lanes);
            }
        }
        (
            result.syntax.digest(),
            result.alpha.map(|s| s.digest()),
            result.parameterized.map(|s| s.digest()),
            result.tokens,
        )
    }
}
