//! Release or default-branch snapshot of callable caller/callee sets.
//!
//! A snapshot is not an inventory view. Identity is the span-free occurrence
//! key (`lang|path|scope|symbol|kind`). `structural_hash` and `signature_hash`
//! record whether the definition changed; the subscriber event is the diff of
//! caller, callee, and unresolved sets. A comment that leaves those sets
//! unchanged emits nothing.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::{EdgeKind, NodeKind, TopologyBuildOptions, TopologyGraph};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS contract_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    revision TEXT NOT NULL,
    tree_digest TEXT NOT NULL,
    callable_count INTEGER NOT NULL,
    callee_count INTEGER NOT NULL,
    unresolved_count INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS contract_callables (
    snapshot_id TEXT NOT NULL REFERENCES contract_snapshots(snapshot_id),
    occurrence_key TEXT NOT NULL,
    address_id TEXT NOT NULL,
    logic_address TEXT NOT NULL,
    structural_hash TEXT,
    signature_hash TEXT,
    path TEXT NOT NULL,
    symbol TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, occurrence_key)
);
CREATE TABLE IF NOT EXISTS contract_callees (
    snapshot_id TEXT NOT NULL,
    caller_key TEXT NOT NULL,
    callee_key TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, caller_key, callee_key)
);
CREATE TABLE IF NOT EXISTS contract_unresolved (
    snapshot_id TEXT NOT NULL,
    caller_key TEXT NOT NULL,
    target TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, caller_key, target)
);
"#;

#[derive(Debug, thiserror::Error)]
pub enum ContractSnapshotError {
    #[error("{0}")]
    Message(String),
    #[error("contract snapshot does not store {0}")]
    MissingFact(&'static str),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Facts a contract snapshot never records. Callers surface these instead of
/// opening an inventory publication.
pub const FACT_SOURCE_TEXT: &str = "source text";
pub const FACT_NATIVE_DISPATCH: &str = "native dispatch";
pub const FACT_PROVIDER_ROLES: &str = "provider-role seeds";
pub const FACT_GRAPH_PUBLICATION: &str = "a graph publication";
pub const FACT_OTHER_EVIDENCE: &str = "evidence other than caller, callee, and unresolved sets";
pub const FACT_NON_CALLABLE: &str = "non-callable inventory";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallableContract {
    pub occurrence_key: String,
    pub address_id: String,
    pub logic_address: String,
    pub structural_hash: Option<String>,
    pub signature_hash: Option<String>,
    pub path: String,
    pub symbol: String,
    pub callees: BTreeSet<String>,
    pub unresolved: BTreeSet<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallableContractChange {
    pub occurrence_key: String,
    pub presence: ContractPresence,
    pub callers_added: BTreeSet<String>,
    pub callers_removed: BTreeSet<String>,
    pub callees_added: BTreeSet<String>,
    pub callees_removed: BTreeSet<String>,
    pub unresolved_added: BTreeSet<String>,
    pub unresolved_removed: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractPresence {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractDiff {
    pub base_snapshot_id: String,
    pub head_snapshot_id: String,
    pub changes: Vec<CallableContractChange>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractSnapshotReceipt {
    pub snapshot_id: String,
    pub subject: String,
    pub revision: String,
    pub tree_digest: String,
    pub database: PathBuf,
    pub callable_count: usize,
    pub callee_count: usize,
    pub unresolved_count: usize,
    pub diff: Option<ContractDiff>,
}

/// Parse `root` and store caller/callee sets for `revision`. Refuses to replace
/// an existing snapshot id. Diffs against the previous snapshot for `subject`.
pub fn capture_contract_snapshot(
    root: &Path,
    database: &Path,
    subject: &str,
    revision: &str,
) -> Result<ContractSnapshotReceipt, ContractSnapshotError> {
    let (graph, _) = crate::build_topology_with_syntax_cache(
        root,
        TopologyBuildOptions {
            repository: Some(subject),
            ..TopologyBuildOptions::default()
        },
        None,
    )
    .map_err(|error| ContractSnapshotError::Message(error.to_string()))?;
    let callables = extract_contracts(&graph)?;
    let tree_digest = graph
        .content_digest()
        .map_err(|error| ContractSnapshotError::Message(error.to_string()))?;
    let snapshot_id = entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
        "entmoot.contract_snapshot.v1",
        subject,
        revision,
        &tree_digest,
    ))?);
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(database)?;
    connection.execute_batch(SCHEMA)?;
    let previous: Option<String> = match connection.query_row(
        "SELECT snapshot_id FROM contract_snapshots WHERE subject=?1 ORDER BY rowid DESC LIMIT 1",
        [subject],
        |row| row.get(0),
    ) {
        Ok(id) => Some(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(error) => return Err(error.into()),
    };
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM contract_snapshots WHERE snapshot_id=?1)",
        [&snapshot_id],
        |row| row.get(0),
    )?;
    if !exists {
        let tx = connection.transaction()?;
        let callee_count: usize = callables.iter().map(|item| item.callees.len()).sum();
        let unresolved_count: usize = callables.iter().map(|item| item.unresolved.len()).sum();
        let callable_len = sqlite_count(callables.len())?;
        let callee_len = sqlite_count(callee_count)?;
        let unresolved_len = sqlite_count(unresolved_count)?;
        tx.execute(
            "INSERT INTO contract_snapshots VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                snapshot_id,
                subject,
                revision,
                tree_digest,
                callable_len,
                callee_len,
                unresolved_len
            ],
        )?;
        let mut insert_callable =
            tx.prepare("INSERT INTO contract_callables VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        let mut insert_callee = tx.prepare("INSERT INTO contract_callees VALUES(?1,?2,?3)")?;
        let mut insert_unresolved =
            tx.prepare("INSERT INTO contract_unresolved VALUES(?1,?2,?3)")?;
        for callable in &callables {
            insert_callable.execute(params![
                snapshot_id,
                callable.occurrence_key,
                callable.address_id,
                callable.logic_address,
                callable.structural_hash,
                callable.signature_hash,
                callable.path,
                callable.symbol
            ])?;
            for callee in &callable.callees {
                insert_callee.execute(params![snapshot_id, callable.occurrence_key, callee])?;
            }
            for target in &callable.unresolved {
                insert_unresolved.execute(params![snapshot_id, callable.occurrence_key, target])?;
            }
        }
        drop(insert_callable);
        drop(insert_callee);
        drop(insert_unresolved);
        tx.commit()?;
    }
    let diff = previous
        .filter(|prior| prior != &snapshot_id)
        .map(|prior| diff_snapshots(&connection, &prior, &snapshot_id))
        .transpose()?;
    let (callable_count, callee_count, unresolved_count) = connection.query_row(
        "SELECT callable_count,callee_count,unresolved_count FROM contract_snapshots WHERE snapshot_id=?1",
        [&snapshot_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
    )?;
    Ok(ContractSnapshotReceipt {
        snapshot_id,
        subject: subject.to_owned(),
        revision: revision.to_owned(),
        tree_digest,
        database: database.to_path_buf(),
        callable_count: host_count(callable_count)?,
        callee_count: host_count(callee_count)?,
        unresolved_count: host_count(unresolved_count)?,
        diff,
    })
}

pub fn contract_callees(
    database: &Path,
    snapshot_id: &str,
    occurrence_key: &str,
) -> Result<BTreeSet<String>, ContractSnapshotError> {
    let connection = Connection::open(database)?;
    set_for(
        &connection,
        "SELECT callee_key FROM contract_callees WHERE snapshot_id=?1 AND caller_key=?2 ORDER BY callee_key",
        snapshot_id,
        occurrence_key,
    )
}

pub fn contract_callers(
    database: &Path,
    snapshot_id: &str,
    occurrence_key: &str,
) -> Result<BTreeSet<String>, ContractSnapshotError> {
    let connection = Connection::open(database)?;
    set_for(
        &connection,
        "SELECT caller_key FROM contract_callees WHERE snapshot_id=?1 AND callee_key=?2 ORDER BY caller_key",
        snapshot_id,
        occurrence_key,
    )
}

pub fn diff_contract_snapshots(
    database: &Path,
    base_snapshot_id: &str,
    head_snapshot_id: &str,
) -> Result<ContractDiff, ContractSnapshotError> {
    let connection = Connection::open(database)?;
    diff_snapshots(&connection, base_snapshot_id, head_snapshot_id)
}

fn set_for(
    connection: &Connection,
    sql: &str,
    snapshot_id: &str,
    occurrence_key: &str,
) -> Result<BTreeSet<String>, ContractSnapshotError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params![snapshot_id, occurrence_key], |row| row.get(0))?;
    rows.collect::<Result<BTreeSet<_>, _>>()
        .map_err(ContractSnapshotError::from)
}

/// Span-free occurrence key. Colliding keys in one snapshot keep `address_id`
/// so equal bodies at different sites stay distinct.
pub fn extract_contracts(
    graph: &TopologyGraph,
) -> Result<Vec<CallableContract>, ContractSnapshotError> {
    let mut grouped: BTreeMap<String, Vec<CallableContract>> = BTreeMap::new();
    for node in graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Callable)
    {
        let logic_address = node.logic_address.clone().ok_or_else(|| {
            ContractSnapshotError::Message(format!("callable {} lacks a logic address", node.id))
        })?;
        let span_free = span_free_occurrence(&logic_address).ok_or_else(|| {
            ContractSnapshotError::Message(format!("unreadable logic address on {}", node.id))
        })?;
        grouped
            .entry(span_free)
            .or_default()
            .push(CallableContract {
                occurrence_key: String::new(),
                address_id: node.address_id.clone().unwrap_or_default(),
                logic_address,
                structural_hash: node.structural_hash.clone(),
                signature_hash: node.signature_hash.clone(),
                path: node.path.clone().unwrap_or_default(),
                symbol: node.label.clone(),
                callees: BTreeSet::new(),
                unresolved: BTreeSet::new(),
            });
    }
    let mut by_node: BTreeMap<&str, String> = BTreeMap::new();
    let mut callables = Vec::new();
    for (span_free, mut group) in grouped {
        let unique = group.len() == 1;
        for callable in &mut group {
            callable.occurrence_key = if unique {
                span_free.clone()
            } else {
                format!("{span_free}#{}", callable.address_id)
            };
        }
        for node in graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Callable)
        {
            if let Some(address) = &node.logic_address {
                if span_free_occurrence(address).as_deref() == Some(span_free.as_str()) {
                    let key = if unique {
                        span_free.clone()
                    } else {
                        format!(
                            "{span_free}#{}",
                            node.address_id.clone().unwrap_or_default()
                        )
                    };
                    by_node.insert(node.id.as_str(), key);
                }
            }
        }
        callables.append(&mut group);
    }
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (ordinal, callable) in callables.iter().enumerate() {
        index.insert(callable.occurrence_key.clone(), ordinal);
    }
    let labels: BTreeMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.label.as_str()))
        .collect();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls)
    {
        let Some(caller) = by_node.get(edge.from.as_str()) else {
            continue;
        };
        let Some(slot) = index.get(caller).copied() else {
            continue;
        };
        if edge.resolved {
            if let Some(callee) = by_node.get(edge.to.as_str()) {
                callables[slot].callees.insert(callee.clone());
            }
        } else if let Some(label) = labels.get(edge.to.as_str()).copied() {
            // Stub node ids include spans. The called name is the contract token.
            if !label.is_empty() {
                callables[slot].unresolved.insert(label.to_owned());
            }
        }
    }
    Ok(callables)
}

fn span_free_occurrence(logic_address: &str) -> Option<String> {
    let parts: Vec<&str> = logic_address.split('|').collect();
    if parts.len() < 7 || parts[0] != "logic" {
        return None;
    }
    Some(format!(
        "{}|{}|{}|{}|{}",
        parts[2], parts[3], parts[4], parts[5], parts[6]
    ))
}

fn diff_snapshots(
    connection: &Connection,
    base: &str,
    head: &str,
) -> Result<ContractDiff, ContractSnapshotError> {
    let base_keys = keys(connection, base)?;
    let head_keys = keys(connection, head)?;
    let mut changes = Vec::new();
    for key in base_keys.union(&head_keys) {
        let in_base = base_keys.contains(key);
        let in_head = head_keys.contains(key);
        let presence = match (in_base, in_head) {
            (false, true) => ContractPresence::Added,
            (true, false) => ContractPresence::Removed,
            (true, true) => ContractPresence::Changed,
            (false, false) => continue,
        };
        let base_callees = if in_base {
            edge_set(connection, "callee_key", "caller_key", base, key)?
        } else {
            BTreeSet::new()
        };
        let head_callees = if in_head {
            edge_set(connection, "callee_key", "caller_key", head, key)?
        } else {
            BTreeSet::new()
        };
        let base_callers = if in_base {
            edge_set(connection, "caller_key", "callee_key", base, key)?
        } else {
            BTreeSet::new()
        };
        let head_callers = if in_head {
            edge_set(connection, "caller_key", "callee_key", head, key)?
        } else {
            BTreeSet::new()
        };
        let base_unresolved = if in_base {
            unresolved_set(connection, base, key)?
        } else {
            BTreeSet::new()
        };
        let head_unresolved = if in_head {
            unresolved_set(connection, head, key)?
        } else {
            BTreeSet::new()
        };
        let change = CallableContractChange {
            occurrence_key: key.clone(),
            presence,
            callers_added: head_callers.difference(&base_callers).cloned().collect(),
            callers_removed: base_callers.difference(&head_callers).cloned().collect(),
            callees_added: head_callees.difference(&base_callees).cloned().collect(),
            callees_removed: base_callees.difference(&head_callees).cloned().collect(),
            unresolved_added: head_unresolved
                .difference(&base_unresolved)
                .cloned()
                .collect(),
            unresolved_removed: base_unresolved
                .difference(&head_unresolved)
                .cloned()
                .collect(),
        };
        let sets_unchanged = change.callers_added.is_empty()
            && change.callers_removed.is_empty()
            && change.callees_added.is_empty()
            && change.callees_removed.is_empty()
            && change.unresolved_added.is_empty()
            && change.unresolved_removed.is_empty();
        if presence == ContractPresence::Changed && sets_unchanged {
            continue;
        }
        changes.push(change);
    }
    changes.sort_by(|left, right| left.occurrence_key.cmp(&right.occurrence_key));
    Ok(ContractDiff {
        base_snapshot_id: base.to_owned(),
        head_snapshot_id: head.to_owned(),
        changes,
    })
}

fn keys(
    connection: &Connection,
    snapshot: &str,
) -> Result<BTreeSet<String>, ContractSnapshotError> {
    let mut statement =
        connection.prepare("SELECT occurrence_key FROM contract_callables WHERE snapshot_id=?1")?;
    let rows = statement.query_map([snapshot], |row| row.get(0))?;
    rows.collect::<Result<_, _>>()
        .map_err(ContractSnapshotError::from)
}

fn edge_set(
    connection: &Connection,
    select_column: &str,
    match_column: &str,
    snapshot: &str,
    key: &str,
) -> Result<BTreeSet<String>, ContractSnapshotError> {
    let sql = format!(
        "SELECT {select_column} FROM contract_callees WHERE snapshot_id=?1 AND {match_column}=?2"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![snapshot, key], |row| row.get(0))?;
    rows.collect::<Result<_, _>>()
        .map_err(ContractSnapshotError::from)
}

fn unresolved_set(
    connection: &Connection,
    snapshot: &str,
    key: &str,
) -> Result<BTreeSet<String>, ContractSnapshotError> {
    let mut statement = connection
        .prepare("SELECT target FROM contract_unresolved WHERE snapshot_id=?1 AND caller_key=?2")?;
    let rows = statement.query_map(params![snapshot, key], |row| row.get(0))?;
    rows.collect::<Result<_, _>>()
        .map_err(ContractSnapshotError::from)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractMatch {
    pub occurrence_key: String,
    pub symbol: String,
    pub path: String,
    pub structural_hash: Option<String>,
    pub signature_hash: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractStatus {
    pub snapshot_id: String,
    pub subject: String,
    pub revision: String,
    pub previous_snapshot_id: Option<String>,
    pub callable_count: usize,
    pub callee_count: usize,
    pub unresolved_count: usize,
    pub diff: Option<ContractDiff>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractInspection {
    pub occurrence_key: String,
    pub structural_hash: Option<String>,
    pub signature_hash: Option<String>,
    pub callers: BTreeSet<String>,
    pub callees: BTreeSet<String>,
    pub unresolved: BTreeSet<String>,
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractCallSet {
    pub occurrence_key: String,
    pub set: BTreeSet<String>,
    pub unresolved: BTreeSet<String>,
    pub change: Option<CallableContractChange>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractInventory {
    pub snapshot_id: String,
    pub gap: Option<String>,
    pub callables: Vec<ContractMatch>,
    pub offset: usize,
    pub total: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractBoundaries {
    pub occurrence_key: Option<String>,
    pub unresolved: BTreeSet<String>,
    pub gap: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UnboundEdge {
    pub caller_key: String,
    pub target: String,
    pub status: String,
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractPath {
    pub start: String,
    pub target: String,
    pub routes: Vec<Vec<String>>,
    pub unbound: Vec<UnboundEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractNeighborhood {
    pub occurrence_key: String,
    pub direction: String,
    pub depth: usize,
    pub bound: BTreeSet<String>,
    pub unbound: Vec<UnboundEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractComponents {
    pub total: usize,
    pub offset: usize,
    pub components: Vec<Vec<String>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContractRelay {
    pub diff: ContractDiff,
    pub affected: Vec<ContractCallSet>,
}

struct DirectedEdges {
    out: BTreeMap<String, BTreeSet<String>>,
    incoming: BTreeMap<String, BTreeSet<String>>,
    unresolved: BTreeMap<String, BTreeSet<String>>,
}

fn open_snapshot(database: &Path) -> Result<Connection, ContractSnapshotError> {
    if !database.is_file() {
        return Err(ContractSnapshotError::Message(
            "no contract snapshot; run entmoot init".into(),
        ));
    }
    Ok(Connection::open(database)?)
}

fn latest_row(
    connection: &Connection,
) -> Result<(String, String, String, i64, i64, i64), ContractSnapshotError> {
    connection
        .query_row(
            "SELECT snapshot_id, subject, revision, callable_count, callee_count, unresolved_count
             FROM contract_snapshots ORDER BY rowid DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                ContractSnapshotError::Message("no contract snapshot; run entmoot init".into())
            }
            other => other.into(),
        })
}

fn snapshot_id(
    connection: &Connection,
    snapshot: Option<&str>,
) -> Result<String, ContractSnapshotError> {
    match snapshot {
        Some(id) => {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM contract_snapshots WHERE snapshot_id=?1)",
                [id],
                |row| row.get(0),
            )?;
            if exists {
                Ok(id.to_owned())
            } else {
                Err(ContractSnapshotError::Message(format!(
                    "no contract snapshot {id}"
                )))
            }
        }
        None => Ok(latest_row(connection)?.0),
    }
}

fn previous_snapshot(
    connection: &Connection,
    subject: &str,
    current: &str,
) -> Result<Option<String>, ContractSnapshotError> {
    match connection.query_row(
        "SELECT snapshot_id FROM contract_snapshots
         WHERE subject=?1 AND snapshot_id!=?2
         ORDER BY rowid DESC LIMIT 1",
        params![subject, current],
        |row| row.get(0),
    ) {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Latest snapshot for this database, with the set diff against the previous
/// snapshot of the same subject.
pub fn contract_status(database: &Path) -> Result<ContractStatus, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let (snapshot_id, subject, revision, callables, callees, unresolved) = latest_row(&connection)?;
    let previous = previous_snapshot(&connection, &subject, &snapshot_id)?;
    let diff = previous
        .as_ref()
        .map(|prior| diff_snapshots(&connection, prior, &snapshot_id))
        .transpose()?;
    Ok(ContractStatus {
        snapshot_id,
        subject,
        revision,
        previous_snapshot_id: previous,
        callable_count: host_count(callables)?,
        callee_count: host_count(callees)?,
        unresolved_count: host_count(unresolved)?,
        diff,
    })
}

pub fn resolve_occurrence(
    database: &Path,
    snapshot: Option<&str>,
    selector: &str,
) -> Result<String, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, snapshot)?;
    resolve_on(&connection, &snapshot, selector)
}

fn resolve_on(
    connection: &Connection,
    snapshot: &str,
    selector: &str,
) -> Result<String, ContractSnapshotError> {
    let exact: Option<String> = match connection.query_row(
        "SELECT occurrence_key FROM contract_callables WHERE snapshot_id=?1 AND occurrence_key=?2",
        params![snapshot, selector],
        |row| row.get(0),
    ) {
        Ok(key) => Some(key),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(key) = exact {
        return Ok(key);
    }
    let mut statement = connection.prepare(
        "SELECT occurrence_key FROM contract_callables WHERE snapshot_id=?1 AND symbol=?2 ORDER BY occurrence_key",
    )?;
    let mut keys = statement
        .query_map(params![snapshot, selector], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    match keys.len() {
        1 => Ok(keys.remove(0)),
        0 => Err(ContractSnapshotError::Message(format!(
            "no callable matches {selector}"
        ))),
        _ => Err(ContractSnapshotError::Message(format!(
            "{selector} matches more than one callable; pass the occurrence key"
        ))),
    }
}

pub fn find_contracts(
    database: &Path,
    query: Option<&str>,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<ContractMatch>, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let mut statement = connection.prepare(
        "SELECT occurrence_key, symbol, path, structural_hash, signature_hash
         FROM contract_callables WHERE snapshot_id=?1 ORDER BY occurrence_key",
    )?;
    let rows = statement.query_map(params![snapshot], |row| {
        Ok(ContractMatch {
            occurrence_key: row.get(0)?,
            symbol: row.get(1)?,
            path: row.get(2)?,
            structural_hash: row.get(3)?,
            signature_hash: row.get(4)?,
        })
    })?;
    let mut matches = Vec::new();
    for row in rows {
        let row = row?;
        if let Some(path) = path {
            if row.path != path && !row.path.starts_with(&format!("{path}/")) {
                continue;
            }
        }
        if let Some(query) = query {
            if row.symbol != query && !row.symbol.contains(query) && row.occurrence_key != query {
                continue;
            }
        }
        matches.push(row);
        if matches.len() == limit {
            break;
        }
    }
    Ok(matches)
}

pub fn inspect_contract(
    database: &Path,
    selector: &str,
) -> Result<ContractInspection, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let key = resolve_on(&connection, &snapshot, selector)?;
    let (structural_hash, signature_hash) = connection.query_row(
        "SELECT structural_hash, signature_hash FROM contract_callables WHERE snapshot_id=?1 AND occurrence_key=?2",
        params![snapshot, key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ContractInspection {
        callers: edge_set(&connection, "caller_key", "callee_key", &snapshot, &key)?,
        callees: edge_set(&connection, "callee_key", "caller_key", &snapshot, &key)?,
        unresolved: unresolved_set(&connection, &snapshot, &key)?,
        occurrence_key: key,
        structural_hash,
        signature_hash,
        gaps: vec![FACT_OTHER_EVIDENCE.to_owned()],
    })
}

fn change_for(
    database: &Path,
    key: &str,
) -> Result<Option<CallableContractChange>, ContractSnapshotError> {
    let status = contract_status(database)?;
    Ok(status.diff.and_then(|diff| {
        diff.changes
            .into_iter()
            .find(|change| change.occurrence_key == key)
    }))
}

pub fn contract_caller_set(
    database: &Path,
    selector: &str,
) -> Result<ContractCallSet, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let key = resolve_on(&connection, &snapshot, selector)?;
    Ok(ContractCallSet {
        set: edge_set(&connection, "caller_key", "callee_key", &snapshot, &key)?,
        unresolved: BTreeSet::new(),
        change: change_for(database, &key)?,
        occurrence_key: key,
    })
}

pub fn contract_callee_set(
    database: &Path,
    selector: &str,
) -> Result<ContractCallSet, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let key = resolve_on(&connection, &snapshot, selector)?;
    Ok(ContractCallSet {
        set: edge_set(&connection, "callee_key", "caller_key", &snapshot, &key)?,
        unresolved: unresolved_set(&connection, &snapshot, &key)?,
        change: change_for(database, &key)?,
        occurrence_key: key,
    })
}

pub fn contract_inventory(
    database: &Path,
    kind: Option<&str>,
    path: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<ContractInventory, ContractSnapshotError> {
    if kind.is_some_and(|kind| kind != "callable") {
        return Ok(ContractInventory {
            snapshot_id: String::new(),
            gap: Some(FACT_NON_CALLABLE.to_owned()),
            callables: Vec::new(),
            offset,
            total: 0,
        });
    }
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let found = find_contracts(database, None, path, usize::MAX)?;
    let total = found.len();
    let callables = found.into_iter().skip(offset).take(limit).collect();
    Ok(ContractInventory {
        snapshot_id: snapshot,
        gap: None,
        callables,
        offset,
        total,
    })
}

pub fn contract_boundaries(
    database: &Path,
    selector: Option<&str>,
    path: Option<&str>,
) -> Result<ContractBoundaries, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let occurrence = match selector {
        Some(selector) => Some(resolve_on(&connection, &snapshot, selector)?),
        None => None,
    };
    let mut statement = connection.prepare(
        "SELECT caller_key, target FROM contract_unresolved WHERE snapshot_id=?1 ORDER BY caller_key, target",
    )?;
    let rows = statement.query_map(params![snapshot], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut unresolved = BTreeSet::new();
    for row in rows {
        let (caller, target) = row?;
        if occurrence.as_ref().is_some_and(|key| key != &caller) {
            continue;
        }
        if let Some(path) = path {
            let caller_path: String = connection.query_row(
                "SELECT path FROM contract_callables WHERE snapshot_id=?1 AND occurrence_key=?2",
                params![snapshot, caller],
                |row| row.get(0),
            )?;
            if caller_path != path && !caller_path.starts_with(&format!("{path}/")) {
                continue;
            }
        }
        unresolved.insert(target);
    }
    Ok(ContractBoundaries {
        occurrence_key: occurrence,
        unresolved,
        gap: FACT_NATIVE_DISPATCH.to_owned(),
    })
}

fn load_edges(
    connection: &Connection,
    snapshot: &str,
) -> Result<DirectedEdges, ContractSnapshotError> {
    let mut out = BTreeMap::<String, BTreeSet<String>>::new();
    let mut incoming = BTreeMap::<String, BTreeSet<String>>::new();
    let mut statement = connection
        .prepare("SELECT caller_key, callee_key FROM contract_callees WHERE snapshot_id=?1")?;
    let rows = statement.query_map(params![snapshot], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (caller, callee) = row?;
        out.entry(caller.clone())
            .or_default()
            .insert(callee.clone());
        incoming.entry(callee).or_default().insert(caller);
    }
    let mut unresolved = BTreeMap::<String, BTreeSet<String>>::new();
    let mut statement = connection
        .prepare("SELECT caller_key, target FROM contract_unresolved WHERE snapshot_id=?1")?;
    let rows = statement.query_map(params![snapshot], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (caller, target) = row?;
        unresolved.entry(caller).or_default().insert(target);
    }
    Ok(DirectedEdges {
        out,
        incoming,
        unresolved,
    })
}

fn classify_unbound(
    connection: &Connection,
    against: Option<&str>,
    caller: &str,
    target: &str,
) -> Result<UnboundEdge, ContractSnapshotError> {
    let matches = match against {
        None => Vec::new(),
        Some(snapshot) => {
            let mut statement = connection.prepare(
                "SELECT occurrence_key FROM contract_callables WHERE snapshot_id=?1 AND symbol=?2 ORDER BY occurrence_key",
            )?;
            let matched = statement
                .query_map(params![snapshot, target], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            matched
        }
    };
    let status = match (against.is_some(), matches.len()) {
        (false, _) => "unbound",
        (true, 0) => "unbound",
        (true, 1) => "bound",
        (true, _) => "ambiguous",
    };
    Ok(UnboundEdge {
        caller_key: caller.to_owned(),
        target: target.to_owned(),
        status: status.to_owned(),
        matches,
    })
}

pub fn contract_path(
    database: &Path,
    start: &str,
    target: &str,
    max_hops: usize,
    against: Option<&str>,
) -> Result<ContractPath, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let start = resolve_on(&connection, &snapshot, start)?;
    let target = resolve_on(&connection, &snapshot, target)?;
    let edges = load_edges(&connection, &snapshot)?;
    let mut routes = Vec::new();
    let mut stack = vec![(start.clone(), vec![start.clone()])];
    let mut seen_unbound = BTreeSet::new();
    while let Some((node, path)) = stack.pop() {
        if path.len() - 1 > max_hops {
            continue;
        }
        if let Some(names) = edges.unresolved.get(&node) {
            for name in names {
                seen_unbound.insert((node.clone(), name.clone()));
            }
        }
        if node == target && path.len() > 1 {
            routes.push(path);
            if routes.len() == 10 {
                break;
            }
            continue;
        }
        if path.len() - 1 == max_hops {
            continue;
        }
        for next in edges.out.get(&node).into_iter().flatten() {
            if path.contains(next) {
                continue;
            }
            let mut extended = path.clone();
            extended.push(next.clone());
            stack.push((next.clone(), extended));
        }
    }
    if start == target {
        routes.push(vec![start.clone()]);
    }
    let mut unbound = Vec::new();
    for (caller, name) in seen_unbound {
        unbound.push(classify_unbound(&connection, against, &caller, &name)?);
    }
    Ok(ContractPath {
        start,
        target,
        routes,
        unbound,
    })
}

pub fn contract_neighborhood(
    database: &Path,
    selector: &str,
    direction: &str,
    depth: usize,
    against: Option<&str>,
) -> Result<ContractNeighborhood, ContractSnapshotError> {
    if direction != "callers" && direction != "callees" && direction != "both" {
        return Err(ContractSnapshotError::Message(format!(
            "unknown direction {direction}"
        )));
    }
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let start = resolve_on(&connection, &snapshot, selector)?;
    let edges = load_edges(&connection, &snapshot)?;
    let mut bound = BTreeSet::new();
    let mut frontier = BTreeSet::from([start.clone()]);
    let mut seen_unbound = BTreeSet::new();
    for _ in 0..depth.max(1) {
        let mut next = BTreeSet::new();
        for node in &frontier {
            if direction != "callers" {
                if let Some(names) = edges.unresolved.get(node) {
                    for name in names {
                        seen_unbound.insert((node.clone(), name.clone()));
                    }
                }
                for callee in edges.out.get(node).into_iter().flatten() {
                    if bound.insert(callee.clone()) {
                        next.insert(callee.clone());
                    }
                }
            }
            if direction != "callees" {
                for caller in edges.incoming.get(node).into_iter().flatten() {
                    if bound.insert(caller.clone()) {
                        next.insert(caller.clone());
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let mut unbound = Vec::new();
    for (caller, name) in seen_unbound {
        unbound.push(classify_unbound(&connection, against, &caller, &name)?);
    }
    Ok(ContractNeighborhood {
        occurrence_key: start,
        direction: direction.to_owned(),
        depth,
        bound,
        unbound,
    })
}

pub fn contract_components(
    database: &Path,
    offset: usize,
    limit: usize,
) -> Result<ContractComponents, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let snapshot = snapshot_id(&connection, None)?;
    let edges = load_edges(&connection, &snapshot)?;
    let mut keys = keys(&connection, &snapshot)?;
    for endpoint in edges.out.keys().chain(edges.incoming.keys()) {
        keys.insert(endpoint.clone());
    }
    let mut parent: BTreeMap<String, String> =
        keys.iter().cloned().map(|key| (key.clone(), key)).collect();
    fn find(parent: &mut BTreeMap<String, String>, key: &str) -> String {
        let current = parent.get(key).cloned().unwrap_or_else(|| key.to_owned());
        if current == key {
            return current;
        }
        let root = find(parent, &current);
        parent.insert(key.to_owned(), root.clone());
        root
    }
    for (caller, callees) in &edges.out {
        for callee in callees {
            let left = find(&mut parent, caller);
            let right = find(&mut parent, callee);
            if left != right {
                parent.insert(left, right);
            }
        }
    }
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for key in &keys {
        let root = find(&mut parent, key);
        groups.entry(root).or_default().insert(key.clone());
    }
    let mut components: Vec<Vec<String>> = groups
        .into_values()
        .map(|set| set.into_iter().collect())
        .collect();
    components.sort();
    let total = components.len();
    let components = components.into_iter().skip(offset).take(limit).collect();
    Ok(ContractComponents {
        total,
        offset,
        components,
    })
}

pub fn contract_relay(
    database: &Path,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<ContractRelay, ContractSnapshotError> {
    let status = contract_status(database)?;
    let head_id = head.unwrap_or(&status.snapshot_id).to_owned();
    let base_id = match base {
        Some(base) => base.to_owned(),
        None => status.previous_snapshot_id.ok_or_else(|| {
            ContractSnapshotError::Message("relay impact needs two snapshots".into())
        })?,
    };
    let diff = diff_contract_snapshots(database, &base_id, &head_id)?;
    let connection = open_snapshot(database)?;
    let mut affected = Vec::new();
    for change in &diff.changes {
        let snapshot = if change.presence == ContractPresence::Removed {
            &base_id
        } else {
            &head_id
        };
        affected.push(ContractCallSet {
            set: edge_set(
                &connection,
                "caller_key",
                "callee_key",
                snapshot,
                &change.occurrence_key,
            )?,
            unresolved: BTreeSet::new(),
            occurrence_key: change.occurrence_key.clone(),
            change: Some(change.clone()),
        });
    }
    Ok(ContractRelay { diff, affected })
}

pub fn bind_unbound_edges(
    database: &Path,
    snapshot: &str,
    against: &str,
) -> Result<Vec<UnboundEdge>, ContractSnapshotError> {
    let connection = open_snapshot(database)?;
    let mut statement = connection.prepare(
        "SELECT caller_key, target FROM contract_unresolved WHERE snapshot_id=?1 ORDER BY caller_key, target",
    )?;
    let rows = statement.query_map(params![snapshot], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut edges = Vec::new();
    for row in rows {
        let (caller, target) = row?;
        edges.push(classify_unbound(
            &connection,
            Some(against),
            &caller,
            &target,
        )?);
    }
    Ok(edges)
}

fn sqlite_count(value: usize) -> Result<i64, ContractSnapshotError> {
    i64::try_from(value).map_err(|error| ContractSnapshotError::Message(error.to_string()))
}

fn host_count(value: i64) -> Result<usize, ContractSnapshotError> {
    usize::try_from(value).map_err(|error| ContractSnapshotError::Message(error.to_string()))
}

impl From<std::io::Error> for ContractSnapshotError {
    fn from(error: std::io::Error) -> Self {
        Self::Message(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_snapshot_diffs_empty_and_a_new_callee_does_not() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let database = data.path().join("contracts.sqlite3");
        std::fs::write(
            source.path().join("mod.py"),
            "def callee():\n    return 1\n\ndef caller():\n    return callee()\n",
        )
        .unwrap();
        let first =
            capture_contract_snapshot(source.path(), &database, "fixture", "rev-1").unwrap();
        assert!(first.callable_count >= 2);
        assert!(first.callee_count >= 1);
        assert!(first.diff.is_none());
        let caller = first_caller_key(&database, &first.snapshot_id);
        let before = contract_callees(&database, &first.snapshot_id, &caller).unwrap();
        assert!(!before.is_empty());

        std::fs::write(
            source.path().join("mod.py"),
            "# comment only\ndef callee():\n    return 1\n\ndef caller():\n    return callee()\n",
        )
        .unwrap();
        let comment =
            capture_contract_snapshot(source.path(), &database, "fixture", "rev-2").unwrap();
        let diff = comment.diff.expect("second snapshot diffs the first");
        assert!(
            diff.changes.is_empty(),
            "comment must not emit a contract change: {diff:?}"
        );

        std::fs::write(
            source.path().join("mod.py"),
            "def other():\n    return 2\n\ndef callee():\n    return other()\n\ndef caller():\n    return callee()\n",
        )
        .unwrap();
        let changed =
            capture_contract_snapshot(source.path(), &database, "fixture", "rev-3").unwrap();
        let diff = changed.diff.expect("body change diffs");
        assert!(
            diff.changes
                .iter()
                .any(|change| !change.callees_added.is_empty() || !change.callers_added.is_empty()),
            "a new callee must show up in the set diff: {diff:?}"
        );
    }

    #[test]
    fn path_keeps_unbound_edges_and_another_snapshot_can_bind_them() {
        let source = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let database = data.path().join("contracts.sqlite3");
        std::fs::write(
            source.path().join("walk.py"),
            "def target():\n    return 1\n\ndef middle():\n    return target()\n\ndef start():\n    middle()\n    missing()\n",
        )
        .unwrap();
        let first = capture_contract_snapshot(source.path(), &database, "walk", "rev-a").unwrap();
        let walked = contract_path(&database, "start", "target", 4, None).unwrap();
        assert!(
            walked.routes.iter().any(|route| route.len() == 3),
            "bound calls should reach target: {walked:?}"
        );
        assert!(
            walked
                .unbound
                .iter()
                .any(|edge| edge.target == "missing" && edge.status == "unbound"),
            "the unnamed call stays on the walk: {walked:?}"
        );
        std::fs::write(
            other.path().join("other.py"),
            "def missing():\n    return 0\n",
        )
        .unwrap();
        let second = capture_contract_snapshot(other.path(), &database, "other", "rev-b").unwrap();
        let matches =
            bind_unbound_edges(&database, &first.snapshot_id, &second.snapshot_id).unwrap();
        assert!(
            matches.iter().any(|edge| edge.target == "missing"
                && edge.status == "bound"
                && edge.matches.len() == 1),
            "one symbol in the other snapshot binds the edge: {matches:?}"
        );
    }

    fn first_caller_key(database: &Path, snapshot: &str) -> String {
        let connection = Connection::open(database).unwrap();
        connection
            .query_row(
                "SELECT caller_key FROM contract_callees WHERE snapshot_id=?1 LIMIT 1",
                [snapshot],
                |row| row.get(0),
            )
            .unwrap()
    }
}
