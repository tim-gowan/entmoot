//! Weakly connected call-relation components (review projection, not dead code).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::query::{QueryGapCode, ResultBounds, SchemaJson};
use crate::SourceSnapshot;

/// Wire schema id for [`CallComponentsReport`].
pub const CALL_COMPONENTS_REPORT_SCHEMA: &str = "entmoot.call_components.v1";

/// Default page size for call-component listing.
pub const DEFAULT_CALL_COMPONENTS_LIMIT: usize = 32;

/// Maximum page size for call-component listing.
pub const MAX_CALL_COMPONENTS_LIMIT: usize = 200;

/// Default sample member ids retained per component.
pub const DEFAULT_CALL_COMPONENT_SAMPLE_LIMIT: usize = 16;

/// How the component's display anchor was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallComponentAnchorKind {
    /// Unique callable in the component with directed `calls` in-degree zero.
    Root,
    /// Maximum incident degree (`in + out`); ties broken by node id.
    Hub,
}

/// One weakly connected component of observed `calls` among callables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallComponent {
    /// Stable callable node id used to name this component.
    pub anchor: String,
    /// Whether `anchor` is a local root or a degree hub.
    pub anchor_kind: CallComponentAnchorKind,
    /// Number of callable members in the component.
    pub member_count: u64,
    /// Stable-ordered sample of member node ids (may be truncated).
    pub sample_members: Vec<String>,
    /// Repository-relative path of the anchor when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_hint: Option<String>,
}

/// Read-only request to page call-relation components.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallComponentsRequest {
    /// Relation predicate; v1 accepts only `calls`.
    #[serde(default = "default_calls_predicate")]
    pub predicate: String,
    /// Zero-based offset into the component list (size-desc, then anchor id).
    #[serde(default)]
    pub offset: usize,
    /// Maximum components returned in this page.
    #[serde(default = "default_call_components_limit")]
    pub limit: usize,
    /// Maximum member ids retained per component sample.
    #[serde(default = "default_sample_limit")]
    pub sample_limit: usize,
    /// Maximum callable nodes admitted to the projection.
    #[serde(default = "default_component_work")]
    pub max_nodes: usize,
    /// Maximum relation records examined, including rejected predicates.
    #[serde(default = "default_component_work")]
    pub max_edges: usize,
}

fn default_component_work() -> usize {
    100_000
}

fn default_calls_predicate() -> String {
    "calls".into()
}

fn default_call_components_limit() -> usize {
    DEFAULT_CALL_COMPONENTS_LIMIT
}

fn default_sample_limit() -> usize {
    DEFAULT_CALL_COMPONENT_SAMPLE_LIMIT
}

impl Default for CallComponentsRequest {
    fn default() -> Self {
        Self {
            predicate: default_calls_predicate(),
            offset: 0,
            limit: DEFAULT_CALL_COMPONENTS_LIMIT,
            sample_limit: DEFAULT_CALL_COMPONENT_SAMPLE_LIMIT,
            max_nodes: default_component_work(),
            max_edges: default_component_work(),
        }
    }
}

/// Paginated call-relation components for one publication.
///
/// Components are a disposable review projection over observed `calls` edges.
/// They do not establish unused code, reachability completeness, or safe deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallComponentsReport {
    /// Version of this report envelope.
    pub schema_version: String,
    /// Exact inventory snapshot.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Applied request (after clamp).
    pub request: CallComponentsRequest,
    /// Ordered components for this page.
    pub components: Vec<CallComponent>,
    /// Pagination over the component list.
    pub bounds: ResultBounds,
    /// Explicit gaps / interpretation limits.
    pub gaps: Vec<QueryGapCode>,
    /// Human-readable limitations for consumers.
    pub limitations: Vec<String>,
}

impl CallComponentsReport {
    /// Construct a report with the current schema version.
    #[must_use]
    pub fn new(snapshot: SourceSnapshot, request: CallComponentsRequest) -> Self {
        Self {
            schema_version: CALL_COMPONENTS_REPORT_SCHEMA.into(),
            snapshot: SchemaJson::new(snapshot),
            request,
            components: Vec::new(),
            bounds: ResultBounds::complete(0),
            gaps: Vec::new(),
            limitations: vec![
                "Call components are weakly connected islands of observed calls among callables."
                    .into(),
                "Missing edges are not absence; anchors are not dead-code or importance ranking."
                    .into(),
            ],
        }
    }
}
