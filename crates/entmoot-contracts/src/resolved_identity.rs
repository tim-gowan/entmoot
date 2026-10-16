//! Occurrence-scoped binding identity and complete evidence references (ADR-0024).
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{hash_parts, AddressId, ArtifactRef, PolicyId, ProducerIdentity, ResolvedHash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedIdentityCompleteness {
    Complete,
    Partial,
    Unavailable,
}

/// Resolver identity is provider attribution, not structural identity or a manifest name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolverEvidenceRef {
    pub artifact: ArtifactRef,
    pub producer: ProducerIdentity,
}

impl ResolverEvidenceRef {
    pub fn validate(&self) -> Result<(), String> {
        self.artifact
            .validate()
            .map_err(|error| error.to_string())?;
        if self.producer.producer_id.trim().is_empty()
            || [&self.producer.version, &self.producer.tool_name]
                .iter()
                .any(|value| value.as_deref().is_some_and(|text| text.trim().is_empty()))
        {
            return Err("resolver evidence requires nonempty producer identity fields".into());
        }
        Ok(())
    }
}

/// Complete and partial observations always identify both artifact and producer.
/// Unavailable evidence cannot masquerade as a complete empty reference set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "completeness", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolvedBindingEvidence {
    Complete {
        source: ResolverEvidenceRef,
        reference_count: u64,
    },
    Partial {
        source: ResolverEvidenceRef,
        reference_count: u64,
        /// Includes inventoried references without an observation.
        unresolved_count: u64,
        ambiguous_count: u64,
        reason_codes: Vec<String>,
    },
    Unavailable {
        reason_codes: Vec<String>,
    },
}

impl ResolvedBindingEvidence {
    pub fn completeness(&self) -> ResolvedIdentityCompleteness {
        match self {
            Self::Complete { .. } => ResolvedIdentityCompleteness::Complete,
            Self::Partial { .. } => ResolvedIdentityCompleteness::Partial,
            Self::Unavailable { .. } => ResolvedIdentityCompleteness::Unavailable,
        }
    }

    pub fn source(&self) -> Option<&ResolverEvidenceRef> {
        match self {
            Self::Complete { source, .. } | Self::Partial { source, .. } => Some(source),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn reference_count(&self) -> Option<u64> {
        match self {
            Self::Complete {
                reference_count, ..
            }
            | Self::Partial {
                reference_count, ..
            } => Some(*reference_count),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn reason_codes(&self) -> &[String] {
        match self {
            Self::Complete { .. } => &[],
            Self::Partial { reason_codes, .. } | Self::Unavailable { reason_codes } => reason_codes,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(source) = self.source() {
            source.validate()?;
        }
        if let Self::Partial {
            reference_count,
            unresolved_count,
            ambiguous_count,
            ..
        } = self
        {
            let gaps = unresolved_count
                .checked_add(*ambiguous_count)
                .ok_or("resolved identity gap counters overflow")?;
            if gaps > *reference_count {
                return Err(
                    "partial binding gaps must be bounded by inventoried references".into(),
                );
            }
        }
        let reasons = self.reason_codes();
        if !matches!(self, Self::Complete { .. }) && reasons.is_empty() {
            return Err("partial or unavailable binding requires explicit reasons".into());
        }
        let mut unique = std::collections::BTreeSet::new();
        if reasons
            .iter()
            .any(|reason| reason.trim().is_empty() || !unique.insert(reason))
        {
            return Err("binding reasons must be nonempty and unique".into());
        }
        Ok(())
    }
}

/// Binding digest never rewrites structural hashes or occurrence addresses.
/// An unavailable digest is an audit commitment, never comparable for reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedIdentityRecord {
    pub address_id: String,
    pub resolved_hash: String,
    pub resolution_policy_id: String,
    pub binding: ResolvedBindingEvidence,
}

impl ResolvedIdentityRecord {
    /// Commits assessment state for derived views without changing the binding hash.
    pub fn state_fingerprint(&self) -> String {
        let mut parts = vec![
            self.record_id(),
            self.resolved_hash.clone(),
            self.resolution_policy_id.clone(),
        ];
        match &self.binding {
            ResolvedBindingEvidence::Complete {
                reference_count, ..
            } => {
                parts.extend(["complete".into(), reference_count.to_string()]);
            }
            ResolvedBindingEvidence::Partial {
                reference_count,
                unresolved_count,
                ambiguous_count,
                ..
            } => {
                parts.extend([
                    "partial".into(),
                    reference_count.to_string(),
                    unresolved_count.to_string(),
                    ambiguous_count.to_string(),
                ]);
            }
            ResolvedBindingEvidence::Unavailable { .. } => parts.push("unavailable".into()),
        }
        if let Some(source) = self.binding.source() {
            parts.extend([
                source.artifact.content_digest.to_string(),
                source.producer.version.clone().unwrap_or_default(),
                source.producer.tool_name.clone().unwrap_or_default(),
            ]);
        }
        let mut reasons = self.binding.reason_codes().to_vec();
        reasons.sort_unstable();
        parts.extend(reasons);
        hash_parts(&parts.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// Stable storage key: one record per occurrence and attributed artifact.
    pub fn record_id(&self) -> String {
        let source = self.binding.source();
        hash_parts(&[
            "resolved_identity",
            &self.address_id,
            source
                .map(|value| value.artifact.artifact_id.as_str())
                .unwrap_or(""),
            source
                .map(|value| value.producer.producer_id.as_str())
                .unwrap_or(""),
        ])
    }

    pub fn validate(&self) -> Result<(), String> {
        AddressId::parse(&self.address_id)
            .map_err(|error| format!("invalid address_id: {error}"))?;
        ResolvedHash::parse(&self.resolved_hash)
            .map_err(|error| format!("invalid resolved_hash: {error}"))?;
        PolicyId::parse(&self.resolution_policy_id)
            .map_err(|error| format!("invalid resolution_policy_id: {error}"))?;
        self.binding.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactId, ContentDigest};

    fn complete() -> ResolvedIdentityRecord {
        ResolvedIdentityRecord {
            address_id: "a".repeat(64),
            resolved_hash: crate::blake3_hex(b"fixture bindings"),
            resolution_policy_id: "b".repeat(64),
            binding: ResolvedBindingEvidence::Complete {
                source: ResolverEvidenceRef {
                    artifact: ArtifactRef {
                        artifact_id: ArtifactId::from_trusted("c".repeat(64)),
                        content_digest: ContentDigest::from_trusted(crate::blake3_hex(
                            b"fixture bytes",
                        )),
                    },
                    producer: ProducerIdentity {
                        producer_id: "fixture".into(),
                        version: Some("1".into()),
                        tool_name: None,
                    },
                },
                reference_count: 0,
            },
        }
    }

    #[test]
    fn tagged_evidence_rejects_legacy_half_populated_and_contradictory_wire_shapes() {
        let value = serde_json::to_value(complete()).unwrap();
        let roundtrip: ResolvedIdentityRecord = serde_json::from_value(value.clone()).unwrap();
        roundtrip.validate().unwrap();
        for mutation in [
            "missing_digest",
            "missing_producer",
            "complete_with_gaps",
            "unavailable_with_artifact",
            "legacy_fields",
        ] {
            let mut invalid = value.clone();
            match mutation {
                "missing_digest" => {
                    invalid["binding"]["source"]["artifact"]
                        .as_object_mut()
                        .unwrap()
                        .remove("content_digest");
                }
                "missing_producer" => {
                    invalid["binding"]["source"]
                        .as_object_mut()
                        .unwrap()
                        .remove("producer");
                }
                "complete_with_gaps" => invalid["binding"]["unresolved_count"] = 1.into(),
                "unavailable_with_artifact" => {
                    invalid["binding"]["completeness"] = "unavailable".into()
                }
                "legacy_fields" => {
                    invalid.as_object_mut().unwrap().remove("binding");
                    invalid["completeness"] = "complete".into();
                    invalid["artifact_id"] = "c".repeat(64).into();
                }
                _ => unreachable!(),
            }
            assert!(
                serde_json::from_value::<ResolvedIdentityRecord>(invalid).is_err(),
                "{mutation}"
            );
        }
    }

    #[test]
    fn partial_gaps_are_bounded_and_unavailable_is_not_empty_evidence() {
        let mut record = complete();
        let source = record.binding.source().unwrap().clone();
        for (count, unresolved, ambiguous, valid) in [
            (1, 1, 0, true),
            (2, 1, 1, true),
            (1, 0, 0, true),
            (1, 1, 1, false),
            (u64::MAX, u64::MAX, 1, false),
        ] {
            record.binding = ResolvedBindingEvidence::Partial {
                source: source.clone(),
                reference_count: count,
                unresolved_count: unresolved,
                ambiguous_count: ambiguous,
                reason_codes: vec!["inventory_reference_unassessed".into()],
            };
            assert_eq!(record.validate().is_ok(), valid);
        }
        record.binding = ResolvedBindingEvidence::Unavailable {
            reason_codes: vec!["no_artifact".into()],
        };
        record.validate().unwrap();
        assert_eq!(record.binding.source(), None);
        assert_eq!(record.binding.reference_count(), None);
        assert_eq!(complete().binding.reference_count(), Some(0));
        for reasons in [
            vec![],
            vec!["".into()],
            vec!["no_artifact".into(), "no_artifact".into()],
        ] {
            record.binding = ResolvedBindingEvidence::Unavailable {
                reason_codes: reasons,
            };
            assert!(record.validate().is_err());
        }
    }

    #[test]
    fn evidence_state_changes_invalidate_views_without_rewriting_binding_identity() {
        let original = complete();
        let mut record = original.clone();
        record.binding = ResolvedBindingEvidence::Partial {
            source: original.binding.source().unwrap().clone(),
            reference_count: 1,
            unresolved_count: 1,
            ambiguous_count: 0,
            reason_codes: vec!["unresolved".into()],
        };
        assert_eq!(original.resolved_hash, record.resolved_hash);
        assert_eq!(original.record_id(), record.record_id());
        assert_ne!(original.state_fingerprint(), record.state_fingerprint());
        let before = record.state_fingerprint();
        let ResolvedBindingEvidence::Partial {
            source,
            reason_codes,
            ..
        } = &mut record.binding
        else {
            unreachable!()
        };
        source.artifact.content_digest =
            ContentDigest::from_trusted(crate::blake3_hex(b"replacement bytes"));
        reason_codes.push("provider_gap".into());
        assert_ne!(before, record.state_fingerprint());
        let before = record.state_fingerprint();
        let ResolvedBindingEvidence::Partial { reason_codes, .. } = &mut record.binding else {
            unreachable!()
        };
        reason_codes.reverse();
        assert_eq!(before, record.state_fingerprint());
    }
}
