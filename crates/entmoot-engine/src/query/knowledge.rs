//! Deterministic knowledge-store query projections.

use entmoot_contracts::query::ResultBounds;
use entmoot_contracts::{PropositionId, RelationEdgeId};
use serde_json::{json, Value};

use super::{QueryError, QUERY_RESULT_SCHEMA};
use crate::KnowledgeRepository;

pub(super) fn relation_claims_result(
    repository: &impl KnowledgeRepository,
    relation_edge: &str,
) -> Result<Value, QueryError> {
    let relation_edge = RelationEdgeId::parse(relation_edge.to_owned())
        .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
    let relation_claims = repository.relation_claims_for_relation_edge(relation_edge.as_str())?;
    let status = if relation_claims.is_empty() {
        "unknown"
    } else {
        "known"
    };
    Ok(json!({
        "schema_version": QUERY_RESULT_SCHEMA,
        "operation": "inspect",
        "view": "relation_claims",
        "status": status,
        "relation_edge_id": relation_edge,
        "bounds": ResultBounds::complete(relation_claims.len()),
        "relation_claims": relation_claims,
    }))
}

pub(super) fn proposition_result(
    repository: &impl KnowledgeRepository,
    proposition: &str,
) -> Result<Value, QueryError> {
    let proposition = PropositionId::parse(proposition.to_owned())
        .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
    let fact = repository.proposition_by_id(proposition.as_str())?;
    let occurrences = repository.relation_edges_for_proposition(proposition.as_str())?;
    let relation_claims = repository.relation_claims_for_proposition(proposition.as_str())?;
    let status = if fact.is_some() { "known" } else { "unknown" };
    Ok(json!({
        "schema_version": QUERY_RESULT_SCHEMA,
        "operation": "inspect",
        "view": "proposition",
        "status": status,
        "proposition_id": proposition,
        "proposition": fact,
        "bounds": ResultBounds::complete(occurrences.len() + relation_claims.len()),
        "occurrences": occurrences,
        "relation_claims": relation_claims,
    }))
}
