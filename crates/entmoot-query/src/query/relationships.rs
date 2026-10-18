//! One bounded adjacency owner for assessment components and reverse impact.
use crate::{EdgeKind, TopologyGraph};
use entmoot_contracts::{address_identity_key, RelationKind};
use std::collections::{BTreeSet, HashMap, VecDeque};

pub(super) struct RelationshipIndex {
    pub outgoing: Vec<Vec<usize>>,
    pub incoming: Vec<Vec<usize>>,
    pub external_incoming: BTreeSet<usize>,
    pub examined_edges: usize,
    pub limitations: Vec<String>,
    exhausted: bool,
}

impl RelationshipIndex {
    pub fn new(
        graph: &TopologyGraph,
        max_nodes: usize,
        max_edges: usize,
        calls_only: bool,
    ) -> Self {
        let count = graph.nodes.len().min(max_nodes.clamp(1, 200_000));
        let mut index = Self {
            outgoing: vec![Vec::new(); count],
            incoming: vec![Vec::new(); count],
            external_incoming: BTreeSet::new(),
            examined_edges: 0,
            limitations: vec![],
            exhausted: count < graph.nodes.len(),
        };
        if count < graph.nodes.len() {
            index.limitations.push("node_budget_exceeded".into());
        }
        let mut addresses: HashMap<String, Vec<usize>> = HashMap::new();
        let mut ids = HashMap::new();
        let resources: HashMap<_, _> = graph
            .resources
            .iter()
            .map(|r| (r.occurrence_id.as_str(), &r.logical_address))
            .collect();
        for (i, node) in graph.nodes.iter().take(count).enumerate() {
            ids.insert(node.id.as_str(), i);
            for address in super::node_addresses_with_resource(
                node,
                &graph.snapshot.repository_id,
                resources.get(node.id.as_str()).copied(),
            ) {
                addresses
                    .entry(address_identity_key(&address))
                    .or_default()
                    .push(i);
            }
        }
        let mut pairs = Vec::new();
        let mut admit = |from: Option<usize>, to: Option<usize>| {
            if let (Some(from), Some(to)) = (from, to) {
                pairs.push((from, to));
            }
        };
        let budget = max_edges.clamp(1, 1_000_000);
        for edge in &graph.relation_edges {
            if index.examined_edges == budget {
                index.exhausted = true;
                index.limitations.push("edge_budget_exceeded".into());
                break;
            }
            index.examined_edges += 1;
            if calls_only && edge.predicate != RelationKind::Calls {
                continue;
            }
            let unique = |key: String| {
                addresses
                    .get(&key)
                    .filter(|nodes| nodes.len() == 1)
                    .map(|nodes| nodes[0])
            };
            let from = unique(address_identity_key(&edge.subject));
            let to = unique(address_identity_key(&edge.object));
            if from.is_none() || to.is_none() {
                if let Some(targets) = addresses.get(&address_identity_key(&edge.object)) {
                    index.external_incoming.extend(targets);
                }
            }
            if from.is_none() || to.is_none() {
                index
                    .limitations
                    .push("relationship_endpoint_unattached_or_outside_inventory".into());
            }
            admit(from, to);
        }
        for edge in &graph.edges {
            if index.examined_edges == budget {
                index.exhausted = true;
                index.limitations.push("edge_budget_exceeded".into());
                break;
            }
            index.examined_edges += 1;
            if edge.kind != EdgeKind::Calls {
                continue;
            }
            let from = ids.get(edge.from.as_str()).copied();
            let to = ids.get(edge.to.as_str()).copied();
            if from.is_none() {
                index.external_incoming.extend(to);
            }
            admit(from, to);
        }
        pairs.sort_unstable();
        pairs.dedup();
        for (from, to) in pairs {
            index.outgoing[from].push(to);
            index.incoming[to].push(from);
        }
        index.limitations.sort();
        index.limitations.dedup();
        index
    }

    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    /// Iterative Kosaraju; linear work and no recursion on adversarial chains.
    pub fn components(&self) -> Vec<Vec<usize>> {
        super::adjacency::strong_components(&self.outgoing, &self.incoming, |node, _| node, || true)
            .unwrap_or_default()
    }

    /// Each reached node points toward one changed seed, including seeds themselves.
    pub fn reverse_paths(&self, seeds: impl Iterator<Item = usize>) -> HashMap<usize, usize> {
        let mut parents = HashMap::new();
        let mut queue = VecDeque::new();
        for seed in seeds.filter(|&n| n < self.incoming.len()) {
            parents.insert(seed, seed);
            queue.push_back(seed);
        }
        while let Some(node) = queue.pop_front() {
            for &next in &self.incoming[node] {
                if let std::collections::hash_map::Entry::Vacant(entry) = parents.entry(next) {
                    entry.insert(node);
                    queue.push_back(next);
                }
            }
        }
        parents
    }
}
