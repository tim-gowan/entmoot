//! Subject–predicate–object assertions with epistemic status.
//!
//! `predicate` and object values are **open producer strings**. Core does not
//! close vocabularies for file origin, path role, or security judgment. Durable
//! generated / vendored / spec / first-party path roles belong here on
//! `EntityKind::File` (or `Address::RepoPath`) subjects with a versioned
//! [`DerivationRecord::method_id`] when an Asserter emits them—not as
//! `FileMarkers`, LogicId inputs, or closed contract enums.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicStatus {
    Observed,
    DeterministicallyDerived,
    HeuristicallyDerived,
    Possible,
    Contradicted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub artifact_id: String,
    /// Stable pointer into the raw or normalized observation.
    pub provider_pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DerivationRecord {
    pub method_id: String,
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_refs: Vec<EvidenceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limitations: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_evidence_ref: Option<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssertionObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<crate::entity::EntityRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub assertion_id: String,
    pub subject: crate::entity::EntityRef,
    pub predicate: String,
    pub object: AssertionObject,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<EvidenceRef>,
    pub epistemic_status: EpistemicStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation: Option<DerivationRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradicts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    /// Semantic claim grain (INV-01 / INV-18). Absent on legacy rows = treat as occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_scope: Option<crate::foundational::ClaimScope>,
    /// Prefer scheme-aware subject when present; `subject` EntityRef is the legacy display key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_address: Option<crate::address::Subject>,
}

impl Assertion {
    pub fn validate_epistemic(&self) -> Result<(), String> {
        if self.epistemic_status == EpistemicStatus::Observed {
            return Ok(());
        }
        if self.derivation.is_none() {
            return Err(format!(
                "non-observed assertion {} requires DerivationRecord",
                self.assertion_id
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{EntityKind, EntityRef};

    #[test]
    fn derived_requires_derivation() {
        let a = Assertion {
            assertion_id: "a1".into(),
            subject: EntityRef {
                entity_id: "e1".into(),
                entity_kind: EntityKind::Finding,
            },
            predicate: "affects".into(),
            object: AssertionObject {
                entity: Some(EntityRef {
                    entity_id: "c1".into(),
                    entity_kind: EntityKind::Callable,
                }),
                value: None,
            },
            evidence_refs: vec![],
            epistemic_status: EpistemicStatus::HeuristicallyDerived,
            derivation: None,
            supports: vec![],
            contradicts: vec![],
            observation_context_id: None,
            valid_from: None,
            valid_to: None,
            claim_scope: None,
            subject_address: None,
        };
        assert!(a.validate_epistemic().is_err());
    }
}
