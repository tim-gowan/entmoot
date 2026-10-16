//! Compare owner materialization with bounded staging on the same dependency fanout.
//! cargo run -p entmoot-engine --example buffered_publication_bench -- buffered 10000
use entmoot_contracts::{Assertion, AssertionObject, EntityKind, EntityRef, EpistemicStatus};
use entmoot_engine::{
    KnowledgeDependency, KnowledgeRecordRef, KnowledgeRepository, KnowledgeStore, OwnerBatch,
    PublicationBuffer, SqliteKnowledgeRepository,
};
use serde_json::json;
use std::time::Instant;

fn assertion(id: String) -> Assertion {
    Assertion {
        assertion_id: id,
        subject: EntityRef {
            entity_kind: EntityKind::Callable,
            entity_id: "fixture".into(),
        },
        predicate: "fixture.property".into(),
        object: AssertionObject {
            entity: None,
            value: Some("candidate".into()),
        },
        epistemic_status: EpistemicStatus::Observed,
        evidence_refs: vec![],
        derivation: None,
        supports: vec![],
        contradicts: vec![],
        observation_context_id: None,
        valid_from: None,
        valid_to: None,
        claim_scope: Some(entmoot_contracts::ClaimScope::Occurrence),
        subject_address: None,
    }
}
fn batch(start: usize, end: usize) -> OwnerBatch {
    let mut store = KnowledgeStore::default();
    let mut dependencies = Vec::new();
    for i in start..end {
        let id = format!("derived-{i:08}");
        dependencies.push(KnowledgeDependency::new(
            KnowledgeRecordRef::new("assertions", &id),
            vec![KnowledgeRecordRef::new("assertions", "input-v1")],
        ));
        store.assertions.push(assertion(id));
    }
    OwnerBatch {
        store,
        dependencies,
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args()
        .nth(1)
        .ok_or("mode required: buffered or materialized")?;
    let count: usize = std::env::args()
        .nth(2)
        .ok_or("record count required")?
        .parse()?;
    if count == 0 || !["buffered", "materialized"].contains(&mode.as_str()) {
        return Err("invalid benchmark arguments".into());
    }
    let directory = tempfile::tempdir()?;
    let mut repository =
        SqliteKnowledgeRepository::open(&directory.path().join("knowledge.sqlite3"))?;
    repository.replace_owner(
        "input",
        &KnowledgeStore {
            assertions: vec![assertion("input-v1".into())],
            ..Default::default()
        },
        &[],
    )?;
    let publish = Instant::now();
    let mut peak_batch_bytes = None;
    let mut peak_batch_records = None;
    let mut batches = None;
    let stats = if mode == "buffered" {
        let mut stream = (0..count)
            .step_by(256)
            .map(|start| Ok(batch(start, (start + 256).min(count))));
        let result = repository.replace_owner_buffered(
            "producer",
            &mut stream,
            PublicationBuffer::default(),
        )?;
        peak_batch_bytes = Some(result.peak_batch_bytes);
        peak_batch_records = Some(result.peak_batch_records);
        batches = Some(result.batches);
        result.publication
    } else {
        let value = batch(0, count);
        repository.replace_owner("producer", &value.store, &value.dependencies)?
    };
    let publish_ms = publish.elapsed().as_secs_f64() * 1000.;
    let invalidate = Instant::now();
    let changed = repository.replace_owner(
        "input",
        &KnowledgeStore {
            assertions: vec![assertion("input-v2".into())],
            ..Default::default()
        },
        &[],
    )?;
    let invalidate_ms = invalidate.elapsed().as_secs_f64() * 1000.;
    if stats.became_visible != count || changed.became_inactive != count + 1 {
        return Err("unexpected dependency frontier counts".into());
    }
    let output = json!({"mode":mode,"records":count,"publish_ms":publish_ms,"invalidation_ms":invalidate_ms,
        "inserted_records":stats.inserted_records,"invalidated_records":changed.became_inactive,"evaluated_records":changed.evaluated_records,
        "peak_batch_bytes":peak_batch_bytes,"peak_batch_records":peak_batch_records,"batches":batches,
        "database_bytes":std::fs::metadata(directory.path().join("knowledge.sqlite3"))?.len()});
    println!("{}", output);
    Ok(())
}
