//! Reference-engine claim-scope gate shared by artifact-backed Asserter methods.
//!
//! Resolver and Asserter are semantic capabilities of producers. Their executable
//! ingress is `ArtifactAdapter` followed by verified attachment, not synchronous
//! address-map or request-to-assertion facades without immutable evidence.

use entmoot_contracts::ScopePromotionError;
#[cfg(test)]
use entmoot_contracts::{validate_lah_local_emission, ClaimScope, ProducerCapabilities, Subject};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssertError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    ScopePromotion(#[from] ScopePromotionError),
    #[error("requested claim scope not allowed by producer capabilities")]
    ScopeNotAllowed,
}

/// Shared gate used by Asserter implementations before emitting `lah_local` claims.
#[cfg(test)]
pub(crate) fn enforce_claim_scope(
    caps: &ProducerCapabilities,
    subject: &Subject,
    requested: ClaimScope,
) -> Result<(), AssertError> {
    if !caps.allows_scope(requested) {
        return Err(AssertError::ScopeNotAllowed);
    }
    if requested == ClaimScope::LahLocal {
        validate_lah_local_emission(caps, subject)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        Address, InvocationMode, PolicyId, ProducerIdentity, StructuralHash, Subject,
    };

    fn caps(scopes: Vec<ClaimScope>) -> ProducerCapabilities {
        ProducerCapabilities {
            identity: ProducerIdentity {
                producer_id: "test.asserter".into(),
                version: Some("0".into()),
                tool_name: None,
            },
            accepts_schemes: vec![],
            resolves_to_schemes: vec![],
            predicates: vec![],
            claim_scopes: scopes,
            emits_relations: false,
            supports_completeness: false,
            invocation: InvocationMode::Attach,
            environment: None,
        }
    }

    #[test]
    fn pt29_rejects_silent_lah_promotion_from_path_subject() {
        let capabilities = caps(vec![ClaimScope::Occurrence, ClaimScope::LahLocal]);
        let subject = Subject {
            address: Address::RepoPath {
                repository_id: None,
                path: "a.py".into(),
                start_byte: Some(0),
                end_byte: Some(10),
            },
            display: None,
        };
        let err = enforce_claim_scope(&capabilities, &subject, ClaimScope::LahLocal).unwrap_err();
        assert!(matches!(
            err,
            AssertError::ScopePromotion(ScopePromotionError::SubjectNotLogicId)
        ));
    }

    #[test]
    fn occurrence_scope_allowed_for_path_subject() {
        let capabilities = caps(vec![ClaimScope::Occurrence]);
        let subject = Subject {
            address: Address::RepoPath {
                repository_id: None,
                path: "a.py".into(),
                start_byte: None,
                end_byte: None,
            },
            display: None,
        };
        assert!(enforce_claim_scope(&capabilities, &subject, ClaimScope::Occurrence).is_ok());
    }

    #[test]
    fn lah_local_ok_with_capability_and_logic_id() {
        let capabilities = caps(vec![ClaimScope::LahLocal]);
        let subject = Subject {
            address: Address::LogicId {
                policy_id: PolicyId::from_trusted(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                structural_hash: StructuralHash::from_trusted(
                    "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
            },
            display: None,
        };
        assert!(enforce_claim_scope(&capabilities, &subject, ClaimScope::LahLocal).is_ok());
    }
}
