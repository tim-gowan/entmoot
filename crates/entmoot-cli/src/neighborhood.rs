//! Task-shaped frontend for the existing read-only neighborhood contract.
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};
use entmoot_contracts::query::{
    BoundedWalkBudget, CallDirection, HopEvidenceFilter, ProviderRoleSelector, QueryService,
    TraverseQuery,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Direction {
    /// Follow incoming edges to dependents or source-labeled origins.
    Callers,
    /// Follow outgoing edges to dependencies or sink-labeled destinations.
    Callees,
}

/// Neighborhood witnesses from one exact inventory subject. A call route is not dataflow.
#[derive(Debug, Args)]
pub(super) struct NeighborhoodArgs {
    #[command(flatten)]
    output: super::output::InformationArgs,
    /// Exact node ID, occurrence address or display reference (use request/find first).
    subject: String,
    /// Retain the bounded frontier so later requests can continue this search.
    #[arg(long)]
    resumable: bool,
    /// Save frontier and replay state so standalone invocations can resume after restart.
    #[arg(long, requires = "resumable")]
    durable: bool,
    /// Retain durable checkpoints this long between slices (standalone only).
    #[arg(long, requires = "durable", value_parser = clap::value_parser!(u64).range(1..))]
    continuation_idle_seconds: Option<u64>,
    /// Opaque next-slice token from the same backend generation.
    #[arg(long, requires = "resumable")]
    continuation: Option<String>,
    #[arg(long, value_enum, default_value = "callers")]
    direction: Direction,
    /// Optional terminal role. Unlabeled intermediate nodes remain searchable.
    #[arg(long)]
    role: Option<String>,
    /// Include possible/heuristically-derived role assertions with their original status.
    #[arg(long, requires = "role")]
    include_hypotheses: bool,
    /// Include all represented edges, including unclaimed, stale and refuted evidence.
    #[arg(long)]
    include_candidates: bool,
    /// Relation wire name; repeat to allow additional predicates.
    #[arg(long = "predicate", default_value = "calls")]
    predicates: Vec<String>,
    #[arg(long)]
    graph: Option<PathBuf>,
    #[arg(long, conflicts_with = "graph")]
    publication: Option<PathBuf>,
    #[arg(long)]
    store: Option<PathBuf>,
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_hops)]
    max_hops: usize,
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_expansions)]
    max_expansions: usize,
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_examined_edges)]
    max_examined_edges: usize,
    /// Zero removes the local fanout cap; global work limits still apply.
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_edges_per_node)]
    max_edges_per_node: usize,
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_results)]
    max_results: usize,
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_evidence_per_hop)]
    max_evidence_per_hop: usize,
    #[arg(long, default_value_t = 20_000)]
    max_role_assertions: usize,
}

fn query(args: &NeighborhoodArgs) -> Result<entmoot_contracts::query::QueryRequest> {
    let request = TraverseQuery::Neighborhood {
        resumable: args.resumable,
        continuation: args.continuation.clone(),
        start: args.subject.clone(),
        direction: match args.direction {
            Direction::Callers => CallDirection::Callers,
            Direction::Callees => CallDirection::Callees,
        },
        predicates: args.predicates.clone(),
        evidence_filter: if args.include_candidates {
            HopEvidenceFilter::Any
        } else {
            HopEvidenceFilter::ResolvedOnly
        },
        endpoint_role: args.role.clone().map(|role| ProviderRoleSelector {
            role,
            include_hypotheses: args.include_hypotheses,
            max_assertions: args.max_role_assertions,
        }),
        budget: BoundedWalkBudget {
            max_hops: args.max_hops,
            max_expansions: args.max_expansions,
            max_examined_edges: args.max_examined_edges,
            max_edges_per_node: args.max_edges_per_node,
            max_results: args.max_results,
            max_evidence_per_hop: args.max_evidence_per_hop,
        },
    };
    let mut output = entmoot_contracts::query::OutputOptions::default();
    args.output.apply(&mut output)?;
    Ok(entmoot_contracts::query::QueryRequest::Traverse { request, output })
}

impl NeighborhoodArgs {
    pub(super) fn backend_request(&self) -> Result<entmoot_contracts::query::QueryRequest> {
        if self.durable {
            anyhow::bail!("backend durability is configured when starting the server with --durable-continuations; use --standalone for local --durable checkpoints");
        }
        if self.graph.is_some() || self.publication.is_some() || self.store.is_some() {
            anyhow::bail!("backend neighborhood uses the server's publication; remove local graph/store options or use --standalone");
        }
        query(self)
    }
}

pub(super) fn run(args: NeighborhoodArgs) -> Result<()> {
    let request = query(&args)?;
    let (graph, store, _) =
        super::resolve_read_paths(args.graph, args.publication, args.store, None)?;
    let store = store.unwrap_or_else(|| entmoot_engine::workspace::default_store_path(&graph));
    let opened =
        entmoot_engine::workspace::open_query_bundle_paths(&graph, &store, None, None, false)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let engine = if args.durable {
        let engine = opened.engine.with_durable_continuations();
        match args.continuation_idle_seconds {
            Some(seconds) => engine.with_durable_continuation_lifetime(seconds),
            None => engine,
        }
    } else {
        opened.engine
    };
    let result = QueryService::execute(&engine, request).map_err(super::report_query_error)?;
    super::output::emit(&result)?;
    Ok(())
}
