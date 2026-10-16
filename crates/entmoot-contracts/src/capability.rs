//! Scoped statements about what an artifact did and could establish.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stable, provider-neutral capability vocabulary. Extend deliberately; never use
/// a missing observation as evidence that a capability produced a negative result.
///
/// Authorization *enforcement* is not a core capability. When authz was not
/// established, emit [`CapabilityOutcome::NotEvaluated`] or
/// [`CapabilityOutcome::RequiresExternalEvidence`] (often with
/// [`EvidenceCapability::RuntimeExposure`]) plus limitation text—do not mint
/// `Authorization*` variants or allow/deny graph truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceCapability {
    FindingDetection,
    OrderedCodeFlow,
    SemanticSymbols,
    SymbolDefinitions,
    SymbolReferences,
    SemanticRoleClassification,
    StaticCallReachability,
    RuntimeExecutionReachability,
    Exploitability,
    DeploymentState,
    RuntimeExposure,
    Ownership,
    RemediationVerification,
    ComponentInventory,
    ComponentDependencies,
    VulnerabilityAffects,
    VexAnalysis,
    BomCompleteness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutcome {
    /// The artifact contains at least one observation for this capability.
    Observed,
    /// The producer evaluated the declared scope and explicitly reported none.
    NoObservationInCoveredScope,
    /// The format/producer does not provide this evidence class.
    NotProvidedByProducer,
    /// Establishing this requires another evidence plane (usually runtime or human).
    RequiresExternalEvidence,
    /// The producer supports it, but this run/configuration did not evaluate it.
    NotEvaluated,
    /// The artifact or scope is insufficient to distinguish the above outcomes.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityObservation {
    pub artifact_id: String,
    pub scope_universe_id: String,
    pub capability: EvidenceCapability,
    pub outcome: CapabilityOutcome,
    /// Versioned adapter rule or explicit provider declaration supporting the outcome.
    pub basis: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limitation: Option<String>,
}
