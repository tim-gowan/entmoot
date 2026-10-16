use std::path::PathBuf;

mod execution;
mod http;

use anyhow::{Context, Result};
use clap::Parser;
use entmoot_contracts::query::{
    BoundedWalkBudget, CallDirection, FindQuery, HopEvidenceFilter, InspectQuery, QueryRequest,
    QueryService, QueryServiceError, ReadQuery, SeededPathRequest, TraverseQuery,
};
use entmoot_core::{ground_truth::policy_from_config_name, StructuralHashPolicy};
use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, schemars, tool, tool_router,
    ServiceExt,
};
use serde::Deserialize;

type Failure = (&'static str, String);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NeighborhoodArgs {
    #[serde(default)]
    resumable: bool,
    continuation: Option<String>,
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    start: String,
    direction: CallDirection,
    #[serde(default)]
    predicates: Vec<String>,
    evidence_filter: HopEvidenceFilter,
    #[serde(default)]
    endpoint_role: Option<entmoot_contracts::query::ProviderRoleSelector>,
    #[serde(default)]
    budget: BoundedWalkBudget,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImpactRelayArgs {
    /// Same portable request as CLI relay-impact; compact caller pages by default.
    #[serde(default)]
    request: entmoot_contracts::query::ImpactRelayRequest,
}

fn operation_result<T: serde::Serialize>(
    operation: &str,
    result: Result<T, Failure>,
) -> CallToolResult {
    match result.and_then(|value| {
        serde_json::to_value(value).map_err(|e| ("serialization_error", e.to_string()))
    }) {
        Ok(value) => CallToolResult::structured(value),
        Err((kind, message)) => CallToolResult::structured_error(serde_json::json!({
            "schema_version": format!("entmoot.{operation}.error.v1"),
            "status": "error", "kind": kind, "message": message,
        })),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "entmoot-mcp",
    version,
    about = "MCP transport for Entmoot evidence queries"
)]
struct Args {
    /// Print generated MCP tools/input schemas without opening a workspace.
    #[arg(long)]
    tools_schema: bool,
    /// Source workspace; inventory is prepared automatically on first use.
    #[arg(short = 'C', long)]
    directory: Option<PathBuf>,
    /// Local data root (default: ~/.entmoot). Never writes under the analyzed source.
    #[arg(long, env = "ENTMOOT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Advanced collaboration-store override; defaults to the workspace repository.
    #[arg(long)]
    collaboration_store: Option<PathBuf>,
    /// Explicit source worktree for local producer execution; never read from requests.
    #[arg(long)]
    collaboration_source_root: Option<PathBuf>,
    /// Serve Streamable HTTP on an explicit loopback address; omission keeps stdio.
    #[arg(long)]
    listen: Option<std::net::SocketAddr>,
    /// Read the local bearer credential here, or from ENTMOOT_QUERY_TOKEN.
    #[arg(long)]
    token_file: Option<PathBuf>,
    #[arg(long, default_value_t = 64)]
    continuation_capacity: usize,
    #[arg(long, default_value_t = 67_108_864)]
    continuation_bytes: usize,
    #[arg(long, default_value_t = 16_777_216)]
    continuation_search_bytes: usize,
    #[arg(long, default_value_t = 300)]
    continuation_idle_seconds: u64,
    /// Persist resumable query state across backend restarts within the configured idle lifetime.
    #[arg(long)]
    durable_continuations: bool,
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=2))]
    continuation_generations: u8,
    /// Disable chain contraction for comparison and diagnosis.
    #[arg(long)]
    no_contraction: bool,
    #[arg(long, default_value_t = 67_108_864)]
    shortcut_cache_bytes: usize,
    #[arg(long, default_value_t = 16)]
    shortcut_cache_entries: usize,
    #[arg(long, default_value_t = 1_000_000)]
    max_index_edges: usize,
    /// Maximum simultaneous blocking reads in this server process.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u16).range(1..=256))]
    read_workers: u16,
    /// Waiting reads beyond active workers; further requests fail as unavailable.
    #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u16).range(0..=4096))]
    read_queue: u16,
    /// Maximum simultaneous blocking operation requests in this server process.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u16).range(1..=64))]
    operation_workers: u16,
    /// Waiting operation requests; shared-store admission still applies across clients.
    #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u16).range(0..=4096))]
    operation_queue: u16,
    /// Provider graph(s), current.json pointers, or published checkouts for compatibility.
    /// When omitted, discover bounded local .entmoot/deps publications.
    #[arg(long)]
    provider: Vec<PathBuf>,
    /// Published logic.graph.v1 topology used for subject discovery.
    /// Mutually exclusive with `--publication`.
    #[arg(long, conflicts_with = "publication")]
    graph: Option<PathBuf>,
    /// Immutable publication pointer (`current.json`) written by entmoot-producer.
    #[arg(long)]
    publication: Option<PathBuf>,
    /// Committed SQLite knowledge store used for authoritative lookups.
    /// Defaults to `<graph>.knowledge.sqlite3`.
    #[arg(long)]
    store: Option<PathBuf>,
    /// Materialized source root for snapshot-verified navigation.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Optional compare graph for `changed_subject` status/find facets.
    #[arg(long)]
    compare_graph: Option<PathBuf>,
    /// Working-tree normalization policy. Presets or @path; must match the baseline.
    #[arg(long, default_value = "v1")]
    config: String,
    /// Seconds to wait for publication open on each tools/call (default 180).
    #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u64).range(1..=3600))]
    ready_timeout_seconds: u64,
    /// Fail boot when `.entmoot/adapter-profile.json` has hard gaps (opt-in).
    #[arg(long)]
    require_adapter_config: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindArgs {
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    /// subject | initializer | location | finding | evidence_role | changed_subject |
    /// unresolved_boundary | external_boundary | conflicted_relation |
    /// stale_evidence | producer_gap | high_fan_in_or_out
    kind: String,
    query: Option<String>,
    /// Exact plain decimal initializer spelling; only for kind=initializer.
    integer: Option<String>,
    path: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
    role: Option<String>,
    reason: Option<String>,
    compare_revision: Option<String>,
    compare_snapshot_digest: Option<String>,
    direction: Option<String>,
    min_degree: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BoundaryArgs {
    #[serde(default)]
    request: entmoot_contracts::query::CallBoundaryRequest,
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
}

impl TryFrom<FindArgs> for FindQuery {
    type Error = QueryServiceError;

    fn try_from(value: FindArgs) -> Result<Self, Self::Error> {
        match value.kind.as_str() {
            "initializer" => Ok(Self::Initializer {
                integer: value.integer.ok_or_else(|| QueryServiceError::InvalidRequest("find initializer requires integer".into()))?,
                path: value.path,
                limit: value.limit,
            }),
            "subject" => Ok(Self::Subject {
                query: value.query,
                path: value.path,
                line: value.line,
                limit: value.limit,
            }),
            "location" => Ok(Self::Location {
                path: value.path.ok_or_else(|| {
                    QueryServiceError::InvalidRequest("find location requires path".into())
                })?,
                line: value.line.ok_or_else(|| {
                    QueryServiceError::InvalidRequest("find location requires line".into())
                })?,
                column: value.column,
            }),
            "finding" => Ok(Self::Finding {
                query: value.query,
                path: value.path,
                limit: value.limit,
            }),
            "evidence_role" => Ok(Self::EvidenceRole {
                role: value.role,
                path: value.path,
                limit: value.limit,
            }),
            "changed_subject" => Ok(Self::ChangedSubject {
                compare_revision: value.compare_revision.ok_or_else(|| {
                    QueryServiceError::InvalidRequest(
                        "find changed_subject requires compare_revision".into(),
                    )
                })?,
                compare_snapshot_digest: value.compare_snapshot_digest,
                path: value.path,
                limit: value.limit,
            }),
            "unresolved_boundary" => Ok(Self::UnresolvedBoundary {
                reason: value.reason,
                path: value.path,
                limit: value.limit,
            }),
            "external_boundary" => Ok(Self::ExternalBoundary {
                path: value.path,
                limit: value.limit,
            }),
            "conflicted_relation" => Ok(Self::ConflictedRelation {
                path: value.path,
                limit: value.limit,
            }),
            "stale_evidence" => Ok(Self::StaleEvidence {
                path: value.path,
                limit: value.limit,
            }),
            "producer_gap" => Ok(Self::ProducerGap {
                path: value.path,
                limit: value.limit,
            }),
            "high_fan_in_or_out" => {
                let direction = match value.direction.as_deref().unwrap_or("either") {
                    "in" => entmoot_contracts::query::FanDirection::In,
                    "out" => entmoot_contracts::query::FanDirection::Out,
                    "either" => entmoot_contracts::query::FanDirection::Either,
                    _ => {
                        return Err(QueryServiceError::InvalidRequest(
                            "high_fan_in_or_out direction must be in, out, or either".into(),
                        ))
                    }
                };
                Ok(Self::HighFanInOrOut {
                    direction,
                    min_degree: value.min_degree,
                    path: value.path,
                    limit: value.limit,
                })
            }
            _ => Err(QueryServiceError::InvalidRequest(
                "find kind must be subject, location, finding, evidence_role, changed_subject, unresolved_boundary, external_boundary, conflicted_relation, stale_evidence, producer_gap, or high_fan_in_or_out".into(),
            )),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectArgs {
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    /// Continuation for kind=unresolved only.
    after: Option<entmoot_contracts::query::UnresolvedPageCursor>,
    /// subject | evidence | trace | proposition | relation_edge | unresolved
    kind: String,
    id: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

impl TryFrom<InspectArgs> for InspectQuery {
    type Error = QueryServiceError;

    fn try_from(value: InspectArgs) -> Result<Self, Self::Error> {
        if value.after.is_some() && value.kind != "unresolved" {
            return Err(QueryServiceError::InvalidRequest(
                "inspect after is only supported for kind=unresolved".into(),
            ));
        }
        match value.kind.as_str() {
            "unresolved" => Ok(Self::Unresolved {
                id: value.id,
                after: value.after,
                limit: value.limit,
            }),
            "subject" => Ok(Self::Subject {
                id: value.id,
                limit: value.limit,
            }),
            "evidence" => Ok(Self::Evidence { id: value.id }),
            "trace" => Ok(Self::Trace {
                id: value.id,
                offset: value.offset,
                limit: value.limit,
            }),
            "proposition" => Ok(Self::Proposition { id: value.id }),
            "relation_edge" => Ok(Self::RelationEdge { id: value.id }),
            _ => Err(QueryServiceError::InvalidRequest(
                "inspect kind must be subject, evidence, trace, proposition, relation_edge, or unresolved"
                    .into(),
            )),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TraverseArgs {
    #[serde(default)]
    resumable: bool,
    continuation: Option<String>,
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    /// Exact durable node id, address id, logic address, or display reference.
    start: String,
    /// Exact durable node id, address id, logic address, or display reference.
    target: String,
    /// Optional relation allowlist. Empty means every durable relation predicate.
    #[serde(default)]
    predicates: Vec<String>,
    /// Maximum path length (default 6, maximum 256).
    max_hops: Option<usize>,
    /// Maximum nodes expanded across both frontiers (default 1000, maximum 100000).
    max_expansions: Option<usize>,
    /// Global edge-work budget across both frontiers (default 100000).
    max_examined_edges: Option<usize>,
    /// Maximum edges considered per node (default 50; zero disables the local cap).
    max_edges_per_node: Option<usize>,
    /// Maximum path receipts (default 10, maximum 20).
    max_results: Option<usize>,
    /// Entries per hop in each evidence detail list (default 3, maximum 100;
    /// zero retains counts and classification). Inspect exact edges for details.
    max_evidence_per_hop: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadSourceArgs {
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    path: String,
    start_line: usize,
    end_line: Option<usize>,
    context_lines: Option<usize>,
    max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CallNeighborhoodArgs {
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    /// Exact durable node id, address id, logic address, or display reference.
    subject: String,
    /// Exclusive snapshot- and visibility-pinned cursor from the prior page.
    after: Option<entmoot_contracts::query::CallPageCursor>,
    /// Page size (default 50, maximum 1000).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct QueryPassthroughArgs {
    request: QueryRequest,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompatibilityArgs {
    /// Omit to evaluate all publications configured at server startup.
    request: Option<entmoot_contracts::query::CompatibilityRequest>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct InventoryOverlayArgs {
    /// Base publication graph path (or current.json pointer).
    base_graph: PathBuf,
    /// Head publication graph path (or current.json pointer).
    head_graph: PathBuf,
    /// Optional name substring filters.
    #[serde(default)]
    name_filters: Vec<String>,
    /// Optional path prefix filters.
    #[serde(default)]
    path_prefixes: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SeededPathArgs {
    /// Compact by default (10 items, 16 KiB); explain/export expand selected evidence.
    /// Output continuation is pinned; deferred detail is not search truncation.
    #[serde(default)]
    output: entmoot_contracts::query::OutputOptions,
    include_hypotheses: Option<bool>,
    from_role: Option<String>,
    to_role: Option<String>,
    max_seed_assertions: Option<usize>,
    max_pairs: Option<usize>,
    max_discovery_expansions: Option<usize>,
    max_discovery_edges: Option<usize>,
    walk: Option<BoundedWalkBudget>,
}

#[derive(Clone)]
struct EntmootMcp {
    collaboration_store: Option<PathBuf>,
    collaboration_source_root: Option<PathBuf>,
    operation_executor: execution::OperationExecutor,
    executor: execution::ReadExecutor,
    deferred: entmoot_engine::workspace::DeferredQueryBundle,
    working_tree_policy: StructuralHashPolicy,
    ready_timeout: std::time::Duration,
}

impl EntmootMcp {
    async fn collaboration_call(
        &self,
        request: entmoot_contracts::CollaborationWrite,
    ) -> CallToolResult {
        let Some(store) = self.collaboration_store.clone() else {
            return operation_result::<serde_json::Value>(
                "collaboration",
                Err((
                    "unavailable",
                    "configure --collaboration-store explicitly".into(),
                )),
            );
        };
        let source_root = self.collaboration_source_root.clone();
        match self
            .operation_executor
            .run(request, move |request| {
                entmoot_engine::collaboration::write_with_source(
                    &store,
                    request,
                    source_root.as_deref(),
                )
            })
            .await
        {
            Ok(value) => operation_result(
                "collaboration",
                value.map_err(|e| (e.code(), e.to_string())),
            ),
            Err(error) => Self::tool_result(Err(error)),
        }
    }
    async fn dispatch(
        &self,
        work: impl FnOnce(Self) -> CallToolResult + Send + 'static,
    ) -> CallToolResult {
        let server = self.clone();
        match self.executor.run(move || work(server)).await {
            Ok(result) => result,
            Err(error) => Self::tool_result(Err(error)),
        }
    }

    fn ready(
        &self,
    ) -> Result<std::sync::Arc<entmoot_engine::workspace::OpenedQueryBundle>, QueryServiceError>
    {
        self.deferred.wait(self.ready_timeout)
    }

    fn execute_query(&self, request: QueryRequest) -> CallToolResult {
        let token = request
            .output()
            .after
            .as_ref()
            .and_then(|c| c.search_slice.as_deref())
            .or_else(|| match &request {
                QueryRequest::Traverse { request, .. } => request.continuation(),
                _ => None,
            });
        Self::tool_result(
            self.deferred
                .wait_for_token(token, self.ready_timeout)
                .and_then(|bundle| bundle.engine.execute(request)),
        )
    }

    fn tool_result(
        result: Result<entmoot_contracts::query::QueryResponse, QueryServiceError>,
    ) -> CallToolResult {
        let result = result.and_then(|value| {
            value
                .into_value()
                .map_err(|error| QueryServiceError::Execution(error.to_string()))
        });
        Self::structured(result)
    }

    fn structured<T: serde::Serialize>(result: Result<T, QueryServiceError>) -> CallToolResult {
        match result.and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| QueryServiceError::Execution(error.to_string()))
        }) {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(serde_json::json!({
                "schema_version": "entmoot.query.error.v1",
                "status": "error",
                "kind": error.code(),
                "message": error.to_string(),
            })),
        }
    }

    fn warming_or<T: serde::Serialize>(
        &self,
        run: impl FnOnce(&entmoot_engine::workspace::OpenedQueryBundle) -> Result<T, QueryServiceError>,
    ) -> CallToolResult {
        match self.ready().and_then(|bundle| run(bundle.as_ref())) {
            Ok(value) => Self::structured(Ok(value)),
            Err(error) => Self::structured(Err::<T, _>(error)),
        }
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CollaborationReadParams {
    request: entmoot_contracts::CollaborationRead,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CollaborationWriteParams {
    request: entmoot_contracts::CollaborationWrite,
}

#[tool_router(server_handler)]
impl EntmootMcp {
    #[tool(
        description = "Read local workspace registrations, operation progress, or structural branch conflicts from explicitly retained views. No producers or publication mutations.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn collaboration_read(
        &self,
        Parameters(CollaborationReadParams { request }): Parameters<CollaborationReadParams>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            let result = server
                .collaboration_store
                .as_deref()
                .ok_or((
                    "unavailable",
                    "configure --collaboration-store explicitly".into(),
                ))
                .and_then(|store| {
                    entmoot_engine::collaboration::read_with_source(
                        store,
                        request,
                        server.collaboration_source_root.as_deref(),
                    )
                    .map_err(|e| (e.code(), e.to_string()))
                });
            operation_result("collaboration", result)
        })
        .await
    }

    #[tool(
        description = "Explicitly publish a retained workspace view, capture raw Git inputs, submit conflicts, ordered Git integrations, revalidation or producer work, control an operation, or run a bounded slice. Producers and Git execution require --collaboration-source-root. Native binding additionally requires a matching entmoot-producer executable and host-isolated environment token; it executes explicit native families with private scratch and durable checkpoints. Uses separate bounded execution capacity. Reads never start producers; source Git refs and index remain unchanged.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn collaboration_write(
        &self,
        Parameters(CollaborationWriteParams { request }): Parameters<CollaborationWriteParams>,
    ) -> CallToolResult {
        self.collaboration_call(request).await
    }
    #[tool(
        description = "Assess evidenced direct package API boundaries across independently published consumer and provider graphs. Omit request to use configured/discovered publications. Missing exports break only under explicit complete export coverage; absent binding evidence is incomplete_bind. Never produces, mutates, installs, or fetches dependencies.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn compatibility(
        &self,
        Parameters(args): Parameters<CompatibilityArgs>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| {
                let request = args
                    .request
                    .map(Ok)
                    .unwrap_or_else(|| bundle.engine.compatibility_request())?;
                bundle.engine.compatibility(request)
            })
        })
        .await
    }

    #[tool(
        description = "By default compare the current publication with a freshly indexed, digest-verified working tree. Explicit --publication or --graph/--compare-graph startup arguments compare immutable publications. Returns attributable impact and explicit unknowns; never executes a producer or moves the publication pointer.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn impact(&self) -> CallToolResult {
        self.dispatch(move |server| {
            let bundle = match server.ready() {
                Ok(bundle) => bundle,
                Err(error) => {
                    return operation_result::<entmoot_contracts::query::RevisionImpactManifest>(
                        "impact",
                        Err(("unavailable", error.to_string())),
                    );
                }
            };
            let working_tree_root = bundle
                .compare
                .is_none()
                .then(|| bundle.source_root.clone())
                .flatten();
            let result = bundle
                .engine
                .complete_graph()
                .map_err(|error| ("invalid_input", error.to_string()))
                .and_then(|graph| {
                    if let Some(root) = &working_tree_root {
                        entmoot_engine::workspace::compare_working_tree_impact(
                            &graph,
                            root,
                            &server.working_tree_policy,
                        )
                        .map_err(|error| ("invalid_input", error.to_string()))
                    } else {
                        let base = bundle.compare.as_deref().ok_or((
                            "invalid_input",
                            "operation requires --compare-graph at server startup".into(),
                        ))?;
                        entmoot_engine::compare_revision_impact(base, &graph)
                            .map_err(|error| ("invalid_input", error.to_string()))
                    }
                });
            operation_result("impact", result)
        })
        .await
    }

    #[tool(
        description = "Relay changed callables in the configured immutable base/head publication pair to bounded compact head caller pages. Requires an explicit compare publication. Default resolved_only excludes candidates; any retains candidate evidence. Each caller page has its own neighborhood query and cursor. Frontiers and empty pages never establish closed coverage, exploitability or a CVE fix. No producers or mutations. Use single-subject explain/export with a higher max_bytes when expanded evidence exceeds its byte limit.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn relay_impact(&self, Parameters(args): Parameters<ImpactRelayArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::relay_impact(&bundle.engine, args.request))
        })
        .await
    }

    #[tool(
        description = "Explore bounded incoming dependents or outgoing dependencies with exact per-hop evidence filters and explicit frontier gaps. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn neighborhood(&self, Parameters(args): Parameters<NeighborhoodArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            server.execute_query(QueryRequest::Traverse {
                request: TraverseQuery::Neighborhood {
                    resumable: args.resumable,
                    continuation: args.continuation,
                    start: args.start,
                    direction: args.direction,
                    predicates: args.predicates,
                    evidence_filter: args.evidence_filter,
                    endpoint_role: args.endpoint_role,
                    budget: args.budget,
                },
                output: args.output,
            })
        })
        .await
    }

    #[tool(
        description = "Pass one typed Entmoot QueryService request through unchanged. Use task-shaped tools when their narrower contract fits. Compact is the default; request.output.tier=explain/export expands evidence. Compact request.output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn query(&self, Parameters(args): Parameters<QueryPassthroughArgs>) -> CallToolResult {
        self.dispatch(move |server| server.execute_query(args.request))
            .await
    }

    #[tool(
        description = "Count and page distinct source call sites by callee pattern (e.g. *.toString()), native outcome, reason, path or build context. result_bounds.total_matches counts sites before paging. Explain/export preserves receiver syntax, source spans, native targets/types and artifact attribution. Syntax does not infer a receiver type; missing native evidence remains unreported. Repository, dependency and native-library completeness are independent. Never launches producers.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn boundaries(&self, Parameters(args): Parameters<BoundaryArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            server.execute_query(QueryRequest::Find {
                request: FindQuery::CallBoundary {
                    request: args.request,
                },
                output: args.output,
            })
        })
        .await
    }

    #[tool(
        description = "Find bounded inventory anchors by subject/name/path, exact initializer integer syntax, location, analyzer observation, or provider-stated semantic role. Initializer syntax is not a runtime value. Results preserve ambiguity and unknown coverage. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn find(&self, Parameters(args): Parameters<FindArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            let bundle = match server.ready() {
                Ok(bundle) => bundle,
                Err(error) => return Self::tool_result(Err(error)),
            };
            let output = args.output.clone();
            match args.try_into() {
                Ok(request) => Self::tool_result(QueryService::execute(
                    &bundle.engine,
                    QueryRequest::Find { request, output },
                )),
                Err(error) => Self::tool_result(Err(error)),
            }
        })
        .await
    }

    #[tool(
        description = "Inspect one exact subject, evidence id, trace, stable proposition, or relation-edge occurrence. Returns provenance, relation claims, uncertainty, and navigable identities without adjudicating truth for the client. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn inspect(&self, Parameters(args): Parameters<InspectArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            let bundle = match server.ready() {
                Ok(bundle) => bundle,
                Err(error) => return Self::tool_result(Err(error)),
            };
            let output = args.output.clone();
            match args.try_into() {
                Ok(request) => Self::tool_result(QueryService::execute(
                    &bundle.engine,
                    QueryRequest::Inspect { request, output },
                )),
                Err(error) => Self::tool_result(Err(error)),
            }
        })
        .await
    }

    #[tool(
        description = "Traverse typed durable relations between two exact subjects with explicit hop, expansion, fanout, and result budgets. Returns ordered edge references and evidence classifications and never treats backward search as reversed semantics. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn traverse(&self, Parameters(args): Parameters<TraverseArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            let defaults = BoundedWalkBudget::default();
            server.execute_query(QueryRequest::Traverse {
                request: TraverseQuery::Path {
                    resumable: args.resumable,
                    continuation: args.continuation,
                    start: args.start,
                    target: args.target,
                    predicates: args.predicates,
                    budget: BoundedWalkBudget {
                        max_hops: args.max_hops.unwrap_or(defaults.max_hops),
                        max_expansions: args.max_expansions.unwrap_or(defaults.max_expansions),
                        max_examined_edges: args
                            .max_examined_edges
                            .unwrap_or(defaults.max_examined_edges),
                        max_edges_per_node: args
                            .max_edges_per_node
                            .unwrap_or(defaults.max_edges_per_node),
                        max_results: args.max_results.unwrap_or(defaults.max_results),
                        max_evidence_per_hop: args.max_evidence_per_hop.unwrap_or(3),
                    },
                },
                output: args.output,
            })
        })
        .await
    }

    #[tool(
        description = "Read a bounded source excerpt from the configured SourceStore after verifying its digest against the indexed snapshot. Paths are repository-relative and traversal is rejected. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn read_source(&self, Parameters(args): Parameters<ReadSourceArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            let bundle = match server.ready() {
                Ok(bundle) => bundle,
                Err(error) => return Self::tool_result(Err(error)),
            };
            Self::tool_result(QueryService::execute(
                &bundle.engine,
                QueryRequest::Read {
                    request: ReadQuery::Source {
                        path: args.path,
                        start_line: args.start_line,
                        end_line: args.end_line,
                        context_lines: args.context_lines,
                        max_bytes: args.max_bytes,
                    },
                    output: args.output,
                },
            ))
        })
        .await
    }

    #[tool(
        description = "Return a keyset-paged list of materialized incoming call relations for one exact subject. Edges reference expandable producer claims and the result explicitly states that runtime/dispatch completeness is not established. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn callers(&self, Parameters(args): Parameters<CallNeighborhoodArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            let bundle = match server.ready() {
                Ok(bundle) => bundle,
                Err(error) => return Self::tool_result(Err(error)),
            };
            Self::tool_result(QueryService::execute(
                &bundle.engine,
                QueryRequest::Inspect {
                    request: InspectQuery::Callers {
                        id: args.subject,
                        after: args.after,
                        limit: args.limit,
                    },
                    output: args.output,
                },
            ))
        })
        .await
    }

    #[tool(
        description = "Return a keyset-paged list of materialized outgoing call relations and unresolved call attempts for one exact subject. Results preserve producer claims and never imply a closed runtime call graph. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn callees(&self, Parameters(args): Parameters<CallNeighborhoodArgs>) -> CallToolResult {
        self.dispatch(move |server| {
            let bundle = match server.ready() {
                Ok(bundle) => bundle,
                Err(error) => return Self::tool_result(Err(error)),
            };
            Self::tool_result(QueryService::execute(
                &bundle.engine,
                QueryRequest::Inspect {
                    request: InspectQuery::Callees {
                        id: args.subject,
                        after: args.after,
                        limit: args.limit,
                    },
                    output: args.output,
                },
            ))
        })
        .await
    }

    #[tool(
        description = "Report publication health and bounded high-signal find selectors for one exact graph/store/source publication. Suggested selectors are not an importance ranking and never invoke producers.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn status(
        &self,
        Parameters(request): Parameters<entmoot_contracts::query::StatusRequest>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::status(&bundle.engine, request))
        })
        .await
    }

    #[tool(
        description = "Page materialized inventory subjects for one publication (default kind: callable). Empty pages are not absence. Never invokes producers.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn inventory(
        &self,
        Parameters(request): Parameters<entmoot_contracts::query::InventoryRequest>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::inventory(&bundle.engine, request))
        })
        .await
    }

    #[tool(
        description = "Read clone candidates from a retained canon graph. If unavailable, run entmoot bind --experimental canon. Exact, syntax, alpha, parameterized and canonical matches do not prove behavioral equivalence. Never runs producers.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn assess_clones(
        &self,
        Parameters(request): Parameters<entmoot_contracts::CloneRequest>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::clones(&bundle.engine, request))
        })
        .await
    }

    #[tool(
        description = "Assess unused-code review candidates. Reports completeness separately; absent callers never establish dead code. Never runs producers or deletes source.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn assess_unused(
        &self,
        Parameters(request): Parameters<entmoot_contracts::UnusedRequest>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::unused(&bundle.engine, request))
        })
        .await
    }

    #[tool(
        description = "Assess test impact across the configured base/head pair. Explicit ownership is the minimum; graph evidence may add tests. Static paths are not assertion coverage. Never executes test commands.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn assess_test_impact(
        &self,
        Parameters(request): Parameters<entmoot_contracts::TestImpactRequest>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::test_impact(&bundle.engine, request))
        })
        .await
    }

    #[tool(
        description = "Inventory-only symbol presence overlay across two publications. Not Bound/Fits and not SemVer; missing evidence is not absence. Never runs producers.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn assess_overlay(
        &self,
        Parameters(args): Parameters<InventoryOverlayArgs>,
    ) -> CallToolResult {
        self.dispatch(move |_server| {
            operation_result(
                "inventory_overlay",
                (|| {
                    let (base_path, _, _) =
                        entmoot_engine::compatibility::publication_input(&args.base_graph)
                            .map_err(|e| ("invalid_request", e.to_string()))?;
                    let (head_path, _, _) =
                        entmoot_engine::compatibility::publication_input(&args.head_graph)
                            .map_err(|e| ("invalid_request", e.to_string()))?;
                    let base = entmoot_engine::load_graph_json(&base_path)
                        .map_err(|e| ("invalid_request", e.to_string()))?;
                    let head = entmoot_engine::load_graph_json(&head_path)
                        .map_err(|e| ("invalid_request", e.to_string()))?;
                    let request = entmoot_contracts::query::InventoryOverlayRequest {
                        base_publication: base_path.display().to_string(),
                        head_publication: head_path.display().to_string(),
                        name_filters: args.name_filters,
                        path_prefixes: args.path_prefixes,
                    };
                    entmoot_engine::compare_inventory_overlay(&base, &head, &request)
                        .map_err(|e| ("overlay_error", e.to_string()))
                })(),
            )
        })
        .await
    }

    #[tool(
        description = "Page weakly connected call-relation components among callables. Anchors are structural roots or degree hubs for review—not dead-code or importance ranking. Never invokes producers.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn call_components(
        &self,
        Parameters(request): Parameters<entmoot_contracts::query::CallComponentsRequest>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            server.warming_or(|bundle| QueryService::call_components(&bundle.engine, request))
        })
        .await
    }

    #[tool(
        description = "Discover bounded directed call routes between exact provider-stated role locations. Routes are candidates and never imply taint flow, execution order, sanitizer efficacy, or exploitability. Compact is the default; output.tier=explain/export expands evidence. Compact output.after resumes a pinned page; deferred evidence does not mean search truncation.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn trace_seeded_paths(
        &self,
        Parameters(args): Parameters<SeededPathArgs>,
    ) -> CallToolResult {
        self.dispatch(move |server| {
            let defaults = SeededPathRequest::default();
            let request = SeededPathRequest {
                output: args.output,
                include_hypotheses: args
                    .include_hypotheses
                    .unwrap_or(defaults.include_hypotheses),
                from_role: args.from_role.unwrap_or(defaults.from_role),
                to_role: args.to_role.unwrap_or(defaults.to_role),
                max_seed_assertions: args
                    .max_seed_assertions
                    .unwrap_or(defaults.max_seed_assertions),
                max_pairs: args.max_pairs.unwrap_or(defaults.max_pairs),
                max_discovery_expansions: args
                    .max_discovery_expansions
                    .unwrap_or(defaults.max_discovery_expansions),
                max_discovery_edges: args
                    .max_discovery_edges
                    .unwrap_or(defaults.max_discovery_edges),
                walk: args.walk.unwrap_or(defaults.walk),
            };
            server.warming_or(|bundle| QueryService::trace_seeds(&bundle.engine, request))
        })
        .await
    }
}

fn resolve_publication(
    path: &std::path::Path,
) -> Result<(PathBuf, Option<PathBuf>, Option<PathBuf>)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read publication pointer {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("parse publication pointer {}", path.display()))?;
    if value.get("schema_version").and_then(|v| v.as_str())
        == Some("entmoot.workspace_publication.v1")
    {
        let publication = entmoot_engine::workspace::read_publication(path)?;
        return Ok((
            publication.current.graph,
            Some(publication.current.store),
            Some(publication.current.source_root),
        ));
    }
    let parent = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let freshness = value
        .get("freshness_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("publication pointer missing freshness_key"))?;
    let generation = parent.join("publications").join(freshness);
    let graph_rel = value
        .get("graph")
        .and_then(|v| v.as_str())
        .unwrap_or("graph.json");
    let store_rel = value
        .get("store")
        .and_then(|v| v.as_str())
        .unwrap_or("graph.json.knowledge.sqlite3");
    let graph = generation.join(graph_rel);
    let store = generation.join(store_rel);
    let source_root = value
        .get("source_root")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    Ok((graph, Some(store), source_root))
}

fn load_normalization_policy(config: &str) -> Result<StructuralHashPolicy> {
    let policy = if let Some(path) = config.strip_prefix('@') {
        serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("read policy {path}"))?,
        )?
    } else {
        policy_from_config_name(config).map_err(anyhow::Error::msg)?
    };
    Ok(policy)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.tools_schema {
        println!(
            "{}",
            serde_json::to_string_pretty(&EntmootMcp::tool_router().list_all())?
        );
        return Ok(());
    }
    entmoot_engine::workspace::configure_data_directory(args.data_dir.as_deref())?;
    if let Some(directory) = &args.directory {
        std::env::set_current_dir(directory)?;
    }
    let explicit_directory = args
        .directory
        .as_ref()
        .map(|_| std::env::current_dir())
        .transpose()?;
    if args.graph.is_none() && args.publication.is_none() {
        match args.source_root.as_ref().or(explicit_directory.as_ref()) {
            Some(root) => {
                entmoot_engine::workspace::prepare_workspace(root)?;
            }
            None => {
                entmoot_engine::workspace::location::prepare_discovered_workspace(
                    &std::env::current_dir()?,
                )?;
            }
        }
    }
    let working_tree_policy = load_normalization_policy(&args.config)?;
    let discovery_start = args.source_root.clone().unwrap_or(std::env::current_dir()?);
    let pointer = if let Some(path) = &args.publication {
        Some(path.clone())
    } else if args.graph.is_none() {
        discovery_start
            .ancestors()
            .map(|p| entmoot_engine::workspace::storage_directory(p).join("current.json"))
            .find(|p| p.is_file())
    } else {
        None
    };
    let follow_compare = args.compare_graph.is_none();
    let query_options = entmoot_engine::query::QueryEngineOptions {
        durable_continuations: args.durable_continuations,
        continuation_capacity: args.continuation_capacity,
        continuation_bytes: args.continuation_bytes,
        continuation_search_bytes: args.continuation_search_bytes,
        continuation_idle_seconds: args.continuation_idle_seconds,
        contract_paths: !args.no_contraction,
        shortcut_cache_bytes: args.shortcut_cache_bytes,
        shortcut_cache_entries: args.shortcut_cache_entries,
        max_index_edges: args.max_index_edges,
        ..Default::default()
    };
    let (graph_path, publication_store, publication_root) =
        if let Some(publication) = &args.publication {
            resolve_publication(publication)?
        } else if let Some(graph) = &args.graph {
            (graph.clone(), None, None)
        } else {
            let publication = entmoot_engine::workspace::discover_publication(&discovery_start)?;
            (
                publication.current.graph,
                Some(publication.current.store),
                Some(publication.current.source_root),
            )
        };
    let store = args
        .store
        .or(publication_store)
        .unwrap_or_else(|| entmoot_engine::workspace::default_store_path(&graph_path));
    let require_source = args.source_root.is_some();
    let source_root = args.source_root.or(publication_root);
    let discovered_base = if args.graph.is_none() {
        let publication = if let Some(path) = &args.publication {
            entmoot_engine::workspace::read_publication(path).ok()
        } else {
            entmoot_engine::workspace::discover_publication(&discovery_start).ok()
        };
        publication.and_then(|p| p.base.map(|b| b.graph))
    } else {
        None
    };
    // Bare impact uses a working-tree compare when no immutable base is configured
    // (including `--publication` with `base: null`, matching Cursor mcp.json).
    let compare_path = args.compare_graph.or(discovered_base);
    let discovery_root = source_root.clone().or_else(|| std::env::current_dir().ok());
    let profile_root = source_root
        .clone()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| entmoot_engine::native_resolver::find_workspace_root(&cwd))
        })
        .or(discovery_root.clone());
    if let Some(root) = &profile_root {
        match entmoot_engine::native_resolver::load_profile(root) {
            Ok(Some(profile)) => {
                let gaps = profile.hard_gaps();
                if gaps.is_empty() {
                    eprintln!(
                        "entmoot-mcp: adapter profile ready ({} families)",
                        profile.families.len()
                    );
                } else {
                    eprintln!(
                        "entmoot-mcp: adapter profile hard gaps: {}; run entmoot init after building adapters, or omit --require-adapter-config",
                        gaps.join(", ")
                    );
                }
                if args.require_adapter_config && !gaps.is_empty() {
                    anyhow::bail!(
                        "adapter profile hard gaps under {}: {}",
                        root.display(),
                        gaps.join(", ")
                    );
                }
            }
            Ok(None) => {
                eprintln!(
                    "entmoot-mcp: no .entmoot/adapter-profile.json under {}; run entmoot init",
                    root.display()
                );
                if args.require_adapter_config {
                    anyhow::bail!(
                        "adapter profile missing under {} (--require-adapter-config)",
                        root.display()
                    );
                }
            }
            Err(error) => {
                eprintln!("entmoot-mcp: adapter profile read failed: {error}");
                if args.require_adapter_config {
                    anyhow::bail!("adapter profile invalid: {error}");
                }
            }
        }
    }
    let collaboration_store = match args.collaboration_store {
        Some(path) => Some(path),
        None if store.is_file() => Some(
            entmoot_engine::KnowledgeViewLocator::read(&store)?
                .map_or_else(|| store.clone(), |view| view.database),
        ),
        None => None,
    };
    let spec = entmoot_engine::workspace::PublicationOpenSpec {
        query_options,
        graph: graph_path,
        store,
        source_root,
        compare_graph: compare_path,
        require_source,
        providers: args.provider,
        discovery_root,
    };
    let deferred = match pointer {
        Some(pointer) => entmoot_engine::workspace::DeferredQueryBundle::start_monitored(
            spec,
            pointer,
            follow_compare,
            args.continuation_generations.into(),
        ),
        None => entmoot_engine::workspace::DeferredQueryBundle::start(spec),
    };
    let service = EntmootMcp {
        collaboration_store,
        collaboration_source_root: args.collaboration_source_root,
        operation_executor: execution::OperationExecutor::new(
            args.operation_workers.into(),
            args.operation_queue.into(),
        ),
        executor: execution::ReadExecutor::new(args.read_workers.into(), args.read_queue.into()),
        deferred,
        working_tree_policy,
        ready_timeout: std::time::Duration::from_secs(args.ready_timeout_seconds),
    };
    if let Some(address) = args.listen {
        return http::serve(service, address, args.token_file.as_deref()).await;
    }
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
