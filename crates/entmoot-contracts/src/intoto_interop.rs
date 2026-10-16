//! in-toto-style interchange mapping for compatible artifact-oriented claims.
//!
//! Entmoot does not adopt in-toto as ontology. This module documents field
//! alignment and provides a minimal export/import round-trip for artifact
//! digest subjects only.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::address::{Address, Subject};
use crate::assertion::{Assertion, AssertionObject, EpistemicStatus, EvidenceRef};
use crate::entity::{EntityKind, EntityRef};
use crate::foundational::ClaimScope;
use crate::ids::ContentDigest;

/// Subset of an in-toto Statement sufficient for Entmoot artifact claim interchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InTotoStyleStatement {
    #[serde(rename = "_type")]
    pub statement_type: String,
    pub subject: Vec<InTotoResourceDescriptor>,
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    pub predicate: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InTotoResourceDescriptor {
    pub name: String,
    pub digest: InTotoDigestSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InTotoDigestSet {
    /// BLAKE3 hex without prefix (in-toto digest maps typically use algorithm keys).
    pub blake3: String,
}

/// Export an artifact-digest Assertion into an in-toto-style statement.
///
/// Returns `None` when the assertion is not representable without fabricating
/// Entmoot-only topology/scope semantics as supply-chain facts.
pub fn export_artifact_assertion(assertion: &Assertion) -> Option<InTotoStyleStatement> {
    let digest = match assertion.subject_address.as_ref().map(|s| &s.address) {
        Some(Address::ExactBody { exact_body_id }) => {
            exact_body_id.as_str().strip_prefix("blake3:")?.to_string()
        }
        Some(Address::Opaque { system, value }) if system == "content_digest" => {
            ContentDigest::parse(value.clone())
                .ok()?
                .as_str()
                .strip_prefix("blake3:")?
                .to_string()
        }
        _ => return None,
    };

    let predicate = serde_json::json!({
        "entmootPredicate": assertion.predicate,
        "object": assertion.object,
        "claimScope": assertion.claim_scope,
        "epistemicStatus": assertion.epistemic_status,
        "evidenceRefs": assertion.evidence_refs,
    });

    Some(InTotoStyleStatement {
        statement_type: "https://in-toto.io/Statement/v1".into(),
        subject: vec![InTotoResourceDescriptor {
            name: assertion.assertion_id.clone(),
            digest: InTotoDigestSet { blake3: digest },
        }],
        predicate_type: format!("https://entmoot.dev/predicates/{}", assertion.predicate),
        predicate,
    })
}

/// Import a compatible in-toto-style statement back into an Assertion.
pub fn import_artifact_statement(statement: &InTotoStyleStatement) -> Result<Assertion, String> {
    if statement.subject.len() != 1 {
        return Err("expected exactly one subject".into());
    }
    let subj = &statement.subject[0];
    let digest = format!("blake3:{}", subj.digest.blake3);
    let _ = ContentDigest::parse(&digest).map_err(|e| e.to_string())?;

    let predicate = statement
        .predicate
        .get("entmootPredicate")
        .and_then(|v| v.as_str())
        .unwrap_or("entmoot.artifact.claim")
        .to_string();

    let object: AssertionObject = serde_json::from_value(
        statement
            .predicate
            .get("object")
            .cloned()
            .unwrap_or(serde_json::json!({})),
    )
    .unwrap_or(AssertionObject {
        entity: None,
        value: None,
    });

    let claim_scope = statement
        .predicate
        .get("claimScope")
        .and_then(|v| serde_json::from_value::<ClaimScope>(v.clone()).ok());

    let evidence_refs: Vec<EvidenceRef> = statement
        .predicate
        .get("evidenceRefs")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    Ok(Assertion {
        assertion_id: subj.name.clone(),
        subject: EntityRef {
            entity_id: digest.clone(),
            entity_kind: EntityKind::Opaque,
        },
        predicate,
        object,
        evidence_refs,
        epistemic_status: EpistemicStatus::Observed,
        derivation: None,
        supports: vec![],
        contradicts: vec![],
        observation_context_id: None,
        valid_from: None,
        valid_to: None,
        claim_scope,
        subject_address: Some(Subject {
            address: Address::Opaque {
                system: "content_digest".into(),
                value: digest,
            },
            display: None,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_claim_round_trips_intoto_style() {
        let assertion = Assertion {
            assertion_id: "a1".into(),
            subject: EntityRef {
                entity_id: "opaque".into(),
                entity_kind: EntityKind::Opaque,
            },
            predicate: "entmoot.artifact.scanned".into(),
            object: AssertionObject {
                entity: None,
                value: Some("ok".into()),
            },
            evidence_refs: vec![],
            epistemic_status: EpistemicStatus::Observed,
            derivation: None,
            supports: vec![],
            contradicts: vec![],
            observation_context_id: None,
            valid_from: None,
            valid_to: None,
            claim_scope: Some(ClaimScope::Occurrence),
            subject_address: Some(Subject {
                address: Address::Opaque {
                    system: "content_digest".into(),
                    value:
                        "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                            .into(),
                },
                display: None,
            }),
        };

        let stmt = export_artifact_assertion(&assertion).expect("exportable");
        let back = import_artifact_statement(&stmt).expect("import");
        assert_eq!(back.assertion_id, assertion.assertion_id);
        assert_eq!(back.predicate, assertion.predicate);
        assert_eq!(back.claim_scope, assertion.claim_scope);
        assert_eq!(back.subject_address, assertion.subject_address);
    }

    #[test]
    fn topology_only_assertion_not_forced_into_intoto() {
        let assertion = Assertion {
            assertion_id: "a2".into(),
            subject: EntityRef {
                entity_id: "e".into(),
                entity_kind: EntityKind::Callable,
            },
            predicate: "entmoot.calls".into(),
            object: AssertionObject {
                entity: None,
                value: None,
            },
            evidence_refs: vec![],
            epistemic_status: EpistemicStatus::Observed,
            derivation: None,
            supports: vec![],
            contradicts: vec![],
            observation_context_id: None,
            valid_from: None,
            valid_to: None,
            claim_scope: Some(ClaimScope::Relational),
            subject_address: Some(Subject {
                address: Address::LogicAddress {
                    logic_address: "logic|-|python|a.py|-|f|function|-|-".into(),
                },
                display: None,
            }),
        };
        assert!(export_artifact_assertion(&assertion).is_none());
    }
}
