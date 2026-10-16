//! Source-read operation result payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SourceSnapshot;

use crate::query::common::{
    LineRange, QueryGapCode, RequestedSourceRange, ResultBounds, SchemaJson, SourceLine,
};

/// View-tagged read payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadPayload {
    /// Digest-verified source span.
    Source(SourceReadResult),
}

/// Result of a digest-verified source read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReadResult {
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Repository-relative POSIX path.
    pub path: String,
    /// Verified content digest when the path was readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    /// Total lines in the verified file when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    /// Caller-requested window including context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_range: Option<RequestedSourceRange>,
    /// Actually returned line window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_range: Option<LineRange>,
    /// Ordered source lines.
    pub items: Vec<SourceLine>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}
