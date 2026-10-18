//! Thin operator verbs over the transport-neutral query service.
use super::{output::InformationArgs, Commands};
use anyhow::Result;
use clap::Args;
use entmoot_contracts::query::*;
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
    fn selector(&self) -> Result<Option<SubjectSelector>> {
        let literal = |value: &str| TextMatch::Contains {
            value: value.into(),
            case: TextCase::Sensitive,
        };
        let kinds = self
            .kind
            .iter()
            .map(|kind| {
                serde_json::from_value::<InventoryKind>(serde_json::Value::String(kind.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let name = if let Some(max_distance) = self.name_distance {
            self.query.as_ref().map(|value| TextMatch::Levenshtein {
                value: value.clone(),
                max_distance,
                case: TextCase::Sensitive,
            })
        } else if let Some(value) = &self.name_exact {
            Some(TextMatch::Exact {
                value: value.clone(),
                case: TextCase::Sensitive,
            })
        } else {
            self.name_contains.as_deref().map(literal)
        };
        let callable = (self.parameter_name.is_some() || self.parameter_type_contains.is_some())
            .then(|| CallableSelector {
                parameters: vec![ParameterSelector {
                    name: self.parameter_name.as_ref().map(|value| TextMatch::Exact {
                        value: value.clone(),
                        case: TextCase::Sensitive,
                    }),
                    declared_type_syntax: self.parameter_type_contains.as_deref().map(literal),
                    ..Default::default()
                }],
                ..Default::default()
            });
        let selector = SubjectSelector {
            kinds,
            languages: self.language.clone(),
            name,
            callable,
            path: self
                .path_prefix
                .clone()
                .map(PathSelector::Subtree)
                .or_else(|| self.path_glob.clone().map(PathSelector::Glob)),
            ..Default::default()
        };
        Ok((selector != SubjectSelector::default()).then_some(selector))
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

pub(super) fn request(command: &Commands) -> Result<Option<QueryRequest>> {
    let (mut request, information) = match command {
        Commands::Find(args) => (
            QueryRequest::Find {
                request: FindQuery::Subject {
                    selector: args.selector()?,
                    query: if args.name_distance.is_some() {
                        None
                    } else {
                        args.query.clone()
                    },
                    path: args.path.clone(),
                    line: args.line,
                    limit: Some(args.limit),
                },
                output: Default::default(),
            },
            &args.output,
        ),
        Commands::Inspect(args) => (
            QueryRequest::Inspect {
                request: InspectQuery::Subject {
                    id: args.id.clone(),
                    limit: Some(args.limit),
                },
                output: Default::default(),
            },
            &args.output,
        ),
        Commands::Callers(args) | Commands::Callees(args) => {
            let after = args
                .after
                .as_ref()
                .map(|p| -> Result<CallPageCursor> {
                    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
                })
                .transpose()?;
            let request = if matches!(command, Commands::Callers(_)) {
                InspectQuery::Callers {
                    id: args.id.clone(),
                    after,
                    limit: Some(args.limit),
                }
            } else {
                InspectQuery::Callees {
                    id: args.id.clone(),
                    after,
                    limit: Some(args.limit),
                }
            };
            (
                QueryRequest::Inspect {
                    request,
                    output: Default::default(),
                },
                &args.output,
            )
        }
        Commands::ReadSource(args) => (
            QueryRequest::Read {
                request: ReadQuery::Source {
                    path: args.path.clone(),
                    start_line: args.start_line,
                    end_line: args.end_line,
                    context_lines: Some(args.context_lines),
                    max_bytes: Some(16_384),
                },
                output: Default::default(),
            },
            &args.output,
        ),
        Commands::Boundaries(args) => (
            QueryRequest::Find {
                request: FindQuery::CallBoundary {
                    request: CallBoundaryRequest {
                        path_selector: args
                            .path_exact
                            .clone()
                            .map(PathSelector::Exact)
                            .or_else(|| args.path_glob.clone().map(PathSelector::Glob)),
                        callee: args.callee.clone(),
                        path: args.path.clone(),
                        outcome: args.outcome.clone(),
                        reason: args.reason.clone(),
                        build_context: args
                            .build_context
                            .as_ref()
                            .map(entmoot_contracts::ContentDigest::parse)
                            .transpose()?,
                        offset: args.offset,
                        limit: Some(args.limit),
                    },
                },
                output: Default::default(),
            },
            &args.output,
        ),
        _ => return Ok(None),
    };
    let mut output = request.output().clone();
    information.apply(&mut output)?;
    request.set_output(output);
    Ok(Some(request))
}

pub(super) fn run(request: QueryRequest) -> Result<()> {
    let (graph, store, source) = super::resolve_read_paths(None, None, None, None)?;
    let store = store.unwrap_or_else(|| entmoot_engine::workspace::default_store_path(&graph));
    let bundle = entmoot_engine::workspace::open_query_bundle_paths(
        &graph,
        &store,
        source.as_deref(),
        None,
        true,
    )?;
    let result =
        QueryService::execute(&bundle.engine, request).map_err(super::report_query_error)?;
    crate::output::emit(&result)?;
    Ok(())
}
