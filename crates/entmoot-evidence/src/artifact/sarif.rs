//! SARIF 2.1 lossless normalization (no taint-role invention).

use std::collections::{BTreeMap, HashMap};

use entmoot_contracts::{
    file_location_id, finding_observation_id, location_id as make_location_id, normalize_repo_path,
    step_id as make_step_id, trace_id as make_trace_id, ArtifactFormat, ArtifactManifest,
    CapabilityObservation, CapabilityOutcome, EvidenceCapability, FindingObservation,
    LocationExtent, LocationRecord, SourceRegion, SuppressionDetail, TraceRecord, TraceStepRecord,
};
use serde_json::Value;

use super::{
    decode_text_bytes, ArtifactAdapter, ArtifactDiagnostic, ArtifactError, DiagnosticSeverity,
    InspectedArtifact, NormalizedArtifact, ProviderRoleHint,
};

pub struct SarifAdapter;

const OPENGREP_RULE_ROLE_POLICY: &str = "logic.sarif.opengrep.rule_role.v1";

impl ArtifactAdapter for SarifAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let doc = parse_sarif_document(bytes)?;
        let mut finding_count = 0usize;
        let mut location_count = 0usize;
        let mut trace_count = 0usize;
        let mut producer_name = None;
        let mut producer_version = None;
        let mut diagnostics = Vec::new();

        let runs = doc
            .get("runs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if runs.is_empty() {
            diagnostics.push(ArtifactDiagnostic::error(
                "sarif.no_runs",
                "SARIF document contains no runs",
            ));
        }
        for run in &runs {
            if producer_name.is_none() {
                producer_name = run
                    .pointer("/tool/driver/name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                producer_version = run
                    .pointer("/tool/driver/version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            let results = run
                .get("results")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            finding_count += results;
            for result in run
                .get("results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                location_count += result
                    .get("locations")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                for cf in result
                    .get("codeFlows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    trace_count += cf
                        .get("threadFlows")
                        .and_then(Value::as_array)
                        .map(|a| a.len())
                        .unwrap_or(0);
                }
            }
        }

        Ok(InspectedArtifact {
            format_hint: "sarif_2_1".into(),
            producer_name,
            producer_version,
            finding_count,
            location_count,
            trace_count,
            document_count: 0,
            occurrence_count: 0,
            diagnostics,
        })
    }

    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        if manifest.format != ArtifactFormat::Sarif21 {
            return Err(ArtifactError::Message(format!(
                "SarifAdapter requires format sarif21, got {:?}",
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
        let doc = parse_sarif_document(bytes)?;
        let mut out = NormalizedArtifact::default();
        let runs = doc
            .get("runs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for (run_index, run) in runs.iter().enumerate() {
            let results = run
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let rule_roles = opengrep_rule_roles(run, manifest, u32_index(run_index, "run_index")?);
            for (result_index, result) in results.iter().enumerate() {
                normalize_result(
                    u32_index(run_index, "run_index")?,
                    u32_index(result_index, "result_index")?,
                    result,
                    run,
                    &rule_roles,
                    manifest,
                    &mut out,
                )?;
            }
        }
        out.capabilities = sarif_capabilities(&out, manifest);
        Ok(out)
    }
}

fn sarif_capabilities(
    out: &NormalizedArtifact,
    manifest: &ArtifactManifest,
) -> Vec<CapabilityObservation> {
    let observed_or_unknown = |observed: bool| {
        if observed {
            CapabilityOutcome::Observed
        } else {
            CapabilityOutcome::Unknown
        }
    };
    let mut capabilities = vec![
        CapabilityObservation {
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            capability: EvidenceCapability::FindingDetection,
            outcome: observed_or_unknown(!out.findings.is_empty()),
            basis: "logic.sarif21.adapter.v1".into(),
            provider_pointer: Some("/runs".into()),
            limitation: Some(
                "zero results is not negative evidence unless producer coverage is explicit".into(),
            ),
        },
        CapabilityObservation {
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            capability: EvidenceCapability::OrderedCodeFlow,
            outcome: observed_or_unknown(!out.traces.is_empty()),
            basis: "logic.sarif21.adapter.v1".into(),
            provider_pointer: Some("/runs/*/results/*/codeFlows".into()),
            limitation: Some(
                "SARIF permits traces but does not require a producer to emit them".into(),
            ),
        },
        CapabilityObservation {
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            capability: EvidenceCapability::StaticCallReachability,
            outcome: CapabilityOutcome::NotEvaluated,
            basis: "logic.sarif21.adapter.v1".into(),
            provider_pointer: None,
            limitation: Some("artifact normalization does not run graph traversal".into()),
        },
    ];
    for capability in [
        EvidenceCapability::RuntimeExecutionReachability,
        EvidenceCapability::Exploitability,
        EvidenceCapability::DeploymentState,
        EvidenceCapability::RuntimeExposure,
        EvidenceCapability::Ownership,
        EvidenceCapability::RemediationVerification,
    ] {
        capabilities.push(CapabilityObservation {
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            capability,
            outcome: CapabilityOutcome::RequiresExternalEvidence,
            basis: "logic.static_evidence_boundary.v1".into(),
            provider_pointer: None,
            limitation: Some("cannot be established from a static SARIF artifact alone".into()),
        });
    }
    capabilities
}

fn u32_index(value: usize, label: &str) -> Result<u32, ArtifactError> {
    u32::try_from(value)
        .map_err(|_| ArtifactError::Message(format!("{label} exceeds u32 protocol index")))
}

fn parse_sarif_document(bytes: &[u8]) -> Result<Value, ArtifactError> {
    let text = decode_text_bytes(bytes)?;
    let doc: Value = serde_json::from_str(&text)?;
    if doc.get("runs").is_none() {
        return Err(ArtifactError::Message(
            "not a SARIF document: missing runs".into(),
        ));
    }
    Ok(doc)
}

fn normalize_result(
    run_index: u32,
    result_index: u32,
    result: &Value,
    run: &Value,
    rule_roles: &HashMap<String, (String, String)>,
    manifest: &ArtifactManifest,
    out: &mut NormalizedArtifact,
) -> Result<(), ArtifactError> {
    let artifact_id = manifest.artifact_id.as_str();
    let rule_id = result
        .get("ruleId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rule_role = rule_id.as_ref().and_then(|rule_id| rule_roles.get(rule_id));
    let level = result
        .get("level")
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = result
        .pointer("/message/text")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut fingerprints = BTreeMap::new();
    if let Some(obj) = result.get("fingerprints").and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                fingerprints.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(obj) = result.get("partialFingerprints").and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                fingerprints.insert(format!("partial:{k}"), s.to_string());
            }
        }
    }
    let fingerprint_or_result = if fingerprints.is_empty() {
        format!("result:{result_index}")
    } else {
        // Include result_index so duplicate provider fingerprints in one run remain unique.
        let fp = fingerprints
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        format!("{fp}|result:{result_index}")
    };
    let observation_id = finding_observation_id(artifact_id, run_index, &fingerprint_or_result);
    let suppressions = parse_suppressions(result);

    let mut primary_location_ids = Vec::new();
    let locations = result
        .get("locations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (loc_index, location) in locations.iter().enumerate() {
        let pointer = format!("/runs/{run_index}/results/{result_index}/locations/{loc_index}");
        match extract_location(artifact_id, location, run, &pointer) {
            Ok(loc) => {
                if loc.extent.is_file() {
                    out.diagnostics.push(
                        ArtifactDiagnostic::warning(
                            "sarif.primary_location_missing_region",
                            "primary physical location lacks required SARIF region; retained as file-grain location",
                        )
                        .with_artifact(artifact_id)
                        .with_pointer(&pointer),
                    );
                }
                if !manifest.path_in_scope(&loc.path)? {
                    out.diagnostics.push(ArtifactDiagnostic {
                        code: "artifact.scope_violation".into(),
                        severity: DiagnosticSeverity::Info,
                        message: format!("location excluded by scope: {}", loc.path),
                        artifact_id: Some(artifact_id.into()),
                        provider_pointer: Some(pointer),
                    });
                }
                primary_location_ids.push(loc.location_id.clone());
                if let Some((role, provider_pointer)) = rule_role {
                    out.provider_role_hints.push(ProviderRoleHint {
                        finding_observation_id: observation_id.clone(),
                        location_id: loc.location_id.clone(),
                        role: role.clone(),
                        provider_pointer: provider_pointer.clone(),
                        policy_id: OPENGREP_RULE_ROLE_POLICY.into(),
                    });
                }
                out.locations.push(loc);
            }
            Err(diag) => out.diagnostics.push(diag.with_artifact(artifact_id)),
        }
    }

    let mut related_location_ids = Vec::new();
    if let Some(related) = result.get("relatedLocations").and_then(Value::as_array) {
        for (index, location) in related.iter().enumerate() {
            let pointer =
                format!("/runs/{run_index}/results/{result_index}/relatedLocations/{index}");
            match extract_location(artifact_id, location, run, &pointer) {
                Ok(loc) => {
                    if loc.extent.is_file() {
                        out.diagnostics.push(
                            ArtifactDiagnostic::warning(
                                "sarif.location_missing_region",
                                "related physical location lacks region; retained as file-grain location",
                            )
                            .with_artifact(artifact_id)
                            .with_pointer(&pointer),
                        );
                    }
                    if !manifest.path_in_scope(&loc.path)? {
                        out.diagnostics.push(ArtifactDiagnostic {
                            code: "artifact.scope_violation".into(),
                            severity: DiagnosticSeverity::Info,
                            message: format!("related location excluded by scope: {}", loc.path),
                            artifact_id: Some(artifact_id.into()),
                            provider_pointer: Some(pointer),
                        });
                    }
                    related_location_ids.push(loc.location_id.clone());
                    out.locations.push(loc);
                }
                Err(diag) => out.diagnostics.push(diag.with_artifact(artifact_id)),
            }
        }
    }

    let logical_location_ids = result
        .get("locations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .flat_map(|(location_index, location)| {
            location
                .get("logicalLocations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .map(move |(logical_index, _)| {
                    format!("/runs/{run_index}/results/{result_index}/locations/{location_index}/logicalLocations/{logical_index}")
                })
        })
        .collect();
    let taxonomies = result
        .get("taxa")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|taxon| entmoot_contracts::TaxonomyRef {
            taxonomy: taxon
                .get("toolComponent")
                .and_then(|v| v.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("sarif_taxa")
                .to_string(),
            taxon: taxon.get("id").and_then(Value::as_str).map(str::to_string),
        })
        .collect();
    let fix_pointers = result
        .get("fixes")
        .and_then(Value::as_array)
        .map(|fixes| {
            (0..fixes.len())
                .map(|i| format!("/runs/{run_index}/results/{result_index}/fixes/{i}"))
                .collect()
        })
        .unwrap_or_default();
    let rule_metadata_pointer = result
        .get("ruleIndex")
        .or_else(|| result.get("ruleId"))
        .map(|_| format!("/runs/{run_index}/tool/driver/rules"));

    let mut trace_ids = Vec::new();
    let code_flows = result
        .get("codeFlows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (cf_index, code_flow) in code_flows.iter().enumerate() {
        let threads = code_flow
            .get("threadFlows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (thread_index, thread) in threads.iter().enumerate() {
            let tid = make_trace_id(
                &observation_id,
                u32_index(cf_index, "code_flow_index")?,
                u32_index(thread_index, "thread_index")?,
            );
            let mut step_ids = Vec::new();
            let steps = thread
                .get("locations")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for (ordinal, step) in steps.iter().enumerate() {
                let pointer = format!(
                    "/runs/{run_index}/results/{result_index}/codeFlows/{cf_index}/threadFlows/{thread_index}/locations/{ordinal}"
                );
                let loc_value = step.get("location").unwrap_or(step);
                match extract_location(artifact_id, loc_value, run, &pointer) {
                    Ok(loc) => {
                        if loc.extent.is_file() {
                            out.diagnostics.push(
                                ArtifactDiagnostic::warning(
                                    "sarif.location_missing_region",
                                    "trace step physical location lacks region; retained as file-grain location",
                                )
                                .with_artifact(artifact_id)
                                .with_pointer(&pointer),
                            );
                        }
                        if !manifest.path_in_scope(&loc.path)? {
                            out.diagnostics.push(ArtifactDiagnostic {
                                code: "artifact.scope_violation".into(),
                                severity: DiagnosticSeverity::Info,
                                message: format!("trace step excluded by scope: {}", loc.path),
                                artifact_id: Some(artifact_id.into()),
                                provider_pointer: Some(pointer.clone()),
                            });
                        }
                        let sid = make_step_id(&tid, u32_index(ordinal, "step_ordinal")?);
                        let importance = step
                            .get("importance")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let provider_message = step
                            .pointer("/location/message/text")
                            .or_else(|| step.pointer("/message/text"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let provider_kinds = step
                            .get("kinds")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        step_ids.push(sid.clone());
                        out.trace_steps.push(TraceStepRecord {
                            step_id: sid,
                            trace_id: tid.clone(),
                            ordinal: u32_index(ordinal, "step_ordinal")?,
                            location_id: loc.location_id.clone(),
                            provider_importance: importance,
                            provider_message,
                            provider_kinds,
                        });
                        if !out
                            .locations
                            .iter()
                            .any(|l| l.location_id == loc.location_id)
                        {
                            out.locations.push(loc);
                        }
                    }
                    Err(diag) => out.diagnostics.push(diag.with_artifact(artifact_id)),
                }
            }
            if !step_ids.is_empty() {
                trace_ids.push(tid.clone());
                out.traces.push(TraceRecord {
                    trace_id: tid,
                    finding_observation_id: observation_id.clone(),
                    code_flow_index: u32_index(cf_index, "code_flow_index")?,
                    thread_index: u32_index(thread_index, "thread_index")?,
                    step_ids,
                });
            }
        }
    }

    out.findings.push(FindingObservation {
        observation_id,
        artifact_id: artifact_id.into(),
        run_index,
        result_index,
        rule_id,
        level,
        message,
        fingerprints,
        taxonomies,
        primary_location_ids,
        related_location_ids,
        logical_location_ids,
        trace_ids,
        suppressions,
        baseline_state: result
            .get("baselineState")
            .and_then(Value::as_str)
            .map(str::to_string),
        fix_pointers,
        rule_metadata_pointer,
    });
    Ok(())
}

fn opengrep_rule_roles(
    run: &Value,
    manifest: &ArtifactManifest,
    run_index: u32,
) -> HashMap<String, (String, String)> {
    if !manifest.producer.name.eq_ignore_ascii_case("opengrep oss")
        || !run
            .pointer("/tool/driver/name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case("opengrep oss"))
    {
        return HashMap::new();
    }
    run.pointer("/tool/driver/rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, descriptor)| {
            let id = descriptor.get("id")?.as_str()?;
            let (role, property_pointer) = descriptor
                .pointer("/properties/finding_type")
                .and_then(Value::as_str)
                .map(|role| (role, "finding_type".to_owned()))
                .or_else(|| {
                    descriptor
                        .pointer("/properties/tags")
                        .and_then(Value::as_array)?
                        .iter()
                        .enumerate()
                        .find_map(|(tag_index, tag)| {
                            tag.as_str()?
                                .strip_prefix("entmoot.role:")
                                .map(|role| (role, format!("tags/{tag_index}")))
                        })
                })?;
            crate::assertions::map_provider_role_kind(role)?;
            Some((
                id.to_owned(),
                (
                    role.to_owned(),
                    format!(
                        "/runs/{run_index}/tool/driver/rules/{index}/properties/{property_pointer}"
                    ),
                ),
            ))
        })
        .collect()
}

fn parse_suppressions(result: &Value) -> Vec<SuppressionDetail> {
    let Some(arr) = result.get("suppressions").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .map(|s| SuppressionDetail {
            kind: s.get("kind").and_then(Value::as_str).map(str::to_string),
            status: s.get("status").and_then(Value::as_str).map(str::to_string),
            justification: s
                .get("justification")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
        .collect()
}

fn extract_location(
    artifact_id: &str,
    location: &Value,
    run: &Value,
    pointer: &str,
) -> Result<LocationRecord, ArtifactDiagnostic> {
    let physical = location.get("physicalLocation").unwrap_or(location);
    let artifact_loc = physical
        .get("artifactLocation")
        .cloned()
        .unwrap_or(Value::Null);
    let uri = artifact_loc
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ArtifactDiagnostic::error("sarif.invalid_region", "missing artifactLocation.uri")
                .with_pointer(pointer)
        })?;
    let uri_base_id = artifact_loc.get("uriBaseId").and_then(Value::as_str);
    let resolved = resolve_uri(
        uri,
        uri_base_id,
        run.get("originalUriBaseIds").unwrap_or(&Value::Null),
    )
    .map_err(|msg| {
        ArtifactDiagnostic::error("sarif.unresolved_uri_base", msg).with_pointer(pointer)
    })?;
    let path = normalize_repo_path(&normalize_sarif_uri(&resolved)).map_err(|e| {
        ArtifactDiagnostic::error("sarif.invalid_region", e.to_string()).with_pointer(pointer)
    })?;
    let region = physical.get("region");
    let coordinate = |name: &str| -> Result<Option<u32>, ArtifactDiagnostic> {
        let Some(value) = region.and_then(|region| region.get(name)) else {
            return Ok(None);
        };
        value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| {
                ArtifactDiagnostic::error(
                    "sarif.invalid_region",
                    format!("{name} must be a positive u32"),
                )
                .with_pointer(pointer)
            })
    };
    let start_line = coordinate("startLine")?;
    if region.is_some() && start_line.is_none() {
        return Err(ArtifactDiagnostic::error("sarif.unsupported_region", "present region must supply supported line coordinates; offset-only or malformed regions are not file-grain locations").with_pointer(pointer));
    }
    // This lane supports standard UTF-8 text with the SARIF default newlines.
    // Never silently ignore a second coordinate system or a declared text policy.
    if region.is_some() {
        if region.is_some_and(|region| {
            ["byteOffset", "byteLength", "charOffset", "charLength"]
                .iter()
                .any(|key| region.get(key).is_some())
        }) {
            return Err(ArtifactDiagnostic::error(
                "sarif.unsupported_region",
                "offset coordinates require independent validation against the text region",
            )
            .with_pointer(pointer));
        }
        if run
            .get("newlineSequences")
            .is_some_and(|value| *value != serde_json::json!(["\r\n", "\n"]))
        {
            return Err(ArtifactDiagnostic::error(
                "sarif.unsupported_newlines",
                "non-default newline sequences are not qualified for source attachment",
            )
            .with_pointer(pointer));
        }
        let artifact = run
            .get("artifacts")
            .and_then(Value::as_array)
            .and_then(|artifacts| {
                artifact_loc
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| artifacts.get(index))
                    .or_else(|| {
                        artifacts.iter().find(|artifact| {
                            artifact.pointer("/location/uri").and_then(Value::as_str) == Some(uri)
                        })
                    })
            });
        if artifact
            .and_then(|artifact| artifact.get("encoding"))
            .or_else(|| run.get("defaultEncoding"))
            .is_some_and(|value| {
                !value
                    .as_str()
                    .is_some_and(|encoding| encoding.eq_ignore_ascii_case("utf-8"))
            })
        {
            return Err(ArtifactDiagnostic::error("sarif.unsupported_encoding", "declared source encoding is not UTF-8; retain provider bytes without deriving a source span").with_pointer(pointer));
        }
    }
    // Never copy Semgrep/OpenGrep rule confidence — only a location-level native field.
    let provider_confidence = physical
        .get("properties")
        .and_then(|p| p.get("confidence"))
        .and_then(Value::as_str)
        .map(str::to_string);

    match start_line {
        None => {
            let location_id = file_location_id(artifact_id, &path, pointer);
            Ok(LocationRecord {
                location_id,
                artifact_id: artifact_id.into(),
                path,
                extent: LocationExtent::File,
                provider_pointer: pointer.into(),
                provider_confidence,
            })
        }
        Some(0) => Err(ArtifactDiagnostic::error(
            "sarif.invalid_region",
            "region startLine must be >= 1",
        )
        .with_pointer(pointer)),
        Some(start_line) => {
            let start_column = coordinate("startColumn")?;
            let end_line = coordinate("endLine")?;
            let end_column = coordinate("endColumn")?;
            if end_line.unwrap_or(start_line) == start_line
                && end_column.is_some_and(|end| end < start_column.unwrap_or(1))
                || start_column == Some(0)
                || end_line == Some(0)
                || end_column == Some(0)
                || matches!((start_line, end_line), (start, Some(end)) if end < start)
            {
                return Err(ArtifactDiagnostic::error(
                    "sarif.invalid_region",
                    "malformed physical location region",
                )
                .with_pointer(pointer));
            }
            let location_id = make_location_id(
                artifact_id,
                &path,
                start_line,
                start_column,
                end_line,
                end_column,
                pointer,
            );
            let encoding = match run.get("columnKind") {
                None => entmoot_contracts::ColumnEncoding::Unknown,
                Some(Value::String(kind)) if kind == "utf16CodeUnits" => {
                    entmoot_contracts::ColumnEncoding::Utf16CodeUnits
                }
                Some(Value::String(kind)) if kind == "unicodeCodePoints" => {
                    entmoot_contracts::ColumnEncoding::Utf32CodePoints
                }
                Some(_) => {
                    return Err(ArtifactDiagnostic::error(
                        "sarif.invalid_column_kind",
                        "columnKind must be utf16CodeUnits or unicodeCodePoints",
                    )
                    .with_pointer(pointer))
                }
            };
            let end_line = end_line.unwrap_or(start_line);
            let end = match end_column {
                Some(column) => entmoot_contracts::SourceTextEnd::Position {
                    position: entmoot_contracts::SourcePosition {
                        line: end_line,
                        column,
                    },
                },
                None => entmoot_contracts::SourceTextEnd::LineEnd { line: end_line },
            };
            Ok(LocationRecord {
                location_id,
                artifact_id: artifact_id.into(),
                path,
                extent: LocationExtent::Region {
                    region: SourceRegion::Text {
                        range: entmoot_contracts::SourceTextRange {
                            start: entmoot_contracts::SourcePosition {
                                line: start_line,
                                column: start_column.unwrap_or(1),
                            },
                            end,
                        },
                        encoding,
                    },
                },
                provider_pointer: pointer.into(),
                provider_confidence,
            })
        }
    }
}

fn resolve_uri(uri: &str, uri_base_id: Option<&str>, uri_bases: &Value) -> Result<String, String> {
    if uri.contains("://") || uri.starts_with('/') {
        return Ok(uri.to_string());
    }
    let Some(base_id) = uri_base_id else {
        return Ok(uri.to_string());
    };
    if let Some(base) = uri_bases
        .get(base_id)
        .and_then(|v| v.get("uri").and_then(Value::as_str).or(v.as_str()))
    {
        let base = base.trim_end_matches('/');
        return Ok(format!("{base}/{uri}"));
    }
    // Common scanner placeholders meaning "repository root" when originalUriBaseIds
    // omits an explicit mapping (Snyk Code / OpenGrep).
    if base_id.eq_ignore_ascii_case("%SRCROOT%") || base_id.eq_ignore_ascii_case("SRCROOT") {
        return Ok(uri.to_string());
    }
    Err(format!("unresolved uriBaseId: {base_id}"))
}

fn normalize_sarif_uri(uri: &str) -> String {
    let stripped = uri
        .strip_prefix("file:///")
        .or_else(|| uri.strip_prefix("file://"))
        .unwrap_or(uri);
    // Drop Windows drive if present after file:// — keep relative when possible.
    let mut path = stripped.to_string(); // keep percent-decoding minimal
    path = path.replace("%20", " ").replace('\\', "/");
    if path.len() >= 3 && path.as_bytes()[1] == b':' {
        // file:///C:/repo/src → drop drive, keep path after first segment if it looks absolute
        if let Some(idx) = path.find('/') {
            // C:/foo/bar → try to keep as-is for normalize_repo_path rejection, or strip to relative
            // Prefer last-resort: if path has multiple segments, return as-is for validation.
            let _ = idx;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::test_manifest::test_manifest;
    use entmoot_contracts::{blake3_hex, ArtifactManifest};
    use std::path::PathBuf;

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/entmoot-contracts/tests/fixtures/artifacts/sarif")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    fn sarif_manifest(bytes: &[u8], producer: &str) -> ArtifactManifest {
        test_manifest(
            bytes,
            ArtifactFormat::Sarif21,
            producer,
            Some("test"),
            vec!["target/**"],
            vec![],
        )
    }

    #[test]
    fn malformed_or_unsupported_regions_never_become_file_grain() {
        for region in [
            serde_json::json!({"startLine": "1"}),
            serde_json::json!({"startLine": 4294967296_u64}),
            serde_json::json!({"startLine": 1, "startColumn": 9, "endColumn": 2}),
            serde_json::json!({"byteOffset": 0}),
            serde_json::json!({}),
            serde_json::Value::Null,
        ] {
            let location = serde_json::json!({"physicalLocation": {"artifactLocation": {"uri": "a.py"}, "region": region}});
            assert!(
                extract_location("artifact", &location, &serde_json::Value::Null, "/location")
                    .is_err()
            );
        }
    }

    #[test]
    fn sarif_defaults_are_explicit_and_unsupported_coordinate_policies_do_not_disappear() {
        use entmoot_contracts::{ColumnEncoding, SourcePosition, SourceTextEnd, SourceTextRange};
        let mut location = serde_json::json!({"physicalLocation":{"artifactLocation":{"uri":"a.py"},"region":{"startLine":2,"startColumn":4,"endColumn":6}}});
        for (run, encoding) in [
            (
                serde_json::json!({"columnKind":"utf16CodeUnits"}),
                ColumnEncoding::Utf16CodeUnits,
            ),
            (
                serde_json::json!({"columnKind":"unicodeCodePoints"}),
                ColumnEncoding::Utf32CodePoints,
            ),
            (serde_json::json!({}), ColumnEncoding::Unknown),
        ] {
            let normalized = extract_location("artifact", &location, &run, "/location").unwrap();
            assert_eq!(
                normalized.extent.region(),
                Some(&SourceRegion::Text {
                    range: SourceTextRange {
                        start: SourcePosition { line: 2, column: 4 },
                        end: SourceTextEnd::Position {
                            position: SourcePosition { line: 2, column: 6 }
                        }
                    },
                    encoding,
                })
            );
        }
        location["physicalLocation"]["region"] = serde_json::json!({"startLine":2});
        let normalized =
            extract_location("artifact", &location, &Value::Null, "/location").unwrap();
        assert_eq!(
            normalized.extent.region(),
            Some(&SourceRegion::Text {
                range: SourceTextRange {
                    start: SourcePosition { line: 2, column: 1 },
                    end: SourceTextEnd::LineEnd { line: 2 }
                },
                encoding: ColumnEncoding::Unknown,
            })
        );
        for run in [
            serde_json::json!({"columnKind":"utf8CodeUnits"}),
            serde_json::json!({"columnKind":null}),
            serde_json::json!({"newlineSequences":["\r"]}),
            serde_json::json!({"defaultEncoding":"utf-16"}),
            serde_json::json!({"defaultEncoding":"utf-8","artifacts":[{"location":{"uri":"a.py"},"encoding":"utf-16"}]}),
        ] {
            assert!(
                extract_location("artifact", &location, &run, "/location").is_err(),
                "{run}"
            );
        }
        for field in ["byteOffset", "byteLength", "charOffset", "charLength"] {
            let mut competing = location.clone();
            competing["physicalLocation"]["region"][field] = 0.into();
            assert!(extract_location("artifact", &competing, &Value::Null, "/location").is_err());
        }
        let run = serde_json::json!({"defaultEncoding":"UTF-8","columnKind":"utf16CodeUnits","newlineSequences":["\r\n","\n"]});
        assert!(extract_location("artifact", &location, &run, "/location").is_ok());
    }

    #[test]
    fn snyk_fixture_retains_results_and_traces() {
        let bytes = fixture_bytes("snyk-code.min.sarif");
        let manifest = sarif_manifest(&bytes, "SnykCode");
        let inspected = SarifAdapter.inspect(&bytes).unwrap();
        assert_eq!(inspected.finding_count, 2);
        assert_eq!(inspected.trace_count, 2);
        let norm = SarifAdapter.normalize(&bytes, &manifest).unwrap();
        assert_eq!(norm.findings.len(), 2);
        // Exclusion affects attachment, never preservation of provider trace facts.
        assert_eq!(norm.traces.len(), 2);
        assert!(norm.trace_steps.len() >= 2);
        let excluded = norm
            .diagnostics
            .iter()
            .filter(|d| d.code == "artifact.scope_violation")
            .count();
        assert!(
            excluded >= 1,
            "expected target/** exclusions, got {excluded}"
        );
        assert!(norm
            .locations
            .iter()
            .any(|location| location.path.starts_with("target/")));
        for trace in &norm.traces {
            let steps: Vec<_> = norm
                .trace_steps
                .iter()
                .filter(|s| s.trace_id == trace.trace_id)
                .collect();
            assert_eq!(steps.len(), trace.step_ids.len());
            for (i, step) in steps.iter().enumerate() {
                assert_eq!(step.ordinal as usize, i);
            }
        }
        assert!(norm
            .findings
            .iter()
            .any(|f| !f.primary_location_ids.is_empty()));
        assert!(norm.capabilities.iter().any(|c| {
            c.capability == EvidenceCapability::Exploitability
                && c.outcome == CapabilityOutcome::RequiresExternalEvidence
        }));
    }

    #[test]
    fn opengrep_fixture_retains_primaries() {
        let bytes = fixture_bytes("opengrep.min.sarif");
        let manifest = sarif_manifest(&bytes, "Opengrep OSS");
        let norm = SarifAdapter.normalize(&bytes, &manifest).unwrap();
        assert_eq!(norm.findings.len(), 2);
        assert_eq!(
            norm.findings
                .iter()
                .map(|f| f.primary_location_ids.len())
                .sum::<usize>(),
            2
        );
        assert!(norm.traces.is_empty());
    }

    #[test]
    fn opengrep_explicit_rule_role_becomes_an_attachment_hint_only() {
        let bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"Opengrep OSS","rules":[{"id":"visible-sql","properties":{"finding_type":"sink"}}]}},"results":[{"ruleId":"visible-sql","message":{"text":"visible SQL call"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"app.py"},"region":{"startLine":1,"startColumn":1}}}]}]}]}"#;
        let manifest = sarif_manifest(bytes, "Opengrep OSS");
        let normalized = SarifAdapter.normalize(bytes, &manifest).unwrap();
        assert_eq!(normalized.provider_role_hints.len(), 1);
        let hint = &normalized.provider_role_hints[0];
        assert_eq!(hint.role, "sink");
        assert_eq!(hint.policy_id, OPENGREP_RULE_ROLE_POLICY);
        assert!(hint.provider_pointer.ends_with("/properties/finding_type"));

        let other_manifest = sarif_manifest(bytes, "AnotherScanner");
        let other = SarifAdapter.normalize(bytes, &other_manifest).unwrap();
        assert!(other.provider_role_hints.is_empty());

        let tag_bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"Opengrep OSS","rules":[{"id":"visible-sql","properties":{"tags":["entmoot.role:sink"]}}]}},"results":[{"ruleId":"visible-sql","message":{"text":"visible SQL call"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"app.py"},"region":{"startLine":1,"startColumn":1}}}]}]}]}"#;
        let tag_manifest = sarif_manifest(tag_bytes, "Opengrep OSS");
        let tagged = SarifAdapter.normalize(tag_bytes, &tag_manifest).unwrap();
        assert_eq!(tagged.provider_role_hints[0].role, "sink");
        assert!(tagged.provider_role_hints[0]
            .provider_pointer
            .ends_with("/properties/tags/0"));
    }

    #[test]
    fn srcroot_uri_base_without_mapping_is_repo_relative() {
        let bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"Opengrep OSS"}},"results":[{"ruleId":"r","message":{"text":"x"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"lib/a.ts","uriBaseId":"%SRCROOT%"},"region":{"startLine":1,"startColumn":1}}}]}]}]}"#;
        let manifest = sarif_manifest(bytes, "Opengrep OSS");
        let norm = SarifAdapter.normalize(bytes, &manifest).unwrap();
        assert_eq!(norm.locations.len(), 1);
        assert_eq!(norm.locations[0].path, "lib/a.ts");
        assert!(norm
            .diagnostics
            .iter()
            .all(|d| d.code != "sarif.unresolved_uri_base"));
    }

    #[test]
    fn dense_fixture_preserves_queryable_provider_features() {
        let bytes = fixture_bytes("dense-features.min.sarif");
        let manifest = sarif_manifest(&bytes, "FixtureScanner");
        let norm = SarifAdapter.normalize(&bytes, &manifest).unwrap();
        assert_eq!(norm.findings.len(), 1);
        let finding = &norm.findings[0];
        assert_eq!(finding.baseline_state.as_deref(), Some("new"));
        assert_eq!(finding.related_location_ids.len(), 1);
        assert_eq!(finding.logical_location_ids.len(), 1);
        assert_eq!(finding.fix_pointers.len(), 1);
        assert_eq!(finding.taxonomies[0].taxon.as_deref(), Some("CWE-79"));
        assert_eq!(finding.suppressions[0].status.as_deref(), Some("accepted"));
        assert_eq!(norm.traces.len(), 1);
        assert_eq!(norm.trace_steps.len(), 2);
        assert!(norm
            .locations
            .iter()
            .all(|l| matches!(l.extent.region(), Some(SourceRegion::Text { .. }))));
    }

    #[test]
    fn empty_results_remain_unknown_without_coverage_evidence() {
        let bytes =
            br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"empty"}},"results":[]}]}"#;
        let manifest = sarif_manifest(bytes, "empty");
        let norm = SarifAdapter.normalize(bytes, &manifest).unwrap();
        assert!(norm
            .capabilities
            .iter()
            .any(|c| c.capability == EvidenceCapability::FindingDetection
                && c.outcome == CapabilityOutcome::Unknown));
    }

    #[test]
    fn target_path_excluded_by_scope() {
        let bytes = fixture_bytes("snyk-code.min.sarif");
        let manifest = sarif_manifest(&bytes, "SnykCode");
        // Fixture includes one result under target/
        let norm = SarifAdapter.normalize(&bytes, &manifest).unwrap();
        let excluded = norm
            .diagnostics
            .iter()
            .filter(|d| d.code == "artifact.scope_violation")
            .count();
        assert!(excluded >= 1, "expected scope exclusion diagnostics");
    }

    #[test]
    fn utf16_le_sarif_matches_utf8() {
        let utf8 = fixture_bytes("opengrep.min.sarif");
        let mut utf16 = vec![0xff, 0xfe];
        for unit in String::from_utf8(utf8.clone()).unwrap().encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let m8 = sarif_manifest(&utf8, "Opengrep OSS");
        let m16 = sarif_manifest(&utf16, "Opengrep OSS");
        let n8 = SarifAdapter.normalize(&utf8, &m8).unwrap();
        let n16 = SarifAdapter.normalize(&utf16, &m16).unwrap();
        assert_eq!(n8.findings.len(), n16.findings.len());
        assert_eq!(n8.locations.len(), n16.locations.len());
        for (a, b) in n8.locations.iter().zip(n16.locations.iter()) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.extent, b.extent);
        }
    }

    fn inline_sarif_manifest(bytes: &[u8], included: Vec<&str>) -> ArtifactManifest {
        test_manifest(
            bytes,
            ArtifactFormat::Sarif21,
            "fixture",
            Some("test"),
            included,
            vec![],
        )
    }

    #[test]
    fn file_only_primary_location_is_retained_with_conformance_warning() {
        let bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[{"ruleId":"R1","message":{"text":"lock"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"poetry.lock"}}}]}]}]}"#;
        let manifest = inline_sarif_manifest(bytes, vec!["poetry.lock"]);
        let norm = SarifAdapter.normalize(bytes, &manifest).unwrap();
        assert_eq!(norm.locations.len(), 1);
        assert!(norm.locations[0].extent.is_file());
        assert!(!norm.findings[0].primary_location_ids.is_empty());
        assert!(norm.diagnostics.iter().any(|d| {
            d.code == "sarif.primary_location_missing_region"
                && d.severity == DiagnosticSeverity::Warning
        }));
        assert!(!norm
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error));
    }

    #[test]
    fn malformed_start_line_zero_is_error_not_file_grain() {
        let bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[{"ruleId":"R1","message":{"text":"x"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/a.rs"},"region":{"startLine":0}}}]}]}]}"#;
        let manifest = inline_sarif_manifest(bytes, vec!["src/a.rs"]);
        let norm = SarifAdapter.normalize(bytes, &manifest).unwrap();
        assert!(norm.locations.is_empty());
        assert!(norm.diagnostics.iter().any(|d| {
            d.code == "sarif.invalid_region" && d.severity == DiagnosticSeverity::Error
        }));
    }

    #[test]
    fn mixed_file_only_and_region_locations_do_not_error() {
        let bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[{"ruleId":"R1","message":{"text":"a"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/a.rs"},"region":{"startLine":1}}}]},{"ruleId":"R2","message":{"text":"b"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"poetry.lock"}}}]}]}]}"#;
        let manifest = inline_sarif_manifest(bytes, vec!["src/a.rs", "poetry.lock"]);
        let norm = SarifAdapter.normalize(bytes, &manifest).unwrap();
        assert_eq!(norm.locations.len(), 2);
        assert_eq!(norm.findings.len(), 2);
        assert!(norm.locations.iter().any(|l| l.extent.is_file()));
        assert!(norm.locations.iter().any(|l| l.extent.region().is_some()));
        assert!(!norm
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error));
    }

    #[test]
    fn digest_mismatch_fails_before_normalize() {
        let bytes = fixture_bytes("opengrep.min.sarif");
        let mut manifest = sarif_manifest(&bytes, "Opengrep OSS");
        manifest.content_digest = blake3_hex(b"wrong");
        let (id, series) = ArtifactManifest::derive_ids(
            manifest.format,
            &manifest.producer,
            &manifest.snapshot,
            &manifest.scope,
            &manifest.content_digest,
            manifest.config_digest.as_deref(),
        );
        manifest.artifact_id = id;
        manifest.artifact_series_id = series;
        let err = SarifAdapter.normalize(&bytes, &manifest).unwrap_err();
        assert!(err.to_string().contains("digest mismatch") || err.to_string().contains("Digest"));
    }
}
