//! Derive Wave B/C occurrence-scoped assertions from attachments (no invented taint roles).

use entmoot_contracts::{
    assertion_id, Assertion, AssertionObject, ClaimScope, DerivationRecord, EntityKind, EntityRef,
    EpistemicStatus, EvidenceRef, FindingObservation, LocationAttachment, TraceStepRecord,
};

use crate::artifact::ProviderRoleHint;
use crate::graph::TopologyGraph;

/// Exact-token map from SARIF `threadFlowLocation.kinds` (and peers) to role assertions.
pub const PROVIDER_ROLE_POLICY: &str = "logic.assertion.provider_role.v1";

pub const PREDICATE_FINDING_LOCATION: &str = "entmoot.finding_location";
pub const PREDICATE_TRACE_STEP: &str = "entmoot.trace_step";
pub const PREDICATE_PROVIDER_ROLE: &str = "entmoot.provider_role";

/// Map a provider-stated kind token to a normalized role slug. Case-insensitive exact match only.
pub fn map_provider_role_kind(token: &str) -> Option<&'static str> {
    match token.trim().to_ascii_lowercase().as_str() {
        "source" => Some("source"),
        "sink" => Some("sink"),
        "propagator" => Some("propagator"),
        "sanitizer" => Some("sanitizer"),
        _ => None,
    }
}

fn occurrence_subject(location_id: &str) -> EntityRef {
    EntityRef {
        entity_id: location_id.to_owned(),
        entity_kind: EntityKind::Opaque,
    }
}

fn evidence(
    artifact_id: &str,
    provider_pointer: impl Into<String>,
    record_id: Option<String>,
) -> EvidenceRef {
    EvidenceRef {
        artifact_id: artifact_id.to_owned(),
        provider_pointer: provider_pointer.into(),
        record_id,
    }
}

/// Emit finding_location / trace_step assertions for newly attached locations.
/// Emits provider_role only when `provider_kinds` states a recognized token.
pub fn derive_attachment_assertions(
    graph: &TopologyGraph,
    new_attachments: &[LocationAttachment],
    findings: &[FindingObservation],
    trace_steps: &[TraceStepRecord],
) -> Vec<Assertion> {
    let attachment_by_location: std::collections::HashMap<&str, &LocationAttachment> =
        new_attachments
            .iter()
            .map(|a| (a.location_id.as_str(), a))
            .collect();

    let mut assertions = Vec::new();
    for finding in findings {
        for location_id in &finding.primary_location_ids {
            let Some(attachment) = attachment_by_location.get(location_id.as_str()) else {
                continue;
            };
            assertions.push(Assertion {
                assertion_id: assertion_id(
                    location_id,
                    PREDICATE_FINDING_LOCATION,
                    &finding.observation_id,
                    &attachment.attachment_id,
                ),
                subject: occurrence_subject(location_id),
                predicate: PREDICATE_FINDING_LOCATION.into(),
                object: AssertionObject {
                    entity: Some(EntityRef {
                        entity_id: finding.observation_id.clone(),
                        entity_kind: EntityKind::Finding,
                    }),
                    value: None,
                },
                evidence_refs: vec![evidence(
                    &finding.artifact_id,
                    format!("attachment:{}", attachment.attachment_id),
                    Some(finding.observation_id.clone()),
                )],
                epistemic_status: EpistemicStatus::Observed,
                derivation: None,
                supports: vec![],
                contradicts: vec![],
                observation_context_id: None,
                valid_from: None,
                valid_to: None,
                claim_scope: Some(ClaimScope::Occurrence),
                subject_address: None,
            });
        }
    }
    for step in trace_steps {
        let Some(attachment) = attachment_by_location.get(step.location_id.as_str()) else {
            continue;
        };
        let finding_id = graph
            .traces
            .iter()
            .find(|t| t.trace_id == step.trace_id)
            .map(|t| t.finding_observation_id.clone());
        assertions.push(Assertion {
            assertion_id: assertion_id(
                &step.location_id,
                PREDICATE_TRACE_STEP,
                &step.step_id,
                &attachment.attachment_id,
            ),
            subject: occurrence_subject(&step.location_id),
            predicate: PREDICATE_TRACE_STEP.into(),
            object: AssertionObject {
                entity: finding_id.as_ref().map(|id| EntityRef {
                    entity_id: id.clone(),
                    entity_kind: EntityKind::Finding,
                }),
                value: Some(step.step_id.clone()),
            },
            evidence_refs: vec![evidence(
                &attachment.artifact_id,
                format!("trace_step:{}", step.step_id),
                Some(step.step_id.clone()),
            )],
            epistemic_status: EpistemicStatus::Observed,
            derivation: None,
            supports: vec![],
            contradicts: vec![],
            observation_context_id: None,
            valid_from: None,
            valid_to: None,
            claim_scope: Some(ClaimScope::Occurrence),
            subject_address: None,
        });

        for kind_token in &step.provider_kinds {
            let Some(role_slug) = map_provider_role_kind(kind_token) else {
                continue;
            };
            assertions.push(Assertion {
                assertion_id: assertion_id(
                    &step.location_id,
                    PREDICATE_PROVIDER_ROLE,
                    role_slug,
                    &attachment.attachment_id,
                ),
                subject: occurrence_subject(&step.location_id),
                predicate: PREDICATE_PROVIDER_ROLE.into(),
                object: AssertionObject {
                    entity: finding_id.as_ref().map(|id| EntityRef {
                        entity_id: id.clone(),
                        entity_kind: EntityKind::Finding,
                    }),
                    value: Some(kind_token.clone()),
                },
                evidence_refs: vec![evidence(
                    &attachment.artifact_id,
                    format!("trace_step:{}/kinds", step.step_id),
                    Some(step.step_id.clone()),
                )],
                epistemic_status: EpistemicStatus::DeterministicallyDerived,
                derivation: Some(DerivationRecord {
                    method_id: PROVIDER_ROLE_POLICY.into(),
                    policy_version: "1".into(),
                    input_refs: vec![evidence(
                        &attachment.artifact_id,
                        format!("trace_step:{}/kinds:{kind_token}", step.step_id),
                        Some(step.step_id.clone()),
                    )],
                    limitations: Some(
                        "Exact-token map from provider kinds; never inferred from ordinals".into(),
                    ),
                    calibration_evidence_ref: None,
                }),
                supports: vec![],
                contradicts: vec![],
                observation_context_id: None,
                valid_from: None,
                valid_to: None,
                claim_scope: Some(ClaimScope::Occurrence),
                subject_address: None,
            });
        }
    }
    assertions
}

/// Derive assertions using findings/traces already on the candidate graph after merge.
pub fn derive_assertions_for_artifact(
    graph: &TopologyGraph,
    artifact_id: &str,
    new_attachments: &[LocationAttachment],
) -> Vec<Assertion> {
    let findings: Vec<_> = graph
        .findings
        .iter()
        .filter(|f| f.artifact_id == artifact_id)
        .cloned()
        .collect();
    let trace_ids: std::collections::HashSet<_> = findings
        .iter()
        .flat_map(|f| f.trace_ids.iter().cloned())
        .collect();
    let trace_steps: Vec<_> = graph
        .trace_steps
        .iter()
        .filter(|s| trace_ids.contains(&s.trace_id))
        .cloned()
        .collect();
    derive_attachment_assertions(graph, new_attachments, &findings, &trace_steps)
}

/// Convert explicit provider rule-role metadata into occurrence-scoped
/// assertions after the referenced finding location has attached.
pub fn derive_provider_role_hint_assertions(
    artifact_id: &str,
    hints: &[ProviderRoleHint],
    attachments: &[LocationAttachment],
) -> Vec<Assertion> {
    let attachment_by_location: std::collections::HashMap<_, _> = attachments
        .iter()
        .map(|attachment| (attachment.location_id.as_str(), attachment))
        .collect();
    hints
        .iter()
        .filter_map(|hint| {
            let attachment = attachment_by_location.get(hint.location_id.as_str())?;
            Some(Assertion {
                assertion_id: assertion_id(
                    &hint.location_id,
                    PREDICATE_PROVIDER_ROLE,
                    &hint.role,
                    &format!("{}:{}", hint.policy_id, attachment.attachment_id),
                ),
                subject: occurrence_subject(&hint.location_id),
                predicate: PREDICATE_PROVIDER_ROLE.into(),
                object: AssertionObject {
                    entity: Some(EntityRef {
                        entity_id: hint.finding_observation_id.clone(),
                        entity_kind: EntityKind::Finding,
                    }),
                    value: Some(hint.role.clone()),
                },
                evidence_refs: vec![evidence(
                    artifact_id,
                    &hint.provider_pointer,
                    Some(hint.finding_observation_id.clone()),
                )],
                epistemic_status: EpistemicStatus::DeterministicallyDerived,
                derivation: Some(DerivationRecord {
                    method_id: hint.policy_id.clone(),
                    policy_version: "1".into(),
                    input_refs: vec![evidence(
                        artifact_id,
                        &hint.provider_pointer,
                        Some(hint.finding_observation_id.clone()),
                    )],
                    limitations: Some(
                        "Exact provider rule property; does not establish reachability or exploitability"
                            .into(),
                    ),
                    calibration_evidence_ref: None,
                }),
                supports: vec![],
                contradicts: vec![],
                observation_context_id: None,
                valid_from: None,
                valid_to: None,
                claim_scope: Some(ClaimScope::Occurrence),
                subject_address: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        AttachmentDisposition, AttachmentFreshness, AttachmentReason, AttachmentTarget,
        AttachmentTargetKind,
    };

    fn base_attachment(node_kind: AttachmentTargetKind) -> LocationAttachment {
        LocationAttachment {
            verified_span: None,
            attachment_id: "att1".into(),
            artifact_id: "art".into(),
            location_id: "loc1".into(),
            disposition: AttachmentDisposition::Attached,
            selected_target: Some(AttachmentTarget {
                port: None,
                kind: node_kind,
                node_id: Some("target".into()),
                address_id: None,
            }),
            candidate_targets: vec![],
            freshness: AttachmentFreshness::Current,
            reason_codes: vec![AttachmentReason::FileFallback],
            conflicts: vec![],
            location_ref: None,
        }
    }

    #[test]
    fn primary_locations_become_finding_location_not_provider_role() {
        let attachment = base_attachment(AttachmentTargetKind::File);
        let finding = FindingObservation {
            observation_id: "obs".into(),
            artifact_id: "art".into(),
            run_index: 0,
            result_index: 0,
            rule_id: Some("R".into()),
            level: Some("error".into()),
            message: Some("x".into()),
            fingerprints: Default::default(),
            taxonomies: vec![],
            primary_location_ids: vec!["loc1".into()],
            related_location_ids: vec![],
            logical_location_ids: vec![],
            trace_ids: vec![],
            suppressions: vec![],
            baseline_state: None,
            fix_pointers: vec![],
            rule_metadata_pointer: None,
        };
        let graph = TopologyGraph::new("C:/repo");
        let assertions = derive_attachment_assertions(&graph, &[attachment], &[finding], &[]);
        assert_eq!(assertions.len(), 1);
        assert_eq!(assertions[0].predicate, PREDICATE_FINDING_LOCATION);
        assert_eq!(assertions[0].claim_scope, Some(ClaimScope::Occurrence));
        assert!(!assertions
            .iter()
            .any(|a| a.predicate == PREDICATE_PROVIDER_ROLE));
    }

    #[test]
    fn provider_kinds_emit_role_assertions() {
        let attachment = base_attachment(AttachmentTargetKind::Callable);
        let step = TraceStepRecord {
            step_id: "s1".into(),
            trace_id: "t1".into(),
            ordinal: 0,
            location_id: "loc1".into(),
            provider_importance: None,
            provider_message: None,
            provider_kinds: vec!["sink".into()],
        };
        let graph = TopologyGraph::new("C:/repo");
        let assertions = derive_attachment_assertions(&graph, &[attachment], &[], &[step]);
        assert!(assertions
            .iter()
            .any(|a| a.predicate == PREDICATE_TRACE_STEP));
        let sink = assertions
            .iter()
            .find(|a| a.predicate == PREDICATE_PROVIDER_ROLE)
            .expect("provider_role assertion");
        assert_eq!(sink.object.value.as_deref(), Some("sink"));
        assert_eq!(
            sink.derivation.as_ref().map(|d| d.method_id.as_str()),
            Some(PROVIDER_ROLE_POLICY)
        );
    }

    #[test]
    fn map_provider_role_kind_exact_only() {
        assert_eq!(map_provider_role_kind("Sink"), Some("sink"));
        assert_eq!(map_provider_role_kind("source"), Some("source"));
        assert_eq!(map_provider_role_kind("important"), None);
        assert_eq!(map_provider_role_kind("passThrough"), None);
    }
}
