//! Whole-file complements to the occurrence-scoped revision_pairs suite.
//! Expectations come from reviewed source pairs, never from current hash output.
use entmoot_contracts::{
    NormalizationCaseExpectation as Expectation, StructuralHashRelation, SyntaxAcceptance as Acceptance,
};
use entmoot_core::{CoreError, StructuralHasher};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    id: String,
    language: String,
    before: String,
    after: String,
    expectation: Expectation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixtures {
    cases: Vec<Pair>,
}

#[test]
fn authored_file_pairs_preserve_hash_and_parse_expectations() {
    let pairs = serde_json::from_str::<Fixtures>(include_str!("fixtures/file_revision_pairs.json"))
        .unwrap()
        .cases;
    let hasher = StructuralHasher::new();
    assert!(!pairs.is_empty());
    let mut ids = BTreeSet::new();
    let mut relations = BTreeMap::<String, BTreeSet<bool>>::new();
    for pair in pairs {
        assert!(ids.insert(pair.id.clone()), "duplicate pair {}", pair.id);
        let left = hasher.compute_hash(&pair.before, &pair.language, None);
        let right = hasher.compute_hash(&pair.after, &pair.language, None);
        match pair.expectation {
            Expectation::Hash { relation } => {
                let equal = relation == StructuralHashRelation::SameHash;
                assert_eq!(
                    left.unwrap_or_else(|e| panic!("{} base: {e}", pair.id))
                        == right.unwrap_or_else(|e| panic!("{} head: {e}", pair.id)),
                    equal,
                    "{}",
                    pair.id
                );
                relations.entry(pair.language).or_default().insert(equal);
            }
            Expectation::Syntax { expected } => {
                for (side, result, expected) in [("base", left, expected.before), ("head", right, expected.after)] {
                    let actual = match result {
                        Ok(_) => Acceptance::Accepted,
                        Err(CoreError::Parse(_)) => Acceptance::Rejected,
                        Err(e) => panic!("{} {side}: not a syntax result: {e}", pair.id),
                    };
                    assert_eq!(actual, expected, "{} {side}", pair.id);
                }
            }
            Expectation::Unqualified => panic!("{}: unqualified fixture", pair.id),
        }
    }
    for language in hasher.get_supported_languages() {
        assert_eq!(
            relations.get(language.as_str()),
            Some(&BTreeSet::from([false, true])),
            "{language}: both hash relations need controls"
        );
    }
}
