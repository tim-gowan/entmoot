//! Request DTOs for the four public query operations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::query::inspect::{CallPageCursor, UnresolvedPageCursor};
use crate::query::status::FanDirection;

/// Public wire contract for the general read-only query capability.
///
/// Frontends decode this contract and delegate to `QueryService::execute`.
/// Publication paths and transport settings belong to host configuration.
///
/// ```
/// use entmoot_contracts::query::QueryRequest;
/// let request: QueryRequest = serde_json::from_str(
///     r#"{"operation":"find","request":{"kind":"subject","query":"main","limit":5}}"#,
/// )?;
/// assert!(matches!(request, QueryRequest::Find { .. }));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueryRequest {
    /// Discover bounded anchors.
    Find {
        /// Typed anchor selector.
        request: FindQuery,
        /// Information delivery; omission selects compact navigation.
        #[serde(default)]
        output: super::OutputOptions,
    },
    /// Inspect an exact subject or evidence record.
    Inspect {
        /// Typed inspection selector.
        request: InspectQuery,
        /// Information delivery; omission selects compact navigation.
        #[serde(default)]
        output: super::OutputOptions,
    },
    /// Traverse the represented directed relations.
    Traverse {
        /// Typed path and budget specification.
        request: TraverseQuery,
        /// Information delivery; omission selects compact navigation.
        #[serde(default)]
        output: super::OutputOptions,
    },
    /// Read verified source from the configured source capability.
    Read {
        /// Typed source window specification.
        request: ReadQuery,
        /// Information delivery; omission selects compact navigation.
        #[serde(default)]
        output: super::OutputOptions,
    },
}

impl QueryRequest {
    /// Borrow delivery controls independently of operation/search budgets.
    #[must_use]
    pub fn output(&self) -> &super::OutputOptions {
        match self {
            Self::Find { output, .. }
            | Self::Inspect { output, .. }
            | Self::Traverse { output, .. }
            | Self::Read { output, .. } => output,
        }
    }
    /// Set delivery controls without changing the semantic operation.
    pub fn set_output(&mut self, options: super::OutputOptions) {
        match self {
            Self::Find { output, .. }
            | Self::Inspect { output, .. }
            | Self::Traverse { output, .. }
            | Self::Read { output, .. } => *output = options,
        }
    }
}

/// Expansion and result budgets for typed relation traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundedWalkBudget {
    /// Maximum directed hops in a returned path.
    pub max_hops: usize,
    /// Maximum nodes expanded during search.
    pub max_expansions: usize,
    /// Maximum total edges examined across both directed frontiers.
    #[serde(default = "default_walk_edges")]
    pub max_examined_edges: usize,
    /// Maximum predicate-matching edges per expanded node; zero removes this local cap.
    pub max_edges_per_node: usize,
    /// Maximum paths returned to the caller.
    pub max_results: usize,
    /// Maximum entries per hop in each assertion, unresolved-call, and claim-id
    /// detail list (0..=100). Counts, truncation, and classification remain intact.
    #[serde(default = "default_walk_evidence")]
    pub max_evidence_per_hop: usize,
}

impl Default for BoundedWalkBudget {
    fn default() -> Self {
        Self {
            max_hops: 6,
            max_expansions: 1_000,
            max_examined_edges: default_walk_edges(),
            max_edges_per_node: 50,
            max_results: 10,
            max_evidence_per_hop: default_walk_evidence(),
        }
    }
}

fn default_walk_edges() -> usize {
    100_000
}

fn default_walk_evidence() -> usize {
    20
}

/// Find an inventory or evidence anchor without inventing identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FindQuery {
    /// Exact observed plain signed-decimal initializer syntax. Equal spellings
    /// do not establish runtime values, clone equivalence or property reuse.
    Initializer {
        /// Canonical decimal integer spelling under declaration syntax policy v1.
        integer: String,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional bounded subject count; omissions remain explicit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Resolve text, path, and optional line to inventory subjects.
    Subject {
        /// Free-text or durable id selector: node id, `address_id`,
        /// `logic_address`, `display_ref`, label/FQN text, or an exact
        /// `structural_hash` (`blake3:<64-hex>` or bare 64-hex). Hash match is
        /// equality-only (not substring); multi-hits remain distinct occurrences.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional one-based line preference.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Resolve an exact source coordinate to the most specific site.
    Location {
        /// Repository-relative POSIX path.
        path: String,
        /// One-based line.
        line: usize,
        /// Optional one-based column.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column: Option<usize>,
    },
    /// Select findings by text and optional path.
    Finding {
        /// Optional free-text filter over finding fields.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Select provider-stated evidence roles only.
    EvidenceRole {
        /// Optional role token filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Subjects whose normalized bodies differ from an explicit compare revision.
    ChangedSubject {
        /// Compare snapshot revision (required).
        compare_revision: String,
        /// Optional compare snapshot digest when the publication records digests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compare_snapshot_digest: Option<String>,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Open unresolved relation attempts (boundaries remaining unknown).
    CallBoundary {
        /// Source spelling, outcome and build-context selectors.
        request: super::CallBoundaryRequest,
    },
    /// Open unresolved relation attempts (boundaries remaining unknown).
    UnresolvedBoundary {
        /// Optional reason-token filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Subjects participating in external or dependency-boundary relations.
    ExternalBoundary {
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Relation edges with active conflicting claims.
    ConflictedRelation {
        /// Optional repository-relative POSIX path filter over endpoints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Evidence or claims marked stale for the current snapshot.
    StaleEvidence {
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Subjects lacking expected resolver coverage for their language/scope.
    ProducerGap {
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Subjects whose relation degree meets a minimum fan-in/out threshold.
    HighFanInOrOut {
        /// Degree direction to score.
        direction: FanDirection,
        /// Inclusive minimum degree (defaults to implementation threshold).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_degree: Option<usize>,
        /// Optional repository-relative POSIX path filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Optional result page limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
}

/// Inspect an exact subject, evidence record, trace, proposition, or relation edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InspectQuery {
    /// Assemble compact context around one inventory subject.
    Subject {
        /// Subject node id, address id, logic address, or display ref.
        id: String,
        /// Optional fan-out limit for neighbors and related collections.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Explain a finding, assertion, proposition, or relation-edge id.
    Evidence {
        /// Evidence or knowledge record id.
        id: String,
    },
    /// Page ordered trace steps for a finding or trace id.
    Trace {
        /// Finding observation id or trace id.
        id: String,
        /// Zero-based step offset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<usize>,
        /// Optional page size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Fetch a proposition and its scoped occurrences/claims.
    Proposition {
        /// Proposition id.
        id: String,
    },
    /// Fetch relation claims for one relation-edge id.
    RelationEdge {
        /// Relation-edge id.
        id: String,
    },
    /// Read one unresolved relation by its id, or page attempts for one exact subject.
    /// Exhaustion is not proof of complete resolution or a negative relationship.
    Unresolved {
        /// Unresolved relation id or exact inventory subject selector.
        id: String,
        /// Exclusive collection-pinned cursor from the prior page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<UnresolvedPageCursor>,
        /// Number of records per page (1..=1000; default 50).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Page materialized incoming call relations for one exact subject.
    Callers {
        /// Exact subject selector.
        id: String,
        /// Exclusive cursor returned by the prior page. It pins snapshot and
        /// durable visibility state as well as the relation key.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<CallPageCursor>,
        /// Page size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Page materialized outgoing call relations and open call attempts.
    Callees {
        /// Exact subject selector.
        id: String,
        /// Exclusive cursor returned by the prior page. It pins snapshot and
        /// durable visibility state as well as the relation key.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<CallPageCursor>,
        /// Page size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
}

/// Traverse typed relations between exact inventory subjects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TraverseQuery {
    /// Enumerate scoped, bounded dependents or dependencies from one exact root.
    /// Every hop passes the filter. Context changes require an explicit resource
    /// observed-as bridge; witnesses never cross snapshots or implicit build contexts.
    Neighborhood {
        /// Exact inventory selector; ambiguous selectors return an explicit gap.
        start: String,
        /// Incoming dependents or outgoing dependencies.
        direction: crate::query::CallDirection,
        /// Allowed relation predicate wire names; empty allows all predicates.
        predicates: Vec<String>,
        /// Evidence requirements applied independently to every hop.
        evidence_filter: HopEvidenceFilter,
        /// Optional provider-role filter on endpoints only; intermediates remain searchable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint_role: Option<crate::query::ProviderRoleSelector>,
        /// Expansion, examination, depth, result and evidence-detail limits.
        budget: BoundedWalkBudget,
        /// Retain bounded search state so later executions can continue discovery.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resumable: bool,
        /// Opaque in-memory search token, distinct from an output page cursor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<String>,
    },
    /// Find bounded directed paths between two subjects.
    Path {
        /// Start subject selector.
        start: String,
        /// Target subject selector.
        target: String,
        /// Allowed relation predicate wire names.
        predicates: Vec<String>,
        /// Expansion and result budgets.
        budget: BoundedWalkBudget,
        /// Retain bounded search state so later executions can continue discovery.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resumable: bool,
        /// Opaque in-memory search token, distinct from an output page cursor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<String>,
    },
}

impl TraverseQuery {
    /// Applied execution slice and fixed depth/fanout controls.
    pub fn budget(&self) -> BoundedWalkBudget {
        match self {
            Self::Path { budget, .. } | Self::Neighborhood { budget, .. } => *budget,
        }
    }
    /// Whether the request opts into retained search state.
    pub fn resumable(&self) -> bool {
        match self {
            Self::Path { resumable, .. } | Self::Neighborhood { resumable, .. } => *resumable,
        }
    }
    /// Opaque token for the next execution or a replay of an earlier slice.
    pub fn continuation(&self) -> Option<&str> {
        match self {
            Self::Path { continuation, .. } | Self::Neighborhood { continuation, .. } => {
                continuation.as_deref()
            }
        }
    }
    /// Set a search token without changing the semantic selector.
    pub fn set_continuation(&mut self, token: Option<String>) {
        match self {
            Self::Path { continuation, .. } | Self::Neighborhood { continuation, .. } => {
                *continuation = token
            }
        }
    }
}

/// Evidence filter for each relation occurrence in a neighborhood witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HopEvidenceFilter {
    /// Retain every represented edge, including unclaimed and refuted edges.
    Any,
    /// Require at least one exact-scope active resolution claim, with every
    /// applicable state artifact-resolved, semantic-resolved or validated.
    /// Conflicting, stale, candidate and unclaimed occurrences cannot pass.
    ResolvedOnly,
}

/// Read digest-verified source through the authoritative source store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadQuery {
    /// Read a bounded line window from one snapshot path.
    Source {
        /// Repository-relative POSIX path.
        path: String,
        /// One-based start line.
        start_line: usize,
        /// Optional one-based end line (defaults to start).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_line: Option<usize>,
        /// Optional context lines around the window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_lines: Option<usize>,
        /// Optional maximum returned bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<usize>,
    },
}
