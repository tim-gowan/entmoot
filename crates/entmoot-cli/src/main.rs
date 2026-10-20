//! CLI for structural hashing: file/stdin hash and ground-truth validation.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

static SOURCE_DIRECTORY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

mod backend;
mod collaborate;
mod navigation;
mod navigation_walk;
mod neighborhood;
mod operator_summary;
mod output;
mod rust_profile;
mod subject_args;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use tracing::info;
use tracing_subscriber::EnvFilter;

use entmoot_contracts::{Address, ArtifactManifest};
use entmoot_core::ground_truth::{
    policy_from_config_name, resolve_dataset_path, GroundTruthDataset, GroundTruthValidator,
};
use entmoot_core::index::build_file_index;
use entmoot_core::StructuralHasher;
use entmoot_workspace::query::{
    artifact_result, assertions_result, dependency_result, expert_proposition_result,
    expert_relation_claims_result, owner_result, relations_result, resources_result, trace_result,
    unresolved_result,
};
use entmoot_workspace::{
    adapter_for_manifest, build_topology_from_repo_with_options, compare_inventory_overlay,
    compare_revision_impact_with_correspondence, ingest_artifact_manifest, load_graph_json,
    load_inventory_for_paths, write_graph_json, ArtifactAdapter, AtomicGraphWrite,
    FilesystemArtifactObjectRepository, KnowledgeRead, KnowledgeRepository, KnowledgeStore,
    KnowledgeStoreBatches, PublicationBuffer, ResourceInventoryAdapter, SarifAdapter,
    SemanticClusterAdapter, SqliteKnowledgeRepository, TopologyBuildOptions,
};

#[derive(Debug, Parser)]
#[command(
    name = "entmoot",
    version,
    about = "Logic-addressable indexing, structural hashing, and validation"
)]
struct Cli {
    /// Emit stable JSON even on an interactive terminal (pipes always receive JSON).
    #[arg(long, global = true)]
    json: bool,
    /// Operate in this source directory (defaults to the current workspace).
    #[arg(short = 'C', long, global = true)]
    directory: Option<PathBuf>,
    /// Local data root (default: ~/.entmoot). Never writes under the analyzed source.
    #[arg(long, global = true, env = "ENTMOOT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Connect read operations to an explicitly started loopback MCP backend.
    #[arg(long, global = true, env = "ENTMOOT_QUERY_BACKEND")]
    backend: Option<String>,
    /// Override the shared backend environment setting for this invocation.
    #[arg(long, global = true)]
    standalone: bool,
    /// Read the backend bearer credential here, or from ENTMOOT_QUERY_TOKEN.
    #[arg(long, global = true)]
    backend_token_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Find source symbols in the current workspace; prepares inventory automatically.
    Find(navigation::FindArgs),
    /// Find bounded directed witnesses between two exact subjects.
    Path(navigation_walk::PathArgs),
    /// Inspect one exact symbol and its evidence.
    Inspect(navigation::SubjectArgs),
    /// Read verified snapshot source lines.
    ReadSource(navigation::SourceArgs),
    /// Page represented incoming calls; completeness is scoped explicitly.
    Callers(navigation::SubjectArgs),
    /// Page represented outgoing calls and open attempts.
    Callees(navigation::SubjectArgs),
    /// Count and inspect source call boundaries, including unsupported and external calls.
    Boundaries(navigation::BoundaryArgs),
    /// Plan whole-publication retention. No evidence is removed.
    StoragePlan {
        #[arg(long)]
        root: PathBuf,
        /// Local RetentionPolicy JSON; defaults to the last applied workspace policy.
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Apply a reviewed storage plan during a quiet maintenance window.
    StorageApply {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        plan: PathBuf,
    },
    /// Fetch an immutable revision into an existing Git repository without changing its checkout.
    #[command(hide = true)]
    SourceFetch {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        remote: String,
        #[arg(long)]
        revision: String,
        #[arg(long, default_value_t=120, value_parser=clap::value_parser!(u64).range(1..=3600))]
        timeout_seconds: u64,
    },
    /// Walk dependents, source-labeled origins, or sink-labeled destinations.
    Neighborhood(neighborhood::NeighborhoodArgs),
    /// Refresh source inventory and discover optional native adapters.
    Init {
        #[command(flatten)]
        rust: rust_profile::RustProfileArgs,
        /// Git top-level from cwd when omitted; explicit roots preserve scoped indexing.
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value = "v1")]
        config: String,
        /// Inventory only (default). Refreshes adapter profile without requiring adapters.
        #[arg(long, conflicts_with = "require_native")]
        structural_only: bool,
        /// Require native adapters during init (opt-in; default init stays structural-first).
        #[arg(long)]
        require_native: bool,
        /// Per native producer wall-clock allowance (seconds).
        #[arg(long, default_value_t = entmoot_workspace::ProducerBudget::DEFAULT_WALL_SECONDS, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_seconds: u64,
        /// Declared publication subject (`pkg:…` or repository[`#path`]).
        #[arg(long)]
        subject: Option<String>,
    },
    /// Explicitly produce and attach binding evidence. Reads never invoke this operation.
    Bind {
        /// Opt in to retained canon graph production for clone operations.
        #[arg(long, value_parser = ["canon"])]
        experimental: Option<String>,
        #[command(flatten)]
        rust: rust_profile::RustProfileArgs,
        #[arg(long, conflicts_with = "from_impact")]
        root: Option<PathBuf>,
        /// Recompute and execute fenced work from a saved local impact package.
        #[arg(long)]
        from_impact: Option<PathBuf>,
        /// Adopt an existing structural graph and bind into .entmoot/current.
        #[arg(long, conflicts_with = "from_impact")]
        graph: Option<PathBuf>,
        /// Existing durable store for --graph; defaults to its adjacent store.
        #[arg(long, requires = "graph")]
        store: Option<PathBuf>,
        /// Bypass qualified reuse and produce fresh artifacts.
        #[arg(long)]
        full: bool,
        /// Deprecated no-op: native is required by default. Prefer omitting this flag.
        #[arg(long, hide = true)]
        require_native: bool,
        /// Skip native producers (waiver from default required-native bind).
        #[arg(long, conflicts_with = "require_native")]
        structural_only: bool,
        /// Report stages without producing, attaching or publishing.
        #[arg(long)]
        plan_only: bool,
        /// Per native producer wall-clock allowance (seconds).
        #[arg(long, default_value_t = entmoot_workspace::ProducerBudget::DEFAULT_WALL_SECONDS, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_seconds: u64,
        /// Declared publication subject (`pkg:…` or repository[`#path`]).
        #[arg(long)]
        subject: Option<String>,
    },
    /// Execute a portable QueryRequest JSON document using the shared read-only service.
    Request {
        #[command(flatten)]
        output: output::InformationArgs,
        /// Validated graph publication to query.
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Local publication pointer; otherwise discover the workspace's current publication.
        #[arg(long, conflicts_with = "graph")]
        publication: Option<PathBuf>,
        /// Defaults to the graph's adjacent knowledge store.
        #[arg(long)]
        store: Option<PathBuf>,
        /// JSON QueryRequest file (same request as MCP query.request).
        #[arg(long)]
        request: Option<PathBuf>,
        /// Print a complete portable request and exit without opening a publication.
        #[arg(long, conflicts_with = "request", value_parser = ["find-subject", "find-parameter", "path", "neighborhood", "unresolved", "read-source"])]
        example: Option<String>,
        /// Interpret the request file as 1..=1000 QueryRequests and return results in order.
        /// Shares one immutable inventory/index load; no producers run.
        #[arg(long)]
        batch: bool,
        /// Source checkout for digest-verified reads.
        #[arg(long)]
        source_root: Option<PathBuf>,
    },
    /// Relay changed callables from an immutable pair to compact head caller pages.
    RelayImpact {
        #[command(flatten)]
        output: output::InformationArgs,
        /// Head graph publication to query.
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Base graph publication to compare.
        #[arg(long)]
        compare_graph: Option<PathBuf>,
        /// Defaults to the head graph's adjacent knowledge store.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Portable ImpactRelayRequest JSON; same request as MCP relay_impact.
        #[arg(long)]
        request: Option<PathBuf>,
    },
    /// Discover directed call routes between attached provider-role seeds.
    TraceSeededPaths {
        #[command(flatten)]
        output: output::InformationArgs,
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Defaults to the graph's adjacent durable knowledge store.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Typed SeededPathRequest JSON; defaults to bounded source-to-sink discovery.
        #[arg(long)]
        request: Option<PathBuf>,
    },
    /// Report publication health and bounded high-signal find selectors.
    Status {
        #[arg(long)]
        graph: Option<PathBuf>,
        /// Local publication pointer; otherwise discover the workspace's current publication.
        #[arg(long, conflicts_with = "graph")]
        publication: Option<PathBuf>,
        /// Defaults to the graph's adjacent durable knowledge store.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Source checkout for health reporting when configured.
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Optional compare graph for `changed_subject` suggestions.
        #[arg(long)]
        compare_graph: Option<PathBuf>,
        /// Optional StatusRequest JSON; defaults omit compare identity.
        #[arg(long)]
        request: Option<PathBuf>,
    },
    /// Page materialized inventory subjects (default: callables).
    Inventory {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long, conflicts_with = "graph")]
        publication: Option<PathBuf>,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Optional InventoryRequest JSON; otherwise use flags below.
        #[arg(long)]
        request: Option<PathBuf>,
        /// Subject kinds (repeatable). Defaults to callable.
        #[arg(long = "kind")]
        kinds: Vec<String>,
        /// Repository-relative path prefix filter.
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Page weakly connected call-relation components (review islands, not dead code).
    CallComponents {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long, conflicts_with = "graph")]
        publication: Option<PathBuf>,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Optional CallComponentsRequest JSON.
        #[arg(long)]
        request: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 32)]
        limit: usize,
        #[arg(long, default_value_t = 16)]
        sample_limit: usize,
        #[arg(long, default_value_t = 100_000)]
        max_nodes: usize,
        #[arg(long, default_value_t = 100_000)]
        max_edges: usize,
    },
    /// Account for every inventoried callable and call site against native evidence.
    AuditCalls {
        #[arg(long)]
        graph: PathBuf,
        /// Native resolver manifests. Missing evidence remains a static-work gap.
        #[arg(long = "artifact")]
        artifacts: Vec<PathBuf>,
        /// Exact callable node IDs for a bounded native reachability report.
        #[arg(long, requires = "build_context")]
        entrypoint: Vec<String>,
        #[arg(long, requires = "entrypoint")]
        build_context: Option<String>,
        /// Emit the report, then fail if static work or producer omissions remain.
        #[arg(long)]
        require_accounted: bool,
    },
    /// Emit a deterministic work decision; never execute a compiler or scanner.
    PlanAnalysis {
        #[arg(long)]
        request: PathBuf,
        /// Choose automatic planning, explicitly request qualified work, or disable requests.
        #[arg(long = "adapter-pass", default_value = "auto", value_parser = ["auto", "always", "never"])]
        adapter_pass: String,
    },
    /// Persist and advance fenced work handed to an external producer.
    Work {
        #[arg(long)]
        queue: PathBuf,
        #[command(subcommand)]
        command: WorkCmd,
    },
    /// Print the current contract, derivation and storage layout fingerprints without opening a store.
    Fingerprint,
    /// Local repository collaboration and durable conflict operations.
    Collaborate {
        /// Advanced override; otherwise use the workspace knowledge repository.
        #[arg(long, hide = true)]
        store: Option<PathBuf>,
        #[command(subcommand)]
        command: collaborate::Command,
    },
    /// Explicitly run declared test targets and write attributable status evidence.
    ValidateTests {
        #[arg(long)]
        graph: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Internal evaluation boundary: validate typed run inputs without opening a store.
    #[command(hide = true)]
    ValidateCampaignInputs {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        evaluator_digest: String,
    },
    /// Print the exact normalization policy, fingerprint, and information-loss audit.
    Policy {
        /// Preset name or @path to a complete JSON StructuralHashPolicy.
        #[arg(long, default_value = "v1")]
        config: String,
    },
    /// Hash source from a file or stdin
    Hash {
        #[arg(long)]
        lang: String,
        #[arg(long, default_value = "v1")]
        config: String,
        #[arg(long)]
        file: Option<PathBuf>,
        /// Emit the per-callable `FileIndexRecord` used by incremental consumers.
        #[arg(long)]
        callable_inventory: bool,
        /// Repository-relative path used to seal callable occurrence addresses.
        #[arg(long, requires = "callable_inventory")]
        path: Option<String>,
    },
    /// Validate the labeled ground-truth dataset
    ValidateGroundTruth {
        #[arg(long, default_value = "v1")]
        config: String,
        #[arg(long)]
        json_out: Option<PathBuf>,
        #[arg(long)]
        dataset: Option<PathBuf>,
    },
    /// Build a topology graph for a repository (low-level; operators use init).
    #[command(hide = true)]
    Index {
        #[arg(long)]
        root: PathBuf,
        /// Stable repository identity for the snapshot. When omitted, index derives
        /// `local/<git-toplevel-basename>` from the checkout (never the scan path).
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        out: PathBuf,
        /// Compact machine-readable validation and count summary.
        #[arg(long)]
        summary_out: Option<PathBuf>,
        /// Durable knowledge database. Defaults to `<out>.knowledge.sqlite3`.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Immutable native artifact object directory. Defaults to `<store>.objects`.
        #[arg(long)]
        artifact_store: Option<PathBuf>,
        #[arg(long, default_value = "v1")]
        config: String,
        #[arg(long)]
        revision: Option<String>,
        /// Explicit inventory-only escape hatch (index never invokes a producer).
        #[arg(long)]
        structural_only: bool,
        /// Optional one-shot co-ingest after topology build (cold start / CI).
        /// Prefer `attach` to enrich an existing graph without re-indexing.
        #[arg(long = "artifact", value_name = "MANIFEST")]
        artifacts: Vec<PathBuf>,
        #[arg(long)]
        replace_series: bool,
    },
    /// Preferred path to enrich an existing graph with evidence/resource manifests
    Attach {
        #[arg(long)]
        graph: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Durable knowledge database. Defaults to `<graph>.knowledge.sqlite3`.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Immutable native artifact object directory. Defaults to `<store>.objects`.
        #[arg(long)]
        artifact_store: Option<PathBuf>,
        #[arg(long = "artifact", value_name = "MANIFEST", required = true)]
        artifacts: Vec<PathBuf>,
        #[arg(long)]
        replace_series: bool,
    },
    /// Inspect an artifact (and optional sibling/manifest)
    Artifact {
        #[command(subcommand)]
        command: ArtifactCmd,
    },
    /// Query the durable knowledge store without mutating it
    Query {
        /// Advanced override; otherwise use the workspace knowledge repository.
        #[arg(long, hide = true)]
        store: Option<PathBuf>,
        #[command(subcommand)]
        command: QueryCmd,
    },
    /// Publish an isolated conditional applicability ledger and scheduling receipt.
    /// Does not run producers or mutate either graph.
    Revalidate {
        #[arg(long)]
        base_graph: PathBuf,
        #[arg(long)]
        head_graph: PathBuf,
        #[arg(long)]
        request: PathBuf,
        /// New ledger path; existing ledgers are never overwritten.
        #[arg(long)]
        ledger: PathBuf,
    },
    /// Compare revision impact and breaking-change signals; never runs producers.
    /// Signals are not SemVer or vulnerability-fixed verdicts.
    Impact(ImpactArgs),
    /// Evidence judgments: revision impact or direct package API compatibility.
    Assess {
        #[command(subcommand)]
        command: AssessCommand,
    },
    /// Materialize dependency package trees in workspace storage with declared subjects.
    Deps {
        #[command(subcommand)]
        command: DepsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DepsCommand {
    /// Copy or link a package source tree and record subject=PURL for later impact/overlay.
    Materialize {
        /// Package-URL coordinate without version (`pkg:pypi/pydantic`).
        #[arg(long)]
        subject: String,
        /// Exact version label used in the deps path.
        #[arg(long)]
        version: String,
        /// Source tree to materialize (extracted zip / checkout).
        #[arg(long)]
        from: PathBuf,
        /// Workspace root; defaults to cwd.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Optional python_environment.json to copy beside the materialization.
        #[arg(long)]
        python_environment: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum AssessCommand {
    /// Find repeated source patterns for review; no behavioral-equivalence claim.
    Clones {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long, conflicts_with = "graph")]
        publication: Option<PathBuf>,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Optional CloneRequest JSON with file/pattern budgets and page controls.
        #[arg(long)]
        request: Option<PathBuf>,
    },

    /// Review declarations without observed callers; never a deletion verdict.
    Unused {
        #[arg(long)]
        graph: Option<PathBuf>,
        #[arg(long, conflicts_with = "graph")]
        publication: Option<PathBuf>,
        #[arg(long)]
        store: Option<PathBuf>,
        #[arg(long)]
        source_root: Option<PathBuf>,
        #[arg(long)]
        request: Option<PathBuf>,
        /// Print the report then fail unless all completeness dimensions are qualified.
        #[arg(long)]
        require_complete: bool,
    },
    /// Assess the working tree by default, or explicit immutable graph pairs.
    Impact(ImpactArgs),
    /// Compare independently published direct package boundaries; never runs producers.
    Compatibility {
        /// Consumer graph, current.json pointer, or published checkout; defaults to this workspace.
        #[arg(long)]
        consumer: Option<PathBuf>,
        /// Provider graphs, pointers, or published checkouts; defaults to the workspace's deps directory.
        #[arg(long)]
        provider: Vec<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Inventory-only symbol presence overlay across two publications (not Bound/Fits).
    Overlay {
        #[arg(long)]
        base_graph: PathBuf,
        #[arg(long)]
        head_graph: PathBuf,
        /// Optional name substring filters (repeatable).
        #[arg(long)]
        name: Vec<String>,
        /// Optional path prefix filters (repeatable).
        #[arg(long)]
        path_prefix: Vec<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, clap::Args)]
struct ImpactArgs {
    /// Select tests conservatively using ownership and both revision graphs.
    #[arg(long, conflicts_with = "revalidate")]
    tests: bool,
    /// TestImpactRequest JSON with explicit target identities, owners and fixture dependencies.
    #[arg(long, requires = "tests")]
    test_targets: Option<PathBuf>,
    /// Explicit base graph. Supply with --head-graph for publication/commit pairs.
    #[arg(long, requires = "head_graph")]
    base_graph: Option<PathBuf>,
    /// Explicit head graph. Supply with --base-graph for publication/commit pairs.
    #[arg(long, requires = "base_graph")]
    head_graph: Option<PathBuf>,
    /// Working-tree normalization policy. Presets or @path; must match the baseline.
    #[arg(long, default_value = "v1")]
    config: String,
    /// Include a conditional applicability ledger and work hints in a local package.
    #[arg(long, conflicts_with = "no_revalidate", requires = "base_graph")]
    revalidate: bool,
    /// Read-only portable comparison, without an applicability ledger.
    #[arg(long)]
    no_revalidate: bool,
    /// Save the result as UTF-8 JSON. With --revalidate, this is a new package.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Correspondence mode: subject_match | relative_occurrence | explicit_alias.
    #[arg(long)]
    correspondence_mode: Option<String>,
    /// Asserted subject override for the compare (`pkg:…` or repository[`#path`]).
    #[arg(long)]
    subject: Option<String>,
    /// Asserted subject alias `from=to` (repeatable); enables fork correspondence.
    #[arg(long)]
    alias: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum WorkCmd {
    Enqueue {
        #[arg(long)]
        decision: PathBuf,
    },
    Claim {
        #[arg(long)]
        id: String,
        #[arg(long)]
        worker: PathBuf,
    },
    ArtifactReady {
        #[arg(long)]
        id: String,
        #[arg(long)]
        fencing_token: String,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        graph: PathBuf,
    },
    Ingested {
        #[arg(long)]
        id: String,
        #[arg(long)]
        fencing_token: String,
    },
    Fail {
        #[arg(long)]
        id: String,
        #[arg(long)]
        fencing_token: String,
        #[arg(long)]
        reason: String,
    },
    Cancel {
        #[arg(long)]
        id: String,
    },
    Get {
        #[arg(long)]
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum ArtifactCmd {
    Inspect {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum QueryCmd {
    /// Return assertions whose entity id exactly matches the subject
    Assertions {
        #[arg(long)]
        subject: String,
    },
    /// Return outgoing relations, their relation_claims, and unresolved attempts
    Relations {
        /// JSON file containing one versioned `Address`
        #[arg(long)]
        address: PathBuf,
    },
    /// Return all producer relation claims for one addressable relation edge
    RelationClaims {
        #[arg(long = "relation-edge")]
        relation_edge: String,
    },
    /// Resolve one stable proposition to snapshot edge occurrences and relation_claims
    Proposition {
        #[arg(long)]
        proposition: String,
    },
    /// Return every declaration/observation of one logical resource
    Resources {
        /// Bare 64-hex logical resource id
        #[arg(long)]
        resource: String,
    },
    /// Return one exact unresolved relation attempt
    Unresolved {
        #[arg(long)]
        id: String,
    },
    /// Return one exact immutable artifact manifest
    Artifact {
        #[arg(long)]
        id: String,
    },
    /// Return one exact trace and an ordered, bounded step page
    Trace {
        #[arg(long)]
        id: String,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Return a bounded page of records published by one owner
    Owner {
        #[arg(long)]
        id: String,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Return bounded owners, inputs, and dependents for one exact record
    Dependency {
        #[arg(long)]
        collection: String,
        #[arg(long = "record")]
        record_id: String,
        #[arg(long)]
        limit: Option<usize>,
    },
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // Operator progress on Entmoot targets only; ra_ap_/salsa stay at warn+
            // unless RUST_LOG opts in (e.g. RUST_LOG=warn,entmoot_workspace=info,ra_ap_=debug).
            "warn,entmoot=info,entmoot_workspace=info".into()
        }))
        .with_writer(std::io::stderr)
        .try_init();

    // Windows' main-thread reserve is too small for debug builds of the
    // composition root. Keep a bounded, explicit stack for command execution.
    let result = std::thread::Builder::new()
        .name("entmoot".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
        .map_err(anyhow::Error::from)
        .and_then(|worker| match worker.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        });
    if let Err(err) = result {
        eprintln!("{err:#}");
        let code = if err.chain().any(|e| e.to_string().contains("mismatch")) {
            2
        } else {
            1
        };
        process::exit(code);
    }
}

fn report_query_error(error: entmoot_contracts::query::QueryServiceError) -> anyhow::Error {
    println!(
        "{}",
        serde_json::json!({
            "schema_version": "entmoot.query.error.v1", "status": "error",
            "kind": error.code(), "message": error.to_string(),
        })
    );
    error.into()
}

fn resolve_default_database(store: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = store {
        return Ok(path);
    }
    let publication = prepare_current_workspace()?;
    Ok(
        entmoot_workspace::KnowledgeViewLocator::read(&publication.current.store)?
            .map_or(publication.current.store, |view| view.database),
    )
}

fn prepare_current_workspace() -> Result<entmoot_workspace::workspace::WorkspacePublication> {
    Ok(match SOURCE_DIRECTORY.get() {
        Some(root) => entmoot_workspace::workspace::prepare_workspace(root)?,
        None => {
            entmoot_workspace::workspace::prepare_discovered_workspace(&std::env::current_dir()?)?
        }
    })
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    output::configure(cli.json);
    entmoot_workspace::workspace::configure_data_directory(cli.data_dir.as_deref())?;
    if let Some(directory) = &cli.directory {
        std::env::set_current_dir(directory)?;
        let _ = SOURCE_DIRECTORY.set(std::env::current_dir()?);
    }
    if !cli.standalone {
        if let Some(url) = &cli.backend {
            if backend::run(&cli.command, url, cli.backend_token_file.as_deref())? {
                return Ok(());
            }
        }
    }
    if let Some(request) = navigation::request(&cli.command)? {
        return navigation::run(request);
    }
    match cli.command {
        Commands::Find(_)
        | Commands::Inspect(_)
        | Commands::ReadSource(_)
        | Commands::Callers(_)
        | Commands::Callees(_)
        | Commands::Boundaries(_) => bail!("navigation request was not handled"),
        Commands::StoragePlan { root, policy, out } => {
            use entmoot_workspace::workspace::retention::{plan_retention, read_retention_policy};
            let policy = match policy {
                Some(path) => serde_json::from_slice(&fs::read(path)?)?,
                None => read_retention_policy(&root)?,
            };
            let plan = plan_retention(&root, policy)?;
            entmoot_workspace::workspace::write_json(&out, &plan)?;
            crate::output::emit(&plan)?;
        }
        Commands::StorageApply { root, plan } => {
            let plan = serde_json::from_slice(&fs::read(plan)?)?;
            let receipt = entmoot_workspace::workspace::retention::apply_retention(&root, &plan)?;
            crate::output::emit(&receipt)?;
        }
        Commands::SourceFetch {
            root,
            remote,
            revision,
            timeout_seconds,
        } => {
            use entmoot_workspace::source_store::{
                GitFetchRequest, GitSourceRepository, SourceRepository,
            };
            let receipt = GitSourceRepository::open(&root)?.fetch_revision(GitFetchRequest {
                remote: &remote,
                revision: &revision,
                timeout: std::time::Duration::from_secs(timeout_seconds),
            })?;
            crate::output::emit(&receipt)?;
        }
        Commands::Init {
            rust,
            root,
            repo,
            config,
            structural_only,
            require_native,
            timeout_seconds,
            subject,
        } => {
            let root = match root.or_else(|| SOURCE_DIRECTORY.get().cloned()) {
                Some(root) => fs::canonicalize(root)?,
                None => entmoot_workspace::workspace::workspace_root(&std::env::current_dir()?)?,
            };
            // Bare init is structural-first: publish inventory + refresh adapter profile.
            // Opt in with --require-native to hard-fail missing adapters during init.
            let structural_only = structural_only || !require_native;
            let mut options = workspace_options(
                false,
                require_native,
                false,
                structural_only,
                timeout_seconds,
            );
            options.rust_profile = rust.into_profile()?;
            if let Some(raw) = subject {
                options.subject = Some(subject_args::parse_subject(&raw)?);
            }
            let summary = entmoot_workspace::workspace::initialize_workspace(
                &root,
                repo.as_deref(),
                &load_normalization_policy(&config)?,
                &options,
            )?;
            operator_summary::write_workspace_summary("init", &summary);
            crate::output::emit(&summary)?;
        }
        Commands::Bind {
            experimental,
            rust,
            root,
            graph,
            store,
            from_impact,
            full,
            require_native: _deprecated_require_native,
            structural_only,
            plan_only,
            timeout_seconds,
            subject,
        } => {
            // Native is required by default; --structural-only is the only waiver.
            let require_native = !structural_only;
            let mut options = workspace_options(
                full,
                require_native,
                plan_only,
                structural_only,
                timeout_seconds,
            );
            options.canon = experimental.as_deref() == Some("canon");
            options.rust_profile = rust.into_profile()?;
            if let Some(raw) = subject {
                options.subject = Some(subject_args::parse_subject(&raw)?);
            }
            if let Some(path) = from_impact {
                let receipt =
                    entmoot_workspace::workspace::impact::bind_from_package(&path, &options)?;
                crate::output::emit(&receipt)?;
                return Ok(());
            }
            if let Some(graph) = graph {
                let summary = entmoot_workspace::workspace::bind_publication(
                    &graph,
                    root.as_deref(),
                    store.as_deref(),
                    &options,
                )?;
                operator_summary::write_workspace_summary("bind", &summary);
                crate::output::emit(&summary)?;
                return Ok(());
            }
            let root = match root.or_else(|| SOURCE_DIRECTORY.get().cloned()) {
                Some(root) => root,
                None => entmoot_workspace::workspace::workspace_root(&std::env::current_dir()?)?,
            };
            if !options.plan_only {
                entmoot_workspace::workspace::prepare_workspace(&root)?;
            }
            let summary = entmoot_workspace::workspace::bind_workspace(&root, &options)?;
            operator_summary::write_workspace_summary("bind", &summary);
            crate::output::emit(&summary)?;
        }
        Commands::Neighborhood(args) => neighborhood::run(args)?,
        Commands::Path(args) => args.run()?,
        Commands::Request {
            output,
            graph,
            publication,
            store,
            request,
            example,
            source_root,
            batch,
        } => {
            if let Some(example) = example {
                let value = match example.as_str() {
                    "find-parameter" => serde_json::json!({"operation":"find","request":{
                        "kind":"subject","selector":{"kinds":["callable"],
                        "name":{"mode":"contains","value":"handle"},
                        "callable":{"parameters":[{"declared_type_syntax":{"mode":"contains","value":"Request"}}]}}
                    }}),
                    "path" => serde_json::json!({"operation":"traverse","request":{
                        "kind":"path","start":"<origin-id>","target":"<destination-id>","predicates":["calls"],
                        "evidence_filter":"resolved_only","prune":{"subjects":[]},"budget":{}
                    }}),
                    "neighborhood" => serde_json::json!({"operation":"traverse","request":{
                        "kind":"neighborhood","start":"<subject-id>","direction":"callers","predicates":["calls"],
                        "evidence_filter":"resolved_only","endpoint_selector":{"name":{"mode":"contains","value":"handle"}},"budget":{}
                    }}),
                    "unresolved" => {
                        serde_json::json!({"operation":"inspect","request":{"kind":"unresolved","id":"<subject-id>","limit":50}})
                    }
                    "read-source" => {
                        serde_json::json!({"operation":"read","request":{"kind":"source","path":"src/main.rs","start_line":1,"end_line":40}})
                    }
                    _ => {
                        serde_json::json!({"operation":"find","request":{"kind":"subject","query":"main","limit":20}})
                    }
                };
                let _: entmoot_contracts::query::QueryRequest =
                    serde_json::from_value(value.clone())?;
                crate::output::emit(&value)?;
                return Ok(());
            }
            let (graph, store, source_root) =
                resolve_read_paths(graph, publication, store, source_root)?;
            let bytes = fs::read(
                request.context("--request is required (or use --example find-subject)")?,
            )?;
            let mut requests: Vec<entmoot_contracts::query::QueryRequest> = if batch {
                serde_json::from_slice(&bytes).context("decode QueryRequest array")?
            } else {
                vec![serde_json::from_slice(&bytes).context("decode QueryRequest")?]
            };
            for request in &mut requests {
                let mut options = request.output().clone();
                output.apply(&mut options)?;
                request.set_output(options);
            }
            if requests.is_empty() || requests.len() > 1000 {
                bail!("request batch must contain 1..=1000 requests");
            }
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                source_root.as_deref(),
                None,
                source_root.is_some(),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let engine = opened.engine;
            let service: &dyn entmoot_contracts::query::QueryService = &engine;
            let results = requests
                .into_iter()
                .map(|request| service.execute(request))
                .collect::<Result<Vec<_>, _>>();
            match results {
                Ok(results) => {
                    if batch {
                        crate::output::emit(&results)?;
                    } else if let Some(result) = results.first() {
                        crate::output::emit(result)?;
                    }
                }
                Err(error) => {
                    return Err(report_query_error(error));
                }
            }
        }
        Commands::RelayImpact {
            output,
            graph,
            compare_graph,
            store,
            request,
        } => {
            let graph = graph.context("--graph is required for standalone reads")?;
            let compare_graph =
                compare_graph.context("--compare-graph is required for standalone relay impact")?;
            let mut request =
                match request {
                    Some(path) => serde_json::from_slice::<
                        entmoot_contracts::query::ImpactRelayRequest,
                    >(&fs::read(path)?)?,
                    None => entmoot_contracts::query::ImpactRelayRequest::default(),
                };
            output.apply(&mut request.output)?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                None,
                Some(&compare_graph),
                false,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let report =
                entmoot_contracts::query::QueryService::relay_impact(&opened.engine, request)
                    .map_err(report_query_error)?;
            crate::output::emit(&report)?;
        }
        Commands::TraceSeededPaths {
            output,
            graph,
            store,
            request,
        } => {
            let graph = graph.context("--graph is required for standalone reads")?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph, &store, None, None, false,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let mut request =
                match request {
                    Some(path) => serde_json::from_slice::<
                        entmoot_contracts::query::SeededPathRequest,
                    >(&fs::read(path)?)?,
                    None => entmoot_contracts::query::SeededPathRequest::default(),
                };
            output.apply(&mut request.output)?;
            let report =
                entmoot_contracts::query::QueryService::trace_seeds(&opened.engine, request)
                    .map_err(report_query_error)?;
            crate::output::emit(&report)?;
        }
        Commands::Status {
            graph,
            publication,
            store,
            source_root,
            compare_graph,
            request,
        } => {
            let (graph, store, source_root) =
                resolve_read_paths(graph, publication, store, source_root)?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let request = match request {
                Some(path) => serde_json::from_slice::<entmoot_contracts::query::StatusRequest>(
                    &fs::read(path)?,
                )?,
                None => entmoot_contracts::query::StatusRequest::default(),
            };
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                source_root.as_deref(),
                compare_graph.as_deref(),
                source_root.is_some(),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let report = entmoot_contracts::query::QueryService::status(&opened.engine, request)?;
            crate::output::emit(&report)?;
        }
        Commands::Inventory {
            graph,
            publication,
            store,
            source_root,
            request,
            kinds,
            path,
            offset,
            limit,
        } => {
            let (graph, store, source_root) =
                resolve_read_paths(graph, publication, store, source_root)?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let request = match request {
                Some(path) => serde_json::from_slice::<entmoot_contracts::query::InventoryRequest>(
                    &fs::read(path)?,
                )?,
                None => {
                    let mut request = entmoot_contracts::query::InventoryRequest::default();
                    if !kinds.is_empty() {
                        request.kinds = kinds;
                    }
                    request.path = path;
                    request.offset = offset;
                    request.limit = limit;
                    request
                }
            };
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                source_root.as_deref(),
                None,
                source_root.is_some(),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let report =
                entmoot_contracts::query::QueryService::inventory(&opened.engine, request)?;
            crate::output::emit(&report)?;
        }
        Commands::CallComponents {
            graph,
            publication,
            store,
            source_root,
            request,
            offset,
            limit,
            sample_limit,
            max_nodes,
            max_edges,
        } => {
            let (graph, store, source_root) =
                resolve_read_paths(graph, publication, store, source_root)?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let request = match request {
                Some(path) => serde_json::from_slice::<
                    entmoot_contracts::query::CallComponentsRequest,
                >(&fs::read(path)?)?,
                None => entmoot_contracts::query::CallComponentsRequest {
                    predicate: "calls".into(),
                    offset,
                    limit,
                    sample_limit,
                    max_nodes,
                    max_edges,
                },
            };
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                source_root.as_deref(),
                None,
                source_root.is_some(),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let report =
                entmoot_contracts::query::QueryService::call_components(&opened.engine, request)?;
            crate::output::emit(&report)?;
        }
        Commands::Assess {
            command:
                AssessCommand::Clones {
                    graph,
                    publication,
                    store,
                    source_root,
                    request,
                },
        } => {
            let (graph, store, source_root) =
                resolve_read_paths(graph, publication, store, source_root)?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let request = match request {
                Some(path) => {
                    serde_json::from_slice::<entmoot_contracts::CloneRequest>(&fs::read(path)?)?
                }
                None => entmoot_contracts::CloneRequest::default(),
            };
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                source_root.as_deref(),
                None,
                source_root.is_some(),
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let report = entmoot_contracts::query::QueryService::clones(&opened.engine, request)?;
            if let Some(nudge) = &report.nudge {
                eprintln!("{nudge}");
            }
            crate::output::emit(&report)?;
        }
        Commands::Assess {
            command:
                AssessCommand::Unused {
                    graph,
                    publication,
                    store,
                    source_root,
                    request,
                    require_complete,
                },
        } => {
            let (graph, store, source_root) =
                resolve_read_paths(graph, publication, store, source_root)?;
            let store =
                store.unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(&graph));
            let request = match request {
                Some(path) => {
                    serde_json::from_slice::<entmoot_contracts::UnusedRequest>(&fs::read(path)?)?
                }
                None => Default::default(),
            };
            let opened = entmoot_workspace::workspace::open_query_bundle_paths(
                &graph,
                &store,
                source_root.as_deref(),
                None,
                false,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
            let report = entmoot_contracts::query::QueryService::unused(&opened.engine, request)?;
            crate::output::emit(&report)?;
            if require_complete && !report.completeness.established() {
                bail!("completeness not established");
            }
        }
        Commands::AuditCalls {
            graph,
            artifacts,
            entrypoint,
            build_context,
            require_accounted,
        } => {
            let graph = load_graph_json(&graph).map_err(|e| anyhow!(e.to_string()))?;
            let audit = entmoot_workspace::resolver_accounting::audit_resolver_manifests(
                &graph, &artifacts,
            )?;
            if let Some(context) = build_context {
                let context = entmoot_contracts::ContentDigest::parse(context)?;
                let reachability = entmoot_workspace::resolver_accounting::audit_reachability(
                    &audit,
                    &context,
                    &entrypoint,
                )?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"accounting":audit,"reachability":reachability})
                    )?
                );
            } else {
                crate::output::emit(&audit)?;
            }
            if require_accounted
                && (audit.static_work_remaining > 0
                    || audit.producer_omissions > 0
                    || !audit.inventory_gaps.is_empty())
            {
                bail!("native accounting incomplete: {} sites require static work; {} producer omissions; {} inventory gaps", audit.static_work_remaining, audit.producer_omissions, audit.inventory_gaps.len());
            }
        }
        Commands::PlanAnalysis {
            request,
            adapter_pass,
        } => {
            let demand: entmoot_contracts::AnalysisDemand =
                serde_json::from_slice(&fs::read(request)?)?;
            let mode = match adapter_pass.as_str() {
                "auto" => entmoot_contracts::AnalysisPassMode::Auto,
                "always" => entmoot_contracts::AnalysisPassMode::Always,
                "never" => entmoot_contracts::AnalysisPassMode::Never,
                _ => bail!("invalid adapter-pass mode"),
            };
            let decision = entmoot_workspace::plan_analysis(demand, mode)?;
            crate::output::emit(&decision)?;
        }
        Commands::Work { queue, command } => cmd_work(&queue, command)?,
        Commands::Collaborate { store, command } => {
            collaborate::run(&resolve_default_database(store)?, command)?
        }
        Commands::Fingerprint => {
            println!(
                "{}",
                serde_json::to_string_pretty(&entmoot_workspace::storage_fingerprint())?
            );
        }
        Commands::ValidateTests {
            graph,
            request,
            out,
        } => {
            let graph = entmoot_workspace::export::load_graph_json(&graph)
                .map_err(|error| anyhow!(error.to_string()))?;
            let request = serde_json::from_slice(&fs::read(request)?)?;
            let evidence = entmoot_workspace::test_validation::validate_tests(&graph, request)?;
            let path = entmoot_workspace::test_validation::write_evidence(&evidence, &out)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"manifest":path,"observations":evidence.observations})
                )?
            );
            if evidence.observations.iter().any(|o| matches!(o, entmoot_contracts::AssessmentObservation::TestExecution { outcome, .. } if *outcome != entmoot_contracts::TestExecutionOutcome::Passed)) {
                bail!("test validation incomplete or failed; see the attributable observations");
            }
        }
        Commands::ValidateCampaignInputs {
            manifest,
            evaluator_digest,
        } => {
            let inputs: entmoot_contracts::RevisionCampaignInputs =
                serde_json::from_slice(&std::fs::read(manifest)?)?;
            let evaluator = entmoot_contracts::ContentDigest::parse(evaluator_digest)
                .map_err(|error| anyhow::anyhow!("invalid evaluator digest: {error}"))?;
            inputs.validate(&entmoot_workspace::implementation_fingerprint(), &evaluator)?;
            crate::output::emit(&inputs.input_digest()?)?;
        }
        Commands::Policy { config } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&load_normalization_policy(&config)?.audit())?
            );
        }
        Commands::Hash {
            lang,
            config,
            file,
            callable_inventory,
            path,
        } => cmd_hash(
            &lang,
            &config,
            file.as_deref(),
            callable_inventory,
            path.as_deref(),
        )?,
        Commands::ValidateGroundTruth {
            config,
            json_out,
            dataset,
        } => cmd_validate_ground_truth(&config, json_out.as_deref(), dataset.as_deref())?,
        Commands::Index {
            structural_only: _,
            root,
            repo,
            out,
            summary_out,
            store,
            artifact_store,
            config,
            revision,
            artifacts,
            replace_series,
        } => cmd_index(
            &root,
            repo.as_deref(),
            &out,
            summary_out.as_deref(),
            store.as_deref(),
            artifact_store.as_deref(),
            &config,
            revision.as_deref(),
            &artifacts,
            replace_series,
        )?,
        Commands::Attach {
            graph,
            out,
            store,
            artifact_store,
            artifacts,
            replace_series,
        } => cmd_attach(
            &graph,
            out.as_deref(),
            store.as_deref(),
            artifact_store.as_deref(),
            &artifacts,
            replace_series,
        )?,
        Commands::Artifact {
            command: ArtifactCmd::Inspect { artifact, manifest },
        } => cmd_artifact_inspect(&artifact, manifest.as_deref())?,
        Commands::Query { store, command } => {
            cmd_query(&resolve_default_database(store)?, command)?
        }
        Commands::Revalidate {
            base_graph,
            head_graph,
            request,
            ledger,
        } => cmd_revalidate(&base_graph, &head_graph, &request, &ledger)?,
        Commands::Assess {
            command:
                AssessCommand::Compatibility {
                    consumer,
                    provider,
                    out,
                },
        } => {
            use entmoot_workspace::compatibility::{CompatibilityCatalog, PackagePublication};
            let cwd = std::env::current_dir()?;
            let (consumer_path, store, source_root) = if let Some(path) = consumer {
                entmoot_workspace::compatibility::publication_input(&path)?
            } else {
                let publication = prepare_current_workspace()?;
                (
                    publication.current.graph,
                    Some(publication.current.store),
                    Some(publication.current.source_root),
                )
            };
            let consumer = PackagePublication::load(&consumer_path, store.as_deref())?;
            eprintln!(
                "consumer={} publication={}",
                consumer_path.display(),
                consumer.id()
            );
            let providers = entmoot_workspace::compatibility::load_providers(
                source_root.as_deref().unwrap_or(&cwd),
                &provider,
            )?;
            let catalog = CompatibilityCatalog::new(consumer, providers)?;
            let result = catalog.assess(catalog.request())?;
            eprintln!(
                "compatibility={:?} packages={} sites={} limitations={}",
                result.verdict,
                result.boundaries.len(),
                result.sites.len(),
                result.limitations.len()
            );
            if !result.sites.is_empty() {
                use entmoot_contracts::query::CompatibilityReason as Reason;
                let mut seeking = 0usize;
                let mut stitched = 0usize;
                let mut bound_fit = 0usize;
                let mut broken = 0usize;
                let mut lock_unproven = 0usize;
                for site in &result.sites {
                    match site.reason {
                        Reason::ConsumerBoundaryUnassessed => seeking += 1,
                        Reason::ProviderExportLocatedContractUnassessed => stitched += 1,
                        Reason::ObservedBoundaryEqualBodyNotAssessed => bound_fit += 1,
                        Reason::LockInstanceUnproven => lock_unproven += 1,
                        Reason::ObservedBoundaryChanged | Reason::ExportAbsentInCompleteSurface => {
                            broken += 1
                        }
                        _ => {}
                    }
                }
                eprintln!(
                    "rung seeking={seeking} stitched={stitched} bound_fit={bound_fit} broken={broken} lock_unproven={lock_unproven}"
                );
                if seeking + stitched > 0 && seeking + stitched >= bound_fit + broken {
                    eprintln!(
                        "tip: seeking/stitched sites found; fit needs assessed boundary + lock (package-root enrich or lock assertion)"
                    );
                }
            }
            if let Some(path) = out {
                let mut receipt = AtomicGraphWrite::begin(&path).map_err(box_err)?;
                receipt.write_json(&result).map_err(box_err)?;
                receipt.commit_new().map_err(box_err).with_context(||
                    "compatibility --out must be a new file; publications and existing receipts are never overwritten")?;
            }
            crate::output::emit(&result)?;
        }
        Commands::Impact(args)
        | Commands::Assess {
            command: AssessCommand::Impact(args),
        } => {
            let ImpactArgs {
                tests,
                test_targets,
                base_graph,
                head_graph,
                config,
                revalidate,
                no_revalidate: _,
                out,
                correspondence_mode,
                subject,
                alias,
            } = args;
            let correspondence = subject_args::build_correspondence_policy(
                correspondence_mode.as_deref(),
                subject.as_deref(),
                &alias,
            )?;
            let explicit_pair = base_graph.zip(head_graph);
            if tests {
                let request = match test_targets {
                    Some(path) => serde_json::from_slice::<entmoot_contracts::TestImpactRequest>(
                        &fs::read(path)?,
                    )?,
                    None => Default::default(),
                };
                let (base, head) = if let Some((base, head)) = explicit_pair {
                    (
                        load_graph_json(&base).map_err(box_err)?,
                        load_graph_json(&head).map_err(box_err)?,
                    )
                } else {
                    let publication = prepare_current_workspace()?;
                    let base = load_graph_json(&publication.current.graph).map_err(box_err)?;
                    let policy = load_normalization_policy(&config)?;
                    let head = entmoot_workspace::workspace::working_tree_graph(
                        &base,
                        &publication.current.source_root,
                        &policy,
                    )?;
                    (base, head)
                };
                let report = entmoot_workspace::query::assess_test_impact(&base, &head, request)?;
                if let Some(path) = out {
                    entmoot_workspace::workspace::write_json(&path, &report)?;
                }
                crate::output::emit(&report)?;
                return Ok(());
            }
            if explicit_pair.is_none() {
                let publication = prepare_current_workspace()?;
                let base = load_graph_json(&publication.current.graph).map_err(box_err)?;
                let policy = load_normalization_policy(&config)?;
                let manifest = entmoot_workspace::workspace::compare_working_tree_impact(
                    &base,
                    &publication.current.source_root,
                    &policy,
                )?;
                if let Some(path) = out {
                    entmoot_workspace::workspace::write_json(&path, &manifest)?;
                }
                crate::output::emit(&manifest)?;
                return Ok(());
            }
            let (base, head) = explicit_pair.context("explicit impact requires both graphs")?;
            if revalidate {
                let destination = match out {
                    Some(path) => path,
                    None => {
                        let root = entmoot_workspace::workspace::workspace_root(
                            &std::env::current_dir()?,
                        )?;
                        let parent =
                            entmoot_workspace::workspace::storage_directory(&root)?.join("impact");
                        fs::create_dir_all(&parent)?;
                        tempfile::Builder::new()
                            .prefix("transition-")
                            .tempdir_in(parent)?
                            .keep()
                            .join("package.json")
                    }
                };
                if destination.exists() {
                    bail!("impact package already exists; use a new --out path");
                }
                let ledger = destination.with_extension("ledger.sqlite3");
                let package = entmoot_workspace::workspace::impact::create_package(
                    entmoot_workspace::workspace::impact::location(&base)?,
                    entmoot_workspace::workspace::impact::location(&head)?,
                    Some(&ledger),
                )?;
                entmoot_workspace::workspace::write_json(&destination, &package)?;
                eprintln!(
                    "package={} ledger={}",
                    destination.display(),
                    ledger.display()
                );
                crate::output::emit(&package)?;
            } else {
                let base_graph = load_graph_json(&base).map_err(box_err)?;
                let head_graph = load_graph_json(&head).map_err(box_err)?;
                let manifest = compare_revision_impact_with_correspondence(
                    &base_graph,
                    &head_graph,
                    correspondence.as_ref(),
                )?;
                if let Some(path) = out {
                    entmoot_workspace::workspace::write_json(&path, &manifest)?;
                }
                crate::output::emit(&manifest)?;
            }
        }
        Commands::Assess {
            command:
                AssessCommand::Overlay {
                    base_graph,
                    head_graph,
                    name,
                    path_prefix,
                    out,
                },
        } => {
            let base = load_inventory_for_paths(&base_graph, &path_prefix).map_err(box_err)?;
            let head = load_inventory_for_paths(&head_graph, &path_prefix).map_err(box_err)?;
            let request = entmoot_contracts::query::InventoryOverlayRequest {
                base_publication: base_graph.display().to_string(),
                head_publication: head_graph.display().to_string(),
                name_filters: name,
                path_prefixes: path_prefix,
            };
            let report = compare_inventory_overlay(&base, &head, &request)?;
            if let Some(path) = out {
                entmoot_workspace::workspace::write_json(&path, &report)?;
            }
            crate::output::emit(&report)?;
        }
        Commands::Deps {
            command:
                DepsCommand::Materialize {
                    subject,
                    version,
                    from,
                    root,
                    python_environment,
                },
        } => {
            let subject = subject_args::parse_subject(&subject)?;
            let root = root.unwrap_or(std::env::current_dir()?);
            let meta = serde_json::json!({
                "schema": "entmoot.deps_materialization.v1", "subject": subject,
                "version": version, "source": from.display().to_string(),
            });
            let dest = entmoot_workspace::materialize::materialize(
                &entmoot_workspace::workspace::storage_directory(&root)?.join("deps"),
                &subject,
                &version,
                &from,
                &serde_json::to_vec_pretty(&meta)?,
                python_environment.as_deref(),
                Default::default(),
            )?;
            let mut meta = meta;
            meta["path"] = serde_json::json!(dest);
            crate::output::emit(&meta)?;
        }
    }
    Ok(())
}

fn workspace_options(
    full: bool,
    require_native: bool,
    plan_only: bool,
    structural_only: bool,
    seconds: u64,
) -> entmoot_workspace::workspace::BindOptions {
    entmoot_workspace::workspace::BindOptions {
        canon: false,
        subject: None,
        subject_aliases: Vec::new(),
        full,
        require_native,
        plan_only,
        structural_only,
        rust_profile: Default::default(),
        budget: entmoot_workspace::ProducerBudget {
            wall: std::time::Duration::from_secs(seconds),
            ..Default::default()
        },
    }
}

fn resolve_read_paths(
    graph: Option<PathBuf>,
    publication: Option<PathBuf>,
    store: Option<PathBuf>,
    source: Option<PathBuf>,
) -> Result<(PathBuf, Option<PathBuf>, Option<PathBuf>)> {
    if let Some(graph) = graph {
        eprintln!(
            "graph={} store={} source_root={}",
            graph.display(),
            store
                .as_deref()
                .unwrap_or(&entmoot_workspace::workspace::default_store_path(&graph))
                .display(),
            source
                .as_deref()
                .unwrap_or(Path::new("<graph root>"))
                .display()
        );
        return Ok((graph, store, source));
    }
    let discovered = match publication {
        Some(path) => entmoot_workspace::workspace::read_publication(&path)?,
        None => match source.as_deref() {
            Some(root) => entmoot_workspace::workspace::prepare_workspace(root)?,
            None => prepare_current_workspace()?,
        },
    };
    let location = discovered.current;
    let store = store.unwrap_or(location.store);
    let source = source.unwrap_or(location.source_root);
    Ok((location.graph, Some(store), Some(source)))
}

fn cmd_work(path: &Path, command: WorkCmd) -> Result<()> {
    match cmd_work_item(path, command) {
        Ok(item) => println!("{}", serde_json::to_string_pretty(&item)?),
        Err(error) => {
            let kind = error
                .downcast_ref::<entmoot_workspace::AnalysisWorkError>()
                .map(entmoot_workspace::AnalysisWorkError::code)
                .unwrap_or("invalid_input");
            println!(
                "{}",
                serde_json::json!({
                    "schema_version":"entmoot.analysis-work.error.v1",
                    "status":"error","kind":kind,"message":error.to_string()
                })
            );
            return Err(error);
        }
    }
    Ok(())
}

fn cmd_work_item(path: &Path, command: WorkCmd) -> Result<entmoot_contracts::AnalysisWorkItem> {
    use entmoot_workspace::{AnalysisWorkQueue, ManifestUnverified, SqliteAnalysisWorkQueue};
    if let WorkCmd::Get { id } = &command {
        let queue = SqliteAnalysisWorkQueue::open_read_only(path)
            .with_context(|| format!("open analysis work queue read-only {}", path.display()))?;
        return queue.get(id)?.ok_or_else(|| {
            entmoot_workspace::AnalysisWorkError::Invalid("unknown analysis work id".into()).into()
        });
    }
    let mut queue = SqliteAnalysisWorkQueue::open(path)
        .with_context(|| format!("open analysis work queue {}", path.display()))?;
    let item = match command {
        WorkCmd::Enqueue { decision } => {
            let value: entmoot_contracts::AnalysisDecision =
                serde_json::from_slice(&fs::read(&decision)?)?;
            queue.enqueue(value)?
        }
        WorkCmd::Claim { id, worker } => {
            let value: entmoot_contracts::ProducerRef =
                serde_json::from_slice(&fs::read(&worker)?)?;
            queue.claim(&id, value)?
        }
        WorkCmd::ArtifactReady {
            id,
            fencing_token,
            manifest,
            artifact,
            graph,
        } => {
            let value: ArtifactManifest = serde_json::from_slice(&fs::read(&manifest)?)?;
            let snapshot = load_graph_json(&graph).map_err(box_err)?.snapshot;
            let bytes = fs::read(&artifact)?;
            let verified = ManifestUnverified::new(value).verify(&bytes, &snapshot)?;
            queue.artifact_ready(&id, &fencing_token, &verified)?
        }
        WorkCmd::Ingested { id, fencing_token } => queue.ingested(&id, &fencing_token)?,
        WorkCmd::Fail {
            id,
            fencing_token,
            reason,
        } => queue.fail(&id, &fencing_token, reason)?,
        WorkCmd::Cancel { id } => queue.cancel(&id)?,
        WorkCmd::Get { id } => queue
            .get(&id)?
            .ok_or_else(|| anyhow!("unknown analysis work id"))?,
    };
    Ok(item)
}

fn cmd_revalidate(base: &Path, head: &Path, request: &Path, ledger: &Path) -> Result<()> {
    let request: entmoot_contracts::TransitionRevalidationRequest =
        serde_json::from_slice(&fs::read(request)?)?;
    let base = load_graph_json(base).map_err(box_err)?;
    let head = load_graph_json(head).map_err(box_err)?;
    match entmoot_workspace::publish_revalidation_ledger(&base, &head, &request, ledger) {
        Ok(receipt) => println!("{}", serde_json::to_string_pretty(&receipt)?),
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({
                    "schema_version":"entmoot.revalidation.error.v1",
                    "status":"error","kind":error.code(),"message":error.to_string()
                })
            );
            return Err(error.into());
        }
    }
    Ok(())
}

fn cmd_query(store: &Path, command: QueryCmd) -> Result<()> {
    let repository = SqliteKnowledgeRepository::open_read_only(store)
        .with_context(|| format!("open knowledge store read-only {}", store.display()))?;
    let result = match command {
        QueryCmd::Assertions { subject } => {
            if subject.trim().is_empty() {
                bail!("query subject must not be empty");
            }
            serde_json::to_value(
                assertions_result(&repository, &subject).context("query assertions by subject")?,
            )?
        }
        QueryCmd::Relations { address } => {
            let bytes =
                fs::read(&address).with_context(|| format!("read {}", address.display()))?;
            let address_value: Address = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse Address from {}", address.display()))?;
            address_value.validate().context("validate query Address")?;
            serde_json::to_value(
                relations_result(&repository, &address_value)
                    .context("query relations by address")?,
            )?
        }
        QueryCmd::RelationClaims { relation_edge } => serde_json::to_value(
            expert_relation_claims_result(&repository, &relation_edge)
                .context("query relation claims by relation edge")?,
        )?,
        QueryCmd::Proposition { proposition } => serde_json::to_value(
            expert_proposition_result(&repository, &proposition).context("query proposition")?,
        )?,
        QueryCmd::Resources { resource } => serde_json::to_value(
            resources_result(&repository, &resource).context("query logical resource")?,
        )?,
        QueryCmd::Unresolved { id } => serde_json::to_value(unresolved_result(&repository, &id)?)?,
        QueryCmd::Artifact { id } => serde_json::to_value(artifact_result(&repository, &id)?)?,
        QueryCmd::Trace { id, limit } => {
            serde_json::to_value(trace_result(&repository, &id, limit)?)?
        }
        QueryCmd::Owner { id, limit } => {
            serde_json::to_value(owner_result(&repository, &id, limit)?)?
        }
        QueryCmd::Dependency {
            collection,
            record_id,
            limit,
        } => serde_json::to_value(dependency_result(
            &repository,
            entmoot_workspace::KnowledgeRecordRef::new(collection, record_id),
            limit,
        )?)?,
    };
    serde_json::to_writer_pretty(io::stdout().lock(), &result).context("write query result")?;
    println!();
    Ok(())
}

fn box_err(err: Box<dyn std::error::Error>) -> anyhow::Error {
    anyhow!("{err}")
}

fn load_manifest_json(text: &str) -> Result<ArtifactManifest> {
    let m: ArtifactManifest = serde_json::from_str(text).context("parse manifest")?;
    m.validate_structure()
        .context("manifest validation error")?;
    Ok(m)
}

fn load_normalization_policy(config: &str) -> Result<entmoot_contracts::StructuralHashPolicy> {
    let policy = if let Some(path) = config.strip_prefix('@') {
        serde_json::from_slice(&fs::read(path).with_context(|| format!("read policy {path}"))?)?
    } else {
        policy_from_config_name(config).map_err(anyhow::Error::msg)?
    };
    Ok(policy)
}

fn cmd_hash(
    lang: &str,
    config_name: &str,
    file: Option<&Path>,
    callable_inventory: bool,
    path: Option<&str>,
) -> Result<()> {
    let policy = load_normalization_policy(config_name)?;
    let source = read_source(file)?;
    let hasher = StructuralHasher::new();
    if callable_inventory {
        let granular = hasher
            .compute_granular_hash_at(&source, lang, path, None, Some(&policy))
            .map_err(anyhow::Error::msg)?;
        let record = build_file_index(&granular);
        print!(
            "{}",
            serde_json::to_string(&record).context("serialize callable inventory")?
        );
    } else {
        let hash = hasher
            .compute_hash(&source, lang, Some(&policy))
            .map_err(anyhow::Error::msg)?;
        print!("{hash}");
    }
    Ok(())
}

fn read_source(file: Option<&Path>) -> Result<String> {
    match file {
        Some(path) => fs::read_to_string(path).with_context(|| format!("read {}", path.display())),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf).context("read stdin")?;
            Ok(buf)
        }
    }
}

fn cmd_validate_ground_truth(
    config_name: &str,
    json_out: Option<&Path>,
    dataset_path: Option<&Path>,
) -> Result<()> {
    let path = dataset_path
        .map(Path::to_path_buf)
        .or_else(resolve_dataset_path)
        .context(
            "Ground truth dataset not found. Set STRUCTURAL_HASHING_GROUND_TRUTH or pass --dataset PATH",
        )?;

    let policy = policy_from_config_name(config_name).map_err(anyhow::Error::msg)?;
    let dataset = GroundTruthDataset::load(&path)
        .map_err(|e| anyhow!("Failed to load dataset {}: {e}", path.display()))?;
    let case_count = dataset.len();
    let pb = ProgressBar::new(case_count as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner} validating {pos}/{len}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    pb.set_message("ground truth");
    let validator = GroundTruthValidator::new(dataset);
    let implementation = entmoot_workspace::implementation_fingerprint();
    let report = validator.validate(&policy, implementation.clone());
    report
        .validate(&implementation)
        .map_err(anyhow::Error::msg)?;
    pb.finish_and_clear();

    use entmoot_contracts::NormalizationCaseOutcome;
    let count = |outcome| {
        report
            .cases
            .iter()
            .filter(|case| case.outcome() == outcome)
            .count()
    };
    eprintln!(
        "Normalization fixtures: {} passed, {} mismatched, {} not comparable, {} unqualified ({case_count} total)",
        count(NormalizationCaseOutcome::Passed), count(NormalizationCaseOutcome::Mismatch),
        count(NormalizationCaseOutcome::NotComparable), count(NormalizationCaseOutcome::Unqualified),
    );
    let json = serde_json::to_string_pretty(&report).context("JSON serialize")?;
    println!("{json}");

    if let Some(out) = json_out {
        fs::write(out, &json).with_context(|| format!("write {}", out.display()))?;
        info!("Wrote report to {}", out.display());
    }

    if !report.passes() {
        process::exit(2);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_index(
    root: &Path,
    repo: Option<&str>,
    out: &Path,
    summary_out: Option<&Path>,
    store: Option<&Path>,
    artifact_store: Option<&Path>,
    config: &str,
    expected_revision: Option<&str>,
    artifacts: &[PathBuf],
    replace_series: bool,
) -> Result<()> {
    let index_started = std::time::Instant::now();
    let policy = load_normalization_policy(config)?;
    info!(root = %root.display(), ?repo, "indexing repository");
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    pb.set_message("scanning and hashing");
    let mut graph = build_topology_from_repo_with_options(
        root,
        TopologyBuildOptions {
            policy: Some(&policy),
            repository: repo,
        },
    )
    .map_err(box_err)?;
    let inventory_elapsed = index_started.elapsed();
    pb.finish_and_clear();
    info!(
        repository_id = %graph.snapshot.repository_id,
        revision = %graph.snapshot.revision,
        ?graph.snapshot.source_state,
        "snapshot from local git"
    );
    if let Some(expected) = expected_revision {
        if graph.snapshot.revision != expected {
            bail!(
                "index revision mismatch: requested {expected}, but source snapshot is {}; index the requested checkout or omit --revision",
                graph.snapshot.revision
            );
        }
    }
    for manifest in artifacts {
        let stats = ingest_artifact_manifest(&mut graph, manifest, replace_series)
            .map_err(|e| anyhow!("artifact ingest failed for {}: {e}", manifest.display()))?;
        info!(
            artifact = %manifest.display(),
            parsed = stats.parsed,
            resolver_observations = stats.resolver_observations,
            resolver_targets = stats.resolver_targets,
            excluded = stats.excluded,
            file_attached = stats.file_attached,
            callable_attached = stats.callable_attached,
            call_site_attached = stats.call_site_attached,
            resource_attached = stats.resource_attached,
            unresolved = stats.unresolved,
            invalid = stats.invalid,
            assertions = stats.assertions,
            "artifact ingested"
        );
    }

    let store_path = store
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(out));
    entmoot_workspace::workspace::retention::check_store_admission(&store_path)?;
    // Hold the database lifecycle lease across archival and publication. GC
    // cannot retire a referenced native object in the gap between those steps.
    let mut repository = SqliteKnowledgeRepository::open(&store_path)
        .with_context(|| format!("open knowledge store {}", store_path.display()))?;
    archive_artifacts(
        artifacts,
        artifact_store
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(|| default_artifact_store_path(&store_path))?,
    )?;
    let extraction_started = std::time::Instant::now();
    entmoot_workspace::workspace::retention::check_store_admission(&store_path)?;
    let knowledge = KnowledgeStore::extract_from_graph(&graph);
    let extraction_elapsed = extraction_started.elapsed();
    let synchronization_started = std::time::Instant::now();
    let buffer = PublicationBuffer::default();
    let mut batches = KnowledgeStoreBatches::new(knowledge, buffer)
        .context("prepare bounded knowledge batches")?;
    let sync = repository
        .replace_owner_buffered(
            &format!("snapshot:{}", graph.snapshot.repository_id),
            &mut batches,
            buffer,
        )
        .with_context(|| format!("commit knowledge store {}", store_path.display()))?;
    repository.store_inventory(&graph, None)?;
    let synchronization_elapsed = synchronization_started.elapsed();
    let graph_write_started = std::time::Instant::now();
    if !sync.outcome.is_unchanged() || !out.is_file() {
        write_graph_json(&graph, out).map_err(box_err)?;
    }
    let graph_write_elapsed = graph_write_started.elapsed();
    let summary_write_started = std::time::Instant::now();
    if let Some(summary_path) = summary_out {
        let mut writer = AtomicGraphWrite::begin(summary_path)
            .map_err(box_err)
            .with_context(|| format!("begin graph summary {}", summary_path.display()))?;
        writer
            .write_json(&graph.summary())
            .map_err(box_err)
            .with_context(|| format!("write graph summary {}", summary_path.display()))?;
        writer
            .commit()
            .map_err(box_err)
            .with_context(|| format!("publish graph summary {}", summary_path.display()))?;
    }
    let summary_write_elapsed = summary_write_started.elapsed();
    info!(
        inventory_ms = inventory_elapsed.as_millis(),
        knowledge_extract_ms = extraction_elapsed.as_millis(),
        knowledge_sync_ms = synchronization_elapsed.as_millis(),
        graph_write_ms = graph_write_elapsed.as_millis(),
        summary_write_ms = summary_write_elapsed.as_millis(),
        total_ms = index_started.elapsed().as_millis(),
        "index stage timings"
    );
    info!(
        nodes = graph.nodes.len(),
        edges = graph.edges.len(),
        out = %out.display(),
        store = %store_path.display(),
        publication_outcome = sync.outcome.as_str(),
        publication_digest = %sync.outcome.digest(),
        knowledge_inserted = sync.publication.inserted_records,
        knowledge_owner_additions = sync.publication.added_owners,
        knowledge_owner_removals = sync.publication.removed_owners,
        knowledge_batches = sync.batches,
        peak_batch_records = sync.peak_batch_records,
        peak_batch_bytes = sync.peak_batch_bytes,
        "wrote graph"
    );

    Ok(())
}

fn cmd_attach(
    graph_path: &Path,
    out: Option<&Path>,
    store: Option<&Path>,
    artifact_store: Option<&Path>,
    artifacts: &[PathBuf],
    replace_series: bool,
) -> Result<()> {
    let graph = load_graph_json(graph_path).map_err(box_err)?;
    let store_path = store
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entmoot_workspace::workspace::default_store_path(graph_path));
    let knowledge = if store_path.exists() {
        match SqliteKnowledgeRepository::open(&store_path) {
            Ok(repository) => {
                if repository.is_initialized().with_context(|| {
                    format!(
                        "inspect knowledge store initialization {}",
                        store_path.display()
                    )
                })? {
                    repository.load().with_context(|| {
                        format!("load durable knowledge from {}", store_path.display())
                    })?
                } else {
                    KnowledgeStore::extract_from_graph(&graph)
                }
            }
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context(format!("open knowledge store {}", store_path.display())));
            }
        }
    } else {
        KnowledgeStore::extract_from_graph(&graph)
    };
    let mut graph = knowledge.into_inventory(entmoot_workspace::inventory_skeleton(&graph));
    graph
        .validate()
        .context("validate graph rematerialized from durable knowledge")?;
    let mut attachment_receipts = Vec::new();
    for manifest in artifacts {
        let stats = ingest_artifact_manifest(&mut graph, manifest, replace_series)
            .map_err(|e| anyhow!("artifact ingest failed for {}: {e}", manifest.display()))?;
        info!(
            artifact = %manifest.display(),
            parsed = stats.parsed,
            resolver_observations = stats.resolver_observations,
            resolver_targets = stats.resolver_targets,
            excluded = stats.excluded,
            file_attached = stats.file_attached,
            callable_attached = stats.callable_attached,
            call_site_attached = stats.call_site_attached,
            resource_attached = stats.resource_attached,
            unresolved = stats.unresolved,
            invalid = stats.invalid,
            assertions = stats.assertions,
            "artifact attached"
        );
        attachment_receipts.push(stats);
    }
    entmoot_workspace::workspace::retention::check_store_admission(&store_path)?;
    let mut repository = SqliteKnowledgeRepository::open(&store_path)
        .with_context(|| format!("open knowledge store {}", store_path.display()))?;
    archive_artifacts(
        artifacts,
        artifact_store
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(|| default_artifact_store_path(&store_path))?,
    )?;
    let buffer = PublicationBuffer::default();
    entmoot_workspace::workspace::retention::check_store_admission(&store_path)?;
    let mut batches =
        KnowledgeStoreBatches::new(KnowledgeStore::extract_from_graph(&graph), buffer)
            .context("prepare bounded knowledge batches")?;
    let sync = repository
        .replace_owner_buffered(
            &format!("snapshot:{}", graph.snapshot.repository_id),
            &mut batches,
            buffer,
        )
        .with_context(|| format!("commit knowledge store {}", store_path.display()))?;
    repository.store_inventory(&graph, None)?;
    let destination = out.unwrap_or(graph_path);
    if !sync.outcome.is_unchanged() || !destination.is_file() {
        write_graph_json(&graph, destination).map_err(box_err)?;
    }
    info!(
        store = %store_path.display(),
        publication_outcome = sync.outcome.as_str(),
        publication_digest = %sync.outcome.digest(),
        knowledge_inserted = sync.publication.inserted_records,
        knowledge_owner_additions = sync.publication.added_owners,
        knowledge_owner_removals = sync.publication.removed_owners,
        knowledge_batches = sync.batches,
        "committed durable knowledge"
    );
    println!(
        "{}",
        serde_json::to_string(&entmoot_contracts::ArtifactAttachReceipt {
            schema_version: "entmoot.attach.v1".into(),
            snapshot: graph.snapshot.clone(),
            artifacts: attachment_receipts,
            limitations: vec![
                "attachment counts are not runtime validation or dispatch completeness".into(),
                format!(
                    "publication_outcome={} digest={}",
                    sync.outcome.as_str(),
                    sync.outcome.digest()
                ),
            ],
        })?
    );
    Ok(())
}

fn default_artifact_store_path(store_path: &Path) -> Result<PathBuf> {
    Ok(entmoot_workspace::workspace::artifact_store_path(
        store_path,
    )?)
}

fn archive_artifacts(manifests: &[PathBuf], root: PathBuf) -> Result<()> {
    if manifests.is_empty() {
        return Ok(());
    }
    let repository = FilesystemArtifactObjectRepository::new(&root);
    for manifest in manifests {
        repository.archive_manifest(manifest).with_context(|| {
            format!(
                "archive native artifact from {} into {}",
                manifest.display(),
                root.display()
            )
        })?;
    }
    Ok(())
}

fn cmd_artifact_inspect(artifact_path: &Path, manifest_path: Option<&Path>) -> Result<()> {
    let bytes =
        fs::read(artifact_path).with_context(|| format!("read {}", artifact_path.display()))?;

    let (manifest, used_manifest) = if let Some(mp) = manifest_path {
        let text =
            fs::read_to_string(mp).with_context(|| format!("read manifest {}", mp.display()))?;
        (
            Some(load_manifest_json(&text)?),
            Some(mp.display().to_string()),
        )
    } else {
        let sibling = format!("{}.manifest.json", artifact_path.display());
        if Path::new(&sibling).exists() {
            let text =
                fs::read_to_string(&sibling).with_context(|| format!("read manifest {sibling}"))?;
            (Some(load_manifest_json(&text)?), Some(sibling))
        } else {
            (None, None)
        }
    };

    let inspected = if let Some(ref m) = manifest {
        m.validate_structure()
            .context("manifest validation error")?;
        m.verify_content_digest(&bytes)
            .context("artifact.digest_mismatch")?;
        adapter_for_manifest(m).inspect(&bytes)
    } else if SarifAdapter.inspect(&bytes).is_ok() {
        SarifAdapter.inspect(&bytes)
    } else if ResourceInventoryAdapter.inspect(&bytes).is_ok() {
        ResourceInventoryAdapter.inspect(&bytes)
    } else {
        SemanticClusterAdapter.inspect(&bytes)
    }
    .map_err(|e| anyhow!("inspect failed: {e}"))?;

    println!("artifact: {}", artifact_path.display());
    if let Some(mp) = used_manifest {
        println!("manifest: {mp}");
    } else {
        println!("manifest: (none — digest/repo checks skipped)");
    }
    println!("format_hint: {}", inspected.format_hint);
    if let Some(name) = &inspected.producer_name {
        println!(
            "producer: {name}{}",
            inspected
                .producer_version
                .as_ref()
                .map(|v| format!(" {v}"))
                .unwrap_or_default()
        );
    }
    println!("findings: {}", inspected.finding_count);
    println!("locations: {}", inspected.location_count);
    println!("traces: {}", inspected.trace_count);
    println!("documents: {}", inspected.document_count);
    println!("occurrences: {}", inspected.occurrence_count);
    if !inspected.diagnostics.is_empty() {
        println!("diagnostics: {}", inspected.diagnostics.len());
        for d in &inspected.diagnostics {
            println!("  [{}] {}: {}", d.severity_label(), d.code, d.message);
        }
    }

    if let Some(ref m) = manifest {
        match adapter_for_manifest(m).normalize(&bytes, m) {
            Ok(norm) => {
                println!("normalized_findings: {}", norm.findings.len());
                println!("normalized_locations: {}", norm.locations.len());
                println!("normalized_traces: {}", norm.traces.len());
                println!("normalized_trace_steps: {}", norm.trace_steps.len());
                println!("normalized_resources: {}", norm.resources.len());
                println!("normalized_components: {}", norm.components.len());
                println!(
                    "normalized_component_dependencies: {}",
                    norm.component_dependencies.len()
                );
                println!(
                    "normalized_component_vulnerabilities: {}",
                    norm.component_vulnerabilities.len()
                );
                println!(
                    "normalized_bom_compositions: {}",
                    norm.bom_compositions.len()
                );
                println!("normalized_relation_edges: {}", norm.relation_edges.len());
                println!("normalized_relation_claims: {}", norm.relation_claims.len());
                println!("capability_observations: {}", norm.capabilities.len());
                for capability in &norm.capabilities {
                    println!(
                        "  capability.{:?}: {:?} ({})",
                        capability.capability, capability.outcome, capability.basis
                    );
                }
                let excluded = norm
                    .diagnostics
                    .iter()
                    .filter(|d| d.code == "artifact.scope_violation")
                    .count();
                let errors = norm
                    .diagnostics
                    .iter()
                    .filter(|d| matches!(d.severity, entmoot_workspace::DiagnosticSeverity::Error))
                    .count();
                println!("diagnostics_excluded: {excluded}");
                println!("diagnostics_errors: {errors}");
                for d in &norm.diagnostics {
                    println!("  [{}] {}: {}", d.severity_label(), d.code, d.message);
                }
            }
            Err(e) => bail!("normalize failed: {e}"),
        }
    }
    Ok(())
}
