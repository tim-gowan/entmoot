//! Derived JSON Schema emission for interchange contracts.
//!
//! Normative invariants remain in docs/architecture/ARCHITECTURE.md and Rust tests — these schemas
//! check wire structure only.

use schemars::{schema_for, JsonSchema};
use serde_json::{json, Map, Value};

use crate::{
    Address, AddressId, AdvisoryAffectedPackage, AdvisoryRange, AdvisoryRangeEvent,
    AffectedVersion, AnalysisEnvironmentFingerprint, ArtifactFormat, ArtifactId, ArtifactManifest,
    ArtifactProducer, Assertion, AssertionObject, AttachmentDisposition, AttachmentFreshness,
    AttachmentId, AttachmentReason, AttachmentTarget, AttachmentTargetKind, BomComposition,
    BomReferenceOutcome, CallableBoundaryRecord, CapabilityObservation, CapabilityOutcome,
    ClaimScope, ColumnEncoding, CompletenessAssertion, ComponentAffect, ComponentDependency,
    ComponentDependencyKind, ComponentOccurrence, ComponentVulnerability, ContentDigest,
    CoverageObservation, DefinitionRecord, DerivationRecord, EntityKind, EntityRef,
    EpistemicStatus, EvidenceCapability, EvidenceRef, ExactBodyId, FileIndexRecord, FindingEntity,
    FindingId, FindingObservation, InTotoStyleStatement, InvocationMode,
    KnowledgePublicationManifest, KnowledgePublicationMode, KnowledgeSegmentDescriptor,
    KnowledgeVisibility, KnowledgeVisibilityEvent, KnowledgeVisibilityPage,
    KnowledgeVisibilityReason, LifecycleEvent, LifecycleEventKind, LocationAttachment,
    LocationExtent, LocationRecord, LocationRef, LocationResolveStatus, Locator, LogicId,
    ObservationContext, OccurrenceRecord, PolicyId, PredicateMaturity, PredicateRef,
    ProducerCapabilities, ProducerCapabilityKind, ProducerIdentity, ProducerRef,
    PropertyDependencySpec, PropertyStoreKey, Proposition, PropositionId, Provenance,
    ProviderCallableRef, ProviderSymbolSpelling, PublicationEmitter, PublicationId, RelationClaim,
    RelationClaimId, RelationClaimState, RelationClaimValue, RelationEdge, RelationEdgeId,
    RelationKind, Resolution, ResolutionAttempt, ResolutionAttemptStatus, ResolutionState,
    ResolvedHash, ResolvedIdentityCompleteness, ResolvedIdentityRecord, ResolverArtifact,
    ResolverArtifactProducer, ResolverCallSource, ResolverCoverageObservation, ResolverObservation,
    ResolverObservationOutcome, ResolverTarget, ResourceInventoryArtifact, ResourceRecord,
    Scope as RelationScope, ScopeCompleteness, ScopeUniverse, SourceLocation, SourcePosition,
    SourceRegion, SourceSnapshot, SourceState, SourceTextEnd, SourceTextRange, StructuralHash,
    Subject, SubjectId, SuppressionDetail, TargetConstraint, TaxonomyRef, TraceRecord,
    TraceStepRecord, UnresolvedReason, UnresolvedRelation, UnresolvedRelationId, Validity,
    VerifiedSourceSpan, VexAnalysis, VulnerabilityRating,
};

pub const CORE_CONTRACT_CATALOG_ID: &str =
    "https://entmoot.dev/schemas/contracts/entmoot-contracts.v1.json";

fn def<T: JsonSchema>(defs: &mut Map<String, Value>, name: &str) -> Result<(), serde_json::Error> {
    let mut schema = serde_json::to_value(schema_for!(T))?;
    schema["$id"] = json!(format!("{CORE_CONTRACT_CATALOG_ID}?type={name}"));
    defs.insert(name.to_owned(), schema);
    Ok(())
}

/// Pretty-printed JSON Schema document covering Entmoot interchange contracts.
pub fn emit_contract_schemas_json() -> Result<String, serde_json::Error> {
    let mut defs = Map::new();
    def::<crate::WorkspacePublication>(&mut defs, "WorkspacePublication")?;
    def::<crate::GitTreeCapture>(&mut defs, "GitTreeCapture")?;
    def::<crate::IntegrationRequest>(&mut defs, "IntegrationRequest")?;
    def::<crate::IntegrationStepReport>(&mut defs, "IntegrationStepReport")?;
    def::<crate::IntegrationResult>(&mut defs, "IntegrationResult")?;
    def::<crate::RevalidationRequest>(&mut defs, "RevalidationRequest")?;
    def::<crate::BindingFamily>(&mut defs, "BindingFamily")?;
    def::<crate::BindingPlan>(&mut defs, "BindingPlan")?;
    def::<crate::BindingOutcome>(&mut defs, "BindingOutcome")?;
    def::<crate::RevalidationResult>(&mut defs, "RevalidationResult")?;
    def::<crate::RevalidationRecordState>(&mut defs, "RevalidationRecordState")?;
    def::<crate::OperationSpec>(&mut defs, "OperationSpec")?;
    def::<crate::OperationOutputPage>(&mut defs, "OperationOutputPage")?;
    def::<crate::OperationRecord>(&mut defs, "OperationRecord")?;
    def::<crate::SchedulerStatus>(&mut defs, "SchedulerStatus")?;
    def::<crate::ProducerRequest>(&mut defs, "ProducerRequest")?;
    def::<crate::ProducerSource>(&mut defs, "ProducerSource")?;
    def::<crate::BranchConflictReport>(&mut defs, "BranchConflictReport")?;
    def::<crate::CollaborationRead>(&mut defs, "CollaborationRead")?;
    def::<crate::CollaborationWrite>(&mut defs, "CollaborationWrite")?;
    def::<crate::CanonGraph>(&mut defs, "CanonGraph")?;
    def::<crate::CanonFile>(&mut defs, "CanonFile")?;
    def::<crate::CanonExpressionFact>(&mut defs, "CanonExpressionFact")?;
    def::<crate::AssessmentEvidence>(&mut defs, "AssessmentEvidence")?;
    def::<crate::TestValidationRequest>(&mut defs, "TestValidationRequest")?;
    def::<crate::ImplementationFingerprint>(&mut defs, "ImplementationFingerprint")?;
    def::<crate::RevisionCampaignInputs>(&mut defs, "RevisionCampaignInputs")?;
    def::<crate::NativeRawEnvelope>(&mut defs, "NativeRawEnvelope")?;
    def::<crate::RustBuildProfile>(&mut defs, "RustBuildProfile")?;
    def::<crate::NativeReceipt>(&mut defs, "NativeReceipt")?;
    def::<crate::NativeProducerContext>(&mut defs, "NativeProducerContext")?;
    def::<crate::NativeRawObservation>(&mut defs, "NativeRawObservation")?;
    def::<crate::NativeSemanticStatus>(&mut defs, "NativeSemanticStatus")?;
    def::<crate::AdapterProfile>(&mut defs, "AdapterProfile")?;
    def::<crate::AdapterProfileFamily>(&mut defs, "AdapterProfileFamily")?;
    def::<crate::EntmootConfig>(&mut defs, "EntmootConfig")?;

    def::<crate::StorageFingerprint>(&mut defs, "StorageFingerprint")?;
    def::<crate::OccurrenceRef>(&mut defs, "OccurrenceRef")?;
    def::<crate::SourceOccurrenceRef>(&mut defs, "SourceOccurrenceRef")?;
    def::<crate::StructuralHashPolicy>(&mut defs, "StructuralHashPolicy")?;
    def::<crate::StructuralPolicySeries>(&mut defs, "StructuralPolicySeries")?;
    def::<crate::NormalizationAudit>(&mut defs, "NormalizationAudit")?;
    def::<crate::NormalizationValidationReport>(&mut defs, "NormalizationValidationReport")?;
    def::<crate::FileMarkers>(&mut defs, "FileMarkers")?;
    def::<crate::CodeContract>(&mut defs, "CodeContract")?;
    def::<crate::CodeContractRecord>(&mut defs, "CodeContractRecord")?;
    def::<crate::CallSitePorts>(&mut defs, "CallSitePorts")?;
    def::<crate::CallSiteRecord>(&mut defs, "CallSiteRecord")?;
    def::<crate::DeclarationSiteRecord>(&mut defs, "DeclarationSiteRecord")?;
    def::<crate::ResolverAccounting>(&mut defs, "ResolverAccounting")?;
    def::<crate::ReachabilityAudit>(&mut defs, "ReachabilityAudit")?;
    def::<crate::CloneRequest>(&mut defs, "CloneRequest")?;
    def::<crate::CloneReport>(&mut defs, "CloneReport")?;
    def::<crate::UnusedRequest>(&mut defs, "UnusedRequest")?;
    def::<crate::UnusedReport>(&mut defs, "UnusedReport")?;
    def::<crate::TestImpactRequest>(&mut defs, "TestImpactRequest")?;
    def::<crate::TestImpactReport>(&mut defs, "TestImpactReport")?;
    def::<crate::AnalysisDemand>(&mut defs, "AnalysisDemand")?;
    def::<crate::AnalysisDecision>(&mut defs, "AnalysisDecision")?;
    def::<crate::AnalysisWorkItem>(&mut defs, "AnalysisWorkItem")?;
    def::<crate::AnalysisInputRoot>(&mut defs, "AnalysisInputRoot")?;
    def::<crate::RevalidationDeclaration>(&mut defs, "RevalidationDeclaration")?;
    def::<Address>(&mut defs, "Address")?;
    def::<AddressId>(&mut defs, "AddressId")?;
    def::<AnalysisEnvironmentFingerprint>(&mut defs, "AnalysisEnvironmentFingerprint")?;
    def::<ArtifactFormat>(&mut defs, "ArtifactFormat")?;
    def::<ArtifactId>(&mut defs, "ArtifactId")?;
    def::<ArtifactManifest>(&mut defs, "ArtifactManifest")?;
    def::<crate::ArtifactRef>(&mut defs, "ArtifactRef")?;
    def::<crate::ArtifactAttachReceipt>(&mut defs, "ArtifactAttachReceipt")?;
    def::<ArtifactProducer>(&mut defs, "ArtifactProducer")?;
    def::<Assertion>(&mut defs, "Assertion")?;
    def::<AssertionObject>(&mut defs, "AssertionObject")?;
    def::<AttachmentDisposition>(&mut defs, "AttachmentDisposition")?;
    def::<AttachmentFreshness>(&mut defs, "AttachmentFreshness")?;
    def::<AttachmentId>(&mut defs, "AttachmentId")?;
    def::<AttachmentReason>(&mut defs, "AttachmentReason")?;
    def::<AttachmentTarget>(&mut defs, "AttachmentTarget")?;
    def::<AttachmentTargetKind>(&mut defs, "AttachmentTargetKind")?;
    def::<StructuralHash>(&mut defs, "StructuralHash")?;
    def::<ResolvedHash>(&mut defs, "ResolvedHash")?;
    def::<ResolvedIdentityCompleteness>(&mut defs, "ResolvedIdentityCompleteness")?;
    def::<ResolvedIdentityRecord>(&mut defs, "ResolvedIdentityRecord")?;
    def::<crate::ResolverEvidenceRef>(&mut defs, "ResolverEvidenceRef")?;
    def::<crate::ResolvedBindingEvidence>(&mut defs, "ResolvedBindingEvidence")?;
    def::<CallableBoundaryRecord>(&mut defs, "CallableBoundaryRecord")?;
    def::<CapabilityObservation>(&mut defs, "CapabilityObservation")?;
    def::<CapabilityOutcome>(&mut defs, "CapabilityOutcome")?;
    def::<ComponentOccurrence>(&mut defs, "ComponentOccurrence")?;
    def::<ComponentDependency>(&mut defs, "ComponentDependency")?;
    def::<ComponentDependencyKind>(&mut defs, "ComponentDependencyKind")?;
    def::<BomReferenceOutcome>(&mut defs, "BomReferenceOutcome")?;
    def::<ComponentVulnerability>(&mut defs, "ComponentVulnerability")?;
    def::<ComponentAffect>(&mut defs, "ComponentAffect")?;
    def::<AffectedVersion>(&mut defs, "AffectedVersion")?;
    def::<AdvisoryAffectedPackage>(&mut defs, "AdvisoryAffectedPackage")?;
    def::<AdvisoryRange>(&mut defs, "AdvisoryRange")?;
    def::<AdvisoryRangeEvent>(&mut defs, "AdvisoryRangeEvent")?;
    def::<VulnerabilityRating>(&mut defs, "VulnerabilityRating")?;
    def::<VexAnalysis>(&mut defs, "VexAnalysis")?;
    def::<BomComposition>(&mut defs, "BomComposition")?;
    def::<RelationClaim>(&mut defs, "RelationClaim")?;
    def::<RelationClaimId>(&mut defs, "RelationClaimId")?;
    def::<RelationClaimState>(&mut defs, "RelationClaimState")?;
    def::<RelationClaimValue>(&mut defs, "RelationClaimValue")?;
    def::<RelationEdge>(&mut defs, "RelationEdge")?;
    def::<RelationEdgeId>(&mut defs, "RelationEdgeId")?;
    def::<Proposition>(&mut defs, "Proposition")?;
    def::<PropositionId>(&mut defs, "PropositionId")?;
    def::<ProducerCapabilityKind>(&mut defs, "ProducerCapabilityKind")?;
    def::<ProducerRef>(&mut defs, "ProducerRef")?;
    def::<Provenance>(&mut defs, "Provenance")?;
    def::<RelationKind>(&mut defs, "RelationKind")?;
    def::<RelationScope>(&mut defs, "RelationScope")?;
    def::<ResolutionState>(&mut defs, "ResolutionState")?;
    def::<ResourceRecord>(&mut defs, "ResourceRecord")?;
    def::<ResourceInventoryArtifact>(&mut defs, "ResourceInventoryArtifact")?;
    def::<SubjectId>(&mut defs, "SubjectId")?;
    def::<TargetConstraint>(&mut defs, "TargetConstraint")?;
    def::<UnresolvedReason>(&mut defs, "UnresolvedReason")?;
    def::<UnresolvedRelation>(&mut defs, "UnresolvedRelation")?;
    def::<UnresolvedRelationId>(&mut defs, "UnresolvedRelationId")?;
    def::<ClaimScope>(&mut defs, "ClaimScope")?;
    def::<ColumnEncoding>(&mut defs, "ColumnEncoding")?;
    def::<CompletenessAssertion>(&mut defs, "CompletenessAssertion")?;
    def::<ContentDigest>(&mut defs, "ContentDigest")?;
    def::<CoverageObservation>(&mut defs, "CoverageObservation")?;
    def::<DefinitionRecord>(&mut defs, "DefinitionRecord")?;
    def::<DerivationRecord>(&mut defs, "DerivationRecord")?;
    def::<EntityKind>(&mut defs, "EntityKind")?;
    def::<EntityRef>(&mut defs, "EntityRef")?;
    def::<EpistemicStatus>(&mut defs, "EpistemicStatus")?;
    def::<EvidenceCapability>(&mut defs, "EvidenceCapability")?;
    def::<EvidenceRef>(&mut defs, "EvidenceRef")?;
    def::<ExactBodyId>(&mut defs, "ExactBodyId")?;
    def::<FileIndexRecord>(&mut defs, "FileIndexRecord")?;
    def::<FindingEntity>(&mut defs, "FindingEntity")?;
    def::<FindingId>(&mut defs, "FindingId")?;
    def::<FindingObservation>(&mut defs, "FindingObservation")?;
    def::<KnowledgeVisibilityPage>(&mut defs, "KnowledgeVisibilityPage")?;
    def::<KnowledgePublicationManifest>(&mut defs, "KnowledgePublicationManifest")?;
    def::<KnowledgePublicationMode>(&mut defs, "KnowledgePublicationMode")?;
    def::<KnowledgeSegmentDescriptor>(&mut defs, "KnowledgeSegmentDescriptor")?;
    def::<KnowledgeVisibility>(&mut defs, "KnowledgeVisibility")?;
    def::<KnowledgeVisibilityEvent>(&mut defs, "KnowledgeVisibilityEvent")?;
    def::<KnowledgeVisibilityReason>(&mut defs, "KnowledgeVisibilityReason")?;
    def::<PublicationEmitter>(&mut defs, "PublicationEmitter")?;
    def::<InTotoStyleStatement>(&mut defs, "InTotoStyleStatement")?;
    def::<InvocationMode>(&mut defs, "InvocationMode")?;
    def::<LifecycleEvent>(&mut defs, "LifecycleEvent")?;
    def::<LifecycleEventKind>(&mut defs, "LifecycleEventKind")?;
    def::<LocationAttachment>(&mut defs, "LocationAttachment")?;
    def::<LocationExtent>(&mut defs, "LocationExtent")?;
    def::<LocationRecord>(&mut defs, "LocationRecord")?;
    def::<LocationRef>(&mut defs, "LocationRef")?;
    def::<LocationResolveStatus>(&mut defs, "LocationResolveStatus")?;
    def::<Locator>(&mut defs, "Locator")?;
    def::<LogicId>(&mut defs, "LogicId")?;
    def::<ObservationContext>(&mut defs, "ObservationContext")?;
    def::<OccurrenceRecord>(&mut defs, "OccurrenceRecord")?;
    def::<PolicyId>(&mut defs, "PolicyId")?;
    def::<PredicateMaturity>(&mut defs, "PredicateMaturity")?;
    def::<PredicateRef>(&mut defs, "PredicateRef")?;
    def::<ProducerCapabilities>(&mut defs, "ProducerCapabilities")?;
    def::<ProducerIdentity>(&mut defs, "ProducerIdentity")?;
    def::<PublicationId>(&mut defs, "PublicationId")?;
    def::<PropertyDependencySpec>(&mut defs, "PropertyDependencySpec")?;
    def::<PropertyStoreKey>(&mut defs, "PropertyStoreKey")?;
    def::<crate::PropertyInputManifest>(&mut defs, "PropertyInputManifest")?;
    def::<crate::CallDispatch>(&mut defs, "CallDispatch")?;
    def::<Resolution>(&mut defs, "Resolution")?;
    def::<ResolutionAttempt>(&mut defs, "ResolutionAttempt")?;
    def::<ResolutionAttemptStatus>(&mut defs, "ResolutionAttemptStatus")?;
    def::<ResolverCoverageObservation>(&mut defs, "ResolverCoverageObservation")?;
    def::<ProviderSymbolSpelling>(&mut defs, "ProviderSymbolSpelling")?;
    def::<ProviderCallableRef>(&mut defs, "ProviderCallableRef")?;
    def::<ResolverArtifact>(&mut defs, "ResolverArtifact")?;
    def::<ResolverArtifactProducer>(&mut defs, "ResolverArtifactProducer")?;
    def::<ResolverCallSource>(&mut defs, "ResolverCallSource")?;
    def::<ResolverObservation>(&mut defs, "ResolverObservation")?;
    def::<ResolverObservationOutcome>(&mut defs, "ResolverObservationOutcome")?;
    def::<ResolverTarget>(&mut defs, "ResolverTarget")?;
    def::<ScopeCompleteness>(&mut defs, "ScopeCompleteness")?;
    def::<ScopeUniverse>(&mut defs, "ScopeUniverse")?;
    def::<SourceLocation>(&mut defs, "SourceLocation")?;
    def::<SourceRegion>(&mut defs, "SourceRegion")?;
    def::<VerifiedSourceSpan>(&mut defs, "VerifiedSourceSpan")?;
    def::<SourcePosition>(&mut defs, "SourcePosition")?;
    def::<SourceTextEnd>(&mut defs, "SourceTextEnd")?;
    def::<SourceTextRange>(&mut defs, "SourceTextRange")?;
    def::<SourceSnapshot>(&mut defs, "SourceSnapshot")?;
    def::<SourceState>(&mut defs, "SourceState")?;
    def::<Subject>(&mut defs, "Subject")?;
    def::<SuppressionDetail>(&mut defs, "SuppressionDetail")?;
    def::<TaxonomyRef>(&mut defs, "TaxonomyRef")?;
    def::<TraceRecord>(&mut defs, "TraceRecord")?;
    def::<TraceStepRecord>(&mut defs, "TraceStepRecord")?;
    def::<Validity>(&mut defs, "Validity")?;

    def::<crate::TransitionRevalidationRequest>(&mut defs, "TransitionRevalidationRequest")?;
    def::<crate::TransitionRevalidationReceipt>(&mut defs, "TransitionRevalidationReceipt")?;

    let doc = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": CORE_CONTRACT_CATALOG_ID,
        "title": "entmoot-contracts",
        "description": "Derived from Rust types via schemars. Structure only — not normative Entmoot invariants.",
        "$defs": defs,
    });
    let value: Value = serde_json::from_str(&serde_json::to_string(&doc)?)?;
    let mut out = serde_json::to_string_pretty(&value)?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_schemas_match_golden() {
        let generated = emit_contract_schemas_json().expect("emit");
        let golden = include_str!("../generated/entmoot_contracts.schema.json");
        assert_eq!(
            generated, golden,
            "schema drift: run `cargo run -p entmoot-contracts --example emit_json_schema`"
        );
    }
}
