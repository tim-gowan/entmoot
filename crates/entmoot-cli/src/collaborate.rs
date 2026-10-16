//! Local composition; portable contracts and execution live in the engine.
use anyhow::{Context, Result};
use clap::Subcommand;
use entmoot_engine::{LocalOperations, SqliteKnowledgeRepository};
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Read shared admission policy and bounded live waiting/active observations.
    Scheduler {
        #[arg(long)]
        repository: String,
    },
    /// Replace repository admission policy by generation; policy is SchedulerPolicy JSON.
    ConfigureScheduler {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        expected_generation: u64,
        #[arg(long)]
        policy: PathBuf,
    },
    /// Execute native families in a host-isolated environment matching the request.
    RunBinding {
        #[arg(long)]
        id: String,
        #[arg(long)]
        worker: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long, default_value_t = 10)]
        steps: usize,
        #[arg(long, default_value_t = 3600)]
        timeout_seconds: u64,
        #[arg(long)]
        wall_time_ms: Option<u64>,
    },
    /// Freeze conditional policies over two retained repository views.
    SubmitRevalidation {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        owner: String,
    },
    /// Continue policy checkpoints; publish the ledger only when all finish.
    RunRevalidation {
        #[arg(long)]
        id: String,
        #[arg(long)]
        worker: String,
        #[arg(long, default_value_t = 10)]
        steps: usize,
        #[arg(long, default_value_t = 100_000)]
        max_records: usize,
        #[arg(long)]
        wall_time_ms: Option<u64>,
    },
    /// Read applicability or historical evidence state in a completed ledger.
    RevalidationRecord {
        #[arg(long)]
        id: String,
        #[arg(long)]
        collection: String,
        #[arg(long)]
        record_id: String,
    },
    /// Freeze one explicit member order; member options retain their given order.
    CaptureIntegration {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        workspace: String,
        #[arg(long = "member", required = true)]
        members: Vec<String>,
    },
    SubmitIntegration {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        owner: String,
    },
    /// Continue a private ordered integration; never updates source refs or index.
    RunIntegration {
        #[arg(long)]
        id: String,
        #[arg(long)]
        worker: String,
        #[arg(long)]
        git_root: PathBuf,
        #[arg(long, default_value_t = 10)]
        steps: usize,
        #[arg(long, default_value_t = 67_108_864)]
        max_staged_bytes: u64,
        #[arg(long)]
        wall_time_ms: Option<u64>,
    },
    /// Retain raw working-tree Git inputs and advance the workspace generation.
    CaptureGit {
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        expected_generation: u64,
        #[arg(long)]
        publisher: String,
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long, default_value_t = 10_000)]
        max_files: usize,
        #[arg(long, default_value_t = 67_108_864)]
        max_bytes: u64,
    },
    /// Capture complete source identity without parsing or submitting work.
    Source {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        repository: String,
    },
    /// Submit a portable ProducerRequest JSON (inventory, structural canon or binding).
    SubmitProducer {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        owner: String,
    },
    /// Prepare a sealed producer result, then atomically publish its registry update.
    RunProducer {
        #[arg(long)]
        id: String,
        #[arg(long)]
        worker: String,
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long, default_value_t = 5)]
        steps: usize,
        #[arg(long, default_value_t = 128)]
        files_per_step: usize,
        /// Stop admitting work after this duration, preserving completed checkpoints.
        #[arg(long)]
        wall_time_ms: Option<u64>,
    },
    /// Release a workspace's retention pin while preserving its generation fence.
    Retire {
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        expected_generation: u64,
    },
    /// Capture a frozen request for selected peers (empty means all other workspaces).
    Capture {
        #[arg(long)]
        git_merge: bool,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        base_workspace: String,
        #[arg(long)]
        workspace: String,
        #[arg(long = "peer")]
        peers: Vec<String>,
    },
    /// Publish an existing validated graph as one local workspace state.
    Register {
        #[arg(long)]
        graph: PathBuf,
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        publisher: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long, default_value_t = 0)]
        expected_generation: u64,
    },
    /// List registered snapshots; never inventory source or run producers.
    List {
        #[arg(long)]
        repository: String,
    },
    /// Submit a frozen conflict batch described by ConflictBatchRequest JSON.
    Submit {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        owner: String,
    },
    /// Execute one durable slice; pauses with progress when the slice is exhausted.
    Run {
        /// Parent Git objects for batches submitted with Git observation enabled.
        #[arg(long)]
        git_root: Option<PathBuf>,
        #[arg(long)]
        id: String,
        #[arg(long)]
        worker: String,
        #[arg(long, default_value_t = 10)]
        pairs: usize,
    },
    /// Read a bounded page of committed pair reports from a running or finished job.
    Outputs {
        #[arg(long)]
        id: String,
        #[arg(long, default_value_t = 0)]
        after_sequence: u64,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Status {
        #[arg(long)]
        id: String,
    },
    Pause {
        #[arg(long)]
        id: String,
    },
    Resume {
        #[arg(long)]
        id: String,
    },
    Cancel {
        #[arg(long)]
        id: String,
    },
    /// Compare three supplied immutable graphs without publishing or producing.
    Compare {
        #[arg(long)]
        base: PathBuf,
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        /// Optional local Git object source for isolated committed-snapshot merge checks.
        #[arg(long)]
        git_root: Option<PathBuf>,
    },
}

fn graph(path: &Path) -> Result<entmoot_engine::TopologyGraph> {
    entmoot_engine::load_graph_json(path)
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .with_context(|| format!("load graph {}", path.display()))
}

pub fn run(store: &Path, command: Command) -> Result<()> {
    use entmoot_contracts::OperationControl;
    use entmoot_engine::collaboration as engine;
    let value = match command {
        Command::Scheduler { repository } => engine::read(
            store,
            entmoot_contracts::CollaborationRead::Scheduler {
                repository_id: repository,
            },
        )?,
        Command::ConfigureScheduler {
            repository,
            expected_generation,
            policy,
        } => engine::write(
            store,
            entmoot_contracts::CollaborationWrite::ConfigureScheduler {
                repository_id: repository,
                expected_generation,
                policy: serde_json::from_slice(
                    &std::fs::read(&policy)
                        .with_context(|| format!("read policy {}", policy.display()))?,
                )?,
            },
        )?,
        Command::RunBinding {
            id,
            worker,
            environment,
            source_root,
            steps,
            timeout_seconds,
            wall_time_ms,
        } => engine::write_with_source(
            store,
            entmoot_contracts::CollaborationWrite::RunBinding {
                id: entmoot_contracts::ContentDigest::parse(id)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                environment: entmoot_contracts::ContentDigest::parse(environment)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                worker,
                steps,
                timeout_seconds,
                wall_time_ms,
            },
            Some(&source_root),
        )?,
        Command::SubmitRevalidation {
            request,
            key,
            owner,
        } => engine::write(
            store,
            entmoot_contracts::CollaborationWrite::SubmitRevalidation {
                request: serde_json::from_slice(&std::fs::read(request)?)?,
                key,
                owner,
            },
        )?,
        Command::RunRevalidation {
            id,
            worker,
            steps,
            max_records,
            wall_time_ms,
        } => engine::write(
            store,
            entmoot_contracts::CollaborationWrite::RunRevalidation {
                id: entmoot_contracts::ContentDigest::parse(id)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                worker,
                steps,
                max_records,
                wall_time_ms,
            },
        )?,
        Command::RevalidationRecord {
            id,
            collection,
            record_id,
        } => engine::read(
            store,
            entmoot_contracts::CollaborationRead::RevalidationRecord {
                id: entmoot_contracts::ContentDigest::parse(id)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                record: entmoot_contracts::KnowledgeRecordRef {
                    collection,
                    record_id,
                },
            },
        )?,
        Command::CaptureIntegration {
            repository,
            workspace,
            members,
        } => engine::read(
            store,
            entmoot_contracts::CollaborationRead::CaptureIntegration {
                repository_id: repository,
                workspace,
                members,
            },
        )?,
        Command::SubmitIntegration {
            request,
            key,
            owner,
        } => engine::write(
            store,
            entmoot_contracts::CollaborationWrite::SubmitIntegration {
                request: serde_json::from_slice(&std::fs::read(request)?)?,
                key,
                owner,
            },
        )?,
        Command::RunIntegration {
            id,
            worker,
            git_root,
            steps,
            max_staged_bytes,
            wall_time_ms,
        } => engine::write_with_source(
            store,
            entmoot_contracts::CollaborationWrite::RunIntegration {
                id: entmoot_contracts::ContentDigest::parse(id)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                worker,
                steps,
                max_staged_bytes,
                wall_time_ms,
            },
            Some(&git_root),
        )?,
        Command::CaptureGit {
            workspace,
            expected_generation,
            publisher,
            source_root,
            max_files,
            max_bytes,
        } => engine::write_with_source(
            store,
            entmoot_contracts::CollaborationWrite::CaptureGit {
                workspace_id: workspace,
                expected_generation,
                publisher,
                max_files,
                max_bytes,
            },
            Some(&source_root),
        )?,
        Command::Source {
            source_root,
            repository,
        } => serde_json::to_value(entmoot_engine::producer_operations::observe_source(
            &source_root,
            &repository,
        )?)?,
        Command::SubmitProducer {
            request,
            key,
            owner,
        } => engine::write(
            store,
            entmoot_contracts::CollaborationWrite::SubmitProducer {
                request: serde_json::from_slice(&std::fs::read(request)?)?,
                key,
                owner,
            },
        )?,
        Command::RunProducer {
            id,
            worker,
            source_root,
            steps,
            files_per_step,
            wall_time_ms,
        } => engine::write_with_source(
            store,
            entmoot_contracts::CollaborationWrite::RunProducer {
                id: entmoot_contracts::ContentDigest::parse(id)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                worker,
                steps,
                files_per_step,
                wall_time_ms,
            },
            Some(&source_root),
        )?,
        Command::Retire {
            workspace,
            expected_generation,
        } => engine::write(
            store,
            entmoot_contracts::CollaborationWrite::Retire {
                workspace_id: workspace,
                expected_generation,
            },
        )?,
        Command::Capture {
            git_merge,
            repository,
            base_workspace,
            workspace,
            peers,
        } => engine::read(
            store,
            entmoot_contracts::CollaborationRead::Capture {
                git_merge,
                repository_id: repository,
                base_workspace,
                workspace,
                peers,
            },
        )?,
        Command::Compare {
            base,
            left,
            right,
            git_root,
        } => {
            let mut report =
                engine::compare_branch_conflicts(&graph(&base)?, &graph(&left)?, &graph(&right)?)?;
            if let Some(root) = git_root {
                let observation = entmoot_engine::collaboration_git::observe_merge(
                    &root,
                    &root.join(".tmp/entmoot-conflicts"),
                    &report.left,
                    &report.right,
                )?;
                if observation.merge_bases != vec![report.base.revision.clone()] {
                    report.limitations.push(
                        "supplied structural baseline is not the unique observed Git merge base"
                            .into(),
                    );
                }
                report.git = Some(observation);
            }
            serde_json::to_value(report)?
        }
        Command::Register {
            graph: path,
            workspace,
            publisher,
            branch,
            expected_generation,
        } => serde_json::to_value(engine::register_graph(
            store,
            &graph(&path)?,
            &workspace,
            &publisher,
            branch,
            expected_generation,
        )?)?,
        Command::List { repository } => serde_json::to_value(
            SqliteKnowledgeRepository::open_operations_read_only(store)?.workspaces(&repository)?,
        )?,
        Command::Outputs {
            id,
            after_sequence,
            limit,
        } => serde_json::to_value(
            SqliteKnowledgeRepository::open_operations_read_only(store)?.operation_outputs(
                &id,
                after_sequence,
                limit,
            )?,
        )?,
        Command::Status { id } => serde_json::to_value(
            SqliteKnowledgeRepository::open_operations_read_only(store)?
                .operation(&id)?
                .context("unknown operation")?,
        )?,
        Command::Submit {
            request,
            key,
            owner,
        } => {
            let request = serde_json::from_slice(&std::fs::read(request)?)?;
            serde_json::to_value(engine::submit_conflicts(
                &mut SqliteKnowledgeRepository::open(store)?,
                request,
                &key,
                &owner,
            )?)?
        }
        Command::Run {
            id,
            worker,
            pairs,
            git_root,
        } => serde_json::to_value(engine::run_conflicts_with_git(
            &mut SqliteKnowledgeRepository::open(store)?,
            &id,
            &worker,
            pairs,
            git_root.as_deref(),
        )?)?,
        command => {
            let (id, control) = match command {
                Command::Pause { id } => (id, OperationControl::Pause),
                Command::Resume { id } => (id, OperationControl::Resume),
                Command::Cancel { id } => (id, OperationControl::Cancel),
                _ => anyhow::bail!("unsupported collaboration control"),
            };
            serde_json::to_value(
                SqliteKnowledgeRepository::open(store)?.control_operation(&id, control)?,
            )?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
