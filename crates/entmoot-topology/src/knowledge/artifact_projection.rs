//! Positive artifact projection: inventory is never owned by a provider artifact.
use super::*;
use std::collections::BTreeSet;

impl KnowledgeStore {
    /// Count the same artifact/support selection without cloning its payloads.
    /// Used for admission before preparing an atomic revalidation policy.
    pub fn artifact_evidence_record_count(graph: &TopologyGraph, artifact: &str) -> usize {
        let independent = |references: &[entmoot_contracts::EvidenceRef]| {
            !references.is_empty() && references.iter().all(|e| e.artifact_id == artifact)
        };
        let findings: BTreeSet<_> = graph
            .findings
            .iter()
            .filter(|r| r.artifact_id == artifact)
            .map(|r| r.observation_id.as_str())
            .collect();
        let traces: BTreeSet<_> = graph
            .traces
            .iter()
            .filter(|r| findings.contains(r.finding_observation_id.as_str()))
            .map(|r| r.trace_id.as_str())
            .collect();
        let claims: Vec<_> = graph
            .relation_claims
            .iter()
            .filter(|r| {
                r.evidence.iter().any(|e| e.artifact_id == artifact)
                    && (r.derivation.is_none() || independent(&r.evidence))
            })
            .collect();
        let edge_ids: BTreeSet<_> = claims
            .iter()
            .filter_map(|r| match &r.subject {
                entmoot_contracts::SubjectId::RelationEdge { edge_id } => Some(edge_id.as_str()),
                _ => None,
            })
            .collect();
        let edges = graph
            .relation_edges
            .iter()
            .filter(|r| edge_ids.contains(r.id.as_str()))
            .count();
        let propositions: BTreeSet<_> = graph
            .relation_edges
            .iter()
            .filter(|r| edge_ids.contains(r.id.as_str()))
            .map(|r| r.proposition_id.as_str())
            .collect();
        macro_rules! owned_count {
            ($field:ident) => {
                graph
                    .$field
                    .iter()
                    .filter(|r| r.artifact_id == artifact)
                    .count()
            };
        }
        owned_count!(artifacts) + owned_count!(capabilities) + owned_count!(components)
            + owned_count!(component_dependencies) + owned_count!(component_vulnerabilities)
            + owned_count!(bom_compositions) + owned_count!(locations) + owned_count!(attachments)
            + owned_count!(resolver_coverages) + findings.len() + traces.len()
            + graph.trace_steps.iter().filter(|r| traces.contains(r.trace_id.as_str())).count()
            + graph.resources.iter().filter(|r| matches!(&r.origin, entmoot_contracts::ResourceOrigin::Artifact { artifact_id, .. } if artifact_id == artifact)).count()
            + graph.assertions.iter().filter(|r| independent(&r.evidence_refs)).count()
            + graph.resolutions.iter().filter(|r| independent(&r.evidence_refs)).count()
            + graph.resolved_identities.iter().filter(|r| r.binding.source().is_some_and(|source| source.artifact.artifact_id.as_str() == artifact)).count()
            + graph.unresolved_relations.iter().filter(|r| r.evidence.iter().any(|e| e.artifact_id == artifact)).count()
            + graph.diagnostics.iter().filter(|r| r.artifact_id.as_deref() == Some(artifact)).count()
            + claims.len() + edges + propositions.len()
    }

    /// The caller validates the graph and exact publication scope first. Retain
    /// provider meaning, using the same independent/derived support rules as removal.
    pub fn extract_artifact_evidence(graph: &TopologyGraph, artifact: &str) -> Self {
        macro_rules! owned {
            ($field:ident) => {
                graph
                    .$field
                    .iter()
                    .filter(|r| r.artifact_id == artifact)
                    .cloned()
                    .collect()
            };
        }
        let selected = |references: &[entmoot_contracts::EvidenceRef]| {
            !references.is_empty() && references.iter().all(|e| e.artifact_id == artifact)
        };
        let findings: Vec<FindingObservation> = owned!(findings);
        let finding_ids: BTreeSet<_> = findings.iter().map(|f| f.observation_id.as_str()).collect();
        let traces: Vec<TraceRecord> = graph
            .traces
            .iter()
            .filter(|t| finding_ids.contains(t.finding_observation_id.as_str()))
            .cloned()
            .collect();
        let trace_ids: BTreeSet<_> = traces.iter().map(|t| t.trace_id.as_str()).collect();
        let trace_steps = graph
            .trace_steps
            .iter()
            .filter(|s| trace_ids.contains(s.trace_id.as_str()))
            .cloned()
            .collect();
        let relation_claims: Vec<_> = graph
            .relation_claims
            .iter()
            .filter(|c| c.evidence.iter().any(|e| e.artifact_id == artifact))
            .filter_map(|c| {
                if c.derivation.is_some() && !selected(&c.evidence) {
                    return None;
                }
                let mut claim = c.clone();
                crate::relation_claims::retain_independent_evidence(
                    &mut claim.evidence,
                    &mut claim.provenance,
                    |id| id != artifact,
                );
                Some(claim)
            })
            .collect();
        let edge_ids: BTreeSet<_> = relation_claims
            .iter()
            .filter_map(|c| match &c.subject {
                entmoot_contracts::SubjectId::RelationEdge { edge_id } => Some(edge_id.as_str()),
                _ => None,
            })
            .collect();
        let relation_edges: Vec<RelationEdge> = graph
            .relation_edges
            .iter()
            .filter(|e| edge_ids.contains(e.id.as_str()))
            .cloned()
            .collect();
        let propositions = relation_edges
            .iter()
            .map(RelationEdge::proposition)
            .map(|p| (p.id.as_str().to_owned(), p))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        let unresolved_relations = graph
            .unresolved_relations
            .iter()
            .filter(|u| u.evidence.iter().any(|e| e.artifact_id == artifact))
            .map(|u| {
                let mut unknown = u.clone();
                crate::relation_claims::retain_independent_evidence(
                    &mut unknown.evidence,
                    &mut unknown.provenance,
                    |id| id != artifact,
                );
                unknown
            })
            .collect();
        // Name every collection: a new collection must make an explicit choice
        // about artifact ownership instead of silently inheriting a broad default.
        Self {
            code_contracts: vec![],
            declaration_sites: vec![],
            call_sites: vec![],
            coverages: vec![],
            input_roots: vec![],
            structural_edges: vec![],
            artifacts: owned!(artifacts),
            capabilities: owned!(capabilities),
            findings,
            resources: graph.resources.iter().filter(|r| matches!(&r.origin,
                entmoot_contracts::ResourceOrigin::Artifact { artifact_id, .. } if artifact_id == artifact)).cloned().collect(),
            components: owned!(components),
            component_dependencies: owned!(component_dependencies),
            component_vulnerabilities: owned!(component_vulnerabilities),
            bom_compositions: owned!(bom_compositions),
            locations: owned!(locations),
            traces,
            trace_steps,
            attachments: owned!(attachments),
            assertions: graph.assertions.iter().filter(|r| selected(&r.evidence_refs)).cloned().collect(),
            resolutions: graph.resolutions.iter().filter(|r| selected(&r.evidence_refs)).cloned().collect(),
            resolver_coverages: owned!(resolver_coverages),
            resolved_identities: graph.resolved_identities.iter().filter(|r| r.binding.source()
                .is_some_and(|source| source.artifact.artifact_id.as_str() == artifact)).cloned().collect(),
            propositions,
            relation_edges,
            relation_claims,
            unresolved_relations,
            diagnostics: graph.diagnostics.iter().filter(|d| d.artifact_id.as_deref() == Some(artifact)).cloned().collect(),
        }
    }
}
