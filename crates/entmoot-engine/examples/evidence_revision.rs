//! Executable local OSS exercise; independent producers run before this consumer.
use entmoot_contracts::{
    AnalysisGate, AnalysisGateClaim, AnalysisGateKind, AnalysisPassMode, EvidenceRef,
    RevalidationDeclaration, ScopeCompleteness, ScopeUniverse,
};
use entmoot_engine::{
    evaluate_artifact_revalidation, plan_revalidation_analysis, publish_artifact_evidence,
    KnowledgeRead, SqliteKnowledgeRepository,
};
use serde::Deserialize;
use serde_json::json;
use std::{fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unit {
    base_artifact_id: String,
    head_artifact_id: Option<String>,
    input_paths: Vec<String>,
    method_id: String,
    assumptions: Vec<String>,
    pointer: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    base_graph: PathBuf,
    head_graph: PathBuf,
    ledger: PathBuf,
    units: Vec<Unit>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request: Request = serde_json::from_slice(&fs::read(
        std::env::args()
            .nth(1)
            .ok_or("usage: evidence_revision request.json")?,
    )?)?;
    let base = entmoot_engine::load_graph_json(&request.base_graph)?;
    let head = entmoot_engine::load_graph_json(&request.head_graph)?;
    base.validate()?;
    head.validate()?;
    if base.snapshot.revision == head.snapshot.revision {
        return Err("exercise requires two revisions".into());
    }
    if request.ledger.exists() {
        return Err("use a new ledger for an independently replayable exercise".into());
    }
    let mut ledger = SqliteKnowledgeRepository::open(&request.ledger)?;
    let mut reports = Vec::new();
    for unit in &request.units {
        let artifact = base
            .artifacts
            .iter()
            .find(|a| a.artifact_id == unit.base_artifact_id)
            .ok_or("missing base artifact")?;
        let selected = ScopeUniverse::from_selectors(
            "revalidation-inputs",
            vec![],
            unit.input_paths.clone(),
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let paths = base
            .snapshot_files
            .iter()
            .filter_map(|f| {
                selected
                    .path_in_scope(&f.path)
                    .ok()
                    .filter(|yes| *yes)
                    .map(|_| f.path.clone())
            })
            .collect::<Vec<_>>();
        let declaration = RevalidationDeclaration {
            policy_producer: entmoot_contracts::ProducerRef {
                id: "entmoot.evidence-exercise".into(),
                name: "Entmoot reference input policy".into(),
                version: Some("1".into()),
                config_fingerprint: None,
                environment_fingerprint: None,
                capabilities: vec![entmoot_contracts::ProducerCapabilityKind::Deriver],
            },
            method_id: unit.method_id.clone(),
            input_scope: ScopeUniverse::from_selectors(
                "revalidation-inputs",
                vec![],
                unit.input_paths.clone(),
                vec![],
                ScopeCompleteness::CompleteRelativeToUniverse,
                Some(&paths),
            ),
            assumptions: unit.assumptions.clone(),
            evidence_refs: vec![EvidenceRef {
                artifact_id: artifact.artifact_id.clone(),
                provider_pointer: unit.pointer.clone(),
                record_id: None,
            }],
        };
        let publication = publish_artifact_evidence(&mut ledger, &base, artifact, &declaration)?;
        let replay = publish_artifact_evidence(&mut ledger, &base, artifact, &declaration)?;
        if replay.input_root != publication.input_root {
            return Err("replay changed the input root".into());
        }
        let impact_receipt = evaluate_artifact_revalidation(
            &mut ledger,
            &head,
            artifact,
            &declaration,
            &publication,
        )?;
        if impact_receipt.inputs_changed && !impact_receipt.retained_applicability.is_empty() {
            return Err("changed input retained stale applicability".into());
        }
        if !impact_receipt.inputs_changed && !impact_receipt.invalidated_applicability.is_empty() {
            return Err("unchanged input lost evidence".into());
        }
        for record in &publication.retained_evidence {
            if !ledger
                .record_state(record)?
                .is_some_and(|state| state.visible)
            {
                return Err("original provider evidence was lost".into());
            }
        }
        let repair = if let Some(id) = &unit.head_artifact_id {
            let fresh = head
                .artifacts
                .iter()
                .find(|a| &a.artifact_id == id)
                .ok_or("missing head artifact")?;
            if fresh.artifact_series_id != artifact.artifact_series_id {
                return Err("head artifact is a different producer series".into());
            }
            let mut head_declaration = declaration.clone();
            let paths = head
                .snapshot_files
                .iter()
                .filter_map(|f| {
                    selected
                        .path_in_scope(&f.path)
                        .ok()
                        .filter(|yes| *yes)
                        .map(|_| f.path.clone())
                })
                .collect::<Vec<_>>();
            head_declaration.input_scope.observed_document_set_digest =
                Some(entmoot_contracts::observed_document_set_digest(&paths));
            head_declaration.evidence_refs[0].artifact_id = id.clone();
            let receipt = publish_artifact_evidence(&mut ledger, &head, fresh, &head_declaration)?;
            for record in &receipt.dependent_applicability {
                if !ledger.record_state(record)?.is_some_and(|s| s.visible) {
                    return Err("fresh applicability was not restored".into());
                }
            }
            Some(receipt)
        } else {
            None
        };
        let gate_claim = |kind, id: &str, method: &str, limitation: &str| AnalysisGateClaim {
            kind,
            disposition: AnalysisGate::Accepted,
            claimant: entmoot_contracts::ProducerRef {
                id: id.into(),
                name: "Entmoot exercise gate".into(),
                version: Some("1".into()),
                config_fingerprint: None,
                environment_fingerprint: None,
                capabilities: vec![entmoot_contracts::ProducerCapabilityKind::Deriver],
            },
            method_id: method.into(),
            assumptions: vec!["captured exercise evidence applies to this producer series".into()],
            limitations: vec![limitation.into()],
            evidence_refs: declaration.evidence_refs.clone(),
        };
        let decision = plan_revalidation_analysis(
            &impact_receipt,
            entmoot_contracts::topology_node_address(&artifact.artifact_series_id),
            vec![
                gate_claim(
                    AnalysisGateKind::Qualification,
                    "entmoot.exercise.qualification",
                    "exercise.qualification.v1",
                    "qualification is limited to the pinned fixture and tool version",
                ),
                gate_claim(
                    AnalysisGateKind::Environment,
                    "entmoot.exercise.environment",
                    "exercise.environment.v1",
                    "environment evidence is limited to this captured run",
                ),
                gate_claim(
                    AnalysisGateKind::MarginalValue,
                    "entmoot.exercise.value",
                    "exercise.value.v1",
                    "value is accepted for replaying this exercise, not for adding another adapter",
                ),
            ],
            AnalysisPassMode::Auto,
        )?;
        reports.push(json!({"producer": artifact.producer, "series": artifact.artifact_series_id,
            "base": publication, "revalidation": impact_receipt,
            "requires_revalidation": decision.action == entmoot_contracts::AnalysisAction::RequestArtifact,
            "analysis_decision": decision,
            "invalidated": impact_receipt.invalidated_applicability,
            "retained_under_declared_policy": impact_receipt.retained_applicability, "reingested": repair,
            "limitation": "unchanged inputs retain historical evidence under declared assumptions; only reingestion establishes a new snapshot observation"}));
    }
    let impact = entmoot_engine::compare_revision_impact(&base, &head)?;
    let source_store = std::sync::Arc::new(entmoot_engine::GixSourceStore::open(&head.root)?);
    let head_store = request.head_graph.with_file_name(format!(
        "{}.knowledge.sqlite3",
        request
            .head_graph
            .file_name()
            .ok_or("head filename")?
            .to_string_lossy()
    ));
    let engine =
        entmoot_engine::QueryEngine::new(head.clone(), head_store).with_source_store(source_store);
    let finding_context = engine.find(entmoot_engine::FindQuery::Finding {
        query: None,
        path: Some("routes/login.ts".into()),
        limit: Some(5),
    })?;
    let verified_source = engine.read(entmoot_engine::ReadQuery::Source {
        path: "routes/login.ts".into(),
        start_line: 20,
        end_line: Some(30),
        context_lines: Some(0),
        max_bytes: Some(4096),
    })?;
    let mut paths = Vec::new();
    for edge in head
        .relation_edges
        .iter()
        .filter(|edge| edge.predicate == entmoot_contracts::RelationKind::Calls)
    {
        let site = edge
            .occurrence_id
            .as_deref()
            .and_then(|id| head.nodes.iter().find(|n| n.id == id));
        if site.is_none_or(|site| site.path.as_deref() != Some("routes/login.ts")) {
            continue;
        }
        if let (
            entmoot_contracts::Address::Opaque { value: start, .. },
            entmoot_contracts::Address::Opaque { value: target, .. },
        ) = (&edge.subject, &edge.object)
        {
            paths.push(engine.traverse(entmoot_engine::TraverseQuery::Path {
                evidence_filter: Default::default(),
                prune: Default::default(),
                resumable: false,
                continuation: None,
                start: start.clone(),
                target: target.clone(),
                predicates: vec!["calls".into()],
                budget: entmoot_engine::BoundedWalkBudget {
                    max_hops: 2,
                    max_expansions: 50,
                    max_edges_per_node: 20,
                    max_examined_edges: 100_000,
                    max_results: 1,
                    max_evidence_per_hop: 20,
                },
            })?);
            break;
        }
    }
    let report = json!({"schema_version":"entmoot.evidence_exercise.v1", "base_snapshot":base.snapshot, "head_snapshot":head.snapshot,
        "source_change": impact.files, "units":reports,
        "bounded_context": {"findings": finding_context, "verified_source": verified_source, "candidate_paths": paths},
        "checks": {"source_unchanged": "not reexecuted where complete declared input roots matched", "new_revision_findings": "remaining candidate observations, not exploitability or remediation proofs"},
        "scope": "minimal among the supplied complete input declarations; not a global minimum across unknown producers"});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
