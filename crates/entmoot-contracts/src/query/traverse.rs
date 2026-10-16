//! Traverse operation result payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::RelationEdge;

use crate::query::common::{QueryGapCode, ResultBounds, SchemaJson};
use crate::query::request::BoundedWalkBudget;

/// View-tagged traverse payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
pub enum TraversePayload {
    /// Bounded scoped neighborhood with one witness per returned address.
    Neighborhood(TraverseNeighborhoodResult),
    /// Bounded directed paths between two subjects.
    Path(TraversePathResult),
}

/// Path classification from claim/resolution state along hops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathClassification {
    /// All hops have applicable resolution claims without conflict. This labels
    /// the represented relation evidence, not value flow or runtime execution.
    Supported,
    /// At least one hop is candidate, stale, unclaimed, or lacks an applicable
    /// resolution-state claim for its exact occurrence scope.
    Candidate,
    /// At least one hop has conflicting claims.
    Conflicted,
}

/// One hop on a returned path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraverseHop {
    /// Direct assertions on either exact endpoint. A source/sink/sanitizer label
    /// means provider-stated membership here, never dataflow or sanitizer efficacy.
    pub endpoint_assertions: Vec<SchemaJson<crate::Assertion>>,
    /// Bounds on endpoint assertions; omitted facts cannot support negative answers.
    pub endpoint_assertion_bounds: ResultBounds,
    /// Open call frontiers on either endpoint; absent rows do not establish closure.
    pub unresolved_calls: Vec<SchemaJson<crate::UnresolvedRelation>>,
    /// Bounds on the open frontier records.
    pub unresolved_call_bounds: ResultBounds,
    /// Relation edge traversed by this hop.
    pub relation_edge: SchemaJson<RelationEdge>,
    /// Distinct active resolution states with matching subject, resolution
    /// predicate, and exact occurrence scope.
    pub active_resolution_states: Vec<String>,
    /// Total relation claims examined for the hop.
    pub relation_claim_count: usize,
    /// Active (non-inactive) claim count.
    pub active_claim_count: usize,
    /// Claim ids included in the receipt (bounded).
    pub relation_claim_ids: Vec<String>,
    /// Whether additional claim ids were omitted.
    pub relation_claims_truncated: bool,
}

/// One bounded path receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraversePath {
    /// Aggregate classification of the path.
    pub classification: PathClassification,
    /// Number of hops.
    pub hop_count: usize,
    /// Meet-in-the-middle address key.
    pub meet_address_key: String,
    /// Distinct evidence kinds observed along hops.
    pub evidence_kinds: Vec<String>,
    /// Distinct scope domains observed along hops.
    pub scope_domains: Vec<String>,
    /// Distinct context ids observed along hops.
    pub context_ids: Vec<String>,
    /// Edge ids with candidate resolution state or without an applicable
    /// resolution-state claim for their exact occurrence scope.
    pub candidate_relation_edge_ids: Vec<String>,
    /// Edge ids with conflicted resolution state.
    pub conflicted_relation_edge_ids: Vec<String>,
    /// Edge ids with stale resolution state.
    pub stale_relation_edge_ids: Vec<String>,
    /// Edge ids lacking an applicable active resolution claim.
    pub unclaimed_relation_edge_ids: Vec<String>,
    /// Ordered hops.
    pub hops: Vec<TraverseHop>,
}

/// Search budget and work counters for one traverse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraverseSearchReceipt {
    /// Current slice identity, usable for exact replay while retained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_token: Option<String>,
    /// Next execution token; absent when discovery and stable result delivery end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
    /// Total expansions across retained execution slices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cumulative_expanded_nodes: Option<usize>,
    /// Total original-edge or shortcut inspections across execution slices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cumulative_examined_edges: Option<usize>,
    /// Total shortcut inspections across execution slices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cumulative_examined_shortcuts: Option<usize>,
    /// Discovery reused for this exact request and immutable graph generation.
    /// Work counters are zero on reuse; original bounds/frontiers remain intact.
    /// Claim evidence is refreshed; reuse grants no additional authority.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reused_discovery: bool,
    /// Nodes expanded during search.
    pub expanded_nodes: usize,
    /// Edges examined during search.
    pub examined_edges: usize,
    /// Shortcut entries inspected; their original hop distances remain intact.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub examined_shortcuts: usize,
    /// Original edges inspected while building a disposable query index.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub index_examined_edges: usize,
    /// Time spent obtaining/building the selector overlay, separate from search.
    #[serde(default)]
    pub index_micros: u64,
    /// Time hydrating and classifying original-edge witnesses (including JSON construction).
    #[serde(default)]
    pub evidence_micros: u64,
    /// Original witness edges hydrated; shortcut inspection never hides this cost.
    #[serde(default)]
    pub reconstructed_edges: usize,
    /// Claim records checked for witness classification, including rejected claims.
    #[serde(default)]
    pub evidence_claims_examined: usize,
    /// Wall time for the search in microseconds.
    pub elapsed_micros: u64,
    /// Applied walk budget.
    pub budget: BoundedWalkBudget,
    /// Whether per-node fanout was truncated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fanout_truncated: bool,
    /// Whether the expansion budget was exhausted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expansion_budget_exhausted: bool,
    /// Whether the global examined-edge budget was exhausted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub edge_budget_exhausted: bool,
    /// Number of candidate meets before result capping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_meet_count: Option<usize>,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// Result of a bounded path traverse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraversePathResult {
    /// Exact indexed snapshot for this traversal.
    pub snapshot: SchemaJson<crate::SourceSnapshot>,
    /// Start subject selector.
    pub start: String,
    /// Target subject selector.
    pub target: String,
    /// Ordered path receipts.
    pub paths: Vec<TraversePath>,
    /// Explicit gaps / limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Search work receipt.
    pub search_receipt: Box<TraverseSearchReceipt>,
    /// Result-page bounds (`truncated` mirrors walk budget exhaustion).
    pub bounds: ResultBounds,
}

/// One reachable endpoint and a directed witness preserving stored edge direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeighborhoodHit {
    /// Attributable relation endpoint; external endpoints need not be inventory nodes.
    pub subject: SchemaJson<crate::Address>,
    /// One deterministic shortest represented witness, not every possible route.
    pub witness: TraversePath,
    /// Original roles at this endpoint, never propagated along the witness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_anchors: Vec<crate::query::SeedAnchor>,
    /// Lightweight original role attribution, independent of bulk evidence detail limits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_summaries: Vec<crate::query::CompactRole>,
    /// Present when role selection is requested; capped by `max_evidence_per_hop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_anchor_bounds: Option<ResultBounds>,
}

/// A filtered projection over represented evidence, never dispatch completeness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraverseNeighborhoodResult {
    /// Indexed snapshot.
    pub snapshot: SchemaJson<crate::SourceSnapshot>,
    /// Exact root selector.
    pub start: String,
    /// Edge direction followed from the root.
    pub direction: crate::query::CallDirection,
    /// Applied predicate selector.
    pub predicates: Vec<String>,
    /// Applied exact-scope evidence requirement.
    pub evidence_filter: crate::query::HopEvidenceFilter,
    /// Optional role admission receipt, including unmapped assertions and root co-location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_selection: Option<Box<crate::query::RoleSelectionReceipt>>,
    /// Endpoints in deterministic breadth-first discovery order.
    pub items: Vec<NeighborhoodHit>,
    /// Open attempts at discovered endpoints, independent of the hop filter.
    pub unresolved_calls: Vec<SchemaJson<crate::UnresolvedRelation>>,
    /// Exact unresolved count at discovered endpoints, with bounded details.
    pub unresolved_call_bounds: ResultBounds,
    /// Explicit limitations.
    pub gaps: Vec<QueryGapCode>,
    /// Work counters and limits.
    pub search_receipt: Box<TraverseSearchReceipt>,
    /// Counts only discovered endpoints; never an estimate of unseen closure.
    pub bounds: ResultBounds,
    /// Edges examined but rejected by the predicate selector.
    pub predicate_filtered_edges: usize,
    /// Edges examined but rejected by the evidence requirement.
    pub evidence_filtered_edges: usize,
    /// Edges rejected because their scope differs from the witness build/snapshot scope.
    pub scope_filtered_edges: usize,
    /// Addresses reached at maximum depth with an unexamined relation frontier.
    pub depth_frontier_count: usize,
    /// Whether the result cap left an unexamined frontier.
    pub result_budget_exhausted: bool,
}
