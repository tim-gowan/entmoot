//! Immutable address interning and original-edge postings shared by query operations.
use crate::TopologyGraph;
use entmoot_contracts::address_identity_key;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub(super) struct RelationIndex {
    pub keys: Vec<String>,
    pub ids: HashMap<String, usize>,
    pub outgoing: Vec<Vec<usize>>,
    pub incoming: Vec<Vec<usize>>,
    pub endpoints: Vec<(usize, usize)>,
}

impl RelationIndex {
    pub fn new(graph: &TopologyGraph, nodes: &HashMap<String, Vec<usize>>) -> Self {
        let mut result = Self::default();
        for edge in &graph.relation_edges {
            let mut intern = |key: String| {
                let next = result.keys.len();
                *result.ids.entry(key.clone()).or_insert_with(|| {
                    result.keys.push(key);
                    result.outgoing.push(Vec::new());
                    result.incoming.push(Vec::new());
                    next
                })
            };
            let from = intern(address_identity_key(&edge.subject));
            let to = intern(address_identity_key(&edge.object));
            let index = result.endpoints.len();
            result.endpoints.push((from, to));
            result.outgoing[from].push(index);
            result.incoming[to].push(index);
        }
        for edges in result.outgoing.iter_mut().chain(&mut result.incoming) {
            edges.sort_by(|&a, &b| {
                graph.relation_edges[a]
                    .id
                    .as_str()
                    .cmp(graph.relation_edges[b].id.as_str())
            });
        }
        let mut isolated: Vec<_> = nodes
            .keys()
            .filter(|key| !result.ids.contains_key(*key))
            .cloned()
            .collect();
        isolated.sort();
        for key in isolated {
            result.ids.insert(key.clone(), result.keys.len());
            result.keys.push(key);
            result.outgoing.push(Vec::new());
            result.incoming.push(Vec::new());
        }
        result
    }

    pub fn postings(&self, key: &str, forward: bool) -> Option<&Vec<usize>> {
        self.ids.get(key).map(|&id| self.edges(id, forward))
    }

    pub fn edges(&self, id: usize, forward: bool) -> &Vec<usize> {
        if forward {
            &self.outgoing[id]
        } else {
            &self.incoming[id]
        }
    }

    pub fn iter(&self, forward: bool) -> impl Iterator<Item = (&String, &Vec<usize>)> {
        self.keys.iter().zip(if forward {
            &self.outgoing
        } else {
            &self.incoming
        })
    }
}

/// Dense union-find for disposable connectivity projections. It carries no claims.
pub(super) struct DisjointSet {
    parents: Vec<usize>,
    ranks: Vec<u8>,
}
impl DisjointSet {
    pub fn new(count: usize) -> Self {
        Self {
            parents: (0..count).collect(),
            ranks: vec![0; count],
        }
    }
    pub fn root(&mut self, mut node: usize) -> usize {
        while self.parents[node] != node {
            self.parents[node] = self.parents[self.parents[node]];
            node = self.parents[node];
        }
        node
    }
    pub fn union(&mut self, a: usize, b: usize) {
        let mut a = self.root(a);
        let mut b = self.root(b);
        if a == b {
            return;
        }
        if self.ranks[a] < self.ranks[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parents[b] = a;
        if self.ranks[a] == self.ranks[b] {
            self.ranks[a] += 1;
        }
    }
    pub fn labels(mut self) -> Vec<usize> {
        for node in 0..self.parents.len() {
            self.parents[node] = self.root(node);
        }
        self.parents
    }
}

/// Address plus composition context; boundary arrival matters for reverse walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) struct ScopedState {
    pub node: usize,
    pub scope: Option<usize>,
    pub boundary: bool,
}

impl super::QueryEngine {
    pub(super) fn transition_scope(
        &self,
        prior: Option<usize>,
        previous: Option<usize>,
        edge: usize,
        forward: bool,
    ) -> Option<usize> {
        let scope = self.indexes.composition_scopes[edge]?;
        if prior.is_some_and(|prior| prior != scope) {
            let bridge = previous.is_some_and(|previous| {
                let a = &self.graph.relation_edges[previous];
                let b = &self.graph.relation_edges[edge];
                if forward {
                    super::observation_scope_bridge(a, b)
                } else {
                    super::observation_scope_bridge(b, a)
                }
            });
            if !bridge {
                return None;
            }
        }
        Some(scope)
    }
}

/// Iterative Kosaraju over either node or original-edge postings. A rejected
/// charge discards the incomplete result; it can never justify reachability.
pub(super) fn strong_components(
    outgoing: &[Vec<usize>],
    incoming: &[Vec<usize>],
    neighbor: impl Fn(usize, bool) -> usize,
    mut charge: impl FnMut() -> bool,
) -> Option<Vec<Vec<usize>>> {
    let mut seen = vec![false; outgoing.len()];
    let mut order = Vec::with_capacity(seen.len());
    for root in 0..seen.len() {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut stack = vec![(root, 0)];
        while let Some((node, child)) = stack.last_mut() {
            if let Some(&next) = outgoing[*node].get(*child) {
                if !charge() {
                    return None;
                }
                let next = neighbor(next, true);
                *child += 1;
                if !seen[next] {
                    seen[next] = true;
                    stack.push((next, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for root in order.into_iter().rev() {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut stack = vec![root];
        let mut members = Vec::new();
        while let Some(node) = stack.pop() {
            members.push(node);
            for &edge in &incoming[node] {
                if !charge() {
                    return None;
                }
                let next = neighbor(edge, false);
                if !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        members.sort_unstable();
        components.push(members);
    }
    Some(components)
}
