//! Shared query result structures: status, gaps, bounds, subjects, and receipts.

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use crate::{
    AttachmentDisposition, AttachmentFreshness, AttachmentTargetKind, ResolutionState,
    ResolverCoverageObservation, SourceSnapshot, UnresolvedReason,
};

/// Typed `entmoot-contracts` record embedded in a query DTO.
///
/// Serde and JSON Schema preserve the normative record shape from `entmoot-contracts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchemaJson<T>(pub T);

impl<T> SchemaJson<T> {
    /// Wrap a schema record.
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrow the inner record.
    #[must_use]
    pub const fn get(&self) -> &T {
        &self.0
    }

    /// Unwrap the inner record.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for SchemaJson<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: JsonSchema> JsonSchema for SchemaJson<T> {
    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::json_schema(generator)
    }
}

/// Information state of a query result.
///
/// A result status reports what the query established; it is not confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    /// The requested information was found without a represented contradiction.
    Known,
    /// The store contains incompatible applicable facts or claims.
    Conflict,
    /// Available knowledge cannot establish the requested information.
    Unknown,
    /// The result is partial because an explicit result or work bound was reached.
    Truncated,
}

/// Stable gap / limitation codes returned by the four public operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryGapCode {
    /// Bounded named-binding inventory omits unsupported syntax and implicit values.
    DeclarationInventoryIsPartial,
    /// A source integer spelling does not establish a runtime value or property proof.
    InitializerSyntaxIsNotRuntimeValueOrPropertyEquivalence,
    /// A work, depth, result or detail budget omitted potentially relevant evidence.
    BudgetExceeded,
    /// Traversal intentionally excludes differently scoped occurrences.
    ScopeCompositionNotEstablished,
    /// No inventory subject matched the find selector.
    NoInventorySubjectMatched,
    /// Exact location has no represented inventory subject.
    NoInventorySubjectAtLocation,
    /// Path is absent from the indexed snapshot file set.
    PathNotInSnapshot,
    /// No authoritative source-store reader is configured.
    SourceStoreUnavailable,
    /// Requested line/column is outside the verified source file.
    LocationOutOfRange,
    /// No finding matched the find selector.
    NoFindingMatched,
    /// No provider-stated evidence role matched the find selector.
    NoProviderStatedRoleMatched,
    /// Role selection or call reachability does not establish value transfer.
    DataflowNotEstablished,
    /// At least one examined role lacks an eligible unique current callable mapping.
    RoleMappingNotEstablished,
    /// Subject id did not resolve to inventory.
    SubjectNotResolved,
    /// Subject id matched multiple inventory nodes.
    SubjectIsAmbiguous,
    /// A relation endpoint address has no materialized inventory node.
    RelationEndpointNotMaterialized,
    /// Configuration parse failed for the subject's path.
    ConfigurationParseFailed,
    /// Source inventory failed for the subject's path.
    SourceInventoryFailed,
    /// A file-scoped subject has no complete owning structural path to its file.
    ContainmentPathNotMaterialized,
    /// Evidence id was not found as finding, assertion, proposition, or edge.
    EvidenceIdNotFound,
    /// Finding or trace id was not found.
    FindingOrTraceNotFound,
    /// Traverse start subject is not materialized.
    StartNotMaterialized,
    /// Traverse target subject is not materialized.
    TargetNotMaterialized,
    /// No path existed within the declared walk budget.
    NoPathWithinBudget,
    /// Materialized call relations do not establish dispatch/runtime closure.
    CallGraphCompletenessNotEstablished,
    /// Exhaustive repository-local dispatch and implicit invocation evidence is missing.
    RepositoryCallGraphCompletenessNotEstablished,
    /// Dependency graph closure and possible callbacks are not qualified.
    DependencyCallGraphCompletenessNotEstablished,
    /// Native/standard library graph closure and callbacks are not qualified.
    NativeLibraryCallGraphCompletenessNotEstablished,
}

/// Pagination and truncation receipt for a result page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultBounds {
    /// Zero-based offset into the full match set.
    pub offset: usize,
    /// Maximum items returned in this page.
    pub limit: usize,
    /// Total matches before pagination (when known).
    pub total_matches: usize,
    /// Whether an explicit bound omitted additional eligible results.
    pub truncated: bool,
}

impl ResultBounds {
    /// Build bounds for a zero-offset page.
    #[must_use]
    pub fn page(limit: usize, returned: usize, total_matches: usize) -> Self {
        Self {
            offset: 0,
            limit,
            total_matches,
            truncated: total_matches > returned || total_matches > limit,
        }
    }

    /// Build bounds for an explicit offset page.
    #[must_use]
    pub fn window(offset: usize, limit: usize, returned: usize, total_matches: usize) -> Self {
        Self {
            offset,
            limit,
            total_matches,
            truncated: offset.saturating_add(returned) < total_matches,
        }
    }

    /// Untruncated singleton or empty result with no page limit applied.
    #[must_use]
    pub fn complete(total_matches: usize) -> Self {
        Self {
            offset: 0,
            limit: total_matches.max(1),
            total_matches,
            truncated: false,
        }
    }
}

/// Inventory subject card returned by find/inspect without topology types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectCandidate {
    /// Stable topology/inventory node id.
    pub node_id: String,
    /// Wire subject kind (`callable`, `call_stub`, `file`, …).
    pub kind: String,
    /// Display label.
    pub label: String,
    /// Repository-relative POSIX path when file-scoped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// One-based start line when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    /// Human-readable display reference.
    pub display_ref: String,
    /// Fully qualified name when available (display only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fqn: Option<String>,
    /// Durable address id when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_id: Option<String>,
    /// Occurrence logic address when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logic_address: Option<String>,
    /// Normalized callable body hash when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structural_hash: Option<String>,
    /// Deterministic match preference (higher is better); not confidence.
    pub match_score: u8,
}

/// Inclusive one-based line window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LineRange {
    /// First line (one-based inclusive).
    pub start_line: usize,
    /// Last line (one-based inclusive).
    pub end_line: usize,
}

/// Requested source window including caller context padding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestedSourceRange {
    /// Caller start line (one-based).
    pub start_line: usize,
    /// Caller end line (one-based).
    pub end_line: usize,
    /// Context lines applied around the caller window.
    pub context_lines: usize,
}

/// One verified source line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceLine {
    /// One-based line number.
    pub line: usize,
    /// Line text without the terminating newline.
    pub text: String,
}

/// Compact evidence pointers retained with an inspect/find hit.
///
/// Full provider records remain in view-specific item fields. The operation-level
/// receipt embeds these pointers and adds bounded provenance summaries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReceipt {
    /// Finding observation ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finding_ids: Vec<String>,
    /// Assertion ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertion_ids: Vec<String>,
    /// Artifact ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_ids: Vec<String>,
    /// Trace ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace_ids: Vec<String>,
    /// Relation-edge ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_edge_ids: Vec<String>,
    /// Relation-claim ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_claim_ids: Vec<String>,
    /// Proposition ids referenced by the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposition_ids: Vec<String>,
}

/// One provider represented in the evidence returned by an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderEvidenceReceipt {
    /// Provider name as stated by an artifact or relation producer.
    pub name: String,
    /// Provider version when stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Stable producer ids observed for this name/version pair.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub producer_ids: Vec<String>,
    /// Immutable artifacts attributed to this provider in the result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_ids: Vec<String>,
}

/// Count of attachment decisions with one disposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentDispositionCount {
    /// Attachment outcome.
    pub disposition: AttachmentDisposition,
    /// Number of returned attachment records with this outcome.
    pub count: usize,
}

/// Count of selected attachment targets at one semantic grain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentGrainCount {
    /// Selected target grain.
    pub target_kind: AttachmentTargetKind,
    /// Number of returned attachments selecting this grain.
    pub count: usize,
}

/// Attachment outcomes represented in one bounded query result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentEvidenceSummary {
    /// Distinct attachment records represented by the result.
    pub attachment_count: usize,
    /// Counts by explicit attachment disposition.
    pub dispositions: Vec<AttachmentDispositionCount>,
    /// Counts by selected target grain; unresolved targets do not appear here.
    pub selected_grains: Vec<AttachmentGrainCount>,
    /// Distinct freshness states represented by returned attachments.
    pub freshness: Vec<AttachmentFreshness>,
}

/// Relation claim and resolution states represented in one result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationEvidenceSummary {
    /// Distinct scoped relation occurrences represented by the result.
    pub relation_edge_count: usize,
    /// Distinct relation claims represented or examined by the result.
    pub relation_claim_count: usize,
    /// Active relation claims among those represented.
    pub active_claim_count: usize,
    /// Distinct active resolution states applicable to represented edges: matching
    /// subject, `resolution_state` predicate, and exact scope. Evidence classes,
    /// not confidence; other claims remain in the counts and original evidence.
    pub active_resolution_states: Vec<ResolutionState>,
    /// Bounded edge ids with candidate support.
    pub candidate_relation_edge_ids: Vec<String>,
    /// Bounded edge ids with conflicting active states or explicit contradiction
    /// links between active, scope-applicable claims. Endpoints remain unchanged.
    pub conflicted_relation_edge_ids: Vec<String>,
    /// Bounded edge ids with stale support.
    pub stale_relation_edge_ids: Vec<String>,
    /// Bounded edge ids without an applicable active resolution claim.
    pub unclaimed_relation_edge_ids: Vec<String>,
    /// Whether any relation-id list was capped.
    pub truncated: bool,
}

/// Explicit unresolved relation attempts represented in one result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedEvidenceSummary {
    /// Total distinct unresolved attempts represented before receipt bounding.
    pub count: usize,
    /// Bounded unresolved-relation ids.
    pub unresolved_relation_ids: Vec<String>,
    /// Distinct provider reason codes.
    pub reasons: Vec<UnresolvedReason>,
    /// Whether unresolved ids were capped.
    pub truncated: bool,
}

/// Result of authoritative source digest verification for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceVerificationStatus {
    /// The operation did not request source bytes.
    NotRequested,
    /// Source bytes were read and matched the indexed digest.
    Verified,
    /// Source bytes could not be obtained or verified.
    Unavailable,
}

/// Source verification outcome for one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceVerificationReceipt {
    /// Verification outcome.
    pub status: SourceVerificationStatus,
    /// Repository-relative path when source was requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Digest verified against authoritative bytes when successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// Compact operation-level provenance and limitation receipt carried by every
/// public query result and therefore every MCP response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryEvidenceReceipt {
    /// Exact repository revision and working-tree digest, when applicable.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Exact evidence records referenced by the returned payload.
    pub references: EvidenceReceipt,
    /// Whether any reference-id collection exceeded the receipt bound.
    pub references_truncated: bool,
    /// Providers represented by returned evidence, sorted by name and version.
    pub providers: Vec<ProviderEvidenceReceipt>,
    /// Whether the provider list exceeded the receipt bound.
    pub providers_truncated: bool,
    /// Attachment outcome and target-grain summary.
    pub attachments: AttachmentEvidenceSummary,
    /// Relation and claim-strength summary.
    pub relations: RelationEvidenceSummary,
    /// Explicit unresolved-hop summary.
    pub unresolved: UnresolvedEvidenceSummary,
    /// Bounded applicable resolver coverage observations.
    pub resolver_coverage: Vec<SchemaJson<ResolverCoverageObservation>>,
    /// Whether applicable resolver coverage exceeded the receipt bound.
    pub resolver_coverage_truncated: bool,
    /// Authoritative source digest verification outcome.
    pub source_verification: SourceVerificationReceipt,
}

impl QueryEvidenceReceipt {
    /// Construct a valid inventory-only receipt for an exact snapshot.
    #[must_use]
    pub fn inventory_only(snapshot: SourceSnapshot) -> Self {
        Self {
            snapshot: SchemaJson::new(snapshot),
            references: EvidenceReceipt::default(),
            references_truncated: false,
            providers: Vec::new(),
            providers_truncated: false,
            attachments: AttachmentEvidenceSummary::default(),
            relations: RelationEvidenceSummary::default(),
            unresolved: UnresolvedEvidenceSummary::default(),
            resolver_coverage: Vec::new(),
            resolver_coverage_truncated: false,
            source_verification: SourceVerificationReceipt {
                status: SourceVerificationStatus::NotRequested,
                path: None,
                content_digest: None,
            },
        }
    }
}

/// Exact snapshot scope for a query result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryScope {
    /// Exact source snapshot identity.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Optional repository-relative path constraint applied by the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Structural inventory edge projected without topology implementation types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuralEdgeRef {
    /// Source node id.
    pub from: String,
    /// Destination node id.
    pub to: String,
    /// Wire edge kind.
    pub kind: String,
    /// Whether the edge is treated as resolved in the projection.
    #[serde(default)]
    pub resolved: bool,
    /// Optional resolve confidence label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_confidence: Option<String>,
    /// Durable relation-edge id when projected from knowledge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_edge_id: Option<String>,
}

/// Path-scoped diagnostic surfaced during inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryDiagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// Severity wire label (`error`, `warning`, `info`).
    pub severity: String,
    /// Human-readable message.
    pub message: String,
    /// Related artifact id when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    /// Provider pointer (often a path) when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_pointer: Option<String>,
}

/// Neighbor reached through a typed relation edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationNeighbor {
    /// Relation edge id.
    pub edge_id: String,
    /// Relation predicate wire name.
    pub predicate: String,
    /// `outgoing` or `incoming` relative to the inspect subject.
    pub direction: String,
    /// Opposite endpoint address.
    pub address: SchemaJson<crate::Address>,
    /// Materialized subjects for the opposite endpoint.
    pub subjects: Vec<SubjectCandidate>,
}
