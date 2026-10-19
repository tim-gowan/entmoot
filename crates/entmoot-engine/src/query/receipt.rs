//! Compact evidence receipts for the four public query operations.

use std::collections::{BTreeMap, BTreeSet};

use entmoot_contracts::query::{
    AttachmentDispositionCount, AttachmentEvidenceSummary, AttachmentGrainCount,
    EvidenceInspectBody, EvidenceReceipt, FindPayload, InspectPayload, ProviderEvidenceReceipt,
    QueryEvidenceReceipt, QueryPayload, QueryResult, RelationEvidenceSummary, SchemaJson,
    SourceVerificationReceipt, SourceVerificationStatus, UnresolvedEvidenceSummary,
};
use entmoot_contracts::{
    Assertion, AttachmentDisposition, AttachmentFreshness, AttachmentTargetKind,
    FindingObservation, LocationAttachment, ProducerRef, RelationClaim, RelationClaimState,
    RelationEdge, ResolutionState, UnresolvedReason, UnresolvedRelation,
};

use crate::{KnowledgeRepository, QueryError, TopologyGraph};

const RECEIPT_ITEM_LIMIT: usize = 100;

pub(super) fn build_query_receipt<R, F>(
    graph: &TopologyGraph,
    result: &QueryResult,
    open_repository: F,
) -> Result<QueryEvidenceReceipt, QueryError>
where
    R: KnowledgeRepository,
    F: FnOnce() -> Result<R, QueryError>,
{
    let mut builder = ReceiptBuilder::new(graph);
    builder.collect_payload(&result.payload);
    if !builder.edges.is_empty() {
        builder.hydrate_relation_claims(&open_repository()?)?;
    }
    Ok(builder.finish())
}

struct ReceiptBuilder<'a> {
    graph: &'a TopologyGraph,
    finding_ids: BTreeSet<String>,
    assertion_ids: BTreeSet<String>,
    artifact_ids: BTreeSet<String>,
    trace_ids: BTreeSet<String>,
    relation_edge_ids: BTreeSet<String>,
    relation_claim_ids: BTreeSet<String>,
    proposition_ids: BTreeSet<String>,
    attachments: BTreeMap<String, LocationAttachment>,
    edges: BTreeMap<String, RelationEdge>,
    claims: BTreeMap<String, RelationClaim>,
    unresolved: BTreeMap<String, UnresolvedRelation>,
    providers: BTreeMap<(String, Option<String>), ProviderAccumulator>,
    source_verification: SourceVerificationReceipt,
    explicit_conflicts: BTreeSet<String>,
}

#[derive(Default)]
struct ProviderAccumulator {
    producer_ids: BTreeSet<String>,
    artifact_ids: BTreeSet<String>,
}

impl<'a> ReceiptBuilder<'a> {
    fn new(graph: &'a TopologyGraph) -> Self {
        Self {
            graph,
            finding_ids: BTreeSet::new(),
            assertion_ids: BTreeSet::new(),
            artifact_ids: BTreeSet::new(),
            trace_ids: BTreeSet::new(),
            relation_edge_ids: BTreeSet::new(),
            relation_claim_ids: BTreeSet::new(),
            proposition_ids: BTreeSet::new(),
            attachments: BTreeMap::new(),
            edges: BTreeMap::new(),
            claims: BTreeMap::new(),
            unresolved: BTreeMap::new(),
            providers: BTreeMap::new(),
            source_verification: SourceVerificationReceipt {
                status: SourceVerificationStatus::NotRequested,
                path: None,
                content_digest: None,
            },
            explicit_conflicts: BTreeSet::new(),
        }
    }

    fn collect_payload(&mut self, payload: &QueryPayload) {
        match payload {
            QueryPayload::Find(payload) => self.collect_find(payload),
            QueryPayload::Inspect(payload) => self.collect_inspect(payload),
            QueryPayload::Traverse(payload) => match payload {
                entmoot_contracts::query::TraversePayload::Path(result) => {
                    for path in &result.paths {
                        self.collect_path(path);
                    }
                }
                entmoot_contracts::query::TraversePayload::Neighborhood(result) => {
                    for hit in &result.items {
                        self.collect_path(&hit.witness);
                        for anchor in &hit.role_anchors {
                            self.collect_anchor(anchor);
                        }
                    }
                    if let Some(selection) = &result.role_selection {
                        for anchor in &selection.colocated {
                            self.collect_anchor(anchor);
                        }
                    }
                    for attempt in &result.unresolved_calls {
                        self.collect_unresolved(attempt.get());
                    }
                }
            },
            QueryPayload::Read(payload) => match payload {
                entmoot_contracts::query::ReadPayload::Source(source) => {
                    self.source_verification = SourceVerificationReceipt {
                        status: if source.content_digest.is_some() {
                            SourceVerificationStatus::Verified
                        } else {
                            SourceVerificationStatus::Unavailable
                        },
                        path: Some(source.path.clone()),
                        content_digest: source.content_digest.clone(),
                    };
                }
            },
        }
    }

    fn collect_path(&mut self, path: &entmoot_contracts::query::TraversePath) {
        for hop in &path.hops {
            for assertion in &hop.endpoint_assertions {
                self.collect_assertion(assertion.get());
            }
            for attempt in &hop.unresolved_calls {
                self.collect_unresolved(attempt.get());
            }
            self.collect_edge(hop.relation_edge.get());
            self.relation_claim_ids
                .extend(hop.relation_claim_ids.iter().cloned());
        }
    }

    fn collect_anchor(&mut self, anchor: &entmoot_contracts::query::SeedAnchor) {
        self.collect_assertion(anchor.assertion.get());
        for attachment in &anchor.attachments {
            self.collect_attachment(attachment.get());
        }
    }

    fn collect_find(&mut self, payload: &FindPayload) {
        match payload {
            FindPayload::Subject(_) => {}
            FindPayload::Location(location) => {
                self.source_verification = SourceVerificationReceipt {
                    status: if location.content_digest.is_some()
                        || location
                            .gaps
                            .contains(&entmoot_contracts::query::QueryGapCode::LocationOutOfRange)
                    {
                        SourceVerificationStatus::Verified
                    } else {
                        SourceVerificationStatus::Unavailable
                    },
                    path: Some(location.path.clone()),
                    content_digest: location.content_digest.clone().or_else(|| {
                        self.graph
                            .snapshot_files
                            .iter()
                            .find(|file| file.path == location.path)
                            .map(|file| file.content_digest.clone())
                    }),
                };
            }
            FindPayload::Finding(result) => {
                for item in &result.items {
                    self.collect_finding(item.finding.get());
                    for attachment in &item.attachments {
                        self.collect_attachment(attachment.get());
                    }
                }
            }
            FindPayload::EvidenceRole(result) => {
                for item in &result.items {
                    self.collect_assertion(item.assertion.get());
                    for attachment in &item.attachments {
                        self.collect_attachment(attachment.get());
                    }
                    for finding in &item.findings {
                        self.collect_finding(finding.get());
                    }
                    for step in &item.trace_steps {
                        self.trace_ids.insert(step.get().trace_id.clone());
                    }
                }
            }
            FindPayload::Facet(result) => {
                for item in &result.items {
                    if let Some(detail) = &item.boundary {
                        for evaluation in &detail.evaluations {
                            self.artifact_ids
                                .insert(evaluation.get().artifact_id.clone());
                        }
                        for attempt in &detail.unresolved {
                            self.collect_unresolved(attempt.get());
                        }
                    }
                }
            }
        }
    }

    fn collect_inspect(&mut self, payload: &InspectPayload) {
        match payload {
            InspectPayload::Subject(result) => {
                for edge in &result.relations {
                    self.collect_edge(edge.get());
                }
                for claim in &result.relation_claims {
                    self.collect_claim(claim.get());
                }
                for unresolved in &result.unresolved_relations {
                    self.collect_unresolved(unresolved.get());
                }
                for attached in &result.direct_findings {
                    self.collect_attached_finding(attached);
                }
                for inherited in &result.inherited_file_findings {
                    self.collect_attached_finding(&inherited.attached);
                }
            }
            InspectPayload::Evidence(result) => match &result.body {
                EvidenceInspectBody::Finding {
                    finding,
                    artifact,
                    attachments,
                    traces,
                    ..
                } => {
                    self.collect_finding(finding.get());
                    if let Some(artifact) = artifact {
                        self.artifact_ids.insert(artifact.get().artifact_id.clone());
                    }
                    for attachment in attachments {
                        self.collect_attachment(attachment.get());
                    }
                    for trace in traces {
                        self.trace_ids.insert(trace.get().trace_id.clone());
                    }
                }
                EvidenceInspectBody::Assertions { assertions } => {
                    for assertion in assertions {
                        self.collect_assertion(assertion.get());
                    }
                }
                EvidenceInspectBody::Missing { .. } => {}
            },
            InspectPayload::Trace(result) => {
                if let Some(finding) = &result.finding {
                    self.collect_finding(finding.get());
                }
                for trace in &result.traces {
                    self.trace_ids.insert(trace.get().trace_id.clone());
                }
                for item in &result.items {
                    self.trace_ids.insert(item.step.get().trace_id.clone());
                    for attachment in &item.attachments {
                        self.collect_attachment(attachment.get());
                    }
                }
            }
            InspectPayload::Proposition(result) => {
                self.proposition_ids.insert(result.proposition_id.clone());
                for edge in &result.occurrences {
                    self.collect_edge(edge.get());
                }
                for claim in &result.relation_claims {
                    self.collect_claim(claim.get());
                }
            }
            InspectPayload::RelationClaims(result) => {
                self.relation_edge_ids
                    .insert(result.relation_edge_id.clone());
                for claim in &result.relation_claims {
                    self.collect_claim(claim.get());
                }
            }
            InspectPayload::Unresolved(result) => {
                for attempt in &result.items {
                    self.collect_unresolved(attempt.get());
                }
            }
            InspectPayload::Calls(result) => {
                for item in &result.items {
                    self.collect_edge(item.relation.get());
                    for claim in &item.claims {
                        self.collect_claim(claim.get());
                    }
                }
                for unresolved in &result.unresolved_relations {
                    self.collect_unresolved(unresolved.get());
                }
            }
        }
    }

    fn collect_attached_finding(&mut self, attached: &entmoot_contracts::query::AttachedFinding) {
        self.collect_finding(attached.finding.get());
        for attachment in &attached.attachments {
            self.collect_attachment(attachment.get());
        }
    }

    fn collect_finding(&mut self, finding: &FindingObservation) {
        self.finding_ids.insert(finding.observation_id.clone());
        self.artifact_ids.insert(finding.artifact_id.clone());
        self.trace_ids.extend(finding.trace_ids.iter().cloned());
    }

    fn collect_assertion(&mut self, assertion: &Assertion) {
        self.assertion_ids.insert(assertion.assertion_id.clone());
        for evidence in &assertion.evidence_refs {
            self.artifact_ids.insert(evidence.artifact_id.clone());
        }
        if let Some(derivation) = &assertion.derivation {
            for evidence in &derivation.input_refs {
                self.artifact_ids.insert(evidence.artifact_id.clone());
            }
        }
    }

    fn collect_attachment(&mut self, attachment: &LocationAttachment) {
        self.artifact_ids.insert(attachment.artifact_id.clone());
        self.attachments
            .insert(attachment.attachment_id.clone(), attachment.clone());
    }

    fn collect_edge(&mut self, edge: &RelationEdge) {
        self.relation_edge_ids.insert(edge.id.as_str().to_owned());
        self.proposition_ids
            .insert(edge.proposition_id.as_str().to_owned());
        self.edges.insert(edge.id.as_str().to_owned(), edge.clone());
    }

    fn collect_claim(&mut self, claim: &RelationClaim) {
        self.relation_claim_ids.insert(claim.id.as_str().to_owned());
        for evidence in &claim.evidence {
            self.artifact_ids.insert(evidence.artifact_id.clone());
        }
        if let Some(provenance) = &claim.provenance {
            self.artifact_ids
                .extend(provenance.artifacts.iter().cloned());
        }
        self.collect_producer(&claim.producer, None);
        self.claims
            .insert(claim.id.as_str().to_owned(), claim.clone());
    }

    fn collect_unresolved(&mut self, unresolved: &UnresolvedRelation) {
        for evidence in &unresolved.evidence {
            self.artifact_ids.insert(evidence.artifact_id.clone());
        }
        if let Some(provenance) = &unresolved.provenance {
            self.artifact_ids
                .extend(provenance.artifacts.iter().cloned());
        }
        self.collect_producer(&unresolved.producer, None);
        self.unresolved
            .insert(unresolved.id.as_str().to_owned(), unresolved.clone());
    }

    fn collect_producer(&mut self, producer: &ProducerRef, artifact_id: Option<&str>) {
        let entry = self
            .providers
            .entry((producer.name.clone(), producer.version.clone()))
            .or_default();
        entry.producer_ids.insert(producer.id.clone());
        if let Some(artifact_id) = artifact_id {
            entry.artifact_ids.insert(artifact_id.to_owned());
        }
    }

    fn hydrate_relation_claims(
        &mut self,
        repository: &impl KnowledgeRepository,
    ) -> Result<(), QueryError> {
        self.explicit_conflicts = super::conflicts::retained_conflicts(self.graph, repository)?;
        let edges: Vec<_> = self.edges.values().cloned().collect();
        let references: Vec<_> = edges.iter().collect();
        for claim in repository
            .relation_claims_for_edges(&references)?
            .into_iter()
            .flatten()
        {
            self.collect_claim(&claim);
        }
        Ok(())
    }

    fn finish(mut self) -> QueryEvidenceReceipt {
        for artifact in &self.graph.artifacts {
            if !self.artifact_ids.contains(&artifact.artifact_id) {
                continue;
            }
            let entry = self
                .providers
                .entry((
                    artifact.producer.name.clone(),
                    artifact.producer.version.clone(),
                ))
                .or_default();
            entry.artifact_ids.insert(artifact.artifact_id.clone());
        }

        let mut nested_provider_truncated = false;
        let provider_values = self
            .providers
            .into_iter()
            .map(|((name, version), values)| {
                let (producer_ids, producer_ids_truncated) =
                    bounded(values.producer_ids.into_iter().collect());
                let (artifact_ids, artifact_ids_truncated) =
                    bounded(values.artifact_ids.into_iter().collect());
                nested_provider_truncated |= producer_ids_truncated || artifact_ids_truncated;
                ProviderEvidenceReceipt {
                    name,
                    version,
                    producer_ids,
                    artifact_ids,
                }
            })
            .collect();
        let (providers, provider_count_truncated) = bounded(provider_values);
        let providers_truncated = nested_provider_truncated || provider_count_truncated;
        let mut coverage: Vec<_> = self
            .graph
            .resolver_coverages
            .iter()
            .filter(|item| self.artifact_ids.contains(&item.artifact_id))
            .cloned()
            .map(SchemaJson::new)
            .collect();
        coverage.sort_by(|left, right| left.get().coverage_id.cmp(&right.get().coverage_id));
        let (resolver_coverage, resolver_coverage_truncated) = bounded(coverage);
        let attachments = attachment_summary(self.attachments.values());
        let relations = relation_summary(
            &self.edges,
            &self.claims,
            &self.relation_edge_ids,
            &self.explicit_conflicts,
        );
        let unresolved = unresolved_summary(&self.unresolved);
        let (finding_ids, finding_truncated) = bounded(self.finding_ids.into_iter().collect());
        let (assertion_ids, assertion_truncated) =
            bounded(self.assertion_ids.into_iter().collect());
        let (artifact_ids, artifact_truncated) = bounded(self.artifact_ids.into_iter().collect());
        let (trace_ids, trace_truncated) = bounded(self.trace_ids.into_iter().collect());
        let (relation_edge_ids, relation_truncated) =
            bounded(self.relation_edge_ids.into_iter().collect());
        let (relation_claim_ids, relation_claim_truncated) =
            bounded(self.relation_claim_ids.into_iter().collect());
        let (proposition_ids, proposition_truncated) =
            bounded(self.proposition_ids.into_iter().collect());

        QueryEvidenceReceipt {
            snapshot: SchemaJson::new(self.graph.snapshot.clone()),
            references: EvidenceReceipt {
                finding_ids,
                assertion_ids,
                artifact_ids,
                trace_ids,
                relation_edge_ids,
                relation_claim_ids,
                proposition_ids,
            },
            references_truncated: finding_truncated
                || assertion_truncated
                || artifact_truncated
                || trace_truncated
                || relation_truncated
                || relation_claim_truncated
                || proposition_truncated,
            providers,
            providers_truncated,
            attachments,
            relations,
            unresolved,
            resolver_coverage,
            resolver_coverage_truncated,
            source_verification: self.source_verification,
        }
    }
}

fn attachment_summary<'a>(
    attachments: impl Iterator<Item = &'a LocationAttachment>,
) -> AttachmentEvidenceSummary {
    let attachments: Vec<_> = attachments.collect();
    let dispositions = [
        AttachmentDisposition::Attached,
        AttachmentDisposition::Excluded,
        AttachmentDisposition::Unresolved,
        AttachmentDisposition::Invalid,
    ]
    .into_iter()
    .filter_map(|disposition| {
        let count = attachments
            .iter()
            .filter(|item| item.disposition == disposition)
            .count();
        (count > 0).then_some(AttachmentDispositionCount { disposition, count })
    })
    .collect();
    let selected_grains = [
        AttachmentTargetKind::CallSite,
        AttachmentTargetKind::Callable,
        AttachmentTargetKind::File,
        AttachmentTargetKind::Module,
        AttachmentTargetKind::Resource,
        AttachmentTargetKind::Opaque,
    ]
    .into_iter()
    .filter_map(|target_kind| {
        let count = attachments
            .iter()
            .filter(|item| {
                item.selected_target
                    .as_ref()
                    .is_some_and(|target| target.kind == target_kind)
            })
            .count();
        (count > 0).then_some(AttachmentGrainCount { target_kind, count })
    })
    .collect();
    let freshness = [
        AttachmentFreshness::Current,
        AttachmentFreshness::StaleSnapshot,
        AttachmentFreshness::StaleBody,
        AttachmentFreshness::DegradedOverride,
    ]
    .into_iter()
    .filter(|state| attachments.iter().any(|item| item.freshness == *state))
    .collect();
    AttachmentEvidenceSummary {
        attachment_count: attachments.len(),
        dispositions,
        selected_grains,
        freshness,
    }
}

fn relation_summary(
    edges: &BTreeMap<String, RelationEdge>,
    claims: &BTreeMap<String, RelationClaim>,
    referenced_edge_ids: &BTreeSet<String>,
    explicit_conflicts: &BTreeSet<String>,
) -> RelationEvidenceSummary {
    let active_claims: Vec<_> = claims
        .values()
        .filter(|claim| claim.state == RelationClaimState::Active)
        .collect();
    let mut states = Vec::new();
    let mut candidate = Vec::new();
    let mut conflicted = Vec::new();
    let mut stale = Vec::new();
    let mut unclaimed = Vec::new();
    for (edge_id, edge) in edges {
        let mut edge_states = Vec::new();
        for claim in &active_claims {
            if let Some(state) = super::applicable_resolution_state(claim, edge) {
                if !edge_states.contains(&state) {
                    edge_states.push(state);
                }
                if !states.contains(&state) {
                    states.push(state);
                }
            }
        }
        if edge_states.is_empty() {
            unclaimed.push(edge_id.clone());
        }
        if edge_states.contains(&ResolutionState::Candidate) {
            candidate.push(edge_id.clone());
        }
        if edge_states.contains(&ResolutionState::Stale) {
            stale.push(edge_id.clone());
        }
        if edge_states.len() > 1
            || edge_states.contains(&ResolutionState::Refuted)
            || explicit_conflicts.contains(edge_id)
        {
            conflicted.push(edge_id.clone());
        }
    }
    states.sort_by_key(resolution_state_order);
    let edge_count = referenced_edge_ids.len().max(edges.len());
    let (candidate_relation_edge_ids, candidate_truncated) = bounded(candidate);
    let (conflicted_relation_edge_ids, conflicted_truncated) = bounded(conflicted);
    let (stale_relation_edge_ids, stale_truncated) = bounded(stale);
    let (unclaimed_relation_edge_ids, unclaimed_truncated) = bounded(unclaimed);
    RelationEvidenceSummary {
        relation_edge_count: edge_count,
        relation_claim_count: claims.len(),
        active_claim_count: active_claims.len(),
        active_resolution_states: states,
        candidate_relation_edge_ids,
        conflicted_relation_edge_ids,
        stale_relation_edge_ids,
        unclaimed_relation_edge_ids,
        truncated: candidate_truncated
            || conflicted_truncated
            || stale_truncated
            || unclaimed_truncated,
    }
}

fn unresolved_summary(
    unresolved: &BTreeMap<String, UnresolvedRelation>,
) -> UnresolvedEvidenceSummary {
    let mut reasons = Vec::new();
    for item in unresolved.values() {
        if !reasons.contains(&item.reason) {
            reasons.push(item.reason);
        }
    }
    reasons.sort_by_key(unresolved_reason_order);
    let (unresolved_relation_ids, truncated) = bounded(unresolved.keys().cloned().collect());
    UnresolvedEvidenceSummary {
        count: unresolved.len(),
        unresolved_relation_ids,
        reasons,
        truncated,
    }
}

fn resolution_state_order(state: &ResolutionState) -> u8 {
    match state {
        ResolutionState::Structural => 0,
        ResolutionState::Candidate => 1,
        ResolutionState::ArtifactResolved => 2,
        ResolutionState::SemanticResolved => 3,
        ResolutionState::Validated => 4,
        ResolutionState::Refuted => 5,
        ResolutionState::Stale => 6,
        ResolutionState::Unknown => 7,
    }
}

fn unresolved_reason_order(reason: &UnresolvedReason) -> u8 {
    match reason {
        UnresolvedReason::IndirectCall => 0,
        UnresolvedReason::ProviderError => 11,
        UnresolvedReason::UnresolvedBinding => 1,
        UnresolvedReason::AmbiguousTarget => 2,
        UnresolvedReason::TargetNotInInventory => 3,
        UnresolvedReason::ProducerNotInvoked => 4,
        UnresolvedReason::UnsupportedProducer => 5,
        UnresolvedReason::OutOfScope => 6,
        UnresolvedReason::DispatchNotEvaluated => 7,
        UnresolvedReason::BudgetExceeded => 8,
        UnresolvedReason::InvalidEnvironment => 9,
        UnresolvedReason::UnsupportedConstruct => 10,
    }
}

fn bounded<T>(mut values: Vec<T>) -> (Vec<T>, bool) {
    let truncated = values.len() > RECEIPT_ITEM_LIMIT;
    values.truncate(RECEIPT_ITEM_LIMIT);
    (values, truncated)
}
