//! Transport-independent comparisons over frozen repository states.
use crate::{compare_revision_impact, ImpactError, TopologyGraph};
use crate::{LocalOperations, OperationError, SqliteKnowledgeRepository};
use entmoot_contracts::{BranchConflictReport, ConflictFinding, ConflictFindingKind};
use entmoot_contracts::{
    ConflictBatchRequest, ContentDigest, OperationKind, OperationRecord, OperationSpec,
    OperationState, WorkspacePublication,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn read(
    store: &Path,
    request: entmoot_contracts::CollaborationRead,
) -> Result<serde_json::Value, OperationError> {
    read_with_source(store, request, None)
}

pub fn read_with_source(
    store: &Path,
    request: entmoot_contracts::CollaborationRead,
    source_root: Option<&Path>,
) -> Result<serde_json::Value, OperationError> {
    use entmoot_contracts::CollaborationRead;
    if let CollaborationRead::Source { repository_id } = &request {
        let root = source_root.ok_or_else(|| {
            OperationError::Invalid(
                "source observation requires an explicit local source root".into(),
            )
        })?;
        return Ok(serde_json::to_value(
            crate::producer_operations::observe_source(root, repository_id)?,
        )?);
    }
    let db = SqliteKnowledgeRepository::open_operations_read_only(store)?;
    Ok(match request {
        CollaborationRead::Scheduler { repository_id } => {
            serde_json::to_value(db.scheduler_status(&repository_id, now_ms()?)?)?
        }
        CollaborationRead::RevalidationRecord { id, record } => serde_json::to_value(
            crate::revalidation_operations::record_state(store, id.as_str(), record)?,
        )?,
        CollaborationRead::CaptureIntegration {
            repository_id,
            workspace,
            members,
        } => {
            let rows: BTreeMap<_, _> = db
                .workspaces(&repository_id)?
                .into_iter()
                .filter(|r| r.active)
                .map(|r| (r.workspace_id.clone(), r))
                .collect();
            let lookup = |id: &str| {
                rows.get(id)
                    .cloned()
                    .ok_or_else(|| OperationError::Invalid(format!("workspace unavailable: {id}")))
            };
            let request = entmoot_contracts::IntegrationRequest {
                initial: lookup(&workspace)?,
                members: members
                    .iter()
                    .map(|id| lookup(id))
                    .collect::<Result<_, _>>()?,
            };
            crate::integration_operations::views(&request)?;
            serde_json::to_value(request)?
        }
        CollaborationRead::Source { .. } => {
            return Err(OperationError::Invalid(
                "source observation not dispatched".into(),
            ))
        }
        CollaborationRead::Capture {
            git_merge,
            repository_id,
            base_workspace,
            workspace,
            peers,
        } => {
            let rows: Vec<_> = db
                .workspaces(&repository_id)?
                .into_iter()
                .filter(|r| r.active)
                .collect();
            let by_id: BTreeMap<_, _> = rows.iter().map(|r| (r.workspace_id.as_str(), r)).collect();
            let find =
                |id: &str| {
                    by_id.get(id).copied().cloned().ok_or_else(|| {
                        OperationError::Invalid(format!("workspace unavailable: {id}"))
                    })
                };
            let base = find(&base_workspace)?;
            let left = find(&workspace)?;
            let peers = if peers.is_empty() {
                rows.iter()
                    .filter(|r| r.workspace_id != workspace)
                    .cloned()
                    .collect()
            } else {
                peers
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|id| find(id))
                    .collect::<Result<Vec<_>, _>>()?
            };
            serde_json::to_value(ConflictBatchRequest {
                git_merge,
                base_view: base.view,
                left,
                peers,
            })?
        }
        CollaborationRead::Workspaces { repository_id } => {
            serde_json::to_value(db.workspaces(&repository_id)?)?
        }
        CollaborationRead::Outputs {
            id,
            after_sequence,
            limit,
        } => serde_json::to_value(db.operation_outputs(id.as_str(), after_sequence, limit)?)?,
        CollaborationRead::Status { id } => serde_json::to_value(
            db.operation(id.as_str())?
                .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?,
        )?,
        CollaborationRead::Compare { base, left, right } => serde_json::to_value(
            compare_branch_conflicts(
                &db.retained_graph(base.as_str())?,
                &db.retained_graph(left.as_str())?,
                &db.retained_graph(right.as_str())?,
            )
            .map_err(|e| OperationError::Invalid(e.to_string()))?,
        )?,
    })
}

pub fn write(
    store: &Path,
    request: entmoot_contracts::CollaborationWrite,
) -> Result<serde_json::Value, OperationError> {
    write_with_source(store, request, None)
}

/// Source locators belong to local composition, never the portable operation record.
pub fn write_with_source(
    store: &Path,
    request: entmoot_contracts::CollaborationWrite,
    source_root: Option<&Path>,
) -> Result<serde_json::Value, OperationError> {
    use entmoot_contracts::CollaborationWrite;
    let mut db = SqliteKnowledgeRepository::open(store)?;
    Ok(match request {
        CollaborationWrite::ConfigureScheduler {
            repository_id,
            expected_generation,
            policy,
        } => serde_json::to_value(db.configure_scheduler(
            &repository_id,
            expected_generation,
            policy,
        )?)?,
        CollaborationWrite::RunBinding {
            id,
            worker,
            environment,
            steps,
            timeout_seconds,
            wall_time_ms,
        } => {
            let root = source_root.ok_or_else(|| {
                OperationError::Unavailable(
                    "binding requires an explicit isolated source root".into(),
                )
            })?;
            serde_json::to_value(crate::binding_operations::run(
                store,
                root,
                id.as_str(),
                &worker,
                &environment,
                steps,
                timeout_seconds,
                wall_time_ms,
            )?)?
        }
        CollaborationWrite::SubmitRevalidation {
            request,
            key,
            owner,
        } => serde_json::to_value(crate::revalidation_operations::submit(
            &mut db, request, &key, &owner,
        )?)?,
        CollaborationWrite::RunRevalidation {
            id,
            worker,
            steps,
            max_records,
            wall_time_ms,
        } => serde_json::to_value(crate::revalidation_operations::run(
            store,
            id.as_str(),
            &worker,
            steps,
            max_records,
            wall_time_ms,
        )?)?,
        CollaborationWrite::SubmitIntegration {
            request,
            key,
            owner,
        } => serde_json::to_value(crate::integration_operations::submit(
            &mut db, request, &key, &owner,
        )?)?,
        CollaborationWrite::RunIntegration {
            id,
            worker,
            steps,
            max_staged_bytes,
            wall_time_ms,
        } => {
            let root = source_root.ok_or_else(|| {
                OperationError::Unavailable(
                    "integration execution requires an explicit local Git source root".into(),
                )
            })?;
            serde_json::to_value(crate::integration_operations::run(
                store,
                root,
                id.as_str(),
                &worker,
                steps,
                max_staged_bytes,
                wall_time_ms,
            )?)?
        }
        CollaborationWrite::CaptureGit {
            workspace_id,
            expected_generation,
            publisher,
            max_files,
            max_bytes,
        } => {
            let root = source_root.ok_or_else(|| {
                OperationError::Invalid("Git capture requires an explicit local source root".into())
            })?;
            serde_json::to_value(crate::collaboration_git::capture_workspace(
                store,
                root,
                &workspace_id,
                expected_generation,
                &publisher,
                max_files,
                max_bytes,
            )?)?
        }
        CollaborationWrite::SubmitProducer {
            request,
            key,
            owner,
        } => serde_json::to_value(crate::producer_operations::submit(
            &mut db, request, &key, &owner,
        )?)?,
        CollaborationWrite::RunProducer {
            id,
            worker,
            steps,
            files_per_step,
            wall_time_ms,
        } => {
            let root = source_root.ok_or_else(|| {
                OperationError::Invalid(
                    "producer execution requires an explicit local source root".into(),
                )
            })?;
            serde_json::to_value(crate::producer_operations::run_bounded(
                store,
                root,
                id.as_str(),
                &worker,
                steps,
                files_per_step,
                wall_time_ms,
            )?)?
        }
        CollaborationWrite::Retire {
            workspace_id,
            expected_generation,
        } => serde_json::to_value(db.retire_workspace(&workspace_id, expected_generation)?)?,
        CollaborationWrite::Publish {
            publication,
            expected_generation,
        } => serde_json::to_value(db.publish_workspace(publication, expected_generation)?)?,
        CollaborationWrite::Submit {
            request,
            key,
            owner,
        } => serde_json::to_value(submit_conflicts(&mut db, request, &key, &owner)?)?,
        CollaborationWrite::Control { id, control } => {
            serde_json::to_value(db.control_operation(id.as_str(), control)?)?
        }
        CollaborationWrite::Run { id, worker, pairs } => serde_json::to_value(
            run_conflicts_with_git(&mut db, id.as_str(), &worker, pairs, source_root)?,
        )?,
    })
}

/// Import a validated immutable graph, then atomically advance its workspace
/// reference. Incomplete/orphan staging is never discoverable through the registry.
pub fn register_graph(
    store: &Path,
    graph: &TopologyGraph,
    workspace: &str,
    publisher: &str,
    branch: Option<String>,
    expected: u64,
) -> Result<WorkspacePublication, OperationError> {
    graph
        .validate()
        .map_err(|e| OperationError::Invalid(e.to_string()))?;
    let view = ContentDigest::from_trusted(graph.content_digest()?);
    let mut repository = SqliteKnowledgeRepository::open_view(store, view.as_str())?;
    repository.import_publication(
        graph,
        WorkspacePublication {
            workspace_id: workspace.into(),
            branch,
            snapshot: graph.snapshot.clone(),
            view,
            generation: 0,
            publisher: publisher.into(),
            active: true,
            git_capture: None,
        },
        expected,
    )
}

/// Frozen branch selections are part of the operation input, so later registry
/// advances never silently change the comparison on resume.
pub fn submit_conflicts(
    repository: &mut impl LocalOperations,
    request: ConflictBatchRequest,
    key: &str,
    owner: &str,
) -> Result<OperationRecord, OperationError> {
    let repo = &request.left.snapshot.repository_id;
    for row in std::iter::once(&request.left).chain(&request.peers) {
        if let Some(id) = &row.git_capture {
            check_capture_scope(&repository.git_capture_receipt(id.as_str())?, row)?;
        }
    }
    let views = conflict_views(&request)?;
    let spec = OperationSpec {
        repository_id: repo.clone(),
        idempotency_key: key.into(),
        owner: owner.into(),
        kind: OperationKind::Conflicts,
        views,
        prerequisites: vec![],
        input: serde_json::to_value(request)?,
        implementation: crate::implementation_fingerprint(),
    };
    repository
        .submit_operations(&[spec], 1000)?
        .pop()
        .ok_or_else(|| OperationError::Invalid("missing submitted operation".into()))
}

fn conflict_views(request: &ConflictBatchRequest) -> Result<Vec<ContentDigest>, OperationError> {
    if request
        .peers
        .iter()
        .any(|p| p.snapshot.repository_id != request.left.snapshot.repository_id)
    {
        return Err(OperationError::Invalid(
            "cross-repository conflict request".into(),
        ));
    }
    let mut views = vec![request.base_view.clone(), request.left.view.clone()];
    views.extend(request.peers.iter().map(|p| p.view.clone()));
    views.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    views.dedup();
    Ok(views)
}

/// Execute a bounded conflict slice without holding a database write transaction
/// while comparing graphs. Each completed pair and cursor commit together.
pub fn run_conflicts(
    repository: &mut impl LocalOperations,
    id: &str,
    worker: &str,
    pairs: usize,
) -> Result<OperationRecord, OperationError> {
    run_conflicts_with_git(repository, id, worker, pairs, None)
}

pub fn check_capture_scope(
    receipt: &entmoot_contracts::GitTreeCapture,
    row: &WorkspacePublication,
) -> Result<(), OperationError> {
    if receipt.view != row.view || receipt.source.snapshot != row.snapshot {
        return Err(OperationError::Invalid(
            "Git capture differs from frozen workspace".into(),
        ));
    }
    Ok(())
}

pub fn captured_input(
    repository: &impl LocalOperations,
    row: &WorkspacePublication,
) -> Result<Option<crate::collaboration_git::RetainedGitCapture>, OperationError> {
    row.git_capture
        .as_ref()
        .map(|id| {
            let captured = repository.git_capture(id.as_str())?;
            check_capture_scope(&captured.receipt, row)?;
            Ok(captured)
        })
        .transpose()
}

pub fn run_conflicts_with_git(
    repository: &mut impl LocalOperations,
    id: &str,
    worker: &str,
    pairs: usize,
    git_root: Option<&Path>,
) -> Result<OperationRecord, OperationError> {
    if !(1..=1000).contains(&pairs) {
        return Err(OperationError::Invalid(
            "pair slice requires 1..1000".into(),
        ));
    }
    let mut item = repository
        .operation(id)?
        .ok_or_else(|| OperationError::Invalid("unknown operation".into()))?;
    if item.state == OperationState::Completed {
        return Ok(item);
    }
    if item.spec.kind != OperationKind::Conflicts {
        return Err(OperationError::Invalid(
            "worker requires conflicts operation".into(),
        ));
    }
    let request: ConflictBatchRequest = serde_json::from_value(item.spec.input.clone())?;
    if item.spec.views != conflict_views(&request)?
        || item.spec.repository_id != request.left.snapshot.repository_id
    {
        return Err(OperationError::Invalid(
            "conflict operation envelope does not match frozen inputs".into(),
        ));
    }
    if request.git_merge && git_root.is_none() {
        return Err(OperationError::Unavailable(
            "frozen Git comparison requires a local object source".into(),
        ));
    }
    let base = repository.retained_graph(request.base_view.as_str())?;
    let left = repository.retained_graph(request.left.view.as_str())?;
    if left.snapshot != request.left.snapshot {
        return Err(OperationError::Conflict("left snapshot changed".into()));
    }
    item = repository.claim_operation(id, worker, now_ms()?, 3_600_000)?;
    let generation = item.generation;
    let outcome = (|| {
        let mut completed: usize = if item.checkpoint.is_null() {
            0
        } else {
            serde_json::from_value(item.checkpoint.clone())?
        };
        if completed > request.peers.len() {
            return Err(OperationError::Invalid(
                "checkpoint exceeds peer set".into(),
            ));
        }
        for peer in request.peers.iter().skip(completed).take(pairs) {
            let right = repository.retained_graph(peer.view.as_str())?;
            if right.snapshot != peer.snapshot {
                return Err(OperationError::Conflict("peer snapshot changed".into()));
            }
            let mut report = compare_branch_conflicts(&base, &left, &right)
                .map_err(|e| OperationError::Invalid(e.to_string()))?;
            if request.git_merge {
                if let Some(root) = git_root {
                    let lc = captured_input(repository, &request.left)?;
                    let rc = captured_input(repository, peer)?;
                    let observation = crate::collaboration_git::observe_captured_merge(
                        root,
                        &root.join(".tmp/entmoot-conflicts"),
                        &left.snapshot,
                        &right.snapshot,
                        lc.as_ref(),
                        rc.as_ref(),
                    )
                    .unwrap_or_else(|e| {
                        entmoot_contracts::GitMergeObservation {
                            merge_bases: vec![],
                            status: "unavailable".into(),
                            details: e.to_string(),
                        }
                    });
                    if observation.merge_bases != vec![base.snapshot.revision.clone()] {
                        report.limitations.push("supplied structural baseline is not the unique observed Git merge base".into());
                    }
                    report.git = Some(observation);
                }
            }
            completed += 1;
            let result = (completed == request.peers.len())
                .then(|| serde_json::json!({"completed_pairs": completed}));
            item = repository.checkpoint_operation_output(
                id,
                generation,
                item.checkpoint_sequence + 1,
                now_ms()?,
                serde_json::json!(completed),
                result,
                serde_json::to_value(report)?,
            )?;
        }
        if request.peers.is_empty() {
            item = repository.checkpoint_operation(
                id,
                generation,
                item.checkpoint_sequence + 1,
                now_ms()?,
                serde_json::json!(0),
                Some(serde_json::json!({"completed_pairs": 0})),
            )?;
        }
        if item.state != OperationState::Completed {
            item = repository.yield_operation(id, generation, now_ms()?, "pair slice exhausted")?;
        }
        Ok(item)
    })();
    if let Err(error) = &outcome {
        // Never pause a newer attempt when this worker loses its fence.
        let reason: String = error.to_string().chars().take(1000).collect();
        let _ = repository.yield_operation(id, generation, now_ms()?, &reason);
    }
    outcome
}

/// Compare both branches to a supplied common baseline. Establishing Git ancestry
/// is separate; callers must not describe an arbitrary baseline as a merge base.
pub fn compare_branch_conflicts(
    base: &TopologyGraph,
    left: &TopologyGraph,
    right: &TopologyGraph,
) -> Result<BranchConflictReport, ImpactError> {
    let l = compare_revision_impact(base, left)?;
    let r = compare_revision_impact(base, right)?;
    let mut report = BranchConflictReport {
        schema_version: "entmoot.branch_conflicts.v1".into(), implementation: crate::implementation_fingerprint(),
        base: base.snapshot.clone(), left: left.snapshot.clone(), right: right.snapshot.clone(), findings: vec![], git: None,
        limitations: vec![
            "supplied baseline; Git ancestry and textual merge status are separate observations".into(),
            "structural overlap and represented direct dependencies are review signals, not semantic incompatibility verdicts".into(),
            "absence of findings does not establish compatibility; dynamic, implicit and unrepresented dependencies remain unknown".into(),
        ],
    };
    if base.policy_ids != left.policy_ids || base.policy_ids != right.policy_ids {
        report.findings.push(ConflictFinding {
            kind: ConflictFindingKind::IncompatibleEvidence,
            path: None,
            subjects: vec![],
            explanation: "structural policies differ; structural comparison is not qualified"
                .into(),
        });
        return Ok(report);
    }
    let left_files: BTreeMap<_, _> = l.files.iter().map(|f| (f.path.as_str(), f)).collect();
    for file in &r.files {
        if left_files.contains_key(file.path.as_str()) {
            report.findings.push(ConflictFinding { kind: ConflictFindingKind::StructuralOverlap, path: Some(file.path.clone()), subjects: vec![],
                explanation: "both snapshots changed this path relative to the supplied baseline; identical changes and disjoint edits are still possible".into() });
        }
    }
    let left_changed: BTreeSet<_> = l
        .callables
        .iter()
        .map(|c| c.occurrence.address_id.as_str())
        .chain(l.declarations.iter().map(|d| d.address_id.as_str()))
        .collect();
    let right_changed: BTreeSet<_> = r
        .callables
        .iter()
        .map(|c| c.occurrence.address_id.as_str())
        .chain(r.declarations.iter().map(|d| d.address_id.as_str()))
        .collect();
    for address in left_changed.intersection(&right_changed) {
        report.findings.push(ConflictFinding {
            kind: ConflictFindingKind::StructuralOverlap,
            path: None,
            subjects: vec![(*address).into()],
            explanation: "both snapshots changed the same represented occurrence".into(),
        });
    }
    let mut relationships = BTreeSet::new();
    for graph in [base, left, right] {
        let addresses: BTreeMap<_, _> = graph
            .nodes
            .iter()
            .filter_map(|n| n.address_id.as_deref().map(|a| (n.id.as_str(), a)))
            .collect();
        for edge in &graph.edges {
            if !matches!(
                edge.kind,
                crate::EdgeKind::Calls | crate::EdgeKind::Imports | crate::EdgeKind::BindsCallable
            ) {
                continue;
            }
            if let (Some(a), Some(b)) = (
                addresses.get(edge.from.as_str()),
                addresses.get(edge.to.as_str()),
            ) {
                if (left_changed.contains(a) && right_changed.contains(b))
                    || (right_changed.contains(a) && left_changed.contains(b))
                {
                    relationships.insert(((*a).to_owned(), (*b).to_owned()));
                }
            }
        }
    }
    for (from, to) in relationships {
        report.findings.push(ConflictFinding {
            kind: ConflictFindingKind::PotentialInteraction,
            path: None,
            subjects: vec![from, to],
            explanation:
                "a represented direct dependency connects occurrences changed on opposite branches"
                    .into(),
        });
    }
    report
        .limitations
        .extend(l.breaking_change_signals.limitations);
    report
        .limitations
        .extend(r.breaking_change_signals.limitations);
    report.limitations.sort();
    report.limitations.dedup();
    Ok(report)
}

pub use entmoot_store::clock::now_ms;
