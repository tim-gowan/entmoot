//! Artifact inspect / normalize pipeline (Wave 0 contracts; provisional normalize).

pub mod assessment;
mod cyclonedx;
mod osv_scanner;
pub mod resolver;
mod resource_inventory;
mod sarif;
mod semantic_cluster;

#[cfg(test)]
mod test_manifest;

use entmoot_contracts::ArtifactManifest;

pub use cyclonedx::CycloneDxAdapter;
pub use entmoot_contracts::artifact_data::{
    ArtifactDiagnostic, ArtifactError, DiagnosticSeverity, InspectedArtifact, NormalizedArtifact,
    ProviderRoleHint,
};
pub use osv_scanner::OsvScannerAdapter;
pub use resolver::ResolverArtifactAdapter;
pub use resource_inventory::ResourceInventoryAdapter;
pub use sarif::SarifAdapter;
pub use semantic_cluster::SemanticClusterAdapter;

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
