//! Replay identical typed requests with and without disposable accelerators.
//! Args: graph.json store.sqlite3 cases.json [rounds=20] [clients=16] [requests=1000]
//! Or: --chain-fixture .tmp/query-scale-chain (authored 120-hop Rust workload).
use entmoot_contracts::query::{QueryRequest, QueryService};
use entmoot_contracts::{CloneRequest, TestImpactRequest, UnusedRequest};
use entmoot_query::{query::QueryEngineOptions, QueryEngine, TopologyGraph};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    request: Operation,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Operation {
    Query(Box<QueryRequest>),
    Assessment(Assessment),
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Assessment {
    Clones { request: CloneRequest },
    Unused { request: UnusedRequest },
    TestImpact { request: TestImpactRequest },
}

fn stable(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("budget") && map.contains_key("elapsed_micros") {
                for name in [
                    "elapsed_micros",
                    "index_micros",
                    "evidence_micros",
                    "reused_discovery",
                    "examined_shortcuts",
                    "index_examined_edges",
                    "expanded_nodes",
                    "examined_edges",
                    "cumulative_expanded_nodes",
                    "cumulative_examined_edges",
                    "cumulative_examined_shortcuts",
                ] {
                    map.remove(name);
                }
            }
            for v in map.values_mut() {
                stable(v);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(stable),
        _ => {}
    }
}

fn run(
    engine: &QueryEngine,
    case: &Case,
) -> Result<(u128, Value, usize, Value), Box<dyn std::error::Error + Send + Sync>> {
    let now = Instant::now();
    let mut value = match &case.request {
        Operation::Query(request) => engine.execute(*request.clone())?.into_value()?,
        Operation::Assessment(Assessment::Clones { request }) => {
            serde_json::to_value(engine.clones(request.clone())?)?
        }
        Operation::Assessment(Assessment::Unused { request }) => {
            serde_json::to_value(engine.unused(request.clone())?)?
        }
        Operation::Assessment(Assessment::TestImpact { request }) => {
            serde_json::to_value(engine.test_impact(request.clone())?)?
        }
    };
    let query_us = now.elapsed().as_micros();
    let serialize = Instant::now();
    let bytes = serde_json::to_vec(&value)?.len();
    let serialization_us = serialize.elapsed().as_micros();
    let receipt = value
        .get("search_receipt")
        .or_else(|| value.pointer("/result/search_receipt"))
        .cloned()
        .unwrap_or(Value::Null);
    let part = |name: &str| receipt[name].as_u64().unwrap_or(0);
    let phases = json!({"query_us":query_us,"serialization_us":serialization_us,"index_us":part("index_micros"),"evidence_us":part("evidence_micros"),"search_and_reconstruction_us":part("elapsed_micros").saturating_sub(part("index_micros")).saturating_sub(part("evidence_micros")),"search_receipt":receipt,"accelerator_retained_bytes":engine.accelerator_retained_bytes()});
    let micros = now.elapsed().as_micros();
    stable(&mut value);
    Ok((micros, value, bytes, phases))
}

fn chain_fixture(
    root: &std::path::Path,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    use entmoot_query::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};
    std::fs::create_dir_all(root)?;
    let source = root.join("source");
    std::fs::create_dir_all(&source)?;
    let mut rust = "fn step0() {}\n".to_owned();
    for n in 1..=120 {
        rust.push_str(&format!("fn step{n}() {{ step{}(); }}\n", n - 1));
    }
    std::fs::write(source.join("chain.rs"), rust)?;
    let mut graph =
        entmoot_query::build_topology_from_repo_with_options(&source, Default::default())
            .map_err(|e| e.to_string())?;
    let node = |label: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.kind == entmoot_query::NodeKind::Callable && n.label == label)
            .map(|n| n.id.clone())
            .ok_or("fixture callable missing")
    };
    // Independently authored relations over production occurrence IDs. Keep the
    // extractor's parallel call occurrences intact as the negative control.
    let scope = entmoot_contracts::Scope {
        domain: graph.snapshot.repository_id.clone(),
        snapshot_revision: Some(graph.snapshot.revision.clone()),
        snapshot_digest: graph.snapshot.snapshot_digest.clone(),
        occurrence_id: None,
        context_id: None,
    };
    let authored = (1..=120)
        .map(|n| {
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                entmoot_contracts::relation_edge_with_id(
                    "fixture",
                    entmoot_contracts::topology_node_address(&node(&format!("step{n}"))?),
                    entmoot_contracts::RelationKind::DependsOn,
                    entmoot_contracts::topology_node_address(&node(&format!("step{}", n - 1))?),
                    scope.clone(),
                    None,
                ),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut cases = Vec::new();
    for predicate in ["calls", "depends_on"] {
        for (id, start, target) in [
            ("full_chain", "step120", "step0"),
            ("interior_chain", "step99", "step21"),
        ] {
            cases.push(json!({"id":format!("{predicate}_{id}"),"request":{"operation":"traverse","request":{"kind":"path","start":node(start)?,"target":node(target)?,"predicates":[predicate],"budget":{"max_hops":128,"max_expansions":10000,"max_examined_edges":10000,"max_edges_per_node":0,"max_results":1}},"output":{"tier":"export"}}}));
        }
    }
    graph.relation_edges.extend(authored);
    let graph_path = root.join("graph.json");
    let store = root.join("knowledge.sqlite3");
    let requests = root.join("cases.json");
    entmoot_query::write_graph_json(&graph, &graph_path).map_err(|e| e.to_string())?;
    let mut repository = SqliteKnowledgeRepository::open(&store)?;
    repository.replace_all(&KnowledgeStore::extract_from_graph(&graph))?;
    repository.store_inventory(&graph, None)?;
    std::fs::write(&requests, serde_json::to_vec_pretty(&cases)?)?;
    Ok(vec![
        graph_path.to_string_lossy().into_owned(),
        store.to_string_lossy().into_owned(),
        requests.to_string_lossy().into_owned(),
        "5".into(),
        "4".into(),
        "24".into(),
    ])
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--chain-fixture") {
        args = chain_fixture(std::path::Path::new(
            args.get(1)
                .ok_or("chain fixture requires an output directory")?,
        ))?;
    }
    if args.len() < 3 {
        return Err("expected graph, store, cases, optional rounds/clients/requests".into());
    }
    let rounds: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(20);
    let clients: usize = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(16);
    let requests: usize = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(1000);
    if rounds == 0 || !(1..=256).contains(&clients) || requests == 0 {
        return Err("positive rounds/requests and clients 1..=256 required".into());
    }
    let read_started = Instant::now();
    let input = std::fs::read(&args[0])?;
    let read_ms = read_started.elapsed().as_millis();
    let graph_digest = entmoot_contracts::blake3_hex(&input);
    let now = Instant::now();
    let mut graph: TopologyGraph = serde_json::from_slice(&input)?;
    let parse_ms = now.elapsed().as_millis();
    drop(input);
    graph.rebuild_indexes();
    graph.validate()?;
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    if cases.is_empty() {
        return Err("empty cases".into());
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(clients)
        .build()?;
    let mut expected = Vec::new();
    let mut arms = Vec::new();
    for (arm, enabled, lazy) in [
        ("scan", false, false),
        ("indexed", true, false),
        ("lazy", true, true),
    ] {
        let now = Instant::now();
        let mut engine = QueryEngine::with_options(
            if lazy {
                entmoot_query::SqliteKnowledgeRepository::open_read_snapshot(std::path::Path::new(
                    &args[1],
                ))?
                .load_inventory(true)?
                .ok_or("lazy arm requires persisted inventory; regenerate the publication")?
            } else {
                graph.clone()
            },
            PathBuf::from(&args[1]),
            QueryEngineOptions {
                substring_postings_budget: if enabled {
                    QueryEngineOptions::default().substring_postings_budget
                } else {
                    0
                },
                cache_walk_discovery: enabled,
                contract_paths: enabled,
                ..Default::default()
            },
        );
        if cases.iter().any(|c| {
            matches!(
                c.request,
                Operation::Assessment(Assessment::TestImpact { .. })
            )
        }) {
            // The same-snapshot case measures no-change work; no fabricated revision delta.
            engine = engine.with_compare_graph(graph.clone());
        }
        if cases
            .iter()
            .any(|c| matches!(c.request, Operation::Assessment(Assessment::Clones { .. })))
        {
            engine = engine.with_source_store(std::sync::Arc::new(
                entmoot_query::GixSourceStore::open(&graph.root)?,
            ));
        }
        let build_ms = now.elapsed().as_millis();
        let mut measurements = Vec::new();
        for (index, case) in cases.iter().enumerate() {
            let (cold_us, cold, bytes, cold_phases) = run(&engine, case)?;
            if enabled {
                if cold != expected[index] {
                    return Err(format!("cold parity failure: {}", case.id).into());
                }
            } else {
                expected.push(cold);
            }
            let mut samples = Vec::new();
            for _ in 0..rounds {
                let (us, value, _, _) = run(&engine, case)?;
                if value != expected[index] {
                    return Err(format!("warm parity failure: {}", case.id).into());
                }
                samples.push(us);
            }
            samples.sort_unstable();
            measurements.push(json!({"id":case.id,"cold_us":cold_us,"warm_median_us":samples[rounds/2],"warm_p95_us":samples[(rounds-1)*95/100],"response_bytes":bytes,"cold_phases":cold_phases}));
        }
        let start = Instant::now();
        let samples = pool.install(|| {
            (0..requests)
                .into_par_iter()
                .map(|n| {
                    let i = n % cases.len();
                    let (us, value, _, _) = run(&engine, &cases[i])?;
                    if value != expected[i] {
                        return Err("concurrent parity failure".into());
                    }
                    Ok(us)
                })
                .collect::<Result<Vec<_>, Box<dyn std::error::Error + Send + Sync>>>()
        })?;
        let wall = start.elapsed();
        let mut sorted = samples;
        sorted.sort_unstable();
        arms.push(json!({"arm":arm,"lazy_payloads":lazy,"accelerated":enabled,"build_ms":build_ms,"cases":measurements,"concurrent":{"clients":clients,"requests":requests,"wall_ms":wall.as_millis(),"requests_per_second":requests as f64/wall.as_secs_f64(),"p95_us":sorted[(requests-1)*95/100],"semantic_parity":true}}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"schema_version":"entmoot.eval.query_scale.v1","graph_digest":graph_digest,"snapshot":graph.snapshot,"nodes":graph.nodes.len(),"relations":graph.relation_edges.len(),"read_ms":read_ms,"parse_ms":parse_ms,"rounds":rounds,"arms":arms,"limits":["Machine query replay, not agent reasoning or distributed writer qualification.","Clients are bounded in-process workers; request count is not simultaneous agent count.","Search timing includes witness reconstruction; evidence timing includes classification and witness JSON construction. Retained bytes are conservative cache/search accounting, not process RSS.","Reference runs first; OS caches are not controlled. Build includes a graph clone."]})
        )?
    );
    Ok(())
}
