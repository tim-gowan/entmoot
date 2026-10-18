//! Standalone native bind producer; shares engine pipelines with explicit CLI init/bind.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
#[path = "../rust_profile.rs"]
mod rust_profile;
use entmoot_contracts::blake3_hex;
use entmoot_workspace::native_resolver::oxc::OXC_PIN;
use entmoot_workspace::{
    produce_clang_resolver, produce_go_resolver, produce_java_resolver, produce_oxc_resolver,
    produce_python_resolver, produce_roslyn_resolver, produce_rust_resolver,
    produce_typescript_resolver, publish_dogfood, DogfoodPublishRequest, ExecutionPolicy,
    NativeProduceRequest, ProducerBudget, RA_AP_PIN,
};

#[derive(Parser, Debug)]
#[command(
    name = "entmoot-producer",
    about = "Produce and publish native resolved resolver artifacts (bind)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Report the implementation contract before controlled native execution.
    Fingerprint,
    /// Local worker protocol; the request contains host paths and execution bounds.
    Execute {
        #[arg(long)]
        family: String,
        #[arg(long)]
        request: PathBuf,
    },
    /// TypeScript Program declarations, module exports and observed input closure.
    Typescript {
        #[command(subcommand)]
        command: SidecarCommands,
    },
    /// Pyright batch declarations via ENTMOOT_PYRIGHT_ADAPTER.
    Python {
        #[command(subcommand)]
        command: SidecarCommands,
    },
    /// Rust-analyzer native resolver commands.
    Rust {
        #[command(subcommand)]
        command: RustCommands,
    },
    /// Oxc JS/TS lexical resolver commands.
    Oxc {
        #[command(subcommand)]
        command: OxcCommands,
    },
    /// C# compiler evidence via ENTMOOT_ROSLYN_ADAPTER.
    Roslyn {
        #[command(subcommand)]
        command: SidecarCommands,
    },
    /// Java compiler evidence via ENTMOOT_JAVA_ADAPTER.
    Java {
        #[command(subcommand)]
        command: SidecarCommands,
    },
    /// C/C++ compiler evidence via ENTMOOT_CLANG_ADAPTER.
    Clang {
        #[command(subcommand)]
        command: SidecarCommands,
    },
    /// Go sidecar resolver commands (`ENTMOOT_GO_ADAPTER`).
    Go {
        #[command(subcommand)]
        command: SidecarCommands,
    },
}

#[derive(Subcommand, Debug)]
enum RustCommands {
    /// Produce a resolver artifact for an indexed graph.
    Produce {
        #[arg(long)]
        graph: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = ProducerBudget::DEFAULT_WALL_SECONDS)]
        timeout_seconds: u64,
        #[arg(long, value_enum, default_value_t = CliExecutionPolicy::Safe)]
        execution_policy: CliExecutionPolicy,
        #[command(flatten)]
        rust: rust_profile::RustProfileArgs,
    },
    /// Structural index → produce → attach → immutable publication generation.
    Publish {
        #[command(flatten)]
        rust: rust_profile::RustProfileArgs,
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value = "local/entmoot")]
        repo: String,
        #[arg(long)]
        publications_dir: PathBuf,
        #[arg(long)]
        current: PathBuf,
        #[arg(long, default_value_t = ProducerBudget::DEFAULT_WALL_SECONDS)]
        timeout_seconds: u64,
        #[arg(long, value_enum, default_value_t = CliExecutionPolicy::Safe)]
        execution_policy: CliExecutionPolicy,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        require_native: bool,
    },
}

#[derive(Subcommand, Debug)]
enum OxcCommands {
    /// Produce an Oxc resolver artifact for an indexed JS/TS graph.
    Produce {
        #[arg(long)]
        graph: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = ProducerBudget::DEFAULT_WALL_SECONDS)]
        timeout_seconds: u64,
        #[arg(long, value_enum, default_value_t = CliExecutionPolicy::Safe)]
        execution_policy: CliExecutionPolicy,
    },
}

#[derive(Subcommand, Debug)]
enum SidecarCommands {
    /// Produce a resolver artifact using this family's configured native adapter.
    Produce {
        #[arg(long)]
        graph: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = ProducerBudget::DEFAULT_WALL_SECONDS)]
        timeout_seconds: u64,
        #[arg(long, value_enum, default_value_t = CliExecutionPolicy::Safe)]
        execution_policy: CliExecutionPolicy,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum CliExecutionPolicy {
    Safe,
    Trusted,
}

impl From<CliExecutionPolicy> for ExecutionPolicy {
    fn from(value: CliExecutionPolicy) -> Self {
        match value {
            CliExecutionPolicy::Safe => ExecutionPolicy::Safe,
            CliExecutionPolicy::Trusted => ExecutionPolicy::Trusted,
        }
    }
}

fn producer_digest() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let bytes = std::fs::read(exe).ok()?;
    Some(blake3_hex(&bytes))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Fingerprint => {
            println!(
                "{}",
                serde_json::to_string(&entmoot_workspace::implementation_fingerprint())?
            );
            Ok(())
        }
        Commands::Execute { family, request } => {
            let family = serde_json::from_value(serde_json::Value::String(family))?;
            let request: NativeProduceRequest = serde_json::from_slice(&std::fs::read(request)?)?;
            let bounds = ProducerBudget::default();
            if request.budget.wall < Duration::from_secs(1)
                || request.budget.wall > bounds.wall
                || request.budget.max_files > bounds.max_files
                || request.budget.max_call_sites > bounds.max_call_sites
                || request.budget.max_targets > bounds.max_targets
                || request.execution_policy != ExecutionPolicy::Safe
            {
                bail!("local worker requires safe execution within default cardinality and 1..3600 second bounds");
            }
            request
                .rust_profile
                .validate()
                .map_err(anyhow::Error::msg)?;
            entmoot_workspace::native_resolver::pipeline::produce_family(family, request)?;
            Ok(())
        }
        Commands::Rust { command } => match command {
            RustCommands::Produce {
                graph,
                root,
                out,
                timeout_seconds,
                execution_policy,
                rust,
            } => {
                if !(1..=3600).contains(&timeout_seconds) {
                    bail!("timeout_seconds must be in 1..=3600");
                }
                let result = produce_rust_resolver(NativeProduceRequest {
                    canon: false,
                    graph_path: graph,
                    source_root: root,
                    output_dir: out.clone(),
                    budget: ProducerBudget {
                        wall: Duration::from_secs(timeout_seconds),
                        ..ProducerBudget::default()
                    },
                    execution_policy: execution_policy.into(),
                    rust_profile: rust.into_profile()?,
                })
                .with_context(|| format!("native rust produce into {}", out.display()))?;
                println!(
                    "produced {} (status={}, ra_ap={}, calls={}/{})",
                    result.artifact_path.display(),
                    result.receipt.semantic_status,
                    RA_AP_PIN,
                    result.receipt.observed_calls,
                    result.receipt.inventory_calls
                );
                println!("manifest {}", result.manifest_path.display());
                Ok(())
            }
            RustCommands::Publish {
                rust,
                root,
                repo,
                publications_dir,
                current,
                timeout_seconds,
                execution_policy,
                force,
                require_native,
            } => {
                if !(1..=3600).contains(&timeout_seconds) {
                    bail!("timeout_seconds must be in 1..=3600");
                }
                let result = publish_dogfood(DogfoodPublishRequest {
                    rust_profile: rust.into_profile()?,
                    source_root: root,
                    repository_id: repo,
                    publications_dir,
                    current_pointer: current.clone(),
                    budget: ProducerBudget {
                        wall: Duration::from_secs(timeout_seconds),
                        ..ProducerBudget::default()
                    },
                    execution_policy: execution_policy.into(),
                    force,
                    require_native,
                    producer_bin_digest: producer_digest(),
                })
                .with_context(|| format!("publish dogfood via {}", current.display()))?;
                println!(
                    "{} publication {} (status={}, key={})",
                    if result.reused { "reused" } else { "published" },
                    result.generation_dir.display(),
                    result.publication.semantic_status,
                    result.publication.freshness_key
                );
                Ok(())
            }
        },
        Commands::Oxc { command } => match command {
            OxcCommands::Produce {
                graph,
                root,
                out,
                timeout_seconds,
                execution_policy,
            } => {
                if !(1..=7200).contains(&timeout_seconds) {
                    bail!("timeout_seconds must be in 1..=7200");
                }
                let result = produce_oxc_resolver(NativeProduceRequest {
                    canon: false,
                    graph_path: graph,
                    source_root: root,
                    output_dir: out.clone(),
                    budget: ProducerBudget {
                        wall: Duration::from_secs(timeout_seconds),
                        ..ProducerBudget::default()
                    },
                    execution_policy: execution_policy.into(),
                    rust_profile: Default::default(),
                })
                .with_context(|| format!("native oxc produce into {}", out.display()))?;
                println!(
                    "produced {} (status={}, oxc={}, calls={}/{})",
                    result.artifact_path.display(),
                    result.receipt.semantic_status,
                    OXC_PIN,
                    result.receipt.observed_calls,
                    result.receipt.inventory_calls
                );
                println!("manifest {}", result.manifest_path.display());
                Ok(())
            }
        },
        Commands::Go { command } => run_sidecar(command, produce_go_resolver, "go"),
        Commands::Typescript { command } => {
            run_sidecar(command, produce_typescript_resolver, "typescript")
        }
        Commands::Roslyn { command } => run_sidecar(command, produce_roslyn_resolver, "roslyn"),
        Commands::Python { command } => run_sidecar(command, produce_python_resolver, "python"),
        Commands::Java { command } => run_sidecar(command, produce_java_resolver, "java"),
        Commands::Clang { command } => run_sidecar(command, produce_clang_resolver, "clang"),
    }
}

fn run_sidecar(
    command: SidecarCommands,
    produce: fn(
        NativeProduceRequest,
    ) -> std::result::Result<
        entmoot_workspace::native_resolver::NativeProduceResult,
        entmoot_workspace::native_resolver::NativeProducerError,
    >,
    language: &str,
) -> Result<()> {
    let SidecarCommands::Produce {
        graph,
        root,
        out,
        timeout_seconds,
        execution_policy,
    } = command;
    if !(1..=7200).contains(&timeout_seconds) {
        bail!("timeout_seconds must be in 1..=7200");
    }
    let result = produce(NativeProduceRequest {
        canon: false,
        graph_path: graph,
        source_root: root,
        output_dir: out.clone(),
        budget: ProducerBudget {
            wall: Duration::from_secs(timeout_seconds),
            ..ProducerBudget::default()
        },
        execution_policy: execution_policy.into(),
        rust_profile: Default::default(),
    })
    .with_context(|| format!("native {language} produce into {}", out.display()))?;
    println!("{}", serde_json::to_string(&result.receipt)?);
    if result.receipt.semantic_status == entmoot_contracts::NativeSemanticStatus::Failed {
        bail!("invalid_environment: {language} unavailable; inspect {} for attributable failure and unresolved evidence", out.display());
    }
    Ok(())
}
