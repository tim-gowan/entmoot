//! Per-case fixture observations. Parse rejection is never a hash comparison.
use rayon::prelude::*;

use entmoot_contracts::{
    ContentDigest, ImplementationFingerprint, NormalizationCaseExpectation, NormalizationCaseResult,
    NormalizationObservation, NormalizationReportFormat, NormalizationValidationReport, StructuralHashPolicy,
};

use super::dataset::{GroundTruthCase, GroundTruthDataset};
use crate::{CoreError, StructuralHasher};

thread_local! {
    static THREAD_HASHER: StructuralHasher = StructuralHasher::new();
}

pub struct GroundTruthValidator {
    dataset: GroundTruthDataset,
}

impl GroundTruthValidator {
    pub fn new(dataset: GroundTruthDataset) -> Self {
        Self { dataset }
    }

    pub fn validate(
        &self,
        policy: &StructuralHashPolicy,
        implementation: ImplementationFingerprint,
    ) -> NormalizationValidationReport {
        let mut cases: Vec<_> = self
            .dataset
            .cases()
            .par_iter()
            .map(|case| evaluate_case(case, policy))
            .collect();
        cases.sort_by(|a, b| a.id.cmp(&b.id));
        NormalizationValidationReport {
            format: NormalizationReportFormat::V1,
            implementation,
            policy: policy.clone(),
            dataset_digest: self.dataset.digest().clone(),
            cases,
        }
    }
}

fn observe(source: &str, language: &str, policy: &StructuralHashPolicy) -> NormalizationObservation {
    THREAD_HASHER.with(|hasher| match hasher.compute_hash(source, language, Some(policy)) {
        Ok(hash) => match ContentDigest::parse(hash) {
            Ok(file_hash) => NormalizationObservation::Hashed { file_hash },
            Err(error) => NormalizationObservation::Failed {
                reason: error.to_string(),
            },
        },
        Err(CoreError::Parse(reason)) => NormalizationObservation::ParseRejected { reason },
        Err(CoreError::UnsupportedLanguage(reason)) => NormalizationObservation::UnsupportedLanguage { reason },
        Err(error) => NormalizationObservation::Failed {
            reason: error.to_string(),
        },
    })
}

fn evaluate_case(case: &GroundTruthCase, policy: &StructuralHashPolicy) -> NormalizationCaseResult {
    let expectation = case.expectation;
    let (before, after) = if expectation == NormalizationCaseExpectation::Unqualified {
        (NormalizationObservation::NotRun, NormalizationObservation::NotRun)
    } else {
        // Observe both sides independently; one failure must not erase the other side.
        (
            observe(&case.before, &case.language, policy),
            observe(&case.after, &case.language, policy),
        )
    };
    NormalizationCaseResult {
        id: case.id.clone(),
        language: case.language.clone(),
        expectation,
        before,
        after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{NormalizationCaseOutcome, NormalizationSyntaxExpectation, SyntaxAcceptance};

    #[test]
    fn errors_retain_both_sides_and_do_not_establish_a_hash_relation() {
        let mut case = GroundTruthCase {
            id: "malformed".into(),
            language: "javascript".into(),
            before: "function f() { return 1; }".into(),
            after: "function f( {".into(),
            expectation: NormalizationCaseExpectation::Hash {
                relation: entmoot_contracts::StructuralHashRelation::DifferentHash,
            },
        };
        let policy = StructuralHashPolicy::v1();
        let row = evaluate_case(&case, &policy);
        assert!(matches!(row.before, NormalizationObservation::Hashed { .. }));
        assert!(matches!(row.after, NormalizationObservation::ParseRejected { .. }));
        assert_eq!(row.outcome(), NormalizationCaseOutcome::NotComparable);
        case.expectation = NormalizationCaseExpectation::Syntax {
            expected: NormalizationSyntaxExpectation {
                before: SyntaxAcceptance::Accepted,
                after: SyntaxAcceptance::Rejected,
            },
        };
        assert_eq!(
            evaluate_case(&case, &policy).outcome(),
            NormalizationCaseOutcome::Passed
        );
        case.language = "unsupported".into();
        assert_eq!(
            evaluate_case(&case, &policy).outcome(),
            NormalizationCaseOutcome::NotComparable
        );
        case.language = "javascript".into();
        case.after = case.before.clone();
        assert_eq!(
            evaluate_case(&case, &policy).outcome(),
            NormalizationCaseOutcome::Mismatch
        );
    }
}
