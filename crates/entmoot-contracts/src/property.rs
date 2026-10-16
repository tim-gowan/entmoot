//! Property store key and dependency declarations.

use crate::{ContentDigest, SourceSnapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Future property-store lookup (Phase 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyStoreKey {
    pub policy_id: crate::PolicyId,
    pub structural_hash: crate::StructuralHash,
    pub property_schema_id: String,
}

/// Declared inputs a property schema depends on (beyond structural_hash).
/// When semantics depend on binding coverage, commit `resolved_hash` via
/// `resolver_output` (or an additional input) — never reuse on structural equality alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyDependencySpec {
    pub property_schema_id: String,
    /// When true, signature_hash must be part of the evaluation/cache key.
    pub requires_signature_hash: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_inputs: Vec<String>,
}

/// Minimum exact-input commitments for an occurrence-scoped derived property.
/// Resolver output includes candidates, gaps, coverage and producer policy/version;
/// the evaluation policy includes traversal bounds and any future checker policy.
/// These names are reserved; additional inputs extend rather than replace them.
pub const REQUIRED_PROPERTY_INPUTS: &[&str] = &[
    "exact_body",
    "occurrence",
    "source_manifest",
    "build_context",
    "resolver_output",
    "normalization_policy",
    "evaluation_policy",
];

/// Unavailable inputs remain auditable but cannot participate in a reusable key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PropertyInputValue {
    /// Digest of an immutable, validated input, not a confidence score.
    Known { digest: ContentDigest },
    /// Missing, stale, unsupported or indeterminate input; always a cache miss.
    Unavailable { reason: String },
}

/// Dependency-complete input commitment for exact-occurrence evaluation.
/// This is not a proof certificate or a cache implementation. Producers must
/// verify source inclusion and digest provenance before evaluating; hash equality
/// alone cannot establish correctness or equivalence across occurrences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyInputManifest {
    pub lookup: PropertyStoreKey,
    pub snapshot: SourceSnapshot,
    /// Canonical named input digests, including all required and declared inputs.
    pub inputs: BTreeMap<String, PropertyInputValue>,
}

/// Reasons a property input commitment cannot be reused.
#[derive(Debug, thiserror::Error)]
pub enum PropertyInputError {
    #[error("invalid property identity or snapshot: {0}")]
    InvalidIdentity(String),
    #[error("property schema mismatch or empty schema id")]
    SchemaMismatch,
    #[error("duplicate, empty, reserved, missing or undeclared dependency")]
    DependencyMismatch,
    #[error("unavailable property dependency: {0}")]
    Unavailable(String),
}

impl PropertyInputManifest {
    /// Derive a length-prefixed BLAKE3 key over every input and its declaration.
    /// Input ordering is canonical. Unknown values and incomplete declarations fail.
    pub fn derive_key(
        &self,
        spec: &PropertyDependencySpec,
    ) -> Result<ContentDigest, PropertyInputError> {
        let invalid =
            |error: crate::IdParseError| PropertyInputError::InvalidIdentity(error.to_string());
        crate::PolicyId::parse(self.lookup.policy_id.as_str()).map_err(invalid)?;
        crate::StructuralHash::parse(self.lookup.structural_hash.as_str()).map_err(invalid)?;
        self.snapshot
            .validate()
            .map_err(|error| PropertyInputError::InvalidIdentity(error.to_string()))?;
        if self.snapshot.snapshot_digest.is_none() {
            return Err(PropertyInputError::InvalidIdentity(
                "snapshot inclusion digest required".into(),
            ));
        }
        if spec.property_schema_id.trim().is_empty()
            || spec.property_schema_id != self.lookup.property_schema_id
        {
            return Err(PropertyInputError::SchemaMismatch);
        }
        let mut names: BTreeSet<&str> = REQUIRED_PROPERTY_INPUTS.iter().copied().collect();
        if spec.requires_signature_hash {
            names.insert("signature_hash");
        }
        for name in &spec.additional_inputs {
            if name.trim().is_empty() || !names.insert(name) {
                return Err(PropertyInputError::DependencyMismatch);
            }
        }
        if !names
            .iter()
            .copied()
            .eq(self.inputs.keys().map(String::as_str))
        {
            return Err(PropertyInputError::DependencyMismatch);
        }
        let snapshot = serde_json::to_string(&self.snapshot)
            .map_err(|error| PropertyInputError::InvalidIdentity(error.to_string()))?;
        let mut parts = vec![
            "property_inputs.v1",
            self.lookup.policy_id.as_str(),
            self.lookup.structural_hash.as_str(),
            &spec.property_schema_id,
            &snapshot,
            if spec.requires_signature_hash {
                "signature_required"
            } else {
                "signature_optional"
            },
        ];
        for (name, value) in &self.inputs {
            match value {
                PropertyInputValue::Known { digest } => {
                    ContentDigest::parse(digest.as_str()).map_err(invalid)?;
                    parts.extend([name.as_str(), digest.as_str()]);
                }
                PropertyInputValue::Unavailable { .. } => {
                    return Err(PropertyInputError::Unavailable(name.clone()))
                }
            }
        }
        Ok(ContentDigest::from_trusted(format!(
            "blake3:{}",
            crate::hash_parts(&parts)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentDigest, SourceSnapshot, SourceState};

    fn inputs() -> (PropertyDependencySpec, PropertyInputManifest) {
        let spec = PropertyDependencySpec {
            property_schema_id: "path_membership.v1".into(),
            requires_signature_hash: true,
            additional_inputs: vec!["path_frontier".into()],
        };
        let digest = ContentDigest::parse(format!("blake3:{}", "a".repeat(64))).unwrap();
        let inputs = REQUIRED_PROPERTY_INPUTS
            .iter()
            .copied()
            .chain(["signature_hash", "path_frontier"])
            .map(|name| {
                (
                    name.to_owned(),
                    PropertyInputValue::Known {
                        digest: digest.clone(),
                    },
                )
            })
            .collect();
        let manifest = PropertyInputManifest {
            lookup: PropertyStoreKey {
                policy_id: crate::PolicyId::parse("b".repeat(64)).unwrap(),
                structural_hash: crate::StructuralHash::parse(digest.as_str()).unwrap(),
                property_schema_id: spec.property_schema_id.clone(),
            },
            snapshot: SourceSnapshot {
                repository_id: "test/repo".into(),
                revision: "c".repeat(40),
                source_state: SourceState::Committed,
                snapshot_digest: Some(digest.to_string()),
                estate_id: None,
                subject: None,
                subject_aliases: Vec::new(),
            },
            inputs,
        };
        (spec, manifest)
    }

    #[test]
    fn property_keys_commit_every_dependency_and_reject_gaps() {
        let (spec, manifest) = inputs();
        let original = manifest.derive_key(&spec).unwrap();
        for name in manifest.inputs.keys() {
            let mut changed = manifest.clone();
            changed.inputs.insert(
                name.clone(),
                PropertyInputValue::Known {
                    digest: ContentDigest::parse(format!("blake3:{}", "d".repeat(64))).unwrap(),
                },
            );
            assert_ne!(original, changed.derive_key(&spec).unwrap(), "{name}");
            changed.inputs.remove(name);
            assert!(changed.derive_key(&spec).is_err(), "{name}");
            changed.inputs.insert(
                name.clone(),
                PropertyInputValue::Unavailable {
                    reason: "stale".into(),
                },
            );
            assert!(changed.derive_key(&spec).is_err(), "{name}");
        }
        let mut changed = manifest;
        changed.snapshot.revision = "d".repeat(40);
        assert_ne!(original, changed.derive_key(&spec).unwrap());
        changed.snapshot.snapshot_digest = None;
        assert!(changed.derive_key(&spec).is_err());
    }

    #[test]
    fn undeclared_duplicate_and_mismatched_dependencies_fail_closed() {
        let (mut spec, mut manifest) = inputs();
        manifest.inputs.insert(
            "undeclared".into(),
            manifest.inputs["path_frontier"].clone(),
        );
        assert!(manifest.derive_key(&spec).is_err());
        manifest.inputs.remove("undeclared");
        spec.additional_inputs.push("path_frontier".into());
        assert!(manifest.derive_key(&spec).is_err());
        spec.additional_inputs.pop();
        spec.property_schema_id = "different".into();
        assert!(manifest.derive_key(&spec).is_err());
    }
}
