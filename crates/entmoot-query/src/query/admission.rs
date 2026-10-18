//! One occurrence-preserving admission owner for directed graph discovery.
use super::{
    selection::{CompiledSelector, MatchState},
    QueryEngine, QueryError,
};
use crate::KnowledgeRead;
use entmoot_contracts::query::{HopEvidenceFilter, TraversalPrune};
use entmoot_contracts::{address_identity_key, RelationEdge, ResolutionState};
use std::collections::{HashMap, HashSet};

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct EdgeAdmission {
    excluded_addresses: HashSet<String>,
    excluded_edges: HashSet<String>,
    excluded_occurrences: HashSet<String>,
    evidence: HopEvidenceFilter,
    qualified: HashMap<usize, bool>,
    conflicts: HashSet<String>,
    pub prune_selection: Option<entmoot_contracts::query::SelectionReceipt>,
    pub pruned_edges: usize,
    pub evidence_filtered_edges: usize,
    pub evidence_claims_examined: usize,
}

impl EdgeAdmission {
    pub fn new(
        engine: &QueryEngine,
        prune: &TraversalPrune,
        evidence: HopEvidenceFilter,
    ) -> Result<Self, QueryError> {
        if prune.subjects.len() + prune.relation_edges.len() + prune.occurrences.len() > 1000 {
            return Err(QueryError::InvalidInput(
                "at most 1000 explicit traversal exclusions are supported".into(),
            ));
        }
        let mut admission = Self {
            evidence,
            ..Self::default()
        };
        for subject in &prune.subjects {
            if engine.exact_subject_nodes(subject).len() != 1 {
                return Err(QueryError::InvalidInput(format!(
                    "excluded subject must resolve exactly once: {subject}"
                )));
            }
            admission
                .excluded_addresses
                .extend(engine.walk_endpoint_keys(subject));
        }
        if let Some(selector) = &prune.matching_subjects {
            let compiled = CompiledSelector::new(selector)?;
            let mut receipt = entmoot_contracts::query::SelectionReceipt::default();
            for node in &engine.graph.nodes {
                receipt.examined += 1;
                match compiled.matches(node).0 {
                    MatchState::Yes => {
                        receipt.matched += 1;
                        admission
                            .excluded_addresses
                            .extend(engine.walk_endpoint_keys(&node.id));
                    }
                    MatchState::Unknown => receipt.unknown += 1,
                    MatchState::No => {}
                }
            }
            admission.prune_selection = Some(receipt);
        }
        for id in &prune.relation_edges {
            if !engine
                .graph
                .relation_edges
                .iter()
                .any(|e| e.id.as_str() == id)
            {
                return Err(QueryError::InvalidInput(format!(
                    "excluded relation edge is not represented: {id}"
                )));
            }
            admission.excluded_edges.insert(id.clone());
        }
        for id in &prune.occurrences {
            if !engine.graph.relation_edges.iter().any(|e| {
                e.occurrence_id.as_ref() == Some(id) || e.scope.occurrence_id.as_ref() == Some(id)
            }) {
                return Err(QueryError::InvalidInput(format!(
                    "excluded occurrence is not represented: {id}"
                )));
            }
            admission.excluded_occurrences.insert(id.clone());
        }
        if evidence == HopEvidenceFilter::ResolvedOnly {
            admission.conflicts =
                super::conflicts::retained_conflicts(&engine.graph, &engine.repository()?)?
                    .into_iter()
                    .collect();
        }
        Ok(admission)
    }

    pub fn is_unfiltered(&self) -> bool {
        self.evidence == HopEvidenceFilter::Any
            && self.excluded_addresses.is_empty()
            && self.excluded_edges.is_empty()
            && self.excluded_occurrences.is_empty()
    }
    pub fn needs_evidence(&self) -> bool {
        self.evidence == HopEvidenceFilter::ResolvedOnly
    }
    pub fn excludes_address(&self, key: &str) -> bool {
        self.excluded_addresses.contains(key)
    }

    pub fn retained_bytes(&self) -> usize {
        self.excluded_addresses
            .iter()
            .chain(&self.excluded_edges)
            .chain(&self.excluded_occurrences)
            .chain(&self.conflicts)
            .map(|s| s.capacity() + 64)
            .sum::<usize>()
            + self.qualified.len() * 64
    }

    pub fn allows(
        &mut self,
        index: usize,
        edge: &RelationEdge,
        repository: Option<&impl KnowledgeRead>,
    ) -> Result<bool, QueryError> {
        if self.excluded_edges.contains(edge.id.as_str())
            || edge
                .occurrence_id
                .as_ref()
                .is_some_and(|id| self.excluded_occurrences.contains(id))
            || edge
                .scope
                .occurrence_id
                .as_ref()
                .is_some_and(|id| self.excluded_occurrences.contains(id))
            || self.excludes_address(&address_identity_key(&edge.subject))
            || self.excludes_address(&address_identity_key(&edge.object))
        {
            self.pruned_edges += 1;
            return Ok(false);
        }
        if self.evidence == HopEvidenceFilter::Any {
            return Ok(true);
        }
        let qualifies = if let Some(qualifies) = self.qualified.get(&index) {
            *qualifies
        } else {
            let repository = repository.ok_or_else(|| {
                QueryError::InvalidInput(
                    "resolved-only admission requires a knowledge repository".into(),
                )
            })?;
            let claims = repository
                .relation_claims_for_edges(&[edge])?
                .pop()
                .unwrap_or_default();
            self.evidence_claims_examined += claims.len();
            let states: Vec<_> = claims
                .iter()
                .filter_map(|c| super::applicable_resolution_state(c, edge))
                .collect();
            let qualifies = !self.conflicts.contains(edge.id.as_str())
                && !states.is_empty()
                && states.iter().all(|s| {
                    matches!(
                        s,
                        ResolutionState::ArtifactResolved
                            | ResolutionState::SemanticResolved
                            | ResolutionState::Validated
                    )
                });
            self.qualified.insert(index, qualifies);
            qualifies
        };
        if !qualifies {
            self.evidence_filtered_edges += 1;
        }
        Ok(qualifies)
    }
}
