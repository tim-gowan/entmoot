//! Bounded fixture qualification, separate from semantic bind or corpus coverage.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ContentDigest, ImplementationFingerprint, StructuralHashPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructuralHashRelation {
    SameHash,
    DifferentHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyntaxAcceptance {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizationSyntaxExpectation {
    pub before: SyntaxAcceptance,
    pub after: SyntaxAcceptance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NormalizationCaseExpectation {
    Hash {
        relation: StructuralHashRelation,
    },
    Syntax {
        expected: NormalizationSyntaxExpectation,
    },
    Unqualified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum NormalizationObservation {
    /// Merkle file digest, not a callable definition's structural hash or occurrence.
    Hashed {
        file_hash: ContentDigest,
    },
    ParseRejected {
        reason: String,
    },
    UnsupportedLanguage {
        reason: String,
    },
    Failed {
        reason: String,
    },
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizationCaseResult {
    pub id: String,
    pub language: String,
    pub expectation: NormalizationCaseExpectation,
    pub before: NormalizationObservation,
    pub after: NormalizationObservation,
}

/// Computed from observations; never a second mutable truth field on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationCaseOutcome {
    Passed,
    Mismatch,
    NotComparable,
    Unqualified,
}

impl NormalizationCaseResult {
    pub fn outcome(&self) -> NormalizationCaseOutcome {
        use NormalizationCaseOutcome as Outcome;
        use NormalizationObservation as Observation;
        let matched = match self.expectation {
            NormalizationCaseExpectation::Unqualified => return Outcome::Unqualified,
            NormalizationCaseExpectation::Hash { relation } => {
                let (
                    Observation::Hashed { file_hash: before },
                    Observation::Hashed { file_hash: after },
                ) = (&self.before, &self.after)
                else {
                    return Outcome::NotComparable;
                };
                (before == after) == (relation == StructuralHashRelation::SameHash)
            }
            NormalizationCaseExpectation::Syntax { expected } => {
                let acceptance = |value: &Observation| match value {
                    Observation::Hashed { .. } => Some(SyntaxAcceptance::Accepted),
                    Observation::ParseRejected { .. } => Some(SyntaxAcceptance::Rejected),
                    _ => None,
                };
                let (Some(before), Some(after)) =
                    (acceptance(&self.before), acceptance(&self.after))
                else {
                    return Outcome::NotComparable;
                };
                before == expected.before && after == expected.after
            }
        };
        if matched {
            Outcome::Passed
        } else {
            Outcome::Mismatch
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum NormalizationReportFormat {
    #[serde(rename = "entmoot.normalization_validation.v1")]
    V1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizationValidationReport {
    pub format: NormalizationReportFormat,
    pub implementation: ImplementationFingerprint,
    pub policy: StructuralHashPolicy,
    pub dataset_digest: ContentDigest,
    pub cases: Vec<NormalizationCaseResult>,
}

impl NormalizationValidationReport {
    pub fn validate(&self, current: &ImplementationFingerprint) -> Result<(), &'static str> {
        if &self.implementation != current {
            return Err("normalization report implementation fingerprint mismatch");
        }
        let mut ids = std::collections::BTreeSet::new();
        for case in &self.cases {
            if case.id.trim().is_empty() || case.language.trim().is_empty() || !ids.insert(&case.id)
            {
                return Err("normalization cases require unique nonempty ids and languages");
            }
            for observation in [&case.before, &case.after] {
                match observation {
                    NormalizationObservation::NotRun
                        if case.expectation != NormalizationCaseExpectation::Unqualified =>
                    {
                        return Err("qualified normalization case must be observed");
                    }
                    NormalizationObservation::Failed { reason }
                    | NormalizationObservation::UnsupportedLanguage { reason }
                    | NormalizationObservation::ParseRejected { reason }
                        if reason.trim().is_empty() =>
                    {
                        return Err("missing normalization failure reason")
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// An empty/unqualified-only dataset is not a successful qualification.
    pub fn passes(&self) -> bool {
        self.cases
            .iter()
            .any(|case| case.outcome() == NormalizationCaseOutcome::Passed)
            && self.cases.iter().all(|case| {
                matches!(
                    case.outcome(),
                    NormalizationCaseOutcome::Passed | NormalizationCaseOutcome::Unqualified
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejection_cannot_be_a_hash_comparison_or_unsupported_language_success() {
        let rejected = NormalizationObservation::ParseRejected {
            reason: "syntax error".into(),
        };
        let mut case = NormalizationCaseResult {
            id: "typed-js".into(),
            language: "javascript".into(),
            expectation: NormalizationCaseExpectation::Hash {
                relation: StructuralHashRelation::SameHash,
            },
            before: rejected.clone(),
            after: rejected,
        };
        assert_eq!(case.outcome(), NormalizationCaseOutcome::NotComparable);
        case.expectation = NormalizationCaseExpectation::Syntax {
            expected: NormalizationSyntaxExpectation {
                before: SyntaxAcceptance::Rejected,
                after: SyntaxAcceptance::Rejected,
            },
        };
        assert_eq!(case.outcome(), NormalizationCaseOutcome::Passed);
        case.after = NormalizationObservation::UnsupportedLanguage {
            reason: "no grammar".into(),
        };
        assert_eq!(case.outcome(), NormalizationCaseOutcome::NotComparable);
        let wire = serde_json::to_value(&case).unwrap();
        assert!(wire.get("outcome").is_none());
        assert_eq!(
            serde_json::from_value::<NormalizationCaseResult>(wire).unwrap(),
            case
        );
        assert!(serde_json::from_value::<NormalizationObservation>(serde_json::json!({"outcome":"parse_rejected","reason":"invalid", "hash":crate::blake3_hex(b"bad")})).is_err());
    }

    #[test]
    fn report_requires_fingerprints_and_empty_is_not_success() {
        let current = ImplementationFingerprint {
            contracts: crate::contract_fingerprint(),
            derivation: ContentDigest::parse(crate::blake3_hex(b"test derivation")).unwrap(),
        };
        let report = NormalizationValidationReport {
            format: NormalizationReportFormat::V1,
            implementation: current.clone(),
            policy: StructuralHashPolicy::v1(),
            dataset_digest: ContentDigest::parse(crate::blake3_hex(b"fixture")).unwrap(),
            cases: vec![],
        };
        assert!(report.validate(&current).is_ok());
        assert!(!report.passes());
        let mut stale = current;
        stale.derivation =
            ContentDigest::parse(crate::blake3_hex(b"different derivation")).unwrap();
        assert!(report.validate(&stale).is_err());
        let mut wire = serde_json::to_value(&report).unwrap();
        wire.as_object_mut().unwrap().remove("implementation");
        assert!(serde_json::from_value::<NormalizationValidationReport>(wire).is_err());
    }
}
