//! Reproducible JSON-vs-SQLite knowledge-store microbenchmark.
//!
//! Run in release mode: `cargo run --release -p entmoot-engine
//! --example knowledge_store_bench -- 100000`.

use std::time::Instant;

use entmoot_contracts::{
    Assertion, AssertionObject, ClaimScope, EntityKind, EntityRef, EpistemicStatus,
};
use entmoot_engine::{
    KnowledgeRead, KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let row_count = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("100000")
        .parse::<usize>()?;
    let directory = tempfile::tempdir()?;
    let json_path = directory.path().join("knowledge.json");
    let sqlite_path = directory.path().join("knowledge.sqlite3");
    let store = synthetic_store(row_count);
    let selected_subject = format!("subject-{}", row_count.saturating_sub(1));

    let started = Instant::now();
    let json = serde_json::to_vec(&store)?;
    std::fs::write(&json_path, &json)?;
    let json_write = started.elapsed();

    let started = Instant::now();
    let decoded: KnowledgeStore = serde_json::from_slice(&json)?;
    let json_read = started.elapsed();
    let query_iterations = 100_u32;
    let started = Instant::now();
    let mut json_hits = 0;
    for _ in 0..query_iterations {
        json_hits = decoded
            .assertions
            .iter()
            .filter(|assertion| assertion.subject.entity_id == selected_subject)
            .count();
        std::hint::black_box(json_hits);
    }
    let json_query = started.elapsed() / query_iterations;

    let mut repository = SqliteKnowledgeRepository::open(&sqlite_path)?;
    let started = Instant::now();
    repository.replace_all(&store)?;
    let sqlite_write = started.elapsed();
    let started = Instant::now();
    let no_op_sync = repository.synchronize(&store)?;
    let sqlite_no_op_sync = started.elapsed();

    repository.assertions_for_subject(&selected_subject)?;
    let started = Instant::now();
    let mut sqlite_hits = 0;
    for _ in 0..query_iterations {
        sqlite_hits = repository.assertions_for_subject(&selected_subject)?.len();
        std::hint::black_box(sqlite_hits);
    }
    let sqlite_query = started.elapsed() / query_iterations;
    let started = Instant::now();
    let sqlite_loaded = repository.load()?;
    std::hint::black_box(sqlite_loaded.assertions.len());
    let sqlite_read = started.elapsed();
    drop(repository);

    println!("rows={row_count}");
    println!("json_write_ms={:.3}", json_write.as_secs_f64() * 1000.0);
    println!("json_read_ms={:.3}", json_read.as_secs_f64() * 1000.0);
    println!("json_query_ms={:.3}", json_query.as_secs_f64() * 1000.0);
    println!("json_bytes={}", std::fs::metadata(&json_path)?.len());
    println!("sqlite_write_ms={:.3}", sqlite_write.as_secs_f64() * 1000.0);
    println!(
        "sqlite_no_op_sync_ms={:.3}",
        sqlite_no_op_sync.as_secs_f64() * 1000.0
    );
    println!("sqlite_no_op_upserted={}", no_op_sync.upserted);
    println!("sqlite_no_op_deleted={}", no_op_sync.deleted);
    println!("sqlite_query_ms={:.3}", sqlite_query.as_secs_f64() * 1000.0);
    println!("sqlite_read_ms={:.3}", sqlite_read.as_secs_f64() * 1000.0);
    println!("sqlite_bytes={}", std::fs::metadata(&sqlite_path)?.len());
    println!("json_hits={json_hits}");
    println!("sqlite_hits={sqlite_hits}");
    Ok(())
}

fn synthetic_store(row_count: usize) -> KnowledgeStore {
    let assertions = (0..row_count)
        .map(|index| Assertion {
            assertion_id: format!("assertion-{index:012}"),
            subject: EntityRef {
                entity_id: format!("subject-{index}"),
                entity_kind: EntityKind::Opaque,
            },
            predicate: "entmoot.benchmark".into(),
            object: AssertionObject {
                entity: None,
                value: Some("present".into()),
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
        })
        .collect();
    KnowledgeStore {
        assertions,
        ..KnowledgeStore::default()
    }
}
