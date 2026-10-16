//! Validated query result envelope with operation-tagged payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::query::find::FindPayload;
use crate::query::inspect::InspectPayload;
use crate::query::read::ReadPayload;
use crate::query::traverse::TraversePayload;
use crate::query::{QueryEvidenceReceipt, QueryStatus};

/// Wire discriminator for every query result envelope.
pub const QUERY_RESULT_SCHEMA: &str = "entmoot.query.result.v1";

/// Closed schema discriminator, enforced by both Serde and JSON Schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum QueryResultSchema {
    /// Typed operation-payload contract.
    #[serde(rename = "entmoot.query.result.v1")]
    V1,
}

/// Operation-tagged result payload for the four public query operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueryPayload {
    /// Find anchors.
    Find(FindPayload),
    /// Inspect exact subjects or evidence.
    Inspect(Box<InspectPayload>),
    /// Traverse typed relations.
    Traverse(TraversePayload),
    /// Read verified source.
    Read(ReadPayload),
}

/// Validated query result with typed operation payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QueryResult {
    /// Version of this result envelope.
    pub schema_version: QueryResultSchema,
    /// Information state established by the operation.
    pub status: QueryStatus,
    /// Compact operation-level evidence and limitation receipt.
    pub receipt: QueryEvidenceReceipt,
    /// Operation-specific typed payload.
    #[serde(flatten)]
    pub payload: QueryPayload,
}

impl QueryResult {
    /// Construct a typed v1 result.
    #[must_use]
    pub fn new(status: QueryStatus, receipt: QueryEvidenceReceipt, payload: QueryPayload) -> Self {
        Self {
            schema_version: QueryResultSchema::V1,
            status,
            receipt,
            payload,
        }
    }

    /// Validate and decode a JSON value into the typed query result.
    ///
    /// # Errors
    ///
    /// Returns [`QueryContractError`] when the value is not an object, the schema
    /// version is wrong, or serde cannot decode the tagged payload.
    pub fn from_value(value: Value) -> Result<Self, QueryContractError> {
        let result: Self = serde_json::from_value(value)
            .map_err(|error| QueryContractError::Invalid(error.to_string()))?;
        Ok(result)
    }

    /// Convert the validated envelope back to its JSON transport representation.
    ///
    /// # Errors
    ///
    /// Returns the serialization error instead of silently returning null.
    pub fn into_value(self) -> Result<Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

/// Failure to validate a query result at a transport boundary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueryContractError {
    /// The value could not be decoded as a typed query result.
    #[error("query result is invalid: {0}")]
    Invalid(String),
}
