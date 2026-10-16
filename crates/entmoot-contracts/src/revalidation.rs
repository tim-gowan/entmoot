//! Conditional transition applicability and scheduling receipts.
//! Native evidence retains its original snapshot; retention is never a fresh observation.
use crate::{
    Address, AnalysisDecision, AnalysisGateClaim, AnalysisInputRoot, AnalysisPassMode, ArtifactId,
    RevalidationDeclaration, SourceSnapshot,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
#[error("invalid semantic record reference: {0}")]
pub struct KnowledgeRecordRefError(pub String);

/// Durable semantic reference used by the local ownership/dependency layer.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeRecordRef {
    pub collection: String,
    pub record_id: String,
}

impl KnowledgeRecordRef {
    pub fn new(collection: impl Into<String>, record_id: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            record_id: record_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), KnowledgeRecordRefError> {
        if self.collection.trim().is_empty() || self.record_id.trim().is_empty() {
            return Err(KnowledgeRecordRefError(format!(
                "{}/{}",
                self.collection, self.record_id
            )));
        }
        Ok(())
    }
}

/// Attributable owner-publication receipt. Unchanged inputs are conditional on the declared
/// method and assumptions; this is not a fresh scanner or execution observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInputReceipt {
    pub snapshot: SourceSnapshot,
    pub artifact_id: String,
    pub declaration: RevalidationDeclaration,
    pub input_root: AnalysisInputRoot,
    pub input_paths: Vec<String>,
    pub retained_evidence: Vec<KnowledgeRecordRef>,
    pub dependent_applicability: Vec<KnowledgeRecordRef>,
}

/// Exact visibility consequences of applying the same declared input policy to
/// a later immutable snapshot. Visibility retained through another owner remains
/// explicit and is not described as a fresh scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRevalidationReceipt {
    pub snapshot: SourceSnapshot,
    pub artifact_id: String,
    pub artifact_series_id: String,
    pub declaration: RevalidationDeclaration,
    pub prior_input_root: AnalysisInputRoot,
    pub current_input_root: AnalysisInputRoot,
    pub input_paths: Vec<String>,
    pub inputs_changed: bool,
    pub invalidated_applicability: Vec<KnowledgeRecordRef>,
    pub retained_applicability: Vec<KnowledgeRecordRef>,
    pub retained_provider_evidence: Vec<KnowledgeRecordRef>,
    pub limitations: Vec<String>,
}

/// One explicitly declared producer input policy; no policy is inferred from topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidationUnit {
    pub base_artifact_id: ArtifactId,
    /// Fresh evidence already attached to the exact head snapshot, if available.
    pub head_artifact_id: Option<ArtifactId>,
    pub declaration: RevalidationDeclaration,
    pub subject: Address,
    pub gates: Vec<AnalysisGateClaim>,
    pub mode: AnalysisPassMode,
}

/// Inputs for explicit publication into an isolated transition ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionRevalidationRequest {
    pub base_snapshot: SourceSnapshot,
    pub head_snapshot: SourceSnapshot,
    pub units: Vec<RevalidationUnit>,
    /// Total semantic records in the publication receipts. Exhaustion rejects
    /// the staged ledger; records are never truncated into apparent completeness.
    pub max_records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidationUnitReceipt {
    pub publication: ArtifactInputReceipt,
    pub revalidation: ArtifactRevalidationReceipt,
    pub reingested: Option<ArtifactInputReceipt>,
    pub decision: AnalysisDecision,
}

/// Durable conditional applicability report, separate from a graph comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionRevalidationReceipt {
    pub schema_version: String,
    pub base_snapshot: SourceSnapshot,
    pub head_snapshot: SourceSnapshot,
    pub units: Vec<RevalidationUnitReceipt>,
    /// Base artifacts without a supplied policy remain unknown, not retained.
    pub unselected_artifact_ids: Vec<String>,
    pub limitations: Vec<String>,
}
