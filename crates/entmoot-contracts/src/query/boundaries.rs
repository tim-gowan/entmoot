//! Source-addressed call boundaries and independently scoped completeness.
use super::SchemaJson;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Filter inventoried call sites and their attributed native evaluations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallBoundaryRequest {
    /// Explicit shared path semantics. Mutually exclusive with legacy path prefix.
    #[serde(default)]
    pub path_selector: Option<super::PathSelector>,
    /// Syntactic callee pattern. Only `*` is a wildcard; an optional trailing `()` is ignored.
    #[serde(default)]
    pub callee: Option<String>,
    /// Repository-relative path prefix.
    #[serde(default)]
    pub path: Option<String>,
    /// Native outcome token, or `unreported` for sites without native evidence.
    #[serde(default)]
    pub outcome: Option<String>,
    /// Exact normalized dispatch reason or unresolved-attempt reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// Select one native build context; omission keeps observations attributed separately.
    #[serde(default)]
    pub build_context: Option<crate::ContentDigest>,
    /// Offset in stable node-ID ordering within this publication.
    #[serde(default)]
    pub offset: usize,
    /// Requested page size, default 50, maximum 1000.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Detailed evidence for one distinct inventoried call site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallBoundaryDetail {
    /// Exact source spelling and syntax spans; receiver syntax is not a type claim.
    pub syntax: SchemaJson<crate::CallSiteMarker>,
    /// Snapshot source file commitment for verified source reads.
    pub content_digest: Option<crate::ContentDigest>,
    /// Independent native observations with artifact and evidence pointers.
    pub evaluations: Vec<SchemaJson<crate::CallEvaluation>>,
    /// Original unresolved attempts, preserving producer attribution.
    pub unresolved: Vec<SchemaJson<crate::UnresolvedRelation>>,
    /// Observation gaps, including optional unretained artifact bodies.
    pub limitations: Vec<String>,
}

/// Count of distinct source call sites in a callee/outcome partition.
/// A site may appear in several partitions when providers disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallBoundaryCount {
    /// Exact inventoried callee spelling.
    pub callee: String,
    /// Normalized native outcome or `unreported`.
    pub outcome: String,
    /// Distinct inventoried sites in the filtered collection, before paging.
    pub sites: usize,
}

/// An explicit completeness statement for one target universe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallGraphScope {
    /// `not_established` until an attributed, scope-matching closure claim exists.
    pub status: String,
    /// Why observed edges do not establish this universe's completeness.
    pub reason: String,
}

/// Repository closure is independent of dependency and runtime-library closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallGraphCompleteness {
    /// Calls among this publication's inventoried repository occurrences.
    pub repository: CallGraphScope,
    /// Calls within or returning from dependency source publications.
    pub dependencies: CallGraphScope,
    /// Calls within native/standard libraries and their possible callbacks.
    pub native_libraries: CallGraphScope,
}
impl Default for CallGraphCompleteness {
    fn default() -> Self {
        let statement = |reason: &str| CallGraphScope {
            status: "not_established".into(),
            reason: reason.into(),
        };
        Self {
            repository: statement("repository_dispatch_and_implicit_calls_not_exhaustively_qualified; external_targets_alone_do_not_disprove_local_edges"),
            dependencies: statement("dependency_graphs_and_external_reentry_not_exhaustively_qualified"),
            native_libraries: statement("native_library_graphs_intrinsics_and_callbacks_not_exhaustively_qualified"),
        }
    }
}
