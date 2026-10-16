//! Typed expert views for conformance and durable-store debugging.

use entmoot_contracts::{
    Address, ArtifactManifest, Assertion, Proposition, PropositionId, RelationClaim, RelationEdge,
    RelationEdgeId, ResourceRecord, TraceRecord, TraceStepRecord, UnresolvedRelation,
};
use serde::Serialize;

use super::QueryError;
use crate::{KnowledgeRecordRef, KnowledgeRecordState, KnowledgeRepository};

/// Wire version for typed expert query results.
pub const EXPERT_QUERY_RESULT_SCHEMA: &str = "entmoot.knowledge.query.result.v1";

/// One bounded collection in an expert result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertPage<T> {
    pub items: Vec<T>,
    pub limit: usize,
    pub returned: usize,
    pub truncated: bool,
}

impl<T> ExpertPage<T> {
    fn from_probe(mut items: Vec<T>, limit: usize) -> Self {
        let truncated = items.len() > limit;
        if truncated {
            items.truncate(limit);
        }
        let returned = items.len();
        Self {
            items,
            limit,
            returned,
            truncated,
        }
    }
}

/// Typed, operation-specific expert result. These views expose exact durable
/// records and ownership mechanics without accepting arbitrary SQL or JSON paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "query_kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ExpertQueryResult {
    Assertions {
        schema_version: &'static str,
        status: &'static str,
        subject_id: String,
        count: usize,
        assertions: Vec<Assertion>,
    },
    Relations {
        schema_version: &'static str,
        status: &'static str,
        address: Address,
        relation_count: usize,
        relation_claim_count: usize,
        unresolved_count: usize,
        relations: Vec<RelationEdge>,
        relation_claims: Vec<RelationClaim>,
        unresolved_relations: Vec<UnresolvedRelation>,
    },
    RelationClaims {
        schema_version: &'static str,
        status: &'static str,
        relation_edge_id: RelationEdgeId,
        count: usize,
        relation_claims: Vec<RelationClaim>,
    },
    Proposition {
        schema_version: &'static str,
        status: &'static str,
        proposition_id: PropositionId,
        proposition: Option<Proposition>,
        occurrence_count: usize,
        relation_claim_count: usize,
        occurrences: Vec<RelationEdge>,
        relation_claims: Vec<RelationClaim>,
    },
    Resources {
        schema_version: &'static str,
        status: &'static str,
        resource_id: String,
        count: usize,
        resources: Vec<ResourceRecord>,
    },
    Unresolved {
        schema_version: &'static str,
        status: &'static str,
        unresolved_id: String,
        unresolved: Option<UnresolvedRelation>,
    },
    Artifact {
        schema_version: &'static str,
        status: &'static str,
        artifact_id: String,
        artifact: Option<ArtifactManifest>,
    },
    Trace {
        schema_version: &'static str,
        status: &'static str,
        trace_id: String,
        trace: Option<TraceRecord>,
        steps: ExpertPage<TraceStepRecord>,
        missing_step_ids: Vec<String>,
    },
    Owner {
        schema_version: &'static str,
        status: &'static str,
        owner_id: String,
        records: ExpertPage<KnowledgeRecordRef>,
    },
    Dependency {
        schema_version: &'static str,
        status: &'static str,
        record: KnowledgeRecordRef,
        state: Option<KnowledgeRecordState>,
        owners: ExpertPage<String>,
        inputs: ExpertPage<KnowledgeRecordRef>,
        dependents: ExpertPage<KnowledgeRecordRef>,
    },
}

pub fn expert_relation_claims_result(
    repository: &impl KnowledgeRepository,
    relation_edge: &str,
) -> Result<ExpertQueryResult, QueryError> {
    let relation_edge = RelationEdgeId::parse(relation_edge.to_owned())
        .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
    let relation_claims = repository.relation_claims_for_relation_edge(relation_edge.as_str())?;
    Ok(ExpertQueryResult::RelationClaims {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(!relation_claims.is_empty()),
        relation_edge_id: relation_edge,
        count: relation_claims.len(),
        relation_claims,
    })
}

pub fn expert_proposition_result(
    repository: &impl KnowledgeRepository,
    proposition: &str,
) -> Result<ExpertQueryResult, QueryError> {
    let proposition = PropositionId::parse(proposition.to_owned())
        .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
    let fact = repository.proposition_by_id(proposition.as_str())?;
    let occurrences = repository.relation_edges_for_proposition(proposition.as_str())?;
    let relation_claims = repository.relation_claims_for_proposition(proposition.as_str())?;
    Ok(ExpertQueryResult::Proposition {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(fact.is_some()),
        proposition_id: proposition,
        proposition: fact,
        occurrence_count: occurrences.len(),
        relation_claim_count: relation_claims.len(),
        occurrences,
        relation_claims,
    })
}

pub fn assertions_result(
    repository: &impl KnowledgeRepository,
    subject: &str,
) -> Result<ExpertQueryResult, QueryError> {
    if subject.trim().is_empty() {
        return Err(QueryError::InvalidInput("subject is required".into()));
    }
    let assertions = repository.assertions_for_subject(subject)?;
    Ok(ExpertQueryResult::Assertions {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(!assertions.is_empty()),
        subject_id: subject.into(),
        count: assertions.len(),
        assertions,
    })
}

pub fn relations_result(
    repository: &impl KnowledgeRepository,
    address: &Address,
) -> Result<ExpertQueryResult, QueryError> {
    address
        .validate()
        .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
    let relations = repository.relation_edges_for_subject(address)?;
    let mut relation_claims = Vec::new();
    for edge in &relations {
        relation_claims.extend(repository.relation_claims_for_relation_edge(edge.id.as_str())?);
        relation_claims
            .extend(repository.relation_claims_for_proposition(edge.proposition_id.as_str())?);
    }
    relation_claims.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    relation_claims.dedup_by(|left, right| left.id == right.id);
    let unresolved_relations = repository.unresolved_for_source(address)?;
    Ok(ExpertQueryResult::Relations {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(!relations.is_empty() || !unresolved_relations.is_empty()),
        address: address.clone(),
        relation_count: relations.len(),
        relation_claim_count: relation_claims.len(),
        unresolved_count: unresolved_relations.len(),
        relations,
        relation_claims,
        unresolved_relations,
    })
}

pub fn resources_result(
    repository: &impl KnowledgeRepository,
    resource: &str,
) -> Result<ExpertQueryResult, QueryError> {
    if resource.trim().is_empty() {
        return Err(QueryError::InvalidInput("resource is required".into()));
    }
    let resources = repository.resources_by_logical_id(resource)?;
    Ok(ExpertQueryResult::Resources {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(!resources.is_empty()),
        resource_id: resource.into(),
        count: resources.len(),
        resources,
    })
}

pub fn unresolved_result(
    repository: &impl KnowledgeRepository,
    unresolved_id: &str,
) -> Result<ExpertQueryResult, QueryError> {
    let unresolved = repository.unresolved_by_id(unresolved_id)?;
    Ok(ExpertQueryResult::Unresolved {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(unresolved.is_some()),
        unresolved_id: unresolved_id.into(),
        unresolved,
    })
}

pub fn artifact_result(
    repository: &impl KnowledgeRepository,
    artifact_id: &str,
) -> Result<ExpertQueryResult, QueryError> {
    let artifact = repository.artifact_by_id(artifact_id)?;
    Ok(ExpertQueryResult::Artifact {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(artifact.is_some()),
        artifact_id: artifact_id.into(),
        artifact,
    })
}

pub fn trace_result(
    repository: &impl KnowledgeRepository,
    trace_id: &str,
    limit: Option<usize>,
) -> Result<ExpertQueryResult, QueryError> {
    let limit = expert_limit(limit)?;
    let trace = repository.trace_by_id(trace_id)?;
    let steps = repository.trace_steps_for_trace(trace_id, limit)?;
    let returned_ids = steps
        .iter()
        .map(|step| step.step_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let missing_step_ids = trace
        .iter()
        .flat_map(|trace| &trace.step_ids)
        .take(limit)
        .filter(|step_id| !returned_ids.contains(step_id.as_str()))
        .cloned()
        .collect();
    let trace_truncated = trace
        .as_ref()
        .is_some_and(|trace| trace.step_ids.len() > limit);
    let returned = steps.len();
    Ok(ExpertQueryResult::Trace {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(trace.is_some()),
        trace_id: trace_id.into(),
        trace,
        steps: ExpertPage {
            items: steps,
            limit,
            returned,
            truncated: trace_truncated,
        },
        missing_step_ids,
    })
}

pub fn owner_result(
    repository: &impl KnowledgeRepository,
    owner_id: &str,
    limit: Option<usize>,
) -> Result<ExpertQueryResult, QueryError> {
    let limit = expert_limit(limit)?;
    let records = ExpertPage::from_probe(
        repository.records_for_owner(owner_id, limit.saturating_add(1))?,
        limit,
    );
    Ok(ExpertQueryResult::Owner {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(!records.items.is_empty()),
        owner_id: owner_id.into(),
        records,
    })
}

pub fn dependency_result(
    repository: &impl KnowledgeRepository,
    record: KnowledgeRecordRef,
    limit: Option<usize>,
) -> Result<ExpertQueryResult, QueryError> {
    let limit = expert_limit(limit)?;
    let probe_limit = limit.saturating_add(1);
    let state = repository.record_state(&record)?;
    let owners = ExpertPage::from_probe(repository.owners_for_record(&record, probe_limit)?, limit);
    let inputs = ExpertPage::from_probe(
        repository.dependency_inputs_for_record(&record, probe_limit)?,
        limit,
    );
    let dependents = ExpertPage::from_probe(
        repository.dependent_records_for_input(&record, probe_limit)?,
        limit,
    );
    let known = state.is_some()
        || !owners.items.is_empty()
        || !inputs.items.is_empty()
        || !dependents.items.is_empty();
    Ok(ExpertQueryResult::Dependency {
        schema_version: EXPERT_QUERY_RESULT_SCHEMA,
        status: presence(known),
        record,
        state,
        owners,
        inputs,
        dependents,
    })
}

fn expert_limit(limit: Option<usize>) -> Result<usize, QueryError> {
    let limit = limit.unwrap_or(100);
    if !(1..=1_000).contains(&limit) {
        return Err(QueryError::InvalidInput(
            "expert query limit must be in 1..=1000".into(),
        ));
    }
    Ok(limit)
}

const fn presence(known: bool) -> &'static str {
    if known {
        "known"
    } else {
        "unknown"
    }
}
