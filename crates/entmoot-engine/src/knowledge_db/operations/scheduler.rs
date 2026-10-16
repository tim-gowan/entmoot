//! Short, transactional admission; never sleeps or executes producers.
use super::*;
use entmoot_contracts::{
    OperationKind, ScheduledOperation, SchedulerConfiguration, SchedulerPolicy, SchedulerStatus,
};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE local_scheduler_profiles (
    repository_id TEXT PRIMARY KEY, generation INTEGER NOT NULL, body BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE local_operation_waiters (
    ticket INTEGER PRIMARY KEY, operation_id TEXT NOT NULL UNIQUE REFERENCES local_operations(operation_id) ON DELETE CASCADE,
    repository_id TEXT NOT NULL, generation INTEGER NOT NULL, kind TEXT NOT NULL,
    expires_ms INTEGER NOT NULL
);
CREATE INDEX local_waiters_by_repository ON local_operation_waiters(repository_id,ticket);
CREATE INDEX local_waiters_by_expiry ON local_operation_waiters(repository_id,expires_ms);
"#;

fn configuration(
    connection: &Connection,
    repository: &str,
) -> Result<SchedulerConfiguration, OperationError> {
    let row: Option<(i64, Vec<u8>)> = connection
        .query_row(
            "SELECT generation,body FROM local_scheduler_profiles WHERE repository_id=?1",
            [repository],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (generation, policy) = match row {
        Some((generation, body)) => (
            u64::try_from(generation).map_err(|e| OperationError::Invalid(e.to_string()))?,
            serde_json::from_slice::<SchedulerPolicy>(&body)?,
        ),
        None => (0, SchedulerPolicy::default()),
    };
    if generation > i64::MAX as u64 {
        return Err(OperationError::Invalid(
            "invalid scheduler generation".into(),
        ));
    }
    policy.validate().map_err(OperationError::Invalid)?;
    Ok(SchedulerConfiguration { generation, policy })
}

pub(super) fn configure(
    connection: &mut Connection,
    repository: &str,
    expected: u64,
    policy: SchedulerPolicy,
) -> Result<SchedulerConfiguration, OperationError> {
    check_text(repository)?;
    policy.validate().map_err(OperationError::Invalid)?;
    let generation = next(expected)?;
    let bytes = encode(&policy)?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if configuration(&tx, repository)?.generation != expected {
        return Err(OperationError::Conflict(
            "scheduler configuration concurrently advanced".into(),
        ));
    }
    tx.execute("INSERT INTO local_scheduler_profiles VALUES(?1,?2,?3) ON CONFLICT(repository_id) DO UPDATE SET generation=excluded.generation,body=excluded.body", params![repository,i64::try_from(generation).map_err(|e| OperationError::Invalid(e.to_string()))?,bytes])?;
    tx.commit()?;
    Ok(SchedulerConfiguration { generation, policy })
}

fn running(
    connection: &Connection,
    repository: &str,
    now: u64,
    policy: &SchedulerPolicy,
) -> Result<Vec<ScheduledOperation>, OperationError> {
    let mut statement = connection.prepare("SELECT body FROM local_operations WHERE repository_id=?1 AND state='\"running\"' ORDER BY ordinal")?;
    let rows = statement.query_map([repository], |r| r.get::<_, Vec<u8>>(0))?;
    let mut active = Vec::new();
    for row in rows {
        let item: OperationRecord = serde_json::from_slice(&row?)?;
        if let Some(expires_ms) = item.lease_until_ms.filter(|until| *until > now) {
            active.push(ScheduledOperation {
                id: item.id,
                kind: item.spec.kind,
                work_units: policy.cost(item.spec.kind),
                expires_ms,
            });
        }
    }
    Ok(active)
}

fn waiting(
    connection: &Connection,
    repository: &str,
    now: u64,
    policy: &SchedulerPolicy,
    limit: usize,
    enabled_only: bool,
) -> Result<Vec<ScheduledOperation>, OperationError> {
    let mut statement = connection.prepare("SELECT operation_id,kind,expires_ms FROM local_operation_waiters WHERE repository_id=?1 AND expires_ms>?2 AND (?3 OR kind<>'\"binding\"') ORDER BY ticket LIMIT ?4")?;
    let rows = statement.query_map(
        params![
            repository,
            i64::try_from(now).map_err(|e| OperationError::Invalid(e.to_string()))?,
            !enabled_only || policy.max_binding > 0,
            i64::try_from(limit).map_err(|e| OperationError::Invalid(e.to_string()))?
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    rows.map(|row| {
        let (id, kind, expires) = row?;
        let kind = serde_json::from_str(&kind)?;
        Ok(ScheduledOperation {
            id: ContentDigest::parse(id).map_err(|e| OperationError::Invalid(e.to_string()))?,
            kind,
            work_units: policy.cost(kind),
            expires_ms: u64::try_from(expires)
                .map_err(|e| OperationError::Invalid(e.to_string()))?,
        })
    })
    .collect()
}

fn fits(policy: &SchedulerPolicy, active: &[ScheduledOperation], kind: OperationKind) -> bool {
    policy.enabled(kind)
        && active.len() < policy.max_running as usize
        && active.iter().map(|r| r.work_units as u64).sum::<u64>() + policy.cost(kind) as u64
            <= policy.work_units as u64
        && (kind != OperationKind::Binding
            || active
                .iter()
                .filter(|r| r.kind == OperationKind::Binding)
                .count()
                < policy.max_binding as usize)
}

/// Called only after state, prerequisites and implementation have been checked.
/// A live FIFO head reserves the next admission even when a smaller job fits.
pub(super) fn admit(
    tx: &Transaction<'_>,
    item: &OperationRecord,
    now: u64,
) -> Result<bool, OperationError> {
    let repository = &item.spec.repository_id;
    let policy = configuration(tx, repository)?.policy;
    let expires = now
        .checked_add(policy.wait_lease_ms)
        .filter(|t| *t <= i64::MAX as u64)
        .ok_or_else(|| OperationError::Invalid("waiting lease clock overflow".into()))?;
    tx.execute(
        "DELETE FROM local_operation_waiters WHERE repository_id=?1 AND expires_ms<=?2",
        params![
            repository,
            i64::try_from(now).map_err(|e| OperationError::Invalid(e.to_string()))?
        ],
    )?;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM local_operation_waiters WHERE operation_id=?1 AND generation=?2)", params![item.id.as_str(),i64::try_from(item.generation).map_err(|e| OperationError::Invalid(e.to_string()))?], |r| r.get(0))?;
    if !exists {
        tx.execute(
            "DELETE FROM local_operation_waiters WHERE operation_id=?1",
            [item.id.as_str()],
        )?;
        let count: u32 = tx.query_row(
            "SELECT count(*) FROM local_operation_waiters WHERE repository_id=?1",
            [repository],
            |r| r.get(0),
        )?;
        if count >= policy.max_waiting {
            return Ok(false);
        }
    }
    tx.execute("INSERT INTO local_operation_waiters(operation_id,repository_id,generation,kind,expires_ms) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(operation_id) DO UPDATE SET expires_ms=max(expires_ms,excluded.expires_ms)", params![item.id.as_str(),repository,i64::try_from(item.generation).map_err(|e| OperationError::Invalid(e.to_string()))?,serde_json::to_string(&item.spec.kind)?,i64::try_from(expires).map_err(|e| OperationError::Invalid(e.to_string()))?])?;
    let first = waiting(tx, repository, now, &policy, 1, true)?;
    if first.first().is_none_or(|first| first.id != item.id) {
        return Ok(false);
    }
    Ok(fits(
        &policy,
        &running(tx, repository, now, &policy)?,
        item.spec.kind,
    ))
}

pub(super) fn status(
    connection: &Connection,
    repository: &str,
    now: u64,
) -> Result<SchedulerStatus, OperationError> {
    check_text(repository)?;
    if now > i64::MAX as u64 {
        return Err(OperationError::Invalid("scheduler clock overflow".into()));
    }
    let _snapshot = connection
        .is_autocommit()
        .then(|| connection.unchecked_transaction())
        .transpose()?;
    let tx = connection;
    let configuration = configuration(tx, repository)?;
    let active = running(tx, repository, now, &configuration.policy)?;
    let waiting = waiting(tx, repository, now, &configuration.policy, 100, false)?;
    let first = self::waiting(tx, repository, now, &configuration.policy, 1, true)?
        .into_iter()
        .next();
    let waiting_total: u32 = tx.query_row(
        "SELECT count(*) FROM local_operation_waiters WHERE repository_id=?1 AND expires_ms>?2",
        params![
            repository,
            i64::try_from(now).map_err(|e| OperationError::Invalid(e.to_string()))?
        ],
        |r| r.get(0),
    )?;
    Ok(SchedulerStatus {
        repository_id: repository.into(),
        used_work_units: active.iter().map(|r| r.work_units as u64).sum(),
        next_can_run: first
            .as_ref()
            .is_some_and(|r| fits(&configuration.policy, &active, r.kind)),
        next: first.map(|r| r.id),
        configuration,
        observed_at_ms: now,
        active,
        waiting_truncated: waiting.len() < waiting_total as usize,
        waiting,
        waiting_total: waiting_total as usize,
    })
}
