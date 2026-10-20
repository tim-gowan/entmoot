//! Operator verbs over the workspace contract snapshot.
use super::output::InformationArgs;
use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub(super) struct FindArgs {
    /// Symbol/name text; omit when using explicit selectors.
    query: Option<String>,
    /// Inventory kind (repeatable), e.g. callable or declaration.
    #[arg(long)]
    kind: Vec<String>,
    /// Published language name (repeatable).
    #[arg(long)]
    language: Vec<String>,
    /// Explicit whole-name equality; intersects the positional discovery text.
    #[arg(long, conflicts_with_all = ["name_distance", "name_contains"])]
    name_exact: Option<String>,
    /// Name within this Levenshtein distance of the positional query (0..=8).
    #[arg(long, requires = "query", conflicts_with = "name_contains")]
    name_distance: Option<usize>,
    /// Case-sensitive name substring; intersects positional discovery text.
    #[arg(long)]
    name_contains: Option<String>,
    /// Match an explicit declared parameter type substring.
    #[arg(long)]
    parameter_type_contains: Option<String>,
    /// Constrain the same parameter as --parameter-type-contains.
    #[arg(long)]
    parameter_name: Option<String>,
    /// Exact subtree, including descendants at slash boundaries.
    #[arg(long, conflicts_with = "path_glob")]
    path_prefix: Option<String>,
    /// Repository-relative glob: '*' stays within segments; '**' crosses them.
    #[arg(long)]
    path_glob: Option<String>,
    #[arg(long)]
    path: Option<String>,
    #[arg(long)]
    line: Option<usize>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    #[command(flatten)]
    output: InformationArgs,
}

impl FindArgs {
    pub(super) fn run(self) -> Result<()> {
        if self.parameter_name.is_some()
            || self.parameter_type_contains.is_some()
            || self.name_distance.is_some()
            || self.line.is_some()
            || self.path_glob.is_some()
            || !self.language.is_empty()
        {
            return super::contracts::gap(entmoot_workspace::FACT_OTHER_EVIDENCE);
        }
        if self.kind.iter().any(|kind| kind != "callable") {
            return super::contracts::gap(entmoot_workspace::FACT_NON_CALLABLE);
        }
        let query = self
            .name_exact
            .clone()
            .or(self.name_contains.clone())
            .or(self.query.clone());
        let path = self.path.clone().or(self.path_prefix.clone());
        super::contracts::emit(&entmoot_workspace::find_contracts(
            &super::contracts::database()?,
            query.as_deref(),
            path.as_deref(),
            self.limit,
        )?)
    }
}
#[derive(Debug, Args)]
pub(super) struct SubjectArgs {
    /// Exact ID or display reference returned by find/inventory.
    id: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    /// CallPageCursor JSON returned by callers/callees (use --detail explain).
    #[arg(long)]
    after: Option<PathBuf>,
    #[command(flatten)]
    output: InformationArgs,
}
#[derive(Debug, Args)]
pub(super) struct SourceArgs {
    /// Repository-relative file path.
    path: String,
    #[arg(long, default_value_t = 1)]
    start_line: usize,
    #[arg(long)]
    end_line: Option<usize>,
    #[arg(long, default_value_t = 3)]
    context_lines: usize,
    #[command(flatten)]
    output: InformationArgs,
}
#[derive(Debug, Args)]
pub(super) struct BoundaryArgs {
    /// Exact repository-relative file, mutually exclusive with legacy --path.
    #[arg(long, conflicts_with_all = ["path", "path_glob"])]
    path_exact: Option<String>,
    /// Repository-relative glob; '*' stays within segments and '**' crosses them.
    #[arg(long, conflicts_with = "path")]
    path_glob: Option<String>,
    /// Exact source callee or '*' pattern, e.g. '*.toString()'. Syntax is not a type claim.
    #[arg(long)]
    callee: Option<String>,
    #[arg(long)]
    path: Option<String>,
    /// Native outcome, e.g. unsupported, unresolved, external_symbol, or unreported.
    #[arg(long)]
    outcome: Option<String>,
    #[arg(long)]
    reason: Option<String>,
    #[arg(long)]
    build_context: Option<String>,
    #[arg(long, default_value_t = 0)]
    offset: usize,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    #[command(flatten)]
    output: InformationArgs,
}

impl SubjectArgs {
    pub(super) fn run_inspect(self) -> Result<()> {
        super::contracts::emit(&entmoot_workspace::inspect_contract(
            &super::contracts::database()?,
            &self.id,
        )?)
    }

    pub(super) fn run_callers(self) -> Result<()> {
        super::contracts::emit(&entmoot_workspace::contract_caller_set(
            &super::contracts::database()?,
            &self.id,
        )?)
    }

    pub(super) fn run_callees(self) -> Result<()> {
        super::contracts::emit(&entmoot_workspace::contract_callee_set(
            &super::contracts::database()?,
            &self.id,
        )?)
    }
}

impl BoundaryArgs {
    pub(super) fn run(self) -> Result<()> {
        if self.outcome.is_some() || self.reason.is_some() || self.build_context.is_some() {
            return super::contracts::gap(entmoot_workspace::FACT_NATIVE_DISPATCH);
        }
        if self.path_glob.is_some() {
            return super::contracts::gap(entmoot_workspace::FACT_OTHER_EVIDENCE);
        }
        let mut report = entmoot_workspace::contract_boundaries(
            &super::contracts::database()?,
            None,
            self.path_exact.as_deref().or(self.path.as_deref()),
        )?;
        if let Some(callee) = &self.callee {
            let needle = callee.trim_start_matches('*').trim_start_matches('.');
            report.unresolved.retain(|target| target.contains(needle));
        }
        super::contracts::emit(&report)
    }
}
