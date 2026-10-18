//! Source-pattern review candidates. None of these hashes establishes behavioral equivalence.
use crate::{SiteSpan, SourceSnapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CloneLevel {
    Exact,
    Syntax,
    Alpha,
    Parameterized,
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloneExtent {
    Callable,
    Statements,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneFingerprints {
    pub exact: String,
    pub syntax: String,
    /// Only unambiguous bindings declared inside the extent are normalized;
    /// captures, written types and caller-bindable parameter names stay.
    /// Unsupported scope forms and known name-observing constructs
    /// leave this unavailable; availability is not proof of behavioral equivalence.
    pub alpha: Option<String>,
    /// Alpha form plus consistently substituted literal values; candidates only.
    pub parameterized: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CloneRequest {
    /// Explicit shared path semantics. Mutually exclusive with legacy path prefix.
    pub path_selector: Option<crate::query::PathSelector>,
    pub canonical: bool,
    pub path: Option<String>,
    pub min_tokens: usize,
    pub min_occurrences: usize,
    pub include_statements: bool,
    pub max_files: usize,
    pub max_patterns: usize,
    pub offset: usize,
    pub limit: usize,
}
impl Default for CloneRequest {
    fn default() -> Self {
        Self {
            path_selector: None,
            canonical: false,
            path: None,
            min_tokens: 30,
            min_occurrences: 2,
            include_statements: true,
            max_files: 2000,
            max_patterns: 20000,
            offset: 0,
            limit: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneOccurrence {
    pub node_id: String,
    pub path: String,
    pub span: SiteSpan,
    pub extent: CloneExtent,
    pub token_count: usize,
    pub fingerprints: CloneFingerprints,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneGroup {
    pub level: CloneLevel,
    pub fingerprint: String,
    pub language: String,
    pub members: Vec<CloneOccurrence>,
    /// Review ordering only. Overlapping groups must not be summed as savings.
    pub repeated_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneReport {
    pub canon: crate::CanonAvailability,
    pub nudge: Option<String>,
    pub method: String,
    pub implementation: crate::ImplementationFingerprint,
    pub snapshot: SourceSnapshot,
    pub request: CloneRequest,
    pub groups: Vec<CloneGroup>,
    pub bounds: crate::query::ResultBounds,
    pub scanned_files: usize,
    pub patterns: usize,
    pub alpha_unavailable_callables: usize,
    pub limitations: Vec<String>,
}
