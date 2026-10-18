use entmoot_contracts::{
    address_identity_key, hash_parts, relation_claim_with_id, relation_edge_with_id,
    unresolved_relation_with_id, ArtifactFormat, ArtifactManifest, EvidenceRef,
    ProducerCapabilityKind, ProducerRef, Provenance, RelationClaimValue, RelationKind, Resolution,
    ResolutionState, ResolverArtifact, ResolverCoverageObservation, ResolverObservationOutcome,
    Scope, SubjectId, TargetConstraint, UnresolvedReason,
};

use super::{
    decode_text_bytes, ArtifactAdapter, ArtifactError, InspectedArtifact, NormalizedArtifact,
};

#[derive(Debug, Clone, Copy)]
pub struct ResolverArtifactAdapter;

#[derive(Default)]
pub struct InventoryBindings<'a> {
    sites: std::collections::HashMap<String, (&'a crate::TopologyNode, &'a crate::TopologyNode)>,
    sources: std::collections::HashMap<String, &'a crate::TopologyNode>,
    callable_targets: std::collections::HashMap<String, &'a crate::TopologyNode>,
}

/// Join native attempts to immutable inventory before graph mutation. Provider
/// record IDs remain in evidence pointers; projections and resolved identities use
/// the actual callsite ID. The immutable artifact retains every original address.
pub fn join_inventory_bindings<'a>(
    graph: &'a crate::TopologyGraph,
    manifest: &ArtifactManifest,
    language: &str,
    observations: &[entmoot_contracts::ResolverObservation],
) -> Result<InventoryBindings<'a>, ArtifactError> {
    use crate::NodeKind;
    use entmoot_contracts::{topology_node_address, Address};
    if observations.is_empty() {
        return Ok(InventoryBindings::default());
    }
    let snapshot_paths: std::collections::HashSet<_> = graph
        .snapshot_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let mut nodes: std::collections::HashMap<String, Vec<&crate::TopologyNode>> =
        std::collections::HashMap::new();
    for node in &graph.nodes {
        let mut addresses = vec![topology_node_address(&node.id)];
        if let Some(value) = &node.logic_address {
            addresses.push(Address::LogicAddress {
                logic_address: value.clone(),
            });
        }
        if let Some(value) = &node.address_id {
            addresses.push(Address::AddressId {
                address_id: value.clone(),
            });
        }
        if let Some(path) = &node.path {
            for repository_id in [None, Some(graph.snapshot.repository_id.clone())] {
                if let (Some(start), Some(end)) = (
                    node.start_byte.and_then(|value| u32::try_from(value).ok()),
                    node.end_byte.and_then(|value| u32::try_from(value).ok()),
                ) {
                    addresses.push(Address::RepoPath {
                        repository_id,
                        path: path.clone(),
                        start_byte: Some(start),
                        end_byte: Some(end),
                    });
                }
            }
        }
        for address in addresses {
            nodes
                .entry(address_identity_key(&address))
                .or_default()
                .push(node);
        }
    }
    let unique = |address: &Address| -> Result<&crate::TopologyNode, ArtifactError> {
        match nodes.get(&address_identity_key(address)).map(Vec::as_slice) {
            Some([node]) => Ok(*node),
            _ => Err(ArtifactError::Message(
                "resolver source/claimed callable does not uniquely match inventory".into(),
            )),
        }
    };
    let mut joined = InventoryBindings::default();
    let mut claimed_sites = std::collections::HashSet::new();
    for observation in observations {
        let site = unique(&observation.source.call_site)?;
        let owner = unique(&observation.source.relation_source)?;
        if site.kind != NodeKind::CallStub
            || site.label != observation.source.syntactic_callee
            || owner.id != site.parent.as_deref().unwrap_or(&site.id)
        {
            return Err(ArtifactError::Message(
                "resolver call site, callee, or owner disagrees with inventory".into(),
            ));
        }
        if !claimed_sites.insert(site.id.as_str()) {
            return Err(ArtifactError::Message(
                "multiple resolver observations alias the same inventory call site".into(),
            ));
        }
        joined
            .sites
            .insert(observation.occurrence_id.clone(), (site, owner));
        joined
            .sources
            .insert(address_identity_key(&observation.source.call_site), site);
        let path = site.path.as_deref().ok_or_else(|| {
            ArtifactError::Message("resolver source has no inventory path".into())
        })?;
        if !entmoot_core::provider_language::language_matches(language, path) {
            return Err(ArtifactError::Message(
                "resolver language disagrees with inventory call site".into(),
            ));
        }
        if !manifest.path_in_scope(path)? {
            return Err(ArtifactError::Message(
                "resolver source is outside declared artifact scope".into(),
            ));
        }
        for target in &observation.targets {
            let inventory_identity = matches!(
                target.address,
                Address::LogicAddress { .. } | Address::AddressId { .. }
            ) || matches!(&target.address, Address::Opaque { system, .. } if system == "entmoot.topology_node");
            if matches!(
                observation.outcome,
                ResolverObservationOutcome::RepositoryCallable
                    | ResolverObservationOutcome::RepositorySymbol
            ) && inventory_identity
            {
                let declaration = unique(&target.address)?;
                if !matches!(declaration.kind, NodeKind::Callable | NodeKind::Declaration) {
                    return Err(ArtifactError::Message(
                        "resolver repository target is not an inventoried declaration".into(),
                    ));
                }
            }
            if let entmoot_contracts::Address::RepoPath { path, .. } = &target.address {
                if matches!(
                    observation.outcome,
                    ResolverObservationOutcome::RepositoryCallable
                        | ResolverObservationOutcome::RepositorySymbol
                ) && !snapshot_paths.contains(path.as_str())
                {
                    return Err(ArtifactError::Message(
                        "resolver repository target is outside the indexed source manifest".into(),
                    ));
                }
            }
            if let Some(address) = &target.relation_target {
                let callable = unique(address)?;
                if callable.kind != NodeKind::Callable {
                    return Err(ArtifactError::Message(
                        "resolver callable target is not an inventoried callable".into(),
                    ));
                }
                joined
                    .callable_targets
                    .insert(observation.occurrence_id.clone(), callable);
                if let Ok(declaration) = unique(&target.address) {
                    if declaration.kind == NodeKind::Callable && declaration.id != callable.id {
                        return Err(ArtifactError::Message(
                            "resolver definition identity disagrees with claimed callable".into(),
                        ));
                    }
                }
                if let entmoot_contracts::Address::RepoPath {
                    path, start_byte, ..
                } = &target.address
                {
                    if callable.path.as_deref() != Some(path.as_str())
                        || !start_byte.is_some_and(|start| {
                            callable
                                .start_byte
                                .zip(callable.end_byte)
                                .is_some_and(|(low, high)| {
                                    low <= start as usize && (start as usize) < high
                                })
                        })
                    {
                        return Err(ArtifactError::Message(
                            "resolver definition location disagrees with claimed callable".into(),
                        ));
                    }
                }
            }
            if let Address::RepoPath {
                repository_id: Some(repository),
                ..
            } = &target.address
            {
                if matches!(
                    observation.outcome,
                    ResolverObservationOutcome::RepositoryCallable
                        | ResolverObservationOutcome::RepositorySymbol
                ) && repository != &manifest.snapshot.repository_id
                {
                    return Err(ArtifactError::Message(
                        "resolver repository target names another repository".into(),
                    ));
                }
            }
        }
    }

    Ok(joined)
}

impl InventoryBindings<'_> {
    pub fn site(&self, provider_record_id: &str) -> Option<&crate::TopologyNode> {
        self.sites.get(provider_record_id).map(|(site, _)| *site)
    }

    /// Validation is complete before rewriting derived records. Raw provider
    /// targets are never canonicalized by spelling or a containing-span guess.
    pub fn project(&self, normalized: &mut NormalizedArtifact) -> Result<(), ArtifactError> {
        use entmoot_contracts::topology_node_address;
        let mut edge_ids = std::collections::HashMap::new();
        for edge in &mut normalized.relation_edges {
            if edge.predicate != RelationKind::Calls {
                continue;
            }
            let Some((site, owner)) = edge
                .occurrence_id
                .as_ref()
                .and_then(|id| self.sites.get(id))
            else {
                return Err(ArtifactError::Message(
                    "resolver call projection has no validated observation".into(),
                ));
            };
            let object = edge
                .occurrence_id
                .as_ref()
                .and_then(|id| self.callable_targets.get(id))
                .map(|target| topology_node_address(&target.id))
                .unwrap_or_else(|| edge.object.clone());
            let projected = relation_edge_with_id(
                &edge.schema_domain,
                topology_node_address(&owner.id),
                edge.predicate,
                object,
                edge.scope.clone(),
                Some(site.id.clone()),
            );
            edge_ids.insert(edge.id.clone(), projected.id.clone());
            *edge = projected;
        }
        for claim in &mut normalized.relation_claims {
            if let SubjectId::RelationEdge { edge_id } = &mut claim.subject {
                if let Some(projected_id) = edge_ids.get(edge_id) {
                    *edge_id = projected_id.clone();
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
        for relation in &mut normalized.unresolved_relations {
            if relation.predicate != RelationKind::Calls {
                continue;
            }
            if let Some((site, owner)) = relation
                .occurrence_id
                .as_ref()
                .and_then(|id| self.sites.get(id))
            {
                *relation = unresolved_relation_with_id(
                    topology_node_address(&owner.id),
                    relation.predicate,
                    relation
                        .occurrence_id
                        .as_ref()
                        .and_then(|id| self.callable_targets.get(id))
                        .map(|target| TargetConstraint::Candidates {
                            addresses: vec![topology_node_address(&target.id)],
                        })
                        .unwrap_or_else(|| relation.target_constraint.clone()),
                    relation.reason,
                    relation.producer.clone(),
                    relation.scope.clone(),
                    Some(site.id.clone()),
                    relation.evidence.clone(),
                    relation.provenance.clone(),
                    relation.remaining_capability,
                );
            }
        }
        for resolution in &mut normalized.resolutions {
            let site = self
                .sources
                .get(&address_identity_key(&resolution.from))
                .ok_or_else(|| {
                    ArtifactError::Message("resolver mapping has no validated call site".into())
                })?;
            resolution.from = topology_node_address(&site.id);
        }
        for observation in &mut normalized.resolver_observations {
            let (site, owner) = self.sites.get(&observation.occurrence_id).ok_or_else(|| {
                ArtifactError::Message("resolver observation lost its inventory join".into())
            })?;
            if let Some(callable) = self.callable_targets.get(&observation.occurrence_id) {
                for target in &mut observation.targets {
                    target.relation_target = Some(topology_node_address(&callable.id));
                }
            }
            observation.occurrence_id = site.id.clone();
            observation.source.call_site = topology_node_address(&site.id);
            observation.source.relation_source = topology_node_address(&owner.id);
        }
        Ok(())
    }
}

pub fn parse_scoped_artifact(
    bytes: &[u8],
    manifest: &ArtifactManifest,
) -> Result<ResolverArtifact, ArtifactError> {
    if manifest.format != ArtifactFormat::Resolver {
        return Err(ArtifactError::Message(format!(
            "ResolverArtifactAdapter requires format resolver, got {:?}",
            manifest.format
        )));
    }
    let text = decode_text_bytes(bytes)?;
    let artifact: ResolverArtifact = serde_json::from_str(&text)?;
    if artifact.implementation != crate::implementation_fingerprint() {
        return Err(ArtifactError::Message(
            "resolver artifact implementation fingerprint mismatch".into(),
        ));
    }
    artifact
        .validate()
        .map_err(|error| ArtifactError::Message(error.to_string()))?;
    if artifact.snapshot != manifest.snapshot {
        return Err(ArtifactError::Message(
            "resolver artifact snapshot does not match manifest snapshot".into(),
        ));
    }
    if !manifest.scope.languages.is_empty()
        && !manifest.scope.languages.contains(&artifact.language)
    {
        return Err(ArtifactError::Message(
            "resolver language is outside declared artifact scope".into(),
        ));
    }
    if artifact.producer.name != manifest.producer.name
        || manifest.producer.version.as_deref() != Some(artifact.producer.version.as_str())
    {
        return Err(ArtifactError::Message(
            "resolver artifact producer does not match manifest producer".into(),
        ));
    }
    Ok(artifact)
}

impl ArtifactAdapter for ResolverArtifactAdapter {
    fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, ArtifactError> {
        let text = decode_text_bytes(bytes)?;
        let artifact: ResolverArtifact = serde_json::from_str(&text)?;
        if artifact.implementation != crate::implementation_fingerprint() {
            return Err(ArtifactError::Message(
                "resolver artifact implementation fingerprint mismatch".into(),
            ));
        }
        artifact
            .validate()
            .map_err(|error| ArtifactError::Message(error.to_string()))?;
        Ok(InspectedArtifact {
            format_hint: "resolver".into(),
            producer_name: Some(artifact.producer.name),
            producer_version: Some(artifact.producer.version),
            finding_count: 0,
            location_count: 0,
            trace_count: 0,
            document_count: 0,
            occurrence_count: artifact.observations.len(),
            diagnostics: vec![],
        })
    }

    fn normalize(
        &self,
        bytes: &[u8],
        manifest: &ArtifactManifest,
    ) -> Result<NormalizedArtifact, ArtifactError> {
        let artifact = parse_scoped_artifact(bytes, manifest)?;

        let producer = ProducerRef {
            id: artifact.producer.id.clone(),
            name: artifact.producer.name.clone(),
            version: Some(artifact.producer.version.clone()),
            config_fingerprint: Some(format!(
                "blake3:{}",
                hash_parts(&[
                    "entmoot.resolver_projection.v1",
                    &artifact.producer.method_id,
                    manifest.config_digest.as_deref().unwrap_or(""),
                ])
            )),
            environment_fingerprint: Some(artifact.build_context_digest.to_string()),
            capabilities: vec![
                ProducerCapabilityKind::Resolver,
                ProducerCapabilityKind::ArtifactAdapter,
            ],
        };
        let scope = Scope {
            domain: manifest.snapshot.repository_id.clone(),
            snapshot_revision: Some(manifest.snapshot.revision.clone()),
            snapshot_digest: manifest.snapshot.snapshot_digest.clone(),
            occurrence_id: None,
            context_id: Some(artifact.build_context_digest.to_string()),
        };
        let provenance = Provenance {
            snapshot_revision: Some(manifest.snapshot.revision.clone()),
            snapshot_digest: manifest.snapshot.snapshot_digest.clone(),
            artifacts: vec![manifest.artifact_id.clone()],
            tool_digest: manifest.config_digest.clone(),
            produced_at: manifest.generated_at.clone(),
        };
        let mut normalized = NormalizedArtifact {
            source_record_count: artifact.observations.len() + artifact.assessments.len(),
            assessment_evidence: Some(entmoot_contracts::AssessmentEvidence {
                schema_version: "entmoot.assessment-evidence.v1".into(),
                implementation: artifact.implementation.clone(),
                snapshot: artifact.snapshot.clone(),
                producer: producer.clone(),
                tools: vec![],
                build_context: artifact.build_context_digest.clone(),
                observations: artifact.assessments.clone(),
            }),
            ..Default::default()
        };
        let mut coverage = ResolverCoverageObservation {
            coverage_id: String::new(),
            artifact_id: manifest.artifact_id.clone(),
            scope_universe_id: manifest.scope.scope_universe_id.clone(),
            producer_id: artifact.producer.id.clone(),
            producer_version: Some(artifact.producer.version.clone()),
            language: artifact.language.clone(),
            language_version: artifact.language_version.clone(),
            build_context_digest: artifact.build_context_digest.clone(),
            call_sites_observed: artifact.observations.len() as u64,
            repository_occurrences: 0,
            indexed_dependency_occurrences: 0,
            external_symbols_without_source: 0,
            intrinsics_without_callable_target: 0,
            unresolved_or_ambiguous: 0,
            unsupported: 0,
            failed: 0,
        };

        for (observation_index, observation) in artifact.observations.iter().enumerate() {
            let base_pointer = format!("/observations/{observation_index}");
            let evidence = |pointer: String| EvidenceRef {
                artifact_id: manifest.artifact_id.clone(),
                provider_pointer: pointer,
                record_id: Some(observation.occurrence_id.clone()),
            };
            match observation.outcome {
                ResolverObservationOutcome::RepositoryCallable => {
                    coverage.repository_occurrences += 1;
                }
                ResolverObservationOutcome::RepositorySymbol => {
                    coverage.repository_occurrences += 1;
                }
                ResolverObservationOutcome::IndexedDependency => {
                    coverage.indexed_dependency_occurrences += 1;
                }
                ResolverObservationOutcome::ExternalSymbol => {
                    coverage.external_symbols_without_source += 1;
                }
                ResolverObservationOutcome::Intrinsic => {
                    coverage.intrinsics_without_callable_target += 1;
                }
                ResolverObservationOutcome::Ambiguous | ResolverObservationOutcome::Unresolved => {
                    // Both outcomes describe incomplete binding, independently
                    // of dispatch indirection. Full outcomes remain attributable.
                    coverage.unresolved_or_ambiguous += 1;
                }
                ResolverObservationOutcome::Unsupported => coverage.unsupported += 1,
                ResolverObservationOutcome::Failed => coverage.failed += 1,
            }

            for (target_index, target) in observation.targets.iter().enumerate() {
                let pointer = format!("{base_pointer}/targets/{target_index}");
                normalized.resolutions.push(Resolution {
                    resolution_id: hash_parts(&[
                        "resolver_resolution",
                        &manifest.artifact_id,
                        &observation.occurrence_id,
                        &address_identity_key(&target.address),
                        &pointer,
                    ]),
                    from: observation.source.call_site.clone(),
                    to: target.address.clone(),
                    producer_id: artifact.producer.id.clone(),
                    producer_version: Some(artifact.producer.version.clone()),
                    evidence_refs: vec![evidence(pointer.clone())],
                    validity: None,
                    method_id: Some(artifact.producer.method_id.clone()),
                });
                // A binding is a possible call target, not proof of dispatch or execution.
                // Keep external APIs addressable without inventing repository callables.
                if matches!(
                    observation.outcome,
                    ResolverObservationOutcome::RepositoryCallable
                        | ResolverObservationOutcome::ExternalSymbol
                        | ResolverObservationOutcome::IndexedDependency
                        | ResolverObservationOutcome::Intrinsic
                        | ResolverObservationOutcome::Ambiguous
                ) {
                    let relation_target =
                        target.relation_target.as_ref().unwrap_or(&target.address);
                    let edge = relation_edge_with_id(
                        "source",
                        observation.source.relation_source.clone(),
                        RelationKind::Calls,
                        relation_target.clone(),
                        scope.clone(),
                        Some(observation.occurrence_id.clone()),
                    );
                    let edge_id = edge.id.clone();
                    normalized.relation_edges.push(edge);
                    normalized.relation_claims.push(relation_claim_with_id(
                        SubjectId::RelationEdge { edge_id },
                        "resolution_state",
                        RelationClaimValue::ResolutionState {
                            state: if observation.dispatch.is_closed() {
                                ResolutionState::ArtifactResolved
                            } else {
                                ResolutionState::Candidate
                            },
                        },
                        producer.clone(),
                        scope.clone(),
                        vec![evidence(pointer), evidence(base_pointer.clone())],
                        Some(provenance.clone()),
                    ));
                }
            }

            if !observation.dispatch.is_closed()
                || (observation.outcome != ResolverObservationOutcome::Intrinsic
                    && observation
                        .targets
                        .iter()
                        .any(|target| target.relation_target.is_none()))
            {
                let addresses = observation
                    .targets
                    .iter()
                    .map(|target| target.address.clone())
                    .collect::<Vec<_>>();
                let target_constraint = if addresses.is_empty() {
                    TargetConstraint::Syntactic {
                        callee: observation.source.syntactic_callee.clone(),
                    }
                } else {
                    TargetConstraint::Candidates { addresses }
                };
                let reason = match observation.outcome {
                    ResolverObservationOutcome::Ambiguous => UnresolvedReason::AmbiguousTarget,
                    ResolverObservationOutcome::Unsupported => {
                        UnresolvedReason::UnsupportedProducer
                    }
                    ResolverObservationOutcome::Failed | ResolverObservationOutcome::Unresolved => {
                        UnresolvedReason::UnresolvedBinding
                    }
                    ResolverObservationOutcome::RepositoryCallable
                    | ResolverObservationOutcome::Intrinsic => {
                        UnresolvedReason::DispatchNotEvaluated
                    }
                    _ => UnresolvedReason::TargetNotInInventory,
                };
                // Open dispatch reasons refine the frontier, but ambiguous or
                // unresolved binding outcomes must not be re-labeled as runtime
                // indirection when a producer attaches a mechanism by mistake.
                let reason = match &observation.dispatch {
                    entmoot_contracts::CallDispatch::Open { reason: gap } => match gap {
                        entmoot_contracts::DispatchGap::BudgetExceeded => {
                            UnresolvedReason::BudgetExceeded
                        }
                        entmoot_contracts::DispatchGap::InvalidEnvironment => {
                            UnresolvedReason::InvalidEnvironment
                        }
                        entmoot_contracts::DispatchGap::UnsupportedConstruct => {
                            UnresolvedReason::UnsupportedConstruct
                        }
                        entmoot_contracts::DispatchGap::ExternalImplementation => {
                            UnresolvedReason::TargetNotInInventory
                        }
                        entmoot_contracts::DispatchGap::IndirectCall(_) => {
                            // Candidate ambiguity is a binding outcome; do not
                            // let a mislabeled mechanism erase AmbiguousTarget.
                            // Honest indirect calls remain Unresolved + IndirectCall.
                            if matches!(observation.outcome, ResolverObservationOutcome::Ambiguous)
                            {
                                UnresolvedReason::AmbiguousTarget
                            } else {
                                UnresolvedReason::IndirectCall
                            }
                        }
                        entmoot_contracts::DispatchGap::UnresolvedBinding(_) => {
                            if matches!(observation.outcome, ResolverObservationOutcome::Ambiguous)
                            {
                                UnresolvedReason::AmbiguousTarget
                            } else {
                                UnresolvedReason::UnresolvedBinding
                            }
                        }
                        entmoot_contracts::DispatchGap::ProviderError => {
                            UnresolvedReason::ProviderError
                        }
                    },
                    _ => reason,
                };
                normalized
                    .unresolved_relations
                    .push(unresolved_relation_with_id(
                        observation.source.relation_source.clone(),
                        RelationKind::Calls,
                        target_constraint,
                        reason,
                        producer.clone(),
                        scope.clone(),
                        Some(observation.occurrence_id.clone()),
                        vec![evidence(base_pointer)],
                        Some(provenance.clone()),
                        Some(ProducerCapabilityKind::Resolver),
                    ));
            }
        }
        coverage.coverage_id = coverage.derive_id();
        coverage
            .validate()
            .map_err(|error| ArtifactError::Message(error.to_string()))?;
        normalized.resolver_coverages.push(coverage);
        normalized.resolver_observations = artifact.observations;
        if let Some(api) = &artifact.package_api {
            project_package_depends_on(
                &mut normalized,
                api,
                &scope,
                &manifest.artifact_id,
                &producer,
                &provenance,
            )?;
        }
        normalized.resolver_package_api = artifact.package_api;
        Ok(normalized)
    }
}

fn project_package_depends_on(
    normalized: &mut NormalizedArtifact,
    api: &entmoot_contracts::package_api::PackageApiEvidence,
    scope: &Scope,
    artifact_id: &str,
    producer: &ProducerRef,
    provenance: &Provenance,
) -> Result<(), ArtifactError> {
    use entmoot_contracts::Address;
    let mut emitted = std::collections::BTreeSet::<(String, String)>::new();
    for (index, reference) in api.references.iter().enumerate() {
        let Some(observation) = normalized
            .resolver_observations
            .iter()
            .find(|row| row.occurrence_id == reference.observation_id)
        else {
            continue;
        };
        let Ok(purl) = reference.target.package.coordinate_purl() else {
            continue;
        };
        let object = Address::Purl { purl: purl.clone() };
        if object.validate().is_err() {
            continue;
        }
        let subject_key = address_identity_key(&observation.source.relation_source);
        if !emitted.insert((subject_key, purl.clone())) {
            continue;
        }
        let edge = relation_edge_with_id(
            "source",
            observation.source.relation_source.clone(),
            RelationKind::DependsOn,
            object,
            scope.clone(),
            Some(observation.occurrence_id.clone()),
        );
        let edge_id = edge.id.clone();
        normalized.relation_edges.push(edge);
        normalized.relation_claims.push(relation_claim_with_id(
            SubjectId::RelationEdge { edge_id },
            "resolution_state",
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Candidate,
            },
            producer.clone(),
            scope.clone(),
            vec![EvidenceRef {
                artifact_id: artifact_id.into(),
                provider_pointer: format!("/package_api/references/{index}"),
                record_id: Some(observation.occurrence_id.clone()),
            }],
            Some(provenance.clone()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        blake3_hex, provider_symbol_schemes, topology_node_address, Address, ArtifactProducer,
        ContentDigest, ProviderSymbolSpelling, ResolverArtifactProducer, ResolverCallSource,
        ResolverObservation, ResolverTarget, ScopeCompleteness, ScopeUniverse, SourceSnapshot,
        SourceState, RESOLVER_ARTIFACT_SCHEMA,
    };

    fn snapshot() -> SourceSnapshot {
        SourceSnapshot {
            repository_id: "example/repo".into(),
            revision: "a".repeat(40),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        }
    }

    fn normalize_artifact(artifact: &ResolverArtifact) -> NormalizedArtifact {
        let bytes = serde_json::to_vec(artifact).unwrap();
        let scope = ScopeUniverse::from_selectors(
            "test",
            vec![artifact.language.clone()],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: artifact.producer.name.clone(),
                version: Some(artifact.producer.version.clone()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            artifact.snapshot.clone(),
            scope,
            blake3_hex(&bytes),
            None,
        )
        .unwrap();
        ResolverArtifactAdapter
            .normalize(&bytes, &manifest)
            .unwrap()
    }

    #[test]
    fn normalizes_positive_call_and_coverage() {
        let mut artifact = ResolverArtifact {
            assessments: vec![],
            package_api: None,
            schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
            implementation: crate::implementation_fingerprint(),
            producer: ResolverArtifactProducer {
                id: "pyright".into(),
                name: "pyright".into(),
                version: "1.1.414".into(),
                method_id: "definition.v1".into(),
            },
            snapshot: snapshot(),
            language: "python".into(),
            language_version: Some("3.13".into()),
            build_context_digest: ContentDigest::from_trusted(format!("blake3:{}", "b".repeat(64))),
            observations: vec![ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: "call".into(),
                source: ResolverCallSource {
                    call_site: topology_node_address("call"),
                    relation_source: topology_node_address("caller"),
                    syntactic_callee: "target".into(),
                },
                outcome: ResolverObservationOutcome::RepositoryCallable,
                targets: vec![ResolverTarget {
                    address: topology_node_address("target"),
                    relation_target: Some(topology_node_address("target")),
                    provider_pointer: "targets/0".into(),
                    provider_symbol: Some(ProviderSymbolSpelling {
                        scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                        spelling: "pkg.mod.target".into(),
                    }),
                    provider_callable: None,
                    declaration: None,
                }],
                limitation: None,
            }],
        };
        let normalized = normalize_artifact(&artifact);
        assert_eq!(normalized.resolutions.len(), 1);
        assert_eq!(normalized.relation_edges.len(), 1);
        assert_eq!(normalized.unresolved_relations.len(), 1);
        assert!(matches!(
            normalized.relation_claims[0].value,
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Candidate
            }
        ));
        assert_eq!(normalized.relation_claims.len(), 1);
        assert_eq!(normalized.resolver_coverages.len(), 1);
        assert_eq!(normalized.resolver_coverages[0].repository_occurrences, 1);
        artifact.observations[0].dispatch = entmoot_contracts::CallDispatch::Closed {
            policy_id: "fixture.dispatch.v1".into(),
            evidence_pointer: "/dispatch_evidence".into(),
            assumptions: vec!["fixed build; no runtime replacement".into()],
        };
        let closed = normalize_artifact(&artifact);
        assert!(closed.unresolved_relations.is_empty());
        assert!(matches!(
            closed.relation_claims[0].value,
            RelationClaimValue::ResolutionState {
                state: ResolutionState::ArtifactResolved
            }
        ));

        // A known binding can still have open dispatch. Projection must preserve
        // the shared category and the original mechanism-bearing observation.
        for (gap, expected) in [
            (
                "indirect_call:future_dispatch",
                UnresolvedReason::IndirectCall,
            ),
            ("reflection", UnresolvedReason::IndirectCall),
            ("new_provider_reason", UnresolvedReason::UnresolvedBinding),
            ("provider_error", UnresolvedReason::ProviderError),
            (
                "unsupported_declaration_or_stub",
                UnresolvedReason::UnsupportedConstruct,
            ),
        ] {
            artifact.observations[0].dispatch = entmoot_contracts::CallDispatch::Open {
                reason: entmoot_contracts::DispatchGap::from_provider("python", gap),
            };
            let open = normalize_artifact(&artifact);
            assert_eq!(open.relation_edges.len(), 1);
            assert_eq!(open.unresolved_relations[0].reason, expected);
            assert_eq!(open.resolver_observations[0], artifact.observations[0]);
            assert!(!open.unresolved_relations[0].evidence.is_empty());
        }

        // Ambiguous binding must not be re-labeled as runtime indirection when a
        // producer attaches an indirect_call mechanism by mistake.
        let restored_targets = artifact.observations[0].targets.clone();
        artifact.observations[0].outcome = ResolverObservationOutcome::Ambiguous;
        for target in &mut artifact.observations[0].targets {
            target.relation_target = None;
        }
        artifact.observations[0].dispatch = entmoot_contracts::CallDispatch::Open {
            reason: entmoot_contracts::DispatchGap::from_provider("csharp", "dynamic_dispatch"),
        };
        let ambiguous = normalize_artifact(&artifact);
        assert_eq!(
            ambiguous.unresolved_relations[0].reason,
            UnresolvedReason::AmbiguousTarget
        );
        artifact.observations[0].outcome = ResolverObservationOutcome::RepositoryCallable;
        artifact.observations[0].targets = restored_targets;

        for language in [
            "python",
            "javascript",
            "typescript",
            "tsx",
            "java",
            "csharp",
            "go",
            "rust",
            "c",
            "cpp",
        ] {
            artifact.language = language.into();
            artifact.observations[0].outcome = ResolverObservationOutcome::Unsupported;
            artifact.observations[0].targets.clear();
            artifact.observations[0].dispatch = entmoot_contracts::CallDispatch::Open {
                reason: entmoot_contracts::DispatchGap::BudgetExceeded,
            };
            artifact.observations[0].limitation =
                Some("dynamic target expansion exceeded producer budget".into());
            let skipped = normalize_artifact(&artifact);
            assert!(skipped.relation_edges.is_empty(), "{language}");
            assert_eq!(skipped.unresolved_relations.len(), 1, "{language}");
            assert_eq!(
                skipped.unresolved_relations[0].reason,
                UnresolvedReason::BudgetExceeded
            );
            assert_eq!(skipped.resolver_coverages[0].unsupported, 1, "{language}");
        }
    }

    #[test]
    fn external_targets_are_candidates_and_all_unassessed_sites_remain_open() {
        let artifact = ResolverArtifact {
            assessments: vec![],
            package_api: None,
            schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
            implementation: crate::implementation_fingerprint(),
            producer: ResolverArtifactProducer {
                id: "multi".into(),
                name: "multi".into(),
                version: "1".into(),
                method_id: "definition.v1".into(),
            },
            snapshot: snapshot(),
            language: "mixed".into(),
            language_version: Some("1".into()),
            build_context_digest: ContentDigest::from_trusted(format!("blake3:{}", "c".repeat(64))),
            observations: vec![
                ResolverObservation {
                    dispatch: Default::default(),
                    occurrence_id: "a-callable".into(),
                    source: ResolverCallSource {
                        call_site: topology_node_address("a-callable"),
                        relation_source: topology_node_address("caller"),
                        syntactic_callee: "helper".into(),
                    },
                    outcome: ResolverObservationOutcome::RepositoryCallable,
                    targets: vec![ResolverTarget {
                        address: topology_node_address("helper"),
                        relation_target: Some(topology_node_address("helper")),
                        provider_pointer: "targets/0".into(),
                        provider_symbol: Some(ProviderSymbolSpelling {
                            scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                            spelling: "pkg.helper".into(),
                        }),
                        provider_callable: None,
                        declaration: None,
                    }],
                    limitation: None,
                },
                ResolverObservation {
                    dispatch: Default::default(),
                    occurrence_id: "b-decl".into(),
                    source: ResolverCallSource {
                        call_site: topology_node_address("b-decl"),
                        relation_source: topology_node_address("caller"),
                        syntactic_callee: "Widget".into(),
                    },
                    outcome: ResolverObservationOutcome::RepositorySymbol,
                    targets: vec![ResolverTarget {
                        address: Address::RepoPath {
                            repository_id: Some("example/repo".into()),
                            path: "pkg/widget.py".into(),
                            start_byte: Some(1),
                            end_byte: Some(8),
                        },
                        relation_target: None,
                        provider_pointer: "targets/0".into(),
                        provider_symbol: Some(ProviderSymbolSpelling {
                            scheme: provider_symbol_schemes::PYTHON_QUALIFIED_NAME.into(),
                            spelling: "pkg.widget.Widget".into(),
                        }),
                        provider_callable: None,
                        declaration: None,
                    }],
                    limitation: Some("declaration without callable".into()),
                },
                ResolverObservation {
                    dispatch: Default::default(),
                    occurrence_id: "c-intrinsic".into(),
                    source: ResolverCallSource {
                        call_site: topology_node_address("c-intrinsic"),
                        relation_source: topology_node_address("caller"),
                        syntactic_callee: "len".into(),
                    },
                    outcome: ResolverObservationOutcome::Intrinsic,
                    targets: vec![ResolverTarget {
                        address: Address::Opaque {
                            system: "pyright.typeshed_uri".into(),
                            value: "file:///typeshed/stdlib/builtins.pyi".into(),
                        },
                        relation_target: None,
                        provider_pointer: "targets/0".into(),
                        provider_symbol: None,
                        provider_callable: None,
                        declaration: None,
                    }],
                    limitation: None,
                },
                ResolverObservation {
                    dispatch: Default::default(),
                    occurrence_id: "d-rust".into(),
                    source: ResolverCallSource {
                        call_site: topology_node_address("d-rust"),
                        relation_source: topology_node_address("caller"),
                        syntactic_callee: "Symbol".into(),
                    },
                    outcome: ResolverObservationOutcome::ExternalSymbol,
                    targets: vec![ResolverTarget {
                        address: Address::Opaque {
                            system: provider_symbol_schemes::RUST_DEF_PATH.into(),
                            value: "example::Symbol".into(),
                        },
                        relation_target: None,
                        provider_pointer: "targets/0".into(),
                        provider_symbol: Some(ProviderSymbolSpelling {
                            scheme: provider_symbol_schemes::RUST_DEF_PATH.into(),
                            spelling: "example::Symbol".into(),
                        }),
                        provider_callable: None,
                        declaration: None,
                    }],
                    limitation: None,
                },
                ResolverObservation {
                    dispatch: Default::default(),
                    occurrence_id: "e-jvm".into(),
                    source: ResolverCallSource {
                        call_site: topology_node_address("e-jvm"),
                        relation_source: topology_node_address("caller"),
                        syntactic_callee: "baz".into(),
                    },
                    outcome: ResolverObservationOutcome::ExternalSymbol,
                    targets: vec![ResolverTarget {
                        address: Address::Opaque {
                            system: provider_symbol_schemes::JVM_METHOD_DESCRIPTOR.into(),
                            value: "Lcom/foo/Bar;.baz:(I)V".into(),
                        },
                        relation_target: None,
                        provider_pointer: "targets/0".into(),
                        provider_symbol: Some(ProviderSymbolSpelling {
                            scheme: provider_symbol_schemes::JVM_METHOD_DESCRIPTOR.into(),
                            spelling: "Lcom/foo/Bar;.baz:(I)V".into(),
                        }),
                        provider_callable: None,
                        declaration: None,
                    }],
                    limitation: None,
                },
                ResolverObservation {
                    dispatch: Default::default(),
                    occurrence_id: "f-unresolved".into(),
                    source: ResolverCallSource {
                        call_site: topology_node_address("f-unresolved"),
                        relation_source: topology_node_address("caller"),
                        syntactic_callee: "missing".into(),
                    },
                    outcome: ResolverObservationOutcome::Unresolved,
                    targets: vec![],
                    limitation: Some("no definition".into()),
                },
            ],
        };
        let normalized = normalize_artifact(&artifact);
        assert_eq!(normalized.resolutions.len(), 5);
        assert_eq!(normalized.relation_edges.len(), 4);
        assert_eq!(normalized.relation_claims.len(), 4);
        assert_eq!(normalized.unresolved_relations.len(), 6);
        assert!(normalized.relation_claims.iter().all(|claim| matches!(
            claim.value,
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Candidate
            }
        )));
        let coverage = &normalized.resolver_coverages[0];
        assert_eq!(coverage.call_sites_observed, 6);
        assert_eq!(coverage.repository_occurrences, 2);
        assert_eq!(coverage.intrinsics_without_callable_target, 1);
        assert_eq!(coverage.external_symbols_without_source, 2);
        assert_eq!(coverage.unresolved_or_ambiguous, 1);
        // Shared accounting must preserve the same boundary in every claimed
        // family; cpp here is only an experimental control.
        for language in [
            "rust",
            "python",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "java",
            "csharp",
            "c",
            "cpp",
        ] {
            let mut sample = artifact.clone();
            sample.language = language.into();
            let normalized = normalize_artifact(&sample);
            let intrinsic = normalized
                .unresolved_relations
                .iter()
                .find(|attempt| attempt.occurrence_id.as_deref() == Some("c-intrinsic"))
                .unwrap();
            assert_eq!(
                intrinsic.reason,
                UnresolvedReason::DispatchNotEvaluated,
                "{language}"
            );
            for occurrence in ["b-decl", "d-rust"] {
                let attempt = normalized
                    .unresolved_relations
                    .iter()
                    .find(|attempt| attempt.occurrence_id.as_deref() == Some(occurrence))
                    .unwrap();
                assert_eq!(
                    attempt.reason,
                    UnresolvedReason::TargetNotInInventory,
                    "{language}"
                );
            }
            assert_eq!(normalized.relation_edges.len(), 4);
            assert_eq!(normalized.unresolved_relations.len(), 6);
            sample.observations[2].dispatch = entmoot_contracts::CallDispatch::Closed {
                policy_id: "fixture.intrinsic.v1".into(),
                evidence_pointer: "/intrinsic".into(),
                assumptions: vec!["explicit fixed intrinsic implementation".into()],
            };
            let closed = normalize_artifact(&sample);
            assert_eq!(closed.relation_edges.len(), 4);
            assert_eq!(closed.unresolved_relations.len(), 5);
            assert!(closed
                .unresolved_relations
                .iter()
                .all(|attempt| attempt.occurrence_id.as_deref() != Some("c-intrinsic")));
            sample.observations[2].dispatch = entmoot_contracts::CallDispatch::Open {
                reason: entmoot_contracts::DispatchGap::InvalidEnvironment,
            };
            let normalized = normalize_artifact(&sample);
            assert_eq!(
                normalized
                    .unresolved_relations
                    .iter()
                    .find(|attempt| attempt.occurrence_id.as_deref() == Some("c-intrinsic"))
                    .unwrap()
                    .reason,
                UnresolvedReason::InvalidEnvironment
            );
        }
        assert_eq!(
            coverage.repository_occurrences
                + coverage.indexed_dependency_occurrences
                + coverage.external_symbols_without_source
                + coverage.intrinsics_without_callable_target
                + coverage.unresolved_or_ambiguous
                + coverage.unsupported
                + coverage.failed,
            coverage.call_sites_observed
        );
    }
}
