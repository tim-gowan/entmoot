//! Read-only publication status and high-signal discovery suggestions.
//!
//! Status is a separate composition from [`crate::query::QueryResult`]: it reports
//! inventory health and bounded executable follow-up selectors without ranking
//! importance or invoking producers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::query::{QueryRequest, SchemaJson};
use crate::{AdapterProfile, SourceSnapshot};

/// Wire schema id for [`StatusReport`].
pub const STATUS_REPORT_SCHEMA: &str = "entmoot.query.status.v1";

/// Default maximum sample ids retained per suggested selector.
pub const DEFAULT_STATUS_SAMPLE_LIMIT: usize = 10;

/// Default maximum suggested selectors returned by one status call.
pub const DEFAULT_STATUS_SUGGESTION_LIMIT: usize = 32;

/// Direction filter for high fan-in / fan-out discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FanDirection {
    /// Incoming relation degree.
    In,
    /// Outgoing relation degree.
    Out,
    /// Max of incoming and outgoing degree.
    Either,
}

/// Count with an explicit denominator or an unavailable reason.
///
/// Never treat a missing denominator as zero without recording why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CountedStat {
    /// Observed count when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// Population or partition size that `value` is drawn from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denominator: Option<u64>,
    /// Stable reason when the count cannot be established.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl CountedStat {
    /// Construct an available count with its denominator.
    #[must_use]
    pub fn available(value: u64, denominator: u64) -> Self {
        Self {
            value: Some(value),
            denominator: Some(denominator),
            unavailable_reason: None,
        }
    }

    /// Construct an unavailable count with an explicit reason.
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            denominator: None,
            unavailable_reason: Some(reason.into()),
        }
    }
}

/// Read-only request over one exact graph/store/source publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusRequest {
    /// Optional compare snapshot revision for `changed_subject` suggestions.
    ///
    /// When absent, the changed-subject facet is reported unavailable rather than
    /// empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_revision: Option<String>,
    /// Optional compare snapshot digest for `changed_subject` suggestions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_snapshot_digest: Option<String>,
    /// Maximum sample ids retained per suggested selector.
    #[serde(default = "default_sample_limit")]
    pub max_samples_per_suggestion: usize,
    /// Maximum suggested selectors returned.
    #[serde(default = "default_suggestion_limit")]
    pub max_suggestions: usize,
}

fn default_sample_limit() -> usize {
    DEFAULT_STATUS_SAMPLE_LIMIT
}

fn default_suggestion_limit() -> usize {
    DEFAULT_STATUS_SUGGESTION_LIMIT
}

impl Default for StatusRequest {
    fn default() -> Self {
        Self {
            compare_revision: None,
            compare_snapshot_digest: None,
            max_samples_per_suggestion: DEFAULT_STATUS_SAMPLE_LIMIT,
            max_suggestions: DEFAULT_STATUS_SUGGESTION_LIMIT,
        }
    }
}

/// Named aggregate partitions reported by status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedCount {
    /// Stable partition key (kind, producer, reason, …).
    pub name: String,
    /// Count within that partition.
    pub count: CountedStat,
}

/// Publication and dependency health for the statused snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicationHealth {
    /// Graph projection loaded and validated.
    pub graph: CountedStat,
    /// Durable knowledge store opened read-only.
    pub store: CountedStat,
    /// Source store configured for verified reads (`1`/`1` when present).
    pub source: CountedStat,
}

/// One bounded, executable follow-up derived from indexed categories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestedSelector {
    /// High-signal facet wire name (`finding`, `unresolved_boundary`, …).
    pub facet: String,
    /// Evidence or coverage reason this facet was surfaced.
    pub reason: String,
    /// Portable find (or other) request the consumer can execute next.
    pub request: QueryRequest,
    /// Stable ids sampled from the facet (subject, finding, edge, …).
    pub sample_ids: Vec<String>,
    /// Total matches establishing the sample (may exceed `sample_ids.len()`).
    pub match_count: CountedStat,
}

/// Read-only status report for one publication.
///
/// Suggested selectors are not an importance ranking and never authorize
/// producer execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusReport {
    /// Independent call-graph completeness scopes, separate from coverage counters.
    #[serde(default)]
    pub call_graph_completeness: super::CallGraphCompleteness,
    /// Version of this report envelope.
    pub schema_version: String,
    /// Exact inventory snapshot.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Identity policy id when present on the publication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    /// Applied status request.
    pub request: StatusRequest,
    /// Graph / store / source health.
    pub health: PublicationHealth,
    /// Counts by inventory entity kind.
    pub entity_kind_counts: Vec<NamedCount>,
    /// Counts by durable relation predicate.
    pub relation_kind_counts: Vec<NamedCount>,
    /// Attached producer partitions (name/version/scope/freshness summaries).
    pub producer_partitions: Vec<NamedCount>,
    /// Resolver-coverage partitions when indexed.
    pub resolver_coverage_partitions: Vec<NamedCount>,
    /// Durable structural scan counts; exclusions are not negative call evidence.
    pub scan_coverage_partitions: Vec<NamedCount>,
    /// Finding observation count.
    pub findings: CountedStat,
    /// Explicit provider-role assertion count.
    pub evidence_roles: CountedStat,
    /// Unresolved relation attempts by reason token.
    pub unresolved_reasons: Vec<NamedCount>,
    /// Relation edges with active refutation or explicit disagreement between
    /// active claims. This does not adjudicate which producer is correct.
    pub conflicted_relations: CountedStat,
    /// Stale evidence / claim count when indexed.
    pub stale_evidence: CountedStat,
    /// Queued producer work; unavailable when the publication has no queue index.
    pub queued_work: CountedStat,
    /// Active query budgets advertised by the serving implementation.
    pub active_budgets: Vec<NamedCount>,
    /// Bounded suggested follow-up selectors.
    pub suggested_selectors: Vec<SuggestedSelector>,
    /// Workspace adapter profile from `adapter-profile.json` in the selected
    /// storage volume when present.
    ///
    /// Absent when that volume has no profile yet; absence is not a negative
    /// conclusion about source languages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_profile: Option<AdapterProfile>,
    /// Explicit interpretation constraints for consumers.
    pub limitations: Vec<String>,
}

impl StatusReport {
    /// Construct a report with the current schema version.
    #[must_use]
    pub fn new(
        snapshot: SourceSnapshot,
        request: StatusRequest,
        health: PublicationHealth,
    ) -> Self {
        Self {
            call_graph_completeness: Default::default(),
            schema_version: STATUS_REPORT_SCHEMA.into(),
            snapshot: SchemaJson::new(snapshot),
            policy_id: None,
            request,
            health,
            entity_kind_counts: Vec::new(),
            relation_kind_counts: Vec::new(),
            producer_partitions: Vec::new(),
            resolver_coverage_partitions: Vec::new(),
            scan_coverage_partitions: Vec::new(),
            findings: CountedStat::unavailable("not_computed"),
            evidence_roles: CountedStat::unavailable("not_computed"),
            unresolved_reasons: Vec::new(),
            conflicted_relations: CountedStat::unavailable("not_computed"),
            stale_evidence: CountedStat::unavailable("not_computed"),
            queued_work: CountedStat::unavailable("queue_index_unavailable"),
            active_budgets: Vec::new(),
            suggested_selectors: Vec::new(),
            adapter_profile: None,
            limitations: vec![
                "status_is_not_an_importance_ranking".into(),
                "suggested_selectors_are_not_security_judgment".into(),
                "empty_or_unavailable_counts_are_not_negative_conclusions".into(),
            ],
        }
    }
}
