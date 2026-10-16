//! Shared schema types for logic-addressable indexing and evidence contracts.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

mod address;
mod assessments;
pub use assessments::*;
mod artifact;
mod assertion;
mod attachment;
mod canon;
mod capability;
pub use canon::*;
mod adapter_profile;
mod home_config;
mod code_contract;
mod component;
mod entity;
mod exchange;
mod finding;
mod foundational;
mod identity;
mod ids;
mod implementation;
mod intoto_interop;
mod json_schema_emit;
mod location;
mod markers;
mod native;
mod normalization;
mod normalization_validation;
mod occurrence;
mod operations;
pub use operations::*;
pub mod package_api;
mod paths;
mod property;
mod provider_callable_ref;
pub mod query;
mod relation;
mod resolved_identity;
mod resolver_accounting;
mod resolver_artifact;
mod resolver_coverage;
mod revision_campaign;
pub use resolved_identity::{
    ResolvedBindingEvidence, ResolvedIdentityCompleteness, ResolvedIdentityRecord,
    ResolverEvidenceRef,
};
pub use resolver_accounting::*;
pub use revision_campaign::{
    CampaignBudget, CampaignComparison, CampaignComparisonRelation, CampaignEnrichment,
    CampaignInputError, CampaignMaterial, CampaignMaterialRole, CampaignRevision,
    RevisionCampaignInputs, RevisionCampaignSeries,
};
mod resource;
mod resource_artifact;
mod scope;
mod snapshot;
mod source_subject;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use adapter_profile::{
    AdapterFamilyAvailability, AdapterIntegrity, AdapterProfile, AdapterProfileFamily,
    AdapterProfileSelection, ADAPTER_PROFILE_FILE, ADAPTER_PROFILE_SCHEMA,
    OBSOLETE_NATIVE_PROFILE_FILE,
};
pub use home_config::{
    EntmootConfig, ENTMOOT_CONFIG_ENV_KEYS, ENTMOOT_CONFIG_FILE, ENTMOOT_CONFIG_SCHEMA,
    ENTMOOT_DEFAULT_STORE,
};
pub use native::{
    CapturedOutput, ExecutionPolicy, NativeCallFact, NativeCallTarget, NativeProducerContext,
    NativeRawEnvelope, NativeRawObservation, NativeRawSeries, NativeReceipt, NativeReceiptSeries,
    NativeReferenceFact, NativeSemanticStatus, NativeSidecarOutput, NativeSidecarRun,
    NativeSpanRef, NativeTargetLocation, RustAnalysisMode, RustBuildProfile, RustFeatureSelection,
    ToolEnvironment, ToolSelection,
};

pub use normalization::{NormalizationAudit, StructuralHashPolicy, StructuralPolicySeries};
pub use normalization_validation::{
    NormalizationCaseExpectation, NormalizationCaseOutcome, NormalizationCaseResult,
    NormalizationObservation, NormalizationReportFormat, NormalizationSyntaxExpectation,
    NormalizationValidationReport, StructuralHashRelation, SyntaxAcceptance,
};

pub use address::{purl_address, resource_address, Address, AddressValidationError, Subject};
pub use artifact::{
    ArtifactAttachReceipt, ArtifactFormat, ArtifactIngestStats, ArtifactManifest, ArtifactProducer,
    ArtifactRef, ManifestValidationError, ARTIFACT_MANIFEST_SCHEMA,
};
pub use assertion::{Assertion, AssertionObject, DerivationRecord, EpistemicStatus, EvidenceRef};
pub use attachment::{
    AttachmentDisposition, AttachmentFreshness, AttachmentReason, AttachmentTarget,
    AttachmentTargetKind, CallPort, LocationAttachment,
};
pub use capability::{CapabilityObservation, CapabilityOutcome, EvidenceCapability};
pub use code_contract::{
    CodeContract, DeclaredContract, DeclaredExpression, DeclaredMember, DeclaredParameter,
    DeclaredType, MemberKind, ParameterKind,
};
pub use component::{
    reference_outcome, AdvisoryAffectedPackage, AdvisoryRange, AdvisoryRangeEvent, AffectedVersion,
    BomComposition, BomReferenceOutcome, ComponentAffect, ComponentDependency,
    ComponentDependencyKind, ComponentOccurrence, ComponentValidationError, ComponentVulnerability,
    VexAnalysis, VulnerabilityRating, BOM_COMPOSITION_SCHEMA, COMPONENT_DEPENDENCY_SCHEMA,
    COMPONENT_OCCURRENCE_SCHEMA, COMPONENT_VULNERABILITY_SCHEMA,
};
pub use entity::{EntityKind, EntityRef, Locator, ObservationContext};
pub use exchange::{
    KnowledgeExchangeValidationError, KnowledgePublicationManifest, KnowledgePublicationMode,
    KnowledgeSegmentDescriptor, KnowledgeVisibility, KnowledgeVisibilityEvent,
    KnowledgeVisibilityPage, KnowledgeVisibilityReason, PublicationEmitter,
    KNOWLEDGE_PUBLICATION_SCHEMA, KNOWLEDGE_VISIBILITY_PAGE_SCHEMA,
};
pub use finding::{
    FindingEntity, FindingObservation, LifecycleEvent, LifecycleEventKind, LocationRecord,
    SuppressionDetail, TaxonomyRef, TraceRecord, TraceStepRecord,
};
pub use foundational::{
    validate_lah_local_emission, AnalysisEnvironmentFingerprint, ClaimScope, CompletenessAssertion,
    InvocationMode, LocationRef, LocationResolveStatus, LogicId, PredicateMaturity, PredicateRef,
    ProducerCapabilities, ProducerIdentity, Resolution, ResolutionAttempt, ResolutionAttemptStatus,
    ResolutionAttemptValidationError, ResolutionValidationError, ScopePromotionError, Validity,
};
pub use identity::{
    assertion_id, attachment_id, blake3_hex, content_manifest_digest, derive_artifact_id,
    derive_artifact_series_id, file_location_id, finding_observation_id, hash_parts,
    is_immutable_revision, location_id, observed_document_set_digest, scope_universe_id, step_id,
    strip_digest_prefix, topo_call_stub_id, topo_callable_id, topo_control_site_id,
    topo_declaration_id, topo_file_id, topo_import_id, topo_module_stub_id, topo_repo_id, trace_id,
    DIGEST_PREFIX, IDENTITY_SCHEMA_VERSION,
};
pub use ids::{
    AddressId, ArtifactId, AttachmentId, ContentDigest, ExactBodyId, FindingId, IdParseError,
    PolicyId, PublicationId, ResolvedHash, StructuralHash,
};
pub use implementation::{
    contract_fingerprint, ImplementationFingerprint, StorageFingerprint, StorageSeries,
};
pub use intoto_interop::{
    export_artifact_assertion, import_artifact_statement, InTotoDigestSet,
    InTotoResourceDescriptor, InTotoStyleStatement,
};
pub use json_schema_emit::{emit_contract_schemas_json, CORE_CONTRACT_CATALOG_ID};
pub use location::{
    utf16_column_to_codepoint, utf8_column_to_codepoint, ColumnEncoding, CoordinateError,
    LocationExtent, SourceLocation, SourcePosition, SourceRegion, SourceTextEnd, SourceTextRange,
    VerifiedSourceSpan,
};
pub use markers::{
    decimal_integer, ArgumentSite, ArgumentSiteKind, BindingKind, BindingMarker, CallSiteMarker,
    CallSitePorts, CallSiteRecord, ControlSite, ControlSiteKind, DeclarationMarker,
    DeclarationSiteRecord, DeclarationSyntax, DefMarker, DefinitionSiteRef, FileMarkers,
    ImportKind, ImportMarker, InitializerSyntax, InitializerValue, SiteCompleteness,
    SiteLimitation, SiteSpan,
};
pub use occurrence::{OccurrenceRef, SourceOccurrenceRef};
pub use paths::{normalize_repo_path, InvalidRepoPath};
pub use property::{PropertyDependencySpec, PropertyStoreKey};
pub use property::{
    PropertyInputError, PropertyInputManifest, PropertyInputValue, REQUIRED_PROPERTY_INPUTS,
};
pub use provider_callable_ref::{
    decode_provider_callable_ref, display_from_path, encode_provider_callable_ref,
    path_from_go_qualified_name, purl_cargo, purl_golang_module, purl_golang_stdlib, purl_maven,
    purl_npm, purl_nuget, purl_pypi, DecodedProviderCallableParts, ProviderCallableKind,
    ProviderCallableRef, ProviderCallableRefError, ProviderPackageRef, ProviderPackageRole,
    ProviderPathKind, ProviderPathSegment, ProviderSymbolSpelling, PROVIDER_CALLABLE_REF_SCHEMA,
};
pub use relation::{
    address_identity_key, compute_proposition_id, compute_relation_claim_id,
    compute_relation_edge_id, compute_unresolved_relation_id, relation_claim_with_id,
    relation_edge_with_id, resolution_states_for_relation_edge, topology_node_address,
    unresolved_reason_key, unresolved_relation_with_id, ProducerCapabilityKind, ProducerRef,
    Proposition, PropositionId, Provenance, RelationClaim, RelationClaimId, RelationClaimState,
    RelationClaimValue, RelationEdge, RelationEdgeId, RelationKind, RelationValidationError,
    ResolutionState, Scope, SubjectId, TargetConstraint, UnresolvedReason, UnresolvedRelation,
    UnresolvedRelationId, UnresolvedRelationValidationError,
};
pub use resolver_artifact::{
    provider_symbol_schemes, CallDispatch, DispatchGap, ResolverArtifact, ResolverArtifactProducer,
    ResolverArtifactValidationError, ResolverCallSource, ResolverCallableAssociation,
    ResolverDeclaration, ResolverDeclarationKind, ResolverDeclarationOrigin, ResolverObservation,
    ResolverObservationOutcome, ResolverTarget, RESOLVER_ARTIFACT_SCHEMA,
};
pub use resolver_coverage::{ResolverCoverageObservation, ResolverCoverageValidationError};
pub use resource::{
    resource_id, resource_occurrence_id, ResourceObservationKind, ResourceOrigin, ResourceRecord,
    ResourceValidationError, RESOURCE_RECORD_SCHEMA,
};
pub use resource_artifact::{
    ResourceArtifactRecord, ResourceArtifactRelation, ResourceInventoryArtifact,
    RESOURCE_INVENTORY_ARTIFACT_SCHEMA,
};
pub use scope::{
    CoverageMeasurement, CoverageObservation, ScopeCompleteness, ScopeUniverse,
    ScopeValidationError, SELECTOR_LANGUAGE_VERSION,
};
pub use snapshot::{SnapshotValidationError, SourceSnapshot, SourceState};
pub use source_subject::{
    CorrespondenceMode, CorrespondencePolicy, CorrespondenceReceipt, SourceSubject,
    SourceSubjectError, SubjectAlias,
};

/// Deduplicated callable implementation-content identity within a policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DefinitionRecord {
    pub structural_hash: String,
}

/// One definition site; location detail is in `logic_address`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OccurrenceRecord {
    pub address_id: String,
    pub logic_address: String,
    pub structural_hash: String,
    /// Exact observed unit bytes when emitted by a producer that computes ExactBodyId.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_body_id: Option<String>,
}

/// Parser-derived callable boundary attached to one occurrence.
///
/// Kept separate from [`OccurrenceRecord`] so identity rows remain compact and
/// fixed in purpose. This is navigation and invalidation material, not a
/// compiler-resolved type signature or authoritative source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallableBoundaryRecord {
    pub address_id: String,
    pub signature_hash: String,
    pub syntactic_signature: String,
}

/// Per-file index emission for Phase 2 indexer input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileIndexRecord {
    pub policy_id: PolicyId,
    pub file_hash: String,
    pub module_shell_hash: String,
    pub definitions: Vec<DefinitionRecord>,
    pub occurrences: Vec<OccurrenceRecord>,
    pub callable_boundaries: Vec<CallableBoundaryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_identities: Vec<ResolvedIdentityRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markers: Option<FileMarkers>,
}
pub mod revalidation;
pub use revalidation::*;
pub mod analysis_input;
pub mod analysis_plan;
pub mod analysis_work;
pub use analysis_input::{AnalysisInputRoot, RevalidationDeclaration};
pub use analysis_plan::{
    AnalysisAction, AnalysisDecision, AnalysisDecisionReason, AnalysisDemand, AnalysisGate,
    AnalysisGateClaim, AnalysisGateKind, AnalysisPassMode,
};
pub use analysis_work::{AnalysisWorkItem, AnalysisWorkState};

mod code_similarity;
pub use code_similarity::{
    CloneExtent, CloneFingerprints, CloneGroup, CloneLevel, CloneOccurrence, CloneReport,
    CloneRequest,
};

/// Package-local source and release dependency commitment.
#[doc(hidden)]
pub const PACKAGE_FINGERPRINT: &str = env!("ENTMOOT_PACKAGE_FINGERPRINT");
