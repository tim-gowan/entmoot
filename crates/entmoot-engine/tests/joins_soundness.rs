//! Adversarial shared-join gates for every shipping native language family.
use entmoot_contracts::{CallDispatch, ResolverObservationOutcome as Outcome};
use entmoot_contracts::{NativeSpanRef, NativeTargetLocation};
use entmoot_engine::native_resolver::{
    normalize_observations, NativeCallFact, NativeCallTarget, NativeRawObservation,
    NativeReferenceFact,
};
use entmoot_engine::{NodeKind, TopologyGraph, TopologyNode};

// Qualification assertions themselves must reject near misses without live tools.
#[path = "support/callsites_expect.rs"]
mod callsites_expect;

// Raw provider facts below are authored test inputs, not compiler qualification.
// Structural occurrence identities always come from production visitors.
mod revision_fixture {
    include!("../../entmoot-core/tests/support/revision_pairs.rs");
}

const LANGUAGES: &[(&str, &str)] = &[
    ("rust", "rs"),
    ("python", "py"),
    ("javascript", "js"),
    ("typescript", "ts"),
    ("tsx", "tsx"),
    ("go", "go"),
    ("java", "java"),
    ("csharp", "cs"),
    ("c", "c"),
    ("cpp", "cpp"),
];

#[test]
fn reference_uses_attach_exactly_and_reject_conflicts_and_unavailable_evidence() {
    use entmoot_engine::native_resolver::join::normalize_reference_uses;
    for &(language, extension) in LANGUAGES {
        let (_root, graph, calls) = fixture_with_source(extension, "sibling()");
        let call = &calls.calls[0];
        let reference = NativeReferenceFact {
            site: call.site.clone(),
            kind: "reference".into(),
            spelling: "sibling".into(),
            targets: call.targets.clone(),
            owner: call.owner.clone(),
            gap: None,
            limitation: None,
        };
        let mut raw = NativeRawObservation {
            references: vec![reference.clone(), reference.clone()],
            ..Default::default()
        };
        let observations = normalize_reference_uses(&graph, &raw, language);
        assert_eq!(observations.len(), 1, "{language}");
        let mut artifact = authored_artifact(&graph, &calls, language);
        artifact.assessments = observations;
        let (_directory, path, _) = write_artifact(&graph, &artifact);
        let mut attached = graph.clone();
        entmoot_engine::ingest_artifact_manifest(&mut attached, &path, false).unwrap();
        let claim = attached
            .relation_claims
            .iter()
            .find(|c| {
                matches!(
                    c.value,
                    entmoot_contracts::RelationClaimValue::Assessment { .. }
                )
            })
            .unwrap();
        assert_eq!(claim.evidence[0].provider_pointer, "/assessments/0");
        raw.references[1].spelling = "conflicting-provider-record".into();
        assert!(normalize_reference_uses(&graph, &raw, language).is_empty());
        raw.references = vec![reference];
        raw.references[0].targets[0]
            .location
            .repository_mut()
            .unwrap()
            .span
            .start_byte += 1;
        assert!(normalize_reference_uses(&graph, &raw, language).is_empty());
        raw.references[0].targets[0]
            .location
            .repository_mut()
            .unwrap()
            .span
            .start_byte -= 1;
        raw.invalid_environment = true;
        assert!(normalize_reference_uses(&graph, &raw, language).is_empty());
        raw.invalid_environment = false;
        raw.budget_exceeded = true;
        assert!(normalize_reference_uses(&graph, &raw, language).is_empty());
    }
}

#[allow(clippy::unwrap_used)]
fn authored_artifact(
    graph: &TopologyGraph,
    raw: &NativeRawObservation,
    language: &str,
) -> entmoot_contracts::ResolverArtifact {
    use entmoot_contracts::{ContentDigest, ResolverArtifact, ResolverArtifactProducer};
    ResolverArtifact {
        assessments: vec![],
        schema_version: entmoot_contracts::RESOLVER_ARTIFACT_SCHEMA.into(),
        implementation: entmoot_engine::implementation_fingerprint(),
        producer: ResolverArtifactProducer {
            id: "fixture-native".into(),
            name: "fixture-native".into(),
            version: "1".into(),
            method_id: entmoot_engine::native_resolver::JOIN_METHOD_ID.into(),
        },
        snapshot: graph.snapshot.clone(),
        language: language.into(),
        language_version: None,
        build_context_digest: ContentDigest::parse(entmoot_contracts::blake3_hex(
            b"authored native facts",
        ))
        .unwrap(),
        observations: normalize_observations(graph, raw, language),
        package_api: None,
    }
}

#[allow(clippy::unwrap_used)]
fn write_artifact(
    graph: &TopologyGraph,
    artifact: &entmoot_contracts::ResolverArtifact,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    entmoot_contracts::ArtifactManifest,
) {
    let directory = tempfile::tempdir().unwrap();
    let artifact_path = directory.path().join("resolver.json");
    let manifest_path = directory.path().join("resolver.json.manifest.json");
    std::fs::write(&artifact_path, serde_json::to_vec(artifact).unwrap()).unwrap();
    let manifest = entmoot_engine::native_resolver::prepare_resolver_manifest(
        graph,
        &artifact_path,
        "fixture-native",
        "1",
        &artifact.language,
        &[],
    )
    .unwrap();
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    (directory, manifest_path, manifest)
}

#[allow(clippy::unwrap_used)]
fn inventory_alias(node: &TopologyNode, kind: &str) -> entmoot_contracts::Address {
    use entmoot_contracts::{topology_node_address, Address};
    match kind {
        "opaque" => topology_node_address(&node.id),
        "logic" => Address::LogicAddress {
            logic_address: node.logic_address.clone().unwrap(),
        },
        "address" => Address::AddressId {
            address_id: node.address_id.clone().unwrap(),
        },
        "path" => Address::RepoPath {
            repository_id: None,
            path: node.path.clone().unwrap(),
            start_byte: Some(node.start_byte.unwrap().try_into().unwrap()),
            end_byte: Some(node.end_byte.unwrap().try_into().unwrap()),
        },
        _ => panic!("unauthored alias"),
    }
}

#[test]
fn provider_labels_and_address_aliases_share_actual_occurrence_derivations() {
    use entmoot_contracts::{topology_node_address, RelationKind, ResolvedBindingEvidence};
    for &(language, extension) in LANGUAGES {
        // Production Oxc and Clang profiles span several structural dialects.
        let producer_language = match language {
            "typescript" | "tsx" => "javascript",
            "c" | "cpp" => "cxx",
            other => other,
        };
        let (_root, graph, raw) = fixture_with_source(extension, "sibling()");
        let call = &raw.calls[0];
        let owner_span = call.owner.as_ref().unwrap();
        let target_span = call.targets[0].location.repository().unwrap();
        let site = select_node(
            &graph,
            NodeKind::CallStub,
            &call.site.path,
            call.site.span.start_byte,
            call.site.span.end_byte,
        );
        let owner = select_node(
            &graph,
            NodeKind::Callable,
            &owner_span.path,
            owner_span.span.start_byte,
            owner_span.span.end_byte,
        );
        let target = select_node(
            &graph,
            NodeKind::Callable,
            &target_span.path,
            target_span.span.start_byte,
            target_span.span.end_byte,
        );
        let mut expected_hash = None;
        for alias in ["opaque", "logic", "address", "path"] {
            let mut artifact = authored_artifact(&graph, &raw, producer_language);
            let observation = &mut artifact.observations[0];
            observation.occurrence_id = "provider-record-17".into();
            observation.source.call_site = inventory_alias(site, "path");
            observation.source.relation_source = inventory_alias(owner, alias);
            observation.targets[0].address = inventory_alias(target, alias);
            observation.targets[0].relation_target = Some(inventory_alias(target, alias));
            let original = serde_json::to_vec(&artifact).unwrap();
            let (directory, manifest_path, manifest) = write_artifact(&graph, &artifact);
            let mut candidate = graph.clone();
            entmoot_engine::ingest_artifact_manifest(&mut candidate, &manifest_path, false)
                .unwrap();
            candidate.validate().unwrap();
            assert_eq!(
                serde_json::to_value(&candidate.nodes).unwrap(),
                serde_json::to_value(&graph.nodes).unwrap()
            );
            let binding = candidate
                .resolved_identities
                .iter()
                .find(|record| Some(&record.address_id) == owner.address_id.as_ref())
                .unwrap();
            assert!(
                matches!(
                    binding.binding,
                    ResolvedBindingEvidence::Complete {
                        reference_count: 1,
                        ..
                    }
                ),
                "{language}/{alias}: {:?}",
                binding.binding
            );
            if let Some(expected) = &expected_hash {
                assert_eq!(&binding.resolved_hash, expected, "{language}/{alias}");
            } else {
                expected_hash = Some(binding.resolved_hash.clone());
            }
            let edges: Vec<_> = candidate
                .relation_edges
                .iter()
                .filter(|edge| {
                    edge.predicate == RelationKind::Calls
                        && edge.occurrence_id.as_deref() == Some(site.id.as_str())
                        && edge.object == topology_node_address(&target.id)
                })
                .collect();
            assert_eq!(edges.len(), 1, "{language}/{alias}");
            assert_eq!(edges[0].occurrence_id.as_deref(), Some(site.id.as_str()));
            assert_eq!(edges[0].subject, topology_node_address(&owner.id));
            assert_eq!(edges[0].object, topology_node_address(&target.id));
            // C/C++ intentionally have no L2 same-file bind; the other authored
            // controls have one same-scope declaration, with no shadow/overload.
            let shares_structural = !matches!(language, "c" | "cpp");
            if shares_structural {
                assert!(graph
                    .relation_edges
                    .iter()
                    .any(|edge| edge.id == edges[0].id));
                assert!(edges[0].scope.context_id.is_none());
                assert_eq!(candidate.relation_edges.len(), graph.relation_edges.len());
            } else {
                assert_eq!(
                    edges[0].scope.context_id.as_deref(),
                    Some(artifact.build_context_digest.as_str())
                );
            }
            let native_claim = candidate
                .relation_claims
                .iter()
                .find(|claim| claim.producer.id == "fixture-native")
                .unwrap();
            assert_eq!(
                native_claim.scope.context_id.as_deref(),
                Some(artifact.build_context_digest.as_str())
            );
            assert_eq!(
                native_claim.producer.environment_fingerprint.as_deref(),
                Some(artifact.build_context_digest.as_str())
            );
            assert_eq!(
                candidate.diagnostics, graph.diagnostics,
                "agreement is quiet"
            );
            assert_eq!(
                candidate.resolutions[0].from,
                topology_node_address(&site.id)
            );
            assert_eq!(
                candidate.resolutions[0].to,
                artifact.observations[0].targets[0].address
            );
            assert_eq!(
                candidate.resolutions[0].evidence_refs[0]
                    .record_id
                    .as_deref(),
                Some("provider-record-17")
            );
            let frontier = candidate
                .unresolved_relations
                .iter()
                .find(|row| {
                    row.evidence
                        .iter()
                        .any(|evidence| evidence.artifact_id == manifest.artifact_id)
                })
                .unwrap();
            assert_eq!(frontier.occurrence_id.as_deref(), Some(site.id.as_str()));
            assert_eq!(frontier.source, topology_node_address(&owner.id));
            let accounting = entmoot_engine::resolver_accounting::audit_resolvers(
                &candidate,
                &[(&manifest, &original)],
            )
            .unwrap();
            let accounted = accounting
                .call_sites
                .iter()
                .find(|row| row.node_id == site.id)
                .unwrap();
            assert_eq!(accounted.evaluations.len(), 1);
            assert_eq!(
                accounted.evaluations[0].observation,
                artifact.observations[0]
            );
            assert_eq!(accounted.evaluations[0].provider_pointer, "/observations/0");
            assert_eq!(accounting.producer_omissions, 0);
            assert_eq!(
                std::fs::read(directory.path().join("resolver.json")).unwrap(),
                original
            );
        }
    }
}

#[test]
fn duplicated_occurrence_aliases_and_contradictory_target_identities_fail_atomically() {
    for &(language, extension) in LANGUAGES {
        let (_root, graph, raw) = fixture_with_source(extension, "sibling()");
        let owner_span = raw.calls[0].owner.as_ref().unwrap();
        let owner = select_node(
            &graph,
            NodeKind::Callable,
            &owner_span.path,
            owner_span.span.start_byte,
            owner_span.span.end_byte,
        );
        for mutation in [
            "duplicate",
            "opaque",
            "logic",
            "address",
            "missing",
            "language",
        ] {
            let mut artifact = authored_artifact(&graph, &raw, language);
            if mutation == "duplicate" {
                let mut duplicate = artifact.observations[0].clone();
                duplicate.occurrence_id = "provider-record-duplicate".into();
                let site = select_node(
                    &graph,
                    NodeKind::CallStub,
                    &raw.calls[0].site.path,
                    raw.calls[0].site.span.start_byte,
                    raw.calls[0].site.span.end_byte,
                );
                duplicate.source.call_site = inventory_alias(site, "path");
                artifact.observations.push(duplicate);
                artifact
                    .observations
                    .sort_by(|a, b| a.occurrence_id.cmp(&b.occurrence_id));
            } else if mutation == "language" {
                artifact.language = "unrelated-language".into();
            } else if mutation == "missing" {
                artifact.observations[0].targets[0].address =
                    entmoot_contracts::topology_node_address("not-in-inventory");
            } else {
                artifact.observations[0].targets[0].address = inventory_alias(owner, mutation);
            }
            let (_directory, manifest_path, _) = write_artifact(&graph, &artifact);
            let mut candidate = graph.clone();
            let before = serde_json::to_value(&candidate).unwrap();
            let error =
                entmoot_engine::ingest_artifact_manifest(&mut candidate, &manifest_path, false)
                    .expect_err(&format!("{language}/{mutation}"));
            let expected_reason = match mutation {
                "duplicate" => "alias the same inventory call site",
                "language" => "language disagrees with inventory call site",
                "missing" => "does not uniquely match inventory",
                _ => "definition identity disagrees with claimed callable",
            };
            assert!(
                error.to_string().contains(expected_reason),
                "{language}/{mutation}: {error}"
            );
            assert_eq!(
                serde_json::to_value(&candidate).unwrap(),
                before,
                "{language}/{mutation}"
            );
        }
    }
}

#[test]
fn agreeing_native_artifacts_retain_independent_support_and_invalidate_separately() {
    use entmoot_contracts::{RelationClaimValue, ResolutionState};
    use entmoot_engine::{
        KnowledgeRead, KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository,
    };
    for &(language, extension) in LANGUAGES {
        let (_root, graph, raw) = fixture_with_source(extension, "sibling()");
        let mut artifacts = [
            authored_artifact(&graph, &raw, language),
            authored_artifact(&graph, &raw, language),
        ];
        // Different provider record identities for exactly the same inventory site.
        artifacts[0].observations[0].occurrence_id = "provider-a".into();
        artifacts[1].observations[0].occurrence_id = "provider-b".into();
        let (_left, left_path, left_manifest) = write_artifact(&graph, &artifacts[0]);
        let (_right, right_path, right_manifest) = write_artifact(&graph, &artifacts[1]);
        let mut forward = graph.clone();
        for path in [&left_path, &right_path] {
            entmoot_engine::ingest_artifact_manifest(&mut forward, path, false).unwrap();
        }
        let mut reverse = graph.clone();
        for path in [&right_path, &left_path] {
            entmoot_engine::ingest_artifact_manifest(&mut reverse, path, false).unwrap();
        }
        let native_claims = |graph: &TopologyGraph| {
            graph
                .relation_claims
                .iter()
                .filter(|claim| claim.producer.id == "fixture-native")
                .cloned()
                .collect::<Vec<_>>()
        };
        let native_unknowns = |graph: &TopologyGraph| {
            graph
                .unresolved_relations
                .iter()
                .filter(|row| row.producer.id == "fixture-native")
                .cloned()
                .collect::<Vec<_>>()
        };
        let claims = native_claims(&forward);
        assert_eq!(claims.len(), 1, "{language}");
        assert_eq!(claims, native_claims(&reverse), "{language}: input order");
        assert_eq!(
            claims[0].value,
            RelationClaimValue::ResolutionState {
                state: ResolutionState::Candidate
            }
        );
        assert_eq!(
            claims[0].evidence.len(),
            4,
            "both target and observation pointers per artifact"
        );
        assert_eq!(claims[0].provenance.as_ref().unwrap().artifacts.len(), 2);
        let unknowns = native_unknowns(&forward);
        assert_eq!(unknowns.len(), 1);
        assert_eq!(unknowns, native_unknowns(&reverse));
        assert_eq!(unknowns[0].evidence.len(), 2);
        assert_eq!(forward.nodes.len(), graph.nodes.len());
        forward.validate().unwrap();
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&forward))
            .unwrap();
        let loaded = repository
            .load()
            .unwrap()
            .apply_to_inventory(entmoot_engine::inventory_skeleton(&forward));
        loaded.validate().unwrap();
        assert_eq!(native_claims(&loaded), claims);
        assert_eq!(native_unknowns(&loaded), unknowns);

        // Removing either input leaves the other's support, raw mapping and
        // resolved identities. Removing both restores the structural projection.
        for (removed, remaining) in [
            (&left_manifest, &right_manifest),
            (&right_manifest, &left_manifest),
        ] {
            let mut candidate = forward.clone();
            candidate.remove_artifacts(std::slice::from_ref(&removed.artifact_id));
            candidate.validate().unwrap();
            let surviving = native_claims(&candidate);
            assert_eq!(surviving.len(), 1);
            assert!(surviving[0]
                .evidence
                .iter()
                .all(|e| e.artifact_id == remaining.artifact_id));
            assert_eq!(
                surviving[0].provenance.as_ref().unwrap().artifacts,
                vec![remaining.artifact_id.clone()]
            );
            assert_eq!(native_unknowns(&candidate)[0].evidence.len(), 1);
            assert_eq!(candidate.resolutions.len(), 1);
            assert!(candidate.resolved_identities.iter().all(|row| row
                .binding
                .source()
                .unwrap()
                .artifact
                .artifact_id
                .as_str()
                == remaining.artifact_id));
            // Conditional publication keeps the exact selected provider support,
            // without copying structural inventory into that artifact's evidence.
            let paths = forward
                .snapshot_files
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>();
            let declaration = entmoot_contracts::RevalidationDeclaration {
                policy_producer: serde_json::from_value(
                    serde_json::json!({"id":"fixture.inputs","name":"Fixture input policy"}),
                )
                .unwrap(),
                method_id: "fixture.complete-source.v1".into(),
                input_scope: entmoot_contracts::ScopeUniverse::from_selectors(
                    "native-join-policy",
                    vec![],
                    paths.clone(),
                    vec![],
                    entmoot_contracts::ScopeCompleteness::CompleteRelativeToUniverse,
                    Some(&paths),
                ),
                assumptions: vec!["fixed fixture producer, configuration and environment".into()],
                evidence_refs: vec![entmoot_contracts::EvidenceRef {
                    artifact_id: remaining.artifact_id.clone(),
                    provider_pointer: "/".into(),
                    record_id: None,
                }],
            };
            let mut ledger = SqliteKnowledgeRepository::open_in_memory().unwrap();
            let receipt = entmoot_engine::publish_artifact_evidence(
                &mut ledger,
                &forward,
                remaining,
                &declaration,
            )
            .unwrap();
            let retained = ledger.load().unwrap();
            assert!(
                retained.coverages.is_empty()
                    && retained.call_sites.is_empty()
                    && retained.declaration_sites.is_empty()
            );
            assert_eq!(retained.relation_claims, surviving);
            assert_eq!(retained.unresolved_relations, native_unknowns(&candidate));
            assert_eq!(retained.resolutions, candidate.resolutions);
            let identities = |records: &[entmoot_contracts::ResolvedIdentityRecord]| {
                records
                    .iter()
                    .map(|r| (r.record_id(), r.clone()))
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            assert_eq!(
                identities(&retained.resolved_identities),
                identities(&candidate.resolved_identities)
            );
            assert_eq!(retained.resolver_coverages, candidate.resolver_coverages);
            assert_eq!(retained.artifacts, vec![remaining.clone()]);
            assert!(receipt
                .dependent_applicability
                .iter()
                .any(|r| r.collection == "relation_claims"));
            candidate.remove_artifacts(std::slice::from_ref(&remaining.artifact_id));
            candidate.validate().unwrap();
            assert!(candidate.resolved_identities.is_empty());
            assert!(candidate.resolutions.is_empty());
            assert_eq!(candidate.relation_edges, graph.relation_edges);
            assert_eq!(candidate.relation_claims, graph.relation_claims);
            assert_eq!(candidate.unresolved_relations, graph.unresolved_relations);
        }

        // Build and policy identity keep otherwise matching evidence separate.
        for different in ["context", "policy"] {
            let mut other = artifacts[1].clone();
            if different == "context" {
                other.build_context_digest = entmoot_contracts::ContentDigest::parse(
                    entmoot_contracts::blake3_hex(b"different build environment"),
                )
                .unwrap();
            } else {
                other.producer.method_id = "independent.fixture.policy.v1".into();
            }
            let (_directory, path, _) = write_artifact(&graph, &other);
            let mut candidate = graph.clone();
            entmoot_engine::ingest_artifact_manifest(&mut candidate, &left_path, false).unwrap();
            entmoot_engine::ingest_artifact_manifest(&mut candidate, &path, false).unwrap();
            assert_eq!(native_claims(&candidate).len(), 2, "{language}/{different}");
            candidate.validate().unwrap();
        }
    }
}

#[test]
fn shared_projection_requires_active_site_support_and_matching_capability() {
    use entmoot_contracts::{topology_node_address, RelationClaimState, SubjectId};
    use entmoot_engine::{
        KnowledgeRead, KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository,
    };
    for &(language, extension) in LANGUAGES {
        let (_root, original, raw) = fixture_with_source(extension, "sibling()");
        for case in [
            "inactive",
            "unclaimed",
            "aggregate_only",
            "closed_dispatch",
            "different_target",
        ] {
            if matches!(language, "c" | "cpp")
                && matches!(case, "inactive" | "unclaimed" | "aggregate_only")
            {
                continue; // No structural bind exists in these syntactic-only controls.
            }
            let mut graph = original.clone();
            let mut artifact = authored_artifact(&graph, &raw, language);
            let site = artifact.observations[0].occurrence_id.clone();
            if matches!(case, "inactive" | "unclaimed" | "aggregate_only") {
                let site_edges: Vec<_> = graph
                    .relation_edges
                    .iter()
                    .filter(|edge| edge.occurrence_id.as_deref() == Some(&site))
                    .map(|edge| edge.id.clone())
                    .collect();
                assert_eq!(site_edges.len(), 1, "{language}/{case}");
                let relevant = |claim: &entmoot_contracts::RelationClaim| matches!(&claim.subject, SubjectId::RelationEdge { edge_id } if site_edges.contains(edge_id));
                if case == "inactive" {
                    for claim in graph
                        .relation_claims
                        .iter_mut()
                        .filter(|claim| relevant(claim))
                    {
                        claim.state = RelationClaimState::Superseded;
                    }
                } else {
                    graph.relation_claims.retain(|claim| !relevant(claim));
                    if case == "aggregate_only" {
                        graph
                            .relation_edges
                            .retain(|edge| !site_edges.contains(&edge.id));
                    }
                }
            } else if case == "closed_dispatch" {
                // Closed is a target-universe completeness claim: nonempty
                // policy_id, evidence pointer, and assumptions are required
                // (same product fences as dispatch_expectations).
                artifact.observations[0].dispatch = CallDispatch::Closed {
                    policy_id: "fixture.closed-world.v1".into(),
                    evidence_pointer: "/fixture/dispatch".into(),
                    assumptions: vec![
                        "authored single-world dispatch control".into(),
                        "selected fixture build context".into(),
                    ],
                };
                assert!(
                    artifact.validate().is_ok(),
                    "{language}/closed_dispatch must validate with fences"
                );
            } else {
                let owner = &raw.calls[0].owner.as_ref().unwrap();
                let target = select_node(
                    &graph,
                    NodeKind::Callable,
                    &owner.path,
                    owner.span.start_byte,
                    owner.span.end_byte,
                );
                artifact.observations[0].targets[0].address = topology_node_address(&target.id);
                artifact.observations[0].targets[0].relation_target =
                    Some(topology_node_address(&target.id));
            }
            let before_edges = graph.relation_edges.clone();
            let (directory, path, manifest) = write_artifact(&graph, &artifact);
            entmoot_engine::ingest_artifact_manifest(&mut graph, &path, false).unwrap();
            let native = graph
                .relation_claims
                .iter()
                .find(|claim| {
                    claim
                        .evidence
                        .iter()
                        .any(|e| e.artifact_id == manifest.artifact_id)
                })
                .unwrap();
            let SubjectId::RelationEdge { edge_id } = &native.subject else {
                panic!("call projection")
            };
            let edge = graph
                .relation_edges
                .iter()
                .find(|edge| &edge.id == edge_id)
                .unwrap();
            assert!(
                !before_edges.iter().any(|before| before.id == edge.id),
                "{language}/{case}"
            );
            assert_eq!(
                edge.scope.context_id.as_deref(),
                Some(artifact.build_context_digest.as_str())
            );
            assert_eq!(graph.relation_edges.len(), before_edges.len() + 1);
            assert!(before_edges
                .iter()
                .all(|before| graph.relation_edges.contains(before)));
            graph.validate().unwrap();
            let database_path = directory.path().join("knowledge.sqlite3");
            let mut repository = SqliteKnowledgeRepository::open(&database_path).unwrap();
            repository
                .replace_all(&KnowledgeStore::extract_from_graph(&graph))
                .unwrap();
            // Reject foreign scope at both graph and durable publication boundaries.
            let mut foreign = graph.clone();
            let claim = foreign
                .relation_claims
                .iter_mut()
                .find(|claim| claim.id == native.id)
                .unwrap();
            claim.scope.domain = "foreign/repository".into();
            claim.id = entmoot_contracts::compute_relation_claim_id(
                &claim.subject,
                &claim.predicate,
                &claim.value,
                &claim.producer,
                &claim.scope,
            );
            assert!(foreign.validate().is_err());
            assert!(repository
                .replace_all(&KnowledgeStore::extract_from_graph(&foreign))
                .is_err());
            let loaded = repository
                .load()
                .unwrap()
                .apply_to_inventory(entmoot_engine::inventory_skeleton(&graph));
            loaded.validate().unwrap();
            assert_eq!(
                loaded.artifacts.len(),
                graph.artifacts.len(),
                "failed replacement is atomic"
            );
        }
    }
}

#[test]
fn hollow_closed_dispatch_is_rejected_before_join() {
    use entmoot_contracts::ResolverArtifactValidationError;
    let (_root, graph, raw) = fixture_with_source("rs", "sibling()");
    let mut artifact = authored_artifact(&graph, &raw, "rust");
    artifact.observations[0].dispatch = CallDispatch::Closed {
        policy_id: String::new(),
        evidence_pointer: String::new(),
        assumptions: vec![],
    };
    assert!(
        matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::InvalidDispatchClosure)
        ),
        "empty policy/evidence/assumptions must not validate as Closed"
    );
    artifact.observations[0].dispatch = CallDispatch::Closed {
        policy_id: "fixture.closed-world.v1".into(),
        evidence_pointer: "/fixture/dispatch".into(),
        assumptions: vec!["   ".into()],
    };
    assert!(
        matches!(
            artifact.validate(),
            Err(ResolverArtifactValidationError::InvalidDispatchClosure)
        ),
        "whitespace-only assumptions must not validate as Closed"
    );
}

#[test]
fn normalized_native_artifacts_require_matching_fingerprints_before_mutation() {
    use entmoot_engine::artifact::{ArtifactAdapter, ResolverArtifactAdapter};
    for &(language, extension) in LANGUAGES {
        let (_source_root, graph, raw) = fixture_with_source(extension, "sibling()");
        let artifact = authored_artifact(&graph, &raw, language);
        let original = serde_json::to_value(&artifact).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let artifact_path = directory.path().join("resolver.json");
        let manifest_path = directory.path().join("resolver.json.manifest.json");
        let before = serde_json::to_value(&graph).unwrap();
        for mutation in ["current", "missing", "contracts", "derivation"] {
            let mut value = original.clone();
            if mutation == "missing" {
                value.as_object_mut().unwrap().remove("implementation");
            } else if mutation != "current" {
                value["implementation"][mutation] =
                    entmoot_contracts::blake3_hex(b"incompatible v1").into();
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            std::fs::write(&artifact_path, &bytes).unwrap();
            let manifest = entmoot_engine::native_resolver::prepare_resolver_manifest(
                &graph,
                &artifact_path,
                "fixture-native",
                "1",
                language,
                &[],
            )
            .unwrap();
            std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let mut candidate = graph.clone();
            let inspected = ResolverArtifactAdapter.inspect(&bytes);
            let ingested =
                entmoot_engine::ingest_artifact_manifest(&mut candidate, &manifest_path, false);
            if mutation == "current" {
                assert!(inspected.is_ok(), "{language}: {inspected:?}");
                assert!(ingested.is_ok(), "{language}: {ingested:?}");
                assert_eq!(
                    ingested.unwrap().resolver_observations,
                    artifact.observations.len()
                );
            } else {
                assert!(inspected.is_err(), "{language}/{mutation}");
                assert!(ingested.is_err(), "{language}/{mutation}");
                assert_eq!(
                    serde_json::to_value(&candidate).unwrap(),
                    before,
                    "{language}/{mutation}"
                );
            }
        }
    }
}

#[test]
fn malformed_sidecar_alternatives_and_owners_cannot_become_unique_bindings() {
    use entmoot_engine::native_resolver::producers::sidecar::observation_from_sidecar;
    for &(language, extension) in LANGUAGES {
        let (graph, raw) = fixture(extension);
        for mutation in [
            "negative_target",
            "reversed_target",
            "overflow_target",
            "invalid_owner",
        ] {
            let fact = &raw.calls[0];
            let target = fact.targets[0].location.repository().unwrap();
            let owner = fact.owner.as_ref().unwrap();
            // Author the sidecar protocol independently of normalized fact serde.
            let mut call = serde_json::json!({
                "path": fact.site.path,
                "start_byte": fact.site.span.start_byte,
                "end_byte": fact.site.span.end_byte,
                "targets": [{"path":target.path,"start_byte":target.span.start_byte,
                    "end_byte":target.span.end_byte,"symbol":"sibling"}],
                "owner": {"path":owner.path,"start_byte":owner.span.start_byte,
                    "end_byte":owner.span.end_byte}
            });
            if mutation == "invalid_owner" {
                call["owner"]["start_byte"] = (-1).into();
            } else {
                let mut invalid = call["targets"][0].clone();
                invalid["start_byte"] = match mutation {
                    "negative_target" => (-1i64).into(),
                    "reversed_target" => 200i64.into(),
                    _ => (u32::MAX as i64 + 111).into(),
                };
                call["targets"].as_array_mut().unwrap().push(invalid);
            }
            // Sidecar owner envelopes have target metadata; span fields suffice.
            let bytes = serde_json::to_vec(&serde_json::json!({"calls": [call]})).unwrap();
            let captured = entmoot_contracts::NativeSidecarOutput {
                stdout: bytes.clone().into(),
                stderr: vec![0xff, 0x00].into(),
            };
            let mut response: entmoot_engine::native_resolver::producers::sidecar::SidecarResponse =
                serde_json::from_slice(&bytes).unwrap();
            response.native_output = Some(captured.clone());
            let normalized = observation_from_sidecar(response);
            assert_eq!(normalized.sidecar_output.as_ref(), Some(&captured));
            let rows = normalize_observations(&graph, &normalized, language);
            assert!(rows[0].targets.is_empty(), "{language}/{mutation}");
            assert_ne!(rows[0].outcome, Outcome::RepositoryCallable);
        }
    }
}

#[test]
fn malformed_normalized_locations_never_narrow_a_target_set_or_mutate_evidence() {
    for &(language, extension) in LANGUAGES {
        for mutation in [
            "reversed",
            "overflow",
            "empty_path",
            "traversal",
            "absolute",
        ] {
            for alternative in [false, true] {
                let (graph, mut raw) = fixture(extension);
                let mut target = raw.calls[0].targets[0].clone();
                let site = target.location.repository_mut().unwrap();
                match mutation {
                    "reversed" => site.span.start_byte = site.span.end_byte + 1,
                    "overflow" => {
                        let Some(end) = (u32::MAX as usize).checked_add(1) else {
                            continue;
                        };
                        site.span.end_byte = end;
                    }
                    "empty_path" => site.path.clear(),
                    "traversal" => site.path = "../outside".into(),
                    "absolute" => site.path = "/outside".into(),
                    _ => unreachable!(),
                }
                if alternative {
                    raw.calls[0].targets.push(target);
                } else {
                    raw.calls[0].targets[0] = target;
                }
                let original = raw.clone();
                let rows = normalize_observations(&graph, &raw, language);
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    rows[0].outcome,
                    Outcome::Unsupported,
                    "{language}/{mutation}"
                );
                assert!(rows[0].targets.is_empty());
                assert_eq!(raw, original);
            }
        }
    }
}

fn select_node<'a>(
    graph: &'a TopologyGraph,
    kind: NodeKind,
    path: &str,
    start: usize,
    end: usize,
) -> &'a TopologyNode {
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == kind
                && node.path.as_deref() == Some(path)
                && node.start_byte == Some(start)
                && node.end_byte == Some(end)
        })
        .collect();
    assert_eq!(
        nodes.len(),
        1,
        "exact occurrence selector {path}:{start}..{end}"
    );
    nodes[0]
}

fn fixture(extension: &str) -> (TopologyGraph, NativeRawObservation) {
    fixture_with_call(extension, "sibling()")
}

#[allow(clippy::unwrap_used)]
fn fixture_with_call(extension: &str, expression: &str) -> (TopologyGraph, NativeRawObservation) {
    let (_source_root, graph, raw) = fixture_with_source(extension, expression);
    (graph, raw)
}

#[allow(clippy::unwrap_used)]
fn fixture_with_source(
    extension: &str,
    expression: &str,
) -> (tempfile::TempDir, TopologyGraph, NativeRawObservation) {
    let corpus = revision_fixture::load();
    let language = LANGUAGES
        .iter()
        .find(|(_, ext)| *ext == extension)
        .unwrap()
        .0;
    // C++ remains an experimental reuse of the C syntax control, with its own
    // production parser selected by extension. It adds no release-family claim.
    let pair_id = format!(
        "{}-call-owner",
        if language == "cpp" { "c" } else { language }
    );
    let pair = corpus.pairs.iter().find(|pair| pair.id == pair_id).unwrap();
    revision_fixture::verify(pair, &corpus.repository);
    let revision = &pair.base;
    assert_eq!(
        revision.calls.len(),
        1,
        "independently authored single call"
    );
    let call = &revision.calls[0];
    assert_eq!(call.callee, "sibling");
    let caller = revision
        .occurrences
        .iter()
        .find(|value| value.key == call.owner)
        .unwrap();
    let target = revision
        .occurrences
        .iter()
        .find(|value| value.key == "sibling")
        .unwrap();
    let mut source = revision.source.clone();
    assert_eq!(&source[call.span.start..call.span.end], "sibling()");
    source.replace_range(call.span.start..call.span.end, expression);
    let delta = expression.len() - "sibling()".len();
    let path = std::path::Path::new(&revision.path)
        .with_extension(extension)
        .to_string_lossy()
        .replace('\\', "/");
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(&path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, source).unwrap();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        root.path(),
        entmoot_engine::TopologyBuildOptions {
            repository: Some(&corpus.repository),
            policy: None,
        },
    )
    .unwrap();
    graph.validate().unwrap();
    let owner = select_node(
        &graph,
        NodeKind::Callable,
        &path,
        caller.span.start,
        caller.span.end + delta,
    );
    let callee = select_node(
        &graph,
        NodeKind::Callable,
        &path,
        target.span.start + delta,
        target.span.end + delta,
    );
    assert_eq!(owner.label, caller.name);
    assert_eq!(callee.label, target.name);
    let site = select_node(
        &graph,
        NodeKind::CallStub,
        &path,
        call.span.start,
        call.span.start + expression.len(),
    );
    assert_eq!(site.parent.as_deref(), Some(owner.id.as_str()));
    assert!(owner.address_id.is_some() && callee.address_id.is_some());
    let raw = NativeRawObservation {
        calls: vec![NativeCallFact {
            dispatch: Default::default(),
            site: entmoot_contracts::NativeSpanRef {
                path: path.clone(),
                span: entmoot_contracts::SiteSpan {
                    start_byte: call.span.start,
                    end_byte: call.span.start + expression.len(),
                },
            },

            targets: vec![NativeCallTarget {
                location: entmoot_contracts::NativeTargetLocation::from_provider(
                    None,
                    Some(path.clone()),
                    entmoot_contracts::SiteSpan {
                        start_byte: target.span.start + delta,
                        end_byte: target.span.end + delta,
                    },
                ),

                symbol: "sibling".into(),
                kind: "callable".into(),
                package_role: None,
                package_purl: None,
                ecosystem: None,
                module_path: None,
                package_version: None,
                path_segments: vec![],
            }],
            owner: Some(NativeSpanRef {
                path,
                span: entmoot_contracts::SiteSpan {
                    start_byte: caller.span.start,
                    end_byte: caller.span.end + delta,
                },
            }),
            gap: None,
            limitation: None,
            intrinsic: false,
        }],
        ..Default::default()
    };
    (root, graph, raw)
}

#[test]
fn callable_signatures_never_join_an_implementation_even_at_the_same_span() {
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        raw.calls[0].targets[0].kind = "callable_signature".into();
        raw.calls[0].gap = Some("dynamic_dispatch".into());
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows[0].outcome, Outcome::RepositorySymbol, "{language}");
        assert!(rows[0].targets[0].relation_target.is_none(), "{language}");
        let declaration = rows[0].targets[0].declaration.as_ref().unwrap();
        assert_eq!(
            declaration.kind,
            entmoot_contracts::ResolverDeclarationKind::Callable
        );
        assert_eq!(
            declaration.callable_association,
            entmoot_contracts::ResolverCallableAssociation::Unknown
        );
        assert_eq!(
            rows[0].dispatch,
            CallDispatch::Open {
                reason: entmoot_contracts::DispatchGap::IndirectCall(format!(
                    "{language}.dynamic_dispatch"
                ))
            }
        );
    }
}

#[test]
fn python_declarations_require_exact_ranges_across_decorators_and_relocation() {
    let corpus = revision_fixture::load();
    let pair = corpus
        .pairs
        .iter()
        .find(|p| p.id == "python-decorated-endpoint-relocation")
        .unwrap();
    revision_fixture::verify(pair, &corpus.repository);
    for (revision, shift) in [(&pair.base, 0), (&pair.head, 2)] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(&revision.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &revision.source).unwrap();
        let graph = entmoot_engine::build_topology_from_repo_with_options(
            root.path(),
            entmoot_engine::TopologyBuildOptions {
                repository: Some(&corpus.repository),
                policy: None,
            },
        )
        .unwrap();
        let before = serde_json::to_value(&graph).unwrap();
        let leaf = select_node(
            &graph,
            NodeKind::Callable,
            &revision.path,
            45 + shift,
            74 + shift,
        );
        let owner = select_node(
            &graph,
            NodeKind::Callable,
            &revision.path,
            78 + shift,
            114 + shift,
        );
        let class = select_node(
            &graph,
            NodeKind::Declaration,
            &revision.path,
            10 + shift,
            74 + shift,
        );
        assert_eq!(
            class.end_byte, leaf.end_byte,
            "nested declarations share an endpoint"
        );
        let span = |start, end| NativeSpanRef {
            path: revision.path.clone(),
            span: entmoot_contracts::SiteSpan {
                start_byte: start + shift,
                end_byte: end + shift,
            },
        };
        let (_, mut raw) = fixture("py");
        raw.calls[0].site = span(104, 114);
        raw.calls[0].owner = Some(span(78, 114));
        raw.calls[0].targets[0].location = NativeTargetLocation::from_provider(
            None,
            Some(revision.path.clone()),
            span(45, 74).span,
        );
        let rows = normalize_observations(&graph, &raw, "python");
        assert_eq!(rows[0].outcome, Outcome::RepositoryCallable);
        assert_eq!(
            rows[0].source.relation_source,
            entmoot_contracts::topology_node_address(&owner.id)
        );
        assert_eq!(
            rows[0].targets[0].relation_target,
            Some(entmoot_contracts::topology_node_address(&leaf.id))
        );

        // Widening to the class, decorator or indentation must not snap to its
        // final method. The Unicode prefix places `def` at character offset 41,
        // UTF-16 offset 42 and UTF-8 byte offset 45; only the last is admissible.
        for start in [10, 26, 43, 41, 42, 46] {
            let mut shifted = raw.clone();
            shifted.calls[0].targets[0]
                .location
                .repository_mut()
                .unwrap()
                .span
                .start_byte = start + shift;
            let rows = normalize_observations(&graph, &shifted, "python");
            assert_eq!(rows[0].outcome, Outcome::RepositorySymbol, "start {start}");
            assert!(rows[0].targets[0].relation_target.is_none());
            assert!(rows[0]
                .limitation
                .as_ref()
                .unwrap()
                .contains("unmatched or ambiguous"));
        }
        for field in ["owner", "path", "call_coordinates"] {
            let mut shifted = raw.clone();
            match field {
                "owner" => shifted.calls[0].owner.as_mut().unwrap().span.start_byte -= 2,
                "path" => shifted.calls[0].site.path = "other.py".into(),
                _ => shifted.calls[0].site.span.start_byte -= 4,
            }
            let rows = normalize_observations(&graph, &shifted, "python");
            assert_eq!(rows[0].outcome, Outcome::Unsupported, "{field}");
            assert!(rows[0].targets.is_empty());
        }
        let mut competing = raw.clone();
        let mut alternative = raw.calls[0].targets[0].clone();
        alternative
            .location
            .repository_mut()
            .unwrap()
            .span
            .start_byte = 10 + shift;
        competing.calls[0].targets.push(alternative);
        assert_eq!(
            normalize_observations(&graph, &competing, "python")[0].outcome,
            Outcome::Ambiguous
        );
        assert_eq!(
            serde_json::to_value(&graph).unwrap(),
            before,
            "joining cannot rewrite structural inventory"
        );
    }
}

#[test]
fn python_snapshot_drift_is_rejected_before_native_production() {
    use entmoot_engine::native_resolver::{
        produce_python_resolver, ExecutionPolicy, NativeProduceRequest, ProducerBudget,
    };
    let (root, graph, raw) = fixture_with_source("py", "sibling()");
    let output = tempfile::tempdir().unwrap();
    let graph_path = output.path().join("graph.json");
    std::fs::write(&graph_path, serde_json::to_vec(&graph).unwrap()).unwrap();
    let path = root.path().join(&raw.calls[0].site.path);
    let source = std::fs::read(&path).unwrap();
    std::fs::write(&path, [b"# source changed\n".as_slice(), &source].concat()).unwrap();
    let result = produce_python_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root.path().into(),
        output_dir: output.path().join("native"),
        budget: ProducerBudget::default(),
        execution_policy: ExecutionPolicy::default(),
        rust_profile: Default::default(),
    });
    assert!(matches!(
        result,
        Err(entmoot_engine::native_resolver::NativeProducerError::Drift(
            _
        ))
    ));
    assert!(!output.path().join("native").exists());
}

#[test]
fn java_leading_documentation_range_still_joins_uniquely() {
    let (graph, mut raw) = fixture("java");
    raw.calls[0].targets[0]
        .location
        .repository_mut()
        .unwrap()
        .span
        .start_byte -= 1;
    assert_eq!(
        normalize_observations(&graph, &raw, "java")[0].outcome,
        Outcome::RepositoryCallable
    );
}

#[test]
fn conflicting_native_calls_never_choose_the_first_target() {
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        let mut conflicting = raw.calls[0].clone();
        conflicting.targets[0].symbol = "other".into();
        conflicting.targets[0].location = NativeTargetLocation::Unmapped {
            span: *conflicting.targets[0].location.span(),
            origin_file: None,
        };
        raw.calls.push(conflicting);
        let forward = normalize_observations(&graph, &raw, language);
        raw.calls.reverse();
        assert_eq!(
            forward,
            normalize_observations(&graph, &raw, language),
            "{language}"
        );
        assert_eq!(forward[0].outcome, Outcome::Ambiguous, "{language}");
        assert!(forward[0].targets.is_empty());
    }
}

#[test]
fn repeated_identical_compiler_calls_remain_idempotent() {
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        let once = normalize_observations(&graph, &raw, language);
        assert_eq!(once.len(), 1);
        assert_eq!(once[0].outcome, Outcome::RepositoryCallable);
        let call = &raw.calls[0];
        let site = select_node(
            &graph,
            NodeKind::CallStub,
            &call.site.path,
            call.site.span.start_byte,
            call.site.span.end_byte,
        );
        let target = &call.targets[0];
        let callee = select_node(
            &graph,
            NodeKind::Callable,
            &target.location.repository().unwrap().path,
            target.location.span().start_byte,
            target.location.span().end_byte,
        );
        assert_eq!(once[0].occurrence_id, site.id);
        assert_eq!(
            once[0].targets[0].relation_target,
            Some(entmoot_contracts::topology_node_address(&callee.id))
        );
        raw.calls.push(raw.calls[0].clone());
        assert_eq!(
            once,
            normalize_observations(&graph, &raw, language),
            "{language}"
        );
    }
}

#[test]
fn provider_failures_and_omissions_preserve_actual_occurrence_denominators() {
    for &(language, extension) in LANGUAGES {
        for failure in ["budget", "environment", "omitted"] {
            let (graph, mut raw) = fixture(extension);
            let call = &raw.calls[0];
            let site = select_node(
                &graph,
                NodeKind::CallStub,
                &call.site.path,
                call.site.span.start_byte,
                call.site.span.end_byte,
            );
            match failure {
                "budget" => raw.budget_exceeded = true,
                "environment" => raw.invalid_environment = true,
                _ => raw.calls.clear(),
            }
            let rows = normalize_observations(&graph, &raw, language);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].occurrence_id, site.id);
            assert!(rows[0].targets.is_empty());
            assert_eq!(
                rows[0].outcome,
                if failure == "environment" {
                    Outcome::Failed
                } else {
                    Outcome::Unsupported
                }
            );
            match failure {
                "budget" => assert_eq!(
                    rows[0].dispatch,
                    CallDispatch::Open {
                        reason: entmoot_contracts::DispatchGap::BudgetExceeded
                    }
                ),
                "environment" => assert_eq!(
                    rows[0].dispatch,
                    CallDispatch::Open {
                        reason: entmoot_contracts::DispatchGap::InvalidEnvironment
                    }
                ),
                _ => assert_eq!(rows[0].dispatch, CallDispatch::Unassessed),
            }
        }
    }
}

#[test]
fn alternative_or_uninventoried_targets_never_select_a_local_implementation() {
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        let mut alternative = raw.calls[0].targets[0].clone();
        alternative.location = NativeTargetLocation::Unmapped {
            span: *alternative.location.span(),
            origin_file: None,
        };
        alternative.symbol = "dependency.sibling".into();
        raw.calls[0].targets.push(alternative);
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, Outcome::Ambiguous);
        assert_eq!(rows[0].targets.len(), 2);
        assert!(rows[0]
            .targets
            .iter()
            .all(|target| target.relation_target.is_none()));
        raw.calls[0].targets.pop();
        raw.calls[0].targets[0]
            .location
            .repository_mut()
            .unwrap()
            .path = format!("excluded.{extension}");
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows[0].outcome, Outcome::RepositorySymbol);
        assert!(rows[0].targets[0].relation_target.is_none());
        assert!(matches!(
            rows[0].targets[0].address,
            entmoot_contracts::Address::RepoPath { .. }
        ));
    }
}

#[test]
fn a_chained_call_cannot_supply_an_unobserved_inner_call_at_the_same_start() {
    for &(language, extension) in LANGUAGES {
        let expression = if language == "java" {
            "sibling().get()"
        } else {
            "sibling()()"
        };
        let (graph, raw) = fixture_with_call(extension, expression);
        let inner = select_node(
            &graph,
            NodeKind::CallStub,
            &raw.calls[0].site.path,
            raw.calls[0].site.span.start_byte,
            raw.calls[0].site.span.start_byte + "sibling()".len(),
        );
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows.len(), 2);
        let observation = rows
            .iter()
            .find(|row| row.occurrence_id == inner.id)
            .unwrap();
        assert!(observation.targets.is_empty(), "{language}");
        assert_eq!(observation.outcome, Outcome::Unsupported);
    }
}

#[test]
fn a_containing_call_cannot_supply_an_unobserved_nested_callee() {
    for &(language, extension) in LANGUAGES {
        let (graph, raw) = fixture_with_call(extension, "sibling(sibling())");
        // Only the provider's outer call was observed. Both syntax occurrences
        // and their identities come from the production visitor.
        let start = raw.calls[0].site.span.start_byte + "sibling(".len();
        let inner = select_node(
            &graph,
            NodeKind::CallStub,
            &raw.calls[0].site.path,
            start,
            start + "sibling()".len(),
        );
        let observations = normalize_observations(&graph, &raw, language);
        assert_eq!(observations.len(), 2);
        let inner = observations
            .iter()
            .find(|row| row.occurrence_id == inner.id)
            .unwrap();
        assert_eq!(inner.outcome, Outcome::Unsupported, "{language}");
        assert!(inner.targets.is_empty());
    }
}

#[test]
fn exact_owner_is_required_and_other_or_missing_owner_stays_open() {
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows[0].outcome, Outcome::RepositoryCallable, "{language}");
        let other = &raw.calls[0].targets[0];
        let other_span = (
            other.location.span().start_byte,
            other.location.span().end_byte,
        );
        raw.calls[0].owner.as_mut().unwrap().span.start_byte = other_span.0;
        raw.calls[0].owner.as_mut().unwrap().span.end_byte = other_span.1;
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows[0].outcome, Outcome::Unsupported, "{language}");
        assert!(rows[0].targets.is_empty());
        raw.calls[0].owner.as_mut().unwrap().span.start_byte = 200;
        raw.calls[0].owner.as_mut().unwrap().span.end_byte = 250;
        let rows = normalize_observations(&graph, &raw, language);
        assert_eq!(rows[0].outcome, Outcome::Unsupported, "{language}");
        assert!(matches!(rows[0].dispatch, CallDispatch::Open { .. }));
    }
}

#[test]
fn type_and_value_targets_are_never_promoted_by_callable_containment() {
    for &(language, extension) in LANGUAGES {
        for kind in ["type", "value", "module", "unknown"] {
            let (graph, mut raw) = fixture(extension);
            raw.calls[0].targets[0].kind = kind.into();
            let rows = normalize_observations(&graph, &raw, language);
            assert_eq!(
                rows[0].outcome,
                Outcome::RepositorySymbol,
                "{language}/{kind}"
            );
            assert!(rows[0].targets[0].relation_target.is_none());
        }
    }
}

#[test]
fn argument_import_and_nested_references_cannot_replace_a_missing_callee() {
    for &(language, extension) in LANGUAGES {
        for (kind, offset) in [("reference", 0), ("import", 0), ("call_callee", 5)] {
            let (graph, mut raw) = fixture(extension);
            let start = raw.calls[0].site.span.start_byte + offset;
            raw.references.push(NativeReferenceFact {
                site: entmoot_contracts::NativeSpanRef {
                    path: raw.calls[0].site.path.clone(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: start,
                        end_byte: raw.calls[0].site.span.end_byte,
                    },
                },
                kind: kind.into(),
                spelling: "argument".into(),
                targets: std::mem::take(&mut raw.calls[0].targets),
                owner: raw.calls[0].owner.clone(),
                gap: None,
                limitation: None,
            });
            let rows = normalize_observations(&graph, &raw, language);
            assert_eq!(
                rows[0].outcome,
                Outcome::Unresolved,
                "{language}/{kind}/{start}"
            );
            assert!(rows[0].targets.is_empty());
        }
    }
}

#[test]
fn dependency_coordinate_changes_invalidate_resolved_identity() {
    use entmoot_engine::resolved_identity::{derive_resolved_identity, InventoriedReference};
    use std::collections::BTreeMap;
    let source = entmoot_contracts::ResolverEvidenceRef {
        producer: entmoot_contracts::ProducerIdentity {
            producer_id: "fixture.native".into(),
            version: Some("pin".into()),
            tool_name: None,
        },
        artifact: entmoot_contracts::ArtifactRef {
            artifact_id: entmoot_contracts::ArtifactId::from_trusted("b".repeat(64)),
            content_digest: entmoot_contracts::ContentDigest::from_trusted(
                entmoot_contracts::blake3_hex(b"native join fixture"),
            ),
        },
    };
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        let target = &mut raw.calls[0].targets[0];
        target.location = NativeTargetLocation::Unmapped {
            span: *target.location.span(),
            origin_file: None,
        };
        target.package_role = Some("module".into());
        target.package_purl = Some("pkg:generic/dependency@1.0.0".into());
        let before = normalize_observations(&graph, &raw, language);
        raw.calls[0].targets[0].package_purl = Some("pkg:generic/dependency@2.0.0".into());
        let after = normalize_observations(&graph, &raw, language);
        let owner = raw.calls[0].owner.as_ref().unwrap();
        let caller = select_node(
            &graph,
            NodeKind::Callable,
            &owner.path,
            owner.span.start_byte,
            owner.span.end_byte,
        );
        let refs = [InventoriedReference {
            relative_offset: raw.calls[0].site.span.start_byte - owner.span.start_byte,
            spelling: "sibling".into(),
            kind: "call",
        }];
        let left = derive_resolved_identity(
            caller.address_id.as_deref().unwrap(),
            &refs,
            &BTreeMap::from([(refs[0].clone(), &before[0])]),
            &source,
        );
        let right = derive_resolved_identity(
            caller.address_id.as_deref().unwrap(),
            &refs,
            &BTreeMap::from([(refs[0].clone(), &after[0])]),
            &source,
        );
        assert_ne!(
            left.resolved_hash, right.resolved_hash,
            "{language}: dependency pin changed"
        );
    }
}

#[test]
fn indexed_callee_references_preserve_uniqueness_and_end_boundaries() {
    for &(language, extension) in LANGUAGES {
        let (graph, mut raw) = fixture(extension);
        let call = &mut raw.calls[0];
        let reference = NativeReferenceFact {
            site: entmoot_contracts::NativeSpanRef {
                path: call.site.path.clone(),
                span: entmoot_contracts::SiteSpan {
                    start_byte: call.site.span.start_byte,
                    end_byte: call.site.span.start_byte + 3,
                },
            },
            kind: "call_callee".into(),
            spelling: "sibling".into(),
            targets: std::mem::take(&mut call.targets),
            owner: call.owner.clone(),
            gap: None,
            limitation: None,
        };
        raw.references.push(reference.clone());
        assert_eq!(
            normalize_observations(&graph, &raw, language)[0].outcome,
            Outcome::RepositoryCallable
        );
        let mut beyond = reference.clone();
        beyond.site.span.end_byte = raw.calls[0].site.span.end_byte + 1;
        raw.references.push(beyond);
        assert_eq!(
            normalize_observations(&graph, &raw, language)[0].outcome,
            Outcome::RepositoryCallable
        );
        raw.references.push(reference.clone());
        let rows = normalize_observations(&graph, &raw, language);
        assert!(
            rows[0].targets.is_empty(),
            "{language}: multiple reference facts must remain ambiguous"
        );
        let mut reversed = reference.clone();
        reversed.site.span.end_byte = reversed.site.span.start_byte - 1;
        raw.references = vec![reversed];
        assert!(normalize_observations(&graph, &raw, language)[0]
            .targets
            .is_empty());
        raw.references.push(reference);
        assert!(normalize_observations(&graph, &raw, language)[0]
            .targets
            .is_empty());
    }
}

#[test]
fn go_callable_expression_pairs_preserve_sites_without_inventing_outer_targets() {
    let corpus = revision_fixture::load();
    let pairs: Vec<_> = corpus
        .pairs
        .iter()
        .filter(|pair| pair.id.starts_with("go114-callee-"))
        .collect();
    assert_eq!(pairs.len(), 6);
    for pair in pairs {
        revision_fixture::verify(pair, &corpus.repository);
        for revision in [&pair.base, &pair.head] {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join(&revision.path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, &revision.source).unwrap();
            let graph = entmoot_engine::build_topology_from_repo_with_options(
                root.path(),
                Default::default(),
            )
            .unwrap();
            graph.validate().unwrap();
            assert_eq!(
                graph
                    .nodes
                    .iter()
                    .filter(|node| node.kind == NodeKind::CallStub)
                    .count(),
                revision.calls.len()
            );
            let rows = normalize_observations(&graph, &NativeRawObservation::default(), "go");
            assert_eq!(rows.len(), revision.calls.len());
            for call in &revision.calls {
                let site = select_node(
                    &graph,
                    NodeKind::CallStub,
                    &revision.path,
                    call.span.start,
                    call.span.end,
                );
                assert_eq!(site.label, call.callee);
                let selector = revision
                    .occurrences
                    .iter()
                    .find(|value| value.key == call.owner)
                    .unwrap();
                let owner = select_node(
                    &graph,
                    NodeKind::Callable,
                    &revision.path,
                    selector.span.start,
                    selector.span.end,
                );
                assert_eq!(site.parent.as_deref(), Some(owner.id.as_str()));
                let observation = rows
                    .iter()
                    .find(|row| row.occurrence_id == site.id)
                    .unwrap();
                assert_eq!(observation.outcome, Outcome::Unsupported);
                assert!(observation.targets.is_empty());
                let bound = graph
                    .relation_edges
                    .iter()
                    .any(|edge| edge.occurrence_id.as_deref() == Some(site.id.as_str()));
                assert_eq!(
                    bound,
                    matches!(call.callee.as_str(), "factory" | "leaf")
                        && pair.id != "go114-callee-shadow",
                    "{}: only named inner calls have bounded local targets: {}",
                    pair.id,
                    call.callee
                );
            }
        }
    }
}
