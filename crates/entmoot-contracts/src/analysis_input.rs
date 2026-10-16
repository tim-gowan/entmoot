//! Immutable input-set roots and attributable revalidation policies.
//! These authorize conditional dependency invalidation, never proof-cache reuse.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ContentDigest, EvidenceRef, ScopeUniverse};

#[derive(Debug, thiserror::Error)]
pub enum AnalysisInputRootError {
    #[error("input root repository, owner, and method must be nonempty")]
    EmptyIdentity,
    #[error("input root identity mismatch")]
    IdentityMismatch,
    #[error(transparent)]
    Digest(#[from] crate::IdParseError),
}

/// Public wire contract: an attributable policy's declared source-input universe.
/// Completeness is an explicit policy claim, not inferred from observed results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidationDeclaration {
    /// Author of the input-dependency policy, distinct from the observed scanner.
    pub policy_producer: crate::ProducerRef,
    /// Versioned interpretation of input completeness and retention.
    pub method_id: String,
    /// Selectors and checked denominator, including newly appearing inputs.
    pub input_scope: ScopeUniverse,
    /// Conditions required for equal inputs to justify historical retention.
    pub assumptions: Vec<String>,
    /// Immutable observations to which this declaration applies.
    pub evidence_refs: Vec<EvidenceRef>,
}

/// Public wire contract: content identity of an exact dependency input set.
/// Snapshot inclusion is established separately by the publication receipt.
/// Keeping revision outside identity allows unchanged input sets to survive a
/// revision transition without changing the original observation's snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisInputRoot {
    pub id: ContentDigest,
    pub repository_id: String,
    pub owner_key: String,
    pub content_digest: ContentDigest,
    pub method_id: String,
}

impl AnalysisInputRoot {
    pub fn new(
        repository_id: String,
        owner_key: String,
        content_digest: ContentDigest,
        method_id: String,
    ) -> Self {
        let id = Self::identity(&repository_id, &owner_key, &content_digest, &method_id);
        Self {
            id,
            repository_id,
            owner_key,
            content_digest,
            method_id,
        }
    }

    fn identity(
        repository: &str,
        owner: &str,
        digest: &ContentDigest,
        method: &str,
    ) -> ContentDigest {
        ContentDigest::from_trusted(format!(
            "blake3:{}",
            crate::hash_parts(&[
                "analysis_input_root",
                repository,
                owner,
                digest.as_str(),
                method,
            ])
        ))
    }

    pub fn validate(&self) -> Result<(), AnalysisInputRootError> {
        if self.repository_id.trim().is_empty()
            || self.owner_key.trim().is_empty()
            || self.method_id.trim().is_empty()
        {
            return Err(AnalysisInputRootError::EmptyIdentity);
        }
        ContentDigest::parse(self.content_digest.as_str())?;
        if self.id
            != Self::identity(
                &self.repository_id,
                &self.owner_key,
                &self.content_digest,
                &self.method_id,
            )
        {
            return Err(AnalysisInputRootError::IdentityMismatch);
        }
        Ok(())
    }
}
