//! Semantic-cluster JSONL adapter (not SARIF).

use entmoot_contracts::{
    finding_observation_id, location_id as make_location_id, normalize_repo_path, ArtifactFormat,
    ArtifactManifest, CapabilityObservation, CapabilityOutcome, EvidenceCapability,
    FindingObservation, LocationExtent, LocationRecord, SourceRegion,
};
use serde_json::Value;

use super::{
    decode_text_bytes, ArtifactAdapter, ArtifactDiagnostic, ArtifactError, DiagnosticSeverity,
    InspectedArtifact, NormalizedArtifact,
};

pub struct SemanticClusterAdapter;

impl ArtifactAdapter for SemanticClusterAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let rows = parse_jsonl(bytes)?;
        let mut location_count = 0usize;
        for row in &rows {
            location_count += count_examples(row.get("source"));
            location_count += count_examples(row.get("sink"));
        }
        Ok(InspectedArtifact {
            format_hint: "semantic_cluster".into(),
            producer_name: rows
                .first()
                .and_then(|r| r.get("tool").and_then(Value::as_str))
                .map(str::to_string),
            producer_version: None,
            finding_count: rows.len(),
            location_count,
            trace_count: 0,
            document_count: 0,
            occurrence_count: 0,
            diagnostics: Vec::new(),
        })
    }

    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        if manifest.format != ArtifactFormat::SemanticCluster {
            return Err(ArtifactError::Message(format!(
                "SemanticClusterAdapter requires semantic_cluster format, got {:?}",
                manifest.format
            )));
        }
        manifest.validate_structure()?;
        manifest.verify_content_digest(bytes)?;

        if manifest.artifact_id.is_empty() {
            return Err(ArtifactError::Message(
                "manifest.artifact_id is empty; build manifests via ArtifactManifest::build".into(),
            ));
        }
        let artifact_key = manifest.artifact_id.clone();

        let rows = parse_jsonl(bytes)?;
        let mut out = NormalizedArtifact::default();

        for (result_index, row) in rows.iter().enumerate() {
            let rule_id = row
                .get("rule_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            let level = row
                .get("severity")
                .and_then(Value::as_str)
                .map(str::to_string);
            let observation_id = finding_observation_id(
                &artifact_key,
                0,
                &format!(
                    "cluster:{result_index}:{}",
                    rule_id.as_deref().unwrap_or("")
                ),
            );
            let mut primary_location_ids = Vec::new();

            // Preserve source/sink *segments* as provider facts on locations via pointer only.
            // Do NOT emit Source/Sink claims — Wave A keeps roles unknown.
            for (role_label, key) in [("source", "source"), ("sink", "sink")] {
                for (ex_index, example) in examples_from(row.get(key)).into_iter().enumerate() {
                    let pointer =
                        format!("/rows/{result_index}/{key}/examples/{ex_index}#{role_label}");
                    let Ok((line, column)) = example.coordinates else {
                        out.diagnostics.push(
                            ArtifactDiagnostic::error(
                                "semantic_cluster.invalid_region",
                                "line and column must be positive u32 coordinates",
                            )
                            .with_artifact(&artifact_key)
                            .with_pointer(&pointer),
                        );
                        continue;
                    };
                    let path = match normalize_repo_path(&example.file.replace('\\', "/")) {
                        Ok(p) => p,
                        Err(e) => {
                            out.diagnostics.push(
                                ArtifactDiagnostic::error("sarif.invalid_region", e.to_string())
                                    .with_artifact(&artifact_key)
                                    .with_pointer(&pointer),
                            );
                            continue;
                        }
                    };
                    if !manifest.path_in_scope(&path)? {
                        out.diagnostics.push(ArtifactDiagnostic {
                            code: "artifact.scope_violation".into(),
                            severity: DiagnosticSeverity::Info,
                            message: format!("cluster location excluded: {path}"),
                            artifact_id: Some(artifact_key.clone()),
                            provider_pointer: Some(pointer),
                        });
                        continue;
                    }
                    let loc_id =
                        make_location_id(&artifact_key, &path, line, column, None, None, &pointer);
                    primary_location_ids.push(loc_id.clone());
                    out.locations.push(LocationRecord {
                        location_id: loc_id,
                        artifact_id: artifact_key.clone(),
                        path,
                        extent: LocationExtent::Region {
                            region: SourceRegion::Text {
                                range: entmoot_contracts::SourceTextRange {
                                    start: entmoot_contracts::SourcePosition {
                                        line,
                                        column: column.unwrap_or(1),
                                    },
                                    end: entmoot_contracts::SourceTextEnd::Unspecified,
                                },
                                encoding: entmoot_contracts::ColumnEncoding::Unknown,
                            },
                        },

                        provider_pointer: pointer,
                        provider_confidence: None,
                    });
                }
            }

            out.findings.push(FindingObservation {
                observation_id,
                artifact_id: artifact_key.clone(),
                run_index: 0,
                result_index: u32::try_from(result_index)
                    .map_err(|_| ArtifactError::Message("result_index exceeds u32".into()))?,
                rule_id,
                level,
                message: None,
                fingerprints: Default::default(),
                taxonomies: Vec::new(),
                primary_location_ids,
                related_location_ids: Vec::new(),
                logical_location_ids: Vec::new(),
                trace_ids: Vec::new(),
                suppressions: Vec::new(),
                baseline_state: None,
                fix_pointers: Vec::new(),
                rule_metadata_pointer: None,
            });
        }
        out.capabilities = vec![
            CapabilityObservation {
                artifact_id: manifest.artifact_id.clone(),
                scope_universe_id: manifest.scope.scope_universe_id.clone(),
                capability: EvidenceCapability::FindingDetection,
                outcome: if out.findings.is_empty() {
                    CapabilityOutcome::Unknown
                } else {
                    CapabilityOutcome::Observed
                },
                basis: "logic.semantic_cluster.adapter.v1".into(),
                provider_pointer: Some("/rows".into()),
                limitation: Some(
                    "zero rows is not negative evidence without an explicit coverage declaration"
                        .into(),
                ),
            },
            CapabilityObservation {
                artifact_id: manifest.artifact_id.clone(),
                scope_universe_id: manifest.scope.scope_universe_id.clone(),
                capability: EvidenceCapability::StaticCallReachability,
                outcome: CapabilityOutcome::NotEvaluated,
                basis: "logic.semantic_cluster.adapter.v1".into(),
                provider_pointer: None,
                limitation: Some("artifact normalization does not run graph traversal".into()),
            },
            CapabilityObservation {
                artifact_id: manifest.artifact_id.clone(),
                scope_universe_id: manifest.scope.scope_universe_id.clone(),
                capability: EvidenceCapability::RuntimeExecutionReachability,
                outcome: CapabilityOutcome::RequiresExternalEvidence,
                basis: "logic.static_evidence_boundary.v1".into(),
                provider_pointer: None,
                limitation: Some("cannot be established from static examples alone".into()),
            },
            CapabilityObservation {
                artifact_id: manifest.artifact_id.clone(),
                scope_universe_id: manifest.scope.scope_universe_id.clone(),
                capability: EvidenceCapability::Exploitability,
                outcome: CapabilityOutcome::RequiresExternalEvidence,
                basis: "logic.static_evidence_boundary.v1".into(),
                provider_pointer: None,
                limitation: Some("requires reproducible runtime validation".into()),
            },
        ];
        Ok(out)
    }
}

#[derive(Clone)]
struct Example {
    file: String,
    coordinates: Result<(u32, Option<u32>), ()>,
}

fn parse_jsonl(bytes: &[u8]) -> Result<Vec<Value>, ArtifactError> {
    let text = decode_text_bytes(bytes)?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(ArtifactError::from))
        .collect()
}

fn count_examples(seg: Option<&Value>) -> usize {
    examples_from(seg).len()
}

fn examples_from(seg: Option<&Value>) -> Vec<Example> {
    let Some(seg) = seg else {
        return Vec::new();
    };
    fn example(value: &Value) -> Option<Example> {
        let positive = |value: &Value| {
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
        };
        let line = value.get("line").and_then(positive);
        let column = match value.get("column") {
            None => Some(None),
            Some(value) => positive(value).map(Some),
        };
        Some(Example {
            file: value.get("file")?.as_str()?.to_owned(),
            coordinates: line.zip(column).ok_or(()),
        })
    }
    if let Some(arr) = seg.get("examples").and_then(Value::as_array) {
        arr.iter().filter_map(example).collect()
    } else {
        example(seg).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::test_manifest::test_manifest;
    use std::path::PathBuf;

    #[test]
    fn malformed_cluster_coordinates_are_diagnostic_without_wrapping() {
        for (line, column) in [
            (serde_json::json!(4294967297_u64), serde_json::json!(1)),
            (serde_json::json!(1), serde_json::json!(4294967297_u64)),
            (serde_json::json!(0), serde_json::json!(1)),
            (serde_json::json!(-1), serde_json::json!(1)),
            (serde_json::json!(1), serde_json::json!("1")),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({"source":{"examples":[{"file":"a.py","line":line,"column":column}]}})).unwrap();
            let manifest = test_manifest(
                &bytes,
                ArtifactFormat::SemanticCluster,
                "fixture",
                None,
                vec![],
                vec![],
            );
            let normalized = SemanticClusterAdapter.normalize(&bytes, &manifest).unwrap();
            assert_eq!(normalized.findings.len(), 1);
            assert!(normalized.locations.is_empty());
            assert_eq!(normalized.diagnostics.len(), 1);
            assert_eq!(
                normalized.diagnostics[0].code,
                "semantic_cluster.invalid_region"
            );
            assert!(normalized.diagnostics[0].provider_pointer.is_some());
        }
    }

    #[test]
    fn cluster_fixture_preserves_locations_without_roles() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/entmoot-contracts/tests/fixtures/artifacts/sarif/semantic-cluster.min.jsonl");
        let bytes = std::fs::read(&path).unwrap();
        let manifest = test_manifest(
            &bytes,
            ArtifactFormat::SemanticCluster,
            "opengrep",
            None,
            vec![],
            vec![],
        );
        let inspected = SemanticClusterAdapter.inspect(&bytes).unwrap();
        assert_eq!(inspected.finding_count, 1);
        assert_eq!(inspected.location_count, 2);
        let norm = SemanticClusterAdapter.normalize(&bytes, &manifest).unwrap();
        assert_eq!(norm.findings.len(), 1);
        assert_eq!(norm.locations.len(), 2);
        assert!(norm.traces.is_empty());
        // Pointers retain source/sink labels for audit; no role Assertion emitted here.
        assert!(norm
            .locations
            .iter()
            .any(|l| l.provider_pointer.contains("#source")));
        assert!(norm
            .locations
            .iter()
            .any(|l| l.provider_pointer.contains("#sink")));
    }
}
