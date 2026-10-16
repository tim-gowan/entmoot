//! Read-only ratchet over immutable native artifacts and the full inventory.
use crate::{ManifestUnverified, NodeKind, TopologyGraph};
use entmoot_contracts::{
    ArtifactManifest, CallDispatch, CallEvaluation, CallSiteAccounting, CallableAccounting,
    ContentDigest, DispatchGap, ResolverAccounting, ResolverArtifactAccounting, ResolverNextStep,
    ResolverObservationOutcome,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, thiserror::Error)]
#[error("invalid resolver accounting input: {0}")]
pub struct AccountingError(String);

fn invalid(error: impl std::fmt::Display) -> AccountingError {
    AccountingError(error.to_string())
}

/// Traverse validated native targets in exactly one build context. The result
/// describes explicit calls only; caller-supplied roots do not close implicit execution.
pub fn audit_reachability(
    audit: &ResolverAccounting,
    context: &ContentDigest,
    entrypoints: &[String],
) -> Result<entmoot_contracts::ReachabilityAudit, AccountingError> {
    let artifacts: BTreeMap<_, _> = audit
        .artifacts
        .iter()
        .filter(|a| &a.build_context_digest == context)
        .map(|a| (a.manifest.artifact_id.as_str(), a))
        .collect();
    if artifacts.is_empty() {
        return Err(invalid("build context has no validated artifacts"));
    }
    let missing: BTreeSet<_> = artifacts
        .values()
        .flat_map(|a| a.missing_call_sites.iter().map(String::as_str))
        .collect();
    let callables: BTreeMap<_, _> = audit
        .callables
        .iter()
        .map(|c| (c.node_id.as_str(), c))
        .collect();
    if entrypoints.is_empty()
        || entrypoints
            .iter()
            .any(|id| !callables.contains_key(id.as_str()))
    {
        return Err(invalid(
            "entrypoints must identify inventoried callable occurrences",
        ));
    }
    let sites: BTreeMap<_, _> = audit
        .call_sites
        .iter()
        .map(|s| (s.node_id.as_str(), s))
        .collect();
    let mut pending: Vec<_> = entrypoints.to_vec();
    let mut reached = BTreeSet::new();
    let mut open = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !reached.insert(id.clone()) {
            continue;
        }
        for site_id in &callables[id.as_str()].call_sites {
            let Some(site) = sites.get(site_id.as_str()) else {
                open.insert(site_id.clone());
                continue;
            };
            let evaluations: Vec<_> = site
                .evaluations
                .iter()
                .filter(|e| artifacts.contains_key(e.artifact_id.as_str()))
                .collect();
            if evaluations.is_empty() || missing.contains(site_id.as_str()) {
                open.insert(site_id.clone());
            }
            let mut previous_targets = None;
            for evaluation in evaluations {
                if evaluation.next_step != ResolverNextStep::StaticTargetsClosed {
                    open.insert(site_id.clone());
                }
                let mut targets = BTreeSet::new();
                for target in &evaluation.observation.targets {
                    let Some(entmoot_contracts::Address::Opaque { system, value }) =
                        &target.relation_target
                    else {
                        if evaluation.observation.outcome != ResolverObservationOutcome::Intrinsic {
                            open.insert(site_id.clone());
                        }
                        continue;
                    };
                    if system != "entmoot.topology_node" || !callables.contains_key(value.as_str())
                    {
                        open.insert(site_id.clone());
                        continue;
                    }
                    targets.insert(value.clone());
                    pending.push(value.clone());
                }
                if previous_targets.as_ref().is_some_and(|old| old != &targets) {
                    open.insert(site_id.clone());
                }
                previous_targets = Some(targets);
            }
        }
    }
    let mut roots = entrypoints.to_vec();
    roots.sort();
    roots.dedup();
    Ok(entmoot_contracts::ReachabilityAudit {
        build_context_digest: context.clone(),
        entrypoints: roots,
        not_observed_reachable: callables
            .keys()
            .filter(|id| !reached.contains(**id))
            .map(|id| (*id).to_owned())
            .collect(),
        potentially_reachable: reached.into_iter().collect(),
        explicit_calls_closed: open.is_empty() && audit.inventory_gaps.is_empty(),
        open_call_sites: open.into_iter().collect(),
        limitations: vec![
            "caller_supplied_entrypoints_not_qualified_as_exhaustive".into(),
            "implicit_invocations_callbacks_and_external_reentry_unassessed".into(),
            "not_observed_reachable_is_not_dead_code".into(),
        ],
    })
}

/// Load native artifact manifests using the same repository-relative path policy
/// as ingestion. Does not run producers or mutate the knowledge repository.
pub fn audit_resolver_manifests(
    graph: &TopologyGraph,
    paths: &[std::path::PathBuf],
) -> Result<ResolverAccounting, AccountingError> {
    let mut inputs = Vec::new();
    for path in paths {
        let manifest: ArtifactManifest =
            serde_json::from_slice(&std::fs::read(path).map_err(invalid)?).map_err(invalid)?;
        let artifact_path =
            crate::ingest::resolve_artifact_path(path, &manifest).map_err(invalid)?;
        let bytes = std::fs::read(artifact_path).map_err(invalid)?;
        inputs.push((manifest, bytes));
    }
    audit_resolvers(
        graph,
        &inputs
            .iter()
            .map(|(m, b)| (m, b.as_slice()))
            .collect::<Vec<_>>(),
    )
}

/// Compiler or solver work is the default for unknown, unsupported, budget-limited
/// or failed attempts. Only explicit runtime-dependent producer claims request a
/// runtime probe. This is a next-evidence decision, not an impossibility theorem.
fn next_step(observation: &entmoot_contracts::ResolverObservation) -> ResolverNextStep {
    use ResolverNextStep::*;
    use ResolverObservationOutcome::*;
    if matches!(observation.outcome, Unsupported | Failed | RepositorySymbol) {
        return StaticAnalysisRequired;
    }
    if matches!(
        observation.dispatch,
        CallDispatch::Open {
            reason: DispatchGap::BudgetExceeded
                | DispatchGap::InvalidEnvironment
                | DispatchGap::UnsupportedConstruct
                | DispatchGap::ProviderError
                | DispatchGap::UnresolvedBinding(_)
        }
    ) {
        return StaticAnalysisRequired;
    }
    if matches!(
        &observation.dispatch,
        CallDispatch::Open { reason } if reason.requires_runtime_evidence()
    ) {
        return if observation
            .limitation
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            RuntimeEvidenceRequired
        } else {
            StaticAnalysisRequired
        };
    }
    if matches!(observation.outcome, ExternalSymbol | IndexedDependency) {
        return RepositoryBoundaryRecorded;
    }
    if observation.dispatch.is_closed()
        && matches!(
            observation.outcome,
            RepositoryCallable | Intrinsic | Ambiguous
        )
    {
        return StaticTargetsClosed;
    }
    StaticAnalysisRequired
}

/// Validate all inputs before returning accounting. Producer omissions are computed
/// from inventory, never from the producer's self-reported denominator. Each
/// artifact remains independent; a favorable producer cannot erase another's gaps.
pub fn audit_resolvers(
    graph: &TopologyGraph,
    inputs: &[(&ArtifactManifest, &[u8])],
) -> Result<ResolverAccounting, AccountingError> {
    graph.validate().map_err(invalid)?;
    let hasher = entmoot_core::StructuralHasher::new();
    let language = |node: &crate::TopologyNode| {
        node.path
            .as_deref()
            .and_then(|p| p.rsplit_once('.'))
            .and_then(|(_, ext)| hasher.language_for_extension(&format!(".{ext}")))
            .unwrap_or_else(|| "unknown".into())
    };
    let mut sites = BTreeMap::new();
    let mut callables = BTreeMap::new();
    let mut paths = HashMap::new();
    for node in &graph.nodes {
        if node.kind == NodeKind::Callable {
            callables.insert(
                node.id.clone(),
                CallableAccounting {
                    node_id: node.id.clone(),
                    language: language(node),
                    call_sites: vec![],
                },
            );
        }
        if node.kind != NodeKind::CallStub {
            continue;
        }
        let owner_id = node
            .parent
            .clone()
            .ok_or_else(|| invalid("call site has no owner"))?;
        sites.insert(
            node.id.clone(),
            CallSiteAccounting {
                node_id: node.id.clone(),
                owner_id,
                language: language(node),
                evaluations: vec![],
            },
        );
        paths.insert(
            node.id.clone(),
            node.path
                .as_deref()
                .ok_or_else(|| invalid("call site has no path"))?,
        );
    }
    let mut artifacts = BTreeMap::new();
    for (manifest, bytes) in inputs {
        let verified = ManifestUnverified::new((*manifest).clone())
            .verify(bytes, &graph.snapshot)
            .map_err(invalid)?;
        let manifest = verified.manifest();
        if artifacts.contains_key(&manifest.artifact_id) {
            continue;
        }
        let artifact =
            crate::artifact::resolver::parse_scoped_artifact(bytes, manifest).map_err(invalid)?;
        let joined = crate::artifact::resolver::join_inventory_bindings(
            graph,
            manifest,
            &artifact.language,
            &artifact.observations,
        )
        .map_err(invalid)?;
        let mut seen = BTreeSet::new();
        for (index, observation) in artifact.observations.into_iter().enumerate() {
            let id = &joined
                .site(&observation.occurrence_id)
                .ok_or_else(|| invalid("observation does not identify an inventoried call site"))?
                .id;
            if !seen.insert(id.clone()) {
                return Err(invalid("duplicate call site within one producer artifact"));
            }
            let site = sites
                .get_mut(id)
                .ok_or_else(|| invalid("call site index mismatch"))?;
            site.evaluations.push(CallEvaluation {
                artifact_id: manifest.artifact_id.clone(),
                provider_pointer: format!("/observations/{index}"),
                next_step: next_step(&observation),
                observation,
            });
        }
        let mut missing_call_sites = vec![];
        for id in sites.keys() {
            if crate::native_resolver::join::language_matches(&artifact.language, paths[id])
                && manifest.path_in_scope(paths[id]).map_err(invalid)?
                && !seen.contains(id)
            {
                missing_call_sites.push(id.clone());
            }
        }
        artifacts.insert(
            manifest.artifact_id.clone(),
            ResolverArtifactAccounting {
                manifest: manifest.clone(),
                producer: artifact.producer,
                language: artifact.language,
                build_context_digest: artifact.build_context_digest,
                missing_call_sites,
            },
        );
    }
    for site in sites.values_mut() {
        site.evaluations
            .sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));
        if let Some(owner) = callables.get_mut(&site.owner_id) {
            owner.call_sites.push(site.node_id.clone());
        }
    }
    let static_work_remaining = sites
        .values()
        .filter(|s| {
            s.evaluations.is_empty()
                || s.evaluations
                    .iter()
                    .any(|e| e.next_step == ResolverNextStep::StaticAnalysisRequired)
        })
        .count();
    let producer_omissions = artifacts.values().map(|a| a.missing_call_sites.len()).sum();
    let inventory_gaps = graph
        .diagnostics
        .iter()
        .filter(|d| d.code.starts_with("source."))
        .map(|d| {
            format!(
                "{}:{}",
                d.code,
                d.provider_pointer.as_deref().unwrap_or("unknown")
            )
        })
        .chain(
            graph
                .nodes
                .iter()
                .filter(|n| {
                    n.kind == NodeKind::File && language(n) != "unknown" && n.file_hash.is_none()
                })
                .map(|n| {
                    format!(
                        "source.not_indexed:{}",
                        n.path.as_deref().unwrap_or("unknown")
                    )
                }),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let coverage = artifacts
        .values()
        .map(|artifact| {
            let evaluations: Vec<_> = sites
                .values()
                .flat_map(|site| &site.evaluations)
                .filter(|e| e.artifact_id == artifact.manifest.artifact_id)
                .collect();
            let closed_sites = evaluations
                .iter()
                .filter(|e| e.next_step == ResolverNextStep::StaticTargetsClosed)
                .count();
            let mut outcomes = BTreeMap::new();
            for evaluation in &evaluations {
                let outcome =
                    serde_json::to_value(evaluation.observation.outcome).map_err(invalid)?;
                *outcomes
                    .entry(
                        outcome
                            .as_str()
                            .ok_or_else(|| invalid("outcome is not a string"))?
                            .to_owned(),
                    )
                    .or_default() += 1;
            }
            Ok(entmoot_contracts::ResolverScopeCoverage {
                artifact_id: artifact.manifest.artifact_id.clone(),
                language: artifact.language.clone(),
                build_context_digest: artifact.build_context_digest.clone(),
                inventoried_sites: evaluations.len() + artifact.missing_call_sites.len(),
                observed_sites: evaluations.len(),
                missing_sites: artifact.missing_call_sites.len(),
                closed_sites,
                open_sites: evaluations.len() - closed_sites,
                outcomes,
            })
        })
        .collect::<Result<Vec<_>, AccountingError>>()?;
    let ecosystems = callables
        .values()
        .map(|c| c.language.clone())
        .chain(sites.values().map(|s| s.language.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut graph_hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut graph_hasher, graph).map_err(invalid)?;
    Ok(ResolverAccounting {
        coverage,
        snapshot: graph.snapshot.clone(), graph_digest: ContentDigest::parse(format!("blake3:{}", graph_hasher.finalize().to_hex())).map_err(invalid)?,
        method_id: "entmoot.inventory-native-accounting.v1".into(), ecosystems,
        artifacts: artifacts.into_values().collect(), callables: callables.into_values().collect(),
        call_sites: sites.into_values().collect(), static_work_remaining, producer_omissions, inventory_gaps,
        limitations: vec![
            "Accounts for inventoried callables and explicit call sites only; parser exclusions and implicit calls remain outside this denominator.".into(),
            "No incoming edge is not evidence of dead code or an external entrypoint. A callable with no observed calls is only a syntactic leaf.".into(),
            "Definition lookup and candidate edges do not close dispatch. Runtime evidence requests do not prove static impossibility or require a full deployment.".into(),
            "Producer assertions are validated for provenance and inventory consistency, not independently proved. Build contexts remain separate.".into(),
        ],
    })
}
