//! Real repository bounded-walk and connectivity benchmark.
//!
//! Usage:
//! `cargo run --release -p entmoot-engine --example repository_walk_bench -- graph.json store.sqlite3`

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use entmoot_contracts::{address_identity_key, topology_node_address, Address};
use entmoot_engine::{
    load_graph_json, BoundedWalkBudget, QueryEngine, TopologyGraph, TopologyNode, TraverseQuery,
};

fn node_addresses(graph: &TopologyGraph, node: &TopologyNode) -> Vec<Address> {
    let mut addresses = vec![topology_node_address(&node.id)];
    if let Some(address_id) = &node.address_id {
        addresses.push(Address::AddressId {
            address_id: address_id.clone(),
        });
    }
    if let Some(logic_address) = &node.logic_address {
        addresses.push(Address::LogicAddress {
            logic_address: logic_address.clone(),
        });
    }
    if let Some(path) = &node.path {
        addresses.push(Address::RepoPath {
            repository_id: Some(graph.snapshot.repository_id.clone()),
            path: path.clone(),
            start_byte: node.start_byte.and_then(|value| u32::try_from(value).ok()),
            end_byte: node.end_byte.and_then(|value| u32::try_from(value).ok()),
        });
    }
    if let Some(resource) = graph
        .resources
        .iter()
        .find(|resource| resource.occurrence_id == node.id)
    {
        addresses.push(resource.logical_address.clone());
    }
    addresses
}

fn percentile(sorted: &[Duration], numerator: usize, denominator: usize) -> Duration {
    sorted[(sorted.len().saturating_sub(1) * numerator) / denominator]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let graph_path = args
        .get(1)
        .map(PathBuf::from)
        .ok_or("graph path is required")?;
    let store_path = args
        .get(2)
        .map(PathBuf::from)
        .ok_or("knowledge store path is required")?;

    let load_started = Instant::now();
    let graph = load_graph_json(&graph_path)?;
    let load_elapsed = load_started.elapsed();

    let analysis_started = Instant::now();
    let mut directed: HashMap<String, Vec<String>> = HashMap::new();
    let mut undirected: HashMap<String, Vec<String>> = HashMap::new();
    let mut predicate_counts = BTreeMap::new();
    let mut schema_domain_counts = BTreeMap::new();
    for edge in &graph.relation_edges {
        *predicate_counts
            .entry(format!("{:?}", edge.predicate))
            .or_insert(0usize) += 1;
        *schema_domain_counts
            .entry(edge.schema_domain.clone())
            .or_insert(0usize) += 1;
        let from = address_identity_key(&edge.subject);
        let to = address_identity_key(&edge.object);
        directed.entry(from.clone()).or_default().push(to.clone());
        directed.entry(to.clone()).or_default();
        undirected.entry(from.clone()).or_default().push(to.clone());
        undirected.entry(to).or_default().push(from);
    }
    for neighbors in directed.values_mut() {
        neighbors.sort();
        neighbors.dedup();
    }
    for neighbors in undirected.values_mut() {
        neighbors.sort();
        neighbors.dedup();
    }

    let mut subject_by_address = HashMap::new();
    for node in &graph.nodes {
        for address in node_addresses(&graph, node) {
            subject_by_address
                .entry(address_identity_key(&address))
                .or_insert_with(|| node.id.clone());
        }
    }

    let mut remaining: HashSet<String> = undirected.keys().cloned().collect();
    let mut component_sizes = Vec::new();
    while let Some(root) = remaining.iter().next().cloned() {
        remaining.remove(&root);
        let mut size = 0usize;
        let mut queue = VecDeque::from([root]);
        while let Some(current) = queue.pop_front() {
            size += 1;
            if let Some(neighbors) = undirected.get(&current) {
                for neighbor in neighbors {
                    if remaining.remove(neighbor) {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }
        component_sizes.push(size);
    }
    component_sizes.sort_unstable_by(|left, right| right.cmp(left));
    let component_buckets = [2usize, 5, 10, 32, 100, 1_000]
        .into_iter()
        .map(|ceiling| {
            (
                ceiling,
                component_sizes
                    .iter()
                    .filter(|size| **size <= ceiling)
                    .count(),
            )
        })
        .collect::<Vec<_>>();
    let mut unresolved_reason_counts = BTreeMap::new();
    let mut unresolved_predicate_counts = BTreeMap::new();
    let mut unresolved_by_reason = BTreeMap::new();
    for relation in &graph.unresolved_relations {
        let reason = format!("{:?}", relation.reason);
        *unresolved_reason_counts
            .entry(reason.clone())
            .or_insert(0usize) += 1;
        *unresolved_predicate_counts
            .entry(format!("{:?}", relation.predicate))
            .or_insert(0usize) += 1;
        unresolved_by_reason
            .entry(reason)
            .or_insert_with(Vec::new)
            .push(relation);
    }
    let mut label_counts = HashMap::new();
    for node in &graph.nodes {
        *label_counts.entry(node.label.as_str()).or_insert(0usize) += 1;
    }
    let unique_label_subject: HashMap<_, _> = graph
        .nodes
        .iter()
        .filter(|node| label_counts.get(node.label.as_str()).copied() == Some(1))
        .map(|node| (node.label.as_str(), node.display_ref.as_str()))
        .collect();
    // Spellings from Go's predeclared universe. These are candidates only:
    // package declarations may shadow them, which requires binding evidence.
    let go_predeclared_spellings: HashSet<_> = [
        "append",
        "bool",
        "byte",
        "cap",
        "clear",
        "close",
        "complex",
        "complex64",
        "complex128",
        "copy",
        "delete",
        "error",
        "float32",
        "float64",
        "imag",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "len",
        "make",
        "max",
        "min",
        "new",
        "panic",
        "print",
        "println",
        "real",
        "recover",
        "rune",
        "string",
        "uint",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uintptr",
    ]
    .into_iter()
    .collect();
    let sample_sizes = [
        ("UnresolvedBinding", 384usize),
        ("TargetNotInInventory", 383usize),
        ("AmbiguousTarget", 214usize),
    ];
    let mut audit_lines = Vec::new();
    for (reason, requested) in sample_sizes {
        let Some(relations) = unresolved_by_reason.get_mut(reason) else {
            continue;
        };
        relations.sort_by_key(|relation| relation.id.as_str());
        let sample = relations
            .iter()
            .take(requested)
            .copied()
            .collect::<Vec<_>>();
        let source_materialized = sample
            .iter()
            .filter(|relation| {
                subject_by_address.contains_key(&address_identity_key(&relation.source))
            })
            .count();
        let occurrence_materialized = sample
            .iter()
            .filter(|relation| {
                relation
                    .occurrence_id
                    .as_deref()
                    .is_some_and(|id| graph.node(id).is_some())
            })
            .count();
        if source_materialized != sample.len() || occurrence_materialized != sample.len() {
            return Err(format!(
                "unresolved-relation audit failed for {reason}: sample={} source_materialized={source_materialized} occurrence_materialized={occurrence_materialized}",
                sample.len()
            )
            .into());
        }
        let syntactic_targets = sample
            .iter()
            .filter_map(|relation| match &relation.target_constraint {
                entmoot_contracts::TargetConstraint::Syntactic { callee } => Some(callee.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let qualified_targets = syntactic_targets
            .iter()
            .filter(|callee| callee.contains('.') || callee.contains('/'))
            .count();
        let unique_leaf_label = syntactic_targets
            .iter()
            .filter(|callee| {
                let leaf = callee
                    .rsplit(['.', '/'])
                    .next()
                    .unwrap_or(callee)
                    .trim_start_matches('*');
                label_counts.get(leaf).copied() == Some(1)
            })
            .count();
        let predeclared_spelling_targets = syntactic_targets
            .iter()
            .filter(|callee| go_predeclared_spellings.contains(**callee))
            .count();
        audit_lines.push(format!(
            "audit reason={reason} population={} sample={} source_materialized={} occurrence_materialized={} syntactic_targets={} qualified_targets={} unique_leaf_label={} go_predeclared_spelling_targets={}",
            relations.len(),
            sample.len(),
            source_materialized,
            occurrence_materialized,
            syntactic_targets.len(),
            qualified_targets,
            unique_leaf_label,
            predeclared_spelling_targets
        ));
        for relation in sample.iter().take(8) {
            let source_key = address_identity_key(&relation.source);
            let source_display = subject_by_address
                .get(&source_key)
                .and_then(|id| graph.node(id))
                .map(|node| node.display_ref.as_str())
                .unwrap_or("<unmaterialized>");
            audit_lines.push(format!(
                "audit_sample reason={reason} source={source_display:?} occurrence={:?} target={:?} candidates={:?}",
                relation
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| graph.node(id))
                    .map(|node| node.display_ref.as_str()),
                relation.target_constraint,
                match &relation.target_constraint {
                    entmoot_contracts::TargetConstraint::Candidates { addresses } => addresses
                        .iter()
                        .filter_map(|address| {
                            subject_by_address
                                .get(&address_identity_key(address))
                                .and_then(|id| graph.node(id))
                                .map(|node| node.display_ref.as_str())
                        })
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                }
            ));
        }
        if reason == "AmbiguousTarget" {
            for relation in sample.iter().filter(|relation| {
                matches!(
                    relation.target_constraint,
                    entmoot_contracts::TargetConstraint::Syntactic { .. }
                )
            }) {
                let source = subject_by_address
                    .get(&address_identity_key(&relation.source))
                    .and_then(|id| graph.node(id))
                    .map(|node| node.display_ref.as_str())
                    .unwrap_or("<unmaterialized>");
                audit_lines.push(format!(
                    "audit_ambiguous_without_candidates source={source:?} occurrence={:?} target={:?}",
                    relation
                        .occurrence_id
                        .as_deref()
                        .and_then(|id| graph.node(id))
                        .map(|node| node.display_ref.as_str()),
                    relation.target_constraint,
                ));
            }
        }
        if reason == "UnresolvedBinding" {
            for (relation, callee, target) in sample
                .iter()
                .filter_map(|relation| match &relation.target_constraint {
                    entmoot_contracts::TargetConstraint::Syntactic { callee } => {
                        let leaf = callee
                            .rsplit(['.', '/'])
                            .next()
                            .unwrap_or(callee)
                            .trim_start_matches('*');
                        unique_label_subject
                            .get(leaf)
                            .map(|target| (*relation, callee.as_str(), *target))
                    }
                    _ => None,
                })
                .take(12)
            {
                let source = subject_by_address
                    .get(&address_identity_key(&relation.source))
                    .and_then(|id| graph.node(id))
                    .map(|node| node.display_ref.as_str())
                    .unwrap_or("<unmaterialized>");
                audit_lines.push(format!(
                    "audit_unique_leaf source={source:?} callee={callee:?} unique_inventory_target={target:?}"
                ));
            }
        }
    }

    let mut seeds: Vec<_> = directed
        .keys()
        .filter(|key| subject_by_address.contains_key(*key))
        .cloned()
        .collect();
    seeds.sort();
    let stride = (seeds.len() / 512).max(1);
    let sampled: Vec<_> = seeds.into_iter().step_by(stride).take(512).collect();
    let mut selected: Option<(String, String, usize)> = None;
    for start in sampled {
        let mut distance = HashMap::from([(start.clone(), 0usize)]);
        let mut queue = VecDeque::from([start.clone()]);
        while let Some(current) = queue.pop_front() {
            let depth = distance[&current];
            if depth == 12 {
                continue;
            }
            for next in directed.get(&current).into_iter().flatten() {
                if distance.contains_key(next) {
                    continue;
                }
                let next_depth = depth + 1;
                distance.insert(next.clone(), next_depth);
                queue.push_back(next.clone());
                if subject_by_address.contains_key(next)
                    && selected
                        .as_ref()
                        .is_none_or(|(_, _, best_depth)| next_depth > *best_depth)
                {
                    selected = Some((start.clone(), next.clone(), next_depth));
                }
            }
        }
    }
    let (start_key, target_key, distance) = selected.ok_or("no materialized reachable pair")?;
    let start_subject = subject_by_address
        .get(&start_key)
        .ok_or("selected start is not materialized")?
        .clone();
    let target_subject = subject_by_address
        .get(&target_key)
        .ok_or("selected target is not materialized")?
        .clone();
    let start_display = graph
        .node(&start_subject)
        .map(|node| node.display_ref.clone())
        .unwrap_or_else(|| start_subject.clone());
    let target_display = graph
        .node(&target_subject)
        .map(|node| node.display_ref.clone())
        .unwrap_or_else(|| target_subject.clone());
    let materialized_relation_subjects: HashSet<_> = undirected
        .keys()
        .filter_map(|key| subject_by_address.get(key))
        .collect();
    let graph_nodes = graph.nodes.len();
    let analysis_elapsed = analysis_started.elapsed();

    let service_started = Instant::now();
    let service = QueryEngine::new(graph, store_path);
    let service_elapsed = service_started.elapsed();

    println!("load_ms={:.3}", load_elapsed.as_secs_f64() * 1_000.0);
    println!(
        "analysis_ms={:.3}",
        analysis_elapsed.as_secs_f64() * 1_000.0
    );
    println!(
        "query_index_ms={:.3}",
        service_elapsed.as_secs_f64() * 1_000.0
    );
    println!(
        "graph_nodes={} materialized_relation_subjects={} relation_vertices={} weak_components={} largest_component={} second_component={}",
        graph_nodes,
        materialized_relation_subjects.len(),
        undirected.len(),
        component_sizes.len(),
        component_sizes.first().copied().unwrap_or(0),
        component_sizes.get(1).copied().unwrap_or(0)
    );
    println!("component_buckets={component_buckets:?}");
    println!("resolved_predicates={predicate_counts:?}");
    println!("relation_schema_domains={schema_domain_counts:?}");
    println!("unresolved_reasons={unresolved_reason_counts:?}");
    println!("unresolved_predicates={unresolved_predicate_counts:?}");
    for line in audit_lines {
        println!("{line}");
    }
    println!(
        "start_subject={start_subject} start_display={start_display:?} target_subject={target_subject} target_display={target_display:?} selected_distance={distance}"
    );
    println!("hop_budget,status,median_us,p95_us,expanded_nodes,examined_edges");
    for max_hops in 1..=distance {
        let mut samples = Vec::with_capacity(7);
        let mut last = serde_json::Value::Null;
        for _ in 0..7 {
            let started = Instant::now();
            last = service
                .traverse(TraverseQuery::Path {
                    resumable: false,
                    continuation: None,
                    start: start_subject.clone(),
                    target: target_subject.clone(),
                    predicates: vec![],
                    budget: BoundedWalkBudget {
                        max_hops,
                        max_expansions: 100_000,
                        max_edges_per_node: 1_000,
                        max_examined_edges: 100_000,
                        max_results: 10,
                        max_evidence_per_hop: 20,
                    },
                })?
                .into_value()?;
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        println!(
            "{max_hops},{},{},{},{},{}",
            last["status"].as_str().unwrap_or("error"),
            percentile(&samples, 1, 2).as_micros(),
            percentile(&samples, 95, 100).as_micros(),
            last["search_receipt"]["expanded_nodes"],
            last["search_receipt"]["examined_edges"]
        );
    }
    Ok(())
}
