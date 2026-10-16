//! Finding observations, stable entities, lifecycle, traces, locations.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::assertion::EvidenceRef;
use crate::location::LocationExtent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaxonomyRef {
    pub taxonomy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taxon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppressionDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

/// Immutable artifact-scoped finding observation (provider fact).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingObservation {
    pub observation_id: String,
    pub artifact_id: String,
    pub run_index: u32,
    pub result_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fingerprints: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taxonomies: Vec<TaxonomyRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_location_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_location_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logical_location_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressions: Vec<SuppressionDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_state: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix_pointers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_metadata_pointer: Option<String>,
}

/// Stable finding candidate across scans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingEntity {
    pub finding_entity_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    pub fingerprint_key: String,
    pub locator_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEventKind {
    Observed,
    AbsentInScan,
    Deduplicated,
    Suppressed,
    Disproven,
    Validated,
    Assigned,
    Acknowledged,
    RemediationProposed,
    Merged,
    Deployed,
    IndependentlyVerified,
    Reopened,
    Retracted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEvent {
    pub event_id: String,
    pub finding_entity_id: String,
    pub event_kind: LifecycleEventKind,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceRecord {
    pub trace_id: String,
    pub finding_observation_id: String,
    pub code_flow_index: u32,
    pub thread_index: u32,
    pub step_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceStepRecord {
    pub step_id: String,
    pub trace_id: String,
    pub ordinal: u32,
    pub location_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_importance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationRecord {
    pub location_id: String,
    pub artifact_id: String,
    pub path: String,
    pub extent: LocationExtent,
    pub provider_pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_confidence: Option<String>,
}
