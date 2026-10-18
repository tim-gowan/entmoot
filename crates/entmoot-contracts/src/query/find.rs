//! Find operation result payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Assertion, FindingObservation, LocationAttachment, LocationRecord, SourceSnapshot,
    TraceStepRecord,
};

use crate::query::common::{QueryGapCode, ResultBounds, SchemaJson, SubjectCandidate};

/// View-tagged find payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
pub enum FindPayload {
    /// Inventory subject candidates.
    Subject(SubjectFindResult),
    /// Exact location hierarchy.
    Location(Box<LocationFindResult>),
    /// Finding page.
    Finding(FindingFindResult),
    /// Provider-stated evidence roles.
    EvidenceRole(EvidenceRoleFindResult),
    /// High-signal facet page (status-linked selectors).
    Facet(FacetFindResult),
}

/// One hit from a high-signal facet find.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FacetHit {
    /// Source and native detail for a call-boundary facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<super::CallBoundaryDetail>,
    /// Stable record id (subject, finding, relation edge, unresolved attempt, …).
    pub id: String,
    /// Wire kind for the id (`callable`, `relation_edge`, `finding`, …).
    pub kind: String,
    /// Display label when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Repository-relative path when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Facet-specific reason this id was selected.
    pub reason: String,
    /// Optional degree or score retained for fan facets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<u64>,
}

/// Result of a status-linked high-signal facet find.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FacetFindResult {
    /// Evidence that could not be inspected, including unretained native detail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    /// Distinct-site partitions before paging; never a count of unresolved attempts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundary_counts: Vec<super::CallBoundaryCount>,
    /// Independent completeness universes for call-boundary queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completeness: Option<super::CallGraphCompleteness>,
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Facet wire name matching [`crate::query::FindQuery`] kind.
    pub facet: String,
    /// Ordered facet hits.
    pub items: Vec<FacetHit>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// Result of resolving text/path selectors to inventory subjects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectFindResult {
    /// Counts over evaluated candidates, before output/result limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<super::SelectionReceipt>,
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Ordered subject candidates.
    pub items: Vec<SubjectCandidate>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// Result of resolving an exact source coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationFindResult {
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Repository-relative POSIX path.
    pub path: String,
    /// Requested one-based line.
    pub line: usize,
    /// Requested one-based column when provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// Verified content digest when the path was readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    /// Most specific overlapping peer sites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<SubjectCandidate>,
    /// Overlapping call sites ordered by span size then id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub call_sites: Vec<SubjectCandidate>,
    /// Overlapping control sites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_sites: Vec<SubjectCandidate>,
    /// Enclosing callables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enclosing_callables: Vec<SubjectCandidate>,
    /// Overlapping addressable declarations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declarations: Vec<SubjectCandidate>,
    /// Overlapping resource subjects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<SubjectCandidate>,
    /// Containing file subject when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<SubjectCandidate>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// One finding with its primary locations and attachments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingHit {
    /// Finding observation record.
    pub finding: SchemaJson<FindingObservation>,
    /// Primary locations.
    pub locations: Vec<SchemaJson<LocationRecord>>,
    /// Attachment outcomes for those locations.
    pub attachments: Vec<SchemaJson<LocationAttachment>>,
}

/// Result of selecting findings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingFindResult {
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Ordered finding hits.
    pub items: Vec<FindingHit>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// One provider-stated role hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRoleHit {
    /// Provider-stated role token.
    pub role: String,
    /// Role assertion.
    pub assertion: SchemaJson<Assertion>,
    /// Location carrying the role when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SchemaJson<LocationRecord>>,
    /// Attachments for the location.
    pub attachments: Vec<SchemaJson<LocationAttachment>>,
    /// Trace steps referencing the location.
    pub trace_steps: Vec<SchemaJson<TraceStepRecord>>,
    /// Related findings.
    pub findings: Vec<SchemaJson<FindingObservation>>,
}

/// Result of selecting provider-stated evidence roles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRoleFindResult {
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Role filter applied (lowercased) when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Path filter applied when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Ordered role hits.
    pub items: Vec<EvidenceRoleHit>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}
