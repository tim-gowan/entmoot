//! Bounded composition of provider-stated roles and directed call traversal.
use crate::query::{BoundedWalkBudget, QueryResult, SchemaJson};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A bounded endpoint selector over original provider-role assertions.
///
/// ```
/// use entmoot_contracts::query::ProviderRoleSelector;
/// let selector: ProviderRoleSelector = serde_json::from_str(r#"{"role":"source"}"#)?;
/// assert!(!selector.include_hypotheses);
/// assert_eq!(selector.max_assertions, 20_000);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderRoleSelector {
    /// Exact open provider role token, e.g. source or sink. Never inferred from position.
    pub role: String,
    /// Admit possible/heuristically-derived roles with their original derivation.
    /// Unknown, contradicted or malformed assertions are never admitted.
    #[serde(default)]
    pub include_hypotheses: bool,
    /// Maximum matching assertions examined in stable assertion-id order (1..=100000).
    #[serde(default = "default_role_assertions")]
    pub max_assertions: usize,
}

fn default_role_assertions() -> usize {
    20_000
}

/// Endpoint selection and co-location receipt; this never asserts value flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleSelectionReceipt {
    /// Applied role admission policy and budget.
    pub selector: ProviderRoleSelector,
    /// Matching assertions before the admission budget and epistemic checks.
    pub matching_assertions: usize,
    /// Hypotheses excluded among examined assertions because opt-in was absent.
    pub excluded_hypotheses: usize,
    /// Examined assertions without an eligible unique current callable mapping.
    pub unmapped_assertion_ids: Vec<String>,
    /// More matching assertions exist than the admission budget allowed.
    pub truncated: bool,
    /// Anchors on the root itself; co-location does not establish ordering or flow.
    pub colocated: Vec<SeedAnchor>,
    /// Detail limit and total count for co-located anchors.
    pub colocated_bounds: crate::query::ResultBounds,
}

/// Select role assertions, map current attachments, and discover candidate routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeededPathRequest {
    /// Shared discovery exclusions, including seed origins and destinations.
    #[serde(default)]
    pub prune: super::TraversalPrune,
    /// Shared exact-scope per-hop admission in discovery and returned witnesses.
    #[serde(default)]
    pub evidence_filter: super::HopEvidenceFilter,
    /// Information delivery; search/selection budgets below retain their meaning.
    #[serde(default)]
    pub output: super::OutputOptions,
    /// Explicit provider role at route origins.
    pub from_role: String,
    /// Explicit provider role at route destinations.
    pub to_role: String,
    /// Explicit opt-in to possible/heuristically-derived role assertions.
    #[serde(default)]
    pub include_hypotheses: bool,
    /// Maximum matching assertion records admitted, in stable assertion-id order.
    pub max_seed_assertions: usize,
    /// Maximum distinct callable pairs to hydrate into path receipts.
    pub max_pairs: usize,
    /// Aggregate discovery-node expansion budget across all seed origins.
    pub max_discovery_expansions: usize,
    /// Aggregate discovery-edge examination budget across all seed origins.
    pub max_discovery_edges: usize,
    /// Per-pair meet-in-the-middle path budget, including maximum hop distance.
    pub walk: BoundedWalkBudget,
}
impl Default for SeededPathRequest {
    fn default() -> Self {
        Self {
            prune: Default::default(),
            evidence_filter: super::HopEvidenceFilter::Any,
            output: super::OutputOptions::default(),
            from_role: "source".into(),
            to_role: "sink".into(),
            include_hypotheses: false,
            max_seed_assertions: 20_000,
            max_pairs: 100,
            max_discovery_expansions: 100_000,
            max_discovery_edges: 1_000_000,
            walk: BoundedWalkBudget {
                max_hops: 12,
                max_edges_per_node: 0,
                max_results: 1,
                ..Default::default()
            },
        }
    }
}
/// Exact provider seed and its current callable attachment, without lifting roles
/// into callable-level security facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeedAnchor {
    /// Original provider assertion, preserving epistemic status, derivation and evidence.
    pub assertion: SchemaJson<crate::Assertion>,
    /// Exact current attachment evidence for this mapping.
    pub attachments: Vec<SchemaJson<crate::LocationAttachment>>,
    /// Immutable provider-role assertion to rehydrate.
    pub assertion_id: String,
    /// Exact original location subject.
    pub location_id: String,
    /// Provider-stated role.
    pub role: String,
    /// Unique enclosing callable reached through the selected attachment.
    pub callable_id: String,
    /// Current attachments justifying this mapping.
    pub attachment_ids: Vec<String>,
}
/// A directed pair of distinct callable endpoints with an ordinary query receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeededRoute {
    /// Origin callable; original seed assertions remain in the report's anchors.
    pub from_callable: String,
    /// Destination callable.
    pub to_callable: String,
    /// Entmoot's own directed traversal, including provenance, gaps and bounds.
    pub query: QueryResult,
}
/// Read-only seed-route report. No routes is never evidence of no dataflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeededPathReport {
    /// Pruning selector accounting over the full materialized inventory.
    /// Unknown candidates remain traversable; they are not proven nonmatches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_selection: Option<super::SelectionReceipt>,

    /// Discovery edge visits rejected by explicit pruning.
    #[serde(default)]
    pub pruned_edges: usize,
    /// Discovery edge visits rejected by per-hop evidence admission.
    #[serde(default)]
    pub evidence_filtered_edges: usize,

    /// Versioned attachment and discovery method.
    pub method_id: String,
    /// Exact inventory snapshot.
    pub snapshot: SchemaJson<crate::SourceSnapshot>,
    /// Applied selection and execution budgets.
    pub request: SeededPathRequest,
    /// Current, unambiguous callable anchors for exact role assertions.
    pub anchors: Vec<SeedAnchor>,
    /// Admitted role assertions lacking a unique current callable attachment.
    pub unmapped_assertion_ids: Vec<String>,
    /// Total matching role assertions before capping or attachment checks.
    pub matching_assertions: usize,
    /// Hypotheses excluded among examined assertions when opt-in was absent.
    pub excluded_hypotheses: usize,
    /// Whether matching assertions were omitted by the seed budget.
    pub seeds_truncated: bool,
    /// Callables containing both roles. This is co-location, never ordering or flow.
    pub colocated: Vec<String>,
    /// Directed candidate pairs evaluated by the path engine. Individual results
    /// may remain unknown when scopes or bounded search cannot establish a route.
    pub routes: Vec<SeededRoute>,
    /// Number of discovery nodes expanded.
    pub discovery_expansions: usize,
    /// Number of discovery edges examined.
    pub discovery_edges: usize,
    /// True when aggregate discovery or pair limits prevent finishing discovery.
    pub discovery_truncated: bool,
    /// Explicit interpretation constraints; do not infer taint or exploitability.
    pub limitations: Vec<String>,
}
