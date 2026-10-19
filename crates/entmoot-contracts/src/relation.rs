//! Addressable relation edges and relation-claim resolution (ADR-0009).
//!
//! Provisional wire shapes. RelationEdge identity is producer-independent; claim identity
//! includes producer/config. `ResolutionState` is evidence kind — not a trust ladder.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::address::{Address, AddressValidationError};
use crate::assertion::{DerivationRecord, EvidenceRef};
use crate::identity::{hash_parts, DIGEST_PREFIX};
use crate::ids::{ContentDigest, IdParseError};

#[cfg(test)]
#[path = "relation/identity_tests.rs"]
mod identity_tests;

/// Snapshot-independent digest of one positive relational proposition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PropositionId(String);

impl PropositionId {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdParseError> {
        let value = value.into();
        ContentDigest::parse(value.clone())?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_trusted(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for PropositionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Prefixed relation-edge digest (`blake3:<64-hex>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RelationEdgeId(String);

impl RelationEdgeId {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdParseError> {
        let value = value.into();
        ContentDigest::parse(value.clone())?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_trusted(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for RelationEdgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Prefixed relation-claim digest (`blake3:<64-hex>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RelationClaimId(String);

impl RelationClaimId {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdParseError> {
        let value = value.into();
        ContentDigest::parse(value.clone())?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_trusted(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for RelationClaimId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Prefixed unresolved-relation digest (`blake3:<64-hex>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct UnresolvedRelationId(String);

impl UnresolvedRelationId {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdParseError> {
        let value = value.into();
        ContentDigest::parse(value.clone())?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_trusted(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for UnresolvedRelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Relation kinds for durable relation edges (extensible; start small).
///
/// Topology and inventory correspondence only (`RoutesTo`, `ConnectsTo`,
/// `ObservedAs`, …). Do **not** add allow/deny/authorization predicates here;
/// authz judgment stays on consumer planes or external evidence with
/// capability limitations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    HasCanonicalForm,
    Calls,
    Imports,
    Implements,
    DependsOn,
    Selects,
    RoutesTo,
    UsesConfiguration,
    ConnectsTo,
    /// A desired/declarative resource occurrence corresponds to an observed one.
    ObservedAs,
}

/// Nature of a resolution relation claim — **not** a confidence / trust ladder (ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    Structural,
    Candidate,
    ArtifactResolved,
    SemanticResolved,
    Validated,
    Refuted,
    Stale,
    Unknown,
}

/// Why an observed relation intent did not yield an addressable positive edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    /// Name binding did not evaluate the possible dispatch target set.
    DispatchNotEvaluated,
    /// Producer stopped at an explicit analysis budget.
    BudgetExceeded,
    /// Toolchain/build environment could not support a valid analysis.
    InvalidEnvironment,
    /// Producer recognizes the site but cannot model the construct.
    UnsupportedConstruct,
    IndirectCall,
    ProviderError,
    UnresolvedBinding,
    AmbiguousTarget,
    TargetNotInInventory,
    ProducerNotInvoked,
    UnsupportedProducer,
    OutOfScope,
}

/// Constraint retained when a relation target cannot be selected soundly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetConstraint {
    Unknown,
    /// Observed source spelling without claiming that it names a unique Address.
    Syntactic {
        callee: String,
    },
    Candidates {
        addresses: Vec<Address>,
    },
}

impl TargetConstraint {
    fn canonicalized(mut self) -> Self {
        if let Self::Candidates { addresses } = &mut self {
            addresses.sort_by_cached_key(address_identity_key);
            addresses.dedup();
        }
        self
    }

    fn canonical_key(&self) -> String {
        match self {
            Self::Unknown => "unknown".into(),
            Self::Syntactic { callee } => format!("syntactic:{callee}"),
            Self::Candidates { addresses } => {
                let keys: Vec<_> = addresses.iter().map(address_identity_key).collect();
                let refs: Vec<_> = keys.iter().map(String::as_str).collect();
                format!("candidates:{}", hash_parts(&refs))
            }
        }
    }
}

/// Explicit semantic scope for RelationEdgeId / RelationClaimId (never an opaque free string alone).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Semantic domain boundary (e.g. repository id, deployment id).
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
}

impl Scope {
    /// Tagged optional fields preserve both field identity and presence before
    /// length framing. An absent context cannot stand in for a present revision.
    pub fn canonical_parts(&self) -> Vec<&str> {
        let mut parts = vec!["domain", self.domain.as_str()];
        for (field, value) in [
            ("snapshot_revision", &self.snapshot_revision),
            ("snapshot_digest", &self.snapshot_digest),
            ("occurrence_id", &self.occurrence_id),
            ("context_id", &self.context_id),
        ] {
            parts.extend([
                field,
                if value.is_some() { "some" } else { "none" },
                value.as_deref().unwrap_or(""),
            ]);
        }
        parts
    }

    pub fn validate(&self) -> Result<(), RelationValidationError> {
        require_nonempty(&self.domain, "scope.domain")?;
        if let Some(revision) = &self.snapshot_revision {
            require_nonempty(revision, "scope.snapshot_revision")?;
        }
        if let Some(digest) = &self.snapshot_digest {
            ContentDigest::parse(digest.clone()).map_err(RelationValidationError::InvalidId)?;
        }
        if let Some(occurrence_id) = &self.occurrence_id {
            require_nonempty(occurrence_id, "scope.occurrence_id")?;
        }
        if let Some(context_id) = &self.context_id {
            require_nonempty(context_id, "scope.context_id")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelationValidationError {
    #[error("invalid assessment observation: {0}")]
    InvalidAssessment(String),
    #[error("invalid relation identity: {0}")]
    InvalidId(IdParseError),
    #[error("invalid relation address: {0}")]
    InvalidAddress(AddressValidationError),
    #[error("{0} must be non-empty")]
    EmptyField(&'static str),
    #[error("producer capabilities must be unique")]
    DuplicateProducerCapability,
    #[error("relation edge id does not match its canonical fields")]
    RelationEdgeIdentityMismatch,
    #[error("proposition id does not match the relation edge's canonical proposition fields")]
    PropositionIdentityMismatch,
    #[error("relation claim id does not match its canonical fields")]
    RelationClaimIdentityMismatch,
    #[error("relation claim supports/contradicts lists must be unique and disjoint")]
    InvalidRelationClaimLinks,
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), RelationValidationError> {
    if value.trim().is_empty() {
        Err(RelationValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

/// What a relation claim speaks about.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubjectId {
    Address { address: Address },
    Proposition { proposition_id: PropositionId },
    RelationEdge { edge_id: RelationEdgeId },
    RelationClaim { claim_id: RelationClaimId },
    Artifact { artifact_id: String },
}

/// Producer capability roles (ADR-0009 / producer protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProducerCapabilityKind {
    Resolver,
    Asserter,
    ArtifactAdapter,
    RuntimeObserver,
    Deriver,
}

/// Provenance and capability metadata — never semantic identity of the subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerRef {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<ProducerCapabilityKind>,
}

impl ProducerRef {
    pub fn validate(&self) -> Result<(), RelationValidationError> {
        require_nonempty(&self.id, "producer.id")?;
        require_nonempty(&self.name, "producer.name")?;
        if let Some(version) = &self.version {
            require_nonempty(version, "producer.version")?;
        }
        let unique: std::collections::HashSet<_> = self.capabilities.iter().copied().collect();
        if unique.len() != self.capabilities.len() {
            return Err(RelationValidationError::DuplicateProducerCapability);
        }
        Ok(())
    }
}

/// Audit / reproducibility envelope (timestamps never enter RelationEdgeId / RelationClaimId).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub produced_at: Option<String>,
}

/// Lifecycle of **this** relation claim, not global relation truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationClaimState {
    Active,
    Superseded,
    Retracted,
}

/// RelationClaim value for resolution-oriented predicates (extensible via `Literal`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelationClaimValue {
    Canon {
        observation: Box<crate::CanonObservation>,
        digest: ContentDigest,
    },
    Assessment {
        observation: crate::AssessmentObservation,
    },
    ResolutionState {
        state: ResolutionState,
    },
    Literal {
        value: String,
    },
}

impl RelationClaimValue {
    pub fn canonical(&self) -> String {
        match self {
            RelationClaimValue::Canon { digest, .. } => format!("canon:{digest}"),
            RelationClaimValue::Assessment { observation } => {
                format!("assessment:{}", observation.canonical())
            }
            RelationClaimValue::ResolutionState { state } => {
                let s = match state {
                    ResolutionState::Structural => "structural",
                    ResolutionState::Candidate => "candidate",
                    ResolutionState::ArtifactResolved => "artifact_resolved",
                    ResolutionState::SemanticResolved => "semantic_resolved",
                    ResolutionState::Validated => "validated",
                    ResolutionState::Refuted => "refuted",
                    ResolutionState::Stale => "stale",
                    ResolutionState::Unknown => "unknown",
                };
                format!("resolution_state:{s}")
            }
            RelationClaimValue::Literal { value } => format!("literal:{value}"),
        }
    }
}

/// Stable, producer-independent positive relational proposition.
///
/// This is the fact-like key shared by every snapshot/context/site occurrence.
/// Epistemic state and provenance live in `RelationClaim` and `RelationEdge`
/// records, never on this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Proposition {
    pub id: PropositionId,
    pub schema_domain: String,
    /// Stable semantic boundary, normally repository or estate identity.
    pub domain: String,
    pub subject: Address,
    pub predicate: RelationKind,
    pub object: Address,
}

impl Proposition {
    pub fn from_relation_edge(edge: &RelationEdge) -> Self {
        Self {
            id: edge.proposition_id.clone(),
            schema_domain: edge.schema_domain.clone(),
            domain: edge.scope.domain.clone(),
            subject: edge.subject.clone(),
            predicate: edge.predicate,
            object: edge.object.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), RelationValidationError> {
        PropositionId::parse(self.id.as_str()).map_err(RelationValidationError::InvalidId)?;
        require_nonempty(&self.schema_domain, "proposition.schema_domain")?;
        require_nonempty(&self.domain, "proposition.domain")?;
        self.subject
            .validate()
            .map_err(RelationValidationError::InvalidAddress)?;
        self.object
            .validate()
            .map_err(RelationValidationError::InvalidAddress)?;
        let expected = compute_proposition_id(
            &self.schema_domain,
            &self.domain,
            &self.subject,
            self.predicate,
            &self.object,
        );
        if self.id != expected {
            return Err(RelationValidationError::PropositionIdentityMismatch);
        }
        Ok(())
    }
}

/// Durable relation occurrence (no validation status field — INV-EDGE-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationEdge {
    pub id: RelationEdgeId,
    /// Stable positive proposition shared across snapshot/site occurrences.
    pub proposition_id: PropositionId,
    /// Versioned identity namespace used in `RelationEdgeId` derivation.
    #[serde(default = "default_relation_schema_domain")]
    pub schema_domain: String,
    pub subject: Address,
    pub predicate: RelationKind,
    pub object: Address,
    pub scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
}

fn default_relation_schema_domain() -> String {
    "source".into()
}

impl RelationEdge {
    pub fn proposition(&self) -> Proposition {
        Proposition::from_relation_edge(self)
    }

    /// A build-scoped claim may refer to a shared source callsite projection.
    /// This permits retention, not application outside the claim's context.
    /// Snapshot, repository and occurrence coordinates must still agree exactly.
    pub fn accepts_claim_scope(&self, scope: &Scope) -> bool {
        self.scope == *scope
            || (self.schema_domain == "source"
                && self.predicate == RelationKind::Calls
                && self.occurrence_id.is_some()
                && self.scope.context_id.is_none()
                && scope.context_id.is_some()
                && self.scope.domain == scope.domain
                && self.scope.snapshot_revision == scope.snapshot_revision
                && self.scope.snapshot_digest == scope.snapshot_digest
                && self.scope.occurrence_id == scope.occurrence_id)
    }

    pub fn validate(&self) -> Result<(), RelationValidationError> {
        RelationEdgeId::parse(self.id.as_str()).map_err(RelationValidationError::InvalidId)?;
        PropositionId::parse(self.proposition_id.as_str())
            .map_err(RelationValidationError::InvalidId)?;
        require_nonempty(&self.schema_domain, "relation_edge.schema_domain")?;
        self.subject
            .validate()
            .map_err(RelationValidationError::InvalidAddress)?;
        self.object
            .validate()
            .map_err(RelationValidationError::InvalidAddress)?;
        self.scope.validate()?;
        if let Some(occurrence_id) = &self.occurrence_id {
            require_nonempty(occurrence_id, "relation_edge.occurrence_id")?;
        }
        let expected_proposition = compute_proposition_id(
            &self.schema_domain,
            &self.scope.domain,
            &self.subject,
            self.predicate,
            &self.object,
        );
        if self.proposition_id != expected_proposition {
            return Err(RelationValidationError::PropositionIdentityMismatch);
        }
        let expected = compute_relation_edge_id(
            &self.schema_domain,
            &self.subject,
            self.predicate,
            &self.object,
            &self.scope,
            self.occurrence_id.as_deref(),
        );
        if self.id != expected {
            return Err(RelationValidationError::RelationEdgeIdentityMismatch);
        }
        Ok(())
    }
}

/// Independent relation claim about a subject (often a `RelationEdge`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationClaim {
    pub id: RelationClaimId,
    pub subject: SubjectId,
    pub predicate: String,
    pub value: RelationClaimValue,
    pub producer: ProducerRef,
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    pub state: RelationClaimState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supports: Vec<RelationClaimId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicts: Vec<RelationClaimId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation: Option<DerivationRecord>,
}

impl RelationClaim {
    pub fn validate(&self) -> Result<(), RelationValidationError> {
        if let RelationClaimValue::Canon {
            observation,
            digest,
        } = &self.value
        {
            if observation.validate().is_err()
                || !observation.digest().is_ok_and(|actual| &actual == digest)
            {
                return Err(RelationValidationError::RelationClaimIdentityMismatch);
            }
        }
        RelationClaimId::parse(self.id.as_str()).map_err(RelationValidationError::InvalidId)?;
        if let RelationClaimValue::Assessment { observation } = &self.value {
            observation
                .validate()
                .map_err(|e| RelationValidationError::InvalidAssessment(e.to_string()))?;
        }
        require_nonempty(&self.predicate, "relation_claim.predicate")?;
        match &self.subject {
            SubjectId::Address { address } => address
                .validate()
                .map_err(RelationValidationError::InvalidAddress)?,
            SubjectId::Proposition { proposition_id } => {
                PropositionId::parse(proposition_id.as_str())
                    .map_err(RelationValidationError::InvalidId)?;
            }
            SubjectId::RelationEdge { edge_id } => {
                RelationEdgeId::parse(edge_id.as_str())
                    .map_err(RelationValidationError::InvalidId)?;
            }
            SubjectId::RelationClaim { claim_id } => {
                RelationClaimId::parse(claim_id.as_str())
                    .map_err(RelationValidationError::InvalidId)?;
            }
            SubjectId::Artifact { artifact_id } => {
                crate::ArtifactId::parse(artifact_id)
                    .map_err(RelationValidationError::InvalidId)?;
            }
        }
        self.producer.validate()?;
        self.scope.validate()?;
        let expected = compute_relation_claim_id(
            &self.subject,
            &self.predicate,
            &self.value,
            &self.producer,
            &self.scope,
        );
        if self.id != expected {
            return Err(RelationValidationError::RelationClaimIdentityMismatch);
        }
        let supports: std::collections::HashSet<_> = self.supports.iter().collect();
        let contradicts: std::collections::HashSet<_> = self.contradicts.iter().collect();
        if supports.len() != self.supports.len()
            || contradicts.len() != self.contradicts.len()
            || !supports.is_disjoint(&contradicts)
            || supports.contains(&self.id)
            || contradicts.contains(&self.id)
        {
            return Err(RelationValidationError::InvalidRelationClaimLinks);
        }
        Ok(())
    }
}

/// An observed relation question whose target could not be selected soundly.
///
/// This is not a placeholder edge and does not enumerate the complement of the
/// graph. It records one attributable resolution attempt (ADR-0009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedRelation {
    pub id: UnresolvedRelationId,
    pub source: Address,
    pub predicate: RelationKind,
    pub target_constraint: TargetConstraint,
    pub reason: UnresolvedReason,
    pub resolution_state: ResolutionState,
    pub producer: ProducerRef,
    pub scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_capability: Option<ProducerCapabilityKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnresolvedRelationValidationError {
    #[error("invalid unresolved relation identity: {0}")]
    InvalidId(IdParseError),
    #[error("invalid unresolved relation fields: {0}")]
    InvalidRelation(RelationValidationError),
    #[error("unresolved relation resolution_state must be unknown")]
    NonUnknownState,
    #[error("candidate target constraint must contain at least one address")]
    EmptyCandidates,
    #[error("candidate target addresses must be sorted and unique")]
    NonCanonicalCandidates,
    #[error("unresolved relation id does not match its canonical fields")]
    IdentityMismatch,
}

impl UnresolvedRelation {
    pub fn validate(&self) -> Result<(), UnresolvedRelationValidationError> {
        UnresolvedRelationId::parse(self.id.as_str())
            .map_err(UnresolvedRelationValidationError::InvalidId)?;
        self.producer
            .validate()
            .map_err(UnresolvedRelationValidationError::InvalidRelation)?;
        self.scope
            .validate()
            .map_err(UnresolvedRelationValidationError::InvalidRelation)?;
        self.source
            .validate()
            .map_err(RelationValidationError::InvalidAddress)
            .map_err(UnresolvedRelationValidationError::InvalidRelation)?;
        if let Some(occurrence_id) = &self.occurrence_id {
            require_nonempty(occurrence_id, "unresolved_relation.occurrence_id")
                .map_err(UnresolvedRelationValidationError::InvalidRelation)?;
        }
        if self.resolution_state != ResolutionState::Unknown {
            return Err(UnresolvedRelationValidationError::NonUnknownState);
        }
        if let TargetConstraint::Candidates { addresses } = &self.target_constraint {
            if addresses.is_empty() {
                return Err(UnresolvedRelationValidationError::EmptyCandidates);
            }
            if self.target_constraint.clone().canonicalized() != self.target_constraint {
                return Err(UnresolvedRelationValidationError::NonCanonicalCandidates);
            }
            for address in addresses {
                address
                    .validate()
                    .map_err(RelationValidationError::InvalidAddress)
                    .map_err(UnresolvedRelationValidationError::InvalidRelation)?;
            }
        }
        if let TargetConstraint::Syntactic { callee } = &self.target_constraint {
            require_nonempty(callee, "target_constraint.callee")
                .map_err(UnresolvedRelationValidationError::InvalidRelation)?;
        }
        let expected = compute_unresolved_relation_id(
            &self.source,
            self.predicate,
            &self.target_constraint,
            self.reason,
            &self.producer,
            &self.scope,
            self.occurrence_id.as_deref(),
        );
        if self.id != expected {
            return Err(UnresolvedRelationValidationError::IdentityMismatch);
        }
        Ok(())
    }
}

/// Scheme-explicit identity key with length-framed components. This preserves
/// address identity, not cross-scheme correspondence or display equivalence.
pub fn address_identity_key(address: &Address) -> String {
    match address {
        Address::CanonicalForm { digest } => format!("canonical_form:{digest}"),
        Address::LogicId {
            policy_id,
            structural_hash,
        } => format!(
            "logic_id:{}:{}",
            policy_id.as_str(),
            structural_hash.as_str()
        ),
        Address::ExactBody { exact_body_id } => {
            format!("exact_body:{}", exact_body_id.as_str())
        }
        Address::LogicAddress { logic_address } => format!("logic_address:{logic_address}"),
        Address::AddressId { address_id } => format!("address_id:{address_id}"),
        Address::RepoPath {
            repository_id,
            path,
            start_byte,
            end_byte,
        } => format!(
            "repo_path:{}",
            hash_parts(&[
                if repository_id.is_some() {
                    "some"
                } else {
                    "none"
                },
                repository_id.as_deref().unwrap_or(""),
                path,
                if start_byte.is_some() { "some" } else { "none" },
                &start_byte.map(|b| b.to_string()).unwrap_or_default(),
                if end_byte.is_some() { "some" } else { "none" },
                &end_byte.map(|b| b.to_string()).unwrap_or_default(),
            ])
        ),
        Address::GitOid { oid } => format!("git_oid:{oid}"),
        Address::Purl { purl } => format!("purl:{purl}"),
        Address::Resource {
            resource_system,
            resource_namespace,
            resource_type,
            logical_name,
        } => format!(
            "resource:{}",
            hash_parts(&[
                resource_system,
                resource_namespace,
                resource_type,
                logical_name,
            ])
        ),
        Address::Opaque { system, value } => format!("opaque:{}", hash_parts(&[system, value])),
    }
}

fn relation_kind_key(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::HasCanonicalForm => "has_canonical_form",
        RelationKind::Calls => "calls",
        RelationKind::Imports => "imports",
        RelationKind::Implements => "implements",
        RelationKind::DependsOn => "depends_on",
        RelationKind::Selects => "selects",
        RelationKind::RoutesTo => "routes_to",
        RelationKind::UsesConfiguration => "uses_configuration",
        RelationKind::ConnectsTo => "connects_to",
        RelationKind::ObservedAs => "observed_as",
    }
}

/// Stable snake-case wire spelling, shared by identities and query selectors.
pub fn unresolved_reason_key(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::IndirectCall => "indirect_call",
        UnresolvedReason::ProviderError => "provider_error",
        UnresolvedReason::UnresolvedBinding => "unresolved_binding",
        UnresolvedReason::AmbiguousTarget => "ambiguous_target",
        UnresolvedReason::TargetNotInInventory => "target_not_in_inventory",
        UnresolvedReason::ProducerNotInvoked => "producer_not_invoked",
        UnresolvedReason::UnsupportedProducer => "unsupported_producer",
        UnresolvedReason::OutOfScope => "out_of_scope",
        UnresolvedReason::DispatchNotEvaluated => "dispatch_not_evaluated",
        UnresolvedReason::BudgetExceeded => "budget_exceeded",
        UnresolvedReason::InvalidEnvironment => "invalid_environment",
        UnresolvedReason::UnsupportedConstruct => "unsupported_construct",
    }
}

fn subject_id_key(subject: &SubjectId) -> String {
    match subject {
        SubjectId::Address { address } => format!("addr:{}", address_identity_key(address)),
        SubjectId::Proposition { proposition_id } => {
            format!("proposition:{}", proposition_id.as_str())
        }
        SubjectId::RelationEdge { edge_id } => {
            format!("relation_edge:{}", edge_id.as_str())
        }
        SubjectId::RelationClaim { claim_id } => {
            format!("relation_claim:{}", claim_id.as_str())
        }
        SubjectId::Artifact { artifact_id } => format!("artifact:{artifact_id}"),
    }
}

/// Stable positive proposition, excluding snapshot, occurrence, producer, and evidence.
pub fn compute_proposition_id(
    schema_domain: &str,
    proposition_domain: &str,
    subject: &Address,
    predicate: RelationKind,
    object: &Address,
) -> PropositionId {
    let subject_key = address_identity_key(subject);
    let object_key = address_identity_key(object);
    let hex = hash_parts(&[
        "entmoot.proposition.v1",
        schema_domain,
        proposition_domain,
        subject_key.as_str(),
        relation_kind_key(predicate),
        object_key.as_str(),
    ]);
    PropositionId::from_trusted(format!("{DIGEST_PREFIX}{hex}"))
}

/// Snapshot/occurrence-specific RelationEdgeId (INV-EDGE-01 / INV-RES-01).
pub fn compute_relation_edge_id(
    schema_domain: &str,
    subject: &Address,
    predicate: RelationKind,
    object: &Address,
    scope: &Scope,
    occurrence_id: Option<&str>,
) -> RelationEdgeId {
    let proposition =
        compute_proposition_id(schema_domain, &scope.domain, subject, predicate, object);
    let mut owned: Vec<&str> = vec![
        "entmoot.relation_edge.v1",
        proposition.as_str(),
        occurrence_id.unwrap_or(""),
    ];
    let scope_parts = scope.canonical_parts();
    for p in &scope_parts {
        owned.push(p);
    }
    let hex = hash_parts(&owned);
    RelationEdgeId::from_trusted(format!("{DIGEST_PREFIX}{hex}"))
}

/// Producer-scoped RelationClaimId (INV-EDGE-03).
pub fn compute_relation_claim_id(
    subject: &SubjectId,
    predicate: &str,
    value: &RelationClaimValue,
    producer: &ProducerRef,
    scope: &Scope,
) -> RelationClaimId {
    let subject_key = subject_id_key(subject);
    let value_key = value.canonical();
    let version = producer.version.as_deref().unwrap_or("");
    let config = producer.config_fingerprint.as_deref().unwrap_or("");
    let mut owned: Vec<&str> = vec![
        "entmoot.relation_claim.v1",
        subject_key.as_str(),
        predicate,
        value_key.as_str(),
        producer.id.as_str(),
        version,
        config,
    ];
    let scope_parts = scope.canonical_parts();
    for p in &scope_parts {
        owned.push(p);
    }
    let hex = hash_parts(&owned);
    RelationClaimId::from_trusted(format!("{DIGEST_PREFIX}{hex}"))
}

/// Producer- and occurrence-scoped identity for one unresolved relation attempt.
pub fn compute_unresolved_relation_id(
    source: &Address,
    predicate: RelationKind,
    target_constraint: &TargetConstraint,
    reason: UnresolvedReason,
    producer: &ProducerRef,
    scope: &Scope,
    occurrence_id: Option<&str>,
) -> UnresolvedRelationId {
    let source_key = address_identity_key(source);
    let target_key = target_constraint.canonical_key();
    let version = producer.version.as_deref().unwrap_or("");
    let config = producer.config_fingerprint.as_deref().unwrap_or("");
    let mut parts = vec![
        "entmoot.unresolved_relation.v1",
        source_key.as_str(),
        relation_kind_key(predicate),
        target_key.as_str(),
        unresolved_reason_key(reason),
        producer.id.as_str(),
        version,
        config,
        occurrence_id.unwrap_or(""),
    ];
    let scope_parts = scope.canonical_parts();
    parts.extend(scope_parts);
    UnresolvedRelationId::from_trusted(format!("{DIGEST_PREFIX}{}", hash_parts(&parts)))
}

#[allow(clippy::too_many_arguments)]
pub fn unresolved_relation_with_id(
    source: Address,
    predicate: RelationKind,
    target_constraint: TargetConstraint,
    reason: UnresolvedReason,
    producer: ProducerRef,
    scope: Scope,
    occurrence_id: Option<String>,
    evidence: Vec<EvidenceRef>,
    provenance: Option<Provenance>,
    remaining_capability: Option<ProducerCapabilityKind>,
) -> UnresolvedRelation {
    let target_constraint = target_constraint.canonicalized();
    let id = compute_unresolved_relation_id(
        &source,
        predicate,
        &target_constraint,
        reason,
        &producer,
        &scope,
        occurrence_id.as_deref(),
    );
    UnresolvedRelation {
        id,
        source,
        predicate,
        target_constraint,
        reason,
        resolution_state: ResolutionState::Unknown,
        producer,
        scope,
        occurrence_id,
        evidence,
        provenance,
        remaining_capability,
    }
}

/// Topology node as opaque Address (structural call edges).
pub fn topology_node_address(node_id: &str) -> Address {
    Address::Opaque {
        system: "entmoot.topology_node".into(),
        value: node_id.into(),
    }
}

/// Query helper: collect resolution states for an edge (INV-EDGE-04).
pub fn resolution_states_for_relation_edge<'a>(
    edge_id: &RelationEdgeId,
    claims: impl IntoIterator<Item = &'a RelationClaim>,
) -> Vec<(ResolutionState, &'a ProducerRef)> {
    claims
        .into_iter()
        .filter_map(|c| {
            if c.state != RelationClaimState::Active {
                return None;
            }
            match &c.subject {
                SubjectId::RelationEdge { edge_id: eid } if eid == edge_id => {}
                _ => return None,
            }
            if c.predicate != "resolution_state" {
                return None;
            }
            match &c.value {
                RelationClaimValue::ResolutionState { state } => Some((*state, &c.producer)),
                _ => None,
            }
        })
        .collect()
}

/// Build RelationEdge + assign computed id.
pub fn relation_edge_with_id(
    schema_domain: &str,
    subject: Address,
    predicate: RelationKind,
    object: Address,
    scope: Scope,
    occurrence_id: Option<String>,
) -> RelationEdge {
    let proposition_id =
        compute_proposition_id(schema_domain, &scope.domain, &subject, predicate, &object);
    let id = compute_relation_edge_id(
        schema_domain,
        &subject,
        predicate,
        &object,
        &scope,
        occurrence_id.as_deref(),
    );
    RelationEdge {
        id,
        proposition_id,
        schema_domain: schema_domain.into(),
        subject,
        predicate,
        object,
        scope,
        occurrence_id,
    }
}

/// Build RelationClaim + assign computed id.
pub fn relation_claim_with_id(
    subject: SubjectId,
    predicate: impl Into<String>,
    value: RelationClaimValue,
    producer: ProducerRef,
    scope: Scope,
    evidence: Vec<EvidenceRef>,
    provenance: Option<Provenance>,
) -> RelationClaim {
    let predicate = predicate.into();
    let id = compute_relation_claim_id(&subject, &predicate, &value, &producer, &scope);
    RelationClaim {
        id,
        subject,
        predicate,
        value,
        producer,
        scope,
        evidence,
        provenance,
        state: RelationClaimState::Active,
        supports: Vec::new(),
        contradicts: Vec::new(),
        derivation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ArtifactId, ContentDigest};

    #[test]
    fn shared_callsite_claims_may_narrow_context_but_not_snapshot_or_occurrence() {
        let mut edge = relation_edge_with_id(
            "source",
            topology_node_address("caller"),
            RelationKind::Calls,
            topology_node_address("callee"),
            sample_scope(),
            Some("site".into()),
        );
        edge.scope.context_id = None;
        let projection = edge.scope.clone();
        assert!(edge.accepts_claim_scope(&projection));
        let mut native = projection.clone();
        native.context_id = Some("build".into());
        assert!(edge.accepts_claim_scope(&native));
        for field in ["repository", "revision", "digest", "occurrence"] {
            let mut foreign = native.clone();
            match field {
                "repository" => foreign.domain = "other".into(),
                "revision" => foreign.snapshot_revision = Some("other".into()),
                "digest" => foreign.snapshot_digest = Some(format!("blake3:{}", "f".repeat(64))),
                _ => foreign.occurrence_id = Some("other".into()),
            }
            assert!(!edge.accepts_claim_scope(&foreign), "{field}");
        }
        edge.occurrence_id = None;
        assert!(
            !edge.accepts_claim_scope(&native),
            "aggregate is not an exact site"
        );
        edge.occurrence_id = Some("site".into());
        edge.predicate = RelationKind::DependsOn;
        assert!(!edge.accepts_claim_scope(&native));
        edge.predicate = RelationKind::Calls;
        edge.scope.context_id = Some("other-build".into());
        assert!(!edge.accepts_claim_scope(&native));
        assert!(
            !edge.accepts_claim_scope(&projection),
            "cannot broaden context"
        );
    }

    #[test]
    fn unresolved_reason_keys_match_wire_serialization() {
        for reason in [
            UnresolvedReason::DispatchNotEvaluated,
            UnresolvedReason::BudgetExceeded,
            UnresolvedReason::InvalidEnvironment,
            UnresolvedReason::UnsupportedConstruct,
            UnresolvedReason::IndirectCall,
            UnresolvedReason::ProviderError,
            UnresolvedReason::UnresolvedBinding,
            UnresolvedReason::AmbiguousTarget,
            UnresolvedReason::TargetNotInInventory,
            UnresolvedReason::ProducerNotInvoked,
            UnresolvedReason::UnsupportedProducer,
            UnresolvedReason::OutOfScope,
        ] {
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                unresolved_reason_key(reason)
            );
        }
    }

    fn sample_scope() -> Scope {
        Scope {
            domain: "repo/demo".into(),
            snapshot_revision: Some("abc".into()),
            snapshot_digest: Some(format!("blake3:{}", "d".repeat(64))),
            occurrence_id: None,
            context_id: None,
        }
    }

    fn sample_producer(id: &str) -> ProducerRef {
        ProducerRef {
            id: id.into(),
            name: id.into(),
            version: Some("1".into()),
            config_fingerprint: None,
            environment_fingerprint: None,
            capabilities: vec![ProducerCapabilityKind::Resolver],
        }
    }

    #[test]
    fn relation_edge_id_ignores_producer() {
        let a = topology_node_address("n1");
        let b = topology_node_address("n2");
        let scope = sample_scope();
        let e1 = compute_relation_edge_id("source", &a, RelationKind::Calls, &b, &scope, None);
        let e2 = compute_relation_edge_id("source", &a, RelationKind::Calls, &b, &scope, None);
        assert_eq!(e1, e2);
        // Producer must not appear in formula — same inputs ⇒ same id (INV-EDGE-01).
        assert!(e1.as_str().starts_with(DIGEST_PREFIX));
    }

    #[test]
    fn proposition_survives_snapshot_and_occurrence_changes() {
        let subject = topology_node_address("n1");
        let object = topology_node_address("n2");
        let first = relation_edge_with_id(
            "source",
            subject.clone(),
            RelationKind::Calls,
            object.clone(),
            sample_scope(),
            Some("call-site-1".into()),
        );
        let mut next_scope = sample_scope();
        next_scope.snapshot_revision = Some("def".into());
        next_scope.snapshot_digest = Some(format!("blake3:{}", "e".repeat(64)));
        let second = relation_edge_with_id(
            "source",
            subject,
            RelationKind::Calls,
            object,
            next_scope,
            Some("call-site-2".into()),
        );

        assert_eq!(first.proposition_id, second.proposition_id);
        assert_eq!(first.proposition(), second.proposition());
        assert_ne!(first.id, second.id);
        assert!(first.proposition().validate().is_ok());
        assert!(first.validate().is_ok());
        assert!(second.validate().is_ok());
    }

    #[test]
    fn edge_and_claim_validation_reject_identity_field_drift() {
        let scope = sample_scope();
        let mut edge = relation_edge_with_id(
            "source",
            topology_node_address("n1"),
            RelationKind::Calls,
            topology_node_address("n2"),
            scope.clone(),
            None,
        );
        assert!(edge.validate().is_ok());
        edge.proposition_id = PropositionId::from_trusted(format!("blake3:{}", "a".repeat(64)));
        assert_eq!(
            edge.validate(),
            Err(RelationValidationError::PropositionIdentityMismatch)
        );

        edge.proposition_id = compute_proposition_id(
            &edge.schema_domain,
            &edge.scope.domain,
            &edge.subject,
            edge.predicate,
            &edge.object,
        );
        edge.scope.snapshot_revision = Some("def".into());
        assert_eq!(
            edge.validate(),
            Err(RelationValidationError::RelationEdgeIdentityMismatch)
        );

        let mut claim = relation_claim_with_id(
            SubjectId::RelationEdge { edge_id: edge.id },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Structural,
            },
            sample_producer("tree-sitter"),
            scope,
            vec![],
            None,
        );
        assert!(claim.validate().is_ok());
        claim.predicate = "changed".into();
        assert_eq!(
            claim.validate(),
            Err(RelationValidationError::RelationClaimIdentityMismatch)
        );
    }

    #[test]
    fn claim_can_target_stable_proposition() {
        let scope = sample_scope();
        let edge = relation_edge_with_id(
            "source",
            topology_node_address("n1"),
            RelationKind::Calls,
            topology_node_address("n2"),
            scope.clone(),
            None,
        );
        let claim = relation_claim_with_id(
            SubjectId::Proposition {
                proposition_id: edge.proposition_id,
            },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::SemanticResolved,
            },
            sample_producer("compiler"),
            scope,
            vec![],
            None,
        );
        assert!(claim.validate().is_ok());
    }

    #[test]
    fn relation_records_reject_malformed_embedded_addresses() {
        let malformed = Address::Opaque {
            system: String::new(),
            value: "node".into(),
        };
        let edge = relation_edge_with_id(
            "source",
            malformed.clone(),
            RelationKind::Calls,
            topology_node_address("n2"),
            sample_scope(),
            None,
        );
        assert!(matches!(
            edge.validate(),
            Err(RelationValidationError::InvalidAddress(_))
        ));

        let relation = unresolved_relation_with_id(
            topology_node_address("caller"),
            RelationKind::Calls,
            TargetConstraint::Candidates {
                addresses: vec![malformed],
            },
            UnresolvedReason::AmbiguousTarget,
            sample_producer("tree-sitter"),
            sample_scope(),
            Some("call-site-1".into()),
            vec![],
            None,
            Some(ProducerCapabilityKind::Resolver),
        );
        assert!(relation.validate().is_err());
    }

    #[test]
    fn relation_claim_id_differs_by_producer() {
        let a = topology_node_address("n1");
        let b = topology_node_address("n2");
        let scope = sample_scope();
        let edge = relation_edge_with_id("source", a, RelationKind::Calls, b, scope.clone(), None);
        let subject = SubjectId::RelationEdge { edge_id: edge.id };
        let value = RelationClaimValue::ResolutionState {
            state: ResolutionState::Structural,
        };
        let c1 = compute_relation_claim_id(
            &subject,
            "resolution_state",
            &value,
            &sample_producer("tree-sitter"),
            &scope,
        );
        let c2 = compute_relation_claim_id(
            &subject,
            "resolution_state",
            &value,
            &sample_producer("native-dotnet"),
            &scope,
        );
        assert_ne!(c1, c2);
    }

    #[test]
    fn refute_does_not_require_edge_mutation() {
        let a = topology_node_address("n1");
        let b = topology_node_address("n2");
        let scope = sample_scope();
        let edge = relation_edge_with_id("source", a, RelationKind::Calls, b, scope.clone(), None);
        let support = relation_claim_with_id(
            SubjectId::RelationEdge {
                edge_id: edge.id.clone(),
            },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::ArtifactResolved,
            },
            sample_producer("native-rust"),
            scope.clone(),
            vec![],
            None,
        );
        let mut refute = relation_claim_with_id(
            SubjectId::RelationEdge {
                edge_id: edge.id.clone(),
            },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Refuted,
            },
            sample_producer("roslyn"),
            scope,
            vec![],
            None,
        );
        refute.contradicts.push(support.id.clone());
        let claims = vec![support, refute];
        let states = resolution_states_for_relation_edge(&edge.id, &claims);
        assert_eq!(states.len(), 2);
        assert!(states
            .iter()
            .any(|(s, _)| *s == ResolutionState::ArtifactResolved));
        assert!(states.iter().any(|(s, _)| *s == ResolutionState::Refuted));
    }

    #[test]
    fn unknown_distinct_from_refuted() {
        assert_ne!(ResolutionState::Unknown, ResolutionState::Refuted);
    }

    #[test]
    fn unresolved_relation_identity_is_candidate_order_independent() {
        let source = topology_node_address("caller");
        let first = unresolved_relation_with_id(
            source.clone(),
            RelationKind::Calls,
            TargetConstraint::Candidates {
                addresses: vec![topology_node_address("b"), topology_node_address("a")],
            },
            UnresolvedReason::AmbiguousTarget,
            sample_producer("tree-sitter"),
            sample_scope(),
            Some("call-site-1".into()),
            vec![],
            None,
            Some(ProducerCapabilityKind::Resolver),
        );
        let second = unresolved_relation_with_id(
            source,
            RelationKind::Calls,
            TargetConstraint::Candidates {
                addresses: vec![topology_node_address("a"), topology_node_address("b")],
            },
            UnresolvedReason::AmbiguousTarget,
            sample_producer("tree-sitter"),
            sample_scope(),
            Some("call-site-1".into()),
            vec![],
            None,
            Some(ProducerCapabilityKind::Resolver),
        );
        assert_eq!(first.id, second.id);
        assert_eq!(first.target_constraint, second.target_constraint);
        assert_eq!(first.resolution_state, ResolutionState::Unknown);
        assert!(first.validate().is_ok());
    }

    #[test]
    fn unresolved_relation_rejects_non_unknown_state() {
        let mut relation = unresolved_relation_with_id(
            topology_node_address("caller"),
            RelationKind::Calls,
            TargetConstraint::Unknown,
            UnresolvedReason::UnresolvedBinding,
            sample_producer("tree-sitter"),
            sample_scope(),
            Some("call-site-1".into()),
            vec![],
            None,
            Some(ProducerCapabilityKind::Resolver),
        );
        relation.resolution_state = ResolutionState::Refuted;
        assert!(relation.validate().is_err());
    }

    #[test]
    fn unresolved_relation_wire_round_trip_is_strict() {
        let relation = unresolved_relation_with_id(
            topology_node_address("caller"),
            RelationKind::Calls,
            TargetConstraint::Syntactic {
                callee: "obj.call".into(),
            },
            UnresolvedReason::IndirectCall,
            sample_producer("tree-sitter"),
            sample_scope(),
            Some("call-site-1".into()),
            vec![],
            None,
            Some(ProducerCapabilityKind::Resolver),
        );
        let mut value = serde_json::to_value(&relation).unwrap();
        let round_trip: UnresolvedRelation = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(round_trip, relation);

        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), serde_json::json!(true));
        let error = serde_json::from_value::<UnresolvedRelation>(value).unwrap_err();
        assert!(error.to_string().contains("unknown field `unexpected`"));
    }

    #[test]
    fn content_digest_type_unused_in_relation_edge_id() {
        // Compile-time / API presence: ContentDigest / ArtifactId stay available for
        // claims/provenance only — RelationEdgeId formula never takes them.
        let _ = ContentDigest::from_trusted(
            "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let _ = ArtifactId::from_trusted(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
    }
}
