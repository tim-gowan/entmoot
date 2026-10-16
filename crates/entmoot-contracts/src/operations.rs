//! Portable local collaboration and execution contracts. No filesystem locators.
use crate::{ContentDigest, ImplementationFingerprint, SourceSnapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
mod scheduler;
pub use scheduler::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePublication {
    pub workspace_id: String,
    pub branch: Option<String>,
    pub snapshot: SourceSnapshot,
    pub view: ContentDigest,
    pub generation: u64,
    pub publisher: String,
    /// Retired registrations retain their generation to prevent stale recreation.
    pub active: bool,
    /// Optional independently captured Git tree; immutable with this generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_capture: Option<ContentDigest>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Inventory,
    Binding,
    Canon,
    Revalidation,
    Conflicts,
    Integration,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationSpec {
    pub repository_id: String,
    pub idempotency_key: String,
    pub owner: String,
    pub kind: OperationKind,
    /// Pins exact immutable publications; never mutable branch names.
    pub views: Vec<ContentDigest>,
    /// Existing operation IDs. A batch may refer to earlier members only.
    pub prerequisites: Vec<ContentDigest>,
    /// Full request plus source/tool/configuration identity, interpreted by the worker.
    pub input: serde_json::Value,
    pub implementation: ImplementationFingerprint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl OperationState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub id: ContentDigest,
    pub spec: OperationSpec,
    pub state: OperationState,
    /// Monotonically increasing attempt fence; pause/cancel revoke a worker.
    pub generation: u64,
    pub worker: Option<String>,
    pub lease_until_ms: Option<u64>,
    pub checkpoint_sequence: u64,
    /// Worker-owned staged progress; never published evidence.
    pub checkpoint: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub reason: Option<String>,
}

/// Immutable, individually bounded output committed with a worker checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationOutput {
    pub sequence: u64,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationOutputPage {
    pub id: ContentDigest,
    pub outputs: Vec<OperationOutput>,
    /// Exclusive cursor for the next read, unchanged if no output is available.
    pub next_sequence: u64,
    /// True only when the operation is terminal and no later output remains.
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationControl {
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConflictFindingKind {
    StructuralOverlap,
    PotentialInteraction,
    IncompatibleEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConflictFinding {
    pub kind: ConflictFindingKind,
    pub path: Option<String>,
    pub subjects: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BranchConflictReport {
    pub schema_version: String,
    pub implementation: ImplementationFingerprint,
    pub base: SourceSnapshot,
    pub left: SourceSnapshot,
    pub right: SourceSnapshot,
    pub findings: Vec<ConflictFinding>,
    pub git: Option<GitMergeObservation>,
    /// No findings never establishes semantic compatibility.
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitMergeObservation {
    pub merge_bases: Vec<String>,
    /// clean, conflict or unavailable; not a semantic compatibility verdict.
    pub status: String,
    pub details: String,
}

/// A bounded raw working-tree observation, distinct from index-only selection or
/// the result of running clean filters. Git object IDs retain their external form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitTreeCapture {
    pub view: ContentDigest,
    pub source: ProducerSource,
    pub parent_commit: String,
    pub object_format: String,
    pub publisher: String,
    pub files: Vec<GitTreeFile>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitTreeFile {
    pub path: String,
    /// Git tree mode (100644, 100755 or 120000); submodules are unsupported.
    pub mode: String,
    /// Raw file bytes or symlink target bytes.
    pub content_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConflictBatchRequest {
    /// Frozen execution choice; a resumed batch cannot silently change its mode.
    #[serde(default)]
    pub git_merge: bool,
    pub base_view: ContentDigest,
    pub left: WorkspacePublication,
    pub peers: Vec<WorkspacePublication>,
}

/// One explicitly ordered integration, not all possible merge permutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationRequest {
    pub initial: WorkspacePublication,
    pub members: Vec<WorkspacePublication>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationStepReport {
    pub schema_version: String,
    pub implementation: ImplementationFingerprint,
    pub git_version: String,
    /// Zero-based member ordinal in the frozen request.
    pub member_index: usize,
    pub member: WorkspacePublication,
    pub previous_commit: String,
    /// Present only after a clean merge. Private synthetic commits never update refs.
    pub integrated_commit: Option<String>,
    pub git: GitMergeObservation,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationResult {
    /// clean or conflict; clean describes this order's textual merges only.
    pub status: String,
    pub integrated_members: usize,
    pub attempted_members: usize,
    pub untested_members: usize,
    /// Last completely integrated prefix; not a commit published to the source repo.
    pub integrated_commit: String,
}

/// Frozen conditional policies over two retained same-repository publications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidationRequest {
    pub base_view: ContentDigest,
    pub head_view: ContentDigest,
    pub units: Vec<crate::RevalidationUnit>,
}

/// Completed ledger locator and scope. Per-policy receipts use operation outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidationResult {
    pub ledger_view: ContentDigest,
    pub base_snapshot: SourceSnapshot,
    pub head_snapshot: SourceSnapshot,
    pub completed_units: usize,
    pub semantic_records: usize,
    pub unselected_artifact_ids: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidationRecordState {
    pub ledger_view: ContentDigest,
    pub record: crate::KnowledgeRecordRef,
    /// None means unreported in this ledger, not disproven or absent in the repo.
    pub visible: Option<bool>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BindingFamily {
    Rust,
    Oxc,
    Typescript,
    Python,
    Go,
    Roslyn,
    Java,
    Clang,
}
impl BindingFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Oxc => "oxc",
            Self::Typescript => "typescript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Roslyn => "roslyn",
            Self::Java => "java",
            Self::Clang => "clang",
        }
    }
}

/// Host-attested isolated tool/dependency configuration. The host must preserve
/// this environment throughout the job; this token is not an independently
/// verified inventory of external or negative compiler inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingPlan {
    pub families: Vec<BindingFamily>,
    pub environment: ContentDigest,
    /// sysroot must be None: host filesystem locators stay in host setup.
    pub rust_profile: crate::RustBuildProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingOutcome {
    pub family: BindingFamily,
    pub environment: ContentDigest,
    pub receipt: crate::NativeReceipt,
    pub attachment: crate::ArtifactIngestStats,
    /// Failed environment/budget attempts retain supplied facts and pause; they
    /// do not advance the family cursor or establish a complete binding pass.
    pub completed: bool,
}

/// Source identity includes the complete visible path set, even for committed inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerSource {
    pub snapshot: SourceSnapshot,
    pub files_digest: ContentDigest,
}

/// The intended registry update is frozen at submission, not selected on resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerPublication {
    pub workspace_id: String,
    pub branch: Option<String>,
    pub publisher: String,
    pub expected_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "producer", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProducerTask {
    Binding {
        view: ContentDigest,
        plan: BindingPlan,
    },
    Inventory {
        policy: crate::StructuralHashPolicy,
    },
    /// Structural canon; native facts require separately qualified production.
    Canon {
        view: ContentDigest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerRequest {
    pub source: ProducerSource,
    pub publication: ProducerPublication,
    pub task: ProducerTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollaborationRead {
    /// Current admission policy and bounded waiting/active observations; no work starts.
    Scheduler {
        repository_id: String,
    },
    RevalidationRecord {
        id: ContentDigest,
        record: crate::KnowledgeRecordRef,
    },
    /// Freeze explicit order; unlike pair capture this never sorts or deduplicates.
    CaptureIntegration {
        repository_id: String,
        workspace: String,
        members: Vec<String>,
    },
    /// Digest-only observation using the hosting composition's explicit source root.
    Source {
        repository_id: String,
    },
    Capture {
        #[serde(default)]
        git_merge: bool,
        repository_id: String,
        base_workspace: String,
        workspace: String,
        /// Empty selects every other registered workspace in this repository.
        peers: Vec<String>,
    },
    Workspaces {
        repository_id: String,
    },
    Status {
        id: ContentDigest,
    },
    Outputs {
        id: ContentDigest,
        after_sequence: u64,
        limit: usize,
    },
    Compare {
        base: ContentDigest,
        left: ContentDigest,
        right: ContentDigest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollaborationWrite {
    /// Compare-and-swap admission policy. Existing workers are not revoked.
    ConfigureScheduler {
        repository_id: String,
        expected_generation: u64,
        policy: SchedulerPolicy,
    },
    RunBinding {
        id: ContentDigest,
        worker: String,
        environment: ContentDigest,
        steps: usize,
        timeout_seconds: u64,
        #[serde(default)]
        wall_time_ms: Option<u64>,
    },
    SubmitRevalidation {
        request: RevalidationRequest,
        key: String,
        owner: String,
    },
    RunRevalidation {
        id: ContentDigest,
        worker: String,
        /// One step checkpoints a producer policy; final publication is another step.
        steps: usize,
        /// Cumulative publication-receipt record allowance, adjustable on resume.
        max_records: usize,
        #[serde(default)]
        wall_time_ms: Option<u64>,
    },
    SubmitIntegration {
        request: IntegrationRequest,
        key: String,
        owner: String,
    },
    RunIntegration {
        id: ContentDigest,
        worker: String,
        steps: usize,
        /// Compressed intermediate Git objects, adjustable on resume (up to 1 GiB).
        max_staged_bytes: u64,
        #[serde(default)]
        wall_time_ms: Option<u64>,
    },
    /// Observe source locally, then atomically attach the frozen Git capture by CAS.
    CaptureGit {
        workspace_id: String,
        expected_generation: u64,
        publisher: String,
        max_files: usize,
        max_bytes: u64,
    },
    SubmitProducer {
        request: ProducerRequest,
        key: String,
        owner: String,
    },
    /// Local execution requires a source root supplied by the hosting composition.
    RunProducer {
        id: ContentDigest,
        worker: String,
        /// Bounded computation/checkpoint steps; publication consumes one step.
        steps: usize,
        /// Files per parsing, syntax extraction, call stitching or canon-fold step.
        #[serde(default = "default_producer_files_per_step")]
        files_per_step: usize,
        /// Cooperative admission deadline, checked between steps; no mid-file kill.
        #[serde(default)]
        wall_time_ms: Option<u64>,
    },
    Retire {
        workspace_id: String,
        expected_generation: u64,
    },
    Publish {
        publication: WorkspacePublication,
        expected_generation: u64,
    },
    Submit {
        request: ConflictBatchRequest,
        key: String,
        owner: String,
    },
    Control {
        id: ContentDigest,
        control: OperationControl,
    },
    Run {
        id: ContentDigest,
        worker: String,
        pairs: usize,
    },
}

pub const fn default_producer_files_per_step() -> usize {
    128
}
