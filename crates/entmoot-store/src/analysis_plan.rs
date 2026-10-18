//! Deterministic request planning; external workers own execution.
use entmoot_contracts::{
    Address, AnalysisAction as Action, AnalysisDecision, AnalysisDecisionReason as Reason,
    AnalysisDemand, AnalysisGate as Gate, AnalysisGateClaim, AnalysisGateKind,
    AnalysisPassMode as Mode,
};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum AnalysisPlanError {
    #[error("invalid analysis demand: {0}")]
    Invalid(String),
}

pub fn plan_analysis(
    demand: AnalysisDemand,
    mode: Mode,
) -> Result<AnalysisDecision, AnalysisPlanError> {
    let invalid = |e: &dyn std::fmt::Display| AnalysisPlanError::Invalid(e.to_string());
    demand.snapshot.validate().map_err(|e| invalid(&e))?;
    demand.subject.validate().map_err(|e| invalid(&e))?;
    demand.policy_producer.validate().map_err(|e| invalid(&e))?;
    entmoot_contracts::ContentDigest::parse(demand.input_root.as_str()).map_err(|e| invalid(&e))?;
    if let Some(root) = &demand.prior_input_root {
        entmoot_contracts::ContentDigest::parse(root.as_str()).map_err(|e| invalid(&e))?;
    }
    if let Some(id) = &demand.current_artifact {
        entmoot_contracts::ArtifactId::parse(id.as_str()).map_err(|e| invalid(&e))?;
    }
    if demand.method_id.trim().is_empty()
        || demand.evidence_refs.is_empty()
        || demand.assumptions.is_empty()
        || demand.assumptions.iter().any(|a| a.trim().is_empty())
    {
        return Err(AnalysisPlanError::Invalid(
            "method, explicit assumptions, and decision evidence are required".into(),
        ));
    }
    for evidence in &demand.evidence_refs {
        entmoot_contracts::ArtifactId::parse(&evidence.artifact_id).map_err(|e| invalid(&e))?;
        if evidence.provider_pointer.trim().is_empty() {
            return Err(AnalysisPlanError::Invalid("empty evidence pointer".into()));
        }
    }
    let mut gates = BTreeMap::new();
    for gate in &demand.gates {
        gate.claimant.validate().map_err(|e| invalid(&e))?;
        if gate.method_id.trim().is_empty()
            || gate.assumptions.is_empty()
            || gate.assumptions.iter().any(|value| value.trim().is_empty())
            || gate.limitations.is_empty()
            || gate.limitations.iter().any(|value| value.trim().is_empty())
            || gate.evidence_refs.is_empty()
        {
            return Err(AnalysisPlanError::Invalid(
                "every gate requires claimant, method, assumptions, limitations and evidence"
                    .into(),
            ));
        }
        for evidence in &gate.evidence_refs {
            entmoot_contracts::ArtifactId::parse(&evidence.artifact_id).map_err(|e| invalid(&e))?;
            if evidence.provider_pointer.trim().is_empty() {
                return Err(AnalysisPlanError::Invalid(
                    "empty gate evidence pointer".into(),
                ));
            }
        }
        if gates.insert(gate.kind, gate.disposition).is_some() {
            return Err(AnalysisPlanError::Invalid("duplicate gate claim".into()));
        }
    }
    let gate = |kind| {
        gates
            .get(&kind)
            .copied()
            .ok_or_else(|| AnalysisPlanError::Invalid(format!("missing {kind:?} gate claim")))
    };
    let qualification = gate(AnalysisGateKind::Qualification)?;
    let environment = gate(AnalysisGateKind::Environment)?;
    let marginal_value = gate(AnalysisGateKind::MarginalValue)?;
    let changed = demand.prior_input_root.as_ref() != Some(&demand.input_root);
    let (action, reason) = if mode != Mode::Always && !changed && demand.current_artifact.is_some()
    {
        (Action::ReuseCandidate, Reason::ExistingInputMatch)
    } else if mode == Mode::Never {
        (Action::Deferred, Reason::PolicyDisabled)
    } else if qualification != Gate::Accepted {
        (Action::Blocked, Reason::QualificationRequired)
    } else if environment != Gate::Accepted {
        (Action::Blocked, Reason::EnvironmentRequired)
    } else if mode == Mode::Always {
        (Action::RequestArtifact, Reason::ExplicitRequest)
    } else if marginal_value != Gate::Accepted {
        (Action::Deferred, Reason::ValueGateClosed)
    } else if changed {
        (Action::RequestArtifact, Reason::InputsChanged)
    } else if demand.relevant_unknown {
        (Action::RequestArtifact, Reason::RelevantUnknown)
    } else {
        (Action::Deferred, Reason::NoRelevantDemand)
    };
    let mut limitations = demand
        .gates
        .iter()
        .flat_map(|gate| gate.limitations.iter().cloned())
        .collect::<Vec<_>>();
    limitations.push(
        "this decision schedules artifact production; it does not establish graph truth".into(),
    );
    limitations.sort();
    limitations.dedup();
    Ok(AnalysisDecision {
        demand,
        mode,
        action,
        reason,
        limitations,
    })
}

/// Build a demand from a repository-produced invalidation receipt. Gate claims
/// remain independent attributable inputs; roots and artifact reuse are derived.
pub fn plan_revalidation_analysis(
    receipt: &crate::ArtifactRevalidationReceipt,
    subject: Address,
    gates: Vec<AnalysisGateClaim>,
    mode: Mode,
) -> Result<AnalysisDecision, AnalysisPlanError> {
    let current_artifact = (!receipt.inputs_changed)
        .then(|| entmoot_contracts::ArtifactId::parse(receipt.artifact_id.clone()))
        .transpose()
        .map_err(|error| AnalysisPlanError::Invalid(error.to_string()))?;
    let demand = AnalysisDemand {
        policy_producer: receipt.declaration.policy_producer.clone(),
        snapshot: receipt.snapshot.clone(),
        subject,
        method_id: receipt.declaration.method_id.clone(),
        input_root: receipt.current_input_root.id.clone(),
        prior_input_root: Some(receipt.prior_input_root.id.clone()),
        current_artifact,
        gates,
        relevant_unknown: false,
        assumptions: receipt.declaration.assumptions.clone(),
        evidence_refs: receipt.declaration.evidence_refs.clone(),
    };
    let mut decision = plan_analysis(demand, mode)?;
    decision.limitations.extend(receipt.limitations.clone());
    decision.limitations.sort();
    decision.limitations.dedup();
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn demand() -> AnalysisDemand {
        serde_json::from_value(serde_json::json!({
            "policy_producer": {"id":"entmoot.plan", "name":"Fixture policy", "capabilities":["deriver"]},
            "snapshot": {"repository_id":"fixture", "revision":"a".repeat(40), "source_state":"committed"},
            "subject": {"scheme":"opaque", "system":"fixture", "value":"a"},
            "method_id":"fixture.bindings.v1", "input_root":entmoot_contracts::blake3_hex(b"input"),
            "prior_input_root":null, "current_artifact":null,
            "gates":[
              {"kind":"qualification","disposition":"accepted","claimant":{"id":"fixture.qualifier","name":"Fixture qualifier","capabilities":["deriver"]},"method_id":"fixture.qualification.v1","assumptions":["fixture corpus applies"],"limitations":["not ecosystem-wide"],"evidence_refs":[{"artifact_id":"c".repeat(64),"provider_pointer":"/qualification"}]},
              {"kind":"environment","disposition":"accepted","claimant":{"id":"fixture.environment","name":"Fixture environment","capabilities":["deriver"]},"method_id":"fixture.environment.v1","assumptions":["pinned toolchain"],"limitations":["fixture host only"],"evidence_refs":[{"artifact_id":"d".repeat(64),"provider_pointer":"/environment"}]},
              {"kind":"marginal_value","disposition":"accepted","claimant":{"id":"fixture.value","name":"Fixture value gate","capabilities":["deriver"]},"method_id":"fixture.value.v1","assumptions":["matched corpus"],"limitations":["fixture measurement"],"evidence_refs":[{"artifact_id":"e".repeat(64),"provider_pointer":"/measurement"}]}
            ], "relevant_unknown":true,
            "assumptions":["input root commits the declared method, task, source, and build inputs"],
            "evidence_refs":[{"artifact_id":"b".repeat(64),"provider_pointer":"/"}]
        })).unwrap()
    }
    #[test]
    fn decisions_preserve_inputs_and_never_override_qualification() {
        let request = demand();
        let plan = plan_analysis(request.clone(), Mode::Auto).unwrap();
        assert_eq!(plan.action, Action::RequestArtifact);
        assert_eq!(plan.demand, request);
        assert_eq!(
            plan_analysis(request.clone(), Mode::Never).unwrap().reason,
            Reason::PolicyDisabled
        );
        let mut bad = request.clone();
        bad.gates[0].disposition = Gate::Unknown;
        assert_eq!(
            plan_analysis(bad, Mode::Always).unwrap().reason,
            Reason::QualificationRequired
        );
        let mut bad = request.clone();
        bad.gates[1].disposition = Gate::Unknown;
        assert_eq!(
            plan_analysis(bad, Mode::Always).unwrap().action,
            Action::Blocked
        );
        let mut gated = request.clone();
        gated.gates[2].disposition = Gate::Unknown;
        assert_eq!(
            plan_analysis(gated.clone(), Mode::Auto).unwrap().action,
            Action::Deferred
        );
        assert_eq!(
            plan_analysis(gated, Mode::Always).unwrap().reason,
            Reason::ExplicitRequest
        );
        let mut cached = request;
        cached.prior_input_root = Some(cached.input_root.clone());
        cached.current_artifact =
            Some(entmoot_contracts::ArtifactId::parse("b".repeat(64)).unwrap());
        assert_eq!(
            plan_analysis(cached, Mode::Auto).unwrap().action,
            Action::ReuseCandidate
        );
    }

    #[test]
    fn every_gate_has_one_attributable_claim() {
        let mut missing = demand();
        missing.gates.pop();
        assert!(plan_analysis(missing, Mode::Auto)
            .unwrap_err()
            .to_string()
            .contains("missing MarginalValue"));

        let mut duplicate = demand();
        duplicate.gates.push(duplicate.gates[0].clone());
        assert!(plan_analysis(duplicate, Mode::Auto)
            .unwrap_err()
            .to_string()
            .contains("duplicate gate"));

        let mut unattributed = demand();
        unattributed.gates[0].limitations.clear();
        assert!(plan_analysis(unattributed, Mode::Auto)
            .unwrap_err()
            .to_string()
            .contains("every gate requires"));
    }
}
