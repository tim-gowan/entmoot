//! Generic immutable resource-observation artifact adapter.

use std::collections::{BTreeMap, HashSet};

use entmoot_contracts::{
    relation_claim_with_id, relation_edge_with_id, ArtifactFormat, ArtifactManifest,
    CapabilityObservation, CapabilityOutcome, ContentDigest, EvidenceCapability, EvidenceRef,
    ProducerRef, Provenance, RelationClaimValue, RelationKind, ResolutionState,
    ResourceInventoryArtifact, ResourceOrigin, ResourceRecord, Scope, SubjectId,
    RESOURCE_INVENTORY_ARTIFACT_SCHEMA,
};

use super::{ArtifactAdapter, ArtifactError, InspectedArtifact, NormalizedArtifact};

const ADAPTER_VERSION: &str = "logic.resource_inventory.adapter.v1";

pub struct ResourceInventoryAdapter;

impl ArtifactAdapter for ResourceInventoryAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let document = parse(bytes)?;
        validate_document(&document, None)?;
        Ok(InspectedArtifact {
            format_hint: "resource_inventory".into(),
            producer_name: None,
            producer_version: None,
            finding_count: 0,
            location_count: 0,
            trace_count: 0,
            document_count: 1,
            occurrence_count: document.resources.len(),
            diagnostics: Vec::new(),
        })
    }

    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        if manifest.format != ArtifactFormat::ResourceInventory {
            return Err(ArtifactError::Message(format!(
                "ResourceInventoryAdapter requires resource_inventory format, got {:?}",
                manifest.format
            )));
        }
        manifest.validate_structure()?;
        manifest.verify_content_digest(bytes)?;
        let document = parse(bytes)?;
        validate_document(&document, Some(manifest))?;

        let producer = ProducerRef {
            id: manifest.artifact_series_id.clone(),
            name: manifest.producer.name.clone(),
            version: manifest.producer.version.clone(),
            config_fingerprint: manifest.config_digest.clone(),
            environment_fingerprint: None,
            capabilities: vec![
                entmoot_contracts::ProducerCapabilityKind::ArtifactAdapter,
                entmoot_contracts::ProducerCapabilityKind::RuntimeObserver,
            ],
        };
        let scope = Scope {
            domain: document.environment_id.clone(),
            snapshot_revision: Some(manifest.snapshot.revision.clone()),
            snapshot_digest: manifest.snapshot.snapshot_digest.clone(),
            occurrence_id: None,
            context_id: Some(document.observation_context_id.clone()),
        };
        let provenance = Some(Provenance {
            snapshot_revision: Some(manifest.snapshot.revision.clone()),
            snapshot_digest: manifest.snapshot.snapshot_digest.clone(),
            artifacts: vec![manifest.artifact_id.clone()],
            tool_digest: None,
            produced_at: manifest
                .observed_at
                .clone()
                .or_else(|| manifest.generated_at.clone()),
        });
        let mut output = NormalizedArtifact::default();

        for observed in document.resources {
            let mut resource = ResourceRecord::build(
                observed.entity_kind,
                observed.logical_address,
                observed.display_name,
                observed.native_type,
                observed.observation_kind,
                ResourceOrigin::Artifact {
                    artifact_id: manifest.artifact_id.clone(),
                    provider_pointer: observed.provider_pointer,
                },
                observed.body_digest,
            )
            .map_err(|error| ArtifactError::Message(format!("resource record: {error}")))?;
            resource.locators = observed.locators;
            resource.identity_labels = observed.identity_labels;
            resource
                .validate()
                .map_err(|error| ArtifactError::Message(format!("resource record: {error}")))?;
            output.resources.push(resource);
        }

        let mut relations_by_edge = BTreeMap::new();
        for observed in document.relations {
            let edge = relation_edge_with_id(
                "resource_observation",
                observed.subject,
                observed.predicate,
                observed.object,
                scope.clone(),
                None,
            );
            let evidence = EvidenceRef {
                artifact_id: manifest.artifact_id.clone(),
                provider_pointer: observed.provider_pointer,
                record_id: Some(edge.id.as_str().to_string()),
            };
            relations_by_edge
                .entry(edge.id.as_str().to_string())
                .or_insert_with(|| (edge, Vec::new()))
                .1
                .push(evidence);
        }
        for (_, (edge, evidence)) in relations_by_edge {
            let claim = relation_claim_with_id(
                SubjectId::RelationEdge {
                    edge_id: edge.id.clone(),
                },
                "entmoot.relation_exists",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::ArtifactResolved,
                },
                producer.clone(),
                scope.clone(),
                evidence,
                provenance.clone(),
            );
            output.relation_edges.push(edge);
            output.relation_claims.push(claim);
        }

        output
            .resources
            .sort_by(|left, right| left.occurrence_id.cmp(&right.occurrence_id));
        output
            .relation_edges
            .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        output
            .relation_claims
            .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        output.capabilities = capabilities(manifest, &output);
        Ok(output)
    }
}

fn parse(bytes: &[u8]) -> Result<ResourceInventoryArtifact, ArtifactError> {
    Ok(serde_json::from_slice(bytes)?)
}

fn validate_document(
    document: &ResourceInventoryArtifact,
    manifest: Option<&ArtifactManifest>,
) -> Result<(), ArtifactError> {
    if document.schema_version != RESOURCE_INVENTORY_ARTIFACT_SCHEMA {
        return Err(ArtifactError::Message(format!(
            "unsupported resource inventory schema_version `{}`",
            document.schema_version
        )));
    }
    for (field, value) in [
        ("environment_id", document.environment_id.as_str()),
        (
            "observation_context_id",
            document.observation_context_id.as_str(),
        ),
    ] {
        if value.trim().is_empty() || value.trim() != value || value.contains('\0') {
            return Err(ArtifactError::Message(format!(
                "resource inventory {field} must be non-empty and canonical"
            )));
        }
    }
    if manifest.is_some_and(|manifest| manifest.observed_at.is_none()) {
        return Err(ArtifactError::Message(
            "resource observation artifact requires manifest.observed_at".into(),
        ));
    }
    let mut pointers = HashSet::new();
    for resource in &document.resources {
        validate_pointer(&resource.provider_pointer)?;
        if !pointers.insert(resource.provider_pointer.as_str()) {
            return Err(ArtifactError::Message(format!(
                "duplicate provider_pointer `{}`",
                resource.provider_pointer
            )));
        }
        resource
            .logical_address
            .validate()
            .map_err(|error| ArtifactError::Message(format!("resource address: {error}")))?;
        if !matches!(
            resource.logical_address,
            entmoot_contracts::Address::Resource { .. }
        ) {
            return Err(ArtifactError::Message(
                "resource inventory records require resource addresses".into(),
            ));
        }
        if resource.observation_kind != entmoot_contracts::ResourceObservationKind::Observed {
            return Err(ArtifactError::Message(
                "resource observation artifacts accept only observed resource records".into(),
            ));
        }
        if let Some(digest) = &resource.body_digest {
            ContentDigest::parse(digest)
                .map_err(|error| ArtifactError::Message(format!("body_digest: {error}")))?;
        }
        let mut projected = ResourceRecord::build(
            resource.entity_kind.clone(),
            resource.logical_address.clone(),
            resource.display_name.clone(),
            resource.native_type.clone(),
            resource.observation_kind,
            ResourceOrigin::Artifact {
                artifact_id: "0".repeat(64),
                provider_pointer: resource.provider_pointer.clone(),
            },
            resource.body_digest.clone(),
        )
        .map_err(|error| ArtifactError::Message(format!("resource record: {error}")))?;
        projected.locators = resource.locators.clone();
        projected.identity_labels = resource.identity_labels.clone();
        projected
            .validate()
            .map_err(|error| ArtifactError::Message(format!("resource record: {error}")))?;
    }
    for relation in &document.relations {
        validate_pointer(&relation.provider_pointer)?;
        if !pointers.insert(relation.provider_pointer.as_str()) {
            return Err(ArtifactError::Message(format!(
                "duplicate provider_pointer `{}`",
                relation.provider_pointer
            )));
        }
        relation
            .subject
            .validate()
            .map_err(|error| ArtifactError::Message(format!("relation subject: {error}")))?;
        relation
            .object
            .validate()
            .map_err(|error| ArtifactError::Message(format!("relation object: {error}")))?;
        if !matches!(
            relation.subject,
            entmoot_contracts::Address::Resource { .. }
        ) || !matches!(relation.object, entmoot_contracts::Address::Resource { .. })
        {
            return Err(ArtifactError::Message(
                "resource inventory relations require resource endpoints".into(),
            ));
        }
        if !matches!(
            relation.predicate,
            RelationKind::DependsOn
                | RelationKind::Selects
                | RelationKind::RoutesTo
                | RelationKind::UsesConfiguration
                | RelationKind::ConnectsTo
                | RelationKind::ObservedAs
        ) {
            return Err(ArtifactError::Message(format!(
                "unsupported resource relation predicate `{:?}`",
                relation.predicate
            )));
        }
    }
    Ok(())
}

fn validate_pointer(pointer: &str) -> Result<(), ArtifactError> {
    if pointer.trim().is_empty() || pointer.trim() != pointer || pointer.contains('\0') {
        Err(ArtifactError::Message(
            "provider_pointer must be non-empty and canonical".into(),
        ))
    } else {
        Ok(())
    }
}

fn capabilities(
    manifest: &ArtifactManifest,
    output: &NormalizedArtifact,
) -> Vec<CapabilityObservation> {
    let deployment_outcome = if output.resources.is_empty() {
        CapabilityOutcome::Unknown
    } else {
        CapabilityOutcome::Observed
    };
    let exposure_outcome = if output.relation_edges.iter().any(|edge| {
        matches!(
            edge.predicate,
            RelationKind::ConnectsTo | RelationKind::RoutesTo
        )
    }) {
        CapabilityOutcome::Observed
    } else {
        CapabilityOutcome::Unknown
    };
    vec![
        CapabilityObservation {
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            capability: EvidenceCapability::DeploymentState,
            outcome: deployment_outcome,
            basis: ADAPTER_VERSION.into(),
            provider_pointer: Some("/resources".into()),
            limitation: Some(
                "observations apply only to the artifact environment and observation context"
                    .into(),
            ),
        },
        CapabilityObservation {
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            capability: EvidenceCapability::RuntimeExposure,
            outcome: exposure_outcome,
            basis: ADAPTER_VERSION.into(),
            provider_pointer: Some("/relations".into()),
            limitation: Some(
                "absence of a relation is not evidence that no runtime connection exists".into(),
            ),
        },
    ]
}
