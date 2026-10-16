//! Typed revision-impact manifest consumed by CI and maintainer workflows.

use crate::{
    ArtifactFormat, CorrespondencePolicy, CorrespondenceReceipt, ImplementationFingerprint,
    OccurrenceRef, ResolvedIdentityRecord, SourceSnapshot,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stable wire discriminator for revision-impact manifests.
pub const REVISION_IMPACT_SCHEMA: &str = "entmoot.revision_impact.v1";

/// Change classification for one snapshot file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileImpactKind {
    /// The path exists only in the head snapshot.
    Added,
    /// The path exists only in the base snapshot.
    Removed,
    /// Both snapshots contain the path with different bytes.
    Modified,
}

/// One changed repository-relative file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileImpact {
    /// Repository-relative POSIX path.
    pub path: String,
    /// Nature of the change.
    pub kind: FileImpactKind,
    /// Base content digest when the path existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_content_digest: Option<String>,
    /// Head content digest when the path exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_content_digest: Option<String>,
}

/// Change classification for one stable callable occurrence address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallableImpactKind {
    /// The occurrence exists only in the head inventory.
    Introduced,
    /// The occurrence exists only in the base inventory.
    Removed,
    /// Normalized implementation content changed at the same address.
    BodyChanged,
    /// The syntactic callable boundary changed while its body identity survived.
    BoundaryChanged,
    /// Both normalized implementation content and the callable boundary changed.
    BodyAndBoundaryChanged,
}

/// Changed callable identity or boundary at a stable occurrence address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallableImpact {
    /// Stable occurrence envelope shared across revisions when available.
    pub occurrence: OccurrenceRef,
    /// Repository-relative path.
    pub path: String,
    /// Nature of the callable change.
    pub kind: CallableImpactKind,
    /// Base topology node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_node_id: Option<String>,
    /// Head topology node id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_node_id: Option<String>,
    /// Base normalized body digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_structural_hash: Option<String>,
    /// Head normalized body digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_structural_hash: Option<String>,
    /// Base callable-boundary digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_signature_hash: Option<String>,
    /// Head callable-boundary digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_signature_hash: Option<String>,
}

/// Why a producer series needs to run against the head snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RescanRequirement {
    /// Stable producer/config/scope series identity.
    pub artifact_series_id: String,
    /// Producer name retained for operator selection.
    pub producer_name: String,
    /// Native artifact format expected from the producer.
    pub format: ArtifactFormat,
    /// Changed paths intersecting the producer's declared scope.
    pub affected_paths: Vec<String>,
    /// Whether a complete-relative-to-universe head artifact from the same series is attached.
    pub satisfied_by_head_artifact: bool,
    /// Head artifact id satisfying the requirement, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_artifact_id: Option<String>,
}

/// Disposition of a base-snapshot evidence record in the head snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceImpactKind {
    /// The identical record is present in both immutable snapshots.
    Retained,
    /// A matching semantic proposition is present but snapshot-scoped support changed.
    PropositionRetainedClaimStale,
    /// The record is not established for the head snapshot.
    Stale,
}

/// Evidence record affected by the revision boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceImpact {
    /// Typed collection name, including findings, attachments, assertions, and relations.
    pub record_kind: String,
    /// Stable record id in the base snapshot.
    pub base_record_id: String,
    /// Deterministic disposition; never an inferred security conclusion.
    pub disposition: EvidenceImpactKind,
    /// Stable proposition retained in the head, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposition_id: Option<String>,
}

/// Comparison of attributable binding observations, never a reuse authorization.
///
/// Coarse outcome family. Discrete `BindingImpactReason` values explain why a
/// row is not comparable when `kind` is `NotComparable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BindingImpactKind {
    /// Complete observations from matching producer series/policies differ.
    Changed,
    /// Complete observations from matching producer series/policies agree.
    Unchanged,
    /// Missing, partial, unavailable, or incompatible evidence prevents comparison.
    NotComparable,
}

/// Why a binding comparison could not (or did not need to) equate observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BindingImpactReason {
    /// One side lacks attributable resolver identity evidence.
    MissingEvidence,
    /// Evidence is present but incomplete for a strict comparison.
    IncompleteEvidence,
    /// Structural hashing policy set changed across the revision pair.
    StructuralPolicySetChanged,
    /// Producer series or resolved-identity policy is not comparable across sides.
    ProducerSeriesOrPolicyNotComparable,
}

/// All producer observations for one occurrence across the transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingImpact {
    /// Stable callable occurrence identity.
    pub address_id: String,
    /// Whether the represented complete bindings changed or cannot be compared.
    pub kind: BindingImpactKind,
    /// Exact source records, including artifact attribution and completeness.
    pub base: Vec<ResolvedIdentityRecord>,
    /// Exact head records with independent producer attribution.
    pub head: Vec<ResolvedIdentityRecord>,
    /// Discrete reasons preventing comparison. Empty for comparable observations.
    pub reasons: Vec<BindingImpactReason>,
}

/// A head evidence record absent from the base; not a new security conclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntroducedEvidence {
    /// Evidence collection containing the referenced record.
    pub record_kind: String,
    /// Immutable record identity in the head snapshot.
    pub head_record_id: String,
}

/// Inventory observation that merits consumer-contract review, never a break verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BreakingChangeSignalKind {
    /// Both boundary digests exist and differ under the same structural policy.
    BoundaryChanged,
    /// An inventoried callable occurrence disappeared from an assessed source scope.
    CallableRemoved,
}

/// A derived signal retaining its exact callable delta and containing snapshot scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BreakingChangeSignal {
    /// Why consumer review may be useful.
    pub kind: BreakingChangeSignalKind,
    /// Readable interpretation; no exposure, compatibility or exploitability verdict.
    pub summary: String,
    /// Exact evidence in the containing manifest's callable partition.
    pub callable: CallableImpact,
}

/// Orthogonal dimensions which never establish a breaking-change signal by themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactNonSignals {
    /// Callable implementation changes with an unchanged boundary.
    pub body_only_changes: usize,
    /// Changed binding observations; see the binding partition for their evidence.
    pub binding_changes: usize,
    /// Missing, partial or incompatible binding observations remain unknown.
    pub bindings_not_comparable: usize,
    /// Base evidence not retained verbatim; staleness does not mean a broken API.
    pub stale_evidence_records: usize,
    /// Number of unresolved attempts in the base inventory.
    pub base_unresolved_attempts: usize,
    /// Number of unresolved attempts in the head inventory.
    pub head_unresolved_attempts: usize,
}

/// Conservative projection of inventory boundary observations for operator review.
/// Empty signals mean none established within the represented comparison, never
/// compatibility, `SemVer` safety, runtime completeness or a vulnerability fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BreakingChangeSignals {
    /// Versioned derivation committing the inclusion and suppression rules.
    pub derivation_policy: String,
    /// Signals sorted by occurrence address; scope is the containing base/head snapshot.
    pub signals: Vec<BreakingChangeSignal>,
    /// Explicit evidence dimensions excluded from signal derivation.
    pub non_signals: ImpactNonSignals,
    /// Exposure, inventory and compatibility limitations.
    pub limitations: Vec<String>,
}

/// Deterministic comparison of two validated immutable graph snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationImpactKind {
    /// Explicit initializer syntax changed; no runtime or compatibility verdict.
    InitializerChanged,
    /// Observation exists only in base; missing head evidence is not deletion proof.
    ObservedOnlyInBase,
    /// Observation exists only in head; does not prove the binding was newly introduced.
    ObservedOnlyInHead,
}

/// Named declaration syntax delta retaining both exact source observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarationImpact {
    /// Occurrence identity; this is not a callable structural hash.
    pub address_id: String,
    /// Repository-relative source path.
    pub path: String,
    /// Conservative evidence comparison outcome.
    pub kind: DeclarationImpactKind,
    /// Exact base observation scoped by the containing manifest snapshot.
    pub base: Option<crate::DeclarationMarker>,
    /// Exact head observation scoped by the containing manifest snapshot.
    pub head: Option<crate::DeclarationMarker>,
}

/// Deterministic comparison of two validated immutable graph snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionImpactManifest {
    /// Structured declared boundary changes; never automatic breaking verdicts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<CodeContractImpact>,
    /// Wire schema discriminator.
    pub schema_version: String,
    /// Exact contract and derivation implementation; the v1 label alone is insufficient.
    pub implementation: ImplementationFingerprint,
    /// Base source snapshot.
    pub base_snapshot: SourceSnapshot,
    /// Head source snapshot.
    pub head_snapshot: SourceSnapshot,
    /// Requested correspondence policy when supplied by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correspondence: Option<CorrespondencePolicy>,
    /// How subject/occurrence joins were applied for this compare.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correspondence_receipt: Option<CorrespondenceReceipt>,
    /// Structural policies present in the base graph.
    pub base_policy_ids: Vec<String>,
    /// Structural policies present in the head graph.
    pub head_policy_ids: Vec<String>,
    /// Changed paths, sorted by path.
    pub files: Vec<FileImpact>,
    /// Changed callable occurrences, sorted by address.
    pub callables: Vec<CallableImpact>,
    /// Bounded declaration initializer deltas, independent of callable changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declarations: Vec<DeclarationImpact>,
    /// Derived consumer-review signals, separate from body/binding/evidence dimensions.
    pub breaking_change_signals: BreakingChangeSignals,
    /// Comparisons for every occurrence with represented Resolver evidence.
    /// An absent occurrence has no represented binding evidence.
    pub bindings: Vec<BindingImpact>,
    /// Base evidence dispositions, sorted by kind and id.
    pub evidence: Vec<EvidenceImpact>,
    /// Head evidence newly in scope, sorted by kind and id.
    pub introduced_evidence: Vec<IntroducedEvidence>,
    /// Producer series affected by changed paths, sorted by series id.
    pub rescans: Vec<RescanRequirement>,
    /// Explicit limitations that prevent a stronger conclusion.
    pub gaps: Vec<String>,
}

/// Syntactic boundary disposition between two inventoried occurrences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodeContractImpactKind {
    /// The occurrence is represented only in the head inventory.
    Introduced,
    /// The occurrence is represented only in the base inventory.
    Removed,
    /// Both represented declarations have different shapes.
    Changed,
    /// An occurrence exists but its boundary observation is unavailable.
    NotComparable,
}

/// Two declared shapes at a production occurrence, with existing direct callers
/// as navigation hints. Transitive impact still uses bounded relationship reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeContractImpact {
    /// The real occurrence shared by the comparison.
    pub occurrence: OccurrenceRef,
    /// Repository-relative source path.
    pub path: String,
    /// Comparison outcome, without a compatibility verdict.
    pub kind: CodeContractImpactKind,
    /// Observed base shape, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<crate::CodeContract>,
    /// Observed head shape, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<crate::CodeContract>,
    /// Head inventory callers already connected by represented call edges.
    pub related_callers: Vec<String>,
}

/// Bounded caller relay for the configured immutable base/head publication pair.
/// Head occurrences are walked; removed callables retain an explicit gap. Each
/// caller page resumes through its embedded neighborhood query and cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ImpactRelayRequest {
    /// Search controls per changed callable, independent of delivery bytes.
    pub budget: super::BoundedWalkBudget,
    /// All represented evidence includes candidates; resolved-only is the default.
    pub evidence_filter: super::HopEvidenceFilter,
    /// Compact changed-callable pages (at most 16 subjects per request).
    pub output: super::OutputOptions,
    /// Compact caller pages per subject; resume through the returned query.
    /// `after` must be absent because each subject has its own cursor.
    pub callers_output: super::OutputOptions,
}

impl Default for ImpactRelayRequest {
    fn default() -> Self {
        Self {
            budget: super::BoundedWalkBudget::default(),
            evidence_filter: super::HopEvidenceFilter::ResolvedOnly,
            output: super::OutputOptions::default(),
            callers_output: super::OutputOptions::default(),
        }
    }
}

/// One changed callable with its bounded head caller neighborhood.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactRelayItem {
    /// Exact attributable change, including base and head identities.
    pub callable: CallableImpact,
    /// Compact head callers; absence is explained by `gaps`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callers: Option<super::CompactQueryResponse>,
    /// Subject-specific limitations; empty callers never establish absence.
    pub gaps: Vec<String>,
}

/// Compact impact-to-callers composition; no producer or mutation effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactRelayResult {
    /// Wire discriminator (`entmoot.impact_relay.v1`).
    pub schema_version: String,
    /// Same implementation as the underlying revision comparison.
    pub implementation: ImplementationFingerprint,
    /// Immutable base source identity.
    pub base_snapshot: SourceSnapshot,
    /// Immutable head source identity used for caller walks.
    pub head_snapshot: SourceSnapshot,
    /// Semantic request with subject delivery controls reset; page size/bytes may
    /// change on continuation. Caller delivery controls are retained.
    pub request: ImpactRelayRequest,
    /// Changed-callable delivery page; separate from per-subject caller pages.
    pub page: super::OutputPage,
    /// Changes sorted by occurrence address, each carrying its own caller cursor.
    pub items: Vec<ImpactRelayItem>,
    /// Impact comparison and caller-coverage limitations.
    pub gaps: Vec<String>,
    /// Caller evidence is available through ordinary inspect/explain/export.
    pub details_deferred: bool,
}
