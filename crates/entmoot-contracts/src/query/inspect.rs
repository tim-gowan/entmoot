//! Inspect operation result payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactManifest, Assertion, FindingObservation, LocationAttachment, LocationRecord,
    Proposition, RelationClaim, RelationEdge, SourceSnapshot, TraceRecord, TraceStepRecord,
    UnresolvedRelation,
};

use crate::query::common::{
    QueryDiagnostic, QueryGapCode, RelationNeighbor, ResultBounds, SchemaJson, StructuralEdgeRef,
    SubjectCandidate,
};

/// One finding together with the exact locations and attachment decisions that
/// selected the inspected subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachedFinding {
    /// Provider finding observation.
    pub finding: SchemaJson<FindingObservation>,
    /// Primary locations whose attachment selected this subject.
    pub locations: Vec<SchemaJson<LocationRecord>>,
    /// Exact attachment outcomes for those locations, including grain and reasons.
    pub attachments: Vec<SchemaJson<LocationAttachment>>,
}

/// File-scoped evidence visible to a contained subject through an explicit
/// structural containment chain. The finding remains attached to `file`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InheritedFileFinding {
    /// Finding and its direct file attachment.
    pub attached: AttachedFinding,
    /// File occurrence that directly carries the evidence.
    pub file: SubjectCandidate,
    /// Ordered owning structural edges from the file to the inspected subject.
    /// Edge kinds remain unchanged; the subject parent links define containment.
    pub structural_path: Vec<StructuralEdgeRef>,
}

/// Counts for all addressable subjects occurring in one file, excluding the file
/// node itself. Counts describe inventory cardinality, not evidence confidence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileDescendantCounts {
    /// Total file-scoped descendants.
    pub total: usize,
    /// Callable occurrences.
    pub callables: usize,
    /// Addressable declaration occurrences.
    pub declarations: usize,
    /// Call-site occurrences.
    pub call_sites: usize,
    /// Control-site occurrences.
    pub control_sites: usize,
    /// Configuration/resource occurrences.
    pub resources: usize,
    /// Import or module-binding occurrences.
    pub module_bindings: usize,
}

/// File containment projection for one inspected subject.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileContainmentContext {
    /// Exact enclosing file, or the subject itself when inspecting a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<SubjectCandidate>,
    /// Enclosing callable chain, nearest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enclosing_callables: Vec<SubjectCandidate>,
    /// Bounded callable occurrences in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contained_callables: Vec<SubjectCandidate>,
    /// Bounded addressable declarations in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contained_declarations: Vec<SubjectCandidate>,
    /// Bounded call-site occurrences in the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contained_call_sites: Vec<SubjectCandidate>,
    /// Complete descendant counts before result bounding.
    pub descendant_counts: FileDescendantCounts,
}

/// View-tagged inspect payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
pub enum InspectPayload {
    /// Compact subject context.
    Subject(Box<SubjectInspectResult>),
    /// Finding or assertion evidence explanation.
    Evidence(EvidenceInspectResult),
    /// Paged finding/trace steps.
    Trace(TraceInspectResult),
    /// Proposition with occurrences and claims.
    Proposition(PropositionInspectResult),
    /// Relation claims for one edge.
    RelationClaims(RelationClaimsInspectResult),
    /// Keyset-paged materialized callers or callees.
    Calls(CallNeighborhoodResult),
    /// Independent keyset page over original unresolved attempts.
    Unresolved(UnresolvedPageResult),
}

/// Direction of a materialized call-neighborhood page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallDirection {
    /// Incoming `calls` relations.
    Callers,
    /// Outgoing `calls` relations.
    Callees,
}

/// One call relation and every active producer claim returned for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallRelationHit {
    /// Scoped relation occurrence.
    pub relation: SchemaJson<RelationEdge>,
    /// Opposite endpoint in the requested direction.
    pub opposite: SchemaJson<crate::Address>,
    /// Producer-scoped support/refutation records.
    pub claims: Vec<SchemaJson<RelationClaim>>,
}

/// Cursor that prevents a call-neighborhood page from being continued against
/// a different source snapshot or durable visibility state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallPageCursor {
    /// Commitment to the resolved subject, direction, graph and store state.
    /// Visibility events alone cannot fence full-store replacement.
    pub query_generation_digest: crate::ContentDigest,
    /// Last relation edge included in the prior page.
    pub relation_edge_id: String,
    /// Repository-local visibility event cursor observed for the prior page.
    pub visibility_cursor: u64,
    /// Exact source revision observed for the prior page.
    pub revision: String,
    /// Snapshot content identity when supplied by the source snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
}

/// Pinned keyset page over materialized call relations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallNeighborhoodResult {
    /// Completeness is independent for repository, dependencies and native libraries.
    #[serde(default)]
    pub completeness: super::CallGraphCompleteness,
    /// Exact indexed snapshot.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Requested subject selector.
    pub subject: String,
    /// Incoming or outgoing direction.
    pub direction: CallDirection,
    /// Materialized relation occurrences and claims.
    pub items: Vec<CallRelationHit>,
    /// Open outgoing call attempts; populated for callees.
    pub unresolved_relations: Vec<SchemaJson<UnresolvedRelation>>,
    /// Durable visibility state read for this page.
    pub visibility_cursor: u64,
    /// Exclusive pinned cursor for the next relation page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<CallPageCursor>,
    /// Page cardinality and truncation.
    pub bounds: ResultBounds,
    /// Independently bounded unresolved-attempt collection.
    pub unresolved_bounds: ResultBounds,
    /// Continue the undisplayed attempts through `inspect unresolved`, separately
    /// from the concrete-edge cursor. Exhaustion does not establish closure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_continuation: Option<Box<super::InspectQuery>>,
    /// Explicit incompleteness and resolution limitations.
    pub gaps: Vec<QueryGapCode>,
}

/// Continuation pinned to the exact subject, source snapshot and attempt bytes.
/// This cursor is an equality guard, not an authorization token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedPageCursor {
    /// Last attempt returned, excluded from the next page.
    pub unresolved_relation_id: crate::UnresolvedRelationId,
    /// Resolved inventory node identity, independent of selector spelling.
    pub subject_id: String,
    /// Complete indexed source identity.
    pub snapshot: SourceSnapshot,
    /// Durable visibility state for the collection.
    pub visibility_cursor: u64,
    /// Digest of the complete sorted attempt records, including provider evidence.
    pub collection_digest: crate::ContentDigest,
}

/// Bounded detail page; exhaustion covers represented attempts only, never all
/// possible relationships or compiler resolution coverage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedPageResult {
    /// Exact indexed snapshot.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Original exact subject selector.
    pub subject: String,
    /// Original provider-attributed attempts in stable identity order.
    pub items: Vec<SchemaJson<UnresolvedRelation>>,
    /// Durable visibility state read for this page.
    pub visibility_cursor: u64,
    /// Exclusive cursor; absent when this represented collection is exhausted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<UnresolvedPageCursor>,
    /// Counts over this subject's represented attempts.
    pub bounds: ResultBounds,
    /// Open-world and selector limitations.
    pub gaps: Vec<QueryGapCode>,
}

/// Ambiguous or missing subject inspect outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SubjectInspectTarget {
    /// Resolved inventory subject card.
    Resolved(Box<SubjectCandidate>),
    /// Unresolved or ambiguous selector string.
    Selector(String),
}

/// Result of inspecting one inventory subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectInspectResult {
    /// Declared code boundary from this snapshot; not a resolved or inferred type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_contract: Option<crate::CodeContract>,
    /// Exact declaration and initializer syntax. This never asserts runtime values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<crate::DeclarationMarker>,
    /// Exact syntax sites when inspecting a callsite; no parameter binding or flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_site: Option<crate::CallSiteMarker>,
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Resolved subject or original selector.
    pub subject: SubjectInspectTarget,
    /// Structural projection edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structural_edges: Vec<StructuralEdgeRef>,
    /// Structural neighbors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub neighbors: Vec<SubjectCandidate>,
    /// Typed relation neighbors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_neighbors: Vec<RelationNeighbor>,
    /// Relation edges for the subject.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<SchemaJson<RelationEdge>>,
    /// Relation claims for those edges/propositions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_claims: Vec<SchemaJson<RelationClaim>>,
    /// Unresolved relation attempts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_relations: Vec<SchemaJson<UnresolvedRelation>>,
    /// Findings attached directly to the exact subject.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direct_findings: Vec<AttachedFinding>,
    /// File-scoped findings inherited through structural containment. These do not
    /// change the original attachment target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherited_file_findings: Vec<InheritedFileFinding>,
    /// File aggregate and enclosing-subject projection.
    #[serde(default)]
    pub containment: FileContainmentContext,
    /// Path-scoped diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<QueryDiagnostic>,
    /// Candidate subjects when the selector is ambiguous or empty when missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<SubjectCandidate>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// Evidence inspect body variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "evidence_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceInspectBody {
    /// Finding with related artifact, locations, attachments, and traces.
    Finding {
        /// Finding observation.
        finding: Box<SchemaJson<FindingObservation>>,
        /// Source artifact when present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<Box<SchemaJson<ArtifactManifest>>>,
        /// Primary locations.
        locations: Vec<SchemaJson<LocationRecord>>,
        /// Attachments for those locations.
        attachments: Vec<SchemaJson<LocationAttachment>>,
        /// Related traces.
        traces: Vec<SchemaJson<TraceRecord>>,
    },
    /// Assertion records matching the evidence id.
    Assertions {
        /// Matching assertions.
        assertions: Vec<SchemaJson<Assertion>>,
    },
    /// Evidence id was not found.
    Missing {
        /// Requested id.
        id: String,
    },
}

/// Result of explaining one evidence id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInspectResult {
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Evidence body.
    pub body: EvidenceInspectBody,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// One ordered trace step with location and attachments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceStepHit {
    /// Trace step record.
    pub step: SchemaJson<TraceStepRecord>,
    /// Location for the step when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SchemaJson<LocationRecord>>,
    /// Attachments for the step location.
    pub attachments: Vec<SchemaJson<LocationAttachment>>,
}

/// Result of paging a finding/trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceInspectResult {
    /// Exact snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Finding when the request resolved through a finding id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding: Option<SchemaJson<FindingObservation>>,
    /// Matching traces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<SchemaJson<TraceRecord>>,
    /// Requested id when nothing matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Ordered step page.
    pub items: Vec<TraceStepHit>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// Result of inspecting a proposition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropositionInspectResult {
    /// Exact snapshot identity when available from the engine graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SchemaJson<SourceSnapshot>>,
    /// Proposition id.
    pub proposition_id: String,
    /// Proposition record when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposition: Option<SchemaJson<Proposition>>,
    /// Scoped relation-edge occurrences.
    pub occurrences: Vec<SchemaJson<RelationEdge>>,
    /// Proposition-scoped relation claims.
    pub relation_claims: Vec<SchemaJson<RelationClaim>>,
    /// Explicit gaps / limitations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}

/// Result of inspecting relation claims for one edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationClaimsInspectResult {
    /// Exact snapshot identity when available from the engine graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SchemaJson<SourceSnapshot>>,
    /// Relation-edge id.
    pub relation_edge_id: String,
    /// Claims about the edge.
    pub relation_claims: Vec<SchemaJson<RelationClaim>>,
    /// Explicit gaps / limitations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<QueryGapCode>,
    /// Pagination and truncation receipt.
    pub bounds: ResultBounds,
}
