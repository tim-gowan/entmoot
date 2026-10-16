//! Provider-neutral batch artifact for native resolved resolver observations.
//!
//! `logic.resolver_artifact.v1` carries scheme-aware addresses, resolution
//! outcomes, optional call relation targets, evidence pointers, build-context
//! identity, and coverage inputs. Resolvers never rewrite `logic_address`,
//! `address_id`, or `structural_hash`.
//!
//! Provider-qualified symbol spellings are either a scheme-specific
//! [`Address`] (for example [`Address::Opaque`] with a conventional system) or
//! an attributable [`ProviderSymbolSpelling`] on [`ResolverTarget`]. There is
//! no universal cross-language FQN. Tree-sitter `resolved_fqn` remains an L2
//! matching aid outside this artifact.
//!
//! A `relation_target` may appear only for [`ResolverObservationOutcome::RepositoryCallable`]
//! when the target joins an inventoried callable. Other outcomes may still
//! emit multi-valued [`ResolverTarget`] rows for Resolution records.
//!
//! Binding outcomes and dispatch completeness are independent. Coverage groups
//! unresolved and ambiguous bindings without implying runtime indirection.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::provider_callable_ref::{
    ProviderCallableRef, ProviderCallableRefError, ProviderSymbolSpelling,
};
use crate::{Address, ContentDigest, SourceSnapshot};

pub const RESOLVER_ARTIFACT_SCHEMA: &str = "logic.resolver_artifact.v1";

/// Conventional `provider_symbol.scheme` / `Address::Opaque.system` values.
/// These are documentation constants, not an exhaustive enum of languages.
pub mod provider_symbol_schemes {
    pub const PYTHON_QUALIFIED_NAME: &str = "python.qualified_name";
    pub const JVM_METHOD_DESCRIPTOR: &str = "jvm.method_descriptor";
    pub const DOTNET_METADATA_NAME: &str = "dotnet.metadata_name";
    pub const RUST_DEF_PATH: &str = "rust.def_path";
    pub const CXX_USR: &str = "cxx.usr";
}

/// Entmoot occurrence/definition identity schemes that must not appear as
/// `provider_symbol.scheme` (provider spelling is not LogicId).
const FORBIDDEN_PROVIDER_SYMBOL_SCHEMES: &[&str] = &[
    "logic_id",
    "logic_address",
    "address_id",
    "exact_body",
    "structural_hash",
    "resolved_hash",
    "body_hash",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverArtifactProducer {
    pub id: String,
    pub name: String,
    pub version: String,
    pub method_id: String,
}

/// Disjoint outcome for one observed call site under a declared build context.
///
/// Positive outcomes require at least one target. Negative outcomes
/// (`unsupported`, `failed`, `unresolved`) forbid targets.
/// `ambiguous` allows zero or more candidate targets from conflicting facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolverObservationOutcome {
    RepositoryCallable,
    RepositorySymbol,
    IndexedDependency,
    ExternalSymbol,
    Intrinsic,
    Ambiguous,
    Unsupported,
    Failed,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverCallSource {
    pub call_site: Address,
    pub relation_source: Address,
    pub syntactic_callee: String,
}

/// Coarse, provider-neutral declaration category at a resolver target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolverDeclarationKind {
    Callable,
    Type,
    Value,
    Module,
    Unknown,
}

/// Physical origin of a declaration alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolverDeclarationOrigin {
    Source,
    Stub,
    Generated,
    Unknown,
}

/// How a declaration relates to an implementation callable, if at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolverCallableAssociation {
    Direct,
    AssignedValue,
    Transformed,
    ConstructorDispatch,
    Unknown,
}

/// Attributable declaration semantics for one target location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverDeclaration {
    pub kind: ResolverDeclarationKind,
    pub origin: ResolverDeclarationOrigin,
    pub callable_association: ResolverCallableAssociation,
    /// Versioned provider or adapter method that produced this classification.
    pub classification_method_id: String,
    /// Provider-native kind spelling, retained when the producer exposes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<String>,
}

/// One resolution target for an observation.
///
/// `address` is the scheme-aware Resolution endpoint. `relation_target` is
/// set only when the outcome is `repository_callable` and the target joins
/// an inventoried callable. `provider_symbol` is optional attributable
/// spelling and never rewrites structural identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverTarget {
    pub address: Address,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_target: Option<Address>,
    pub provider_pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_symbol: Option<ProviderSymbolSpelling>,
    /// Structured package PURL + in-package callable path (not LogicId).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_callable: Option<ProviderCallableRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration: Option<ResolverDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverObservation {
    /// Dispatch knowledge is independent of definition lookup. Omission is unknown.
    #[serde(default)]
    pub dispatch: CallDispatch,
    pub occurrence_id: String,
    pub source: ResolverCallSource,
    pub outcome: ResolverObservationOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<ResolverTarget>,
    /// Wire-auditable producer limitation; recoverable via artifact + pointer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limitation: Option<String>,
}

/// Provider statement about possible runtime targets, never evidence of execution.
/// Closed sets are relative to the artifact snapshot/build and explicit assumptions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CallDispatch {
    /// Definition lookup did not evaluate dispatch completeness.
    #[default]
    Unassessed,
    /// Known candidates may omit targets; the site must remain a traversal frontier.
    Open {
        /// Machine-readable explanation for the open target universe.
        reason: DispatchGap,
    },
    /// Producer asserts all possible targets under the stated assumptions.
    /// This does not assert that the call executes or authorize proof reuse.
    Closed {
        /// Versioned dispatch analysis policy, distinct from definition lookup.
        policy_id: String,
        /// Pointer into the immutable artifact supporting the completeness statement.
        evidence_pointer: String,
        /// Explicit environment/closed-world assumptions; never inferred from language.
        assumptions: Vec<String>,
    },
}

/// Language-neutral reasons to retain an indeterminate call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DispatchGap {
    /// Indirection was reported. The value is an extensible, language-qualified
    /// mechanism, not a closed enum or an assertion that static resolution is impossible.
    IndirectCall(String),
    /// Binding is unresolved. Retain unfamiliar provider reasons without
    /// guessing a dispatch mechanism from missing targets.
    UnresolvedBinding(String),
    ProviderError,
    ExternalImplementation,
    UnsupportedConstruct,
    BudgetExceeded,
    InvalidEnvironment,
}

impl DispatchGap {
    /// Normalize provider vocabulary once at the Entmoot boundary. Provider
    /// protocols may use legacy spellings; normalized artifacts do not.
    pub fn from_provider(language: &str, reason: &str) -> Self {
        if let Some(mechanism) = reason.strip_prefix("indirect_call:") {
            if !mechanism.trim().is_empty() {
                return Self::IndirectCall(format!("{language}.{mechanism}"));
            }
        }
        match reason {
            "indirect_call" | "dynamic_dispatch" | "reflection" | "runtime_mutation" => {
                Self::IndirectCall(format!("{language}.{reason}"))
            }
            "unsupported_construct" | "unsupported_declaration_or_stub" => {
                Self::UnsupportedConstruct
            }
            "budget_exceeded" => Self::BudgetExceeded,
            "invalid_environment" => Self::InvalidEnvironment,
            "external_implementation" => Self::ExternalImplementation,
            "provider_error" => Self::ProviderError,
            _ => Self::UnresolvedBinding(format!("{language}.{reason}")),
        }
    }

    /// Stable categories used by aggregate receipts and relationship frontiers.
    pub fn category(&self) -> &'static str {
        match self {
            Self::IndirectCall(_) => "indirect_call",
            Self::UnresolvedBinding(_) => "unresolved_binding",
            Self::ProviderError => "provider_error",
            Self::ExternalImplementation => "external_implementation",
            Self::UnsupportedConstruct => "unsupported_construct",
            Self::BudgetExceeded => "budget_exceeded",
            Self::InvalidEnvironment => "invalid_environment",
        }
    }

    /// Only explicit runtime-dependent mechanisms justify requesting a runtime
    /// probe. Unknown future mechanisms retain the static-analysis next step.
    pub fn requires_runtime_evidence(&self) -> bool {
        matches!(self, Self::IndirectCall(mechanism)
            if mechanism.rsplit_once('.').is_some_and(|(_, detail)|
                matches!(detail, "reflection" | "runtime_mutation")))
    }
}

impl CallDispatch {
    /// Whether the producer explicitly asserts target-set closure.
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Closed { .. })
    }
}

/// Batch of resolved resolver observations for one snapshot/language/build context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverArtifact {
    /// Non-call usage facts qualified against the same source inventory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assessments: Vec<crate::AssessmentObservation>,
    pub schema_version: String,
    pub implementation: crate::ImplementationFingerprint,
    pub producer: ResolverArtifactProducer,
    pub snapshot: SourceSnapshot,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_version: Option<String>,
    pub build_context_digest: ContentDigest,
    pub observations: Vec<ResolverObservation>,
    /// Producer-stated package exports and observed consumer contracts.
    /// Absence is incomplete evidence, never an empty export universe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_api: Option<crate::package_api::PackageApiEvidence>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolverArtifactValidationError {
    #[error("resolver artifact contract fingerprint mismatch")]
    IncompatibleContracts,
    #[error(transparent)]
    PackageApi(#[from] crate::package_api::PackageApiError),
    #[error("closed dispatch requires targets, a policy, evidence, and nonempty assumptions")]
    InvalidDispatchClosure,
    #[error("unsupported resolver artifact schema_version: {0}")]
    UnsupportedVersion(String),
    #[error("{0} must be non-empty")]
    EmptyField(&'static str),
    #[error("invalid resolver assessment: {0}")]
    InvalidAssessment(String),
    #[error("invalid resolver address: {0}")]
    InvalidAddress(crate::AddressValidationError),
    #[error("resolver outcome requires at least one target")]
    MissingTarget,
    #[error("repository_callable target requires relation_target")]
    MissingRelationTarget,
    #[error("repository_callable requires exactly one target; alternatives must remain ambiguous")]
    AmbiguousCallableTarget,
    #[error("relation_target is only allowed for repository_callable outcomes")]
    UnexpectedRelationTarget,
    #[error("non-positive resolver outcome cannot contain a unique target")]
    UnexpectedTarget,
    #[error("resolver observation targets must have unique provider_pointer values")]
    DuplicateProviderPointer,
    #[error("provider_symbol.scheme must not be an Entmoot identity scheme: {0}")]
    ForbiddenProviderSymbolScheme(String),
    #[error("invalid provider_callable: {0}")]
    InvalidProviderCallable(ProviderCallableRefError),
    #[error(
        "repository_callable declaration must have a direct or assigned_value callable association"
    )]
    NonCallableDeclarationAssociation,
    #[error("resolver observations must have sorted unique occurrence_id values")]
    NonCanonicalObservations,
    #[error("invalid resolver snapshot: {0}")]
    InvalidSnapshot(crate::SnapshotValidationError),
    #[error("invalid resolver build_context_digest: {0}")]
    InvalidBuildContext(crate::IdParseError),
}

impl ResolverArtifact {
    pub fn validate(&self) -> Result<(), ResolverArtifactValidationError> {
        if self.implementation.contracts != crate::contract_fingerprint() {
            return Err(ResolverArtifactValidationError::IncompatibleContracts);
        }
        for observation in &self.assessments {
            if !matches!(observation, crate::AssessmentObservation::CodeUse { .. }) {
                return Err(ResolverArtifactValidationError::InvalidAssessment(
                    "only usage facts are accepted".into(),
                ));
            }
            observation.validate().map_err(|error| {
                ResolverArtifactValidationError::InvalidAssessment(error.to_string())
            })?;
        }
        if let Some(evidence) = &self.package_api {
            evidence.validate(self)?;
        }
        if self.schema_version != RESOLVER_ARTIFACT_SCHEMA {
            return Err(ResolverArtifactValidationError::UnsupportedVersion(
                self.schema_version.clone(),
            ));
        }
        for (value, field) in [
            (&self.producer.id, "resolver.producer.id"),
            (&self.producer.name, "resolver.producer.name"),
            (&self.producer.version, "resolver.producer.version"),
            (&self.producer.method_id, "resolver.producer.method_id"),
            (&self.language, "resolver.language"),
        ] {
            if value.trim().is_empty() {
                return Err(ResolverArtifactValidationError::EmptyField(field));
            }
        }
        if self
            .language_version
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ResolverArtifactValidationError::EmptyField(
                "resolver.language_version",
            ));
        }
        self.snapshot
            .validate()
            .map_err(ResolverArtifactValidationError::InvalidSnapshot)?;
        ContentDigest::parse(self.build_context_digest.as_str())
            .map_err(ResolverArtifactValidationError::InvalidBuildContext)?;
        let mut previous: Option<&str> = None;
        for observation in &self.observations {
            if let CallDispatch::Open {
                reason: DispatchGap::IndirectCall(detail) | DispatchGap::UnresolvedBinding(detail),
            } = &observation.dispatch
            {
                if detail.trim().is_empty() {
                    return Err(ResolverArtifactValidationError::EmptyField(
                        "resolver.dispatch.reason",
                    ));
                }
            }
            if let CallDispatch::Closed {
                policy_id,
                evidence_pointer,
                assumptions,
            } = &observation.dispatch
            {
                if observation.targets.is_empty()
                    || policy_id.trim().is_empty()
                    || evidence_pointer.trim().is_empty()
                    || assumptions.is_empty()
                    || assumptions.iter().any(|value| value.trim().is_empty())
                    || matches!(
                        observation.outcome,
                        ResolverObservationOutcome::Unsupported
                            | ResolverObservationOutcome::Failed
                            | ResolverObservationOutcome::Unresolved
                    )
                {
                    return Err(ResolverArtifactValidationError::InvalidDispatchClosure);
                }
            }
            if observation.occurrence_id.trim().is_empty()
                || observation.source.syntactic_callee.trim().is_empty()
            {
                return Err(ResolverArtifactValidationError::EmptyField(
                    "resolver.observation",
                ));
            }
            if previous.is_some_and(|value| value >= observation.occurrence_id.as_str()) {
                return Err(ResolverArtifactValidationError::NonCanonicalObservations);
            }
            previous = Some(&observation.occurrence_id);
            observation
                .source
                .call_site
                .validate()
                .map_err(ResolverArtifactValidationError::InvalidAddress)?;
            observation
                .source
                .relation_source
                .validate()
                .map_err(ResolverArtifactValidationError::InvalidAddress)?;

            let mut seen_pointers = std::collections::BTreeSet::new();
            for target in &observation.targets {
                target
                    .address
                    .validate()
                    .map_err(ResolverArtifactValidationError::InvalidAddress)?;
                if let Some(relation_target) = &target.relation_target {
                    relation_target
                        .validate()
                        .map_err(ResolverArtifactValidationError::InvalidAddress)?;
                }
                if target.provider_pointer.trim().is_empty() {
                    return Err(ResolverArtifactValidationError::EmptyField(
                        "resolver.target.provider_pointer",
                    ));
                }
                if !seen_pointers.insert(target.provider_pointer.as_str()) {
                    return Err(ResolverArtifactValidationError::DuplicateProviderPointer);
                }
                if let Some(provider_symbol) = &target.provider_symbol {
                    if provider_symbol.scheme.trim().is_empty() {
                        return Err(ResolverArtifactValidationError::EmptyField(
                            "resolver.target.provider_symbol.scheme",
                        ));
                    }
                    if provider_symbol.spelling.trim().is_empty() {
                        return Err(ResolverArtifactValidationError::EmptyField(
                            "resolver.target.provider_symbol.spelling",
                        ));
                    }
                    if FORBIDDEN_PROVIDER_SYMBOL_SCHEMES.contains(&provider_symbol.scheme.as_str())
                    {
                        return Err(
                            ResolverArtifactValidationError::ForbiddenProviderSymbolScheme(
                                provider_symbol.scheme.clone(),
                            ),
                        );
                    }
                }
                if let Some(provider_callable) = &target.provider_callable {
                    provider_callable
                        .validate()
                        .map_err(ResolverArtifactValidationError::InvalidProviderCallable)?;
                }
                if observation.outcome == ResolverObservationOutcome::RepositoryCallable
                    && target.declaration.as_ref().is_some_and(|declaration| {
                        !matches!(
                            declaration.callable_association,
                            ResolverCallableAssociation::Direct
                                | ResolverCallableAssociation::AssignedValue
                        )
                    })
                {
                    return Err(ResolverArtifactValidationError::NonCallableDeclarationAssociation);
                }
                if target.declaration.as_ref().is_some_and(|declaration| {
                    declaration.classification_method_id.trim().is_empty()
                        || declaration
                            .provider_kind
                            .as_deref()
                            .is_some_and(|kind| kind.trim().is_empty())
                }) {
                    return Err(ResolverArtifactValidationError::EmptyField(
                        "resolver.target.declaration",
                    ));
                }
            }

            let positive = matches!(
                observation.outcome,
                ResolverObservationOutcome::RepositoryCallable
                    | ResolverObservationOutcome::RepositorySymbol
                    | ResolverObservationOutcome::IndexedDependency
                    | ResolverObservationOutcome::ExternalSymbol
                    | ResolverObservationOutcome::Intrinsic
            );
            if positive && observation.targets.is_empty() {
                return Err(ResolverArtifactValidationError::MissingTarget);
            }
            if observation.outcome == ResolverObservationOutcome::RepositoryCallable {
                if observation.targets.len() != 1 {
                    return Err(ResolverArtifactValidationError::AmbiguousCallableTarget);
                }
                if observation
                    .targets
                    .iter()
                    .any(|target| target.relation_target.is_none())
                {
                    return Err(ResolverArtifactValidationError::MissingRelationTarget);
                }
            } else if observation
                .targets
                .iter()
                .any(|target| target.relation_target.is_some())
            {
                return Err(ResolverArtifactValidationError::UnexpectedRelationTarget);
            }
            if matches!(
                observation.outcome,
                ResolverObservationOutcome::Unsupported
                    | ResolverObservationOutcome::Failed
                    | ResolverObservationOutcome::Unresolved
            ) && !observation.targets.is_empty()
            {
                return Err(ResolverArtifactValidationError::UnexpectedTarget);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_gap_aliases_converge_without_guessing_unknown_mechanisms() {
        use super::DispatchGap;
        for reason in [
            "indirect_call",
            "dynamic_dispatch",
            "reflection",
            "runtime_mutation",
        ] {
            let gap = DispatchGap::from_provider("python", reason);
            assert_eq!(gap, DispatchGap::IndirectCall(format!("python.{reason}")));
            assert_eq!(gap.category(), "indirect_call");
        }
        let future = DispatchGap::from_provider("julia", "indirect_call:multiple_dispatch");
        assert_eq!(
            future,
            DispatchGap::IndirectCall("julia.multiple_dispatch".into())
        );
        assert!(!future.requires_runtime_evidence());
        assert!(DispatchGap::from_provider("python", "reflection").requires_runtime_evidence());
        for reason in [
            "unresolved",
            "unresolved_or_dynamic",
            "new_reason",
            "indirect_call:",
        ] {
            let gap = DispatchGap::from_provider("typescript", reason);
            assert_eq!(
                gap,
                DispatchGap::UnresolvedBinding(format!("typescript.{reason}"))
            );
            assert_eq!(gap.category(), "unresolved_binding");
        }
        for (reason, expected) in [
            (
                "unsupported_declaration_or_stub",
                DispatchGap::UnsupportedConstruct,
            ),
            ("unsupported_construct", DispatchGap::UnsupportedConstruct),
            ("provider_error", DispatchGap::ProviderError),
            ("invalid_environment", DispatchGap::InvalidEnvironment),
            ("budget_exceeded", DispatchGap::BudgetExceeded),
        ] {
            assert_eq!(DispatchGap::from_provider("python", reason), expected);
        }
    }

    #[test]
    fn open_dispatch_requires_a_nonempty_mechanism_or_provider_detail() {
        for reason in [
            DispatchGap::IndirectCall(" ".into()),
            DispatchGap::UnresolvedBinding("".into()),
        ] {
            let artifact = base_artifact(vec![ResolverObservation {
                dispatch: CallDispatch::Open { reason },
                occurrence_id: "call".into(),
                source: source("call", "caller", "target"),
                outcome: ResolverObservationOutcome::Unresolved,
                targets: vec![],
                limitation: None,
            }]);
            assert!(matches!(
                artifact.validate(),
                Err(ResolverArtifactValidationError::EmptyField(
                    "resolver.dispatch.reason"
                ))
            ));
        }
    }

    #[test]
    fn indirect_call_marker_accepts_new_mechanisms_without_new_variants() {
        for mechanism in [
            "cxx.virtual_method",
            "go.function_value",
            "python.attribute_lookup",
            "java.reflection",
            "julia.multiple_dispatch",
            "c.dynamic_loader",
        ] {
            let value = serde_json::json!({"kind":"open", "reason":{"indirect_call":mechanism}});
            let dispatch: super::CallDispatch = serde_json::from_value(value.clone()).unwrap();
            assert!(!dispatch.is_closed());
            assert_eq!(serde_json::to_value(dispatch).unwrap(), value);
        }
    }

    use super::*;
    use crate::{topology_node_address, SourceState};

    fn digest() -> ContentDigest {
        ContentDigest::from_trusted(format!("blake3:{}", "b".repeat(64)))
    }

    fn snapshot() -> SourceSnapshot {
        SourceSnapshot {
            repository_id: "example/repo".into(),
            revision: "a".repeat(40),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        }
    }

    fn producer() -> ResolverArtifactProducer {
        ResolverArtifactProducer {
            id: "resolver".into(),
            name: "resolver".into(),
            version: "1".into(),
            method_id: "definition.v1".into(),
        }
    }

    fn source(occurrence: &str, caller: &str, callee: &str) -> ResolverCallSource {
        ResolverCallSource {
            call_site: topology_node_address(occurrence),
            relation_source: topology_node_address(caller),
            syntactic_callee: callee.into(),
        }
    }

    fn base_artifact(observations: Vec<ResolverObservation>) -> ResolverArtifact {
        ResolverArtifact {
            assessments: vec![],
            package_api: None,
            schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
            implementation: crate::ImplementationFingerprint {
                contracts: crate::contract_fingerprint(),
                derivation: digest(),
            },
            producer: producer(),
            snapshot: snapshot(),
            language: "python".into(),
            language_version: Some("3.13".into()),
            build_context_digest: digest(),
            observations,
        }
    }

    #[test]
    fn repository_callable_requires_a_relation_target() {
        let mut artifact = base_artifact(vec![ResolverObservation {
            dispatch: Default::default(),
            occurrence_id: "call".into(),
            source: source("call", "caller", "target"),
            outcome: ResolverObservationOutcome::RepositoryCallable,
            targets: vec![ResolverTarget {
                address: topology_node_address("target"),
                relation_target: None,
                provider_pointer: "targets/0".into(),
                provider_symbol: None,
                provider_callable: None,
                declaration: None,
            }],
            limitation: None,
        }]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::MissingRelationTarget)
        ));
        artifact.observations[0].targets[0].relation_target = Some(topology_node_address("target"));
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn closed_dispatch_cannot_hide_skipped_or_empty_target_sets() {
        let mut artifact = base_artifact(vec![ResolverObservation {
            dispatch: CallDispatch::Closed {
                policy_id: "fixture.dispatch.v1".into(),
                evidence_pointer: "/observations/0/dispatch".into(),
                assumptions: vec!["fixed build".into()],
            },
            occurrence_id: "dynamic".into(),
            source: source("dynamic", "caller", "callback"),
            outcome: ResolverObservationOutcome::Unsupported,
            targets: vec![],
            limitation: Some("reflection".into()),
        }]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::InvalidDispatchClosure)
        ));
        artifact.observations[0].outcome = ResolverObservationOutcome::ExternalSymbol;
        artifact.observations[0].targets.push(ResolverTarget {
            address: crate::Address::Opaque {
                system: "external.api".into(),
                value: "f".into(),
            },
            relation_target: None,
            provider_pointer: "/target".into(),
            provider_symbol: None,
            provider_callable: None,
            declaration: None,
        });
        assert!(artifact.validate().is_ok());
        if let CallDispatch::Closed { assumptions, .. } = &mut artifact.observations[0].dispatch {
            assumptions.clear();
        }
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::InvalidDispatchClosure)
        ));
    }

    #[test]
    fn repository_callable_rejects_constructor_or_transformed_association() {
        let mut artifact = base_artifact(vec![ResolverObservation {
            dispatch: Default::default(),
            occurrence_id: "call".into(),
            source: source("call", "caller", "Widget"),
            outcome: ResolverObservationOutcome::RepositoryCallable,
            targets: vec![ResolverTarget {
                address: topology_node_address("target"),
                relation_target: Some(topology_node_address("target")),
                provider_pointer: "targets/0".into(),
                provider_symbol: None,
                provider_callable: None,
                declaration: Some(ResolverDeclaration {
                    kind: ResolverDeclarationKind::Type,
                    origin: ResolverDeclarationOrigin::Source,
                    callable_association: ResolverCallableAssociation::ConstructorDispatch,
                    classification_method_id: "entmoot.python_ast.declaration.v1".into(),
                    provider_kind: Some("class_definition".into()),
                }),
            }],
            limitation: None,
        }]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::NonCallableDeclarationAssociation)
        ));
        artifact.observations[0].targets[0]
            .declaration
            .as_mut()
            .unwrap()
            .callable_association = ResolverCallableAssociation::AssignedValue;
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn non_callable_outcomes_forbid_relation_target() {
        let artifact = base_artifact(vec![ResolverObservation {
            dispatch: Default::default(),
            occurrence_id: "decl".into(),
            source: source("decl", "caller", "Widget"),
            outcome: ResolverObservationOutcome::RepositorySymbol,
            targets: vec![ResolverTarget {
                address: Address::RepoPath {
                    repository_id: Some("example/repo".into()),
                    path: "pkg/widget.py".into(),
                    start_byte: Some(10),
                    end_byte: Some(20),
                },
                relation_target: Some(topology_node_address("fake")),
                provider_pointer: "targets/0".into(),
                provider_symbol: Some(ProviderSymbolSpelling {
                    scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                    spelling: "pkg.widget.Widget".into(),
                }),
                provider_callable: None,
                declaration: None,
            }],
            limitation: Some("declaration without callable inventory join".into()),
        }]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::UnexpectedRelationTarget)
        ));
    }

    #[test]
    fn rejects_entmoot_identity_as_provider_symbol_scheme() {
        let artifact = base_artifact(vec![ResolverObservation {
            dispatch: Default::default(),
            occurrence_id: "call".into(),
            source: source("call", "caller", "target"),
            outcome: ResolverObservationOutcome::RepositoryCallable,
            targets: vec![ResolverTarget {
                address: topology_node_address("target"),
                relation_target: Some(topology_node_address("target")),
                provider_pointer: "targets/0".into(),
                provider_symbol: Some(ProviderSymbolSpelling {
                    scheme: "logic_address".into(),
                    spelling: "logic|example/repo|python|a.py|m|f|function|0|1".into(),
                }),
                provider_callable: None,
                declaration: None,
            }],
            limitation: None,
        }]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::ForbiddenProviderSymbolScheme(_))
        ));
    }

    #[test]
    fn rejects_empty_language_version_and_duplicate_pointers() {
        let mut artifact = base_artifact(vec![ResolverObservation {
            dispatch: Default::default(),
            occurrence_id: "call".into(),
            source: source("call", "caller", "target"),
            outcome: ResolverObservationOutcome::Intrinsic,
            targets: vec![
                ResolverTarget {
                    address: Address::Opaque {
                        system: "pyright.typeshed_uri".into(),
                        value: "file:///typeshed/stdlib/builtins.pyi".into(),
                    },
                    relation_target: None,
                    provider_pointer: "targets/0".into(),
                    provider_symbol: None,
                    provider_callable: None,
                    declaration: None,
                },
                ResolverTarget {
                    address: Address::Opaque {
                        system: "pyright.typeshed_uri".into(),
                        value: "file:///typeshed/stdlib/builtins.pyi#len".into(),
                    },
                    relation_target: None,
                    provider_pointer: "targets/0".into(),
                    provider_symbol: None,
                    provider_callable: None,
                    declaration: None,
                },
            ],
            limitation: None,
        }]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::DuplicateProviderPointer)
        ));

        artifact.observations[0].targets[1].provider_pointer = "targets/1".into();
        artifact.language_version = Some("  ".into());
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::EmptyField(
                "resolver.language_version"
            ))
        ));
    }

    #[test]
    fn rejects_unsorted_occurrence_ids() {
        let artifact = base_artifact(vec![
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "b".into(),
                source: source("b", "caller", "x"),
                outcome: ResolverObservationOutcome::Unresolved,
                targets: vec![],
                limitation: Some("no definition".into()),
            },
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "a".into(),
                source: source("a", "caller", "y"),
                outcome: ResolverObservationOutcome::Unresolved,
                targets: vec![],
                limitation: None,
            },
        ]);
        assert!(matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::NonCanonicalObservations)
        ));
    }

    #[test]
    fn five_scenario_fixture_validates_and_round_trips() {
        let artifact = base_artifact(vec![
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "call-intrinsic".into(),
                source: source("call-intrinsic", "caller", "len"),
                outcome: ResolverObservationOutcome::Intrinsic,
                targets: vec![ResolverTarget {
                    address: Address::Opaque {
                        system: "pyright.typeshed_uri".into(),
                        value: "file:///typeshed/stdlib/builtins.pyi".into(),
                    },
                    relation_target: None,
                    provider_pointer: "targets/0".into(),
                    provider_symbol: Some(ProviderSymbolSpelling {
                        scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                        spelling: "builtins.len".into(),
                    }),
                    provider_callable: None,
                    declaration: None,
                }],
                limitation: None,
            },
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "call-jvm".into(),
                source: source("call-jvm", "caller", "baz"),
                outcome: ResolverObservationOutcome::ExternalSymbol,
                targets: vec![ResolverTarget {
                    address: Address::Opaque {
                        system: provider_symbol_schemes::JVM_METHOD_DESCRIPTOR.into(),
                        value: "Lcom/foo/Bar;.baz:(I)V".into(),
                    },
                    relation_target: None,
                    provider_pointer: "targets/0".into(),
                    provider_symbol: Some(ProviderSymbolSpelling {
                        scheme: provider_symbol_schemes::JVM_METHOD_DESCRIPTOR.into(),
                        spelling: "Lcom/foo/Bar;.baz:(I)V".into(),
                    }),
                    provider_callable: None,
                    declaration: None,
                }],
                limitation: None,
            },
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "call-python".into(),
                source: source("call-python", "caller", "helper"),
                outcome: ResolverObservationOutcome::RepositoryCallable,
                targets: vec![ResolverTarget {
                    address: topology_node_address("helper"),
                    relation_target: Some(topology_node_address("helper")),
                    provider_pointer: "targets/0".into(),
                    provider_symbol: Some(ProviderSymbolSpelling {
                        scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                        spelling: "pkg.mod.helper".into(),
                    }),
                    provider_callable: None,
                    declaration: None,
                }],
                limitation: None,
            },
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "call-rust".into(),
                source: source("call-rust", "caller", "Symbol"),
                outcome: ResolverObservationOutcome::ExternalSymbol,
                targets: vec![ResolverTarget {
                    address: Address::Opaque {
                        system: provider_symbol_schemes::RUST_DEF_PATH.into(),
                        value: "example::Symbol".into(),
                    },
                    relation_target: None,
                    provider_pointer: "targets/0".into(),
                    provider_symbol: Some(ProviderSymbolSpelling {
                        scheme: provider_symbol_schemes::RUST_DEF_PATH.into(),
                        spelling: "example::Symbol".into(),
                    }),
                    provider_callable: None,
                    declaration: None,
                }],
                limitation: None,
            },
            ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "decl-class".into(),
                source: source("decl-class", "caller", "Widget"),
                outcome: ResolverObservationOutcome::RepositorySymbol,
                targets: vec![ResolverTarget {
                    address: Address::RepoPath {
                        repository_id: Some("example/repo".into()),
                        path: "pkg/widget.py".into(),
                        start_byte: Some(40),
                        end_byte: Some(80),
                    },
                    relation_target: None,
                    provider_pointer: "targets/0".into(),
                    provider_symbol: Some(ProviderSymbolSpelling {
                        scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                        spelling: "pkg.widget.Widget".into(),
                    }),
                    provider_callable: None,
                    declaration: Some(ResolverDeclaration {
                        kind: ResolverDeclarationKind::Type,
                        origin: ResolverDeclarationOrigin::Source,
                        callable_association: ResolverCallableAssociation::ConstructorDispatch,
                        classification_method_id: "entmoot.python_ast.declaration.v1".into(),
                        provider_kind: Some("class_definition".into()),
                    }),
                }],
                limitation: Some(
                    "repository declaration without an inventoried callable target".into(),
                ),
            },
        ]);
        assert!(artifact.validate().is_ok());

        let json = serde_json::to_string_pretty(&artifact).expect("serialize");
        assert!(json.contains("provider_symbol"));
        assert!(json.contains("python.qualified_name"));
        assert!(json.contains("jvm.method_descriptor"));
        assert!(json.contains("rust.def_path"));
        assert_eq!(json.matches("\"relation_target\"").count(), 1);

        let decoded: ResolverArtifact = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, artifact);
        assert!(decoded.validate().is_ok());
        assert_eq!(decoded.observations.len(), 5);
        assert!(decoded.observations[2].targets[0].relation_target.is_some());
        assert!(decoded.observations[4].targets[0].relation_target.is_none());
        assert_eq!(
            decoded.observations[4].targets[0]
                .declaration
                .as_ref()
                .map(|value| value.callable_association),
            Some(ResolverCallableAssociation::ConstructorDispatch)
        );
        assert_eq!(
            decoded.observations[4].limitation.as_deref(),
            Some("repository declaration without an inventoried callable target")
        );
    }
}
