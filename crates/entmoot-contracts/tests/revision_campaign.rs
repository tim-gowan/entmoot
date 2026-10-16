//! Independent run-input controls; Git objects themselves are executor-verified.
#![allow(clippy::unwrap_used)]
use entmoot_contracts::*;

fn fixture() -> RevisionCampaignInputs {
    let digest = ContentDigest::from_trusted(blake3_hex(b"test input bytes"));
    RevisionCampaignInputs {
        schema_version: RevisionCampaignSeries::V1,
        implementation: ImplementationFingerprint {
            contracts: contract_fingerprint(),
            derivation: digest.clone(),
        },
        evaluator_digest: digest.clone(),
        repository_id: "fixture/campaign".into(),
        policy_ids: vec![PolicyId::from_trusted("a".repeat(64))],
        enrichment: CampaignEnrichment::Automatic,
        materials: vec![
            CampaignMaterial {
                role: CampaignMaterialRole::Tool,
                id: "entmoot".into(),
                content_digest: digest.clone(),
            },
            CampaignMaterial {
                role: CampaignMaterialRole::Profile,
                id: "authored-fixture-profile".into(),
                content_digest: digest,
            },
        ],
        revisions: vec![
            CampaignRevision {
                revision: "a".repeat(40),
                tree_oid: "1".repeat(40),
                parents: vec![],
            },
            CampaignRevision {
                revision: "b".repeat(40),
                tree_oid: "2".repeat(40),
                parents: vec!["a".repeat(40)],
            },
            CampaignRevision {
                revision: "c".repeat(40),
                tree_oid: "3".repeat(40),
                parents: vec!["b".repeat(40)],
            },
        ],
        comparisons: vec![CampaignComparison {
            base_revision: "a".repeat(40),
            head_revision: "b".repeat(40),
            relation: CampaignComparisonRelation::DirectParent { parent_index: 0 },
        }],
        budget: CampaignBudget {
            producer_timeout_seconds: 30,
            subject_pages: 1,
            caller_pages_per_subject: 0,
        },
    }
}

#[test]
fn selected_revision_windows_never_masquerade_as_direct_parents() {
    let inputs = fixture();
    inputs
        .validate(&inputs.implementation, &inputs.evaluator_digest)
        .unwrap();
    let mut window = inputs.clone();
    window.comparisons[0].head_revision = "c".repeat(40);
    assert!(window
        .validate(&inputs.implementation, &inputs.evaluator_digest)
        .is_err());
    window.comparisons[0].relation = CampaignComparisonRelation::SelectedWindow {
        reason: "intentional two-commit window".into(),
    };
    window
        .validate(&inputs.implementation, &inputs.evaluator_digest)
        .unwrap();
    assert_ne!(
        inputs.input_digest().unwrap(),
        window.input_digest().unwrap()
    );
    window.comparisons[0].relation =
        CampaignComparisonRelation::SelectedWindow { reason: " ".into() };
    assert!(window
        .validate(&inputs.implementation, &inputs.evaluator_digest)
        .is_err());
    let mut merge = inputs.clone();
    merge.revisions[2].parents = vec!["a".repeat(40), "b".repeat(40)];
    merge.comparisons[0] = CampaignComparison {
        base_revision: "b".repeat(40),
        head_revision: "c".repeat(40),
        relation: CampaignComparisonRelation::DirectParent { parent_index: 1 },
    };
    merge
        .validate(&inputs.implementation, &inputs.evaluator_digest)
        .unwrap();
    merge.comparisons[0].relation = CampaignComparisonRelation::DirectParent { parent_index: 2 };
    assert!(merge
        .validate(&inputs.implementation, &inputs.evaluator_digest)
        .is_err());
}

#[test]
fn run_identity_commits_tools_profiles_budgets_and_both_implementations() {
    let inputs = fixture();
    let original = inputs.input_digest().unwrap();
    assert_eq!(inputs.input_digest().unwrap(), original);
    for field in [
        "tool",
        "profile",
        "budget",
        "implementation",
        "evaluator",
        "policy",
        "source",
    ] {
        let mut changed = inputs.clone();
        let digest = ContentDigest::from_trusted(blake3_hex(b"changed input"));
        match field {
            "tool" => changed.materials[0].content_digest = digest,
            "profile" => changed.materials[1].content_digest = digest,
            "budget" => changed.budget.subject_pages = 0,
            "implementation" => changed.implementation.derivation = digest,
            "evaluator" => changed.evaluator_digest = digest,
            "policy" => changed.policy_ids[0] = PolicyId::from_trusted("b".repeat(64)),
            "source" => changed.revisions[0].tree_oid = "4".repeat(40),
            _ => unreachable!(),
        }
        assert_ne!(changed.input_digest().unwrap(), original, "{field}");
        let result = changed.validate(&inputs.implementation, &inputs.evaluator_digest);
        assert_eq!(
            result.is_err(),
            matches!(field, "implementation" | "evaluator"),
            "{field}"
        );
    }
}

#[test]
fn partial_context_duplicates_and_mutable_revisions_are_rejected() {
    let inputs = fixture();
    for field in [
        "implementation",
        "evaluator_digest",
        "materials",
        "policy_ids",
        "enrichment",
        "budget",
        "comparisons",
    ] {
        let mut value = serde_json::to_value(&inputs).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<RevisionCampaignInputs>(value).is_err(),
            "{field}"
        );
    }
    for case in [
        "no_profile",
        "no_tool",
        "duplicate_material",
        "duplicate_revision",
        "duplicate_pair",
        "missing_side",
        "mutable_revision",
        "timeout",
        "excessive_timeout",
        "no_policy",
    ] {
        let mut invalid = inputs.clone();
        match case {
            "no_profile" => {
                invalid.materials.remove(1);
            }
            "no_tool" => {
                invalid.materials.remove(0);
            }
            "duplicate_material" => invalid.materials.push(invalid.materials[0].clone()),
            "duplicate_revision" => invalid.revisions.push(invalid.revisions[0].clone()),
            "duplicate_pair" => invalid.comparisons.push(invalid.comparisons[0].clone()),
            "missing_side" => invalid.comparisons[0].base_revision = "d".repeat(40),
            "mutable_revision" => invalid.revisions[0].revision = "main".into(),
            "timeout" => invalid.budget.producer_timeout_seconds = 0,
            "excessive_timeout" => invalid.budget.producer_timeout_seconds = 3601,
            "no_policy" => invalid.policy_ids.clear(),
            _ => unreachable!(),
        }
        assert!(
            invalid
                .validate(&inputs.implementation, &inputs.evaluator_digest)
                .is_err(),
            "{case}"
        );
    }
    let mut retired = serde_json::to_value(inputs).unwrap();
    retired["schema_version"] = "entmoot.revision_campaign_inputs.v2".into();
    assert!(serde_json::from_value::<RevisionCampaignInputs>(retired).is_err());
}
