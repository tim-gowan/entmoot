//! Information delivery, independent of search budgets and evidence strength.
use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AttachmentEvidenceSummary, PathClassification, QueryGapCode, QueryRequest, QueryResult,
    QueryStatus, ResultBounds, SeededPathReport, SourceVerificationReceipt,
};
use crate::{
    Address, ContentDigest, EpistemicStatus, ResolutionState, Scope, SourceSnapshot,
    UnresolvedReason,
};

/// Consumer information level. No tier changes relation or role meaning.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InformationTier {
    /// Small navigation page; full records remain addressable through inspect/export.
    #[default]
    Compact,
    /// Expanded selected result with at most three details per traversal list.
    Explain,
    /// Original typed result with all details allowed by the explicit query budgets.
    Export,
}

/// Delivery controls shared by CLI and MCP. Search limits remain in the query.
///
/// Compact defaults to ten items and 16 KiB; explain to 64 KiB; export to 64 MiB.
/// Byte limits measure the UTF-8 JSON value, excluding transport framing/newlines.
/// The limit applies per response, including each member of a CLI batch; it is
/// not a batch or MCP protocol-envelope limit. No tier fetches or changes facts.
/// Expanded results are never silently cut: request narrower evidence or raise the
/// explicit byte budget when the complete envelope cannot fit.
///
/// ```
/// use entmoot_contracts::query::{InformationTier, QueryRequest};
/// let request: QueryRequest = serde_json::from_str(r#"{
///   "operation":"find", "request":{"kind":"subject","query":"handler"},
///   "output":{"tier":"explain","max_bytes":65536}
/// }"#)?;
/// assert_eq!(request.output().tier, InformationTier::Explain);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OutputOptions {
    /// Requested information tier; omission means compact.
    pub tier: InformationTier,
    /// Compact items per page (1..=1000); does not change search exploration.
    pub limit: usize,
    /// Optional UTF-8 serialized byte ceiling (128..=67108864).
    pub max_bytes: Option<usize>,
    /// Compact continuation. Rejected for another query, snapshot, or store state.
    pub after: Option<OutputCursor>,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            tier: InformationTier::Compact,
            limit: 10,
            max_bytes: None,
            after: None,
        }
    }
}

impl OutputOptions {
    /// Effective byte ceiling. Validation belongs to the serving engine.
    #[must_use]
    pub fn byte_limit(&self) -> usize {
        self.max_bytes.unwrap_or(match self.tier {
            InformationTier::Compact => 16 * 1024,
            InformationTier::Explain => 64 * 1024,
            InformationTier::Export => 64 * 1024 * 1024,
        })
    }
}

/// Equality fence for a compact page; never an authorization token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputCursor {
    /// Retained execution slice being paged; never advances its search frontier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_slice: Option<String>,
    /// Exact source identity, including working-tree digest where required.
    pub snapshot: SourceSnapshot,
    /// Canonical semantic request digest; output page size/byte limit are excluded.
    pub query_digest: ContentDigest,
    /// Loaded publication identity; rejects a rebuilt store with a reused visibility cursor.
    pub generation_digest: ContentDigest,
    /// Knowledge visibility state observed while executing the request.
    pub visibility_cursor: u64,
    /// Next index in the deterministically ordered bounded result, not graph closure.
    pub offset: usize,
}

/// Presentation completeness, separate from search and deferred evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputPage {
    /// Offset within this query's bounded result.
    pub offset: usize,
    /// Items present in this response.
    pub returned: usize,
    /// Items available from this bounded query; never an estimate of unseen paths.
    pub total_items: usize,
    /// Continuation for omitted items in this same bounded result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<OutputCursor>,
}

/// Portable delivery envelope schema identifier.
pub const QUERY_RESPONSE_SCHEMA: &str = "entmoot.query.response.v1";

/// Version of the tiered query response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum QueryResponseSchema {
    /// First tiered delivery contract.
    #[serde(rename = "entmoot.query.response.v1")]
    V1,
}

/// Readable endpoint. The dictionary key retains the exact portable node/address id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactSubject {
    /// Human-readable reference; not an alternative identity.
    pub display_ref: String,
    /// Inventory kind, or `external_address` when no node is represented.
    pub kind: String,
    /// Repository-relative path when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// One-based starting line when represented.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    /// Original external address when an inventory node is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_address: Option<Address>,
}

/// Exact edge identity is its dictionary key; inspect it for full producer claims.
/// Omitted qualification/count fields mean unexamined, never zero or disproven.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactEdge {
    /// Caller/source subject dictionary key; direction is never reversed for display.
    pub subject: String,
    /// Callee/target subject dictionary key.
    pub object: String,
    /// Original relation meaning.
    pub predicate: crate::RelationKind,
    /// Response-local scope dictionary key; not a durable or database identity.
    pub scope: String,
    /// Applicable resolution states; absence remains unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub states: Option<Vec<ResolutionState>>,
    /// Claims examined, regardless of inline detail limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_count: Option<usize>,
    /// Active claims examined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_claim_count: Option<usize>,
    /// Exact endpoint assertion count before detail limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_assertions: Option<usize>,
    /// Exact endpoint unresolved count before detail limits; not globally additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved_calls: Option<usize>,
}

/// Compact role anchor, preserving the provider's epistemic state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactRole {
    /// Assertion identity to inspect for complete evidence and derivation.
    pub assertion_id: String,
    /// Original provider-stated role, never propagated along call edges.
    pub role: String,
    /// Original provider epistemic state.
    pub epistemic_status: EpistemicStatus,
}

/// One small answer item. Referenced records are expandable, never new graph facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompactItem {
    /// A discovered or inspected inventory subject.
    Subject {
        /// Subject dictionary key.
        subject: String,
    },
    /// One represented directed witness; not value flow or runtime execution.
    Path {
        /// Classification computed using full applicable claims, not displayed samples.
        classification: PathClassification,
        /// Ordered exact edge dictionary keys.
        edges: Vec<String>,
        /// Reachable endpoint when this is a neighborhood result.
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        /// Original endpoint roles, if requested.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        roles: Vec<CompactRole>,
    },
    /// Materialized relation occurrence; full claims are available by edge id.
    Relation {
        /// Edge dictionary key.
        edge: String,
    },
    /// Exact evidence reference, with provider reason/role when relevant.
    Evidence {
        /// Typed record family to expand.
        record_kind: CompactRecordKind,
        /// Exact record identity.
        id: String,
        /// Display reason or provider label; never confidence.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Verified source line explicitly requested by the consumer.
    SourceLine {
        /// One-based line.
        line: usize,
        /// Original text.
        text: String,
    },
}

/// Record families addressable through existing inspect operations or an expanded query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompactRecordKind {
    /// Finding observation.
    Finding,
    /// Provider assertion.
    Assertion,
    /// Ordered trace step.
    TraceStep,
    /// Proposition.
    Proposition,
    /// Producer relation claim.
    RelationClaim,
    /// Original unresolved attempt.
    Unresolved,
    /// Status-linked facet result; label retains its reason.
    Facet,
}

/// Provenance and limitations once per answer, without repeated full records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactEvidenceReceipt {
    /// Attachment outcome/grain/freshness, never inferred from the displayed node.
    pub attachments: AttachmentEvidenceSummary,
    /// Applicable states retained even when exact claim bodies are deferred.
    pub active_resolution_states: Vec<ResolutionState>,
    /// Count of evidence references represented by the operation receipt.
    pub reference_count: usize,
    /// The original bounded receipt omitted additional metadata; details can be inspected.
    pub receipt_truncated: bool,
    /// Provider name/version pairs, sorted; full attribution remains in expanded receipts.
    pub providers: Vec<CompactProvider>,
    /// Applicable resolver coverage records, preserved by scope; never summed across scopes.
    pub resolver_coverage: Vec<crate::ResolverCoverageObservation>,
    /// Known unresolved reasons from the represented receipt; not an exhaustive universe.
    pub unresolved_reasons: Vec<UnresolvedReason>,
    /// Source-verification outcome, including `not_requested`.
    pub source_verification: SourceVerificationReceipt,
}

/// Minimal producer attribution. Exact artifacts and configuration are expandable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactProvider {
    /// Producer name.
    pub name: String,
    /// Version as provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Role selection facts remain visible when bulky evidence is deferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactRoleSelection {
    /// Original admission policy.
    pub selector: super::ProviderRoleSelector,
    /// Matching assertions before selection budgets.
    pub matching_assertions: usize,
    /// Hypotheses excluded without opt-in.
    pub excluded_hypotheses: usize,
    /// Examined assertions lacking eligible callable mappings.
    pub unmapped_assertions: usize,
    /// Selection budget omitted matching assertions.
    pub selection_truncated: bool,
    /// Original roles on the root; never ordering or value flow.
    pub colocated_count: usize,
}

/// Default navigational answer. Dictionaries contain only records needed by this page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactQueryResponse {
    /// Evidence that could not be inspected, including unretained native detail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    /// Repository, dependency and native-library closure are independent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completeness: Option<super::CallGraphCompleteness>,
    /// Traversal slice, work and search continuation, independent of output paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_receipt: Option<Box<super::TraverseSearchReceipt>>,
    /// Tiered wire contract version.
    pub schema_version: QueryResponseSchema,
    /// Original operation outcome; presentation caps cannot strengthen it.
    pub status: QueryStatus,
    /// Exact source identity once per response.
    pub snapshot: SourceSnapshot,
    /// Original semantic request with default output controls, for explicit expansion.
    pub query: Box<QueryRequest>,
    /// Compact provenance and limitations.
    pub receipt: CompactEvidenceReceipt,
    /// Original search/result bounds, independent of this output page.
    pub result_bounds: ResultBounds,
    /// Traversal work/depth limits left an unexplored frontier; independent of paging.
    pub search_truncated: bool,
    /// Interpretation limits from the operation, never silently omitted.
    pub gaps: Vec<QueryGapCode>,
    /// Open-attempt collection bounds when available; zero displayed is not zero known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved_bounds: Option<ResultBounds>,
    /// Retrieve open attempts through the shared inspect operation, independently
    /// of concrete-edge or presentation paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_continuation: Option<Box<super::InspectQuery>>,
    /// Role admission, unmapped and co-location counts when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_selection: Option<CompactRoleSelection>,
    /// Full evidence and less-used metadata are available by inspect/explain/export.
    pub details_deferred: bool,
    /// Output page and pinned continuation.
    pub page: OutputPage,
    /// Ordered answer items.
    pub items: Vec<CompactItem>,
    /// Readable exact endpoint dictionary.
    pub subjects: BTreeMap<String, CompactSubject>,
    /// Unique scoped edge dictionary.
    pub edges: BTreeMap<String, CompactEdge>,
    /// Exact scopes, interned without merging unequal contexts.
    pub scopes: BTreeMap<String, Scope>,
}

/// Expanded selected query, still bounded by its original search and detail budgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpandedQueryResponse {
    /// Tiered wire contract version.
    pub schema_version: QueryResponseSchema,
    /// Some traversal details remain addressable outside this response.
    pub details_deferred: bool,
    /// Original typed query result; export does not mean exhaustive graph closure.
    pub result: QueryResult,
}

/// Shared CLI/MCP information response. Consumers discriminate on the explicit tier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum QueryResponse {
    /// Default small navigational page.
    Compact(CompactQueryResponse),
    /// Expanded selected evidence.
    Explain(ExpandedQueryResponse),
    /// Full original bounded result.
    Export(ExpandedQueryResponse),
}

impl QueryResponse {
    /// Serialize the typed response for a transport.
    ///
    /// # Errors
    /// Returns the serializer error without replacing evidence with null.
    pub fn into_value(self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

/// Seeded-route delivery uses the same output policy; export preserves the original report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum SeededPathResponse {
    /// Bounded compact route pages with independent discovery and presentation states.
    Compact(CompactSeededResponse),
    /// Original report with at most three traversal details per list.
    Explain {
        /// Bounded report.
        report: SeededPathReport,
    },
    /// Original report and all evidence allowed by explicit budgets.
    Export {
        /// Bounded report.
        report: SeededPathReport,
    },
}

/// One compact seeded route. Co-location and missing witnesses remain explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactSeededRoute {
    /// Original route origin callable id.
    pub from_callable: String,
    /// Original destination callable id.
    pub to_callable: String,
    /// Original provider anchors at the origin; no role is inferred from path position.
    pub from_anchors: Vec<CompactRole>,
    /// Original provider anchors at the destination.
    pub to_anchors: Vec<CompactRole>,
    /// Original bounded query projected using the compact contract.
    pub query: CompactQueryResponse,
}

/// Bounded seed report; source/sink roles never become value-flow assertions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactSeededResponse {
    /// Exact source identity.
    pub snapshot: SourceSnapshot,
    /// Original request, with default output controls, for expansion.
    pub request: super::SeededPathRequest,
    /// Matching provider assertions before selection budgets.
    pub matching_assertions: usize,
    /// Hypotheses excluded by the requested admission policy.
    pub excluded_hypotheses: usize,
    /// Selected assertions without an eligible callable attachment.
    pub unmapped_assertions: usize,
    /// Callable co-locations; no ordering or flow established. Inspect export for identities.
    pub colocated_count: usize,
    /// Role selection budget was exhausted.
    pub seeds_truncated: bool,
    /// Aggregate discovery work/pair budget was exhausted.
    pub discovery_truncated: bool,
    /// Original report's interpretation constraints.
    pub limitations: Vec<String>,
    /// Full role/attachment/claim records remain addressable.
    pub details_deferred: bool,
    /// Route delivery page, distinct from discovery limits.
    pub page: OutputPage,
    /// Directed candidate route queries.
    pub routes: Vec<CompactSeededRoute>,
}
