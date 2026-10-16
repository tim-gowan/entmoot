//! Exact body identity and location verification (INV-15 / INV-16).

use entmoot_contracts::{ExactBodyId, LocationRef, LocationResolveStatus, LogicId, PolicyId, StructuralHash};

/// Digest of observed source bytes for a sealed byte range.
pub fn exact_body_id_of_bytes(bytes: &[u8]) -> ExactBodyId {
    ExactBodyId::from_trusted(format!("blake3:{}", blake3::hash(bytes).to_hex()))
}

/// Digest of a half-open byte span within `source`. Returns `None` if out of range.
pub fn exact_body_id_of_span(source: &[u8], start_byte: usize, end_byte: usize) -> Option<ExactBodyId> {
    if start_byte > end_byte || end_byte > source.len() {
        return None;
    }
    Some(exact_body_id_of_bytes(&source[start_byte..end_byte]))
}

/// Verify a [`LocationRef`] against currently observed exact body bytes.
pub fn verify_location_ref(location: &LocationRef, current_exact_body: Option<&ExactBodyId>) -> LocationResolveStatus {
    match current_exact_body {
        None => LocationResolveStatus::Unresolved,
        Some(current) if current.as_str() == location.expected_exact_body.as_str() => LocationResolveStatus::Current,
        Some(_) => LocationResolveStatus::Stale,
    }
}

/// Build a LogicId from policy + body hash strings (trusted producer path).
pub fn logic_id_from_parts(policy_id: &PolicyId, structural_hash: &str) -> LogicId {
    LogicId::new(policy_id.clone(), StructuralHash::from_trusted(structural_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::SourceRegion;

    #[test]
    fn exact_body_id_deterministic() {
        let a = exact_body_id_of_bytes(b"def foo():\n    return 1\n");
        let b = exact_body_id_of_bytes(b"def foo():\n    return 1\n");
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("blake3:"));
    }

    #[test]
    fn same_exact_body_multiple_locations() {
        let body = b"def foo():\n    pass\n";
        let id = exact_body_id_of_bytes(body);
        let loc_a = LocationRef {
            repository_id: Some("repo".into()),
            path: "a.py".into(),
            region: Some(SourceRegion::Bytes {
                span: entmoot_contracts::SiteSpan {
                    start_byte: 0,
                    end_byte: body.len(),
                },
            }),
            expected_exact_body: id.clone(),
            logic_address: Some("logic|repo|python|a.py|-|foo|function|-|-".into()),
            snapshot_revision: None,
        };
        let loc_b = LocationRef {
            path: "b.py".into(),
            logic_address: Some("logic|repo|python|b.py|-|foo|function|-|-".into()),
            ..loc_a.clone()
        };
        assert_eq!(verify_location_ref(&loc_a, Some(&id)), LocationResolveStatus::Current);
        assert_eq!(verify_location_ref(&loc_b, Some(&id)), LocationResolveStatus::Current);
        assert_eq!(loc_a.expected_exact_body, loc_b.expected_exact_body);
    }

    #[test]
    fn stale_when_body_changes_at_path() {
        let original = exact_body_id_of_bytes(b"def foo():\n    return 1\n");
        let edited = exact_body_id_of_bytes(b"def foo():\n    return 2\n");
        let loc = LocationRef {
            repository_id: None,
            path: "a.py".into(),
            region: None,
            expected_exact_body: original,
            logic_address: None,
            snapshot_revision: None,
        };
        assert_eq!(verify_location_ref(&loc, Some(&edited)), LocationResolveStatus::Stale);
        assert_eq!(verify_location_ref(&loc, None), LocationResolveStatus::Unresolved);
    }

    #[test]
    fn different_exact_bodies_can_share_logic_id_conceptually() {
        // Textually different; under a policy they may share structural_hash (tested in hasher suites).
        // Here we only assert ExactBodyId differs while LogicId construction is independent.
        let e1 = exact_body_id_of_bytes(b"def foo():\n  return 1\n");
        let e2 = exact_body_id_of_bytes(b"def foo():\n    return 1\n");
        assert_ne!(e1, e2);
        let policy = PolicyId::from_trusted("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let logic = logic_id_from_parts(
            &policy,
            "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        assert_eq!(logic.policy_id, policy);
    }
}
