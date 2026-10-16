//! Auditable producer demand, separate from artifact evidence or graph truth.
use crate::{Address, ArtifactId, ContentDigest, EvidenceRef, ProducerRef, SourceSnapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Explicit execution-request policy. None of these modes launches a producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisPassMode {
    Auto,
    Always,
    Never,
}

/// Qualification/environment/value disposition, never numeric confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisGate {
    Accepted,
    Rejected,
    Unknown,
}

/// Independent scheduling question answered by an attributable gate claim.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisGateKind {
    Qualification,
    Environment,
    MarginalValue,
}

/// Evidence-backed gate input. It is an assessor claim, not a scanner fact or
/// proof that the referenced artifact establishes the disposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisGateClaim {
    pub kind: AnalysisGateKind,
    pub disposition: AnalysisGate,
    pub claimant: ProducerRef,
    pub method_id: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
}

/// Declared scheduling inputs. Reused artifacts still require independent
/// provenance validation; this demand cannot establish graph truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisDemand {
    pub policy_producer: ProducerRef,
    pub snapshot: SourceSnapshot,
    pub subject: Address,
    pub method_id: String,
    /// Exact source, build, policy and task-input root for this demand.
    pub input_root: ContentDigest,
    pub prior_input_root: Option<ContentDigest>,
    pub current_artifact: Option<ArtifactId>,
    /// Exactly one claim for each required gate.
    pub gates: Vec<AnalysisGateClaim>,
    pub relevant_unknown: bool,
    /// Conditions under which the supplied input roots and gate evidence apply.
    pub assumptions: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisAction {
    ReuseCandidate,
    RequestArtifact,
    Deferred,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisDecisionReason {
    ExistingInputMatch,
    ExplicitRequest,
    PolicyDisabled,
    QualificationRequired,
    EnvironmentRequired,
    ValueGateClosed,
    InputsChanged,
    RelevantUnknown,
    NoRelevantDemand,
}

/// Scheduling receipt, not an execution receipt. Only validated artifact
/// reingestion changes knowledge; the decision retains every supplied input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisDecision {
    pub demand: AnalysisDemand,
    pub mode: AnalysisPassMode,
    pub action: AnalysisAction,
    pub reason: AnalysisDecisionReason,
    /// Scheduling limitations carried into every downstream work request.
    pub limitations: Vec<String>,
}
