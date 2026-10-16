//! Provider-neutral wire contract for immutable resource-observation artifacts.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Address, EntityKind, Locator, RelationKind, ResourceObservationKind};

pub const RESOURCE_INVENTORY_ARTIFACT_SCHEMA: &str = "logic.resource_inventory_artifact.v1";

/// Compact provider observation. The immutable artifact remains the lossless
/// source of truth; `provider_pointer` makes every projection auditable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceArtifactRecord {
    pub provider_pointer: String,
    pub entity_kind: EntityKind,
    pub logical_address: Address,
    pub display_name: String,
    pub native_type: String,
    pub observation_kind: ResourceObservationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locators: Vec<Locator>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub identity_labels: BTreeMap<String, String>,
}

/// Explicit resource relation stated by the artifact. Adapters do not infer
/// network connections or desired-to-observed correspondence from names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceArtifactRelation {
    pub provider_pointer: String,
    pub subject: Address,
    pub predicate: RelationKind,
    pub object: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceInventoryArtifact {
    pub schema_version: String,
    /// Stable deployment, cluster, account, or environment identity.
    pub environment_id: String,
    /// Stable observation boundary within that environment.
    pub observation_context_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<ResourceArtifactRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<ResourceArtifactRelation>,
}
