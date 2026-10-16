//! Read-only comparison of explicitly evidenced package boundaries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Address, EvidenceRef, SourceSnapshot};

/// Version of the portable compatibility receipt.
pub const PACKAGE_COMPATIBILITY_SCHEMA: &str = "entmoot.package_compatibility.v1";

/// Boundary family assessed by this operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    /// Direct package imports and callable contracts under an explicit policy.
    PackageApi,
}

/// Publication identifiers are content digests, never local paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityRequest {
    /// Immutable consumer publication selected by the composition root.
    pub consumer_publication: String,
    /// Independently loaded candidate provider publications.
    pub provider_publications: Vec<String>,
    /// Requested boundary family.
    pub boundary_kind: BoundaryKind,
}

/// Discrete evidence outcome, not a confidence or a safe-upgrade claim.
///
/// Coarse outcome family only. Site-level `CompatibilityReason` states what was
/// or was not checked (unassessed contract vs incomplete export scope vs digest
/// equality). Ecosystem-agnostic: the same variants apply to Rust, JS/TS, Go,
/// Python, Java, C#, and C/C++ `package_api` evidence.
///
/// Ladder (stronger → weaker positive): `fits` (assessed boundary equality) →
/// `bound` (unique package export stitch; everyday usable) → withheld /
/// ambiguous / broken. Missing lock identity never blocks `bound` or `fits`;
/// checker-green is not `fits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityVerdict {
    /// Assessed boundary digests match under the stated policy.
    Fits,
    /// Unique package export located (seeking/stitch); usable without assessed
    /// boundary equality or lock identity. Not type-assignability or compile-as-fit.
    Bound,
    /// At least one attributable site violates its observed boundary (or a
    /// mapped install root was asserted and missing).
    BrokenAt,
    /// Evidence exists but does not uniquely determine a boundary comparison.
    Unknown,
    /// Judgment withheld: required binding, export, or scope evidence is missing
    /// or partial. See `CompatibilityReason` for whether the site was unassessed,
    /// export scope incomplete, or provider evidence missing.
    IncompleteBind,
}

/// Why a site received its [`CompatibilityVerdict`].
///
/// Derived only by the language-agnostic reader from `package_api` importer
/// (`PackageApiReference`) and importee (`PackageApiExport` / surface) evidence.
/// Not `SemVer`, dispatch closure, or today’s fit-via-typecheck. Type-aware
/// producer claims are deferred to a cost/utility ADR—not banned forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityReason {
    /// Lock/install instance was not asserted. Retained as a strengthen-available
    /// signal / limitation; does not block `bound` or assessed `fits`.
    LockInstanceUnproven,
    /// Default before a unique provider export is selected for the site.
    ProviderExportEvidenceMissing,
    /// Consumer observation is not unique for this occurrence.
    AmbiguousConsumerBinding,
    /// More than one provider publication/surface matches the package.
    AmbiguousProviderPublications,
    /// Provider artifact or export enumeration is incomplete; absence cannot be proven.
    ProviderExportScopeIncomplete,
    /// Dependency site is known, but no stitch contract was recorded (`unassessed`
    /// policy or missing expected boundary)—break/fit cannot be checked.
    ConsumerBoundaryUnassessed,
    /// Unique provider export spelling was located, but the consumer still has no
    /// assessed expected boundary—seeking/stitch succeeded; everyday `bound`, not `fits`.
    ProviderExportLocatedContractUnassessed,
    /// Operator/producer asserted a dependency install/package-root mapping, but
    /// that root was missing for this package coordinate (loud contradiction).
    MappedPackageRootMissing,
    /// Target symbol absent under a producer-stated complete export surface.
    ExportAbsentInCompleteSurface,
    /// Consumer and provider boundary policy IDs differ.
    BoundaryPoliciesDiffer,
    /// Absence is outside the provider's stated symbol/policy universe.
    ExportUniverseUnassessed,
    /// Digests equal under the same policy; body content is outside the contract.
    ObservedBoundaryEqualBodyNotAssessed,
    /// Digests differ under the same policy.
    ObservedBoundaryChanged,
    /// Defensive branch: policy matched but expected boundary was still absent.
    ConsumerExpectedBoundaryMissing,
}

/// One independent publication included in the judgment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityPublication {
    /// Digest of the exact serialized graph bytes.
    pub publication: String,
    /// Repository, revision, source state, and working-tree digest.
    pub snapshot: SourceSnapshot,
}

/// Evidence-backed result at an exact consumer occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompatibilitySite {
    /// Original consumer installation selected by the native binding, when proven.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_instance: Option<crate::package_api::LockedPackageInstance>,
    /// Exact source occurrence in the consumer publication.
    pub location: Address,
    /// Native package-qualified target; never a display-name join.
    pub target: crate::package_api::PackageApiTarget,
    /// Outcome family for this occurrence.
    pub verdict: CompatibilityVerdict,
    /// Discrete explanation of the verdict; what was or was not checked.
    pub reason: CompatibilityReason,
    /// Provider publication when a unique candidate was selected.
    pub provider_publication: Option<String>,
    /// Versioned equality policy; no subtyping or language compatibility inference.
    pub boundary_policy: String,
    /// Boundary observed when the consumer was bound.
    pub expected_boundary: Option<String>,
    /// Boundary observed in the selected provider.
    pub observed_boundary: Option<String>,
    /// Native consumer binding and provider export evidence.
    pub evidence: Vec<EvidenceRef>,
}

/// Per-package summary; counts refer to evaluated sites only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityBoundary {
    /// Ecosystem-qualified package identity, without version-based judgments.
    pub package: crate::package_api::PackageApiPackage,
    /// Aggregate outcome for the package's evaluated sites.
    pub verdict: CompatibilityVerdict,
    /// Number of evaluated consumer occurrences.
    pub sites: usize,
}

/// Deterministic package-boundary receipt.
///
/// Empty evidence never produces `fits` or `bound`. Unique stitch without an
/// assessed contract produces `bound`; assessed digest equality produces `fits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityResult {
    /// Receipt schema discriminator.
    pub schema_version: String,
    /// Overall evidence outcome; a witnessed break dominates incomplete coverage.
    pub verdict: CompatibilityVerdict,
    /// Consumer publication and source provenance.
    pub consumer: CompatibilityPublication,
    /// Candidate provider publications, sorted by digest.
    pub providers: Vec<CompatibilityPublication>,
    /// Requested boundary kind.
    pub boundary_kind: BoundaryKind,
    /// Package summaries sorted by ecosystem and identity.
    pub boundaries: Vec<CompatibilityBoundary>,
    /// Evaluated sites sorted by target and source occurrence.
    pub sites: Vec<CompatibilitySite>,
    /// Explicit scope gaps and interpretation limits.
    pub limitations: Vec<crate::package_api::CompatibilityLimitation>,
}
