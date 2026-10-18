//! Persisted conditional revalidation state.
use crate::OperationError;
use entmoot_contracts::*;
use serde::{Deserialize, Serialize};
pub fn error(error: crate::RevalidationError) -> OperationError {
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

pub fn ledger_view(id: &str) -> ContentDigest {
    ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
        format!("entmoot.operation.revalidation.v1\0{id}").as_bytes(),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub ledger_view: ContentDigest,
    pub completed_units: usize,
    pub total_units: usize,
    pub semantic_records: usize,
}
impl Progress {
    pub fn read(item: &OperationRecord, total: usize) -> Result<Self, OperationError> {
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

pub fn transition(
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
