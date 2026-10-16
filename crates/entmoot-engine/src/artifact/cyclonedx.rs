//! CycloneDX JSON component inventory and VEX adapter.

use std::collections::HashMap;

use entmoot_contracts::{
    hash_parts, AffectedVersion, ArtifactFormat, ArtifactManifest, BomComposition,
    BomReferenceOutcome, CapabilityObservation, CapabilityOutcome, ComponentAffect,
    ComponentDependency, ComponentDependencyKind, ComponentOccurrence, ComponentVulnerability,
    EvidenceCapability, VexAnalysis, VulnerabilityRating, BOM_COMPOSITION_SCHEMA,
    COMPONENT_VULNERABILITY_SCHEMA,
};
use serde::Deserialize;
use serde_json::Value;

use super::{
    ArtifactAdapter, ArtifactDiagnostic, ArtifactError, InspectedArtifact, NormalizedArtifact,
};

const ADAPTER_POLICY: &str = "logic.cyclonedx_json.adapter.v1";

pub struct CycloneDxAdapter;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bom {
    bom_format: String,
    spec_version: String,
    metadata: Option<NativeMetadata>,
    #[serde(default)]
    components: Vec<NativeComponent>,
    #[serde(default)]
    dependencies: Vec<NativeDependency>,
    #[serde(default)]
    vulnerabilities: Vec<NativeVulnerability>,
    #[serde(default)]
    compositions: Vec<NativeComposition>,
}

#[derive(Debug, Deserialize)]
struct NativeMetadata {
    component: Option<NativeComponent>,
}

#[derive(Debug, Deserialize)]
struct NativeComponent {
    #[serde(rename = "type")]
    component_type: String,
    #[serde(rename = "bom-ref")]
    bom_ref: Option<String>,
    group: Option<String>,
    name: String,
    version: Option<String>,
    purl: Option<String>,
    #[serde(default)]
    components: Vec<NativeComponent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeDependency {
    #[serde(rename = "ref")]
    subject: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeVulnerability {
    #[serde(rename = "bom-ref")]
    bom_ref: Option<String>,
    id: String,
    source: Option<NativeSource>,
    #[serde(default)]
    ratings: Vec<NativeRating>,
    analysis: Option<NativeAnalysis>,
    #[serde(default)]
    affects: Vec<NativeAffect>,
}

#[derive(Debug, Deserialize)]
struct NativeSource {
    name: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeRating {
    source: Option<NativeSource>,
    score: Option<Value>,
    severity: Option<String>,
    method: Option<String>,
    vector: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeAnalysis {
    state: String,
    justification: Option<String>,
    #[serde(default)]
    response: Vec<String>,
    detail: Option<String>,
    #[serde(rename = "firstIssued")]
    first_issued: Option<String>,
    #[serde(rename = "lastUpdated")]
    last_updated: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeAffect {
    #[serde(rename = "ref")]
    component: String,
    #[serde(default)]
    versions: Vec<NativeAffectedVersion>,
}

#[derive(Debug, Deserialize)]
struct NativeAffectedVersion {
    version: Option<String>,
    range: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeComposition {
    aggregate: String,
    #[serde(default)]
    assemblies: Vec<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    vulnerabilities: Vec<String>,
}

impl ArtifactAdapter for CycloneDxAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let bom = parse(bytes)?;
        validate_header(&bom)?;
        Ok(InspectedArtifact {
            format_hint: format!("cyclonedx-json-{}", bom.spec_version),
            producer_name: None,
            producer_version: None,
            finding_count: bom.vulnerabilities.len(),
            location_count: 0,
            trace_count: 0,
            document_count: 1,
            occurrence_count: component_count(&bom.components)
                + usize::from(
                    bom.metadata
                        .as_ref()
                        .and_then(|metadata| metadata.component.as_ref())
                        .is_some(),
                ),
            diagnostics: Vec::new(),
        })
    }

    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        if manifest.format != ArtifactFormat::CycloneDx {
            return Err(ArtifactError::Message(format!(
                "CycloneDxAdapter requires cyclonedx format, got {:?}",
                manifest.format
            )));
        }
        manifest.validate_structure()?;
        manifest.verify_content_digest(bytes)?;
        let bom = parse(bytes)?;
        validate_header(&bom)?;
        let mut out = NormalizedArtifact::default();
        if let Some(root) = bom
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.component.as_ref())
        {
            collect_component(
                root,
                "/metadata/component".into(),
                manifest,
                &mut out.components,
            )?;
        }
        collect_components(
            &bom.components,
            "/components",
            manifest,
            &mut out.components,
        )?;
        out.components
            .sort_by(|a, b| a.provider_pointer.cmp(&b.provider_pointer));

        let mut by_ref = HashMap::new();
        for component in &out.components {
            if let Some(reference) = &component.bom_ref {
                if by_ref
                    .insert(reference.clone(), component.occurrence_id.clone())
                    .is_some()
                {
                    return Err(ArtifactError::Message(format!(
                        "duplicate CycloneDX bom-ref `{reference}`"
                    )));
                }
            }
        }

        for (index, dependency) in bom.dependencies.into_iter().enumerate() {
            for (kind, targets, field) in [
                (
                    ComponentDependencyKind::DependsOn,
                    dependency.depends_on,
                    "dependsOn",
                ),
                (
                    ComponentDependencyKind::Provides,
                    dependency.provides,
                    "provides",
                ),
            ] {
                for (target_index, target) in targets.into_iter().enumerate() {
                    let pointer = format!("/dependencies/{index}/{field}/{target_index}");
                    let record = ComponentDependency::build(
                        manifest.artifact_id.clone(),
                        pointer.clone(),
                        kind,
                        dependency.subject.clone(),
                        target.clone(),
                        by_ref.get(&dependency.subject).cloned(),
                        by_ref.get(&target).cloned(),
                    )
                    .map_err(|error| {
                        ArtifactError::Message(format!("CycloneDX dependency: {error}"))
                    })?;
                    if record.outcome != BomReferenceOutcome::Resolved {
                        out.diagnostics.push(
                            ArtifactDiagnostic::warning(
                                "cyclonedx.dangling_dependency_ref",
                                format!(
                                    "dependency `{}` -> `{}` has outcome {:?}",
                                    dependency.subject, target, record.outcome
                                ),
                            )
                            .with_artifact(&manifest.artifact_id)
                            .with_pointer(&pointer),
                        );
                    }
                    out.component_dependencies.push(record);
                }
            }
        }

        for (index, vulnerability) in bom.vulnerabilities.into_iter().enumerate() {
            let pointer = format!("/vulnerabilities/{index}");
            let observation_id =
                hash_parts(&["component_vulnerability", &manifest.artifact_id, &pointer]);
            let source_name = vulnerability
                .source
                .as_ref()
                .and_then(|source| source.name.clone());
            let source_url = vulnerability
                .source
                .as_ref()
                .and_then(|source| source.url.clone());
            let ratings = vulnerability
                .ratings
                .into_iter()
                .map(|rating| VulnerabilityRating {
                    source: rating.source.and_then(|source| source.name),
                    score: rating.score.map(value_string),
                    severity: rating.severity,
                    method: rating.method,
                    vector: rating.vector,
                })
                .collect();
            let analysis = vulnerability.analysis.map(|analysis| VexAnalysis {
                state: analysis.state,
                justification: analysis.justification,
                responses: analysis.response,
                detail: analysis.detail,
                first_issued: analysis.first_issued,
                last_updated: analysis.last_updated,
            });
            let affects = vulnerability
                .affects
                .into_iter()
                .enumerate()
                .map(|(affect_index, affect)| {
                    let component_occurrence_id = by_ref.get(&affect.component).cloned();
                    ComponentAffect {
                        affect_id: hash_parts(&[
                            "component_affect",
                            &observation_id,
                            &affect_index.to_string(),
                            &affect.component,
                        ]),
                        component_reference: affect.component,
                        outcome: if component_occurrence_id.is_some() {
                            BomReferenceOutcome::Resolved
                        } else {
                            BomReferenceOutcome::MissingObject
                        },
                        component_occurrence_id,
                        versions: affect
                            .versions
                            .into_iter()
                            .map(|version| AffectedVersion {
                                version: version.version,
                                range: version.range,
                                status: version.status,
                            })
                            .collect(),
                    }
                })
                .collect::<Vec<_>>();
            for affect in &affects {
                if affect.outcome != BomReferenceOutcome::Resolved {
                    out.diagnostics.push(
                        ArtifactDiagnostic::warning(
                            "cyclonedx.dangling_affect_ref",
                            format!(
                                "vulnerability `{}` affects missing bom-ref `{}`",
                                vulnerability.id, affect.component_reference
                            ),
                        )
                        .with_artifact(&manifest.artifact_id)
                        .with_pointer(&pointer),
                    );
                }
            }
            let record = ComponentVulnerability {
                schema_version: COMPONENT_VULNERABILITY_SCHEMA.into(),
                observation_id,
                artifact_id: manifest.artifact_id.clone(),
                provider_pointer: pointer,
                vulnerability_id: vulnerability.id,
                bom_ref: vulnerability.bom_ref,
                aliases: Vec::new(),
                advisory_schema_version: None,
                modified: None,
                withdrawn: None,
                related: Vec::new(),
                source_name,
                source_url,
                ratings,
                analysis,
                affects,
                advisory_affected: Vec::new(),
            };
            record.validate().map_err(|error| {
                ArtifactError::Message(format!("CycloneDX vulnerability: {error}"))
            })?;
            out.component_vulnerabilities.push(record);
        }

        for (index, composition) in bom.compositions.into_iter().enumerate() {
            let provider_pointer = format!("/compositions/{index}");
            let record = BomComposition {
                schema_version: BOM_COMPOSITION_SCHEMA.into(),
                composition_id: hash_parts(&[
                    "bom_composition",
                    &manifest.artifact_id,
                    &provider_pointer,
                ]),
                artifact_id: manifest.artifact_id.clone(),
                provider_pointer,
                aggregate: composition.aggregate,
                assemblies: composition.assemblies,
                dependencies: composition.dependencies,
                vulnerabilities: composition.vulnerabilities,
            };
            record.validate().map_err(|error| {
                ArtifactError::Message(format!("CycloneDX composition: {error}"))
            })?;
            out.bom_compositions.push(record);
        }
        out.source_record_count = out.components.len()
            + out.component_dependencies.len()
            + out.component_vulnerabilities.len()
            + out.bom_compositions.len();
        out.capabilities = capabilities(manifest, &out);
        Ok(out)
    }
}

fn parse(bytes: &[u8]) -> Result<Bom, ArtifactError> {
    Ok(serde_json::from_slice(bytes)?)
}

fn validate_header(bom: &Bom) -> Result<(), ArtifactError> {
    if bom.bom_format != "CycloneDX" {
        return Err(ArtifactError::Message(format!(
            "expected CycloneDX bomFormat, got `{}`",
            bom.bom_format
        )));
    }
    if !matches!(bom.spec_version.as_str(), "1.4" | "1.5" | "1.6" | "1.7") {
        return Err(ArtifactError::Message(format!(
            "unsupported CycloneDX specVersion `{}`; supported: 1.4-1.7",
            bom.spec_version
        )));
    }
    Ok(())
}

fn component_count(components: &[NativeComponent]) -> usize {
    components
        .iter()
        .map(|component| 1 + component_count(&component.components))
        .sum()
}

fn collect_components(
    components: &[NativeComponent],
    base: &str,
    manifest: &ArtifactManifest,
    output: &mut Vec<ComponentOccurrence>,
) -> Result<(), ArtifactError> {
    for (index, component) in components.iter().enumerate() {
        let pointer = format!("{base}/{index}");
        collect_component(component, pointer, manifest, output)?;
    }
    Ok(())
}

fn collect_component(
    component: &NativeComponent,
    pointer: String,
    manifest: &ArtifactManifest,
    output: &mut Vec<ComponentOccurrence>,
) -> Result<(), ArtifactError> {
    let record = ComponentOccurrence::build(
        manifest.artifact_id.clone(),
        pointer.clone(),
        component.component_type.clone(),
        component.name.clone(),
    )
    .and_then(|record| record.with_bom_ref(component.bom_ref.clone()))
    .and_then(|record| record.with_purl(component.purl.clone()))
    .and_then(|record| record.with_coordinates(component.group.clone(), component.version.clone()))
    .map_err(|error| ArtifactError::Message(format!("CycloneDX component: {error}")))?;
    output.push(record);
    collect_components(
        &component.components,
        &format!("{pointer}/components"),
        manifest,
        output,
    )
}

fn capabilities(
    manifest: &ArtifactManifest,
    out: &NormalizedArtifact,
) -> Vec<CapabilityObservation> {
    [
        (EvidenceCapability::ComponentInventory, !out.components.is_empty()),
        (EvidenceCapability::ComponentDependencies, !out.component_dependencies.is_empty()),
        (EvidenceCapability::VulnerabilityAffects, !out.component_vulnerabilities.is_empty()),
        (EvidenceCapability::VexAnalysis, out.component_vulnerabilities.iter().any(|v| v.analysis.is_some())),
        (EvidenceCapability::BomCompleteness, !out.bom_compositions.is_empty()),
    ].into_iter().map(|(capability, observed)| CapabilityObservation {
        artifact_id: manifest.artifact_id.clone(), scope_universe_id: manifest.scope.scope_universe_id.clone(),
        capability, outcome: if observed { CapabilityOutcome::Observed } else { CapabilityOutcome::Unknown },
        basis: ADAPTER_POLICY.into(), provider_pointer: Some("/".into()),
        limitation: Some("native CycloneDX JSON remains authoritative; normalized facts do not imply source-file declaration or runtime presence".into()),
    }).collect()
}

fn value_string(value: Value) -> String {
    match value {
        Value::String(value) => value,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::test_manifest::test_manifest;
    use crate::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};

    const BOM: &str = r#"{
      "bomFormat":"CycloneDX","specVersion":"1.7","serialNumber":"urn:uuid:00000000-0000-4000-8000-000000000001","version":1,
      "metadata":{"component":{"type":"application","bom-ref":"app","name":"app","version":"1"}},
      "components":[
        {"type":"library","bom-ref":"pkg","group":"acme","name":"lib","version":"2.0.0","purl":"pkg:pypi/lib@2.0.0"}
      ],
      "dependencies":[{"ref":"app","dependsOn":["pkg","absent"],"provides":["pkg"]}],
      "vulnerabilities":[{
        "id":"CVE-2099-0001","source":{"name":"NVD","url":"https://example.invalid/nvd"},
        "ratings":[{"score":9.8,"severity":"critical","method":"CVSSv31","vector":"CVSS:3.1/..."}],
        "analysis":{"state":"not_affected","justification":"code_not_present","response":["will_not_fix"],"detail":"fixture","firstIssued":"2099-01-01T00:00:00Z","lastUpdated":"2099-01-02T00:00:00Z"},
        "affects":[{"ref":"pkg","versions":[{"range":">=2.0.0 <2.1.0","status":"affected"}]},{"ref":"missing"}]
      }],
      "compositions":[{"aggregate":"incomplete_first_party_only","assemblies":["app"],"dependencies":["pkg"],"vulnerabilities":["CVE-2099-0001"]}]
    }"#;

    #[test]
    fn retains_inventory_dependencies_vex_and_completeness() {
        let manifest = test_manifest(
            BOM.as_bytes(),
            ArtifactFormat::CycloneDx,
            "fixture",
            Some("1"),
            vec![],
            vec![],
        );
        let normalized = CycloneDxAdapter
            .normalize(BOM.as_bytes(), &manifest)
            .expect("normalize");
        assert_eq!(normalized.components.len(), 2);
        assert_eq!(normalized.component_dependencies.len(), 3);
        assert_eq!(
            normalized
                .component_dependencies
                .iter()
                .filter(|record| record.outcome == BomReferenceOutcome::MissingObject)
                .count(),
            1
        );
        assert_eq!(normalized.component_vulnerabilities.len(), 1);
        assert_eq!(normalized.component_vulnerabilities[0].affects.len(), 2);
        assert_eq!(
            normalized.component_vulnerabilities[0].ratings[0]
                .score
                .as_deref(),
            Some("9.8")
        );
        assert_eq!(
            normalized.component_vulnerabilities[0]
                .analysis
                .as_ref()
                .map(|analysis| analysis.state.as_str()),
            Some("not_affected")
        );
        assert_eq!(
            normalized.bom_compositions[0].aggregate,
            "incomplete_first_party_only"
        );
        assert_eq!(normalized.diagnostics.len(), 2);
    }

    #[test]
    fn duplicate_bom_refs_and_unknown_versions_are_rejected() {
        let duplicate = r#"{"bomFormat":"CycloneDX","specVersion":"1.7","components":[{"type":"library","bom-ref":"x","name":"a"},{"type":"library","bom-ref":"x","name":"b"}]}"#;
        let manifest = test_manifest(
            duplicate.as_bytes(),
            ArtifactFormat::CycloneDx,
            "fixture",
            None,
            vec![],
            vec![],
        );
        assert!(CycloneDxAdapter
            .normalize(duplicate.as_bytes(), &manifest)
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
        let future = br#"{"bomFormat":"CycloneDX","specVersion":"2.0"}"#;
        assert!(CycloneDxAdapter
            .inspect(future)
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
    }

    #[test]
    fn normalized_facts_round_trip_through_durable_knowledge() {
        let manifest = test_manifest(
            BOM.as_bytes(),
            ArtifactFormat::CycloneDx,
            "fixture",
            Some("1"),
            vec![],
            vec![],
        );
        let normalized = CycloneDxAdapter
            .normalize(BOM.as_bytes(), &manifest)
            .expect("normalize");
        let mut expected_component_ids = normalized
            .components
            .iter()
            .map(|component| component.occurrence_id.clone())
            .collect::<Vec<_>>();
        expected_component_ids.sort();
        let store = KnowledgeStore {
            artifacts: vec![manifest],
            capabilities: normalized.capabilities,
            components: normalized.components,
            component_dependencies: normalized.component_dependencies,
            component_vulnerabilities: normalized.component_vulnerabilities,
            bom_compositions: normalized.bom_compositions,
            diagnostics: normalized.diagnostics,
            ..KnowledgeStore::default()
        };
        let mut repository = SqliteKnowledgeRepository::open_in_memory().expect("repository");
        repository.synchronize(&store).expect("synchronize");
        let loaded = repository.load().expect("load");
        let mut loaded_component_ids = loaded
            .components
            .iter()
            .map(|component| component.occurrence_id.clone())
            .collect::<Vec<_>>();
        loaded_component_ids.sort();
        assert_eq!(loaded_component_ids, expected_component_ids);
        assert_eq!(loaded.component_dependencies.len(), 3);
        assert_eq!(loaded.component_vulnerabilities.len(), 1);
        assert_eq!(loaded.bom_compositions.len(), 1);
    }
}
