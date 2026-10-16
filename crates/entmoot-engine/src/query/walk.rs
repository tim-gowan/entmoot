//! Scope-aware, weighted bidirectional discovery over original edges or chain ranges.
use super::{
    contraction::ShortcutOverlay, walk_cache::WalkDiscovery, BoundedWalkBudget, QueryEngine,
    WalkCounters,
};
use entmoot_contracts::RelationKind;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::Arc;

use super::adjacency::ScopedState as State;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Segment {
    edge: usize,
    range: Option<(usize, usize, usize)>,
}
impl Segment {
    fn edges<'a>(&'a self, overlay: &'a Option<Arc<ShortcutOverlay>>) -> &'a [usize] {
        match (self.range, overlay) {
            (Some((chain, from, to)), Some(overlay)) => &overlay.chains[chain][from..to],
            _ => std::slice::from_ref(&self.edge),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    state: State,
    distance: usize,
    parent: Option<(usize, Segment)>,
    settled: bool,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Side {
    records: Vec<Record>,
    #[serde(with = "super::checkpoint_map")]
    ids: HashMap<State, usize>,
    nodes: HashMap<usize, Vec<usize>>,
    queue: BinaryHeap<Reverse<(usize, usize)>>,
}
impl Side {
    fn roots(roots: &[usize]) -> Self {
        let mut side = Self::default();
        for &node in roots {
            let state = State {
                node,
                scope: None,
                boundary: false,
            };
            if side.ids.contains_key(&state) {
                continue;
            }
            let id = side.records.len();
            side.ids.insert(state, id);
            side.nodes.entry(node).or_default().push(id);
            side.records.push(Record {
                state,
                distance: 0,
                parent: None,
                settled: false,
            });
            side.queue.push(Reverse((0, id)));
        }
        side
    }
    fn next_distance(&mut self) -> Option<usize> {
        while let Some(&Reverse((distance, id))) = self.queue.peek() {
            let record = &self.records[id];
            if record.settled || record.distance != distance {
                self.queue.pop();
            } else {
                return Some(distance);
            }
        }
        None
    }
    fn path(
        &self,
        mut id: usize,
        forward: bool,
        overlay: &Option<Arc<ShortcutOverlay>>,
    ) -> Vec<usize> {
        let mut segments = Vec::new();
        while let Some((parent, segment)) = self.records[id].parent {
            segments.push(segment);
            id = parent;
        }
        if forward {
            segments.reverse();
        }
        segments
            .iter()
            .flat_map(|segment| segment.edges(overlay).iter().copied())
            .collect()
    }
}

/// All mutable discovery state; a slice stops at an exact original adjacency position.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PathSearch {
    left: Side,
    right: Side,
    allowed: HashSet<RelationKind>,
    cuts: HashMap<usize, Vec<usize>>,
    overlay: Option<Arc<ShortcutOverlay>>,
    pending: Option<(bool, usize, usize, usize)>,
    shortest: Option<usize>,
    allow_zero: bool,
    pub done: bool,
    pub counters: WalkCounters,
    incompatible: usize,
}

impl PathSearch {
    pub fn new(
        engine: &QueryEngine,
        start: &[String],
        target: &[String],
        allowed: HashSet<RelationKind>,
        allow_zero: bool,
    ) -> Self {
        let ids = &engine.indexes.relations.ids;
        let start: Vec<_> = start.iter().filter_map(|k| ids.get(k).copied()).collect();
        let target: Vec<_> = target.iter().filter_map(|k| ids.get(k).copied()).collect();
        let index_started = std::time::Instant::now();
        let (overlay, work) = engine.shortcut_overlay(&allowed);
        let index_micros = u64::try_from(index_started.elapsed().as_micros()).unwrap_or(u64::MAX);
        tracing::debug!(
            index_micros,
            index_examined_edges = work,
            shortcuts = overlay.as_ref().map_or(0, |o| o.chains.len()),
            "selected path index"
        );
        let disconnected = overlay.as_ref().is_some_and(|o| {
            let labels: HashSet<_> = target.iter().map(|&b| o.components[b]).collect();
            !start.iter().any(|&a| labels.contains(&o.components[a]))
        });
        let mut cuts: HashMap<usize, Vec<usize>> = HashMap::new();
        if let Some(overlay) = &overlay {
            for &anchor in start.iter().chain(&target) {
                if let Some((chain, offset)) = overlay.interiors[anchor] {
                    cuts.entry(chain).or_default().push(offset);
                }
            }
            for offsets in cuts.values_mut() {
                offsets.sort_unstable();
                offsets.dedup();
            }
        }
        Self {
            left: Side::roots(&start),
            right: Side::roots(&target),
            allowed,
            cuts,
            overlay,
            pending: None,
            shortest: None,
            allow_zero,
            done: disconnected,
            counters: WalkCounters {
                index_examined_edges: work,
                index_micros,
                ..Default::default()
            },
            incompatible: 0,
        }
    }

    fn segment(&self, edge: usize, forward: bool) -> Segment {
        let Some(overlay) = &self.overlay else {
            return Segment { edge, range: None };
        };
        let Some((chain, offset)) = overlay.positions[edge] else {
            return Segment { edge, range: None };
        };
        let edges = &overlay.chains[chain];
        let cuts = self.cuts.get(&chain).map_or(&[][..], Vec::as_slice);
        let position = cuts.partition_point(|&cut| cut <= offset);
        if forward {
            let end = cuts.get(position).copied().unwrap_or(edges.len());
            Segment {
                edge,
                range: Some((chain, offset, end)),
            }
        } else {
            let begin = position.checked_sub(1).map_or(0, |i| cuts[i]);
            Segment {
                edge,
                range: Some((chain, begin, offset + 1)),
            }
        }
    }

    fn joined(&self, engine: &QueryEngine, left: usize, right: usize) -> Option<Vec<usize>> {
        let mut edges = self.left.path(left, true, &self.overlay);
        edges.extend(self.right.path(right, false, &self.overlay));
        if (edges.is_empty() && !self.allow_zero)
            || !super::walk_edges_compatible(&engine.graph, &edges)
        {
            return None;
        }
        Some(edges)
    }

    pub fn retained_bytes(&self) -> usize {
        // Includes spare hash/vector capacity and queue entries; overlays have a
        // separate bounded owner and are shared rather than charged per search.
        1024 + (self.left.records.len() + self.right.records.len()) * 512
            + (self.left.queue.len() + self.right.queue.len()) * 64
            + self
                .cuts
                .values()
                .map(|v| 64 + v.capacity() * 16)
                .sum::<usize>()
    }

    pub fn advance(&mut self, engine: &QueryEngine, budget: BoundedWalkBudget) {
        self.advance_with_limit(engine, budget, usize::MAX);
    }

    pub fn advance_with_limit(
        &mut self,
        engine: &QueryEngine,
        budget: BoundedWalkBudget,
        memory_limit: usize,
    ) -> bool {
        let mut memory_available = true;
        let mut work = WalkCounters::default();
        while !self.done {
            if self.retained_bytes().saturating_add(1024) > memory_limit {
                memory_available = false;
                break;
            }
            if self.pending.is_none() {
                let a = self.left.next_distance();
                let b = self.right.next_distance();
                if a.is_none() && b.is_none() {
                    self.done = true;
                    break;
                }
                if self.shortest.is_some_and(|best| {
                    a.unwrap_or(usize::MAX)
                        .saturating_add(b.unwrap_or(usize::MAX))
                        >= best
                }) {
                    self.done = true;
                    break;
                }
                if work.expanded_nodes >= budget.max_expansions {
                    work.expansion_budget_exhausted = true;
                    break;
                }
                // Each side remains a distance-ordered Dijkstra queue. Choosing
                // the cheaper next expansion avoids scanning a broad source
                // when the target has a narrow incoming frontier.
                let cost = |side: &Side, forward| {
                    side.queue.peek().map_or(usize::MAX, |Reverse((_, id))| {
                        engine
                            .indexes
                            .relations
                            .edges(side.records[*id].state.node, forward)
                            .len()
                    })
                };
                let forward = (cost(&self.left, true), a) <= (cost(&self.right, false), b);
                let side = if forward {
                    &mut self.left
                } else {
                    &mut self.right
                };
                let Some(Reverse((distance, id))) = side.queue.pop() else {
                    continue;
                };
                side.records[id].settled = true;
                work.expanded_nodes += 1;
                let node = side.records[id].state.node;
                let opposite = if forward { &self.right } else { &self.left };
                for &other in opposite.nodes.get(&node).into_iter().flatten() {
                    let (left, right) = if forward { (id, other) } else { (other, id) };
                    if let Some(path) = self.joined(engine, left, right) {
                        if path.len() <= budget.max_hops {
                            self.shortest = Some(
                                self.shortest
                                    .map_or(path.len(), |best| best.min(path.len())),
                            );
                        }
                    } else {
                        self.incompatible += 1;
                    }
                }
                if distance >= budget.max_hops {
                    continue;
                }
                self.pending = Some((forward, id, 0, 0));
            }
            let Some((forward, id, offset, fanout)) = self.pending else {
                continue;
            };
            let side = if forward { &self.left } else { &self.right };
            let record = &side.records[id];
            let state = record.state;
            let distance = record.distance;
            let adjacency = engine.indexes.relations.edges(state.node, forward);
            let Some(&edge_id) = adjacency.get(offset) else {
                self.pending = None;
                continue;
            };
            if work.examined_edges >= budget.max_examined_edges {
                work.edge_budget_exhausted = true;
                break;
            }
            work.examined_edges += 1;
            self.pending = Some((forward, id, offset + 1, fanout));
            let edge = &engine.graph.relation_edges[edge_id];
            if !self.allowed.is_empty() && !self.allowed.contains(&edge.predicate) {
                continue;
            }
            if budget.max_edges_per_node > 0 && fanout >= budget.max_edges_per_node {
                work.fanout_truncated = true;
                self.pending = None;
                continue;
            }
            self.pending = Some((forward, id, offset + 1, fanout + 1));
            let previous = record.parent.and_then(|(_, segment)| {
                let edges = segment.edges(&self.overlay);
                if forward {
                    edges.last().copied()
                } else {
                    edges.first().copied()
                }
            });
            let Some(scope) = engine.transition_scope(state.scope, previous, edge_id, forward)
            else {
                self.incompatible += 1;
                continue;
            };
            let segment = self.segment(edge_id, forward);
            let edges = segment.edges(&self.overlay);
            if segment.range.is_some() {
                work.examined_shortcuts += 1;
            }
            let length = distance.saturating_add(edges.len());
            if length > budget.max_hops {
                continue;
            }
            let last = if forward { edges.last() } else { edges.first() };
            let Some(&last) = last else {
                continue;
            };
            let pair = engine.indexes.relations.endpoints[last];
            let next = State {
                node: if forward { pair.1 } else { pair.0 },
                scope: Some(scope),
                boundary: !forward
                    && super::is_observation_boundary(&engine.graph.relation_edges[last]),
            };
            let side = if forward {
                &mut self.left
            } else {
                &mut self.right
            };
            let replace = side.ids.get(&next).is_none_or(|&old| {
                if length != side.records[old].distance {
                    return length < side.records[old].distance;
                }
                let mut candidate = side.path(id, forward, &self.overlay);
                if forward {
                    candidate.extend_from_slice(edges);
                } else {
                    let mut prefix = edges.to_vec();
                    prefix.extend(candidate);
                    candidate = prefix;
                }
                super::walk_edge_key(&engine.graph, &candidate)
                    < super::walk_edge_key(&engine.graph, &side.path(old, forward, &self.overlay))
            });
            if replace {
                let next_id = if let Some(&old) = side.ids.get(&next) {
                    old
                } else {
                    let next_id = side.records.len();
                    side.ids.insert(next, next_id);
                    side.nodes.entry(next.node).or_default().push(next_id);
                    side.records.push(Record {
                        state: next,
                        distance: length,
                        parent: None,
                        settled: false,
                    });
                    next_id
                };
                side.records[next_id].distance = length;
                side.records[next_id].parent = Some((id, segment));
                side.records[next_id].settled = false;
                side.queue.push(Reverse((length, next_id)));
                let opposite = if forward { &self.right } else { &self.left };
                for &other in opposite.nodes.get(&next.node).into_iter().flatten() {
                    let (left, right) = if forward {
                        (next_id, other)
                    } else {
                        (other, next_id)
                    };
                    if let Some(path) = self.joined(engine, left, right) {
                        if path.len() <= budget.max_hops {
                            self.shortest = Some(
                                self.shortest
                                    .map_or(path.len(), |best| best.min(path.len())),
                            );
                        }
                    }
                }
            }
        }
        self.counters.expanded_nodes += work.expanded_nodes;
        self.counters.examined_edges += work.examined_edges;
        self.counters.examined_shortcuts += work.examined_shortcuts;
        self.counters.expansion_budget_exhausted = work.expansion_budget_exhausted;
        self.counters.edge_budget_exhausted = work.edge_budget_exhausted;
        self.counters.fanout_truncated |= work.fanout_truncated;
        memory_available
    }

    pub fn discovery(&self, engine: &QueryEngine, budget: BoundedWalkBudget) -> WalkDiscovery {
        // Ordinary bounded requests use the same reconstruction semantics.
        self.discovery_with_limit(engine, budget, usize::MAX)
            .unwrap_or_else(|| WalkDiscovery {
                counters: self.counters.clone(),
                candidates: vec![],
                total_paths: 0,
                incompatible_meets: self.incompatible,
            })
    }

    pub fn discovery_with_limit(
        &self,
        engine: &QueryEngine,
        budget: BoundedWalkBudget,
        memory_limit: usize,
    ) -> Option<WalkDiscovery> {
        let mut candidates = Vec::new();
        let mut retained = 1024usize;
        for (left, record) in self.left.records.iter().enumerate() {
            for &right in self
                .right
                .nodes
                .get(&record.state.node)
                .into_iter()
                .flatten()
            {
                if let Some(edges) = self.joined(engine, left, right) {
                    if edges.len() > budget.max_hops {
                        continue;
                    }
                    let meet = edges
                        .get(edges.len().saturating_sub(1) / 2)
                        .map_or(record.state.node, |&edge| {
                            engine.indexes.relations.endpoints[edge].1
                        });
                    retained = retained
                        .saturating_add(128)
                        .saturating_add(engine.indexes.relations.keys[meet].len())
                        .saturating_add(edges.capacity().saturating_mul(8));
                    if retained > memory_limit {
                        return None;
                    }
                    candidates.push((
                        edges.len(),
                        engine.indexes.relations.keys[meet].clone(),
                        edges,
                    ));
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| {
                super::walk_edge_key(&engine.graph, &a.2)
                    .cmp(&super::walk_edge_key(&engine.graph, &b.2))
            })
        });
        candidates.dedup_by(|a, b| a.2 == b.2);
        if let Some(shortest) = candidates.first().map(|c| c.0) {
            candidates.retain(|c| c.0 == shortest);
        }
        let total_paths = candidates.len();
        candidates.truncate(budget.max_results);
        Some(WalkDiscovery {
            counters: self.counters.clone(),
            candidates,
            total_paths,
            incompatible_meets: self.incompatible,
        })
    }
}

impl QueryEngine {
    pub(super) fn discover_walk(
        &self,
        start: &str,
        target: &str,
        start_keys: &[String],
        target_keys: &[String],
        allowed: &HashSet<RelationKind>,
        budget: BoundedWalkBudget,
    ) -> WalkDiscovery {
        let mut search = PathSearch::new(
            self,
            start_keys,
            target_keys,
            allowed.clone(),
            start == target,
        );
        search.advance(self, budget);
        search.discovery(self, budget)
    }
}
