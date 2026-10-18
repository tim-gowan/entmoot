//! Isolated native family execution with durable, source-fenced graph prefixes.
use crate::{collaboration::now_ms, LocalOperations, OperationError, SqliteKnowledgeRepository};
use entmoot_contracts::{
    BindingFamily, BindingOutcome, BindingPlan, ContentDigest, OperationKind, OperationRecord,
    OperationState, ProducerRequest, ProducerTask,
};
pub use entmoot_store::checkpoints::binding::Progress;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn invalid(error: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(error.to_string())
}

pub fn validate_plan(plan: &BindingPlan) -> Result<(), OperationError> {
    let distinct = plan
        .families
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if plan.families.is_empty() || plan.families.len() > 8 || distinct.len() != plan.families.len()
    {
        return Err(invalid("binding requires 1..8 distinct explicit families"));
    }
    plan.rust_profile.validate().map_err(invalid)?;
    if plan.rust_profile.sysroot.is_some() {
        return Err(invalid(
            "binding sysroot is a host setting, not a portable job input",
        ));
    }
    Ok(())
}

/// An explicit host setting is exclusive. The default sibling executable is
/// shipped by the producer-enabled CLI build; execution never installs it.
pub fn producer_path() -> Result<PathBuf, OperationError> {
    let path = if let Some(path) = std::env::var_os("ENTMOOT_PRODUCER") {
        PathBuf::from(path)
    } else {
        std::env::current_exe()
            .map_err(invalid)?
            .with_file_name(if cfg!(windows) {
                "entmoot-producer.exe"
            } else {
                "entmoot-producer"
            })
    };
    if !path.is_file() {
        return Err(OperationError::Unavailable(format!("native worker unavailable at {}; configure ENTMOOT_PRODUCER or build the producer-enabled CLI",path.display())));
    }
    fs::canonicalize(path).map_err(invalid)
}

fn compatible(producer: &Path) -> Result<(), OperationError> {
    let mut output = tempfile::tempfile().map_err(invalid)?;
    let mut command = Command::new(producer);
    command
        .arg("fingerprint")
        .stdout(output.try_clone().map_err(invalid)?)
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let result = crate::process::run(command, Duration::from_secs(10)).map_err(invalid)?;
    if !result.status.is_ok_and(|status| status.success()) {
        return Err(OperationError::Unavailable(
            "native worker fingerprint probe failed".into(),
        ));
    }
    output.seek(SeekFrom::Start(0)).map_err(invalid)?;
    let mut bytes = vec![];
    output
        .take(65536)
        .read_to_end(&mut bytes)
        .map_err(invalid)?;
    let fingerprint: entmoot_contracts::ImplementationFingerprint = serde_json::from_slice(&bytes)?;
    if fingerprint != crate::implementation_fingerprint() {
        return Err(OperationError::Conflict(
            "native worker implementation differs; build matching executables".into(),
        ));
    }
    Ok(())
}

fn execute(
    producer: &Path,
    family: BindingFamily,
    request: &crate::NativeProduceRequest,
    scratch: &Path,
    mut maintain: impl FnMut() -> Result<(), OperationError>,
) -> Result<(), OperationError> {
    let request_path = scratch.join("request.json");
    fs::write(&request_path, serde_json::to_vec(request)?).map_err(invalid)?;
    let mut output = tempfile::tempfile().map_err(invalid)?;
    let mut command = Command::new(producer);
    command
        .args(["execute", "--family", family.as_str(), "--request"])
        .arg(request_path)
        .current_dir(&request.source_root)
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(invalid)?)
        .stderr(output.try_clone().map_err(invalid)?)
        .env("CARGO_TARGET_DIR", scratch.join("cargo-target"))
        .env("GOCACHE", scratch.join("go-cache"));
    // Nested Entmoot adapter runners must stay in this worker's Unix process
    // group. Otherwise killing the outer group leaves nested sidecars alive.
    // The outer runner cleans descendants when the helper exits or is revoked.
    #[cfg(unix)]
    command.env("ENTMOOT_OWNED_PROCESS_GROUP", "1");
    let mut revoked = None;
    let result = crate::process::run_controlled(
        command,
        request.budget.wall + Duration::from_secs(30),
        || {
            if let Err(error) = maintain() {
                revoked = Some(error);
                false
            } else {
                true
            }
        },
    )
    .map_err(invalid)?;
    if let Some(error) = revoked {
        return Err(error);
    }
    if let Some(error) = result.cleanup_error {
        return Err(OperationError::Unavailable(format!(
            "native process cleanup failed: {error}"
        )));
    }
    match result.status {
        Ok(status) if status.success() => Ok(()),
        other => {
            let length = output.metadata().map_err(invalid)?.len();
            output
                .seek(SeekFrom::Start(length.saturating_sub(4096)))
                .map_err(invalid)?;
            let mut bytes = vec![];
            output.take(4096).read_to_end(&mut bytes).map_err(invalid)?;
            Err(OperationError::Unavailable(format!(
                "native {} execution {other:?}: {}",
                family.as_str(),
                String::from_utf8_lossy(&bytes)
            )))
        }
    }
}

/// Families execute in separate process trees. Checkpointed families do not run
/// again; unsuccessful native attempts preserve useful facts and pause the job.
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &Path,
    root: &Path,
    id: &str,
    worker: &str,
    environment: &ContentDigest,
    steps: usize,
    timeout_seconds: u64,
    wall_time_ms: Option<u64>,
) -> Result<OperationRecord, OperationError> {
    if !(1..=1000).contains(&steps) || !(1..=3600).contains(&timeout_seconds) {
        return Err(invalid(
            "binding requires 1..1000 steps and 1..3600 seconds per family",
        ));
    }
    let started = Instant::now();
    let expired = || wall_time_ms.is_some_and(|ms| started.elapsed().as_millis() >= ms as u128);
    let mut db = SqliteKnowledgeRepository::open(store)?;
    let mut item = db
        .operation(id)?
        .ok_or_else(|| invalid("unknown binding operation"))?;
    let request: ProducerRequest = serde_json::from_value(item.spec.input.clone())?;
    let ProducerTask::Binding { view, plan } = &request.task else {
        return Err(invalid("operation is not a binding request"));
    };
    validate_plan(plan)?;
    if item.spec.kind != OperationKind::Binding
        || item.spec.views != vec![view.clone()]
        || item.spec.repository_id != request.source.snapshot.repository_id
    {
        return Err(invalid("binding envelope differs from frozen inputs"));
    }
    if &plan.environment != environment {
        return Err(OperationError::Conflict(
            "host binding environment differs from frozen request".into(),
        ));
    }
    if item.state == OperationState::Completed {
        return Ok(item);
    }
    let root = fs::canonicalize(root).map_err(invalid)?;
    crate::producer_operations::verify_source(&root, &request.source)?;
    // Validate retained input scope before claiming, including low-level submissions.
    if crate::producer_operations::graph_source(&db.retained_graph(view.as_str())?)?
        != request.source
    {
        return Err(invalid("binding input view differs from frozen source"));
    }
    item = db.claim_operation(id, worker, now_ms()?, 60_000)?;
    let generation = item.generation;
    let result = (|| {
        let mut progress = Progress::read(&item, view, plan.families.len())?;
        let mut graph = db.binding_prefix(id, progress.view.as_str())?;
        graph.root = root.to_string_lossy().into_owned();
        let mut producer = None;
        for _ in 0..steps {
            db.renew_operation(id, generation, now_ms()?, 60_000)?;
            if expired() {
                break;
            }
            crate::producer_operations::verify_source(&root, &request.source)?;
            if progress.completed_families == plan.families.len() {
                return db.publish_producer(id, generation, now_ms()?);
            }
            if producer.is_none() {
                let path = producer_path()?;
                compatible(&path)?;
                producer = Some(path);
            }
            let family = plan.families[progress.completed_families];
            let parent = root.join(".tmp/entmoot-binding");
            fs::create_dir_all(&parent).map_err(invalid)?;
            let scratch = tempfile::Builder::new()
                .prefix("attempt-")
                .tempdir_in(&parent)
                .map_err(invalid)?;
            let graph_path = scratch.path().join("graph.json");
            fs::write(&graph_path, serde_json::to_vec(&graph)?).map_err(invalid)?;
            let native = crate::NativeProduceRequest {
                canon: false,
                graph_path,
                source_root: root.clone(),
                output_dir: scratch.path().join("output"),
                budget: crate::ProducerBudget {
                    wall: Duration::from_secs(timeout_seconds),
                    ..Default::default()
                },
                execution_policy: entmoot_contracts::ExecutionPolicy::Safe,
                rust_profile: plan.rust_profile.clone(),
            };
            let mut renewed = Instant::now();
            execute(
                producer
                    .as_deref()
                    .ok_or_else(|| invalid("missing native worker"))?,
                family,
                &native,
                scratch.path(),
                || {
                    let active = db
                        .operation(id)?
                        .ok_or_else(|| invalid("missing binding operation"))?;
                    let now = now_ms()?;
                    if active.generation != generation
                        || active.state != OperationState::Running
                        || active.lease_until_ms.is_none_or(|t| t <= now)
                    {
                        return Err(OperationError::Conflict(
                            "binding worker revoked or expired".into(),
                        ));
                    }
                    if renewed.elapsed() >= Duration::from_secs(5) {
                        db.renew_operation(id, generation, now, 60_000)?;
                        renewed = Instant::now();
                    }
                    Ok(())
                },
            )?;
            db.renew_operation(id, generation, now_ms()?, 60_000)?;
            crate::producer_operations::verify_source(&root, &request.source)?;
            let receipt: entmoot_contracts::NativeReceipt = serde_json::from_slice(
                &fs::read(native.output_dir.join("receipt.json")).map_err(invalid)?,
            )?;
            receipt
                .validate(&crate::implementation_fingerprint())
                .map_err(invalid)?;
            let completed = matches!(
                receipt.semantic_status,
                entmoot_contracts::NativeSemanticStatus::Ok
                    | entmoot_contracts::NativeSemanticStatus::Partial
            );
            let attachment = crate::ingest_artifact_manifest(
                &mut graph,
                &native.output_dir.join("resolver.json.manifest.json"),
                true,
            )
            .map_err(invalid)?;
            crate::producer_operations::verify_source(&root, &request.source)?;
            let outcome = BindingOutcome {
                family,
                environment: environment.clone(),
                receipt,
                attachment,
                completed,
            };
            let next_view = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph)?);
            let source = crate::GixSourceStore::open(&root).map_err(invalid)?;
            item = SqliteKnowledgeRepository::open_view(store, &next_view)?.checkpoint_binding(
                id,
                generation,
                now_ms()?,
                item.checkpoint_sequence,
                &graph,
                Some(&source),
                &outcome,
            )?;
            progress = Progress::read(&item, view, plan.families.len())?;
            if !completed {
                return db.yield_operation(id,generation,now_ms()?,"native family incomplete; saved attributable facts, restore the frozen environment or raise the execution allowance before retry");
            }
        }
        db.yield_operation(
            id,
            generation,
            now_ms()?,
            "binding step or wall-time allowance exhausted",
        )
    })();
    if let Err(error) = &result {
        let reason: String = error.to_string().chars().take(1000).collect();
        let _ = db.yield_operation(id, generation, now_ms()?, &reason);
    }
    result
}
