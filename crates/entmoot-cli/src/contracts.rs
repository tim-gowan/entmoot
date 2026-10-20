//! Reads against the workspace contract snapshot. These commands do not open a
//! graph publication.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

pub(super) fn root() -> Result<PathBuf> {
    match super::SOURCE_DIRECTORY.get().cloned() {
        Some(root) => Ok(root),
        None => Ok(entmoot_workspace::workspace::workspace_root(
            &std::env::current_dir()?,
        )?),
    }
}

pub(super) fn database() -> Result<PathBuf> {
    Ok(entmoot_workspace::workspace::storage_directory(&root()?)?.join("contracts.sqlite3"))
}

pub(super) fn gap(fact: &str) -> Result<()> {
    bail!("contract snapshot does not store {fact}")
}

pub(super) fn emit(value: &impl serde::Serialize) -> Result<()> {
    crate::output::emit(value)?;
    Ok(())
}

pub(super) fn status() -> Result<()> {
    emit(&entmoot_workspace::contract_status(&database()?)?)
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum ContractOperation {
    Find {
        query: Option<String>,
        path: Option<String>,
        limit: Option<usize>,
        kind: Option<String>,
    },
    Callers {
        occurrence: String,
    },
    Callees {
        occurrence: String,
    },
    Inspect {
        occurrence: String,
    },
    Inventory {
        kind: Option<String>,
        path: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    Status,
    Boundaries {
        path: Option<String>,
    },
    Path {
        start: String,
        target: String,
        max_hops: Option<usize>,
        against: Option<String>,
    },
    Neighborhood {
        occurrence: String,
        direction: Option<String>,
        max_hops: Option<usize>,
        against: Option<String>,
    },
    CallComponents {
        offset: Option<usize>,
        limit: Option<usize>,
    },
    RelayImpact {
        base: Option<String>,
        head: Option<String>,
    },
    BindUnbound {
        snapshot: String,
        against: String,
    },
}

pub(super) fn run_request(
    request: Option<std::path::PathBuf>,
    example: Option<String>,
) -> Result<()> {
    if let Some(example) = example {
        let value = match example.as_str() {
            "path" => {
                serde_json::json!({"operation":"path","start":"<occurrence>","target":"<occurrence>","max_hops":6})
            }
            "neighborhood" => {
                serde_json::json!({"operation":"neighborhood","occurrence":"<occurrence>","direction":"callees","max_hops":1})
            }
            "boundaries" => serde_json::json!({"operation":"boundaries"}),
            "status" => serde_json::json!({"operation":"status"}),
            _ => serde_json::json!({"operation":"find","query":"main","limit":20}),
        };
        emit(&value)?;
        return Ok(());
    }
    let path = request.context("--request is required (or use --example find)")?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let operation: ContractOperation = match serde_json::from_value(value) {
        Ok(operation) => operation,
        Err(_) => return gap(entmoot_workspace::FACT_GRAPH_PUBLICATION),
    };
    let database = database()?;
    match operation {
        ContractOperation::Find {
            query,
            path,
            limit,
            kind,
        } => {
            if kind.as_deref().is_some_and(|kind| kind != "callable") {
                return gap(entmoot_workspace::FACT_NON_CALLABLE);
            }
            emit(&entmoot_workspace::find_contracts(
                &database,
                query.as_deref(),
                path.as_deref(),
                limit.unwrap_or(50),
            )?)
        }
        ContractOperation::Callers { occurrence } => emit(&entmoot_workspace::contract_caller_set(
            &database,
            &occurrence,
        )?),
        ContractOperation::Callees { occurrence } => emit(&entmoot_workspace::contract_callee_set(
            &database,
            &occurrence,
        )?),
        ContractOperation::Inspect { occurrence } => emit(&entmoot_workspace::inspect_contract(
            &database,
            &occurrence,
        )?),
        ContractOperation::Inventory {
            kind,
            path,
            offset,
            limit,
        } => {
            if kind.as_deref().is_some_and(|kind| kind != "callable") {
                return gap(entmoot_workspace::FACT_NON_CALLABLE);
            }
            emit(&entmoot_workspace::contract_inventory(
                &database,
                kind.as_deref(),
                path.as_deref(),
                offset.unwrap_or(0),
                limit.unwrap_or(50),
            )?)
        }
        ContractOperation::Status => status(),
        ContractOperation::Boundaries { path } => emit(&entmoot_workspace::contract_boundaries(
            &database,
            None,
            path.as_deref(),
        )?),
        ContractOperation::Path {
            start,
            target,
            max_hops,
            against,
        } => emit(&entmoot_workspace::contract_path(
            &database,
            &start,
            &target,
            max_hops.unwrap_or(6),
            against.as_deref(),
        )?),
        ContractOperation::Neighborhood {
            occurrence,
            direction,
            max_hops,
            against,
        } => emit(&entmoot_workspace::contract_neighborhood(
            &database,
            &occurrence,
            direction.as_deref().unwrap_or("callees"),
            max_hops.unwrap_or(1),
            against.as_deref(),
        )?),
        ContractOperation::CallComponents { offset, limit } => {
            emit(&entmoot_workspace::contract_components(
                &database,
                offset.unwrap_or(0),
                limit.unwrap_or(32),
            )?)
        }
        ContractOperation::RelayImpact { base, head } => emit(&entmoot_workspace::contract_relay(
            &database,
            base.as_deref(),
            head.as_deref(),
        )?),
        ContractOperation::BindUnbound { snapshot, against } => emit(
            &entmoot_workspace::bind_unbound_edges(&database, &snapshot, &against)?,
        ),
    }
}
