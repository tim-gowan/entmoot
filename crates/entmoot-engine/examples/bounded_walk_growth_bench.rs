//! Adversarial branching benchmark for the bounded-walk search frontier.
//!
//! This intentionally stores only an initialized empty claim database so the
//! measurement isolates graph index construction and traversal. Run with:
//! `cargo run --release -p entmoot-engine --example bounded_walk_growth_bench`

use std::time::{Duration, Instant};

use entmoot_contracts::{relation_edge_with_id, Address, RelationKind, Scope};
use entmoot_engine::{
    BoundedWalkBudget, KnowledgeRepository, KnowledgeStore, NodeKind, QueryEngine,
    SqliteKnowledgeRepository, TopologyGraph, TopologyNode, TraverseQuery,
};

fn address(repository: &str, path: &str) -> Address {
    Address::RepoPath {
        repository_id: Some(repository.into()),
        path: path.into(),
        start_byte: None,
        end_byte: None,
    }
}

fn add_node(graph: &mut TopologyGraph, id: &str, path: &str) {
    let mut node = TopologyNode::shell(id, NodeKind::File, id, None);
    node.path = Some(path.into());
    node.display_ref = path.into();
    graph.nodes.push(node);
}

fn add_edge(graph: &mut TopologyGraph, from: &str, to: &str, scope: &Scope) {
    graph.relation_edges.push(relation_edge_with_id(
        "growth-benchmark",
        address(&graph.snapshot.repository_id, from),
        RelationKind::DependsOn,
        address(&graph.snapshot.repository_id, to),
        scope.clone(),
        None,
    ));
}

fn build_case(
    branching: usize,
    half_depth: usize,
) -> Result<(TopologyGraph, String, String), Box<dyn std::error::Error>> {
    let mut graph = TopologyGraph::new("entmoot:bounded-walk-growth-benchmark");
    let scope = Scope {
        domain: graph.snapshot.repository_id.clone(),
        snapshot_revision: Some(graph.snapshot.revision.clone()),
        snapshot_digest: graph.snapshot.snapshot_digest.clone(),
        occurrence_id: None,
        context_id: None,
    };
    add_node(&mut graph, "start", "tree/root");
    let mut level = vec![("start".to_owned(), "tree/root".to_owned())];
    let mut serial = 0usize;
    for depth in 1..=half_depth {
        let mut next = Vec::with_capacity(level.len() * branching);
        for (_, parent_path) in &level {
            for branch in 0..branching {
                serial += 1;
                let id = format!("tree-{serial}");
                let path = format!("tree/{depth}/{serial}-{branch}");
                add_node(&mut graph, &id, &path);
                add_edge(&mut graph, parent_path, &path, &scope);
                next.push((id, path));
            }
        }
        level = next;
    }
    let (_, mut previous_path) = level
        .pop()
        .ok_or("benchmark branching must be greater than zero")?;
    let mut target_id = String::new();
    for index in 1..=half_depth {
        target_id = format!("tail-{index}");
        let path = format!("tail/{index}");
        add_node(&mut graph, &target_id, &path);
        add_edge(&mut graph, &previous_path, &path, &scope);
        previous_path = path;
    }
    graph.rebuild_indexes();
    Ok((graph, "start".into(), target_id))
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cases = [(2usize, 3usize), (4, 4), (6, 5), (8, 6), (9, 6), (10, 6)];
    println!("branching,half_depth,nodes,edges,index_ms,query_ms,status,expanded,examined");
    let mut first_above_one_second = None;
    let mut first_above_five_seconds = None;
    let mut stopped_by_budget = false;
    for (branching, half_depth) in cases {
        let (graph, start, target) = build_case(branching, half_depth)?;
        let nodes = graph.nodes.len();
        let edges = graph.relation_edges.len();
        let directory = tempfile::tempdir()?;
        let store_path = directory.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&store_path)?;
        repository.replace_all(&KnowledgeStore::default())?;
        drop(repository);

        let indexed_at = Instant::now();
        let service = QueryEngine::new(graph, store_path);
        let index_elapsed = indexed_at.elapsed();
        let mut samples = Vec::new();
        let mut last = serde_json::Value::Null;
        for _ in 0..5 {
            let started = Instant::now();
            last = service
                .traverse(TraverseQuery::Path {
                    resumable: false,
                    continuation: None,
                    start: start.clone(),
                    target: target.clone(),
                    predicates: vec!["depends_on".into()],
                    budget: BoundedWalkBudget {
                        max_hops: half_depth * 2,
                        max_expansions: 100_000,
                        max_edges_per_node: branching,
                        max_examined_edges: 100_000,
                        max_results: 1,
                        max_evidence_per_hop: 20,
                    },
                })?
                .into_value()?;
            samples.push(started.elapsed());
        }
        let query_elapsed = median(samples);
        let case = format!("branching={branching} half_depth={half_depth}");
        if query_elapsed >= Duration::from_secs(1) && first_above_one_second.is_none() {
            first_above_one_second = Some(case.clone());
        }
        if query_elapsed >= Duration::from_secs(5) && first_above_five_seconds.is_none() {
            first_above_five_seconds = Some(case);
        }
        println!(
            "{branching},{half_depth},{nodes},{edges},{:.3},{:.3},{},{},{}",
            index_elapsed.as_secs_f64() * 1_000.0,
            query_elapsed.as_secs_f64() * 1_000.0,
            last["status"].as_str().unwrap_or("error"),
            last["search_receipt"]["expanded_nodes"],
            last["search_receipt"]["examined_edges"]
        );
        if last["status"] == "truncated" {
            stopped_by_budget = true;
        }
        if first_above_five_seconds.is_some() || stopped_by_budget {
            break;
        }
    }
    println!(
        "first_above_1s={} first_above_5s={} stopped_by_budget={stopped_by_budget}",
        first_above_one_second.as_deref().unwrap_or("not reached"),
        first_above_five_seconds.as_deref().unwrap_or("not reached")
    );
    Ok(())
}
