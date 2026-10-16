//! Foundational identity, resolution, producer, and validity contracts (additive).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::address::{Address, AddressValidationError, Subject};
use crate::assertion::EvidenceRef;
use crate::ids::{ExactBodyId, PolicyId, StructuralHash};
use crate::location::SourceRegion;

/// Logical-equivalence identity under a declared normalization policy (LAH).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogicId {
    pub policy_id: PolicyId,
    pub structural_hash: StructuralHash,
}

impl LogicId {
    pub fn new(policy_id: PolicyId, structural_hash: StructuralHash) -> Self {
        Self {
            policy_id,
            structural_hash,
        }
    }
}

/// Directional navigation/provenance bound to an expected exact body (INV-16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<SourceRegion>,
    /// Expected ExactBodyId at observation time.
    pub expected_exact_body: ExactBodyId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logic_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_revision: Option<String>,
}

/// Result of verifying a [`LocationRef`] against authoritative bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocationResolveStatus {
    Current,
    Stale,
    Unresolved,
}

/// Explicit attributable identity mapping (INV-17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub resolution_id: String,
    pub from: Address,
    pub to: Address,
    pub producer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity: Option<Validity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionValidationError {
    #[error("{0} must be non-empty")]
    EmptyField(&'static str),
    #[error("invalid resolution address: {0}")]
    InvalidAddress(AddressValidationError),
}

impl Resolution {
    pub fn validate(&self) -> Result<(), ResolutionValidationError> {
        require_resolution_field(&self.resolution_id, "resolution.resolution_id")?;
        require_resolution_field(&self.producer_id, "resolution.producer_id")?;
        if let Some(version) = &self.producer_version {
            require_resolution_field(version, "resolution.producer_version")?;
        }
        if let Some(method_id) = &self.method_id {
            require_resolution_field(method_id, "resolution.method_id")?;
        }
        self.from
            .validate()
            .map_err(ResolutionValidationError::InvalidAddress)?;
        self.to
            .validate()
            .map_err(ResolutionValidationError::InvalidAddress)?;
        Ok(())
    }
}

fn require_resolution_field(
    value: &str,
    field: &'static str,
) -> Result<(), ResolutionValidationError> {
    if value.trim().is_empty() {
        Err(ResolutionValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

/// Deterministic outcome of one Address-to-Address resolver invocation.
///
/// Positive mappings remain individual [`Resolution`] records. This envelope makes
/// open-world non-results explicit instead of overloading an empty vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionAttemptStatus {
    Resolved,
    Unresolved,
    Ambiguous,
    UnsupportedScheme,
    OutOfScope,
    StaleInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolutionAttempt {
    pub input: Address,
    pub producer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer_version: Option<String>,
    pub status: ResolutionAttemptStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<Resolution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_addresses: Vec<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionAttemptValidationError {
    #[error("invalid attempt input or candidate address: {0}")]
    InvalidAddress(AddressValidationError),
    #[error("invalid positive resolution: {0}")]
    InvalidResolution(ResolutionValidationError),
    #[error("resolution attempt producer_id must be non-empty")]
    EmptyProducerId,
    #[error("resolved attempt must contain at least one resolution")]
    ResolvedWithoutMappings,
    #[error("non-resolved attempt cannot contain positive resolutions")]
    NonResolvedHasMappings,
    #[error("resolution input does not match the attempt input")]
    InputMismatch,
    #[error("resolution producer does not match the attempt producer")]
    ProducerMismatch,
    #[error("candidate addresses must be sorted and unique")]
    NonCanonicalCandidates,
}

impl ResolutionAttempt {
    pub fn resolved(
        input: Address,
        producer_id: impl Into<String>,
        producer_version: Option<String>,
        resolutions: Vec<Resolution>,
    ) -> Result<Self, ResolutionAttemptValidationError> {
        let attempt = Self {
            input,
            producer_id: producer_id.into(),
            producer_version,
            status: ResolutionAttemptStatus::Resolved,
            resolutions,
            candidate_addresses: Vec::new(),
        };
        attempt.validate()?;
        Ok(attempt)
    }

    pub fn unresolved(
        input: Address,
        producer_id: impl Into<String>,
        producer_version: Option<String>,
        status: ResolutionAttemptStatus,
        mut candidate_addresses: Vec<Address>,
    ) -> Result<Self, ResolutionAttemptValidationError> {
        candidate_addresses.sort_by_cached_key(crate::relation::address_identity_key);
        candidate_addresses.dedup_by(|left, right| {
            crate::relation::address_identity_key(left)
                == crate::relation::address_identity_key(right)
        });
        let attempt = Self {
            input,
            producer_id: producer_id.into(),
            producer_version,
            status,
            resolutions: Vec::new(),
            candidate_addresses,
        };
        attempt.validate()?;
        Ok(attempt)
    }

    pub fn validate(&self) -> Result<(), ResolutionAttemptValidationError> {
        self.input
            .validate()
            .map_err(ResolutionAttemptValidationError::InvalidAddress)?;
        if self.producer_id.trim().is_empty() {
            return Err(ResolutionAttemptValidationError::EmptyProducerId);
        }
        for candidate in &self.candidate_addresses {
            candidate
                .validate()
                .map_err(ResolutionAttemptValidationError::InvalidAddress)?;
        }
        match self.status {
            ResolutionAttemptStatus::Resolved if self.resolutions.is_empty() => {
                return Err(ResolutionAttemptValidationError::ResolvedWithoutMappings);
            }
            ResolutionAttemptStatus::Resolved => {}
            _ if !self.resolutions.is_empty() => {
                return Err(ResolutionAttemptValidationError::NonResolvedHasMappings);
            }
            _ => {}
        }
        for resolution in &self.resolutions {
            resolution
                .validate()
                .map_err(ResolutionAttemptValidationError::InvalidResolution)?;
            if resolution.from != self.input {
                return Err(ResolutionAttemptValidationError::InputMismatch);
            }
            if resolution.producer_id != self.producer_id
                || resolution.producer_version != self.producer_version
            {
                return Err(ResolutionAttemptValidationError::ProducerMismatch);
            }
        }
        let mut canonical = self.candidate_addresses.clone();
        canonical.sort_by_cached_key(crate::relation::address_identity_key);
        canonical.dedup_by(|left, right| {
            crate::relation::address_identity_key(left)
                == crate::relation::address_identity_key(right)
        });
        if canonical != self.candidate_addresses {
            return Err(ResolutionAttemptValidationError::NonCanonicalCandidates);
        }
        Ok(())
    }
}

#[cfg(test)]
mod resolution_attempt_contract_tests {
    use super::*;

    #[test]
    fn resolved_attempt_requires_at_least_one_matching_resolution() {
        let input = Address::LogicAddress {
            logic_address: "logic|-|python|a.py|-|f|function|-|-".into(),
        };
        let attempt = ResolutionAttempt::resolved(
            input.clone(),
            "resolver",
            Some("1".into()),
            vec![Resolution {
                resolution_id: "r1".into(),
                from: input,
                to: Address::Opaque {
                    system: "rust.def_path".into(),
                    value: "example::sym".into(),
                },
                producer_id: "resolver".into(),
                producer_version: Some("1".into()),
                evidence_refs: vec![],
                validity: None,
                method_id: Some("resolver.v1".into()),
            }],
        )
        .expect("valid attempt");
        assert_eq!(attempt.status, ResolutionAttemptStatus::Resolved);
        assert!(attempt.validate().is_ok());
    }

    #[test]
    fn unresolved_attempt_is_explicit_and_strict_on_the_wire() {
        let input = Address::RepoPath {
            repository_id: Some("repo".into()),
            path: "a.py".into(),
            start_byte: Some(0),
            end_byte: None,
        };
        let attempt = ResolutionAttempt::unresolved(
            input,
            "resolver",
            Some("1".into()),
            ResolutionAttemptStatus::Unresolved,
            vec![],
        )
        .expect("valid attempt");
        assert!(attempt.resolutions.is_empty());
        assert!(attempt.validate().is_ok());

        let mut value = serde_json::to_value(&attempt).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<ResolutionAttempt>(value).is_err());
    }

    #[test]
    fn attempt_rejects_malformed_addresses_and_blank_producers() {
        let malformed = Address::Opaque {
            system: String::new(),
            value: "value".into(),
        };
        let invalid_address = ResolutionAttempt::unresolved(
            malformed,
            "resolver",
            None,
            ResolutionAttemptStatus::Unresolved,
            vec![],
        );
        assert!(matches!(
            invalid_address,
            Err(ResolutionAttemptValidationError::InvalidAddress(_))
        ));

        let blank_producer = ResolutionAttempt::unresolved(
            Address::RepoPath {
                repository_id: None,
                path: "src/lib.rs".into(),
                start_byte: None,
                end_byte: None,
            },
            " ",
            None,
            ResolutionAttemptStatus::Unresolved,
            vec![],
        );
        assert_eq!(
            blank_producer,
            Err(ResolutionAttemptValidationError::EmptyProducerId)
        );
    }
}

/// Semantic grain of a claim (INV-01 / INV-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimScope {
    LahLocal,
    Occurrence,
    Relational,
    Stitch,
    Contextual,
    Derivation,
}

/// Whether recorded conditions still hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_reason: Option<String>,
}

/// Predicate vocabulary maturity (OpenTelemetry-like governance, not telemetry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PredicateMaturity {
    Development,
    Experimental,
    Stable,
    Deprecated,
}

/// Namespaced predicate identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PredicateRef {
    /// e.g. `entmoot` or `producer.sarif.snyk`
    pub namespace: String,
    pub name: String,
    pub maturity: PredicateMaturity,
}

/// Who produced an observation/resolution/claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerIdentity {
    pub producer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// Fingerprint of analysis environment (conservative, not a vendor metadata standard).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisEnvironmentFingerprint {
    pub fingerprint_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factors: Vec<String>,
}

/// How a producer may be invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvocationMode {
    Attach,
    Eager,
    Jit,
}

/// Declared producer capabilities for registry selection and scope enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerCapabilities {
    pub identity: ProducerIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepts_schemes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolves_to_schemes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicates: Vec<PredicateRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claim_scopes: Vec<ClaimScope>,
    #[serde(default)]
    pub emits_relations: bool,
    #[serde(default)]
    pub supports_completeness: bool,
    pub invocation: InvocationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<AnalysisEnvironmentFingerprint>,
}

impl ProducerCapabilities {
    /// Whether this producer may emit the given claim scope.
    pub fn allows_scope(&self, scope: ClaimScope) -> bool {
        self.claim_scopes.contains(&scope)
    }
}

/// Explicit scoped completeness (open-world negative inference gate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompletenessAssertion {
    pub completeness_id: String,
    pub producer: ProducerIdentity,
    pub subject_set_description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicates: Vec<PredicateRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

/// Validate that `lah_local` emission is allowed for this producer and subject.
pub fn validate_lah_local_emission(
    caps: &ProducerCapabilities,
    subject: &Subject,
) -> Result<(), ScopePromotionError> {
    if !caps.allows_scope(ClaimScope::LahLocal) {
        return Err(ScopePromotionError::CapabilityDenied);
    }
    match &subject.address {
        Address::LogicId { .. } => Ok(()),
        _ => Err(ScopePromotionError::SubjectNotLogicId),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopePromotionError {
    CapabilityDenied,
    SubjectNotLogicId,
}

impl std::fmt::Display for ScopePromotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopePromotionError::CapabilityDenied => {
                write!(f, "producer capabilities do not allow lah_local scope")
            }
            ScopePromotionError::SubjectNotLogicId => {
                write!(f, "lah_local claims require a LogicId subject")
            }
        }
    }
}

impl std::error::Error for ScopePromotionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_caps(scopes: Vec<ClaimScope>) -> ProducerCapabilities {
        ProducerCapabilities {
            identity: ProducerIdentity {
                producer_id: "test".into(),
                version: Some("1".into()),
                tool_name: None,
            },
            accepts_schemes: vec![],
            resolves_to_schemes: vec![],
            predicates: vec![],
            claim_scopes: scopes,
            emits_relations: false,
            supports_completeness: false,
            invocation: InvocationMode::Attach,
            environment: None,
        }
    }

    #[test]
    fn rejects_lah_local_without_capability() {
        let caps = sample_caps(vec![ClaimScope::Occurrence]);
        let subject = Subject {
            address: Address::LogicId {
                policy_id: PolicyId::from_trusted(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                structural_hash: StructuralHash::from_trusted(
                    "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
            },
            display: None,
        };
        assert_eq!(
            validate_lah_local_emission(&caps, &subject),
            Err(ScopePromotionError::CapabilityDenied)
        );
    }

    #[test]
    fn rejects_lah_local_on_path_subject() {
        let caps = sample_caps(vec![ClaimScope::LahLocal]);
        let subject = Subject {
            address: Address::RepoPath {
                repository_id: None,
                path: "a.py".into(),
                start_byte: None,
                end_byte: None,
            },
            display: None,
        };
        assert_eq!(
            validate_lah_local_emission(&caps, &subject),
            Err(ScopePromotionError::SubjectNotLogicId)
        );
    }

    #[test]
    fn accepts_lah_local_with_logic_id() {
        let caps = sample_caps(vec![ClaimScope::LahLocal]);
        let subject = Subject {
            address: Address::LogicId {
                policy_id: PolicyId::from_trusted(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                structural_hash: StructuralHash::from_trusted(
                    "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
            },
            display: None,
        };
        assert!(validate_lah_local_emission(&caps, &subject).is_ok());
    }
}
