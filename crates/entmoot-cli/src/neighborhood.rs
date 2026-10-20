//! Task-shaped frontend for the existing read-only neighborhood contract.

use anyhow::Result;
use clap::{Args, ValueEnum};

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
    /// Returned callable names containing this literal; intermediates remain searchable.
    #[arg(long)]
    endpoint_name_contains: Option<String>,
    /// Returned callables with an explicit declared parameter type containing this literal.
    #[arg(long)]
    endpoint_parameter_type_contains: Option<String>,
    #[command(flatten)]
    budget: super::navigation_walk::WalkBudgetArgs,
    /// Snapshot whose callables are checked against unbound edges in this walk.
    #[arg(long)]
    against: Option<String>,
    #[command(flatten)]
    output: super::output::InformationArgs,
    /// Exact node ID, occurrence address or display reference (use request/find first).
    subject: String,
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
    #[arg(long, default_value_t = 20_000)]
    max_role_assertions: usize,
}

pub(super) fn run(args: NeighborhoodArgs) -> Result<()> {
    if args.role.is_some() {
        return super::contracts::gap(entmoot_workspace::FACT_PROVIDER_ROLES);
    }
    if args.endpoint_name_contains.is_some() || args.endpoint_parameter_type_contains.is_some() {
        return super::contracts::gap(entmoot_workspace::FACT_OTHER_EVIDENCE);
    }
    if args
        .predicates
        .iter()
        .any(|predicate| predicate != "calls" && predicate != "all")
    {
        return super::contracts::gap(entmoot_workspace::FACT_OTHER_EVIDENCE);
    }
    let _ = (
        args.include_hypotheses,
        args.include_candidates,
        args.max_role_assertions,
        &args.output,
    );
    let direction = match args.direction {
        Direction::Callers => "callers",
        Direction::Callees => "callees",
    };
    super::contracts::emit(&entmoot_workspace::contract_neighborhood(
        &super::contracts::database()?,
        &args.subject,
        direction,
        args.budget.max_hops,
        args.against.as_deref(),
    )?)
}
