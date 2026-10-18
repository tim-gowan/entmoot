//! Shared CLI traversal controls; semantics and defaults belong to contracts.
use super::output::InformationArgs;
use anyhow::Result;
use clap::Args;
use entmoot_contracts::query::*;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub(super) struct WalkBudgetArgs {
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
}
impl WalkBudgetArgs {
    pub fn request(&self) -> BoundedWalkBudget {
        BoundedWalkBudget {
            max_hops: self.max_hops,
            max_expansions: self.max_expansions,
            max_examined_edges: self.max_examined_edges,
            max_edges_per_node: self.max_edges_per_node,
            max_results: self.max_results,
            max_evidence_per_hop: self.max_evidence_per_hop,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct PruneArgs {
    /// Exact subject reference to exclude during discovery (repeatable).
    #[arg(long)]
    exclude_subject: Vec<String>,
    /// Exact relation-edge ID to exclude during discovery (repeatable).
    #[arg(long)]
    exclude_edge: Vec<String>,
    /// Published occurrence key to exclude during discovery (repeatable).
    #[arg(long)]
    exclude_occurrence: Vec<String>,
}
impl PruneArgs {
    pub fn request(&self) -> TraversalPrune {
        TraversalPrune {
            subjects: self.exclude_subject.clone(),
            relation_edges: self.exclude_edge.clone(),
            occurrences: self.exclude_occurrence.clone(),
            matching_subjects: None,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct WalkExecutionArgs {
    /// Retain the bounded frontier so later requests can continue this search.
    #[arg(long)]
    pub resumable: bool,
    /// Save checkpoints for standalone invocations; backend durability is server configuration.
    #[arg(long, requires = "resumable")]
    durable: bool,
    #[arg(long, requires = "durable", value_parser = clap::value_parser!(u64).range(1..))]
    continuation_idle_seconds: Option<u64>,
    /// Search continuation token; distinct from --output-after.
    #[arg(long, requires = "resumable")]
    pub continuation: Option<String>,
    #[arg(long)]
    graph: Option<PathBuf>,
    #[arg(long, conflicts_with = "graph")]
    publication: Option<PathBuf>,
    #[arg(long)]
    store: Option<PathBuf>,
}
impl WalkExecutionArgs {
    pub fn validate_backend(&self) -> Result<()> {
        anyhow::ensure!(!self.durable,
            "backend durability is configured with --durable-continuations; use --standalone for local --durable checkpoints");
        anyhow::ensure!(self.graph.is_none() && self.publication.is_none() && self.store.is_none(),
            "backend reads use the server's publication; remove local graph/store options or use --standalone");
        Ok(())
    }
    pub fn run(self, request: QueryRequest) -> Result<()> {
        anyhow::ensure!(!self.resumable || self.durable,
            "standalone resumable searches require --durable so the next invocation can load its checkpoint");
        let (graph, store, source) =
            super::resolve_read_paths(self.graph, self.publication, self.store, None)?;
        let store =
            store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
        let opened = entmoot_workspace::workspace::open_query_bundle_paths(
            &graph,
            &store,
            source.as_deref(),
            None,
            false,
        )?;
        let engine = if self.durable {
            let engine = opened.engine.with_durable_continuations();
            match self.continuation_idle_seconds {
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
}

pub(super) fn predicates(values: &[String]) -> Result<Vec<String>> {
    if values.iter().any(|p| p == "all") {
        anyhow::ensure!(
            values.len() == 1,
            "'all' cannot be combined with other predicates"
        );
        Ok(vec![])
    } else {
        Ok(values.to_vec())
    }
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
    #[command(flatten)]
    prune: PruneArgs,
    #[command(flatten)]
    execution: WalkExecutionArgs,
    #[command(flatten)]
    output: InformationArgs,
}
impl PathArgs {
    fn request(&self) -> Result<QueryRequest> {
        let request = TraverseQuery::Path {
            start: self.start.clone(),
            target: self.target.clone(),
            predicates: predicates(&self.predicates)?,
            evidence_filter: if self.evidence == "resolved-only" {
                HopEvidenceFilter::ResolvedOnly
            } else {
                HopEvidenceFilter::Any
            },
            prune: self.prune.request(),
            budget: self.budget.request(),
            resumable: self.execution.resumable,
            continuation: self.execution.continuation.clone(),
        };
        let mut output = OutputOptions::default();
        self.output.apply(&mut output)?;
        Ok(QueryRequest::Traverse { request, output })
    }
    pub fn backend_request(&self) -> Result<QueryRequest> {
        self.execution.validate_backend()?;
        self.request()
    }
    pub fn run(self) -> Result<()> {
        let request = self.request()?;
        self.execution.run(request)
    }
}
