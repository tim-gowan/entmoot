//! OSV-Scanner JSON result adapter.

use std::collections::BTreeMap;

use entmoot_contracts::{
    hash_parts, normalize_repo_path, purl_address, AdvisoryAffectedPackage, AdvisoryRange,
    AdvisoryRangeEvent, AffectedVersion, ArtifactFormat, ArtifactManifest, BomReferenceOutcome,
    CapabilityObservation, CapabilityOutcome, ComponentAffect, ComponentOccurrence,
    ComponentVulnerability, EvidenceCapability, LocationExtent, SourceLocation,
    VulnerabilityRating, COMPONENT_VULNERABILITY_SCHEMA,
};
use serde::Deserialize;
use serde_json::Value;

use super::{
    ArtifactAdapter, ArtifactDiagnostic, ArtifactError, InspectedArtifact, NormalizedArtifact,
};

const ADAPTER_POLICY: &str = "logic.osv_scanner_json.adapter.v1";

pub struct OsvScannerAdapter;

#[derive(Debug, Deserialize)]
struct ScannerOutput {
    #[serde(default)]
    results: Vec<ScanResult>,
}

#[derive(Debug, Deserialize)]
struct ScanResult {
    source: ScanSource,
    #[serde(default)]
    packages: Vec<PackageResult>,
}

#[derive(Debug, Deserialize)]
struct ScanSource {
    path: String,
    #[serde(rename = "type")]
    source_type: String,
}

#[derive(Debug, Deserialize)]
struct PackageResult {
    package: ScannedPackage,
    #[serde(default)]
    vulnerabilities: Vec<OsvVulnerability>,
    #[serde(default)]
    groups: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct ScannedPackage {
    name: String,
    version: String,
    ecosystem: String,
    purl: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OsvVulnerability {
    id: String,
    schema_version: Option<String>,
    modified: Option<String>,
    withdrawn: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    related: Vec<String>,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
}

#[derive(Debug, Deserialize)]
struct OsvSeverity {
    #[serde(rename = "type")]
    severity_type: String,
    score: String,
}

#[derive(Debug, Deserialize)]
struct OsvAffected {
    package: OsvPackage,
    #[serde(default)]
    ranges: Vec<OsvRange>,
    #[serde(default)]
    versions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OsvPackage {
    ecosystem: String,
    name: String,
    purl: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OsvRange {
    #[serde(rename = "type")]
    range_type: String,
    repo: Option<String>,
    #[serde(default)]
    events: Vec<BTreeMap<String, String>>,
}

impl ArtifactAdapter for OsvScannerAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let output = parse(bytes)?;
        let package_count = output
            .results
            .iter()
            .map(|result| result.packages.len())
            .sum();
        let finding_count = output
            .results
            .iter()
            .flat_map(|result| &result.packages)
            .map(|package| package.vulnerabilities.len())
            .sum();
        Ok(InspectedArtifact {
            format_hint: "osv-scanner-json".into(),
            producer_name: Some("osv-scanner".into()),
            producer_version: None,
            finding_count,
            location_count: 0,
            trace_count: 0,
            document_count: output.results.len(),
            occurrence_count: package_count,
            diagnostics: Vec::new(),
        })
    }

    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        if manifest.format != ArtifactFormat::OsvScanner {
            return Err(ArtifactError::Message(format!(
                "OsvScannerAdapter requires osv_scanner format, got {:?}",
                manifest.format
            )));
        }
        manifest.validate_structure()?;
        manifest.verify_content_digest(bytes)?;
        validate_producer(manifest)?;
        let output = parse(bytes)?;
        let mut normalized = NormalizedArtifact::default();

        for (result_index, result) in output.results.into_iter().enumerate() {
            validate_source(&result.source)?;
            let declared_in = source_location(&result.source.path);
            if declared_in.is_none() {
                normalized.diagnostics.push(
                    ArtifactDiagnostic::warning(
                        "osv_scanner.unmapped_source_path",
                        format!(
                            "source path `{}` is not repository-relative; no declared_in relation was inferred",
                            result.source.path
                        ),
                    )
                    .with_artifact(&manifest.artifact_id)
                    .with_pointer(&format!("/results/{result_index}/source/path")),
                );
            }
            for (package_index, package_result) in result.packages.into_iter().enumerate() {
                let component_pointer =
                    format!("/results/{result_index}/packages/{package_index}/package");
                let component = ComponentOccurrence::build(
                    manifest.artifact_id.clone(),
                    component_pointer,
                    "library".into(),
                    package_result.package.name.clone(),
                )
                .and_then(|record| record.with_purl(package_result.package.purl.clone()))
                .and_then(|record| {
                    record.with_coordinates(None, Some(package_result.package.version.clone()))
                })
                .and_then(|record| {
                    record.with_source(
                        Some(package_result.package.ecosystem.clone()),
                        Some(result.source.source_type.clone()),
                        declared_in.clone(),
                    )
                })
                .map_err(|error| ArtifactError::Message(format!("OSV package: {error}")))?;
                let component_id = component.occurrence_id.clone();
                normalized.components.push(component);

                if !package_result.groups.is_empty() {
                    normalized.diagnostics.push(
                        ArtifactDiagnostic::info(
                            "osv_scanner.native_groups_retained",
                            "alias grouping and experimental call-analysis fields remain in the immutable native artifact",
                        )
                        .with_artifact(&manifest.artifact_id)
                        .with_pointer(&format!("/results/{result_index}/packages/{package_index}/groups")),
                    );
                }

                for (vulnerability_index, vulnerability) in
                    package_result.vulnerabilities.into_iter().enumerate()
                {
                    validate_osv_schema(vulnerability.schema_version.as_deref())?;
                    let pointer = format!(
                        "/results/{result_index}/packages/{package_index}/vulnerabilities/{vulnerability_index}"
                    );
                    let observation_id =
                        hash_parts(&["component_vulnerability", &manifest.artifact_id, &pointer]);
                    let advisory_affected = vulnerability
                        .affected
                        .into_iter()
                        .map(normalize_affected)
                        .collect::<Result<Vec<_>, _>>()?;
                    let affect = ComponentAffect {
                        affect_id: hash_parts(&[
                            "component_affect",
                            &observation_id,
                            "0",
                            &component_id,
                        ]),
                        component_reference: component_id.clone(),
                        component_occurrence_id: Some(component_id.clone()),
                        outcome: BomReferenceOutcome::Resolved,
                        versions: vec![AffectedVersion {
                            version: Some(package_result.package.version.clone()),
                            range: None,
                            status: Some("affected".into()),
                        }],
                    };
                    let record = ComponentVulnerability {
                        schema_version: COMPONENT_VULNERABILITY_SCHEMA.into(),
                        observation_id,
                        artifact_id: manifest.artifact_id.clone(),
                        provider_pointer: pointer,
                        vulnerability_id: vulnerability.id,
                        bom_ref: None,
                        aliases: vulnerability.aliases,
                        advisory_schema_version: vulnerability.schema_version,
                        modified: vulnerability.modified,
                        withdrawn: vulnerability.withdrawn,
                        related: vulnerability.related,
                        source_name: Some("OSV".into()),
                        source_url: None,
                        ratings: vulnerability
                            .severity
                            .into_iter()
                            .map(|severity| VulnerabilityRating {
                                source: Some("OSV".into()),
                                score: Some(severity.score),
                                severity: None,
                                method: Some(severity.severity_type),
                                vector: None,
                            })
                            .collect(),
                        analysis: None,
                        affects: vec![affect],
                        advisory_affected,
                    };
                    record.validate().map_err(|error| {
                        ArtifactError::Message(format!("OSV vulnerability: {error}"))
                    })?;
                    normalized.component_vulnerabilities.push(record);
                }
            }
        }
        normalized
            .components
            .sort_by(|left, right| left.occurrence_id.cmp(&right.occurrence_id));
        normalized
            .component_vulnerabilities
            .sort_by(|left, right| left.observation_id.cmp(&right.observation_id));
        normalized.source_record_count =
            normalized.components.len() + normalized.component_vulnerabilities.len();
        normalized.capabilities = capabilities(manifest, &normalized);
        Ok(normalized)
    }
}

fn parse(bytes: &[u8]) -> Result<ScannerOutput, ArtifactError> {
    Ok(serde_json::from_slice(bytes)?)
}

fn validate_source(source: &ScanSource) -> Result<(), ArtifactError> {
    if source.path.trim().is_empty() || source.source_type.trim().is_empty() {
        return Err(ArtifactError::Message(
            "OSV-Scanner source path and type must be non-empty".into(),
        ));
    }
    Ok(())
}

fn validate_producer(manifest: &ArtifactManifest) -> Result<(), ArtifactError> {
    let version = manifest.producer.version.as_deref().ok_or_else(|| {
        ArtifactError::Message("OSV-Scanner artifact requires producer.version".into())
    })?;
    if !version.starts_with("2.") {
        return Err(ArtifactError::Message(format!(
            "unsupported OSV-Scanner producer version `{version}`; adapter supports v2 JSON"
        )));
    }
    Ok(())
}

fn validate_osv_schema(version: Option<&str>) -> Result<(), ArtifactError> {
    let Some(version) = version else {
        return Ok(());
    };
    if matches!(
        version,
        "1.0.0"
            | "1.1.0"
            | "1.2.0"
            | "1.3.0"
            | "1.4.0"
            | "1.5.0"
            | "1.6.0"
            | "1.7.0"
            | "1.8.0"
            | "1.9.0"
    ) {
        Ok(())
    } else {
        Err(ArtifactError::Message(format!(
            "unsupported OSV schema_version `{version}`; supported: 1.0.0-1.9.0"
        )))
    }
}

fn source_location(path: &str) -> Option<SourceLocation> {
    if std::path::Path::new(path).is_absolute() {
        return None;
    }
    let normalized = normalize_repo_path(path).ok()?;
    Some(SourceLocation {
        path: normalized,
        extent: LocationExtent::File,
    })
}

fn normalize_affected(affected: OsvAffected) -> Result<AdvisoryAffectedPackage, ArtifactError> {
    let purl = affected
        .package
        .purl
        .as_deref()
        .map(purl_address)
        .transpose()
        .map_err(|error| ArtifactError::Message(format!("OSV affected package PURL: {error}")))?;
    let ranges = affected
        .ranges
        .into_iter()
        .map(|range| {
            let events = range
                .events
                .into_iter()
                .map(|event| {
                    if event.len() != 1 {
                        return Err(ArtifactError::Message(
                            "OSV range event must contain exactly one event/value pair".into(),
                        ));
                    }
                    let (event, value) = event.into_iter().next().ok_or_else(|| {
                        ArtifactError::Message(
                            "OSV range event must contain exactly one event/value pair".into(),
                        )
                    })?;
                    Ok(AdvisoryRangeEvent { event, value })
                })
                .collect::<Result<Vec<_>, ArtifactError>>()?;
            Ok(AdvisoryRange {
                range_type: range.range_type,
                repository: range.repo,
                events,
            })
        })
        .collect::<Result<Vec<_>, ArtifactError>>()?;
    let fixed_versions = ranges
        .iter()
        .flat_map(|range| &range.events)
        .filter(|event| event.event == "fixed")
        .map(|event| event.value.clone())
        .collect();
    Ok(AdvisoryAffectedPackage {
        ecosystem: affected.package.ecosystem,
        name: affected.package.name,
        purl,
        ranges,
        versions: affected.versions,
        fixed_versions,
    })
}

fn capabilities(
    manifest: &ArtifactManifest,
    normalized: &NormalizedArtifact,
) -> Vec<CapabilityObservation> {
    [
        (EvidenceCapability::ComponentInventory, !normalized.components.is_empty()),
        (
            EvidenceCapability::VulnerabilityAffects,
            !normalized.component_vulnerabilities.is_empty(),
        ),
    ]
    .into_iter()
    .map(|(capability, observed)| CapabilityObservation {
        artifact_id: manifest.artifact_id.clone(),
        scope_universe_id: manifest.scope.scope_universe_id.clone(),
        capability,
        outcome: if observed {
            CapabilityOutcome::Observed
        } else {
            CapabilityOutcome::Unknown
        },
        basis: ADAPTER_POLICY.into(),
        provider_pointer: Some("/results".into()),
        limitation: Some(
            "OSV-Scanner JSON reports vulnerable package results; native groups and experimental call analysis remain raw provider facts"
                .into(),
        ),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::test_manifest::test_manifest;

    const OUTPUT: &str = r#"{
      "results":[{
        "source":{"path":"requirements.txt","type":"lockfile"},
        "packages":[{
          "package":{"name":"jinja2","version":"3.1.2","ecosystem":"PyPI","purl":"pkg:pypi/jinja2@3.1.2"},
          "vulnerabilities":[{
            "schema_version":"1.9.0","id":"GHSA-test","modified":"2099-01-01T00:00:00Z",
            "aliases":["CVE-2099-0001"],"related":["PYSEC-2099-1"],
            "severity":[{"type":"CVSS_V3","score":"CVSS:3.1/AV:N"}],
            "affected":[{
              "package":{"ecosystem":"PyPI","name":"jinja2","purl":"pkg:pypi/jinja2"},
              "ranges":[{"type":"ECOSYSTEM","events":[{"introduced":"0"},{"fixed":"3.1.3"}]}],
              "versions":["3.1.2"]
            }]
          }],
          "groups":[{"ids":["GHSA-test"],"experimentalAnalysis":{"GHSA-test":{"called":false}}}]
        }]
      }]
    }"#;

    #[test]
    fn retains_source_package_alias_ranges_and_fixed_versions() {
        let manifest = test_manifest(
            OUTPUT.as_bytes(),
            ArtifactFormat::OsvScanner,
            "osv-scanner",
            Some("2.3.1"),
            vec![],
            vec![],
        );
        let normalized = OsvScannerAdapter
            .normalize(OUTPUT.as_bytes(), &manifest)
            .expect("normalize");
        assert_eq!(normalized.components.len(), 1);
        let component = &normalized.components[0];
        assert_eq!(component.ecosystem.as_deref(), Some("PyPI"));
        assert_eq!(
            component
                .declared_in
                .as_ref()
                .map(|location| location.path.as_str()),
            Some("requirements.txt")
        );
        let vulnerability = &normalized.component_vulnerabilities[0];
        assert_eq!(vulnerability.aliases, ["CVE-2099-0001"]);
        assert_eq!(
            vulnerability.advisory_schema_version.as_deref(),
            Some("1.9.0")
        );
        assert_eq!(vulnerability.advisory_affected[0].fixed_versions, ["3.1.3"]);
        assert_eq!(
            vulnerability.affects[0].component_occurrence_id.as_deref(),
            Some(component.occurrence_id.as_str())
        );
        assert!(normalized
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "osv_scanner.native_groups_retained"));
    }

    #[test]
    fn absolute_source_path_is_retained_only_in_native_artifact() {
        let output = OUTPUT.replace("requirements.txt", "C:\\\\repo\\\\requirements.txt");
        let manifest = test_manifest(
            output.as_bytes(),
            ArtifactFormat::OsvScanner,
            "osv-scanner",
            Some("2.3.1"),
            vec![],
            vec![],
        );
        let normalized = OsvScannerAdapter
            .normalize(output.as_bytes(), &manifest)
            .expect("normalize");
        assert!(normalized.components[0].declared_in.is_none());
        assert!(normalized
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "osv_scanner.unmapped_source_path"));
    }

    #[test]
    fn unsupported_scanner_and_osv_schema_versions_are_rejected() {
        let old_manifest = test_manifest(
            OUTPUT.as_bytes(),
            ArtifactFormat::OsvScanner,
            "osv-scanner",
            Some("1.9.2"),
            vec![],
            vec![],
        );
        assert!(OsvScannerAdapter
            .normalize(OUTPUT.as_bytes(), &old_manifest)
            .unwrap_err()
            .to_string()
            .contains("producer version"));
        let future = OUTPUT.replace("1.9.0", "2.0.0");
        let manifest = test_manifest(
            future.as_bytes(),
            ArtifactFormat::OsvScanner,
            "osv-scanner",
            Some("2.3.1"),
            vec![],
            vec![],
        );
        assert!(OsvScannerAdapter
            .normalize(future.as_bytes(), &manifest)
            .unwrap_err()
            .to_string()
            .contains("schema_version"));
    }

    #[test]
    fn malformed_range_events_are_rejected_instead_of_flattened() {
        let malformed = OUTPUT.replace(
            r#"{"introduced":"0"}"#,
            r#"{"introduced":"0","fixed":"3.1.3"}"#,
        );
        let manifest = test_manifest(
            malformed.as_bytes(),
            ArtifactFormat::OsvScanner,
            "osv-scanner",
            Some("2.3.1"),
            vec![],
            vec![],
        );
        assert!(OsvScannerAdapter
            .normalize(malformed.as_bytes(), &manifest)
            .unwrap_err()
            .to_string()
            .contains("exactly one"));
    }
}
