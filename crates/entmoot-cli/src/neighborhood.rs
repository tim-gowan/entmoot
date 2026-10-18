//! Task-shaped frontend for the existing read-only neighborhood contract.

use anyhow::Result;
use clap::{Args, ValueEnum};
use entmoot_contracts::query::{
    CallDirection, HopEvidenceFilter, ProviderRoleSelector, TraverseQuery,
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
    /// Returned callable names containing this literal; intermediates remain searchable.
    #[arg(long)]
    endpoint_name_contains: Option<String>,
    /// Returned callables with an explicit declared parameter type containing this literal.
    #[arg(long)]
    endpoint_parameter_type_contains: Option<String>,
    #[command(flatten)]
    prune: super::navigation_walk::PruneArgs,
    #[command(flatten)]
    budget: super::navigation_walk::WalkBudgetArgs,
    #[command(flatten)]
    execution: super::navigation_walk::WalkExecutionArgs,
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

fn query(args: &NeighborhoodArgs) -> Result<entmoot_contracts::query::QueryRequest> {
    use entmoot_contracts::query::{
        CallableSelector, InventoryKind, ParameterSelector, SubjectSelector, TextCase, TextMatch,
    };
    let contains = |value: &String| TextMatch::Contains {
        value: value.clone(),
        case: TextCase::Sensitive,
    };
    let endpoint_selector =
        (args.endpoint_name_contains.is_some() || args.endpoint_parameter_type_contains.is_some())
            .then(|| SubjectSelector {
                kinds: vec![InventoryKind::Callable],
                name: args.endpoint_name_contains.as_ref().map(contains),
                callable: args.endpoint_parameter_type_contains.as_ref().map(|value| {
                    CallableSelector {
                        parameters: vec![ParameterSelector {
                            declared_type_syntax: Some(contains(value)),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }
                }),
                ..Default::default()
            });
    let request = TraverseQuery::Neighborhood {
        endpoint_selector: endpoint_selector.map(Box::new),
        prune: args.prune.request(),
        resumable: args.execution.resumable,
        continuation: args.execution.continuation.clone(),
        start: args.subject.clone(),
        direction: match args.direction {
            Direction::Callers => CallDirection::Callers,
            Direction::Callees => CallDirection::Callees,
        },
        predicates: super::navigation_walk::predicates(&args.predicates)?,
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
        budget: args.budget.request(),
    };
    let mut output = entmoot_contracts::query::OutputOptions::default();
    args.output.apply(&mut output)?;
    Ok(entmoot_contracts::query::QueryRequest::Traverse { request, output })
}

impl NeighborhoodArgs {
    pub(super) fn backend_request(&self) -> Result<entmoot_contracts::query::QueryRequest> {
        self.execution.validate_backend()?;
        query(self)
    }
}

pub(super) fn run(args: NeighborhoodArgs) -> Result<()> {
    let request = query(&args)?;
    args.execution.run(request)
}
