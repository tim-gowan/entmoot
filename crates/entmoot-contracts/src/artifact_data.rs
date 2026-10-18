//! Normalized provider data shared by topology and evidence adapters.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_pointer: Option<String>,
}

impl ArtifactDiagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            artifact_id: None,
            provider_pointer: None,
        }
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            artifact_id: None,
            provider_pointer: None,
        }
    }

    pub fn info(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Info,
            message: message.into(),
            artifact_id: None,
            provider_pointer: None,
        }
    }

    pub fn with_artifact(mut self, artifact_id: &str) -> Self {
        self.artifact_id = Some(artifact_id.into());
        self
    }

    pub fn with_pointer(mut self, pointer: &str) -> Self {
        self.provider_pointer = Some(pointer.into());
        self
    }

    pub fn severity_label(&self) -> &'static str {
        match self.severity {
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Info => "info",
        }
    }
}

use crate::{
    BomComposition, CapabilityObservation, ComponentDependency, ComponentOccurrence,
    ComponentVulnerability, FindingObservation, LocationRecord, RelationClaim, RelationEdge,
    Resolution, ResolverCoverageObservation, ResourceRecord, TraceRecord, TraceStepRecord,
    UnresolvedRelation,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Manifest(#[from] crate::ManifestValidationError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct InspectedArtifact {
    pub format_hint: String,
    pub producer_name: Option<String>,
    pub producer_version: Option<String>,
    pub finding_count: usize,
    pub location_count: usize,
    pub trace_count: usize,
    pub document_count: usize,
    pub occurrence_count: usize,
    pub diagnostics: Vec<ArtifactDiagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct NormalizedArtifact {
    pub assessment_evidence: Option<crate::AssessmentEvidence>,
    /// Provider records parsed before one-to-many normalization.
    pub source_record_count: usize,
    pub capabilities: Vec<CapabilityObservation>,
    pub findings: Vec<FindingObservation>,
    pub locations: Vec<LocationRecord>,
    pub traces: Vec<TraceRecord>,
    pub trace_steps: Vec<TraceStepRecord>,
    pub resources: Vec<ResourceRecord>,
    pub components: Vec<ComponentOccurrence>,
    pub component_dependencies: Vec<ComponentDependency>,
    pub component_vulnerabilities: Vec<ComponentVulnerability>,
    pub bom_compositions: Vec<BomComposition>,
    pub relation_edges: Vec<RelationEdge>,
    pub relation_claims: Vec<RelationClaim>,
    pub unresolved_relations: Vec<UnresolvedRelation>,
    pub resolutions: Vec<Resolution>,
    pub resolver_coverages: Vec<ResolverCoverageObservation>,
    /// Native binding attempts retained until inventory and scope validation.
    pub resolver_observations: Vec<crate::ResolverObservation>,
    /// Package boundary evidence stays in the immutable resolver object; retained
    /// here only for inventory validation before attachment.
    pub resolver_package_api: Option<crate::package_api::PackageApiEvidence>,
    /// Adapter-policy interpretations that require attachment before becoming
    /// occurrence-scoped assertions. Raw provider facts remain in the artifact.
    pub provider_role_hints: Vec<ProviderRoleHint>,
    pub diagnostics: Vec<ArtifactDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRoleHint {
    pub finding_observation_id: String,
    pub location_id: String,
    pub role: String,
    pub provider_pointer: String,
    pub policy_id: String,
}
