#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::{CanonOp, CanonSort, CanonTerm};

fn form(terms: Vec<CanonTerm>) -> entmoot_contracts::CanonForm {
    use entmoot_contracts::*;
    let root = terms.last().unwrap().digest.clone();
    CanonForm {
        owner: "fixture".into(),
        span: SiteSpan {
            start_byte: 0,
            end_byte: 10,
        },
        language: "rust".into(),
        result_boundary: "bool".into(),
        obligation: CanonObligation::new(
            root.clone(),
            vec![CanonAssumption::ScalarValueObservations],
            vec![],
        ),
        root,
        terms,
        limitations: vec![],
        inputs: None,
    }
}

#[test]
fn forms_reject_ill_typed_terms_and_forged_obligations() {
    let a = CanonTerm::new(CanonOp::Bool { value: true }, CanonSort::Bool, vec![]);
    let invalid = CanonTerm::new(
        CanonOp::Less,
        CanonSort::Bool,
        vec![a.digest.clone(), a.digest.clone()],
    );
    assert!(form(vec![a.clone(), invalid]).validate().is_err());
    let mut valid = form(vec![a]);
    valid.validate().unwrap();
    valid.obligation.assumptions.clear();
    assert!(valid.validate().is_err());
}

#[test]
fn invalid_leaf_sorts_and_integer_ranges_are_rejected() {
    assert!(
        CanonTerm::new(CanonOp::Bool { value: true }, CanonSort::Unknown, vec![])
            .validate()
            .is_err()
    );
    for value in ["256", "-1", "00"] {
        assert!(CanonTerm::new(
            CanonOp::Integer {
                value: value.into()
            },
            CanonSort::Integer {
                bits: Some(8),
                signed: false
            },
            vec![]
        )
        .validate()
        .is_err());
    }
}

#[test]
fn input_slots_and_unmodeled_overflow_cannot_claim_complete_forms() {
    use entmoot_contracts::CanonOverflow;
    let sort = CanonSort::Integer {
        bits: Some(32),
        signed: true,
    };
    let a = CanonTerm::new(CanonOp::Input { slot: 1 }, sort.clone(), vec![]);
    assert!(form(vec![a]).validate().is_err());
    let a = CanonTerm::new(CanonOp::Input { slot: 0 }, sort.clone(), vec![]);
    let add = CanonTerm::new(
        CanonOp::Add {
            overflow: CanonOverflow::Checked,
        },
        sort,
        vec![a.digest.clone(), a.digest.clone()],
    );
    assert!(form(vec![a, add]).validate().is_err());
}

#[test]
fn term_identity_commits_to_semantics_and_child_order() {
    let boolean = CanonSort::Bool;
    let a = CanonTerm::new(CanonOp::Input { slot: 0 }, boolean.clone(), vec![]);
    let b = CanonTerm::new(CanonOp::Input { slot: 1 }, boolean.clone(), vec![]);
    let left = CanonTerm::new(
        CanonOp::Equal,
        boolean.clone(),
        vec![a.digest.clone(), b.digest.clone()],
    );
    let right = CanonTerm::new(CanonOp::Equal, boolean, vec![b.digest, a.digest]);
    assert_ne!(left.digest, right.digest);
    assert!(left.validate().is_ok());
    let mut forged = left;
    forged.children.reverse();
    assert!(forged.validate().is_err());
}

#[test]
fn sort_differences_cannot_share_a_digest() {
    let a = CanonTerm::new(
        CanonOp::Input { slot: 0 },
        CanonSort::Integer {
            bits: Some(32),
            signed: true,
        },
        vec![],
    );
    let b = CanonTerm::new(
        CanonOp::Input { slot: 0 },
        CanonSort::Integer {
            bits: Some(64),
            signed: true,
        },
        vec![],
    );
    assert_ne!(a.digest, b.digest);
    assert!(
        serde_json::from_value::<CanonTerm>(serde_json::json!({"op":"proved_equivalent"})).is_err()
    );
}
