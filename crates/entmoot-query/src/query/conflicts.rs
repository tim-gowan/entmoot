//! Explicit disagreement is evidence about claims, never an endpoint rewrite.
use crate::{KnowledgeRead, QueryError, TopologyGraph};
use entmoot_contracts::{RelationClaim, RelationClaimState, SubjectId};
use std::collections::{BTreeSet, HashMap, HashSet};

pub(super) fn explicit_conflicts<'a>(
    graph: &TopologyGraph,
    claims: impl IntoIterator<Item = &'a RelationClaim>,
) -> BTreeSet<String> {
    let active: HashMap<_, _> = claims
        .into_iter()
        .filter(|claim| claim.state == RelationClaimState::Active)
        .map(|claim| (&claim.id, claim))
        .collect();
    let mut edge_scopes = HashSet::new();
    let mut proposition_scopes = HashSet::new();
    for claim in active.values() {
        for id in &claim.contradicts {
            if let Some(other) = active.get(id) {
                for participant in [*claim, *other] {
                    match &participant.subject {
                        SubjectId::RelationEdge { edge_id } => {
                            edge_scopes.insert((edge_id, &participant.scope));
                        }
                        SubjectId::Proposition { proposition_id } => {
                            proposition_scopes.insert((proposition_id, &participant.scope));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if edge_scopes.is_empty() && proposition_scopes.is_empty() {
        return BTreeSet::new();
    }
    graph
        .relation_edges
        .iter()
        .filter(|edge| {
            edge_scopes.contains(&(&edge.id, &edge.scope))
                || proposition_scopes.contains(&(&edge.proposition_id, &edge.scope))
        })
        .map(|edge| edge.id.to_string())
        .collect()
}

/// Refresh referenced claim states from the pinned repository. Visibility changes
/// can retract either side without rebuilding structural traversal candidates.
pub(super) fn retained_conflicts(
    graph: &TopologyGraph,
    repository: &impl KnowledgeRead,
) -> Result<BTreeSet<String>, QueryError> {
    let ids: BTreeSet<_> = graph
        .relation_claims
        .iter()
        .filter(|claim| !claim.contradicts.is_empty())
        .flat_map(|claim| {
            std::iter::once(claim.id.as_str()).chain(claim.contradicts.iter().map(|id| id.as_str()))
        })
        .collect();
    let claims = ids
        .into_iter()
        .map(|id| repository.relation_claim_by_id(id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(explicit_conflicts(
        graph,
        claims.iter().filter_map(Option::as_ref),
    ))
}
