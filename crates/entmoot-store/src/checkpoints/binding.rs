//! Validated native family progress.
use crate::OperationError;
use entmoot_contracts::{ContentDigest, OperationRecord};
use serde::{Deserialize, Serialize};
fn invalid(e: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(e.to_string())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub completed_families: usize,
    pub total_families: usize,
    pub view: ContentDigest,
}
impl Progress {
    pub fn read(
        item: &OperationRecord,
        view: &ContentDigest,
        total: usize,
    ) -> Result<Self, OperationError> {
        let progress = if item.checkpoint.is_null() {
            Self {
                completed_families: 0,
                total_families: total,
                view: view.clone(),
            }
        } else {
            serde_json::from_value(item.checkpoint["binding"].clone())?
        };
        if progress.total_families != total || progress.completed_families > total {
            return Err(invalid("invalid binding family checkpoint"));
        }
        Ok(progress)
    }
}
