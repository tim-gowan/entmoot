//! Reproducible commit-transition invalidation and bounded-walk benchmark.
//!
//! Each synthetic commit changes one occurrence assertion at the next hop while
//! retaining the same topology. Relation/claim scopes carry the commit revision,
//! exposing snapshot-wide churn separately from the one-row logical change.
//!
//! Run with:
//! `cargo run --release -p entmoot-engine --example commit_update_bench -- 12 128 100`

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use entmoot_contracts::{
    relation_claim_with_id, relation_edge_with_id, Address, Assertion, AssertionObject, ClaimScope,
    EntityKind, EntityRef, EpistemicStatus, ProducerCapabilityKind, ProducerRef,
    RelationClaimValue, RelationKind, ResolutionState, Scope, SourceState, SubjectId,
};
use entmoot_engine::{
    BoundedWalkBudget, KnowledgeRepository, KnowledgeStore, NodeKind, QueryEngine,
    SqliteKnowledgeRepository, TopologyGraph, TopologyNode, TraverseQuery,
};

fn address(repository: &str, hop: usize) -> Address {
    Address::RepoPath {
        repository_id: Some(repository.into()),
        path: format!("spine/{hop}"),
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

fn add_relation(graph: &mut TopologyGraph, from: usize, to: usize) {
    let scope = Scope {
        domain: graph.snapshot.repository_id.clone(),
        snapshot_revision: Some(graph.snapshot.revision.clone()),
        snapshot_digest: None,
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
            id: "entmoot.commit-update-benchmark".into(),
            name: "commit-update-benchmark".into(),
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

fn graph_for_commit(max_hops: usize, fanout: usize, changed_hop: usize) -> TopologyGraph {
    let mut graph = TopologyGraph::new("entmoot:commit-update-benchmark");
    graph.snapshot.repository_id = "benchmark/commit-update".into();
    graph.snapshot.revision = format!("{changed_hop:040x}");
    graph.snapshot.source_state = SourceState::Committed;
    graph.snapshot.snapshot_digest = None;

    for hop in 0..=max_hops {
        add_node(&mut graph, &format!("spine-{hop}"), &format!("spine/{hop}"));
    }
    for hop in 0..max_hops {
        add_relation(&mut graph, hop, hop + 1);
        for branch in 0..fanout {
            let branch_id = format!("branch-{hop}-{branch}");
            let branch_path = format!("branches/{hop}/{branch}");
            add_node(&mut graph, &branch_id, &branch_path);
            let scope = Scope {
                domain: graph.snapshot.repository_id.clone(),
                snapshot_revision: Some(graph.snapshot.revision.clone()),
                snapshot_digest: None,
                occurrence_id: None,
                context_id: None,
            };
            let edge = relation_edge_with_id(
                "benchmark",
                address(&graph.snapshot.repository_id, hop),
                RelationKind::DependsOn,
                Address::RepoPath {
                    repository_id: Some(graph.snapshot.repository_id.clone()),
                    path: branch_path,
                    start_byte: None,
                    end_byte: None,
                },
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
                    id: "entmoot.commit-update-benchmark".into(),
                    name: "commit-update-benchmark".into(),
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
    }
    graph.assertions.push(Assertion {
        assertion_id: "changed-occurrence".into(),
        subject: EntityRef {
            entity_id: format!("spine-{changed_hop}"),
            entity_kind: EntityKind::Opaque,
        },
        predicate: "entmoot.benchmark.changed".into(),
        object: AssertionObject {
            entity: None,
            value: Some(format!("commit-{changed_hop}")),
        },
        evidence_refs: vec![],
        epistemic_status: EpistemicStatus::Observed,
        derivation: None,
        supports: vec![],
        contradicts: vec![],
        observation_context_id: None,
        valid_from: None,
        valid_to: None,
        claim_scope: Some(ClaimScope::Occurrence),
        subject_address: None,
    });
    graph.rebuild_indexes();
    graph
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let max_hops = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(12usize);
    let fanout = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(128usize);
    let iterations = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(100usize);
    if !(1..=64).contains(&max_hops) || fanout > 1_000 || iterations == 0 {
        return Err("max_hops must be 1..=64, fanout <= 1000, and iterations > 0".into());
    }

    let directory = tempfile::tempdir()?;
    let store_path = directory.path().join("knowledge.sqlite3");
    let baseline = graph_for_commit(max_hops, fanout, 0);
    let baseline_rows = KnowledgeStore::extract_from_graph(&baseline);
    let baseline_propositions: HashSet<_> = baseline
        .relation_edges
        .iter()
        .map(|edge| edge.proposition_id.clone())
        .collect();
    let baseline_count = baseline_rows.assertions.len()
        + baseline_rows.relation_edges.len()
        + baseline_rows.relation_claims.len();
    let mut repository = SqliteKnowledgeRepository::open(&store_path)?;
    repository.replace_all(&baseline_rows)?;
    drop(repository);

    println!("max_hops={max_hops} fanout={fanout} measured_rows={baseline_count}");
    println!("changed_hop,sync_ms,upserted,deleted,retained_propositions,total_propositions,walk_status,walk_median_us,expanded_nodes,examined_edges");
    for changed_hop in 1..=max_hops {
        let graph = graph_for_commit(max_hops, fanout, changed_hop);
        let store = KnowledgeStore::extract_from_graph(&graph);
        let propositions: HashSet<_> = graph
            .relation_edges
            .iter()
            .map(|edge| edge.proposition_id.clone())
            .collect();
        let retained_propositions = propositions.intersection(&baseline_propositions).count();
        let mut repository = SqliteKnowledgeRepository::open(&store_path)?;
        let started = Instant::now();
        let stats = repository.synchronize(&store)?;
        let sync_elapsed = started.elapsed();
        drop(repository);

        let service = QueryEngine::new(graph, store_path.clone());
        let mut samples = Vec::with_capacity(iterations);
        let mut result = serde_json::Value::Null;
        for _ in 0..iterations {
            let started = Instant::now();
            result = service
                .traverse(TraverseQuery::Path {
                    resumable: false,
                    continuation: None,
                    start: "spine-0".into(),
                    target: format!("spine-{changed_hop}"),
                    predicates: vec!["depends_on".into()],
                    budget: BoundedWalkBudget {
                        max_hops: changed_hop,
                        max_expansions: 100_000,
                        max_edges_per_node: fanout + 1,
                        max_examined_edges: 100_000,
                        max_results: 1,
                        max_evidence_per_hop: 20,
                    },
                })?
                .into_value()?;
            samples.push(started.elapsed());
        }
        println!(
            "{},{:.3},{},{},{},{},{},{},{},{}",
            changed_hop,
            sync_elapsed.as_secs_f64() * 1000.0,
            stats.upserted,
            stats.deleted,
            retained_propositions,
            propositions.len(),
            result["status"].as_str().unwrap_or("error"),
            median(&mut samples).as_micros(),
            result["search_receipt"]["expanded_nodes"],
            result["search_receipt"]["examined_edges"]
        );
    }
    let unchanged = graph_for_commit(max_hops, fanout, max_hops);
    let mut repository = SqliteKnowledgeRepository::open(&store_path)?;
    let started = Instant::now();
    let stats = repository.synchronize(&KnowledgeStore::extract_from_graph(&unchanged))?;
    println!(
        "no_op_sync_ms={:.3} no_op_upserted={} no_op_deleted={}",
        started.elapsed().as_secs_f64() * 1000.0,
        stats.upserted,
        stats.deleted
    );
    Ok(())
}
