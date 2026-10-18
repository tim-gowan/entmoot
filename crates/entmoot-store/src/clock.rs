//! Checked wall-clock timestamps for leases and resumable reads.
use crate::OperationError;
pub fn now_ms() -> Result<u64, OperationError> {
    let value = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| OperationError::Invalid(e.to_string()))?;
    u64::try_from(value.as_millis()).map_err(|e| OperationError::Invalid(e.to_string()))
}
