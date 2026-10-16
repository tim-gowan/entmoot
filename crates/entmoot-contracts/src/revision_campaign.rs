//! Immutable local evaluation inputs. These are run controls, never graph facts.
//! Git object IDs retain Git semantics; material digests commit actual input bytes.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{is_immutable_revision, ContentDigest, ImplementationFingerprint, PolicyId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RevisionCampaignSeries {
    #[serde(rename = "entmoot.revision_campaign_inputs.v1")]
    V1,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CampaignMaterialRole {
    Tool,
    Profile,
    Environment,
}

/// Named bytes whose change invalidates run reuse. Machine paths are supplied
/// by the local executor, not interpreted as portable repository identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignMaterial {
    pub role: CampaignMaterialRole,
    pub id: String,
    pub content_digest: ContentDigest,
}

/// Independently read Git commit metadata; the executor verifies object kinds
/// and values against the selected repository before admitting a receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignRevision {
    pub revision: String,
    pub tree_oid: String,
    pub parents: Vec<String>,
}

/// A selected window never silently becomes a parent edge, even if it happens
/// to contain one. Its reason is part of the immutable run identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignComparisonRelation {
    DirectParent { parent_index: usize },
    SelectedWindow { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignComparison {
    pub base_revision: String,
    pub head_revision: String,
    pub relation: CampaignComparisonRelation,
}

/// Zero delivery pages means deliberately unconsumed work, never empty results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignBudget {
    pub producer_timeout_seconds: u64,
    pub subject_pages: usize,
    pub caller_pages_per_subject: usize,
}

/// Automatic enrichment remains the campaign policy. Native target-mode and
/// runtime qualification are separate results, not consequences of this choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CampaignEnrichment {
    Automatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionCampaignInputs {
    pub schema_version: RevisionCampaignSeries,
    pub implementation: ImplementationFingerprint,
    /// Commits the evaluation reader/writer sources separately from the engine.
    pub evaluator_digest: ContentDigest,
    pub repository_id: String,
    pub policy_ids: Vec<PolicyId>,
    pub enrichment: CampaignEnrichment,
    pub materials: Vec<CampaignMaterial>,
    pub revisions: Vec<CampaignRevision>,
    pub comparisons: Vec<CampaignComparison>,
    pub budget: CampaignBudget,
}

#[derive(Debug, Error)]
pub enum CampaignInputError {
    #[error("campaign implementation fingerprint mismatch")]
    ImplementationMismatch,
    #[error("campaign evaluator fingerprint mismatch")]
    EvaluatorMismatch,
    #[error("invalid campaign inputs: {0}")]
    Invalid(String),
    #[error("cannot serialize campaign inputs: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl RevisionCampaignInputs {
    /// Check the whole input envelope before creating a run or reusing receipts.
    /// Git verification and material-byte verification belong to the executor.
    pub fn validate(
        &self,
        expected: &ImplementationFingerprint,
        evaluator: &ContentDigest,
    ) -> Result<(), CampaignInputError> {
        if &self.implementation != expected {
            return Err(CampaignInputError::ImplementationMismatch);
        }
        if &self.evaluator_digest != evaluator {
            return Err(CampaignInputError::EvaluatorMismatch);
        }
        let invalid = |message: &str| CampaignInputError::Invalid(message.into());
        for digest in [&expected.contracts, &expected.derivation, evaluator] {
            ContentDigest::parse(digest.as_str()).map_err(|error| invalid(&error.to_string()))?;
        }
        if self.repository_id.trim().is_empty()
            || !(1..=3600).contains(&self.budget.producer_timeout_seconds)
        {
            return Err(invalid(
                "repository and producer timeout in 1..=3600 seconds are required",
            ));
        }
        let mut policies = BTreeSet::new();
        for policy in &self.policy_ids {
            PolicyId::parse(policy.as_str()).map_err(|error| invalid(&error.to_string()))?;
            if !policies.insert(policy.as_str()) {
                return Err(invalid("duplicate structural policy"));
            }
        }
        if policies.is_empty() {
            return Err(invalid("structural policies are required"));
        }
        let mut materials = BTreeSet::new();
        for material in &self.materials {
            ContentDigest::parse(material.content_digest.as_str())
                .map_err(|error| invalid(&error.to_string()))?;
            if material.id.trim().is_empty()
                || !materials.insert((material.role, material.id.as_str()))
            {
                return Err(invalid("missing or duplicate material identity"));
            }
        }
        if !materials.contains(&(CampaignMaterialRole::Tool, "entmoot"))
            || !materials
                .iter()
                .any(|(role, _)| *role == CampaignMaterialRole::Profile)
        {
            return Err(invalid(
                "the entmoot tool and at least one profile must be pinned",
            ));
        }
        let mut revisions = BTreeMap::new();
        for revision in &self.revisions {
            for oid in std::iter::once(&revision.revision)
                .chain(std::iter::once(&revision.tree_oid))
                .chain(&revision.parents)
            {
                if !is_immutable_revision(oid) {
                    return Err(invalid(
                        "Git commit, tree and parent IDs must be full object IDs",
                    ));
                }
            }
            if revisions
                .insert(revision.revision.as_str(), revision)
                .is_some()
            {
                return Err(invalid("duplicate revision"));
            }
        }
        if self.comparisons.is_empty() {
            return Err(invalid("at least one explicit comparison is required"));
        }
        let mut pairs = BTreeSet::new();
        for comparison in &self.comparisons {
            if !pairs.insert((&comparison.base_revision, &comparison.head_revision)) {
                return Err(invalid("duplicate comparison"));
            }
            if !revisions.contains_key(comparison.base_revision.as_str()) {
                return Err(invalid("comparison base is not in the input revisions"));
            }
            let head = revisions
                .get(comparison.head_revision.as_str())
                .ok_or_else(|| invalid("comparison head is not in the input revisions"))?;
            match &comparison.relation {
                CampaignComparisonRelation::DirectParent { parent_index } => {
                    if head.parents.get(*parent_index) != Some(&comparison.base_revision) {
                        return Err(invalid(
                            "direct-parent comparison disagrees with head parent metadata",
                        ));
                    }
                }
                CampaignComparisonRelation::SelectedWindow { reason }
                    if reason.trim().is_empty() =>
                {
                    return Err(invalid("selected windows require an explicit reason"));
                }
                CampaignComparisonRelation::SelectedWindow { .. } => {}
            }
        }
        Ok(())
    }

    /// Deterministic identity of the typed inputs, including delivery budgets.
    pub fn input_digest(&self) -> Result<ContentDigest, CampaignInputError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"entmoot.revision_campaign_inputs.v1\0");
        hasher.update(&serde_json::to_vec(self)?);
        Ok(ContentDigest::from_trusted(format!(
            "blake3:{}",
            hasher.finalize().to_hex()
        )))
    }
}
