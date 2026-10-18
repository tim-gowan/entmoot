//! Conservative assessments over retained inventory and explicit test ownership.
use super::relationships::RelationshipIndex;
use super::{QueryEngine, QueryError};
use entmoot_contracts::*;
use std::collections::{BTreeSet, HashSet};

fn observations(
    graph: &crate::TopologyGraph,
) -> impl Iterator<Item = (&AssessmentObservation, &RelationClaim)> {
    let artifacts: std::collections::HashMap<_, _> = graph
        .artifacts
        .iter()
        .map(|a| (a.artifact_id.as_str(), a))
        .collect();
    graph.relation_claims.iter().filter_map(move |claim| {
        let RelationClaimValue::Assessment { observation } = &claim.value else {
            return None;
        };
        (claim.state == RelationClaimState::Active
            && claim.scope.domain == graph.snapshot.repository_id
            && claim.scope.snapshot_revision.as_ref() == Some(&graph.snapshot.revision)
            && claim.scope.snapshot_digest == graph.snapshot.snapshot_digest
            && claim.scope.context_id.is_some()
            && !claim.evidence.is_empty()
            && claim.evidence.iter().all(|e| {
                artifacts.get(e.artifact_id.as_str()).is_some_and(|a| {
                    matches!(
                        a.format,
                        ArtifactFormat::AssessmentEvidence | ArtifactFormat::Resolver
                    ) && a.snapshot == graph.snapshot
                        && (a.format == ArtifactFormat::Resolver
                            || a.config_digest == claim.scope.context_id)
                })
            }))
        .then_some((observation, claim))
    })
}

fn subject(node: &crate::TopologyNode) -> AssessmentSubject {
    AssessmentSubject {
        node_id: node.id.clone(),
        path: node.path.clone(),
        label: node.label.clone(),
        span: node
            .start_byte
            .zip(node.end_byte)
            .map(|(start_byte, end_byte)| SiteSpan {
                start_byte,
                end_byte,
            }),
    }
}

fn path_matches(path: &str, owner: &str) -> bool {
    path == owner || (owner.ends_with('/') && path.starts_with(owner))
}

fn validate_path(path: &str) -> Result<(), QueryError> {
    let trimmed = path.trim_end_matches('/');
    if normalize_repo_path(trimmed).is_ok_and(|p| p == trimmed) {
        Ok(())
    } else {
        Err(QueryError::InvalidInput(format!(
            "invalid repository path: {path}"
        )))
    }
}

pub fn assess_test_impact(
    base: &crate::TopologyGraph,
    head: &crate::TopologyGraph,
    request: TestImpactRequest,
) -> Result<TestImpactReport, QueryError> {
    if request.targets.len() > 10_000 {
        return Err(QueryError::InvalidInput(
            "test target budget exceeds 10000".into(),
        ));
    }
    let mut ids = HashSet::new();
    for target in &request.targets {
        target
            .validate()
            .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
        if !ids.insert(&target.id) {
            return Err(QueryError::InvalidInput(
                "test targets require unique IDs and explicit commands".into(),
            ));
        }
    }
    let impact = crate::compare_revision_impact(base, head)
        .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
    let changed_paths: Vec<_> = impact.files.into_iter().map(|f| f.path).collect();
    let changed: HashSet<_> = changed_paths.iter().map(String::as_str).collect();
    let mut reasons: Vec<Vec<TestSelectionReason>> = vec![Vec::new(); request.targets.len()];
    let mut widen = Vec::new();
    if request.targets.is_empty() {
        widen.push("test ownership unavailable".into());
    }
    for path in &changed_paths {
        let mut owned = false;
        for (target, reasons) in request.targets.iter().zip(&mut reasons) {
            if target.owners.iter().any(|owner| path_matches(path, owner)) {
                reasons.push(TestSelectionReason::Owner { path: path.clone() });
                owned = true;
            }
            if target
                .fixtures
                .iter()
                .any(|owner| path_matches(path, owner))
            {
                reasons.push(TestSelectionReason::Fixture { path: path.clone() });
                owned = true;
            }
        }
        if !owned {
            widen.push(format!("unmapped changed path:{path}"));
        }
    }
    let mut limitations = impact.gaps;
    limitations.extend([
        "static reachability does not establish assertion coverage".into(),
        "ownership is the minimum; missing graph paths never remove required tests".into(),
        "execution observations are bounded hits; missing hits never establish lack of coverage"
            .into(),
    ]);
    for (side, graph) in [("base", base), ("head", head)] {
        for (observation, claim) in observations(graph) {
            let AssessmentObservation::TestExecution {
                target: observed,
                covered,
                limitations: gaps,
                ..
            } = observation
            else {
                continue;
            };
            limitations.extend(
                gaps.iter()
                    .map(|gap| format!("{side}:{}:{gap}", observed.id)),
            );
            if let Some((_, reasons)) = request
                .targets
                .iter()
                .zip(&mut reasons)
                .find(|(target, _)| *target == observed)
            {
                for path in covered
                    .iter()
                    .map(|c| &c.path)
                    .filter(|p| changed.contains(p.as_str()))
                    .collect::<BTreeSet<_>>()
                {
                    reasons.push(TestSelectionReason::ExecutionCoverage {
                        side: side.into(),
                        changed_path: path.clone(),
                        evidence: claim.evidence.clone(),
                    });
                }
            }
        }
        limitations.extend(
            graph
                .diagnostics
                .iter()
                .map(|d| format!("{side}:{}:{}", d.code, d.message)),
        );
        let index = RelationshipIndex::new(graph, request.max_nodes, request.max_edges, false);
        if index.exhausted() {
            widen.push(format!("{side} relationship budget exhausted"));
        }
        limitations.extend(index.limitations.iter().map(|l| format!("{side}:{l}")));
        let seeds = graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.path.as_deref().is_some_and(|p| changed.contains(p)))
            .map(|(i, _)| i);
        let parents = index.reverse_paths(seeds);
        for (target, reasons) in request.targets.iter().zip(&mut reasons) {
            // Deterministic shortest witness from each side; declarations stay target-level.
            if let Some((mut node, _)) = graph.nodes.iter().enumerate().find(|(i, n)| {
                parents.contains_key(i)
                    && (target.occurrences.contains(&n.id)
                        || n.path.as_deref().is_some_and(|p| {
                            target.owners.iter().any(|owner| path_matches(p, owner))
                        }))
            }) {
                let mut nodes = vec![graph.nodes[node].id.clone()];
                while let Some(&next) = parents.get(&node) {
                    if next == node {
                        break;
                    }
                    node = next;
                    nodes.push(graph.nodes[node].id.clone());
                }
                if let Some(path) = &graph.nodes[node].path {
                    reasons.push(TestSelectionReason::ReverseRelationship {
                        side: side.into(),
                        changed_path: path.clone(),
                        nodes,
                    });
                }
            }
        }
    }
    let full_suite_required = !widen.is_empty();
    let mut selected = Vec::new();
    for (target, mut reasons) in request.targets.into_iter().zip(reasons) {
        reasons.extend(widen.iter().map(|reason| TestSelectionReason::Widened {
            reason: reason.clone(),
        }));
        if !reasons.is_empty() {
            selected.push(SelectedTestTarget { target, reasons });
        }
    }
    selected.sort_by(|a, b| a.target.id.cmp(&b.target.id));
    limitations.extend(widen);
    limitations.sort();
    limitations.dedup();
    Ok(TestImpactReport {
        method: "entmoot.test-impact.v1".into(),
        implementation: crate::implementation_fingerprint(),
        base_snapshot: base.snapshot.clone(),
        head_snapshot: head.snapshot.clone(),
        selected,
        full_suite_required,
        changed_paths,
        limitations,
    })
}

pub fn assess_unused(
    graph: &crate::TopologyGraph,
    request: UnusedRequest,
) -> Result<UnusedReport, QueryError> {
    if let Some(path) = &request.path {
        validate_path(path)?;
    }
    let index = RelationshipIndex::new(graph, request.max_nodes, request.max_edges, true);
    let mut candidates = Vec::new();
    let observed: Vec<_> = observations(graph).collect();
    let used: HashSet<_> = observed
        .iter()
        .filter_map(|(o, _)| match o {
            AssessmentObservation::CodeUse { subject, .. } => {
                Some(subject.declaration.occurrence.address_id.as_str())
            }
            _ => None,
        })
        .collect();
    let nodes: std::collections::HashMap<_, _> = graph
        .nodes
        .iter()
        .filter_map(|n| n.address_id.as_deref().map(|id| (id, n)))
        .collect();
    let mut diagnosed = HashSet::new();
    for (observation, claim) in &observed {
        if let AssessmentObservation::CompilerUnused {
            subject: declaration,
            diagnostic_code,
            message,
        } = observation
        {
            let id = declaration.declaration.occurrence.address_id.as_str();
            if let Some(node) = nodes.get(id) {
                if request
                    .path
                    .as_ref()
                    .is_some_and(|p| !path_matches(&declaration.path, p))
                {
                    continue;
                }
                diagnosed.insert(node.id.as_str());
                let mut limitations = vec![
                    format!("compiler diagnostic {diagnostic_code}: {message}"),
                    format!(
                        "producer {} in build context {}; review required",
                        claim.producer.id,
                        claim.scope.context_id.as_deref().unwrap_or("unknown")
                    ),
                ];
                if used.contains(id) {
                    limitations.push("other evidence observes a use; the diagnostic is not a global unused-code verdict".into());
                }
                candidates.push(UnusedCandidate {
                    signal: UnusedSignal::CompilerDiagnostic,
                    members: vec![subject(node)],
                    evidence: claim.evidence.clone(),
                    limitations,
                });
            }
        }
    }
    if !index.exhausted() {
        for members in index.components() {
            if members.iter().any(|&i| {
                diagnosed.contains(graph.nodes[i].id.as_str())
                    || graph.nodes[i]
                        .address_id
                        .as_deref()
                        .is_some_and(|id| used.contains(id))
            }) {
                continue;
            }
            if members
                .iter()
                .any(|&i| graph.nodes[i].kind != crate::NodeKind::Callable)
            {
                continue;
            }
            let set: BTreeSet<_> = members.iter().copied().collect();
            if members.iter().any(|i| index.external_incoming.contains(i)) {
                continue;
            }
            if members
                .iter()
                .any(|&i| index.incoming[i].iter().any(|n| !set.contains(n)))
            {
                continue;
            }
            // Scope filters only presentation; incoming references from outside the scope still count.
            if request.path.as_ref().is_some_and(|prefix| {
                !members.iter().any(|&i| {
                    graph.nodes[i]
                        .path
                        .as_deref()
                        .is_some_and(|path| path_matches(path, prefix))
                })
            }) {
                continue;
            }
            let recursive = members.len() > 1 || index.outgoing[members[0]].contains(&members[0]);
            let mut subjects: Vec<_> = members
                .into_iter()
                .map(|i| subject(&graph.nodes[i]))
                .collect();
            subjects.sort_by(|a, b| a.node_id.cmp(&b.node_id));
            candidates.push(UnusedCandidate { evidence: vec![], signal: if recursive { UnusedSignal::MutuallyReferencingIsland } else { UnusedSignal::NoObservedIncomingCalls }, members: subjects,
                limitations: vec!["review required: exports, entrypoints, escapes, callbacks, reflection, implicit calls and inactive configurations can supply unobserved uses".into()] });
        }
    }
    candidates.sort_by_key(|c| {
        (
            c.signal != UnusedSignal::CompilerDiagnostic,
            c.members[0].node_id.clone(),
        )
    });
    let total = candidates.len();
    let limit = request.limit.clamp(1, 200);
    let candidates: Vec<_> = candidates
        .into_iter()
        .skip(request.offset)
        .take(limit)
        .collect();
    let mut bounds =
        crate::query::ResultBounds::window(request.offset, limit, candidates.len(), total);
    bounds.truncated |= index.exhausted();
    let mut limitations = index.limitations;
    limitations.extend(
        graph
            .diagnostics
            .iter()
            .map(|d| format!("{}:{}", d.code, d.message)),
    );
    limitations.extend([
        "compiler diagnostics apply only to the reported producer/build context".into(),
        "non-call references and exposure coverage not established".into(),
        "candidates are scoped observations, never automatic deletion authorization".into(),
    ]);
    Ok(UnusedReport {
        method: "entmoot.unused-candidates.v1".into(),
        implementation: crate::implementation_fingerprint(),
        snapshot: graph.snapshot.clone(),
        completeness: Default::default(),
        summary: "completeness not established".into(),
        candidates,
        bounds,
        examined_nodes: index.outgoing.len(),
        examined_edges: index.examined_edges,
        limitations,
    })
}

impl QueryEngine {
    pub(super) fn assessment_graph(&self) -> Result<crate::TopologyGraph, QueryError> {
        let repository = self.repository()?;
        let graph = match repository.load_assessment_inventory()? {
            Some(graph) => graph,
            None => repository
                .load_assessment()?
                .apply_to_inventory(crate::inventory_skeleton(&self.graph)),
        };
        if graph.snapshot != self.graph.snapshot || graph.policy_ids != self.graph.policy_ids {
            return Err(QueryError::InvalidInput(
                "publication inventory changed; reopen the query engine".into(),
            ));
        }
        Ok(graph)
    }

    pub fn unused(&self, request: UnusedRequest) -> Result<UnusedReport, QueryError> {
        assess_unused(&self.assessment_graph()?, request)
    }
    pub fn test_impact(&self, request: TestImpactRequest) -> Result<TestImpactReport, QueryError> {
        let base = self.compare_graph.as_deref().ok_or_else(|| {
            QueryError::InvalidInput("test impact requires both revision sides".into())
        })?;
        assess_test_impact(base, &self.assessment_graph()?, request)
    }
}
