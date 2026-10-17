//! Required distinctions complement the existing equivalence/order controls.
use crate::*;
use serde_json::json;
use std::collections::HashSet;

fn producer() -> ProducerRef {
    ProducerRef {
        id: "fixture".into(),
        name: "fixture".into(),
        version: None,
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![],
    }
}

fn scope() -> Scope {
    Scope {
        domain: "repo".into(),
        snapshot_revision: None,
        snapshot_digest: None,
        occurrence_id: None,
        context_id: None,
    }
}

#[test]
fn every_address_variant_preserves_components_and_optional_presence() {
    let digest = format!("blake3:{}", "a".repeat(64));
    let other = format!("blake3:{}", "b".repeat(64));
    let cases = vec![
        json!({"scheme":"canonical_form","digest":digest}),
        json!({"scheme":"canonical_form","digest":other}),
        json!({"scheme":"logic_id","policy_id":"a".repeat(64),"structural_hash":digest}),
        json!({"scheme":"logic_id","policy_id":"b".repeat(64),"structural_hash":digest}),
        json!({"scheme":"logic_id","policy_id":"a".repeat(64),"structural_hash":other}),
        json!({"scheme":"exact_body","exact_body_id":digest}),
        json!({"scheme":"exact_body","exact_body_id":other}),
        json!({"scheme":"logic_address","logic_address":"a:b"}),
        json!({"scheme":"logic_address","logic_address":"a"}),
        json!({"scheme":"address_id","address_id":"a".repeat(64)}),
        json!({"scheme":"address_id","address_id":"b".repeat(64)}),
        json!({"scheme":"repo_path","repository_id":"a:bc","path":"c"}),
        json!({"scheme":"repo_path","repository_id":"a","path":"bc:c"}),
        json!({"scheme":"repo_path","repository_id":null,"path":"c"}),
        json!({"scheme":"repo_path","repository_id":null,"path":"c","start_byte":0}),
        json!({"scheme":"repo_path","repository_id":null,"path":"c","end_byte":0}),
        json!({"scheme":"repo_path","repository_id":null,"path":"c","start_byte":0,"end_byte":0}),
        json!({"scheme":"repo_path","repository_id":null,"path":"c","start_byte":1,"end_byte":2}),
        json!({"scheme":"git_oid","oid":"a".repeat(40)}),
        json!({"scheme":"git_oid","oid":"b".repeat(40)}),
        json!({"scheme":"purl","purl":"pkg:npm/express@4.21.2"}),
        json!({"scheme":"purl","purl":"pkg:npm/express@5.1.0"}),
        json!({"scheme":"resource","resource_system":"a:b","resource_namespace":"c","resource_type":"d","logical_name":"e"}),
        json!({"scheme":"resource","resource_system":"a","resource_namespace":"b:c","resource_type":"d","logical_name":"e"}),
        json!({"scheme":"resource","resource_system":"a","resource_namespace":"b","resource_type":"c:d","logical_name":"e"}),
        json!({"scheme":"resource","resource_system":"a","resource_namespace":"b","resource_type":"c","logical_name":"d:e"}),
        json!({"scheme":"opaque","system":"a:b","value":"c"}),
        json!({"scheme":"opaque","system":"a","value":"b:c"}),
    ];
    let mut keys = HashSet::new();
    let mut propositions = HashSet::new();
    for value in cases {
        let address: Address = serde_json::from_value(value).unwrap();
        address.validate().unwrap();
        assert!(
            keys.insert(address_identity_key(&address)),
            "address collapsed: {address:?}"
        );
        assert!(
            propositions.insert(compute_proposition_id(
                "source",
                "repo",
                &topology_node_address("a"),
                RelationKind::Calls,
                &address
            )),
            "proposition collapsed: {address:?}"
        );
        let roundtrip: Address =
            serde_json::from_str(&serde_json::to_string(&address).unwrap()).unwrap();
        assert_eq!(
            address_identity_key(&address),
            address_identity_key(&roundtrip)
        );
    }
}

#[test]
fn every_scope_field_and_presence_combination_separates_scoped_identities() {
    let value = format!("blake3:{}", "a".repeat(64));
    let mut parts = HashSet::new();
    let mut edges = HashSet::new();
    let mut claims = HashSet::new();
    let mut unresolved = HashSet::new();
    let source = topology_node_address("a");
    let target = topology_node_address("b");
    for mask in 0..16 {
        let scope = Scope {
            snapshot_revision: (mask & 1 != 0).then(|| value.clone()),
            snapshot_digest: (mask & 2 != 0).then(|| value.clone()),
            occurrence_id: (mask & 4 != 0).then(|| value.clone()),
            context_id: (mask & 8 != 0).then(|| value.clone()),
            ..scope()
        };
        scope.validate().unwrap();
        assert!(
            parts.insert(
                scope
                    .canonical_parts()
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            ),
            "scope fields collapsed for mask {mask}"
        );
        assert!(
            edges.insert(compute_relation_edge_id(
                "source",
                &source,
                RelationKind::Calls,
                &target,
                &scope,
                None
            )),
            "edge scope {mask}"
        );
        assert!(
            claims.insert(compute_relation_claim_id(
                &SubjectId::Address {
                    address: source.clone()
                },
                "resolution_state",
                &RelationClaimValue::ResolutionState {
                    state: ResolutionState::Candidate
                },
                &producer(),
                &scope
            )),
            "claim scope {mask}"
        );
        assert!(
            unresolved.insert(compute_unresolved_relation_id(
                &source,
                RelationKind::Calls,
                &TargetConstraint::Unknown,
                UnresolvedReason::AmbiguousTarget,
                &producer(),
                &scope,
                None
            )),
            "attempt scope {mask}"
        );
    }
}

#[test]
fn ambiguous_delimiters_never_drop_candidates_in_either_canonicalization_owner() {
    let a = Address::Opaque {
        system: "a:b".into(),
        value: "c".into(),
    };
    let b = Address::Opaque {
        system: "a".into(),
        value: "b:c".into(),
    };
    let make = |addresses| {
        unresolved_relation_with_id(
            topology_node_address("source"),
            RelationKind::Calls,
            TargetConstraint::Candidates { addresses },
            UnresolvedReason::AmbiguousTarget,
            producer(),
            scope(),
            None,
            vec![],
            None,
            None,
        )
    };
    let pair = make(vec![a.clone(), b.clone(), a.clone()]);
    let reversed = make(vec![b.clone(), a.clone()]);
    assert_eq!(
        pair, reversed,
        "permutations and equivalent duplicates canonicalize equally"
    );
    let TargetConstraint::Candidates { addresses } = &pair.target_constraint else {
        panic!("candidates")
    };
    assert_eq!(addresses.len(), 2);
    assert!(addresses.contains(&a) && addresses.contains(&b));
    assert_ne!(pair.id, make(vec![a.clone()]).id);
    assert_ne!(pair.id, make(vec![b.clone()]).id);
    let attempt = |addresses| {
        ResolutionAttempt::unresolved(
            topology_node_address("source"),
            "fixture",
            None,
            ResolutionAttemptStatus::Ambiguous,
            addresses,
        )
        .unwrap()
    };
    let result = attempt(vec![a.clone(), b.clone(), a.clone()]);
    assert_eq!(result, attempt(vec![b.clone(), a.clone()]));
    assert_eq!(result.candidate_addresses.len(), 2);
    assert!(result.candidate_addresses.contains(&a) && result.candidate_addresses.contains(&b));
}
