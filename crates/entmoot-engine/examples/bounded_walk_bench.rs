//! Reproducible meet-in-the-middle bounded-walk microbenchmark.
//!
//! Run with:
//! `cargo run --release -p entmoot-engine --example bounded_walk_bench -- 500 32 8`

use std::time::Instant;

use entmoot_contracts::{
    relation_claim_with_id, relation_edge_with_id, Address, ProducerCapabilityKind, ProducerRef,
    RelationClaimValue, RelationKind, ResolutionState, Scope, SubjectId,
};
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
    graph.upsert_node(node);
}

fn add_edge(graph: &mut TopologyGraph, from: &str, to: &str) {
    let scope = Scope {
        domain: graph.snapshot.repository_id.clone(),
        snapshot_revision: Some(graph.snapshot.revision.clone()),
        snapshot_digest: graph.snapshot.snapshot_digest.clone(),
        occurrence_id: None,
        context_id: None,
    };
    let edge = relation_edge_with_id(
        "benchmark",
        address(&graph.snapshot.repository_id, from),
        RelationKind::DependsOn,
        address(&graph.snapshot.repository_id, to),
        scope.clone(),
        None,
    );
    let claim = relation_claim_with_id(
        SubjectId::RelationEdge {
            edge_id: edge.id.clone(),
        },
        "resolution_state",
        RelationClaimValue::ResolutionState {
            state: ResolutionState::Structural,
        },
        ProducerRef {
            id: "entmoot.bounded-walk-benchmark".into(),
            name: "bounded-walk-benchmark".into(),
            version: Some("1".into()),
            config_fingerprint: None,
            environment_fingerprint: None,
            capabilities: vec![ProducerCapabilityKind::Resolver],
        },
        scope,
        vec![],
        None,
    );
    graph.relation_edges.push(edge);
    graph.relation_claims.push(claim);
}

fn percentile(sorted: &[u128], numerator: usize, denominator: usize) -> u128 {
    sorted[(sorted.len().saturating_sub(1) * numerator) / denominator]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let iterations = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(500);
    let fanout = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(32usize);
    let distance = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(8usize);
    if !(1..=12).contains(&distance) || iterations == 0 || fanout > 999 {
        return Err("iterations must be positive, fanout <= 999, and distance 1..=12".into());
    }

    let mut graph = TopologyGraph::new("entmoot:bounded-walk-benchmark");
    for index in 0..=distance {
        add_node(
            &mut graph,
            &format!("spine-{index}"),
            &format!("spine/{index}"),
        );
    }
    for index in 0..distance {
        add_edge(
            &mut graph,
            &format!("spine/{index}"),
            &format!("spine/{}", index + 1),
        );
        for branch in 0..fanout {
            let id = format!("branch-{index}-{branch}");
            let path = format!("branches/{index}/{branch}");
            add_node(&mut graph, &id, &path);
            add_edge(&mut graph, &format!("spine/{index}"), &path);
        }
    }
    graph.rebuild_indexes();

    let directory = tempfile::tempdir()?;
    let store_path = directory.path().join("knowledge.sqlite3");
    let mut repository = SqliteKnowledgeRepository::open(&store_path)?;
    repository.replace_all(&KnowledgeStore::extract_from_graph(&graph))?;
    drop(repository);
    let service = QueryEngine::new(graph, store_path);

    println!(
        "iterations={iterations} fanout={fanout} distance={distance} relation_edges={}",
        distance * (fanout + 1)
    );
    println!("hop_budget,status,median_us,p95_us,expanded_nodes,examined_edges");
    for max_hops in 1..=distance {
        let mut samples = Vec::with_capacity(iterations);
        let mut last = serde_json::Value::Null;
        for _ in 0..iterations {
            let started = Instant::now();
            last = service
                .traverse(TraverseQuery::Path {
                    resumable: false,
                    continuation: None,
                    start: "spine-0".into(),
                    target: format!("spine-{distance}"),
                    predicates: vec!["depends_on".into()],
                    budget: BoundedWalkBudget {
                        max_hops,
                        max_expansions: 100_000,
                        max_edges_per_node: fanout + 1,
                        max_examined_edges: 100_000,
                        max_results: 10,
                        max_evidence_per_hop: 20,
                    },
                })?
                .into_value()?;
            samples.push(started.elapsed().as_micros());
        }
        samples.sort_unstable();
        println!(
            "{},{},{},{},{},{}",
            max_hops,
            last["status"].as_str().unwrap_or("error"),
            percentile(&samples, 1, 2),
            percentile(&samples, 95, 100),
            last["search_receipt"]["expanded_nodes"],
            last["search_receipt"]["examined_edges"]
        );
    }
    Ok(())
}
