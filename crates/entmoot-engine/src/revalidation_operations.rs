//! Durable conditional revalidation over immutable retained publications.
use crate::{
    collaboration::now_ms, revalidation::RevalidationPlan, LocalOperations, OperationError,
    SqliteKnowledgeRepository, TopologyGraph,
};
use entmoot_contracts::{
    ContentDigest, OperationKind, OperationRecord, OperationSpec, OperationState,
    RevalidationRequest, RevalidationResult, TransitionRevalidationRequest,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) fn error(error: crate::RevalidationError) -> OperationError {
    match error {
        crate::RevalidationError::Invalid(message) => OperationError::Invalid(message),
        crate::RevalidationError::BudgetExceeded { required, limit } => {
            OperationError::Budget(format!("{required} records exceed {limit}"))
        }
        crate::RevalidationError::Repository(error) => error.into(),
        crate::RevalidationError::Json(error) => error.into(),
        crate::RevalidationError::Io(error) => crate::KnowledgeRepositoryError::from(error).into(),
    }
}

pub(crate) fn ledger_view(id: &str) -> ContentDigest {
    ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
        format!("entmoot.operation.revalidation.v1\0{id}").as_bytes(),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Progress {
    pub ledger_view: ContentDigest,
    pub completed_units: usize,
    pub total_units: usize,
    pub semantic_records: usize,
}
impl Progress {
    pub(crate) fn read(item: &OperationRecord, total: usize) -> Result<Self, OperationError> {
        let progress = if item.checkpoint.is_null() {
            Self {
                ledger_view: ledger_view(item.id.as_str()),
                completed_units: 0,
                total_units: total,
                semantic_records: 0,
            }
        } else {
            serde_json::from_value(item.checkpoint.clone())?
        };
        if progress.ledger_view != ledger_view(item.id.as_str())
            || progress.total_units != total
            || progress.completed_units > total
            || progress.semantic_records > 1_000_000
        {
            return Err(OperationError::Invalid(
                "invalid revalidation checkpoint".into(),
            ));
        }
        Ok(progress)
    }
}

fn views(request: &RevalidationRequest) -> Result<Vec<ContentDigest>, OperationError> {
    if request.units.len() > 1000 {
        return Err(OperationError::Invalid(
            "revalidation supports at most 1000 policies".into(),
        ));
    }
    let mut views = vec![request.base_view.clone(), request.head_view.clone()];
    views.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    views.dedup();
    Ok(views)
}

pub(crate) fn transition(
    request: &RevalidationRequest,
    base: entmoot_contracts::SourceSnapshot,
    head: entmoot_contracts::SourceSnapshot,
) -> TransitionRevalidationRequest {
    TransitionRevalidationRequest {
        base_snapshot: base,
        head_snapshot: head,
        units: request.units.clone(),
        max_records: 1_000_000,
    }
}

fn inputs(
    db: &impl LocalOperations,
    request: &RevalidationRequest,
) -> Result<(TopologyGraph, TopologyGraph), OperationError> {
    Ok((
        db.retained_graph(request.base_view.as_str())?,
        db.retained_graph(request.head_view.as_str())?,
    ))
}

pub fn submit(
    db: &mut impl LocalOperations,
    request: RevalidationRequest,
    key: &str,
    owner: &str,
) -> Result<OperationRecord, OperationError> {
    let views = views(&request)?;
    let (base, head) = inputs(db, &request)?;
    let validation = transition(&request, base.snapshot.clone(), head.snapshot.clone());
    RevalidationPlan::new(&base, &head, &validation).map_err(error)?;
    let spec = OperationSpec {
        repository_id: base.snapshot.repository_id,
        idempotency_key: key.into(),
        owner: owner.into(),
        kind: OperationKind::Revalidation,
        views,
        prerequisites: vec![],
        input: serde_json::to_value(request)?,
        implementation: crate::implementation_fingerprint(),
    };
    db.submit_operations(&[spec], 1000)?
        .pop()
        .ok_or_else(|| OperationError::Invalid("missing submitted revalidation".into()))
}

/// Each policy is atomic. Saved prefixes never expose a completed ledger, and
/// a larger execution allowance can continue them without replaying publication.
pub fn run(
    store: &Path,
    id: &str,
    worker: &str,
    steps: usize,
    max_records: usize,
    wall_time_ms: Option<u64>,
) -> Result<OperationRecord, OperationError> {
    if !(1..=1000).contains(&steps) || !(1..=1_000_000).contains(&max_records) {
        return Err(OperationError::Invalid(
            "revalidation requires 1..1000 steps and 1..1000000 records".into(),
        ));
    }
    let started = std::time::Instant::now();
    let expired = || wall_time_ms.is_some_and(|ms| started.elapsed().as_millis() >= ms as u128);
    let mut db = SqliteKnowledgeRepository::open(store)?;
    let mut item = db
        .operation(id)?
        .ok_or_else(|| OperationError::Invalid("unknown revalidation operation".into()))?;
    let request: RevalidationRequest = serde_json::from_value(item.spec.input.clone())?;
    if item.spec.kind != OperationKind::Revalidation || item.spec.views != views(&request)? {
        return Err(OperationError::Invalid(
            "revalidation envelope differs from frozen inputs".into(),
        ));
    }
    if item.state == OperationState::Completed {
        return Ok(item);
    }
    let (base, head) = inputs(&db, &request)?;
    if item.spec.repository_id != base.snapshot.repository_id {
        return Err(OperationError::Invalid(
            "revalidation repository differs from input".into(),
        ));
    }
    let validation = transition(&request, base.snapshot.clone(), head.snapshot.clone());
    let plan = RevalidationPlan::new(&base, &head, &validation).map_err(error)?;
    let mut progress = Progress::read(&item, plan.len())?;
    let mut ledger = SqliteKnowledgeRepository::open_view(store, progress.ledger_view.as_str())?;
    item = db.claim_operation(id, worker, now_ms()?, 3_600_000)?;
    let generation = item.generation;
    let outcome = (|| {
        // A previous worker may have advanced before this claim acquired the
        // lease. Resume from the record returned by that atomic claim.
        progress = Progress::read(&item, plan.len())?;
        if progress.semantic_records > max_records {
            return Err(OperationError::Budget(
                "revalidation record allowance below saved prefix".into(),
            ));
        }
        for _ in 0..steps {
            let active = db
                .operation(id)?
                .ok_or_else(|| OperationError::Invalid("missing revalidation operation".into()))?;
            let now = now_ms()?;
            if active.generation != generation
                || active.state != OperationState::Running
                || active.lease_until_ms.is_none_or(|t| now >= t)
            {
                return Err(OperationError::Conflict(
                    "revalidation worker revoked or expired".into(),
                ));
            }
            if expired() {
                break;
            }
            if progress.completed_units == plan.len() {
                let header = plan.receipt(vec![]);
                let result = RevalidationResult {
                    ledger_view: progress.ledger_view.clone(),
                    base_snapshot: header.base_snapshot,
                    head_snapshot: header.head_snapshot,
                    completed_units: plan.len(),
                    semantic_records: progress.semantic_records,
                    unselected_artifact_ids: header.unselected_artifact_ids,
                    limitations: header.limitations,
                };
                return ledger.publish_revalidation(
                    id,
                    generation,
                    now_ms()?,
                    item.checkpoint_sequence,
                    result,
                );
            }
            // Graph projection and policy work happen before acquiring the write transaction.
            let prepared = plan.prepare(progress.completed_units).map_err(error)?;
            let required = progress
                .semantic_records
                .saturating_add(prepared.record_count());
            if required > max_records {
                return Err(OperationError::Budget(format!(
                    "revalidation requires {required} records; allowance is {max_records}"
                )));
            }
            item = ledger.checkpoint_revalidation(
                id,
                generation,
                now_ms()?,
                item.checkpoint_sequence,
                &prepared,
                max_records,
            )?;
            progress = Progress::read(&item, plan.len())?;
        }
        db.yield_operation(
            id,
            generation,
            now_ms()?,
            "revalidation step or wall-time allowance exhausted",
        )
    })();
    match outcome {
        Ok(item) => Ok(item),
        Err(OperationError::Budget(reason)) => {
            db.yield_operation(id, generation, now_ms()?, &reason)
        }
        Err(error) => {
            let reason: String = error.to_string().chars().take(1000).collect();
            let _ = db.yield_operation(id, generation, now_ms()?, &reason);
            Err(error)
        }
    }
}

pub fn record_state(
    store: &Path,
    id: &str,
    record: entmoot_contracts::KnowledgeRecordRef,
) -> Result<entmoot_contracts::RevalidationRecordState, OperationError> {
    use crate::KnowledgeRepository;
    let db = SqliteKnowledgeRepository::open_operations_read_only(store)?;
    let item = db
        .operation(id)?
        .ok_or_else(|| OperationError::Invalid("unknown revalidation operation".into()))?;
    if item.spec.kind != OperationKind::Revalidation || item.state != OperationState::Completed {
        return Err(OperationError::Unavailable(
            "revalidation ledger is not published".into(),
        ));
    }
    let result: RevalidationResult = serde_json::from_value(
        item.result
            .ok_or_else(|| OperationError::Invalid("missing revalidation result".into()))?,
    )?;
    if result.ledger_view != ledger_view(id) {
        return Err(OperationError::Invalid(
            "revalidation ledger identity mismatch".into(),
        ));
    }
    db.require_published_revalidation(id)?;
    let ledger =
        SqliteKnowledgeRepository::open_view_read_only(store, result.ledger_view.as_str())?;
    let visible = ledger.record_state(&record)?.map(|state| state.visible);
    Ok(entmoot_contracts::RevalidationRecordState {
        ledger_view: result.ledger_view,
        record,
        visible,
    })
}
