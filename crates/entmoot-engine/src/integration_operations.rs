//! Ordered same-repository Git integration with durable, fenced private prefixes.
use crate::{
    collaboration::{captured_input, check_capture_scope, now_ms},
    collaboration_git::GitIntegration,
    LocalOperations, OperationError, SqliteKnowledgeRepository,
};
use entmoot_contracts::{
    ContentDigest, IntegrationRequest, IntegrationResult, IntegrationStepReport, OperationKind,
    OperationRecord, OperationSpec, OperationState, SourceState, WorkspacePublication,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    next_member: usize,
    head: Option<String>,
    git_version: Option<String>,
}

pub(crate) fn views(request: &IntegrationRequest) -> Result<Vec<ContentDigest>, OperationError> {
    if !(1..=1000).contains(&request.members.len()) {
        return Err(OperationError::Invalid(
            "integration requires 1..1000 ordered members".into(),
        ));
    }
    let mut names = BTreeSet::new();
    let mut views = Vec::new();
    for row in std::iter::once(&request.initial).chain(&request.members) {
        row.snapshot
            .validate()
            .map_err(|e| OperationError::Invalid(e.to_string()))?;
        if row.snapshot.repository_id != request.initial.snapshot.repository_id
            || !names.insert(&row.workspace_id)
        {
            return Err(OperationError::Invalid(
                "integration requires distinct workspace identities in one repository".into(),
            ));
        }
        if row.git_capture.is_none() && row.snapshot.source_state != SourceState::Committed {
            return Err(OperationError::Unavailable(
                "capture working-tree Git inputs before selecting an integration order".into(),
            ));
        }
        views.push(row.view.clone());
    }
    views.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    views.dedup();
    Ok(views)
}

pub fn submit(
    repository: &mut impl LocalOperations,
    request: IntegrationRequest,
    key: &str,
    owner: &str,
) -> Result<OperationRecord, OperationError> {
    let views = views(&request)?;
    for row in std::iter::once(&request.initial).chain(&request.members) {
        if let Some(id) = &row.git_capture {
            check_capture_scope(&repository.git_capture_receipt(id.as_str())?, row)?;
        }
    }
    let spec = OperationSpec {
        repository_id: request.initial.snapshot.repository_id.clone(),
        idempotency_key: key.into(),
        owner: owner.into(),
        kind: OperationKind::Integration,
        views,
        prerequisites: vec![],
        input: serde_json::to_value(request)?,
        implementation: crate::implementation_fingerprint(),
    };
    repository
        .submit_operations(&[spec], 1000)?
        .pop()
        .ok_or_else(|| OperationError::Invalid("missing submitted integration".into()))
}

fn parent(db: &impl LocalOperations, row: &WorkspacePublication) -> Result<String, OperationError> {
    if let Some(id) = &row.git_capture {
        let r = db.git_capture_receipt(id.as_str())?;
        check_capture_scope(&r, row)?;
        Ok(r.parent_commit)
    } else {
        Ok(row.snapshot.revision.clone())
    }
}

fn live(db: &impl LocalOperations, id: &str, generation: u64) -> Result<(), OperationError> {
    let now = now_ms()?;
    let item = db
        .operation(id)?
        .ok_or_else(|| OperationError::Invalid("missing integration operation".into()))?;
    if item.generation != generation
        || item.state != OperationState::Running
        || item.lease_until_ms.is_none_or(|t| now >= t)
    {
        return Err(OperationError::Conflict(
            "integration worker revoked or expired".into(),
        ));
    }
    Ok(())
}

/// Time limits stop admission between merges; stored deltas permit a new process
/// to continue without repeating earlier merges or sharing a mutable Git worktree.
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &Path,
    root: &Path,
    id: &str,
    worker: &str,
    steps: usize,
    max_staged_bytes: u64,
    wall_time_ms: Option<u64>,
) -> Result<OperationRecord, OperationError> {
    if !(1..=1000).contains(&steps) || !(1..=1_073_741_824).contains(&max_staged_bytes) {
        return Err(OperationError::Invalid(
            "integration requires 1..1000 steps and 1..1073741824 staged bytes".into(),
        ));
    }
    let started = std::time::Instant::now();
    let expired = || wall_time_ms.is_some_and(|ms| started.elapsed().as_millis() >= ms as u128);
    let mut db = SqliteKnowledgeRepository::open(store)?;
    let mut item = db
        .operation(id)?
        .ok_or_else(|| OperationError::Invalid("unknown integration operation".into()))?;
    if item.state == OperationState::Completed {
        return Ok(item);
    }
    let request: IntegrationRequest = serde_json::from_value(item.spec.input.clone())?;
    if item.spec.kind != OperationKind::Integration
        || item.spec.views != views(&request)?
        || item.spec.repository_id != request.initial.snapshot.repository_id
    {
        return Err(OperationError::Invalid(
            "integration envelope differs from frozen inputs".into(),
        ));
    }
    for row in std::iter::once(&request.initial).chain(&request.members) {
        if db.retained_snapshot(row.view.as_str())? != row.snapshot {
            return Err(OperationError::Invalid(
                "integration view source differs".into(),
            ));
        }
    }
    let mut progress: Progress = if item.checkpoint.is_null() {
        Progress::default()
    } else {
        serde_json::from_value(item.checkpoint.clone())?
    };
    if progress.next_member >= request.members.len()
        || (progress.next_member > 0 && progress.head.is_none())
    {
        return Err(OperationError::Invalid("invalid integration cursor".into()));
    }
    item = db.claim_operation(id, worker, now_ms()?, 3_600_000)?;
    let generation = item.generation;
    let outcome = (|| {
        if expired() {
            return db.yield_operation(
                id,
                generation,
                now_ms()?,
                "integration wall-time allowance exhausted",
            );
        }
        let mut staged = db.integration_staged_bytes(id)?;
        if staged > max_staged_bytes {
            return Err(OperationError::Budget(
                "integration staging byte allowance exhausted".into(),
            ));
        }
        let initial_parent = parent(&db, &request.initial)?;
        let git = GitIntegration::open(
            root,
            &root.join(".tmp/entmoot-conflicts"),
            initial_parent.len(),
        )?;
        if progress
            .git_version
            .as_ref()
            .is_some_and(|v| v != &git.version)
        {
            return Err(OperationError::Conflict(
                "Git version changed; submit a new integration request".into(),
            ));
        }
        let sequences = db.integration_pack_sequences(id)?;
        if sequences.len() != progress.next_member {
            return Err(OperationError::Invalid(
                "integration cursor and private objects disagree".into(),
            ));
        }
        for sequence in sequences {
            live(&db, id, generation)?;
            if expired() {
                return db.yield_operation(
                    id,
                    generation,
                    now_ms()?,
                    "integration wall-time allowance exhausted during object restoration",
                );
            }
            git.restore(&db.integration_pack(id, sequence)?)?;
        }
        if expired() {
            return db.yield_operation(
                id,
                generation,
                now_ms()?,
                "integration wall-time allowance exhausted before materialization",
            );
        }
        let mut head = if let Some(head) = &progress.head {
            git.check_commit(head)?;
            head.clone()
        } else {
            git.materialize(
                &request.initial.snapshot,
                captured_input(&db, &request.initial)?.as_ref(),
                "initial",
            )?
        };
        progress.git_version = Some(git.version.clone());
        for _ in 0..steps {
            live(&db, id, generation)?;
            if expired() {
                break;
            }
            let ordinal = progress.next_member;
            let member = &request.members[ordinal];
            let captured = captured_input(&db, member)?;
            let revision = git.materialize(
                &member.snapshot,
                captured.as_ref(),
                &format!("member-{ordinal}"),
            )?;
            let (observation, integrated) = git.merge(&head, &revision)?;
            let mut limitations=vec!["one specified order of sequential synthetic merge commits; other orders are untested".into(),"textual Git integration only; combined structural, native, build and runtime behavior remains unqualified".into()];
            if let Some(c) = &captured {
                limitations.extend(c.receipt.limitations.clone());
            }
            if let Some(id) = &request.initial.git_capture {
                limitations.extend(db.git_capture_receipt(id.as_str())?.limitations);
            }
            limitations.sort();
            limitations.dedup();
            let report = IntegrationStepReport {
                schema_version: "entmoot.integration_step.v1".into(),
                implementation: crate::implementation_fingerprint(),
                git_version: git.version.clone(),
                member_index: ordinal,
                member: member.clone(),
                previous_commit: head.clone(),
                integrated_commit: integrated.clone(),
                git: observation,
                limitations,
            };
            let clean = report.git.status == "clean";
            let terminal =
                report.git.status == "conflict" || (clean && ordinal + 1 == request.members.len());
            let mut pack = None;
            if let Some(next) = &integrated {
                if !terminal {
                    let exclude = vec![
                        if ordinal == 0 {
                            initial_parent.clone()
                        } else {
                            head.clone()
                        },
                        parent(&db, member)?,
                    ];
                    let bytes =
                        git.delta(next, &exclude, max_staged_bytes.saturating_sub(staged))?;
                    staged = staged.saturating_add(bytes.len() as u64);
                    pack = Some(bytes);
                }
                progress.next_member += 1;
                head = next.clone();
            }
            progress.head = (progress.next_member > 0).then(|| head.clone());
            let result = terminal.then(|| IntegrationResult {
                status: report.git.status.clone(),
                integrated_members: progress.next_member,
                attempted_members: ordinal + 1,
                untested_members: request.members.len() - ordinal - 1,
                integrated_commit: head.clone(),
            });
            item = db.checkpoint_integration(
                id,
                generation,
                item.checkpoint_sequence + 1,
                now_ms()?,
                serde_json::to_value(&progress)?,
                result.map(serde_json::to_value).transpose()?,
                serde_json::to_value(&report)?,
                pack.as_deref(),
                max_staged_bytes,
            )?;
            if terminal {
                return Ok(item);
            }
            if !clean {
                return db.yield_operation(id,generation,now_ms()?,"Git integration unavailable; inspect the last step and retry after restoring inputs");
            }
        }
        db.yield_operation(
            id,
            generation,
            now_ms()?,
            "integration step or wall-time allowance exhausted",
        )
    })();
    match outcome {
        Ok(value) => Ok(value),
        Err(OperationError::Budget(reason)) => {
            db.yield_operation(id, generation, now_ms()?, &reason)
        }
        Err(error) => {
            let reason: String = error.to_string().chars().take(1000).collect();
            let _ = db.yield_operation(id, generation, now_ms()?, &reason);
            Err(error)
        }
    }
}
