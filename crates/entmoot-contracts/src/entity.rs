//! Cross-plane entity references and locators.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::location::SourceLocation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Repository,
    File,
    Callable,
    Finding,
    SemanticSymbol,
    CloudAccount,
    KubernetesResource,
    InfrastructureResource,
    ConfigurationResource,
    Identity,
    NetworkEndpoint,
    Deployment,
    Owner,
    Team,
    Workload,
    Database,
    /// Durable addressable relation (ADR-0009).
    RelationEdge,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityRef {
    pub entity_id: String,
    pub entity_kind: EntityKind,
}

/// How to locate an entity in some plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Locator {
    CodeLocation {
        location: SourceLocation,
    },
    Uri {
        uri: String,
    },
    CloudArn {
        arn: String,
    },
    K8sObject {
        cluster: String,
        namespace: String,
        api_version: String,
        resource_kind: String,
        name: String,
    },
    Opaque {
        system: String,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationContext {
    pub observation_context_id: String,
    pub artifact_id: String,
    pub snapshot: crate::snapshot::SourceSnapshot,
    pub producer_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer_version: Option<String>,
}
