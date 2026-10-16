//! Typestate wrappers for manifest verification before attachment.

use entmoot_contracts::{ArtifactManifest, ManifestValidationError, SourceSnapshot};

/// Manifest loaded from disk but not yet verified against bytes/snapshot.
#[derive(Debug, Clone)]
pub struct ManifestUnverified {
    manifest: ArtifactManifest,
}

/// Manifest that passed structure, snapshot, and content-digest checks.
///
/// ```compile_fail
/// use entmoot_engine::ManifestVerified;
/// let forged = ManifestVerified { manifest: todo!() };
/// ```
#[derive(Debug, Clone)]
pub struct ManifestVerified {
    manifest: ArtifactManifest,
}

impl ManifestUnverified {
    pub fn new(manifest: ArtifactManifest) -> Self {
        Self { manifest }
    }

    /// Reference-engine trust transition: no gate may be skipped or reordered.
    pub fn verify(
        self,
        bytes: &[u8],
        graph_snapshot: &SourceSnapshot,
    ) -> Result<ManifestVerified, ManifestValidationError> {
        self.manifest.validate_structure()?;
        self.manifest.verify_against_snapshot(graph_snapshot)?;
        self.manifest.verify_content_digest(bytes)?;
        Ok(ManifestVerified {
            manifest: self.manifest,
        })
    }
}

impl ManifestVerified {
    pub fn manifest(&self) -> &ArtifactManifest {
        &self.manifest
    }

    pub fn into_manifest(self) -> ArtifactManifest {
        self.manifest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        ArtifactFormat, ArtifactProducer, ScopeCompleteness, ScopeUniverse, SourceState,
    };

    #[test]
    fn verified_transition_checks_snapshot_structure_and_bytes_together() {
        let snapshot = SourceSnapshot {
            repository_id: "github.com/acme/app".into(),
            revision: "a".repeat(40),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "native".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            snapshot.clone(),
            ScopeUniverse::from_selectors(
                "test",
                vec![],
                vec![],
                vec![],
                ScopeCompleteness::Partial,
                None,
            ),
            entmoot_contracts::blake3_hex(b"facts"),
            None,
        )
        .unwrap();
        assert!(ManifestUnverified::new(manifest.clone())
            .verify(b"facts", &snapshot)
            .is_ok());
        assert!(ManifestUnverified::new(manifest.clone())
            .verify(b"changed", &snapshot)
            .is_err());
        let mut other = snapshot.clone();
        other.revision = "b".repeat(40);
        assert!(ManifestUnverified::new(manifest.clone())
            .verify(b"facts", &other)
            .is_err());
        let mut corrupt = manifest;
        corrupt.artifact_id = "invalid".into();
        assert!(ManifestUnverified::new(corrupt)
            .verify(b"facts", &snapshot)
            .is_err());
    }
}
