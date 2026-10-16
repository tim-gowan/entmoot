//! Attributable usage and execution facts, joined only to exact snapshot declarations.
use super::*;
use entmoot_contracts::*;
use std::collections::HashMap;

pub struct AssessmentEvidenceAdapter;

fn parse(bytes: &[u8]) -> Result<AssessmentEvidence, ArtifactError> {
    let artifact: AssessmentEvidence = serde_json::from_str(&decode_text_bytes(bytes)?)?;
    if artifact.schema_version != "entmoot.assessment-evidence.v1"
        || artifact.implementation != crate::implementation_fingerprint()
    {
        return Err(ArtifactError::Message(
            "incompatible assessment evidence".into(),
        ));
    }
    artifact
        .snapshot
        .validate()
        .map_err(|e| ArtifactError::Message(e.to_string()))?;
    artifact
        .producer
        .validate()
        .map_err(|e| ArtifactError::Message(e.to_string()))?;
    ContentDigest::parse(artifact.build_context.as_str())
        .map_err(|e| ArtifactError::Message(e.to_string()))?;
    if artifact.observations.len() > 100_000 {
        return Err(ArtifactError::Message(
            "assessment observation budget exceeded".into(),
        ));
    }
    for observation in &artifact.observations {
        observation
            .validate()
            .map_err(|e| ArtifactError::Message(e.to_string()))?;
    }

    Ok(artifact)
}

impl ArtifactAdapter for AssessmentEvidenceAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let artifact = parse(bytes)?;
        Ok(InspectedArtifact {
            format_hint: "assessment_evidence".into(),
            producer_name: Some(artifact.producer.name),
            producer_version: artifact.producer.version,
            finding_count: 0,
            location_count: 0,
            trace_count: 0,
            document_count: 0,
            occurrence_count: artifact.observations.len(),
            diagnostics: vec![],
        })
    }
    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        let artifact = parse(bytes)?;
        if manifest.format != ArtifactFormat::AssessmentEvidence
            || artifact.snapshot != manifest.snapshot
            || artifact.producer.name != manifest.producer.name
            || artifact.producer.version != manifest.producer.version
            || manifest.config_digest.as_deref() != Some(artifact.build_context.as_str())
        {
            return Err(ArtifactError::Message(
                "assessment evidence envelope disagrees with manifest".into(),
            ));
        }
        Ok(NormalizedArtifact {
            source_record_count: artifact.observations.len(),
            assessment_evidence: Some(artifact),
            ..Default::default()
        })
    }
}

pub(crate) fn join(
    graph: &crate::TopologyGraph,
    manifest: &ArtifactManifest,
    normalized: &mut NormalizedArtifact,
) -> Result<(), ArtifactError> {
    let Some(artifact) = normalized.assessment_evidence.take() else {
        return Ok(());
    };
    let paths: Vec<_> = artifact
        .observations
        .iter()
        .flat_map(|o| o.declarations())
        .map(|d| d.path.as_str())
        .collect();
    // Check stored source bytes too: matching provider coordinates alone cannot attach.
    crate::read_graph_sources_for_paths(graph, &paths)
        .map_err(|e| ArtifactError::Message(e.to_string()))?;
    let mut declarations: HashMap<&str, Vec<&crate::TopologyNode>> = HashMap::new();
    for node in &graph.nodes {
        if let Some(id) = &node.address_id {
            declarations.entry(id).or_default().push(node);
        }
    }
    let digests: HashMap<_, _> = graph
        .snapshot_files
        .iter()
        .map(|f| (f.path.as_str(), f.content_digest.as_str()))
        .collect();
    for (i, observation) in artifact.observations.into_iter().enumerate() {
        let collection = if manifest.format == ArtifactFormat::Resolver {
            "assessments"
        } else {
            "observations"
        };
        let pointer = format!("/{collection}/{i}");
        let subjects = observation.declarations();
        let valid = subjects.iter().all(|subject| {
            manifest.scope.path_in_scope(&subject.path).unwrap_or(false)
                && digests.get(subject.path.as_str()).copied()
                    == Some(subject.source_digest.as_str())
                && declarations
                    .get(subject.declaration.occurrence.address_id.as_str())
                    .is_some_and(|nodes| {
                        nodes.len() == 1
                            && nodes[0].path.as_deref() == Some(&subject.path)
                            && nodes[0].logic_address.as_deref()
                                == Some(&subject.declaration.occurrence.logic_address)
                            && nodes[0].start_byte == Some(subject.declaration.span.start_byte)
                            && nodes[0].end_byte == Some(subject.declaration.span.end_byte)
                    })
        });
        if !valid {
            normalized.diagnostics.push(ArtifactDiagnostic::warning("assessment_declaration_unmatched",
                "exact declaration/snapshot attachment unavailable; observation retained only in its artifact")
                .with_artifact(&manifest.artifact_id).with_pointer(&pointer));
            continue;
        }
        let subject = match &observation {
            AssessmentObservation::CodeUse { subject, .. }
            | AssessmentObservation::CompilerUnused { subject, .. } => SubjectId::Address {
                address: Address::AddressId {
                    address_id: subject.declaration.occurrence.address_id.to_string(),
                },
            },
            AssessmentObservation::TestExecution { .. } => SubjectId::Artifact {
                artifact_id: manifest.artifact_id.clone(),
            },
        };
        let claim = relation_claim_with_id(
            subject,
            "assessment.observation",
            RelationClaimValue::Assessment { observation },
            artifact.producer.clone(),
            Scope {
                domain: artifact.snapshot.repository_id.clone(),
                snapshot_revision: Some(artifact.snapshot.revision.clone()),
                snapshot_digest: artifact.snapshot.snapshot_digest.clone(),
                occurrence_id: None,
                context_id: Some(artifact.build_context.to_string()),
            },
            vec![EvidenceRef {
                artifact_id: manifest.artifact_id.clone(),
                provider_pointer: pointer,
                record_id: None,
            }],
            None,
        );
        normalized.relation_claims.push(claim);
    }
    Ok(())
}
