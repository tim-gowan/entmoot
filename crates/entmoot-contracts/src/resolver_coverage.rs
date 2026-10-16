//! Coverage partition emitted by semantic Resolver artifacts (ADR-0012).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    hash_parts, strip_digest_prefix, ContentDigest, IdParseError, IDENTITY_SCHEMA_VERSION,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverCoverageObservation {
    pub coverage_id: String,
    pub artifact_id: String,
    pub scope_universe_id: String,
    pub producer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer_version: Option<String>,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_version: Option<String>,
    pub build_context_digest: ContentDigest,
    pub call_sites_observed: u64,
    pub repository_occurrences: u64,
    pub indexed_dependency_occurrences: u64,
    pub external_symbols_without_source: u64,
    pub intrinsics_without_callable_target: u64,
    /// Missing or conflicting bindings; dispatch mechanisms are independent.
    pub unresolved_or_ambiguous: u64,
    pub unsupported: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolverCoverageValidationError {
    #[error("{0} must be non-empty")]
    EmptyField(&'static str),
    #[error("invalid build-context digest: {0}")]
    InvalidBuildContextDigest(IdParseError),
    #[error("resolver coverage counters overflow")]
    CounterOverflow,
    #[error("resolver coverage outcomes do not partition call_sites_observed")]
    CountMismatch,
    #[error("resolver coverage id does not match its identity fields")]
    IdentityMismatch,
}

impl ResolverCoverageObservation {
    pub fn derive_id(&self) -> String {
        hash_parts(&[
            "resolver_coverage",
            IDENTITY_SCHEMA_VERSION,
            &self.artifact_id,
            &self.scope_universe_id,
            &self.producer_id,
            self.producer_version.as_deref().unwrap_or(""),
            &self.language,
            self.language_version.as_deref().unwrap_or(""),
            strip_digest_prefix(self.build_context_digest.as_str()),
        ])
    }

    pub fn validate(&self) -> Result<(), ResolverCoverageValidationError> {
        for (value, field) in [
            (&self.coverage_id, "resolver_coverage.coverage_id"),
            (&self.artifact_id, "resolver_coverage.artifact_id"),
            (
                &self.scope_universe_id,
                "resolver_coverage.scope_universe_id",
            ),
            (&self.producer_id, "resolver_coverage.producer_id"),
            (&self.language, "resolver_coverage.language"),
        ] {
            if value.trim().is_empty() {
                return Err(ResolverCoverageValidationError::EmptyField(field));
            }
        }
        if self
            .producer_version
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ResolverCoverageValidationError::EmptyField(
                "resolver_coverage.producer_version",
            ));
        }
        if self
            .language_version
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ResolverCoverageValidationError::EmptyField(
                "resolver_coverage.language_version",
            ));
        }
        ContentDigest::parse(self.build_context_digest.as_str())
            .map_err(ResolverCoverageValidationError::InvalidBuildContextDigest)?;
        if self.coverage_id != self.derive_id() {
            return Err(ResolverCoverageValidationError::IdentityMismatch);
        }
        let partition = [
            self.repository_occurrences,
            self.indexed_dependency_occurrences,
            self.external_symbols_without_source,
            self.intrinsics_without_callable_target,
            self.unresolved_or_ambiguous,
            self.unsupported,
            self.failed,
        ]
        .into_iter()
        .try_fold(0u64, u64::checked_add)
        .ok_or(ResolverCoverageValidationError::CounterOverflow)?;
        if partition != self.call_sites_observed {
            return Err(ResolverCoverageValidationError::CountMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> ResolverCoverageObservation {
        let mut observation = ResolverCoverageObservation {
            coverage_id: String::new(),
            artifact_id: "artifact-1".into(),
            scope_universe_id: "scope-1".into(),
            producer_id: "go-types".into(),
            producer_version: Some("go1.27".into()),
            language: "go".into(),
            language_version: Some("go1.27".into()),
            build_context_digest: ContentDigest::from_trusted(format!("blake3:{}", "a".repeat(64))),
            call_sites_observed: 7,
            repository_occurrences: 1,
            indexed_dependency_occurrences: 1,
            external_symbols_without_source: 1,
            intrinsics_without_callable_target: 1,
            unresolved_or_ambiguous: 1,
            unsupported: 1,
            failed: 1,
        };
        observation.coverage_id = observation.derive_id();
        observation
    }

    #[test]
    fn outcomes_must_partition_observed_calls() {
        let mut value = observation();
        assert!(value.validate().is_ok());
        value.failed = 0;
        assert_eq!(
            value.validate(),
            Err(ResolverCoverageValidationError::CountMismatch)
        );
    }

    #[test]
    fn build_context_digest_is_required_and_typed() {
        let mut value = observation();
        value.build_context_digest = ContentDigest::from_trusted("sha256:not-allowed");
        assert!(matches!(
            value.validate(),
            Err(ResolverCoverageValidationError::InvalidBuildContextDigest(
                _
            ))
        ));
    }

    #[test]
    fn identity_is_length_prefixed_and_validated() {
        let mut value = observation();
        assert_eq!(value.coverage_id.len(), 64);
        value.language_version = Some("different".into());
        assert_eq!(
            value.validate(),
            Err(ResolverCoverageValidationError::IdentityMismatch)
        );
    }
}
