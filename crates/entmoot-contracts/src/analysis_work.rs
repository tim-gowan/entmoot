//! Durable analysis-work projection. Work decisions are not graph evidence.
use crate::{AnalysisDecision, ArtifactId, ContentDigest, ProducerRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisWorkState {
    Queued,
    Claimed,
    ArtifactReady,
    Ingested,
    Failed,
    Cancelled,
}

/// Auditable scheduler state. `artifact_ready` means a manifest and its bytes
/// passed the ingestion trust transition; only `ingested` records publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisWorkItem {
    pub work_id: ContentDigest,
    pub decision: AnalysisDecision,
    pub state: AnalysisWorkState,
    pub attempt: u32,
    pub worker: Option<ProducerRef>,
    pub fencing_token: Option<ContentDigest>,
    pub artifact_id: Option<ArtifactId>,
    pub failure_reason: Option<String>,
    pub limitations: Vec<String>,
}
