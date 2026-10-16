//! Unified artifact manifest (`logic.artifact.v1`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::identity::{
    blake3_hex, derive_artifact_id, derive_artifact_series_id, strip_digest_prefix,
    ArtifactIdentityInput, IDENTITY_SCHEMA_VERSION,
};
use crate::ids::{ArtifactId, ContentDigest, IdParseError};
use crate::paths::InvalidRepoPath;
use crate::scope::ScopeUniverse;
use crate::snapshot::{SourceSnapshot, SourceState};

pub const ARTIFACT_MANIFEST_SCHEMA: &str = "logic.artifact.v1";

/// Complete reference to one artifact instance and its original content bytes.
/// A digest commits bytes; this reference does not claim those bytes are retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub content_digest: ContentDigest,
}

impl ArtifactRef {
    pub fn from_manifest(manifest: &ArtifactManifest) -> Result<Self, ManifestValidationError> {
        manifest.validate_structure()?;
        Ok(Self {
            artifact_id: ArtifactId::parse(&manifest.artifact_id)
                .map_err(ManifestValidationError::InvalidArtifactId)?,
            content_digest: ContentDigest::parse(&manifest.content_digest)?,
        })
    }

    pub fn validate(&self) -> Result<(), IdParseError> {
        ArtifactId::parse(self.artifact_id.as_str())?;
        ContentDigest::parse(self.content_digest.as_str())?;
        Ok(())
    }

    pub fn verify(&self, manifest: &ArtifactManifest) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())?;
        if self.artifact_id.as_str() != manifest.artifact_id
            || self.content_digest.as_str() != manifest.content_digest
        {
            return Err(
                "artifact reference does not match the manifest identity and content digest".into(),
            );
        }
        Ok(())
    }
}

/// Exact processing counters for one validated artifact attachment invocation.
/// Reattachment is explicit; newly derived counts can be zero on an idempotent retry.
/// Parsed provider records, location joins, and resolver attempts are different
/// denominators. These counters are not a mutually exclusive partition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIngestStats {
    pub artifact_id: String,
    pub artifact_digest: String,
    pub already_present: bool,
    pub parsed: usize,
    pub excluded: usize,
    pub file_attached: usize,
    pub callable_attached: usize,
    pub call_site_attached: usize,
    pub resource_attached: usize,
    pub unresolved: usize,
    pub invalid: usize,
    pub assertions: usize,
    /// Native resolver attempts parsed, separate from location joins.
    pub resolver_observations: usize,
    /// Provider target addresses retained as Resolution records.
    pub resolver_targets: usize,
    /// Callables receiving an occurrence-scoped resolved hash in this invocation.
    pub resolved_identities: usize,
}

/// Emitted only after the durable attachment and graph publication succeed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactAttachReceipt {
    pub schema_version: String,
    pub snapshot: SourceSnapshot,
    pub artifacts: Vec<ArtifactIngestStats>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    AssessmentEvidence,
    Sarif21,
    SemanticCluster,
    ResourceInventory,
    Resolver,
    #[serde(rename = "cyclonedx")]
    CycloneDx,
    OsvScanner,
}

impl ArtifactFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactFormat::AssessmentEvidence => "assessment_evidence",
            ArtifactFormat::Sarif21 => "sarif21",
            ArtifactFormat::SemanticCluster => "semantic_cluster",
            ArtifactFormat::ResourceInventory => "resource_inventory",
            ArtifactFormat::Resolver => "resolver",
            ArtifactFormat::CycloneDx => "cyclonedx",
            ArtifactFormat::OsvScanner => "osv_scanner",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactProducer {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruleset_id: Option<String>,
    /// Logical config class for series identity (defaults to ruleset_id or "default").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_class: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub schema_version: String,
    pub identity_schema_version: String,
    pub artifact_id: String,
    pub artifact_series_id: String,
    pub format: ArtifactFormat,
    pub producer: ArtifactProducer,
    pub snapshot: SourceSnapshot,
    pub scope: ScopeUniverse,
    pub content_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_repositories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction_policy_id: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestValidationError {
    #[error("unsupported manifest schema_version: {0}")]
    UnsupportedVersion(String),
    #[error("unsupported identity_schema_version: {0}")]
    UnsupportedIdentityVersion(String),
    #[error("artifact_id does not match derived instance id")]
    ArtifactIdMismatch,
    #[error("artifact_series_id does not match derived series id")]
    SeriesIdMismatch,
    #[error(transparent)]
    Snapshot(#[from] crate::snapshot::SnapshotValidationError),
    #[error(transparent)]
    Scope(#[from] crate::scope::ScopeValidationError),
    #[error(transparent)]
    Path(#[from] InvalidRepoPath),
    #[error("content digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("invalid content_digest: {0}")]
    InvalidContentDigest(#[from] IdParseError),
    #[error("invalid artifact_id: {0}")]
    InvalidArtifactId(IdParseError),
    #[error("invalid artifact_series_id: {0}")]
    InvalidArtifactSeriesId(IdParseError),
    #[error("invalid config_digest: {0}")]
    InvalidConfigDigest(IdParseError),
    #[error("artifact producer name must be non-empty")]
    EmptyProducerName,
    #[error("artifact producer optional string `{0}` must be non-empty when present")]
    EmptyProducerField(&'static str),
    #[error("artifact optional string `{0}` must be non-empty when present")]
    EmptyOptionalField(&'static str),
    #[error("additional_repositories must be non-empty and unique")]
    InvalidAdditionalRepositories,
    #[error("repository mismatch: manifest={manifest}, graph={graph}")]
    RepositoryMismatch { manifest: String, graph: String },
    #[error("revision mismatch: manifest={manifest}, graph={graph}")]
    RevisionMismatch { manifest: String, graph: String },
    #[error("snapshot_digest mismatch: manifest={manifest}, graph={graph}")]
    SnapshotDigestMismatch { manifest: String, graph: String },
}

impl ArtifactManifest {
    pub fn derive_ids(
        format: ArtifactFormat,
        producer: &ArtifactProducer,
        snapshot: &SourceSnapshot,
        scope: &ScopeUniverse,
        content_digest: &str,
        config_digest: Option<&str>,
    ) -> (String, String) {
        let version = producer.version.as_deref().unwrap_or("");
        let config = config_digest.unwrap_or("");
        let snap = snapshot.snapshot_digest.as_deref().unwrap_or("");
        let state = match snapshot.source_state {
            SourceState::Committed => "committed",
            SourceState::WorkingTree => "working_tree",
        };
        let artifact_id = derive_artifact_id(&ArtifactIdentityInput {
            format: format.as_str(),
            producer_name: &producer.name,
            producer_version: version,
            repository_id: &snapshot.repository_id,
            revision: &snapshot.revision,
            source_state: state,
            snapshot_digest: snap,
            content_digest,
            config_digest: config,
            scope_universe_id: &scope.scope_universe_id,
        });
        let config_class = producer
            .config_class
            .as_deref()
            .or(producer.ruleset_id.as_deref())
            .unwrap_or("default");
        let series_id = derive_artifact_series_id(
            format.as_str(),
            &producer.name,
            config_class,
            &scope.scope_universe_id,
            &snapshot.repository_id,
        );
        (artifact_id, series_id)
    }

    pub fn build(
        format: ArtifactFormat,
        producer: ArtifactProducer,
        snapshot: SourceSnapshot,
        scope: ScopeUniverse,
        content_digest: String,
        config_digest: Option<String>,
    ) -> Result<Self, ManifestValidationError> {
        let (artifact_id, artifact_series_id) = Self::derive_ids(
            format,
            &producer,
            &snapshot,
            &scope,
            &content_digest,
            config_digest.as_deref(),
        );
        let m = Self {
            schema_version: ARTIFACT_MANIFEST_SCHEMA.into(),
            identity_schema_version: IDENTITY_SCHEMA_VERSION.into(),
            artifact_id,
            artifact_series_id,
            format,
            producer,
            snapshot,
            scope,
            content_digest,
            config_digest,
            artifact_path: None,
            additional_repositories: Vec::new(),
            generated_at: None,
            observed_at: None,
            ingested_at: None,
            retention_class: None,
            redaction_policy_id: None,
        };
        m.validate_structure()?;
        Ok(m)
    }

    pub fn validate_structure(&self) -> Result<(), ManifestValidationError> {
        if self.schema_version != ARTIFACT_MANIFEST_SCHEMA {
            return Err(ManifestValidationError::UnsupportedVersion(
                self.schema_version.clone(),
            ));
        }
        if self.identity_schema_version != IDENTITY_SCHEMA_VERSION {
            return Err(ManifestValidationError::UnsupportedIdentityVersion(
                self.identity_schema_version.clone(),
            ));
        }
        self.snapshot.validate()?;
        self.scope.validate()?;
        ArtifactId::parse(self.artifact_id.clone())
            .map_err(ManifestValidationError::InvalidArtifactId)?;
        ArtifactId::parse(self.artifact_series_id.clone())
            .map_err(ManifestValidationError::InvalidArtifactSeriesId)?;
        ContentDigest::parse(&self.content_digest)?;
        if let Some(digest) = &self.config_digest {
            ContentDigest::parse(digest.clone())
                .map_err(ManifestValidationError::InvalidConfigDigest)?;
        }
        if self.producer.name.trim().is_empty() {
            return Err(ManifestValidationError::EmptyProducerName);
        }
        for (field, value) in [
            ("version", self.producer.version.as_deref()),
            ("information_uri", self.producer.information_uri.as_deref()),
            ("ruleset_id", self.producer.ruleset_id.as_deref()),
            ("config_class", self.producer.config_class.as_deref()),
        ] {
            if value.is_some_and(|text| text.trim().is_empty()) {
                return Err(ManifestValidationError::EmptyProducerField(field));
            }
        }
        if let Some(path) = &self.artifact_path {
            crate::normalize_repo_path(path)?;
        }
        let unique_repositories: std::collections::HashSet<_> =
            self.additional_repositories.iter().collect();
        if unique_repositories.len() != self.additional_repositories.len()
            || self
                .additional_repositories
                .iter()
                .any(|repository| repository.trim().is_empty())
        {
            return Err(ManifestValidationError::InvalidAdditionalRepositories);
        }
        for (field, value) in [
            ("generated_at", self.generated_at.as_deref()),
            ("observed_at", self.observed_at.as_deref()),
            ("ingested_at", self.ingested_at.as_deref()),
            ("retention_class", self.retention_class.as_deref()),
            ("redaction_policy_id", self.redaction_policy_id.as_deref()),
        ] {
            if value.is_some_and(|text| text.trim().is_empty()) {
                return Err(ManifestValidationError::EmptyOptionalField(field));
            }
        }
        let (expected_id, expected_series) = Self::derive_ids(
            self.format,
            &self.producer,
            &self.snapshot,
            &self.scope,
            &self.content_digest,
            self.config_digest.as_deref(),
        );
        if self.artifact_id != expected_id {
            return Err(ManifestValidationError::ArtifactIdMismatch);
        }
        if self.artifact_series_id != expected_series {
            return Err(ManifestValidationError::SeriesIdMismatch);
        }
        Ok(())
    }

    pub fn verify_content_digest(&self, bytes: &[u8]) -> Result<(), ManifestValidationError> {
        let actual = blake3_hex(bytes);
        if strip_digest_prefix(&actual) != strip_digest_prefix(&self.content_digest) {
            return Err(ManifestValidationError::DigestMismatch {
                expected: self.content_digest.clone(),
                actual,
            });
        }
        Ok(())
    }

    pub fn verify_against_snapshot(
        &self,
        graph_snapshot: &SourceSnapshot,
    ) -> Result<(), ManifestValidationError> {
        if self.snapshot.repository_id != graph_snapshot.repository_id {
            return Err(ManifestValidationError::RepositoryMismatch {
                manifest: self.snapshot.repository_id.clone(),
                graph: graph_snapshot.repository_id.clone(),
            });
        }
        if self.snapshot.revision != graph_snapshot.revision {
            return Err(ManifestValidationError::RevisionMismatch {
                manifest: self.snapshot.revision.clone(),
                graph: graph_snapshot.revision.clone(),
            });
        }
        if self.snapshot.source_state != graph_snapshot.source_state
            || self.snapshot.snapshot_digest != graph_snapshot.snapshot_digest
        {
            return Err(ManifestValidationError::SnapshotDigestMismatch {
                manifest: format!(
                    "{:?}:{}",
                    self.snapshot.source_state,
                    self.snapshot.snapshot_digest.as_deref().unwrap_or("")
                ),
                graph: format!(
                    "{:?}:{}",
                    graph_snapshot.source_state,
                    graph_snapshot.snapshot_digest.as_deref().unwrap_or("")
                ),
            });
        }
        Ok(())
    }

    pub fn path_in_scope(&self, path: &str) -> Result<bool, ManifestValidationError> {
        Ok(self.scope.path_in_scope(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::{ScopeCompleteness, ScopeUniverse};

    fn sample() -> ArtifactManifest {
        let snapshot = SourceSnapshot {
            repository_id: "github.com/acme/app".into(),
            revision: "f356a09207c7a9550eb6fc4c3945e081922cf998".into(),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let scope = ScopeUniverse::from_selectors(
            "review",
            vec!["typescript".into()],
            vec![],
            vec!["target/**".into()],
            ScopeCompleteness::Partial,
            None,
        );
        let producer = ArtifactProducer {
            name: "SnykCode".into(),
            version: Some("1.0".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        };
        ArtifactManifest::build(
            ArtifactFormat::Sarif21,
            producer,
            snapshot,
            scope,
            blake3_hex(b"hello"),
            None,
        )
        .unwrap()
    }

    #[test]
    fn id_mismatch_rejected() {
        let mut m = sample();
        m.artifact_id = "0".repeat(64);
        assert!(matches!(
            m.validate_structure(),
            Err(ManifestValidationError::ArtifactIdMismatch)
        ));
    }

    #[test]
    fn unsupported_identity_version_is_rejected_before_id_validation() {
        let mut manifest = sample();
        manifest.identity_schema_version = "logic.identity.v999".into();
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestValidationError::UnsupportedIdentityVersion(_))
        ));
    }

    #[test]
    fn bare_or_malformed_content_digest_is_rejected() {
        let mut manifest = sample();
        manifest.content_digest = "abc".into();
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestValidationError::InvalidContentDigest(_))
        ));
    }

    #[test]
    fn malformed_configuration_digest_and_escaping_artifact_path_are_rejected() {
        let mut manifest = sample();
        manifest.config_digest = Some("abc".into());
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestValidationError::InvalidConfigDigest(_))
        ));

        let mut manifest = sample();
        manifest.artifact_path = Some("../outside.sarif".into());
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestValidationError::Path(_))
        ));
    }

    #[test]
    fn empty_producer_and_duplicate_repository_context_are_rejected() {
        let mut manifest = sample();
        manifest.producer.name.clear();
        assert_eq!(
            manifest.validate_structure(),
            Err(ManifestValidationError::EmptyProducerName)
        );

        let mut manifest = sample();
        manifest.additional_repositories = vec!["repo/two".into(), "repo/two".into()];
        assert_eq!(
            manifest.validate_structure(),
            Err(ManifestValidationError::InvalidAdditionalRepositories)
        );
    }

    #[test]
    fn artifact_format_is_a_series_identity_dimension() {
        let sarif = sample();
        let mut resolver = sarif.clone();
        resolver.format = ArtifactFormat::Resolver;
        let (_, resolver_series) = ArtifactManifest::derive_ids(
            resolver.format,
            &resolver.producer,
            &resolver.snapshot,
            &resolver.scope,
            &resolver.content_digest,
            resolver.config_digest.as_deref(),
        );
        assert_ne!(sarif.artifact_series_id, resolver_series);
    }

    #[test]
    fn working_tree_without_digest_fails() {
        let snapshot = SourceSnapshot {
            repository_id: "github.com/acme/app".into(),
            revision: "f356a09207c7a9550eb6fc4c3945e081922cf998".into(),
            source_state: SourceState::WorkingTree,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let scope = ScopeUniverse::from_selectors(
            "review",
            vec![],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let err = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "entmoot-native-rust".into(),
                version: Some("0.1.0".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            snapshot,
            scope,
            blake3_hex(b"x"),
            None,
        );
        assert!(err.is_err());
    }

    #[test]
    fn digest_ok() {
        let m = sample();
        assert!(m.verify_content_digest(b"hello").is_ok());
    }

    #[test]
    fn snapshot_source_state_must_match() {
        let m = sample();
        let mut graph_snapshot = m.snapshot.clone();
        graph_snapshot.source_state = SourceState::WorkingTree;
        graph_snapshot.snapshot_digest = Some("blake3:dirty".into());
        assert!(matches!(
            m.verify_against_snapshot(&graph_snapshot),
            Err(ManifestValidationError::SnapshotDigestMismatch { .. })
        ));
    }
}
