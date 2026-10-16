//! Source snapshot identity (committed vs working tree).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::identity::is_immutable_revision;
use crate::ids::{ContentDigest, IdParseError};
use crate::source_subject::{SourceSubject, SourceSubjectError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Committed,
    WorkingTree,
}

/// Immutable observation of repository contents at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    /// Materialization / estate hint (folder basename, remote URL). Prefer [`Self::subject`]
    /// for cross-publication compares; `repository_id` alone is not the identity floor.
    pub repository_id: String,
    /// Full immutable commit hash when known.
    pub revision: String,
    pub source_state: SourceState,
    /// Required for `working_tree`. For `committed`, git tree OID or path→digest manifest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estate_id: Option<String>,
    /// Declared publication subject (PURL or repository lineage). Additive on older pubs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SourceSubject>,
    /// Additional subjects that identify the same publication (mirrors, aliases).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subject_aliases: Vec<SourceSubject>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotValidationError {
    #[error("repository_id must be non-empty")]
    EmptyRepositoryId,
    #[error("estate_id must be non-empty when present")]
    EmptyEstateId,
    #[error("invalid snapshot_digest: {0}")]
    InvalidSnapshotDigest(IdParseError),
    #[error("working_tree snapshot requires snapshot_digest")]
    MissingWorkingTreeDigest,
    #[error("revision must be a full commit hash, not a branch name: {0}")]
    NonImmutableRevision(String),
    #[error(transparent)]
    InvalidSubject(#[from] SourceSubjectError),
}

impl SourceSnapshot {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        if self.repository_id.trim().is_empty() {
            return Err(SnapshotValidationError::EmptyRepositoryId);
        }
        if !is_immutable_revision(&self.revision) {
            return Err(SnapshotValidationError::NonImmutableRevision(
                self.revision.clone(),
            ));
        }
        if self.source_state == SourceState::WorkingTree {
            match &self.snapshot_digest {
                Some(d) if !d.is_empty() => {}
                _ => return Err(SnapshotValidationError::MissingWorkingTreeDigest),
            }
        }
        if let Some(digest) = &self.snapshot_digest {
            if let Err(error) = ContentDigest::parse(digest.clone()) {
                if !is_immutable_revision(digest) {
                    return Err(SnapshotValidationError::InvalidSnapshotDigest(error));
                }
            }
        }
        if self
            .estate_id
            .as_deref()
            .is_some_and(|estate_id| estate_id.trim().is_empty())
        {
            return Err(SnapshotValidationError::EmptyEstateId);
        }
        if let Some(subject) = &self.subject {
            subject.validate()?;
        }
        for alias in &self.subject_aliases {
            alias.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_tree_requires_digest() {
        let s = SourceSnapshot {
            repository_id: "github.com/acme/app".into(),
            revision: "f356a09207c7a9550eb6fc4c3945e081922cf998".into(),
            source_state: SourceState::WorkingTree,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        assert!(matches!(
            s.validate(),
            Err(SnapshotValidationError::MissingWorkingTreeDigest)
        ));
    }

    #[test]
    fn committed_ok_without_digest() {
        let s = SourceSnapshot {
            repository_id: "github.com/acme/app".into(),
            revision: "f356a09207c7a9550eb6fc4c3945e081922cf998".into(),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn snapshot_rejects_empty_repository_and_malformed_digest() {
        let mut snapshot = SourceSnapshot {
            repository_id: "".into(),
            revision: "f356a09207c7a9550eb6fc4c3945e081922cf998".into(),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        assert_eq!(
            snapshot.validate(),
            Err(SnapshotValidationError::EmptyRepositoryId)
        );
        snapshot.repository_id = "repo/example".into();
        snapshot.snapshot_digest = Some("blake3:short".into());
        assert!(matches!(
            snapshot.validate(),
            Err(SnapshotValidationError::InvalidSnapshotDigest(_))
        ));
    }
}
