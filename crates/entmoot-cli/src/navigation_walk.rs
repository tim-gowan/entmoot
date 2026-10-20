//! Shared CLI traversal controls; semantics and defaults belong to contracts.
use super::output::InformationArgs;
use anyhow::Result;
use clap::Args;
use entmoot_contracts::query::BoundedWalkBudget;

#[derive(Debug, Args)]
pub(super) struct WalkBudgetArgs {
    #[arg(long, default_value_t = BoundedWalkBudget::default().max_hops)]
    pub(crate) max_hops: usize,
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
}
#[derive(Debug, Args)]
pub(super) struct PathArgs {
    /// Exact origin returned by find/inventory.
    start: String,
    /// Exact destination returned by find/inventory.
    target: String,
    /// Relation allowlist (repeatable). All represented relations by default.
    #[arg(long = "predicate", default_value = "all")]
    predicates: Vec<String>,
    /// Per-hop evidence admission.
    #[arg(long, default_value = "any", value_parser = ["any", "resolved-only"])]
    evidence: String,
    #[command(flatten)]
    budget: WalkBudgetArgs,
    /// Snapshot whose callables are checked against unbound edges on this walk.
    #[arg(long)]
    against: Option<String>,
    #[command(flatten)]
    output: InformationArgs,
}
impl PathArgs {
    pub fn run(self) -> Result<()> {
        if self
            .predicates
            .iter()
            .any(|predicate| predicate != "calls" && predicate != "all")
        {
            return super::contracts::gap(entmoot_workspace::FACT_OTHER_EVIDENCE);
        }
        let _ = &self.evidence;
        super::contracts::emit(&entmoot_workspace::contract_path(
            &super::contracts::database()?,
            &self.start,
            &self.target,
            self.budget.max_hops,
            self.against.as_deref(),
        )?)
    }
}
