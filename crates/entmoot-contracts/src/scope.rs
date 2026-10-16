//! Scope universe and coverage observations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::identity::{observed_document_set_digest, scope_universe_id};
use crate::ids::{ContentDigest, IdParseError};

pub const SELECTOR_LANGUAGE_VERSION: &str = "logic.path_glob.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeCompleteness {
    /// Complete relative to the declared scope universe only.
    CompleteRelativeToUniverse,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopeUniverse {
    pub scope_universe_id: String,
    pub selector_language_version: String,
    pub profile: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    pub included_paths: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub completeness: ScopeCompleteness,
    /// Digest of paths actually present in the artifact (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_document_set_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub producer_skips: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScopeValidationError {
    #[error("scope_universe_id does not match derived id")]
    IdMismatch,
    #[error("complete_relative_to_universe requires observed_document_set_digest")]
    CompleteMissingDenominator,
    #[error("invalid scope path: {0}")]
    InvalidPath(String),
    #[error("unsupported selector_language_version: {0}")]
    UnsupportedSelectorLanguageVersion(String),
    #[error("scope profile must be non-empty")]
    EmptyProfile,
    #[error("{0} must be sorted and unique")]
    NonCanonicalSet(&'static str),
    #[error("scope language values must be non-empty")]
    EmptyLanguage,
    #[error("producer_skips values must be non-empty")]
    EmptyProducerSkip,
    #[error("invalid observed_document_set_digest: {0}")]
    InvalidObservedDocumentSetDigest(IdParseError),
}

impl ScopeUniverse {
    pub fn derive_id(
        profile: &str,
        languages: &[String],
        includes: &[String],
        excludes: &[String],
    ) -> String {
        scope_universe_id(
            SELECTOR_LANGUAGE_VERSION,
            profile,
            languages,
            includes,
            excludes,
        )
    }

    pub fn from_selectors(
        profile: impl Into<String>,
        languages: Vec<String>,
        included_paths: Vec<String>,
        excluded_paths: Vec<String>,
        completeness: ScopeCompleteness,
        observed_paths: Option<&[String]>,
    ) -> Self {
        let profile = profile.into();
        let languages = canonical_strings(languages);
        let included_paths = canonical_strings(included_paths);
        let excluded_paths = canonical_strings(excluded_paths);
        let scope_universe_id =
            Self::derive_id(&profile, &languages, &included_paths, &excluded_paths);
        let observed_document_set_digest = observed_paths.map(observed_document_set_digest);
        Self {
            scope_universe_id,
            selector_language_version: SELECTOR_LANGUAGE_VERSION.into(),
            profile,
            languages,
            included_paths,
            excluded_paths,
            completeness,
            observed_document_set_digest,
            producer_skips: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ScopeValidationError> {
        if self.selector_language_version != SELECTOR_LANGUAGE_VERSION {
            return Err(ScopeValidationError::UnsupportedSelectorLanguageVersion(
                self.selector_language_version.clone(),
            ));
        }
        if self.profile.trim().is_empty() {
            return Err(ScopeValidationError::EmptyProfile);
        }
        validate_canonical_set(&self.languages, "languages")?;
        validate_canonical_set(&self.included_paths, "included_paths")?;
        validate_canonical_set(&self.excluded_paths, "excluded_paths")?;
        validate_canonical_set(&self.producer_skips, "producer_skips")?;
        if self.languages.iter().any(|value| value.trim().is_empty()) {
            return Err(ScopeValidationError::EmptyLanguage);
        }
        if self
            .producer_skips
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(ScopeValidationError::EmptyProducerSkip);
        }
        let expected = Self::derive_id(
            &self.profile,
            &self.languages,
            &self.included_paths,
            &self.excluded_paths,
        );
        if self.scope_universe_id != expected {
            return Err(ScopeValidationError::IdMismatch);
        }
        if self.completeness == ScopeCompleteness::CompleteRelativeToUniverse
            && self.observed_document_set_digest.is_none()
        {
            return Err(ScopeValidationError::CompleteMissingDenominator);
        }
        if let Some(digest) = &self.observed_document_set_digest {
            ContentDigest::parse(digest.clone())
                .map_err(ScopeValidationError::InvalidObservedDocumentSetDigest)?;
        }
        for path in self.included_paths.iter().chain(self.excluded_paths.iter()) {
            crate::paths::normalize_repo_path(path)
                .map_err(|e| ScopeValidationError::InvalidPath(e.to_string()))?;
        }
        Ok(())
    }

    pub fn path_in_scope(&self, path: &str) -> Result<bool, ScopeValidationError> {
        let norm = crate::paths::normalize_repo_path(path)
            .map_err(|e| ScopeValidationError::InvalidPath(e.to_string()))?;
        for ex in &self.excluded_paths {
            let exn = crate::paths::normalize_repo_path(ex)
                .map_err(|e| ScopeValidationError::InvalidPath(e.to_string()))?;
            if path_matches(&norm, &exn) {
                return Ok(false);
            }
        }
        if self.included_paths.is_empty() {
            return Ok(true);
        }
        for inc in &self.included_paths {
            let inn = crate::paths::normalize_repo_path(inc)
                .map_err(|e| ScopeValidationError::InvalidPath(e.to_string()))?;
            if path_matches(&norm, &inn) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn path_matches(path: &str, pattern: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return path == pattern || path.starts_with(&format!("{pattern}/"));
    }
    let path_segments: Vec<_> = path.split('/').collect();
    let pattern_segments: Vec<_> = pattern.split('/').collect();
    let mut reachable = vec![vec![false; path_segments.len() + 1]; pattern_segments.len() + 1];
    reachable[0][0] = true;
    for pattern_index in 0..pattern_segments.len() {
        for path_index in 0..=path_segments.len() {
            if !reachable[pattern_index][path_index] {
                continue;
            }
            if pattern_segments[pattern_index] == "**" {
                reachable[pattern_index + 1][path_index] = true;
                if path_index < path_segments.len() {
                    reachable[pattern_index][path_index + 1] = true;
                }
            } else if path_index < path_segments.len()
                && segment_matches(path_segments[path_index], pattern_segments[pattern_index])
            {
                reachable[pattern_index + 1][path_index + 1] = true;
            }
        }
    }
    reachable[pattern_segments.len()][path_segments.len()]
}

fn segment_matches(value: &str, pattern: &str) -> bool {
    let value: Vec<_> = value.chars().collect();
    let pattern: Vec<_> = pattern.chars().collect();
    let mut reachable = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    reachable[0][0] = true;
    for pattern_index in 0..pattern.len() {
        for value_index in 0..=value.len() {
            if !reachable[pattern_index][value_index] {
                continue;
            }
            match pattern[pattern_index] {
                '*' => {
                    reachable[pattern_index + 1][value_index] = true;
                    if value_index < value.len() {
                        reachable[pattern_index][value_index + 1] = true;
                    }
                }
                '?' if value_index < value.len() => {
                    reachable[pattern_index + 1][value_index + 1] = true;
                }
                literal if value_index < value.len() && literal == value[value_index] => {
                    reachable[pattern_index + 1][value_index + 1] = true;
                }
                _ => {}
            }
        }
    }
    reachable[pattern.len()][value.len()]
}

fn canonical_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn validate_canonical_set(
    values: &[String],
    field: &'static str,
) -> Result<(), ScopeValidationError> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ScopeValidationError::NonCanonicalSet(field));
    }
    Ok(())
}

/// Durable, scoped coverage. Structural inventory is never negative scanner evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverageObservation {
    pub coverage_id: String,
    pub snapshot: crate::SourceSnapshot,
    pub scope: ScopeUniverse,
    pub measurement: CoverageMeasurement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoverageMeasurement {
    Findings {
        artifact_id: String,
        finding_count: u32,
    },
    StructuralScan {
        method_id: String,
        content_manifest_digest: ContentDigest,
        observed_files: u64,
        parsed: u64,
        /// Historical producer evidence; the current structural scan never applies heuristics.
        heuristic_skipped: u64,
        unsupported: u64,
        failed: u64,
        /// Actual pruned directories, including hidden directories; contents unknown.
        excluded_directories: Vec<String>,
        /// Repository-relative files omitted from structural parsing. Details live
        /// in retained diagnostics; exclusion never establishes an empty call set.
        excluded_files: Vec<String>,
    },
}

impl CoverageObservation {
    pub fn derive_id(&self) -> Result<String, serde_json::Error> {
        Ok(crate::hash_parts(&[
            "entmoot.coverage.v2",
            &serde_json::to_string(&(
                &self.snapshot,
                &self.scope,
                &self.measurement,
                &self.message,
            ))?,
        ]))
    }

    pub fn validate(&self) -> Result<(), String> {
        self.snapshot.validate().map_err(|e| e.to_string())?;
        self.scope.validate().map_err(|e| e.to_string())?;
        if self.coverage_id != self.derive_id().map_err(|e| e.to_string())? {
            return Err("coverage identity mismatch".into());
        }
        if let CoverageMeasurement::StructuralScan {
            method_id,
            content_manifest_digest,
            observed_files,
            parsed,
            heuristic_skipped,
            unsupported,
            failed,
            excluded_directories,
            excluded_files,
        } = &self.measurement
        {
            if method_id.is_empty() {
                return Err("coverage requires a scan policy".into());
            }
            ContentDigest::parse(content_manifest_digest.as_str()).map_err(|e| e.to_string())?;
            if [*parsed, *heuristic_skipped, *unsupported, *failed]
                .into_iter()
                .try_fold(0u64, u64::checked_add)
                != Some(*observed_files)
            {
                return Err("scan coverage count mismatch".into());
            }
            if excluded_files.len() as u64 != observed_files - parsed {
                return Err("scan excluded file count mismatch".into());
            }
            for paths in [excluded_directories, excluded_files] {
                if paths.windows(2).any(|p| p[0] >= p[1]) {
                    return Err("scan paths must be sorted and unique".into());
                }
                for path in paths {
                    if crate::normalize_repo_path(path).map_err(|e| e.to_string())? != *path {
                        return Err("scan path must be canonical".into());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_requires_observed_digest() {
        let mut u = ScopeUniverse::from_selectors(
            "review",
            vec![],
            vec!["src/**".into()],
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            None,
        );
        assert!(matches!(
            u.validate(),
            Err(ScopeValidationError::CompleteMissingDenominator)
        ));
        u.observed_document_set_digest = Some(format!("blake3:{}", "a".repeat(64)));
        assert!(u.validate().is_ok());
    }

    #[test]
    fn language_and_profile_are_scope_identity_dimensions() {
        let rust = ScopeUniverse::from_selectors(
            "review",
            vec!["rust".into()],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let typescript = ScopeUniverse::from_selectors(
            "review",
            vec!["typescript".into()],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let changed_profile = ScopeUniverse::from_selectors(
            "changed-files",
            vec!["rust".into()],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        assert_ne!(rust.scope_universe_id, typescript.scope_universe_id);
        assert_ne!(rust.scope_universe_id, changed_profile.scope_universe_id);
    }

    #[test]
    fn recursive_and_segment_wildcards_match_real_corpus_selectors() {
        let scope = ScopeUniverse::from_selectors(
            "real-world",
            vec!["typescript".into()],
            vec!["src/**".into(), "compose*.yaml".into()],
            vec!["**/node_modules/**".into(), "**/build/?ache/**".into()],
            ScopeCompleteness::Partial,
            None,
        );
        assert!(scope.path_in_scope("src/frontend/main.ts").unwrap());
        assert!(scope.path_in_scope("compose.test.yaml").unwrap());
        assert!(!scope
            .path_in_scope("src/frontend/node_modules/pkg/index.ts")
            .unwrap());
        assert!(!scope.path_in_scope("src/build/cache/item.ts").unwrap());
        assert!(!scope.path_in_scope("README.md").unwrap());
    }

    #[test]
    fn scope_wire_requires_canonical_selectors_and_valid_digests() {
        let mut scope = ScopeUniverse::from_selectors(
            "review",
            vec!["typescript".into(), "rust".into()],
            vec!["src/**".into()],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        assert_eq!(scope.languages, ["rust", "typescript"]);
        scope.languages.reverse();
        assert!(matches!(
            scope.validate(),
            Err(ScopeValidationError::NonCanonicalSet("languages"))
        ));

        let mut scope = ScopeUniverse::from_selectors(
            "review",
            vec![],
            vec![],
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&["src/a.rs".into()]),
        );
        scope.observed_document_set_digest = Some("blake3:short".into());
        assert!(matches!(
            scope.validate(),
            Err(ScopeValidationError::InvalidObservedDocumentSetDigest(_))
        ));
    }
}
