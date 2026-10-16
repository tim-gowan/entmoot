//! Provider-neutral configuration and runtime resource observations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    address_identity_key, hash_parts, is_immutable_revision, normalize_repo_path, Address,
    AddressValidationError, ArtifactId, ContentDigest, EntityKind, IdParseError, LocationExtent,
    Locator, SourceLocation,
};

pub const RESOURCE_RECORD_SCHEMA: &str = "logic.resource.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceObservationKind {
    Declared,
    Observed,
}

/// Provenance of one resource occurrence. This stays separate from the
/// producer-independent logical resource address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceOrigin {
    Source {
        repository_id: String,
        revision: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        snapshot_digest: Option<String>,
        document_index: u32,
        location: SourceLocation,
    },
    Artifact {
        artifact_id: String,
        provider_pointer: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceRecord {
    pub schema_version: String,
    /// Producer-independent identity derived only from `logical_address`.
    pub resource_id: String,
    /// One source declaration or artifact observation of the logical resource.
    pub occurrence_id: String,
    pub entity_kind: EntityKind,
    pub logical_address: Address,
    pub display_name: String,
    pub native_type: String,
    pub observation_kind: ResourceObservationKind,
    pub origin: ResourceOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locators: Vec<Locator>,
    /// Non-secret identity labels only. Native payloads remain in their source
    /// or immutable artifact and are reachable through `origin`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub identity_labels: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResourceValidationError {
    #[error("unsupported resource record schema_version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid resource address: {0}")]
    InvalidAddress(#[from] AddressValidationError),
    #[error("logical_address must use the resource scheme")]
    NonResourceAddress,
    #[error("resource field `{0}` must be non-empty and canonical")]
    InvalidField(&'static str),
    #[error("invalid resource id: {0}")]
    InvalidId(#[from] IdParseError),
    #[error("resource_id does not match logical_address")]
    ResourceIdMismatch,
    #[error("occurrence_id does not match resource origin")]
    OccurrenceIdMismatch,
    #[error("source revision must be immutable")]
    MutableRevision,
    #[error("invalid source path: {0}")]
    InvalidPath(String),
    #[error("source region is invalid")]
    InvalidRegion,
    #[error("identity labels must have non-empty keys and values")]
    InvalidIdentityLabel,
    #[error("entity_kind is not a resource-plane kind")]
    InvalidEntityKind,
    #[error("resource locator fields must be non-empty and canonical")]
    InvalidLocator,
}

impl ResourceOrigin {
    fn validate(&self) -> Result<(), ResourceValidationError> {
        match self {
            Self::Source {
                repository_id,
                revision,
                snapshot_digest,
                location,
                ..
            } => {
                require_component(repository_id, "origin.repository_id")?;
                if !is_immutable_revision(revision) {
                    return Err(ResourceValidationError::MutableRevision);
                }
                if let Some(digest) = snapshot_digest {
                    ContentDigest::parse(digest)?;
                }
                validate_source_location(location)?;
            }
            Self::Artifact {
                artifact_id,
                provider_pointer,
            } => {
                ArtifactId::parse(artifact_id)?;
                require_component(provider_pointer, "origin.provider_pointer")?;
            }
        }
        Ok(())
    }

    fn identity_parts(&self) -> Vec<String> {
        match self {
            Self::Source {
                repository_id,
                revision,
                snapshot_digest,
                document_index,
                location,
            } => {
                let mut parts = vec![
                    "source".into(),
                    repository_id.clone(),
                    revision.clone(),
                    snapshot_digest.clone().unwrap_or_default(),
                    location.path.clone(),
                    document_index.to_string(),
                ];
                match &location.extent {
                    LocationExtent::File => {
                        parts.push("file".into());
                    }
                    LocationExtent::Region { region } => {
                        parts.extend(region.identity_parts());
                    }
                }
                parts
            }
            Self::Artifact {
                artifact_id,
                provider_pointer,
            } => vec![
                "artifact".into(),
                artifact_id.clone(),
                provider_pointer.clone(),
            ],
        }
    }
}

impl ResourceRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        entity_kind: EntityKind,
        logical_address: Address,
        display_name: String,
        native_type: String,
        observation_kind: ResourceObservationKind,
        origin: ResourceOrigin,
        body_digest: Option<String>,
    ) -> Result<Self, ResourceValidationError> {
        let resource_id = resource_id(&logical_address)?;
        let occurrence_id = resource_occurrence_id(&resource_id, &origin)?;
        let record = Self {
            schema_version: RESOURCE_RECORD_SCHEMA.into(),
            resource_id,
            occurrence_id,
            entity_kind,
            logical_address,
            display_name,
            native_type,
            observation_kind,
            origin,
            body_digest,
            locators: Vec::new(),
            identity_labels: std::collections::BTreeMap::new(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), ResourceValidationError> {
        if self.schema_version != RESOURCE_RECORD_SCHEMA {
            return Err(ResourceValidationError::UnsupportedVersion(
                self.schema_version.clone(),
            ));
        }
        self.logical_address.validate()?;
        if !matches!(self.logical_address, Address::Resource { .. }) {
            return Err(ResourceValidationError::NonResourceAddress);
        }
        require_component(&self.display_name, "display_name")?;
        require_component(&self.native_type, "native_type")?;
        if !matches!(
            self.entity_kind,
            EntityKind::CloudAccount
                | EntityKind::KubernetesResource
                | EntityKind::InfrastructureResource
                | EntityKind::ConfigurationResource
                | EntityKind::Identity
                | EntityKind::NetworkEndpoint
                | EntityKind::Deployment
                | EntityKind::Workload
                | EntityKind::Database
                | EntityKind::Opaque
        ) {
            return Err(ResourceValidationError::InvalidEntityKind);
        }
        self.origin.validate()?;
        if let Some(digest) = &self.body_digest {
            ContentDigest::parse(digest)?;
        }
        if self
            .identity_labels
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            return Err(ResourceValidationError::InvalidIdentityLabel);
        }
        for locator in &self.locators {
            validate_locator(locator)?;
        }
        let expected_resource = resource_id(&self.logical_address)?;
        if self.resource_id != expected_resource {
            return Err(ResourceValidationError::ResourceIdMismatch);
        }
        let expected_occurrence = resource_occurrence_id(&self.resource_id, &self.origin)?;
        if self.occurrence_id != expected_occurrence {
            return Err(ResourceValidationError::OccurrenceIdMismatch);
        }
        Ok(())
    }
}

fn validate_source_location(location: &SourceLocation) -> Result<(), ResourceValidationError> {
    let normalized = normalize_repo_path(&location.path)
        .map_err(|error| ResourceValidationError::InvalidPath(error.to_string()))?;
    if normalized != location.path {
        return Err(ResourceValidationError::InvalidPath(location.path.clone()));
    }
    match &location.extent {
        LocationExtent::File => Ok(()),
        LocationExtent::Region { region } => region
            .validate()
            .map_err(|_| ResourceValidationError::InvalidRegion),
    }
}

fn validate_locator(locator: &Locator) -> Result<(), ResourceValidationError> {
    match locator {
        Locator::CodeLocation { location } => validate_source_location(location),
        Locator::Uri { uri } => require_locator(uri),
        Locator::CloudArn { arn } => require_locator(arn),
        Locator::K8sObject {
            cluster,
            namespace,
            api_version,
            resource_kind,
            name,
        } => {
            for value in [cluster, namespace, api_version, resource_kind, name] {
                require_locator(value)?;
            }
            Ok(())
        }
        Locator::Opaque { system, value } => {
            require_locator(system)?;
            require_locator(value)
        }
    }
}

fn require_locator(value: &str) -> Result<(), ResourceValidationError> {
    if value.trim().is_empty() || value.trim() != value || value.contains('\0') {
        Err(ResourceValidationError::InvalidLocator)
    } else {
        Ok(())
    }
}

pub fn resource_id(address: &Address) -> Result<String, ResourceValidationError> {
    address.validate()?;
    if !matches!(address, Address::Resource { .. }) {
        return Err(ResourceValidationError::NonResourceAddress);
    }
    Ok(hash_parts(&[
        "resource_entity",
        RESOURCE_RECORD_SCHEMA,
        &address_identity_key(address),
    ]))
}

pub fn resource_occurrence_id(
    resource_id: &str,
    origin: &ResourceOrigin,
) -> Result<String, ResourceValidationError> {
    ArtifactId::parse(resource_id)?;
    origin.validate()?;
    let mut parts = vec![
        "resource_occurrence".to_string(),
        RESOURCE_RECORD_SCHEMA.to_string(),
        resource_id.to_string(),
    ];
    parts.extend(origin.identity_parts());
    let refs: Vec<_> = parts.iter().map(String::as_str).collect();
    Ok(hash_parts(&refs))
}

fn require_component(value: &str, field: &'static str) -> Result<(), ResourceValidationError> {
    if value.trim().is_empty() || value.trim() != value || value.contains('\0') {
        return Err(ResourceValidationError::InvalidField(field));
    }
    Ok(())
}
