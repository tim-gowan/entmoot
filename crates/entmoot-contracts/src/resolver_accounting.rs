//! Inventory-denominated semantic analysis accounting. A disposition schedules
//! further evidence; it never proves that static resolution is impossible.
use crate::{ArtifactManifest, ContentDigest, ResolverObservation, SourceSnapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolverNextStep {
    StaticAnalysisRequired,
    RuntimeEvidenceRequired,
    RepositoryBoundaryRecorded,
    StaticTargetsClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallEvaluation {
    pub artifact_id: String,
    pub provider_pointer: String,
    pub observation: ResolverObservation,
    pub next_step: ResolverNextStep,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallSiteAccounting {
    pub node_id: String,
    pub owner_id: String,
    pub language: String,
    /// Empty means no native evidence, regardless of structural edges.
    pub evaluations: Vec<CallEvaluation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallableAccounting {
    pub node_id: String,
    pub language: String,
    /// Directly owned observed call sites. Empty means a syntactic leaf only;
    /// implicit calls, callbacks and entrypoint reachability remain unassessed.
    pub call_sites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverArtifactAccounting {
    pub manifest: ArtifactManifest,
    pub producer: crate::ResolverArtifactProducer,
    pub build_context_digest: ContentDigest,
    pub language: String,
    /// Inventoried sites within this producer's declared scope omitted by it.
    pub missing_call_sites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverAccounting {
    /// Independent source/build denominators; never union configurations to claim closure.
    pub coverage: Vec<ResolverScopeCoverage>,
    pub snapshot: SourceSnapshot,
    /// Commits the entire graph supplied to this audit, including policy and files.
    pub graph_digest: ContentDigest,
    pub method_id: String,
    pub ecosystems: Vec<String>,
    pub artifacts: Vec<ResolverArtifactAccounting>,
    pub callables: Vec<CallableAccounting>,
    pub call_sites: Vec<CallSiteAccounting>,
    /// Sites with no native evidence or any producer still requiring static work.
    pub static_work_remaining: usize,
    pub producer_omissions: usize,
    /// Source inventory diagnostics, including parse failures and exclusions.
    /// These forbid a whole-inventory gate from passing on a reduced denominator.
    pub inventory_gaps: Vec<String>,
    pub limitations: Vec<String>,
}

/// Explicit call-site coverage for one artifact's language and declared source scope.
/// Closing these sites does not account for implicit invocations or entrypoints.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverScopeCoverage {
    pub artifact_id: String,
    pub language: String,
    pub build_context_digest: ContentDigest,
    pub inventoried_sites: usize,
    pub observed_sites: usize,
    pub missing_sites: usize,
    pub closed_sites: usize,
    pub open_sites: usize,
    pub outcomes: std::collections::BTreeMap<String, usize>,
}

/// Bounded potential reachability from caller-supplied roots in one native context.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReachabilityAudit {
    pub build_context_digest: ContentDigest,
    pub entrypoints: Vec<String>,
    pub potentially_reachable: Vec<String>,
    /// Unreached in this represented graph, never a dead-code verdict.
    pub not_observed_reachable: Vec<String>,
    pub open_call_sites: Vec<String>,
    pub explicit_calls_closed: bool,
    pub limitations: Vec<String>,
}
