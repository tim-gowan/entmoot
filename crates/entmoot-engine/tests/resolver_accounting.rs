#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_contracts::*;
use entmoot_engine::resolver_accounting::audit_resolvers;
use entmoot_engine::{build_topology_from_repo_with_options, TopologyBuildOptions};

fn fixture() -> entmoot_engine::TopologyGraph {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.py"),
        "def leaf():\n    pass\ndef caller():\n    leaf()\n    other()\n",
    )
    .unwrap();
    build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default()).unwrap()
}

#[test]
fn broken_source_does_not_disappear_behind_an_empty_callable_denominator() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.py"), "def broken(:\n").unwrap();
    let graph =
        build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default()).unwrap();
    let report = audit_resolvers(&graph, &[]).unwrap();
    assert!(report.callables.is_empty());
    assert!(!report.inventory_gaps.is_empty());
}

fn artifact(graph: &entmoot_engine::TopologyGraph, producer: &str) -> ResolverArtifact {
    let sites = graph
        .nodes
        .iter()
        .filter(|n| n.kind == entmoot_engine::NodeKind::CallStub);
    let mut observations = sites
        .map(|n| ResolverObservation {
            dispatch: CallDispatch::Unassessed,
            occurrence_id: n.id.clone(),
            source: ResolverCallSource {
                call_site: topology_node_address(&n.id),
                relation_source: topology_node_address(n.parent.as_ref().unwrap()),
                syntactic_callee: n.label.clone(),
            },
            outcome: ResolverObservationOutcome::Unresolved,
            targets: vec![],
            limitation: Some("fixture: no definition found".into()),
        })
        .collect::<Vec<_>>();
    observations.sort_by(|a, b| a.occurrence_id.cmp(&b.occurrence_id));
    ResolverArtifact {
        assessments: vec![],
        package_api: None,
        schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
        implementation: entmoot_engine::implementation_fingerprint(),
        producer: ResolverArtifactProducer {
            id: producer.into(),
            name: producer.into(),
            version: "1".into(),
            method_id: "fixture.binding.v1".into(),
        },
        snapshot: graph.snapshot.clone(),
        language: "python".into(),
        language_version: None,
        build_context_digest: ContentDigest::parse(blake3_hex(b"build")).unwrap(),
        observations,
    }
}

fn input(a: &ResolverArtifact) -> (ArtifactManifest, Vec<u8>) {
    let bytes = serde_json::to_vec(a).unwrap();
    let manifest = ArtifactManifest::build(
        ArtifactFormat::Resolver,
        ArtifactProducer {
            name: a.producer.name.clone(),
            version: Some(a.producer.version.clone()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        a.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "fixture",
            vec![a.language.clone()],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        ),
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    (manifest, bytes)
}

#[test]
fn omissions_staleness_and_duplicate_sites_cannot_pass_the_ratchet() {
    let graph = fixture();
    let mut a = artifact(&graph, "compiler");
    a.observations.pop();
    let (m, b) = input(&a);
    let report = audit_resolvers(&graph, &[(&m, &b)]).unwrap();
    assert_eq!(report.producer_omissions, 1);
    assert_eq!(report.static_work_remaining, 2);
    assert!(audit_resolvers(&graph, &[(&m, b"altered")]).is_err());
    a.snapshot.revision = "b".repeat(40);
    let (m, b) = input(&a);
    assert!(audit_resolvers(&graph, &[(&m, &b)]).is_err());
    a.snapshot = graph.snapshot.clone();
    let mut duplicate = a.observations[0].clone();
    duplicate.occurrence_id = "zzzz".into();
    a.observations.push(duplicate);
    let (m, b) = input(&a);
    assert!(audit_resolvers(&graph, &[(&m, &b)]).is_err());
}

#[test]
fn runtime_boundary_and_open_dispatch_are_distinct_and_producers_do_not_erase_gaps() {
    let graph = fixture();
    let mut a = artifact(&graph, "compiler");
    a.observations[0].outcome = ResolverObservationOutcome::Unresolved;
    a.observations[0].dispatch = CallDispatch::Open {
        reason: DispatchGap::IndirectCall("rust.runtime_mutation".into()),
    };
    a.observations[1].outcome = ResolverObservationOutcome::ExternalSymbol;
    a.observations[1].targets.push(ResolverTarget {
        address: Address::Opaque {
            system: "fixture.api".into(),
            value: "api".into(),
        },
        relation_target: None,
        provider_pointer: "/native/1".into(),
        provider_symbol: None,
        provider_callable: None,
        declaration: None,
    });
    let (m, b) = input(&a);
    let report = audit_resolvers(&graph, &[(&m, &b), (&m, &b)]).unwrap();
    assert_eq!(report.static_work_remaining, 0);
    assert_eq!(report.artifacts.len(), 1);
    assert_eq!(report.call_sites[0].evaluations.len(), 1);
    assert!(report
        .call_sites
        .iter()
        .any(|s| s.evaluations[0].next_step == ResolverNextStep::RuntimeEvidenceRequired));
    assert!(report
        .call_sites
        .iter()
        .any(|s| s.evaluations[0].next_step == ResolverNextStep::RepositoryBoundaryRecorded));
    let other = artifact(&graph, "independent");
    let (m2, b2) = input(&other);
    let combined = audit_resolvers(&graph, &[(&m, &b), (&m2, &b2)]).unwrap();
    assert_eq!(combined.static_work_remaining, 2);
    assert_eq!(
        serde_json::to_value(&combined).unwrap(),
        serde_json::to_value(audit_resolvers(&graph, &[(&m2, &b2), (&m, &b)]).unwrap()).unwrap()
    );
    a.observations[0].outcome = ResolverObservationOutcome::Unsupported;
    let (m, b) = input(&a);
    assert_eq!(
        audit_resolvers(&graph, &[(&m, &b)])
            .unwrap()
            .static_work_remaining,
        1
    );
}

#[test]
fn structural_edges_and_native_definition_candidates_do_not_close_dispatch() {
    let graph = fixture();
    let mut a = artifact(&graph, "compiler");
    let leaf = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == "leaf")
        .unwrap();
    for o in &mut a.observations {
        o.outcome = ResolverObservationOutcome::RepositoryCallable;
        o.targets.push(ResolverTarget {
            address: topology_node_address(&leaf.id),
            relation_target: Some(topology_node_address(&leaf.id)),
            provider_pointer: "/native/target".into(),
            provider_symbol: None,
            provider_callable: None,
            declaration: None,
        });
    }
    let (m, b) = input(&a);
    let coverage = audit_resolvers(&graph, &[(&m, &b)]).unwrap().coverage;
    assert_eq!(coverage[0].inventoried_sites, 2);
    assert_eq!(coverage[0].closed_sites, 0);
    assert_eq!(coverage[0].open_sites, 2);
    assert_eq!(
        audit_resolvers(&graph, &[(&m, &b)])
            .unwrap()
            .static_work_remaining,
        2
    );
    for o in &mut a.observations {
        o.dispatch = CallDispatch::Closed {
            policy_id: "fixture.closure".into(),
            evidence_pointer: "/native/closed".into(),
            assumptions: vec!["fixed build; no mutation".into()],
        };
    }
    let (m, b) = input(&a);
    let coverage = audit_resolvers(&graph, &[(&m, &b)]).unwrap().coverage;
    assert_eq!(coverage[0].closed_sites, 2);
    assert_eq!(coverage[0].missing_sites, 0);
    let report = audit_resolvers(&graph, &[(&m, &b)]).unwrap();
    let root = report
        .callables
        .iter()
        .find(|c| !c.call_sites.is_empty())
        .unwrap()
        .node_id
        .clone();
    let reach = entmoot_engine::resolver_accounting::audit_reachability(
        &report,
        &a.build_context_digest,
        std::slice::from_ref(&root),
    )
    .unwrap();
    assert!(reach.explicit_calls_closed);
    assert!(reach.potentially_reachable.contains(&leaf.id));
    assert!(entmoot_engine::resolver_accounting::audit_reachability(
        &report,
        &ContentDigest::parse(blake3_hex(b"other-build")).unwrap(),
        std::slice::from_ref(&root)
    )
    .is_err());
    let other = artifact(&graph, "open-producer");
    let (m2, b2) = input(&other);
    let report = audit_resolvers(&graph, &[(&m, &b), (&m2, &b2)]).unwrap();
    assert!(
        !entmoot_engine::resolver_accounting::audit_reachability(
            &report,
            &a.build_context_digest,
            &[root]
        )
        .unwrap()
        .explicit_calls_closed
    );
    assert_eq!(
        audit_resolvers(&graph, &[(&m, &b)])
            .unwrap()
            .static_work_remaining,
        0
    );
}

#[test]
fn every_ecosystem_callable_is_accounted_without_claiming_runtime_only() {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "a.py",
            "def leaf():\n    pass\ndef caller():\n    unknown()\n",
        ),
        (
            "a.ts",
            "function leaf() {} function caller() { unknown(); }",
        ),
        (
            "a.js",
            "function leaf() {} function caller() { unknown(); }",
        ),
        ("a.rs", "fn leaf() {} fn caller() { unknown(); }"),
        (
            "a.go",
            "package a\nfunc leaf() {}\nfunc caller() { unknown() }",
        ),
        (
            "A.java",
            "class A { void leaf() {} void caller() { unknown(); } }",
        ),
        (
            "A.cs",
            "class A { void leaf() {} void caller() { unknown(); } }",
        ),
    ] {
        std::fs::write(dir.path().join(path), source).unwrap();
    }
    let graph =
        build_topology_from_repo_with_options(dir.path(), TopologyBuildOptions::default()).unwrap();
    let audit = entmoot_engine::resolver_accounting::audit_resolvers(&graph, &[]).unwrap();
    assert_eq!(audit.callables.len(), 14);
    assert_eq!(audit.call_sites.len(), 7);
    assert_eq!(audit.static_work_remaining, 7);
    assert_eq!(
        audit
            .callables
            .iter()
            .filter(|c| c.call_sites.is_empty())
            .count(),
        7
    );
    assert!(audit.call_sites.iter().all(|s| s.evaluations.is_empty()));
    assert_eq!(audit.ecosystems.len(), 7);
    // The same native evidence contract is accepted for each registered ecosystem;
    // no language-specific compiler type enters the accounting implementation.
    let inputs = audit
        .ecosystems
        .iter()
        .map(|language| {
            let mut a = artifact(&graph, language);
            a.language = language.clone();
            a.observations.retain(|o| {
                audit
                    .call_sites
                    .iter()
                    .any(|s| s.node_id == o.occurrence_id && s.language == *language)
            });
            for o in &mut a.observations {
                o.outcome = ResolverObservationOutcome::ExternalSymbol;
                o.targets.push(ResolverTarget {
                    address: Address::Opaque {
                        system: "fixture.external".into(),
                        value: "function".into(),
                    },
                    relation_target: None,
                    provider_pointer: "/native/target".into(),
                    provider_symbol: None,
                    provider_callable: None,
                    declaration: None,
                });
            }
            input(&a)
        })
        .collect::<Vec<_>>();
    let report = audit_resolvers(
        &graph,
        &inputs
            .iter()
            .map(|(m, b)| (m, b.as_slice()))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    // The JavaScript producer profile also covers TypeScript. Its unrestricted
    // manifest cannot hide an omitted TS site behind another producer's result.
    let ts_sites: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == entmoot_engine::NodeKind::CallStub && node.path.as_deref() == Some("a.ts")
        })
        .collect();
    assert_eq!(ts_sites.len(), 1);
    let js_artifact = report
        .artifacts
        .iter()
        .find(|row| row.language == "javascript")
        .unwrap();
    assert_eq!(js_artifact.missing_call_sites, vec![ts_sites[0].id.clone()]);
    assert_eq!(report.static_work_remaining, 0);
    assert_eq!(report.producer_omissions, 1);
    assert!(report.inventory_gaps.is_empty());
}
