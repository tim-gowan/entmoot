//! The single supported structural identity policy.

use crate::PolicyId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The policy series is separate from its implementation fingerprint.
/// Older lossy modes are not aliases for this policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuralHashPolicy {
    pub series: StructuralPolicySeries,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum StructuralPolicySeries {
    #[default]
    #[serde(rename = "entmoot.structural.v1")]
    V1,
}

impl StructuralHashPolicy {
    /// Token-preserving CST identity, excluding layout and non-directive comments.
    pub fn v1() -> Self {
        Self::default()
    }

    /// The descriptor commits to the retained policy, not the retired v1.1 behavior.
    /// Materialization envelopes separately fence implementation and grammar changes.
    pub fn fingerprint(&self) -> PolicyId {
        PolicyId::from_trusted(blake3::hash(
            r#"{"series":"entmoot.structural.v1","representation":"retained_cst","prune":["layout","non_directive_comments"],"callable_name":"occurrence_only","imports":"syntactic_dependencies","callee_bodies":"excluded"}"#.as_bytes(),
        ).to_hex().to_string())
    }

    /// Equal structural identities are not a proof of semantic equivalence.
    pub fn audit(&self) -> NormalizationAudit {
        NormalizationAudit {
            policy_id: self.fingerprint(),
            policy: self.clone(),
            losses: vec![
                "layout_whitespace",
                "non_directive_comments",
                "callable_declared_name",
                "binding_context",
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NormalizationAudit {
    pub policy_id: PolicyId,
    pub policy: StructuralHashPolicy,
    pub losses: Vec<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sound_v1_policy_can_be_deserialized() {
        let policy = StructuralHashPolicy::v1();
        assert_eq!(
            serde_json::to_value(&policy).unwrap(),
            serde_json::json!({"series":"entmoot.structural.v1"})
        );
        assert_eq!(
            serde_json::from_value::<StructuralHashPolicy>(serde_json::to_value(&policy).unwrap())
                .unwrap(),
            policy
        );
        for invalid in [
            serde_json::json!({}),
            serde_json::json!({"version":"1.1"}),
            serde_json::json!({"version":"2.0"}),
            serde_json::json!({"series":"entmoot.structural.v2"}),
            serde_json::json!({"series":"entmoot.structural.v1","alpha_rename":true}),
            serde_json::json!({"series":"entmoot.structural.v1","prune_type_hints":false}),
        ] {
            assert!(serde_json::from_value::<StructuralHashPolicy>(invalid).is_err());
        }
        assert_eq!(
            policy.fingerprint(),
            StructuralHashPolicy::default().fingerprint()
        );
        // Retired lossy 1.1, minimal 1.1 and sound 2.0 identities are negative inputs.
        for retired in [
            "449c1bf3025397a4dcc4f64ad0505f01521626e2082ca251e9d5015438833e43",
            "94efed56919a6cbf8381c1c2a27c61b8e67355d343ab462b6691e7aada26cd5e",
            "770781cad776c249272f8ee63b17b8981ad451162dd185f1bbf196e92df17f7b",
        ] {
            assert_ne!(policy.fingerprint().as_str(), retired);
        }
        assert!(!policy.audit().losses.contains(&"runtime_documentation"));
        for (name, value) in [
            ("strategy", serde_json::json!("Strict")),
            ("shadow_scoping_mode", serde_json::json!("permissive")),
            ("alpha_rename", serde_json::json!(true)),
            ("prune_docstrings", serde_json::json!(true)),
            ("prune_type_hints", serde_json::json!(true)),
            ("normalize_constant_values", serde_json::json!(true)),
            ("inline_single_use_literals", serde_json::json!(true)),
            ("preserve_semantics", serde_json::json!(true)),
        ] {
            let mut payload = serde_json::to_value(&policy).unwrap();
            payload[name] = value;
            assert!(
                serde_json::from_value::<StructuralHashPolicy>(payload).is_err(),
                "{name}"
            );
        }
    }
}
