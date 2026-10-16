//! Artifact inspect / normalize pipeline (Wave 0 contracts; provisional normalize).

pub(crate) mod assessment;
mod cyclonedx;
mod diagnostic;
mod osv_scanner;
pub(crate) mod resolver;
mod resource_inventory;
mod sarif;
mod semantic_cluster;

#[cfg(test)]
mod test_manifest;

use entmoot_contracts::ArtifactManifest;
use thiserror::Error;

pub use cyclonedx::CycloneDxAdapter;
pub use diagnostic::{ArtifactDiagnostic, DiagnosticSeverity};
pub use osv_scanner::OsvScannerAdapter;
pub use resolver::ResolverArtifactAdapter;
pub use resource_inventory::ResourceInventoryAdapter;
pub use sarif::SarifAdapter;
pub use semantic_cluster::SemanticClusterAdapter;

use entmoot_contracts::{
    BomComposition, CapabilityObservation, ComponentDependency, ComponentOccurrence,
    ComponentVulnerability, FindingObservation, LocationRecord, RelationClaim, RelationEdge,
    Resolution, ResolverCoverageObservation, ResourceRecord, TraceRecord, TraceStepRecord,
    UnresolvedRelation,
};

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Manifest(#[from] entmoot_contracts::ManifestValidationError),
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
    pub assessment_evidence: Option<entmoot_contracts::AssessmentEvidence>,
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
    pub resolver_observations: Vec<entmoot_contracts::ResolverObservation>,
    /// Package boundary evidence stays in the immutable resolver object; retained
    /// here only for inventory validation before attachment.
    pub resolver_package_api: Option<entmoot_contracts::package_api::PackageApiEvidence>,
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

pub trait ArtifactAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError>;
    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError>;
}

/// Decode UTF-8 / UTF-16 BOM-aware text from artifact bytes.
pub fn decode_text_bytes(bytes: &[u8]) -> Result<String, ArtifactError> {
    if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        if body.len() % 2 != 0 {
            return Err(ArtifactError::Message(
                "utf-16 le decode: odd byte length".into(),
            ));
        }
        let words: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        return String::from_utf16(&words)
            .map_err(|e| ArtifactError::Message(format!("utf-16 le decode: {e}")));
    }
    if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        if body.len() % 2 != 0 {
            return Err(ArtifactError::Message(
                "utf-16 be decode: odd byte length".into(),
            ));
        }
        let words: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        return String::from_utf16(&words)
            .map_err(|e| ArtifactError::Message(format!("utf-16 be decode: {e}")));
    }
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|e| ArtifactError::Message(format!("utf-8 decode: {e}")))?;
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

pub fn adapter_for_manifest(manifest: &ArtifactManifest) -> Box<dyn ArtifactAdapter> {
    match manifest.format {
        entmoot_contracts::ArtifactFormat::AssessmentEvidence => {
            Box::new(assessment::AssessmentEvidenceAdapter)
        }
        entmoot_contracts::ArtifactFormat::Sarif21 => Box::new(SarifAdapter),
        entmoot_contracts::ArtifactFormat::SemanticCluster => Box::new(SemanticClusterAdapter),
        entmoot_contracts::ArtifactFormat::ResourceInventory => Box::new(ResourceInventoryAdapter),
        entmoot_contracts::ArtifactFormat::Resolver => Box::new(ResolverArtifactAdapter),
        entmoot_contracts::ArtifactFormat::CycloneDx => Box::new(CycloneDxAdapter),
        entmoot_contracts::ArtifactFormat::OsvScanner => Box::new(OsvScannerAdapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_with_an_incomplete_code_unit_is_rejected() {
        for bytes in [[0xff, 0xfe, b'a'], [0xfe, 0xff, b'a']] {
            let error = decode_text_bytes(&bytes).expect_err("odd UTF-16 payload must fail");
            assert!(error.to_string().contains("odd byte length"));
        }
    }
}
