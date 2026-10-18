//! Restartable local producers. Computation never holds a SQLite write transaction.
//! Durable prepared views remain private to the operation until its registry CAS.
use crate::{LocalOperations, OperationError, SqliteKnowledgeRepository};
use entmoot_contracts::{
    ContentDigest, OperationKind, OperationRecord, OperationSpec, OperationState, ProducerRequest,
    ProducerSource, ProducerTask,
};
use std::path::Path;

fn invalid(error: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(error.to_string())
}

/// Observe bytes and the complete path denominator without parsing or publishing.
pub fn observe_source(root: &Path, repository: &str) -> Result<ProducerSource, OperationError> {
    graph_source(&entmoot_store::observe_source_inventory(root, repository).map_err(invalid)?)
}

pub fn identity(request: &ProducerRequest) -> (OperationKind, Vec<ContentDigest>) {
    match &request.task {
        ProducerTask::Inventory { .. } => (OperationKind::Inventory, vec![]),
        ProducerTask::Canon { view } => (OperationKind::Canon, vec![view.clone()]),
        ProducerTask::Binding { view, .. } => (OperationKind::Binding, vec![view.clone()]),
    }
}

pub fn submit(
    repository: &mut impl LocalOperations,
    request: ProducerRequest,
    key: &str,
    owner: &str,
) -> Result<OperationRecord, OperationError> {
    request.source.snapshot.validate().map_err(invalid)?;
    if let ProducerTask::Binding { plan, .. } = &request.task {
        crate::binding_operations::validate_plan(plan)?;
    }
    for value in [
        &request.publication.workspace_id,
        &request.publication.publisher,
    ]
    .into_iter()
    .chain(request.publication.branch.as_ref())
    {
        if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err(invalid(
                "publication identity must contain 1..4096 nonempty bytes without NUL",
            ));
        }
    }
    if request.publication.expected_generation >= i64::MAX as u64 {
        return Err(invalid("publication generation exhausted"));
    }
    let (kind, views) = identity(&request);
    for view in &views {
        if graph_source(&repository.retained_graph(view.as_str())?)? != request.source {
            return Err(invalid("producer input view does not match frozen source"));
        }
    }
    let spec = OperationSpec {
        repository_id: request.source.snapshot.repository_id.clone(),
        idempotency_key: key.into(),
        owner: owner.into(),
        kind,
        views,
        prerequisites: vec![],
        input: serde_json::to_value(request)?,
        implementation: crate::implementation_fingerprint(),
    };
    repository
        .submit_operations(&[spec], 1024)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("submission returned no operation"))
}

pub fn verify_source(root: &Path, expected: &ProducerSource) -> Result<(), OperationError> {
    if observe_source(root, &expected.snapshot.repository_id)? != *expected {
        return Err(OperationError::Conflict(
            "producer source changed; submit a new request".into(),
        ));
    }
    Ok(())
}

/// Execute bounded steps using the default file batch size. File checkpoints
/// preserve completed inventory and canon outcomes with their cumulative limits.
/// Explicit pause/cancel revokes the worker even while computation is in progress.
pub fn run(
    store: &Path,
    source_root: &Path,
    id: &str,
    worker: &str,
    steps: usize,
) -> Result<OperationRecord, OperationError> {
    run_bounded(
        store,
        source_root,
        id,
        worker,
        steps,
        entmoot_contracts::default_producer_files_per_step(),
        None,
    )
}

/// Cooperative budgets preserve completed file outcomes and never kill a parser.
#[allow(clippy::too_many_arguments)]
pub fn run_bounded(
    store: &Path,
    source_root: &Path,
    id: &str,
    worker: &str,
    steps: usize,
    files_per_step: usize,
    wall_time_ms: Option<u64>,
) -> Result<OperationRecord, OperationError> {
    if !(1..=1000).contains(&steps) || !(1..=4096).contains(&files_per_step) {
        return Err(invalid(
            "producer requires 1..1000 steps and 1..4096 files per step",
        ));
    }
    let started = std::time::Instant::now();
    let mut db = SqliteKnowledgeRepository::open(store)?;
    let item = db
        .operation(id)?
        .ok_or_else(|| invalid("unknown operation"))?;
    let request: ProducerRequest = serde_json::from_value(item.spec.input.clone())?;
    if matches!(request.task, ProducerTask::Binding { .. }) {
        return Err(invalid(
            "binding requires the isolated run-binding worker and host environment assertion",
        ));
    }
    let (kind, views) = identity(&request);
    if item.spec.kind != kind
        || item.spec.views != views
        || item.spec.repository_id != request.source.snapshot.repository_id
    {
        return Err(invalid("producer operation envelope mismatch"));
    }
    if item.state == OperationState::Completed {
        return Ok(item);
    }
    let root = std::fs::canonicalize(source_root).map_err(invalid)?;
    verify_source(&root, &request.source)?;
    let mut item = db.claim_operation(id, worker, crate::collaboration::now_ms()?, 3_600_000)?;
    let generation = item.generation;
    let result = (|| {
        let mut remaining = steps;
        let mut workers = None;
        let mut syntax_graph = None;
        let mut call_index = None;
        let mut completed_cache = None;
        let mut prior_cache = None;
        let canon_input = if let ProducerTask::Canon { view } = &request.task {
            if item.checkpoint.get("view").is_none() {
                let mut graph = db.retained_graph(view.as_str())?;
                graph.root = root.to_string_lossy().into_owned();
                Some((
                    graph,
                    SqliteKnowledgeRepository::open_view_read_only(store, view.as_str())?,
                ))
            } else {
                None
            }
        } else {
            None
        };
        let canon_plan = canon_input
            .as_ref()
            .map(|(graph, previous)| {
                crate::canon::CanonPlan::new(graph, &[], &[], Some(previous)).map_err(invalid)
            })
            .transpose()?;
        if item.checkpoint.is_null() && matches!(request.task, ProducerTask::Inventory { .. }) {
            let manifest = entmoot_core::scan::source_manifest(&root).map_err(invalid)?;
            item =
                db.begin_inventory(id, generation, crate::collaboration::now_ms()?, &manifest)?;
        }
        if item.checkpoint.is_null() {
            if let Some(plan) = &canon_plan {
                item = db.begin_canon(
                    id,
                    generation,
                    crate::collaboration::now_ms()?,
                    &plan.inputs().map_err(invalid)?,
                    plan.cursor(),
                )?;
            }
        }
        while remaining > 0 && item.checkpoint.get("view").is_none() {
            let active = db
                .operation(id)?
                .ok_or_else(|| invalid("unknown operation"))?;
            let now = crate::collaboration::now_ms()?;
            if active.generation != generation
                || active.state != OperationState::Running
                || active.lease_until_ms.is_none_or(|t| now >= t)
            {
                return Err(OperationError::Conflict(
                    "producer worker was revoked or expired".into(),
                ));
            }
            if wall_time_ms.is_some_and(|ms| started.elapsed().as_millis() >= ms as u128) {
                break;
            }
            let mut graph = match &request.task {
                ProducerTask::Binding { .. } => {
                    return Err(invalid("binding task dispatched to structural worker"))
                }
                ProducerTask::Inventory { policy } => {
                    if prior_cache.is_none() {
                        let context = crate::knowledge_db::syntax_cache::SyntaxCache::context(
                            &item.spec.repository_id,
                            policy,
                        )?;
                        prior_cache = Some(db.load_syntax_cache(&context)?);
                    }
                    let prior = prior_cache
                        .as_ref()
                        .ok_or_else(|| invalid("inventory cache context unavailable"))?;
                    let inputs = db.producer_file_batch(id, files_per_step)?;
                    if !inputs.is_empty() {
                        if workers.is_none() {
                            workers =
                                Some(entmoot_core::scan::ScanWorkers::new().map_err(invalid)?);
                        }
                        let executor = workers
                            .as_ref()
                            .ok_or_else(|| invalid("scan workers unavailable"))?;
                        let (outcomes, reused) = executor
                            .scan_cached(
                                &root,
                                &item.spec.repository_id,
                                policy,
                                &inputs,
                                &prior.records,
                            )
                            .map_err(invalid)?;
                        item = db.checkpoint_inventory_batch(
                            id,
                            generation,
                            crate::collaboration::now_ms()?,
                            &outcomes,
                            reused,
                        )?;
                        // Parsing, graph extraction and stitching have separate
                        // admission boundaries. A completed batch is never replayed.
                        remaining -= 1;
                        continue;
                    }
                    if let Some(frontier) = db.inventory_frontier(id, files_per_step)? {
                        use crate::inventory_stitch::{CallPart, Part, Phase, SyntaxPart};
                        let mut reused = 0;
                        let parts = match frontier.phase {
                            Phase::Syntax => frontier
                                .outcomes
                                .iter()
                                .map(|outcome| {
                                    match prior.qualified_parts(
                                        &outcome.input.path,
                                        &outcome.input.content_digest,
                                    ) {
                                        Some(parts) => {
                                            reused += 1;
                                            Ok(Part::Syntax(parts.syntax.clone()))
                                        }
                                        None => SyntaxPart::build(&root, outcome).map(Part::Syntax),
                                    }
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                            Phase::Calls => {
                                if syntax_graph.is_none() {
                                    let graph = db.inventory_syntax_graph(
                                        id,
                                        &root,
                                        &request.source.snapshot,
                                    )?;
                                    call_index = Some(
                                        crate::resolve_edges::StructuralCallIndex::new(&graph),
                                    );
                                    syntax_graph = Some(graph);
                                }
                                let index = call_index
                                    .as_ref()
                                    .ok_or_else(|| invalid("inventory call index unavailable"))?;
                                frontier
                                    .outcomes
                                    .iter()
                                    .map(|outcome| {
                                        let cached = prior
                                            .qualified_parts(
                                                &outcome.input.path,
                                                &outcome.input.content_digest,
                                            )
                                            .filter(|parts| parts.calls.matches(index));
                                        Part::Calls(match cached {
                                            Some(parts) => {
                                                reused += 1;
                                                parts.calls.clone()
                                            }
                                            None => CallPart::build(index, outcome),
                                        })
                                    })
                                    .collect()
                            }
                        };
                        item = db.checkpoint_inventory_frontier(
                            id,
                            generation,
                            crate::collaboration::now_ms()?,
                            &frontier,
                            &parts,
                            reused,
                        )?;
                        remaining -= 1;
                        continue;
                    }
                    let mut graph = match syntax_graph.take() {
                        Some(graph) => graph,
                        None => db.inventory_syntax_graph(id, &root, &request.source.snapshot)?,
                    };
                    db.apply_inventory_calls(id, &mut graph)?;
                    let scan = db.completed_inventory(id, &root)?;
                    let options = crate::TopologyBuildOptions {
                        policy: Some(policy),
                        repository: Some(&item.spec.repository_id),
                    };
                    let prepared =
                        crate::prepare_topology_scan(&root, &options).map_err(invalid)?;
                    crate::finalize_topology_graph(&prepared, &scan, &mut graph)
                        .map_err(invalid)?;
                    completed_cache =
                        Some(db.completed_inventory_cache(id, &item.spec.repository_id, policy)?);
                    remaining -= 1;
                    graph
                }
                ProducerTask::Canon { .. } => {
                    let plan = canon_plan
                        .as_ref()
                        .ok_or_else(|| invalid("canon plan unavailable"))?;
                    let mut cursor = db.canon_cursor(id)?;
                    let outcomes = plan
                        .batch(&root, &mut cursor, files_per_step)
                        .map_err(invalid)?;
                    if !outcomes.is_empty() {
                        item = db.checkpoint_canon_batch(
                            id,
                            generation,
                            crate::collaboration::now_ms()?,
                            &outcomes,
                            cursor,
                        )?;
                    }
                    remaining -= 1;
                    if item.checkpoint["canon"]["completed_files"]
                        != item.checkpoint["canon"]["total_files"]
                    {
                        continue;
                    }
                    let (cursor, outcomes) = db.completed_canon(id)?;
                    let projection = plan.finish(&cursor, outcomes).map_err(invalid)?;
                    let mut graph = canon_input
                        .as_ref()
                        .ok_or_else(|| invalid("canon input unavailable"))?
                        .0
                        .clone();
                    projection.apply(&mut graph);
                    graph
                }
            };
            if graph_source(&graph)? != request.source {
                return Err(OperationError::Conflict(
                    "producer observed a different source during computation".into(),
                ));
            }
            graph.root = root.to_string_lossy().into_owned();
            verify_source(&root, &request.source)?;
            let view = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph)?);
            let source = crate::GixSourceStore::open(&root).map_err(invalid)?;
            let mut staged = SqliteKnowledgeRepository::open_view(store, &view)?;
            item = if let Some(cache) = &completed_cache {
                staged.stage_producer_graph_with_cache(
                    id,
                    generation,
                    crate::collaboration::now_ms()?,
                    &graph,
                    Some(&source),
                    Some(cache),
                )?
            } else {
                staged.stage_producer_graph(
                    id,
                    generation,
                    crate::collaboration::now_ms()?,
                    &graph,
                    Some(&source),
                )?
            };
        }
        if remaining == 0
            || wall_time_ms.is_some_and(|ms| started.elapsed().as_millis() >= ms as u128)
        {
            return db.yield_operation(
                id,
                generation,
                crate::collaboration::now_ms()?,
                "producer slice exhausted; resume from durable checkpoint",
            );
        }
        // Recheck after restart and immediately before making the result discoverable.
        verify_source(&root, &request.source)?;
        db.publish_producer(id, generation, crate::collaboration::now_ms()?)
    })();
    if let Err(error) = &result {
        let _ = db.yield_operation(
            id,
            generation,
            crate::collaboration::now_ms()?,
            &error.to_string(),
        );
    }
    result
}

pub use entmoot_store::graph_source;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TopologyGraph;
    use entmoot_contracts::{OperationControl, ProducerPublication};

    fn prepare_canon(root: &Path, store: &Path) -> (OperationRecord, TopologyGraph) {
        std::fs::write(root.join("b.rs"), "fn second() -> bool { true }\n").unwrap();
        let (_, graph) = prepare(root, store);
        let publication =
            crate::collaboration::register_graph(store, &graph, "input", "agent", None, 0).unwrap();
        let mut db = SqliteKnowledgeRepository::open(store).unwrap();
        let request = ProducerRequest {
            source: graph_source(&graph).unwrap(),
            publication: ProducerPublication {
                workspace_id: "canon".into(),
                branch: None,
                publisher: "agent".into(),
                expected_generation: 0,
            },
            task: ProducerTask::Canon {
                view: publication.view,
            },
        };
        let job = submit(&mut db, request, "canon", "agent").unwrap();
        (job, graph)
    }

    #[test]
    fn canon_child_exit() {
        let Some(root) = std::env::var_os("ENTMOOT_TEST_CANON_CRASH") else {
            return;
        };
        let root = Path::new(&root);
        let store = root.parent().unwrap().join("shared.db");
        let (job, graph) = prepare_canon(root, &store);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let worker = db
            .claim_operation(job.id.as_str(), "crashed", 0, 1)
            .unwrap();
        let plan = crate::canon::CanonPlan::new(&graph, &[], &[], None).unwrap();
        let mut cursor = plan.cursor();
        db.begin_canon(
            job.id.as_str(),
            worker.generation,
            0,
            &plan.inputs().unwrap(),
            cursor.clone(),
        )
        .unwrap();
        let outcomes = plan.batch(root, &mut cursor, 1).unwrap();
        db.checkpoint_canon_batch(job.id.as_str(), worker.generation, 0, &outcomes, cursor)
            .unwrap();
        std::fs::write(root.parent().unwrap().join("operation"), job.id.as_str()).unwrap();
        std::process::exit(0);
    }

    #[test]
    fn canon_progress_and_its_input_pin_survive_process_exit_and_collection() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "producer_operations::tests::canon_child_exit"])
            .env("ENTMOOT_TEST_CANON_CRASH", &root)
            .status()
            .unwrap()
            .success());
        let store = tmp.path().join("shared.db");
        let id = std::fs::read_to_string(tmp.path().join("operation")).unwrap();
        {
            let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
            let item = db.operation(&id).unwrap().unwrap();
            assert_eq!(item.state, OperationState::Running);
            assert_eq!(item.checkpoint["canon"]["completed_files"], 1);
            assert_eq!(db.canon_cursor(&id).unwrap().completed, 1);
        }
        crate::knowledge_db::retention::Maintenance::open(&store)
            .unwrap()
            .collect(&std::collections::BTreeSet::new())
            .unwrap();
        let done = run_bounded(&store, &root, &id, "replacement", 2, 1, None).unwrap();
        assert_eq!(done.state, OperationState::Completed);
        let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
        let graph = db
            .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            graph
                .relation_claims
                .iter()
                .filter(|c| c.predicate == "canon.file")
                .count(),
            2
        );
        assert!(db.canon_cursor(&id).is_err());
    }

    #[test]
    fn canon_rejects_stale_workers_and_rolls_back_mismatched_file_batches() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let store = tmp.path().join("shared.db");
        let (job, graph) = prepare_canon(&root, &store);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let old = db.claim_operation(job.id.as_str(), "old", 0, 1).unwrap();
        let plan = crate::canon::CanonPlan::new(&graph, &[], &[], None).unwrap();
        let mut cursor = plan.cursor();
        db.begin_canon(
            job.id.as_str(),
            old.generation,
            0,
            &plan.inputs().unwrap(),
            cursor.clone(),
        )
        .unwrap();
        let outcomes = plan.batch(&root, &mut cursor, 2).unwrap();
        let new = db.claim_operation(job.id.as_str(), "new", 1, 100).unwrap();
        assert!(db
            .checkpoint_canon_batch(
                job.id.as_str(),
                old.generation,
                1,
                &outcomes,
                cursor.clone()
            )
            .is_err());
        let mut mismatched = outcomes.clone();
        mismatched[1].input.path = "wrong.rs".into();
        assert!(db
            .checkpoint_canon_batch(
                job.id.as_str(),
                new.generation,
                1,
                &mismatched,
                cursor.clone()
            )
            .is_err());
        assert_eq!(db.canon_cursor(job.id.as_str()).unwrap().completed, 0);
        assert_eq!(db.producer_file_batch(job.id.as_str(), 2).unwrap().len(), 2);
        assert!(db.completed_canon(job.id.as_str()).is_err());
        db.checkpoint_canon_batch(job.id.as_str(), new.generation, 1, &outcomes, cursor)
            .unwrap();
        db.control_operation(job.id.as_str(), OperationControl::Cancel)
            .unwrap();
        assert!(db.canon_cursor(job.id.as_str()).is_err());
        assert!(db.completed_canon(job.id.as_str()).is_err());
    }

    #[test]
    fn inventory_child_exit() {
        let Some(root) = std::env::var_os("ENTMOOT_TEST_INVENTORY_CRASH") else {
            return;
        };
        let root = Path::new(&root);
        std::fs::write(root.join("b.rs"), "fn second() {}\n").unwrap();
        std::fs::write(root.join("c.rs"), "fn third() {}\n").unwrap();
        let store = root.parent().unwrap().join("shared.db");
        let (job, _) = prepare(root, &store);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let worker = db
            .claim_operation(job.id.as_str(), "crashed", 0, 1)
            .unwrap();
        let manifest = entmoot_core::scan::source_manifest(root).unwrap();
        db.begin_inventory(job.id.as_str(), worker.generation, 0, &manifest)
            .unwrap();
        let inputs = db.producer_file_batch(job.id.as_str(), 1).unwrap();
        let outcomes = entmoot_core::scan::ScanWorkers::new()
            .unwrap()
            .scan(root, "recovery", &Default::default(), &inputs)
            .unwrap();
        db.checkpoint_inventory_batch(job.id.as_str(), worker.generation, 0, &outcomes, 0)
            .unwrap();
        std::fs::write(root.parent().unwrap().join("operation"), job.id.as_str()).unwrap();
        std::process::exit(0);
    }

    #[test]
    fn file_progress_survives_process_exit_and_storage_collection() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "producer_operations::tests::inventory_child_exit",
            ])
            .env("ENTMOOT_TEST_INVENTORY_CRASH", &root)
            .status()
            .unwrap();
        assert!(status.success());
        let store = tmp.path().join("shared.db");
        let id = std::fs::read_to_string(tmp.path().join("operation")).unwrap();
        {
            let db = SqliteKnowledgeRepository::open_operations_read_only(&store).unwrap();
            let partial = db.operation(&id).unwrap().unwrap();
            assert_eq!(partial.state, OperationState::Running);
            assert_eq!(
                partial.checkpoint,
                serde_json::json!({"inventory":{"completed_files":1,"total_files":3,"reused_files":0}})
            );
        }
        // Collection cannot discard the shared payloads owned by live checkpoints.
        crate::knowledge_db::retention::Maintenance::open(&store)
            .unwrap()
            .collect(&std::collections::BTreeSet::new())
            .unwrap();
        let done = run_bounded(&store, &root, &id, "replacement", 10, 1, None).unwrap();
        assert_eq!(done.state, OperationState::Completed);
        let view = done.result.unwrap()["view"].as_str().unwrap().to_owned();
        let graph = SqliteKnowledgeRepository::open_operations_read_only(&store)
            .unwrap()
            .retained_graph(&view)
            .unwrap();
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|n| n.kind == crate::NodeKind::Callable)
                .count(),
            4
        );
    }

    #[test]
    fn inventory_batches_reject_stale_workers_and_mismatched_members_without_partial_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("b.rs"), "fn second() {}\n").unwrap();
        let store = tmp.path().join("shared.db");
        let (job, _) = prepare(&root, &store);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let old = db.claim_operation(job.id.as_str(), "old", 0, 1).unwrap();
        db.begin_inventory(
            job.id.as_str(),
            old.generation,
            0,
            &entmoot_core::scan::source_manifest(&root).unwrap(),
        )
        .unwrap();
        let inputs = db.producer_file_batch(job.id.as_str(), 2).unwrap();
        let outcomes = entmoot_core::scan::ScanWorkers::new()
            .unwrap()
            .scan(&root, "recovery", &Default::default(), &inputs)
            .unwrap();
        let new = db.claim_operation(job.id.as_str(), "new", 1, 100).unwrap();
        assert!(db
            .checkpoint_inventory_batch(job.id.as_str(), old.generation, 1, &outcomes, 0)
            .is_err());
        let mut bad = outcomes.clone();
        bad[1].input.path = "wrong.rs".into();
        assert!(db
            .checkpoint_inventory_batch(job.id.as_str(), new.generation, 1, &bad, 0)
            .is_err());
        assert_eq!(db.producer_file_batch(job.id.as_str(), 2).unwrap().len(), 2);
        assert_eq!(
            db.operation(job.id.as_str()).unwrap().unwrap().checkpoint["inventory"]
                ["completed_files"],
            0
        );
        assert!(db.completed_inventory(job.id.as_str(), &root).is_err());
        db.checkpoint_inventory_batch(job.id.as_str(), new.generation, 1, &outcomes, 0)
            .unwrap();
        db.control_operation(job.id.as_str(), OperationControl::Cancel)
            .unwrap();
        assert!(db
            .producer_file_batch(job.id.as_str(), 2)
            .unwrap()
            .is_empty());
        assert!(db.completed_inventory(job.id.as_str(), &root).is_err());
    }

    fn prepare(root: &Path, store: &Path) -> (OperationRecord, TopologyGraph) {
        std::fs::write(
            root.join("a.rs"),
            "fn target() {} fn caller() { target(); }\n",
        )
        .unwrap();
        let request = ProducerRequest {
            source: observe_source(root, "recovery").unwrap(),
            publication: ProducerPublication {
                workspace_id: "worktree".into(),
                branch: None,
                publisher: "agent".into(),
                expected_generation: 0,
            },
            task: ProducerTask::Inventory {
                policy: Default::default(),
            },
        };
        let mut db = SqliteKnowledgeRepository::open(store).unwrap();
        let job = submit(&mut db, request, "index", "agent").unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            root,
            crate::TopologyBuildOptions {
                repository: Some("recovery"),
                ..Default::default()
            },
        )
        .unwrap();
        (job, graph)
    }

    #[test]
    fn killed_producer_child() {
        let Some(root) = std::env::var_os("ENTMOOT_TEST_PRODUCER_CRASH") else {
            return;
        };
        let root = Path::new(&root);
        let store = root.parent().unwrap().join("shared.db");
        let (job, graph) = prepare(root, &store);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let claimed = db
            .claim_operation(job.id.as_str(), "crashed", 0, 1)
            .unwrap();
        let view = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph).unwrap());
        SqliteKnowledgeRepository::open_view(&store, &view)
            .unwrap()
            .stage_producer_graph(job.id.as_str(), claimed.generation, 0, &graph, None)
            .unwrap();
        std::fs::write(root.parent().unwrap().join("operation"), job.id.as_str()).unwrap();
        // Skip all destructors and the normal worker yield; the parent reclaims the lease.
        std::process::exit(0);
    }

    #[test]
    fn abrupt_exit_after_staging_recovers_without_recomputing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "producer_operations::tests::killed_producer_child",
            ])
            .env("ENTMOOT_TEST_PRODUCER_CRASH", &root)
            .status()
            .unwrap();
        assert!(status.success());
        let store = tmp.path().join("shared.db");
        let id = std::fs::read_to_string(tmp.path().join("operation")).unwrap();
        let before = SqliteKnowledgeRepository::open_operations_read_only(&store)
            .unwrap()
            .operation(&id)
            .unwrap()
            .unwrap();
        assert_eq!(before.state, OperationState::Running);
        assert_eq!(before.checkpoint_sequence, 1);
        let after = run(&store, &root, &id, "replacement", 1).unwrap();
        assert_eq!(after.state, OperationState::Completed);
        assert_eq!(after.checkpoint, before.checkpoint);
        assert_eq!(after.generation, before.generation + 1);
        assert_eq!(after.checkpoint_sequence, 2);
    }

    #[test]
    fn expired_and_cancelled_workers_cannot_stage_or_publish() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let store = tmp.path().join("shared.db");
        let (job, graph) = prepare(&root, &store);
        let view = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph).unwrap());
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let old = db.claim_operation(job.id.as_str(), "old", 0, 1).unwrap();
        let mut stage = SqliteKnowledgeRepository::open_view(&store, &view).unwrap();
        assert!(stage
            .stage_producer_graph(job.id.as_str(), old.generation, 1, &graph, None)
            .is_err());
        let new = db.claim_operation(job.id.as_str(), "new", 1, 100).unwrap();
        assert!(stage
            .stage_producer_graph(job.id.as_str(), old.generation, 1, &graph, None)
            .is_err());
        stage
            .stage_producer_graph(job.id.as_str(), new.generation, 1, &graph, None)
            .unwrap();
        assert!(db
            .publish_producer(job.id.as_str(), old.generation, 2)
            .is_err());
        db.control_operation(job.id.as_str(), OperationControl::Cancel)
            .unwrap();
        assert!(db
            .publish_producer(job.id.as_str(), new.generation, 2)
            .is_err());
        assert!(db.workspaces("recovery").unwrap().is_empty());
        assert!(db.retained_graph(&view).is_err());
    }
}
