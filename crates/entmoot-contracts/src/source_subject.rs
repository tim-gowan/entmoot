//! Declared publication subject and compare-time correspondence.
//!
//! Materialization paths (zip extract folders, clone remotes) are not the identity
//! floor. A subject is a PURL coordinate or a repository id with optional path;
//! aliases assert equivalence (forks, mirrors) and never rewrite historical hashes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable subject for a source publication (package or repository lineage).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSubject {
    /// Ecosystem package coordinate without version (version lives on revision/lock).
    Package {
        /// Package-URL coordinate, typically without version (`pkg:pypi/pydantic`).
        coordinate_purl: String,
    },
    /// VCS or logical repository identity, optionally scoped to a monorepo path.
    Repository {
        /// Canonical repository id or URL.
        id: String,
        /// Optional monorepo / Bazel package path within the repository.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

impl SourceSubject {
    /// Parse and canonicalize a versionless package coordinate at the input boundary.
    pub fn package(value: &str) -> Result<Self, SourceSubjectError> {
        use std::str::FromStr;
        if value.trim().is_empty() {
            return Err(SourceSubjectError::EmptyPackageCoordinate);
        }
        let parsed = packageurl::PackageUrl::from_str(value.trim())
            .map_err(|_| SourceSubjectError::InvalidPackageCoordinate(value.into()))?;
        if parsed.version().is_some() {
            return Err(SourceSubjectError::VersionedPackageCoordinate);
        }
        Ok(Self::Package {
            coordinate_purl: parsed.to_string(),
        })
    }

    pub fn validate(&self) -> Result<(), SourceSubjectError> {
        match self {
            Self::Package { coordinate_purl } => {
                if &Self::package(coordinate_purl)? != self {
                    return Err(SourceSubjectError::InvalidPackageCoordinate(
                        coordinate_purl.clone(),
                    ));
                }
            }
            Self::Repository { id, path } => {
                if id.trim().is_empty() {
                    return Err(SourceSubjectError::EmptyRepositoryId);
                }
                if path.as_ref().is_some_and(|p| p.trim().is_empty()) {
                    return Err(SourceSubjectError::EmptyRepositoryPath);
                }
            }
        }
        Ok(())
    }

    /// Compact wire form for receipts and join diagnostics.
    pub fn display(&self) -> String {
        match self {
            Self::Package { coordinate_purl } => coordinate_purl.clone(),
            Self::Repository { id, path: None } => id.clone(),
            Self::Repository {
                id,
                path: Some(path),
            } => format!("{id}:{path}"),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceSubjectError {
    #[error("package coordinate_purl must be non-empty")]
    EmptyPackageCoordinate,
    #[error("package coordinate_purl must be a canonical versionless Package URL: {0}")]
    InvalidPackageCoordinate(String),
    #[error("package subject must not include a version; select a revision or materialization version separately")]
    VersionedPackageCoordinate,
    #[error("repository subject id must be non-empty")]
    EmptyRepositoryId,
    #[error("repository subject path must be non-empty when present")]
    EmptyRepositoryPath,
}

/// How two publications' occurrences are joined for impact-class compares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrespondencePolicy {
    /// Join strategy.
    pub mode: CorrespondenceMode,
    /// Optional subject override for this compare (asserted for both sides).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SourceSubject>,
    /// Asserted subject equivalences (fork remote → upstream id).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<SubjectAlias>,
}

impl CorrespondencePolicy {
    pub fn relative_occurrence() -> Self {
        Self {
            mode: CorrespondenceMode::RelativeOccurrence,
            subject: None,
            aliases: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), SourceSubjectError> {
        if let Some(subject) = &self.subject {
            subject.validate()?;
        }
        for alias in &self.aliases {
            alias.from.validate()?;
            alias.to.validate()?;
        }
        Ok(())
    }
}

/// Correspondence join mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CorrespondenceMode {
    /// Both snapshots declare the same [`SourceSubject`] (or request override).
    SubjectMatch,
    /// Join on subject-relative occurrence keys; requires declared or overridden subject.
    RelativeOccurrence,
    /// Apply [`SubjectAlias`] then join as relative occurrence.
    ExplicitAlias,
}

/// One asserted subject equivalence (never silent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectAlias {
    /// Materialized or fork subject as observed on a publication.
    pub from: SourceSubject,
    /// Canonical subject for the compare.
    pub to: SourceSubject,
}

/// Receipt of how correspondence was applied during a compare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrespondenceReceipt {
    /// Effective mode after resolution.
    pub mode: CorrespondenceMode,
    /// Effective subject used for the join, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SourceSubject>,
    /// Alias pairs applied (from → to display forms).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases_applied: Vec<String>,
    /// Callable occurrences present on only one side after join (diagnostic).
    pub unmatched_base: usize,
    /// Callable occurrences present only on head after join (diagnostic).
    pub unmatched_head: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_subject_requires_pkg_prefix() {
        let bad = SourceSubject::Package {
            coordinate_purl: "pydantic".into(),
        };
        assert!(matches!(
            bad.validate(),
            Err(SourceSubjectError::InvalidPackageCoordinate(_))
        ));
        let ok = SourceSubject::Package {
            coordinate_purl: "pkg:pypi/pydantic".into(),
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn package_subjects_parse_canonicalize_and_keep_versions_separate() {
        for value in ["pkg:", "pkg:pypi/", "pkg:npm/a@1.0.0", "pkg:cargo/a@main"] {
            assert!(SourceSubject::package(value).is_err(), "{value}");
        }
        let package = SourceSubject::package(" pkg:pypi/My_Package ").unwrap();
        assert_eq!(package.display(), "pkg:pypi/my-package");
        assert!(package.validate().is_ok());
        assert!(SourceSubject::Package {
            coordinate_purl: "pkg:pypi/My_Package".into()
        }
        .validate()
        .is_err());
        assert!(SourceSubject::package("pkg:npm/%40scope/name")
            .unwrap()
            .validate()
            .is_ok());
        assert!(
            SourceSubject::package("pkg:golang/github.com/go-chi/chi/v5")
                .unwrap()
                .validate()
                .is_ok()
        );
    }
}
