//! Inventory and enrichment upsert of durable edges and claims (ADR-0009).

use entmoot_contracts::{
    relation_claim_with_id, relation_edge_with_id, topology_node_address,
    unresolved_relation_with_id, ProducerCapabilityKind, ProducerRef, Provenance, RelationClaim,
    RelationClaimValue, RelationEdge, RelationKind, ResolutionState, Scope, SubjectId,
    TargetConstraint, UnresolvedReason,
};

use crate::graph::{StructuralEdge, TopologyGraph};

pub const STRUCTURAL_PRODUCER_ID: &str = "entmoot.structural.tree_sitter";
pub const STRUCTURAL_PRODUCER_NAME: &str = "entmoot/tree-sitter";
pub const RELATION_SCHEMA_DOMAIN: &str = "source";

/// Reuse an existing structural occurrence on exact agreement. Context-scoped
/// native-only targets keep their own projections, preserving traversal boundaries.
/// Native claims retain their original scopes, evidence and producer policies.
pub fn share_structural_call_projections(
    graph: &TopologyGraph,
    normalized: &mut crate::NormalizedArtifact,
) {
    use entmoot_contracts::RelationClaimState;
    use std::collections::{HashMap, HashSet};

    let scope = graph_relation_scope(graph);
    let structural = structural_producer();
    let supported: HashSet<_> = graph
        .relation_claims
        .iter()
        .filter_map(|claim| {
            if claim.producer != structural
                || claim.scope != scope
                || claim.state != RelationClaimState::Active
                || claim.predicate != "resolution_state"
                || claim.value
                    != (RelationClaimValue::ResolutionState {
                        state: ResolutionState::Structural,
                    })
            {
                return None;
            }
            match &claim.subject {
                SubjectId::RelationEdge { edge_id } => Some(edge_id),
                _ => None,
            }
        })
        .collect();
    let structural_edges: HashMap<_, _> = graph
        .relation_edges
        .iter()
        .filter(|edge| {
            edge.schema_domain == RELATION_SCHEMA_DOMAIN
                && edge.predicate == RelationKind::Calls
                && edge.occurrence_id.is_some()
                && edge.scope == scope
                && supported.contains(&edge.id)
        })
        .map(|edge| (edge.id.clone(), edge))
        .collect();
    if structural_edges.is_empty() {
        return;
    }
    // A closed dispatch claim is a different capability from a structural
    // candidate. Keep its context-specific occurrence so resolved-only traversal
    // still carries that context; matching endpoints alone do not equate it.
    let candidates: HashSet<_> = normalized
        .relation_claims
        .iter()
        .filter_map(|claim| match (&claim.subject, &claim.value) {
            (
                SubjectId::RelationEdge { edge_id },
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Candidate,
                },
            ) if claim.predicate == "resolution_state" => Some(edge_id.clone()),
            _ => None,
        })
        .collect();
    let mut replacements = HashMap::new();
    for edge in &mut normalized.relation_edges {
        if edge.schema_domain != RELATION_SCHEMA_DOMAIN
            || edge.predicate != RelationKind::Calls
            || edge.occurrence_id.is_none()
            || !candidates.contains(&edge.id)
        {
            continue;
        }
        let shared = relation_edge_with_id(
            RELATION_SCHEMA_DOMAIN,
            edge.subject.clone(),
            edge.predicate,
            edge.object.clone(),
            scope.clone(),
            edge.occurrence_id.clone(),
        );
        if let Some(existing) = structural_edges.get(&shared.id) {
            replacements.insert(edge.id.clone(), existing.id.clone());
            *edge = (*existing).clone();
        }
    }
    for claim in &mut normalized.relation_claims {
        if let SubjectId::RelationEdge { edge_id } = &mut claim.subject {
            if let Some(shared) = replacements.get(edge_id) {
                *edge_id = shared.clone();
                claim.id = entmoot_contracts::compute_relation_claim_id(
                    &claim.subject,
                    &claim.predicate,
                    &claim.value,
                    &claim.producer,
                    &claim.scope,
                );
            }
        }
    }
}

/// Native observations with the same claim identity are independent support for
/// one assertion. Keep every artifact/pointer, without an agreement record.
pub fn merge_native_claim(
    graph: &mut TopologyGraph,
    claim: RelationClaim,
) -> Result<(), crate::ArtifactError> {
    let Some(&index) = graph.claim_by_id.get(claim.id.as_str()) else {
        graph.insert_relation_claim(claim);
        return Ok(());
    };
    let existing = &mut graph.relation_claims[index];
    let mut comparable = existing.clone();
    comparable.evidence = claim.evidence.clone();
    comparable.provenance = claim.provenance.clone();
    if comparable != claim || claim.derivation.is_some() {
        return Err(crate::ArtifactError::Message(
            "native claim identity collision outside independent evidence".into(),
        ));
    }
    merge_independent_evidence(
        &mut existing.evidence,
        &mut existing.provenance,
        claim.evidence,
        claim.provenance,
    )
}

pub fn merge_native_unresolved(
    graph: &mut TopologyGraph,
    relation: entmoot_contracts::UnresolvedRelation,
) -> Result<(), crate::ArtifactError> {
    let Some(&index) = graph.unresolved_relation_by_id.get(relation.id.as_str()) else {
        graph.insert_unresolved_relation(relation);
        return Ok(());
    };
    let existing = &mut graph.unresolved_relations[index];
    let mut comparable = existing.clone();
    comparable.evidence = relation.evidence.clone();
    comparable.provenance = relation.provenance.clone();
    if comparable != relation {
        return Err(crate::ArtifactError::Message(
            "native unresolved identity collision outside independent evidence".into(),
        ));
    }
    merge_independent_evidence(
        &mut existing.evidence,
        &mut existing.provenance,
        relation.evidence,
        relation.provenance,
    )
}

fn merge_independent_evidence(
    evidence: &mut Vec<entmoot_contracts::EvidenceRef>,
    provenance: &mut Option<Provenance>,
    other_evidence: Vec<entmoot_contracts::EvidenceRef>,
    other_provenance: Option<Provenance>,
) -> Result<(), crate::ArtifactError> {
    match (provenance.as_mut(), other_provenance) {
        (Some(left), Some(right))
            if left.snapshot_revision == right.snapshot_revision
                && left.snapshot_digest == right.snapshot_digest
                && left.tool_digest == right.tool_digest =>
        {
            left.artifacts.extend(right.artifacts);
            left.artifacts.sort();
            left.artifacts.dedup();
            // A shared assertion has a single time only when every observation
            // agrees; individual times remain on its immutable artifact manifests.
            if left.produced_at != right.produced_at {
                left.produced_at = None;
            }
        }
        (None, None) => {}
        _ => {
            return Err(crate::ArtifactError::Message(
                "native evidence cannot merge incompatible provenance".into(),
            ))
        }
    }
    evidence.extend(other_evidence);
    evidence.sort_by(|a, b| {
        (&a.artifact_id, &a.provider_pointer, &a.record_id).cmp(&(
            &b.artifact_id,
            &b.provider_pointer,
            &b.record_id,
        ))
    });
    evidence.dedup();
    Ok(())
}

pub fn retain_independent_evidence(
    evidence: &mut Vec<entmoot_contracts::EvidenceRef>,
    provenance: &mut Option<Provenance>,
    removed: impl Fn(&str) -> bool,
) -> bool {
    if !evidence
        .iter()
        .any(|reference| removed(&reference.artifact_id))
    {
        return true;
    }
    evidence.retain(|reference| !removed(&reference.artifact_id));
    if let Some(provenance) = provenance {
        provenance.artifacts.retain(|artifact| !removed(artifact));
    }
    !evidence.is_empty()
}

pub fn graph_relation_scope(graph: &TopologyGraph) -> Scope {
    Scope {
        domain: graph.snapshot.repository_id.clone(),
        snapshot_revision: Some(graph.snapshot.revision.clone()),
        snapshot_digest: graph.snapshot.snapshot_digest.clone(),
        occurrence_id: None,
        context_id: None,
    }
}

fn structural_producer() -> ProducerRef {
    ProducerRef {
        id: STRUCTURAL_PRODUCER_ID.into(),
        name: STRUCTURAL_PRODUCER_NAME.into(),
        version: Some("1".into()),
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![ProducerCapabilityKind::Resolver],
    }
}

/// Upsert durable RelationEdge by RelationEdgeId; return the edge id string.
pub fn upsert_relation_edge(graph: &mut TopologyGraph, edge: RelationEdge) -> String {
    graph.upsert_relation_edge(edge)
}

/// Upsert RelationClaim by RelationClaimId (never overwrite a different claim; skip duplicate id).
pub fn upsert_relation_claim(graph: &mut TopologyGraph, claim: RelationClaim) {
    graph.insert_relation_claim(claim);
}

/// Structural: structural / L2 resolved call → RelationEdge + structural claim; set
/// `StructuralEdge.relation_edge_id`.
pub fn record_structural_resolved_call(graph: &mut TopologyGraph, from_node: &str, to_node: &str) {
    record_structural_call(graph, from_node, to_node, None);
}

/// Exact syntactic occurrence supporting an aggregate call proposition. Ports
/// live on the snapshot-local callsite; this is not a parameter/transfer claim.
pub fn record_structural_resolved_call_site(
    graph: &mut TopologyGraph,
    from_node: &str,
    to_node: &str,
    call_site: &str,
) {
    record_structural_call(graph, from_node, to_node, Some(call_site.into()));
}

fn record_structural_call(
    graph: &mut TopologyGraph,
    from_node: &str,
    to_node: &str,
    occurrence: Option<String>,
) {
    let aggregate = occurrence.is_none();
    let scope = graph_relation_scope(graph);
    let edge = relation_edge_with_id(
        RELATION_SCHEMA_DOMAIN,
        topology_node_address(from_node),
        RelationKind::Calls,
        topology_node_address(to_node),
        scope.clone(),
        occurrence,
    );
    let edge_id = edge.id.clone();
    upsert_relation_edge(graph, edge);
    let claim = relation_claim_with_id(
        SubjectId::RelationEdge {
            edge_id: edge_id.clone(),
        },
        "resolution_state",
        RelationClaimValue::ResolutionState {
            state: ResolutionState::Structural,
        },
        structural_producer(),
        scope,
        vec![],
        Some(Provenance {
            snapshot_revision: Some(graph.snapshot.revision.clone()),
            snapshot_digest: graph.snapshot.snapshot_digest.clone(),
            artifacts: vec![],
            tool_digest: None,
            produced_at: None,
        }),
    );
    upsert_relation_claim(graph, claim);
    if aggregate {
        graph.set_resolved_call_relation_edge_id(from_node, to_node, edge_id.as_str().to_string());
    }
}

/// Structural: syntactic call without a sound semantic target → UnresolvedRelation.
pub fn record_structural_unresolved_invoke(
    graph: &mut TopologyGraph,
    from_node: &str,
    syntactic_callee: &str,
    occurrence_id: &str,
) {
    record_structural_unresolved_call(
        graph,
        from_node,
        TargetConstraint::Syntactic {
            callee: syntactic_callee.into(),
        },
        UnresolvedReason::UnresolvedBinding,
        occurrence_id,
    );
}

pub fn record_structural_unresolved_call(
    graph: &mut TopologyGraph,
    from_node: &str,
    target_constraint: TargetConstraint,
    reason: UnresolvedReason,
    occurrence_id: &str,
) {
    let scope = graph_relation_scope(graph);
    let relation = unresolved_relation_with_id(
        topology_node_address(from_node),
        RelationKind::Calls,
        target_constraint,
        reason,
        structural_producer(),
        scope,
        Some(occurrence_id.into()),
        vec![],
        Some(Provenance {
            snapshot_revision: Some(graph.snapshot.revision.clone()),
            snapshot_digest: graph.snapshot.snapshot_digest.clone(),
            artifacts: vec![],
            tool_digest: None,
            produced_at: None,
        }),
        Some(ProducerCapabilityKind::Resolver),
    );
    graph.insert_unresolved_relation(relation);
}

/// Ensure a structural edge names its durable relation-edge projection.
pub fn attach_relation_edge_id_to_structural_edge(
    edge: &mut StructuralEdge,
    relation_edge_id: &str,
) {
    edge.relation_edge_id = Some(relation_edge_id.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_semantic_target_is_unknown_not_absent() {
        let mut graph = TopologyGraph::new("local/demo");
        record_structural_unresolved_invoke(&mut graph, "caller", "foo", "call-site-1");
        assert!(graph.relation_edges.is_empty());
        assert!(graph.relation_claims.is_empty());
        assert_eq!(graph.unresolved_relations.len(), 1);
        assert_eq!(
            graph.unresolved_relations[0].reason,
            UnresolvedReason::UnresolvedBinding
        );
        assert_eq!(
            graph.unresolved_relations[0].resolution_state,
            ResolutionState::Unknown
        );
    }
}
