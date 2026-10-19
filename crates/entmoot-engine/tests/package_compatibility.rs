//! Controlled producer-evidence fixtures, not native producer qualification.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_contracts::package_api::*;
use entmoot_contracts::query::{
    BoundaryKind, CallableImpactKind, CompatibilityReason, CompatibilityVerdict as Verdict,
    QueryService, PACKAGE_COMPATIBILITY_SCHEMA,
};
use entmoot_contracts::*;
use entmoot_engine::compatibility::{CompatibilityCatalog, PackagePublication};
use entmoot_engine::{NodeKind, TopologyBuildOptions, TopologyGraph};

struct Fixture {
    _root: tempfile::TempDir,
    graph: TopologyGraph,
    artifact: ResolverArtifact,
}

fn fixture(language: &str, file: &str, source: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(file), source).unwrap();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        root.path(),
        TopologyBuildOptions {
            repository: Some("fixtures/package-api"),
            ..Default::default()
        },
    )
    .unwrap();
    let artifact = ResolverArtifact {
        assessments: vec![],
        schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
        implementation: entmoot_engine::implementation_fingerprint(),
        producer: ResolverArtifactProducer {
            id: "fixture.package_api".into(),
            name: "fixture.package_api".into(),
            version: "1".into(),
            method_id: "fixture.observed_boundary.v1".into(),
        },
        snapshot: graph.snapshot.clone(),
        language: language.into(),
        language_version: None,
        build_context_digest: ContentDigest::parse(blake3_hex(b"fixture-build-context")).unwrap(),
        observations: vec![],
        package_api: Some(PackageApiEvidence {
            surfaces: vec![],
            references: vec![],
            references_complete: true,
            limitations: vec![CompatibilityLimitation::producer_stated(
                "controlled fixture producer; not native qualification",
            )],
        }),
    };
    Fixture {
        _root: root,
        graph,
        artifact,
    }
}

fn materialize(mut fixture: Fixture) -> (PackagePublication, TopologyGraph) {
    fixture.artifact.validate().unwrap();
    let bytes = serde_json::to_vec(&fixture.artifact).unwrap();
    let paths: Vec<_> = fixture
        .graph
        .snapshot_files
        .iter()
        .map(|row| row.path.clone())
        .collect();
    let manifest = ArtifactManifest::build(
        ArtifactFormat::Resolver,
        ArtifactProducer {
            name: fixture.artifact.producer.name.clone(),
            version: Some(fixture.artifact.producer.version.clone()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        fixture.graph.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "package-fixture",
            vec![fixture.artifact.language.clone()],
            vec![],
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&paths),
        ),
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    fixture.graph.artifacts.push(manifest);
    let digest = blake3_hex(&serde_json::to_vec(&fixture.graph).unwrap());
    let publication =
        PackagePublication::from_graph(&fixture.graph, digest, |_| Ok(Some(bytes.clone())))
            .unwrap();
    (publication, fixture.graph)
}

fn provider(
    language: &str,
    file: &str,
    source: &str,
    target: &PackageApiTarget,
    partial: bool,
) -> Fixture {
    let mut fixture = fixture(language, file, source);
    let exports = fixture
        .graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Callable)
        .map(|node| PackageApiExport {
            target: target.clone(),
            declaration: topology_node_address(&node.id),
            boundary_policy: "entmoot.syntactic_boundary.fixture.v1".into(),
            boundary_digest: node.signature_hash.clone().unwrap(),
        })
        .collect();
    fixture
        .artifact
        .package_api
        .as_mut()
        .unwrap()
        .surfaces
        .push(PackageApiSurface {
            package: target.package.clone(),
            export_universes: vec![PackageApiExportUniverse {
                symbol_scheme: target.symbol.scheme.clone(),
                boundary_policy: "entmoot.syntactic_boundary.fixture.v1".into(),
            }],
            completeness: if partial {
                ScopeCompleteness::Partial
            } else {
                ScopeCompleteness::CompleteRelativeToUniverse
            },
            exports,
            limitations: vec![],
        });
    fixture
}

fn consumer(
    language: &str,
    file: &str,
    source: &str,
    target: &PackageApiTarget,
    expected: &str,
) -> Fixture {
    let mut fixture = fixture(language, file, source);
    let sites: Vec<_> = fixture
        .graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::CallStub)
        .collect();
    let [node] = sites.as_slice() else {
        panic!("the authored consumer fixture requires exactly one call occurrence");
    };
    fixture.artifact.observations.push(ResolverObservation {
        dispatch: CallDispatch::Unassessed,
        occurrence_id: node.id.clone(),
        source: ResolverCallSource {
            call_site: topology_node_address(&node.id),
            relation_source: topology_node_address(node.parent.as_ref().unwrap()),
            syntactic_callee: node.label.clone(),
        },
        outcome: ResolverObservationOutcome::ExternalSymbol,
        targets: vec![ResolverTarget {
            address: Address::Opaque {
                system: target.symbol.scheme.clone(),
                value: target.symbol.spelling.clone(),
            },
            relation_target: None,
            provider_pointer: "/fixture/target".into(),
            provider_symbol: Some(target.symbol.clone()),
            provider_callable: None,
            declaration: None,
        }],
        limitation: None,
    });
    fixture
        .artifact
        .package_api
        .as_mut()
        .unwrap()
        .references
        .push(PackageApiReference {
            lock_instance: Some(fixture_lock(&target.package)),
            observation_id: node.id.clone(),
            target_index: 0,
            target: target.clone(),
            boundary_policy: "entmoot.syntactic_boundary.fixture.v1".into(),
            expected_boundary: Some(expected.into()),
        });
    fixture
}

// Contract harness, deliberately separate from real native lock qualification.
fn fixture_lock(package: &PackageApiPackage) -> LockedPackageInstance {
    let kind = match package.ecosystem {
        PackageApiEcosystem::Rust => "cargo",
        PackageApiEcosystem::Javascript => "npm",
        PackageApiEcosystem::Go => "golang",
        PackageApiEcosystem::Python => "pypi",
        PackageApiEcosystem::Java => "maven",
        PackageApiEcosystem::Csharp => "nuget",
        PackageApiEcosystem::Native => "generic",
    };
    let mut instance = LockedPackageInstance {
        instance_id: String::new(),
        purl: format!(
            "pkg:{kind}/{}@1.0.0",
            package
                .name
                .strip_prefix("generic:")
                .unwrap_or(&package.name)
        ),
        lock_path: "fixture.lock".into(),
        lock_digest: ContentDigest::parse(blake3_hex(b"fixture lock; not a native gate")).unwrap(),
        entry: "/fixture/selected".into(),
        placement: "fixture/dependency".into(),
    };
    instance.instance_id = instance.derive_id();
    instance
}

#[test]
fn five_controls_across_claimed_grammars_and_experimental_cpp() {
    for (language, file, ecosystem, consumer_source, original, changed, body, removed) in [
        (
            "rust",
            "lib.rs",
            PackageApiEcosystem::Rust,
            "fn caller() { api::sanitize(1); }",
            "pub fn sanitize(x: i32) -> i32 { x }",
            "pub fn sanitize(x: i64) -> i64 { x }",
            "pub fn sanitize(x: i32) -> i32 { x + 1 }",
            "// removed export\n",
        ),
        (
            "typescript",
            "api.ts",
            PackageApiEcosystem::Javascript,
            "import {sanitize} from 'api'; function caller() { return sanitize(1); }",
            "export function sanitize(x: number): number { return x; }",
            "export function sanitize(x: string): string { return x; }",
            "export function sanitize(x: number): number { return x + 1; }",
            "// removed export\n",
        ),
        (
            "javascript",
            "api.js",
            PackageApiEcosystem::Javascript,
            "import {sanitize} from 'api'; function caller() { return sanitize(1); }",
            "export function sanitize(x) { return x; }",
            "export function sanitize(x, extra) { return x; }",
            "export function sanitize(x) { return x + 1; }",
            "// removed export\n",
        ),
        (
            "tsx",
            "api.tsx",
            PackageApiEcosystem::Javascript,
            "import {sanitize} from 'api'; function caller() { return <div>{sanitize(1)}</div>; }",
            "export function sanitize(x: number) { return <div>{x}</div>; }",
            "export function sanitize(x: string) { return <div>{x}</div>; }",
            "export function sanitize(x: number) { return <span>{x}</span>; }",
            "// removed export\n",
        ),
        (
            "go",
            "api.go",
            PackageApiEcosystem::Go,
            "package consumer\nimport \"example.test/api\"\nfunc Caller() { api.Sanitize(1) }",
            "package api\nfunc Sanitize(x int) int { return x }",
            "package api\nfunc Sanitize(x string) string { return x }",
            "package api\nfunc Sanitize(x int) int { return x + 1 }",
            "package api\n",
        ),
        (
            "python",
            "api.py",
            PackageApiEcosystem::Python,
            "def caller():\n    sanitize(1)\n",
            "def sanitize(x: int) -> int:\n    return x\n",
            "def sanitize(x: str) -> str:\n    return x\n",
            "def sanitize(x: int) -> int:\n    return x + 1\n",
            "# removed\n",
        ),
        (
            "java",
            "Api.java",
            PackageApiEcosystem::Java,
            "class Caller { void caller() { Api.sanitize(1); } }",
            "public class Api { public static int sanitize(int x) { return x; } }",
            "public class Api { public static long sanitize(long x) { return x; } }",
            "public class Api { public static int sanitize(int x) { return x + 1; } }",
            "public class Api {}",
        ),
        (
            "csharp",
            "Api.cs",
            PackageApiEcosystem::Csharp,
            "class Caller { void caller() { Api.sanitize(1); } }",
            "public class Api { public static int sanitize(int x) { return x; } }",
            "public class Api { public static long sanitize(long x) { return x; } }",
            "public class Api { public static int sanitize(int x) { return x + 1; } }",
            "public class Api {}",
        ),
        (
            "c",
            "api.c",
            PackageApiEcosystem::Native,
            "int caller(void) { return sanitize(1); }",
            "int sanitize(int x) { return x; }",
            "long sanitize(long x) { return x; }",
            "int sanitize(int x) { return x + 1; }",
            "/* removed */\n",
        ),
        (
            "cpp",
            "api.cpp",
            PackageApiEcosystem::Native,
            "int caller() { return sanitize(1); }",
            "int sanitize(int x) { return x; }",
            "long sanitize(long x) { return x; }",
            "int sanitize(int x) { return x + 1; }",
            "// removed\n",
        ),
    ] {
        let target = PackageApiTarget {
            package: PackageApiPackage {
                ecosystem,
                name: match ecosystem {
                    PackageApiEcosystem::Native => "generic:example.test/api",
                    PackageApiEcosystem::Go | PackageApiEcosystem::Java => "example.test/api",
                    _ => "api",
                }
                .into(),
            },
            symbol: ProviderSymbolSpelling {
                scheme: format!("{language}.fixture.qualified_symbol"),
                spelling: "example.test/api::sanitize".into(),
            },
        };
        let baseline = provider(language, file, original, &target, false);
        let expected = baseline.artifact.package_api.as_ref().unwrap().surfaces[0].exports[0]
            .boundary_digest
            .clone();
        let original_graph = baseline.graph.clone();
        let mut unlocked = consumer(language, file, consumer_source, &target, &expected);
        let api = unlocked.artifact.package_api.as_mut().unwrap();
        api.references[0].lock_instance = None;
        api.references_complete = false;
        let catalog =
            CompatibilityCatalog::new(materialize(unlocked).0, vec![materialize(baseline).0])
                .unwrap();
        let unbound = catalog.assess(catalog.request()).unwrap();
        assert_eq!(
            unbound.verdict,
            Verdict::Fits,
            "{language}: assessed equality without lock is fits"
        );
        assert_eq!(
            unbound.sites[0].reason,
            CompatibilityReason::ObservedBoundaryEqualBodyNotAssessed
        );
        assert!(
            unbound
                .limitations
                .iter()
                .any(|gap| { gap.code == CompatibilityLimitationCode::LockInstanceUnasserted }),
            "{language}: lock remain strengthen-available"
        );
        for (name, source, verdict) in [
            ("unchanged", original, Verdict::Fits),
            ("removed", removed, Verdict::BrokenAt),
            ("boundary", changed, Verdict::BrokenAt),
            ("body", body, Verdict::Fits),
            ("structural_only", original, Verdict::IncompleteBind),
        ] {
            let (consumer, graph) = materialize(consumer(
                language,
                file,
                consumer_source,
                &target,
                &expected,
            ));
            let mut candidate = provider(language, file, source, &target, false);
            let observed_boundary = candidate.artifact.package_api.as_ref().unwrap().surfaces[0]
                .exports
                .first()
                .map(|export| export.boundary_digest.clone());
            let (provider, provider_graph) = if name == "structural_only" {
                candidate.graph.artifacts.clear();
                let publication = PackagePublication::from_graph(
                    &candidate.graph,
                    blake3_hex(b"structural-only"),
                    |_| panic!("structural read must not open artifacts"),
                )
                .unwrap();
                (publication, candidate.graph)
            } else {
                materialize(candidate)
            };
            // Expectations come from authored source and input evidence, not a
            // normalized output snapshot. This keeps provenance checks exact
            // when implementation fingerprints change.
            assert_eq!(graph.artifacts.len(), 1);
            assert_eq!(
                provider_graph.artifacts.len(),
                usize::from(name != "structural_only")
            );
            let consumer_id = graph.artifacts[0].artifact_id.to_string();
            let provider_id = provider_graph
                .artifacts
                .first()
                .map(|a| a.artifact_id.to_string());
            let consumer_publication = blake3_hex(&serde_json::to_vec(&graph).unwrap());
            let provider_publication = if name == "structural_only" {
                blake3_hex(b"structural-only")
            } else {
                blake3_hex(&serde_json::to_vec(&provider_graph).unwrap())
            };
            let catalog = CompatibilityCatalog::new(consumer, vec![provider]).unwrap();
            let result = catalog.assess(catalog.request()).unwrap();
            assert_eq!(result.schema_version, PACKAGE_COMPATIBILITY_SCHEMA);
            assert_eq!(result.boundary_kind, BoundaryKind::PackageApi);
            assert_eq!(result.verdict, verdict, "{language}/{name}: {result:?}");
            assert_eq!(result.consumer.publication, consumer_publication);
            assert_eq!(result.consumer.snapshot, graph.snapshot);
            assert_eq!(result.providers.len(), 1);
            assert_eq!(result.providers[0].publication, provider_publication);
            assert_eq!(result.providers[0].snapshot, provider_graph.snapshot);
            assert_eq!(result.boundaries.len(), 1);
            assert_eq!(result.boundaries[0].package, target.package);
            assert_eq!(result.boundaries[0].verdict, verdict);
            assert_eq!(result.boundaries[0].sites, 1);
            assert_eq!(result.sites.len(), 1);
            let site = &result.sites[0];
            let expression = match language {
                "rust" => "api::sanitize(1)",
                "go" => "api.Sanitize(1)",
                "java" | "csharp" => "Api.sanitize(1)",
                _ => "sanitize(1)",
            };
            assert_eq!(consumer_source.matches(expression).count(), 1);
            let start = consumer_source.find(expression).unwrap();
            let nodes: Vec<_> = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == NodeKind::CallStub
                        && node.start_byte == Some(start)
                        && node.end_byte == Some(start + expression.len())
                })
                .collect();
            let [node] = nodes.as_slice() else {
                panic!("{language}: exact authored call selector")
            };
            assert_eq!(
                site.location,
                Address::RepoPath {
                    repository_id: Some("fixtures/package-api".into()),
                    path: file.into(),
                    start_byte: Some(start.try_into().unwrap()),
                    end_byte: Some((start + expression.len()).try_into().unwrap()),
                }
            );
            assert_eq!(site.target, target);
            assert_eq!(site.lock_instance, Some(fixture_lock(&target.package)));
            assert_eq!(site.verdict, verdict);
            assert_eq!(
                site.reason,
                match name {
                    "boundary" => CompatibilityReason::ObservedBoundaryChanged,
                    "removed" => CompatibilityReason::ExportAbsentInCompleteSurface,
                    "structural_only" => CompatibilityReason::ProviderExportEvidenceMissing,
                    _ => CompatibilityReason::ObservedBoundaryEqualBodyNotAssessed,
                }
            );
            assert_eq!(
                site.boundary_policy,
                "entmoot.syntactic_boundary.fixture.v1"
            );
            assert_eq!(site.expected_boundary.as_ref(), Some(&expected));
            assert_eq!(
                site.observed_boundary,
                if name == "structural_only" {
                    None
                } else {
                    observed_boundary
                }
            );
            assert_eq!(
                site.provider_publication,
                (name != "structural_only").then(|| provider_publication.clone())
            );
            if matches!(name, "removed" | "structural_only") {
                assert!(site.observed_boundary.is_none());
            } else if name == "boundary" {
                assert!(site.observed_boundary.is_some());
                assert_ne!(site.observed_boundary.as_ref(), Some(&expected));
            } else {
                assert_eq!(site.observed_boundary.as_ref(), Some(&expected));
            }
            let mut expected_evidence = vec![
                (
                    consumer_id.as_str(),
                    "/observations/0/targets/0",
                    Some(node.id.as_str()),
                ),
                (
                    consumer_id.as_str(),
                    "/package_api/references/0",
                    Some(node.id.as_str()),
                ),
            ];
            if let Some(id) = &provider_id {
                expected_evidence.push((id, "/package_api/surfaces/0", None));
                if name != "removed" {
                    expected_evidence.push((id, "/package_api/surfaces/0/exports/0", None));
                }
            }
            let mut actual: Vec<_> = site
                .evidence
                .iter()
                .map(|e| {
                    (
                        e.artifact_id.as_str(),
                        e.provider_pointer.as_str(),
                        e.record_id.as_deref(),
                    )
                })
                .collect();
            expected_evidence.sort();
            actual.sort();
            assert_eq!(
                actual, expected_evidence,
                "{language}/{name}: exact evidence set"
            );
            let mut expected_limits = vec![
                CompatibilityLimitation {
                    artifact_id: Some(consumer_id),
                    ..CompatibilityLimitation::producer_stated(
                        "controlled fixture producer; not native qualification",
                    )
                },
                CompatibilityLimitation::coded(
                    CompatibilityLimitationCode::BoundaryEqualityPolicyOnly,
                ),
            ];
            expected_limits.push(if let Some(id) = provider_id {
                CompatibilityLimitation {
                    artifact_id: Some(id),
                    publication: Some(provider_publication),
                    ..CompatibilityLimitation::producer_stated(
                        "controlled fixture producer; not native qualification",
                    )
                }
            } else {
                CompatibilityLimitation {
                    publication: Some(provider_publication),
                    ..CompatibilityLimitation::coded(
                        CompatibilityLimitationCode::PackageApiEvidenceMissing,
                    )
                }
            });
            let mut actual_limits = result.limitations.clone();
            actual_limits.sort();
            expected_limits.sort();
            assert_eq!(actual_limits, expected_limits);

            // Same independently authored changes exercise current impact, rather
            // than replaying incompatible September graph exports.
            let impact =
                entmoot_engine::compare_revision_impact(&original_graph, &provider_graph).unwrap();
            let expected_kind = match name {
                "body" => Some(CallableImpactKind::BodyChanged),
                "boundary" => Some(CallableImpactKind::BodyAndBoundaryChanged),
                "removed" => Some(CallableImpactKind::Removed),
                _ => None,
            };
            assert_eq!(
                impact.callables.len(),
                usize::from(expected_kind.is_some()),
                "{language}/{name}"
            );
            if let Some(kind) = expected_kind {
                assert_eq!(impact.callables[0].kind, kind, "{language}/{name}");
                impact.callables[0].occurrence.validate().unwrap();
            }
            assert_eq!(
                impact.breaking_change_signals.signals.len(),
                usize::from(matches!(name, "boundary" | "removed"))
            );
            assert!(impact.bindings.iter().all(|binding| binding.kind
                == entmoot_contracts::query::BindingImpactKind::NotComparable));
            let request = catalog.request();
            let engine = entmoot_engine::QueryEngine::new(graph, "unused.sqlite")
                .with_compatibility(catalog);
            assert_eq!(engine.compatibility(request).unwrap(), result);
        }
    }
}

#[test]
fn partial_enumeration_preserves_known_boundaries_and_unassessed_sites_in_all_ecosystems() {
    for (language, file, ecosystem, package_type, caller, callee) in [
        (
            "rust",
            "lib.rs",
            PackageApiEcosystem::Rust,
            "cargo",
            "fn caller() { api::leaf(); }",
            "pub fn leaf() {}",
        ),
        (
            "javascript",
            "main.js",
            PackageApiEcosystem::Javascript,
            "npm",
            "function caller() { leaf(); }",
            "export function leaf() {}",
        ),
        (
            "go",
            "main.go",
            PackageApiEcosystem::Go,
            "golang",
            "package main\nfunc caller() { Leaf() }",
            "package main\nfunc Leaf() {}",
        ),
        (
            "python",
            "main.py",
            PackageApiEcosystem::Python,
            "pypi",
            "def caller():\n    leaf()\n",
            "def leaf():\n    pass\n",
        ),
    ] {
        let target = PackageApiTarget {
            package: PackageApiPackage {
                ecosystem,
                name: "api".into(),
            },
            symbol: ProviderSymbolSpelling {
                scheme: format!("{language}.fixture"),
                spelling: "api.leaf".into(),
            },
        };
        let base = provider(language, file, callee, &target, true);
        let expected = base.artifact.package_api.as_ref().unwrap().surfaces[0].exports[0]
            .boundary_digest
            .clone();
        for covered in [true, false] {
            let mut input = consumer(language, file, caller, &target, &expected);
            input.artifact.observations[0].targets[0].provider_callable =
                Some(ProviderCallableRef {
                    schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
                    kind: ProviderCallableKind::Callable,
                    package: ProviderPackageRef {
                        role: ProviderPackageRole::Module,
                        purl: Some(format!("pkg:{package_type}/api")),
                        ecosystem: Some(language.into()),
                    },
                    path: vec![],
                    display: None,
                    native: target.symbol.clone(),
                });
            if !covered {
                input
                    .artifact
                    .package_api
                    .as_mut()
                    .unwrap()
                    .references
                    .clear();
            }
            let catalog = CompatibilityCatalog::new(
                materialize(input).0,
                vec![materialize(provider(language, file, callee, &target, true)).0],
            )
            .unwrap();
            let result = catalog.assess(catalog.request()).unwrap();
            assert_eq!(
                result.sites.len(),
                1,
                "{language}: native bindings must not vanish or duplicate"
            );
            assert_eq!(
                result.verdict,
                if covered {
                    Verdict::Fits
                } else {
                    Verdict::Bound
                },
                "{language}: {result:?}"
            );
            if !covered {
                assert_eq!(result.sites[0].verdict, Verdict::Bound);
                assert_eq!(
                    result.sites[0].reason,
                    CompatibilityReason::ProviderExportLocatedContractUnassessed
                );
            }
        }
    }
}

#[test]
fn empty_evidence_does_not_fit_and_wrong_publication_is_rejected() {
    let fixture = fixture("rust", "lib.rs", "fn untouched() {} ");
    let publication =
        PackagePublication::from_graph(&fixture.graph, blake3_hex(b"empty"), |_| Ok(None)).unwrap();
    let catalog = CompatibilityCatalog::new(publication, vec![]).unwrap();
    assert_eq!(
        catalog.assess(catalog.request()).unwrap().verdict,
        Verdict::IncompleteBind
    );
    let mut request = catalog.request();
    request.consumer_publication = "G:/foreign/path".into();
    assert!(catalog.assess(request).is_err());
}

#[test]
fn partial_reference_scope_keeps_limitation_without_blocking_site_fits() {
    let target = PackageApiTarget {
        package: PackageApiPackage {
            ecosystem: PackageApiEcosystem::Javascript,
            name: "api".into(),
        },
        symbol: ProviderSymbolSpelling {
            scheme: "javascript.fixture.qualified_symbol".into(),
            spelling: "example.test/api::sanitize".into(),
        },
    };
    let original = "export function sanitize(x) { return x; }\n";
    let baseline = provider("javascript", "index.js", original, &target, false);
    let expected = baseline.artifact.package_api.as_ref().unwrap().surfaces[0].exports[0]
        .boundary_digest
        .clone();
    let mut input = consumer(
        "javascript",
        "app.js",
        "import {sanitize} from 'api'; function caller() { return sanitize(1); }",
        &target,
        &expected,
    );
    input
        .artifact
        .package_api
        .as_mut()
        .unwrap()
        .references_complete = false;
    let catalog =
        CompatibilityCatalog::new(materialize(input).0, vec![materialize(baseline).0]).unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.verdict, Verdict::Fits);
    assert_eq!(result.sites[0].verdict, Verdict::Fits);
    assert!(result
        .limitations
        .iter()
        .any(|gap| { gap.code == CompatibilityLimitationCode::ConsumerDependencyScopeIncomplete }));
}

#[test]
fn absence_requires_the_exact_symbol_and_policy_universe() {
    let target = rust_target();
    for mode in ["scheme", "policy"] {
        let mut candidate = provider("rust", "lib.rs", "// removed\n", &target, false);
        let universe =
            &mut candidate.artifact.package_api.as_mut().unwrap().surfaces[0].export_universes[0];
        if mode == "scheme" {
            universe.symbol_scheme = "another.compiler.symbol.v2".into();
        } else {
            universe.boundary_policy = "another.boundary.v2".into();
        }
        let input = consumer(
            "rust",
            "lib.rs",
            "fn caller() { api::leaf(); }",
            &target,
            &blake3_hex(b"header"),
        );
        let catalog =
            CompatibilityCatalog::new(materialize(input).0, vec![materialize(candidate).0])
                .unwrap();
        let receipt = catalog.assess(catalog.request()).unwrap();
        assert_eq!(receipt.verdict, Verdict::Unknown, "{mode}");
        assert_eq!(
            receipt.sites[0].reason,
            CompatibilityReason::ExportUniverseUnassessed
        );
    }
    let mut candidate = provider("rust", "lib.rs", "// removed\n", &target, false);
    candidate.artifact.package_api.as_mut().unwrap().surfaces[0]
        .export_universes
        .clear();
    assert!(candidate.artifact.validate().is_err());
}

#[test]
fn external_targets_without_package_metadata_cannot_claim_complete_references() {
    for (purl, reason) in [
        (None, "external_package_coordinate_unavailable"),
        (Some("pkg:gem/example"), "package_ecosystem_unassessed"),
    ] {
        let target = rust_target();
        let mut input = consumer(
            "rust",
            "lib.rs",
            "fn caller() { api::leaf(); }",
            &target,
            &blake3_hex(b"boundary"),
        );
        input
            .artifact
            .package_api
            .as_mut()
            .unwrap()
            .references
            .clear();
        input.artifact.observations[0].targets[0].provider_callable = Some(ProviderCallableRef {
            schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
            kind: ProviderCallableKind::Callable,
            package: ProviderPackageRef {
                role: ProviderPackageRole::Module,
                purl: purl.map(str::to_owned),
                ecosystem: None,
            },
            path: vec![],
            display: None,
            native: target.symbol,
        });
        let catalog = CompatibilityCatalog::new(materialize(input).0, vec![]).unwrap();
        let receipt = catalog.assess(catalog.request()).unwrap();
        assert_eq!(receipt.verdict, Verdict::IncompleteBind);
        assert!(receipt.sites.is_empty());
        assert!(
            receipt.limitations.iter().any(|value| {
                value.code == CompatibilityLimitationCode::UnqualifiedDependencySites
                    && value.detail.as_deref() == Some(reason)
            }),
            "{receipt:?}"
        );
    }
}

#[test]
#[ignore = "native qualification: set ENTMOOT_PYRIGHT_ADAPTER and run with --ignored"]
fn native_python_five_controls() {
    use entmoot_engine::native_resolver::{
        produce_python_resolver, ExecutionPolicy, NativeProduceRequest, ProducerBudget,
    };
    assert!(
        std::env::var_os("ENTMOOT_PYRIGHT_ADAPTER").is_some()
            && entmoot_engine::native_resolver::python::configured_binary().is_some(),
        "native qualification requires an explicit existing ENTMOOT_PYRIGHT_ADAPTER"
    );
    let run = tempfile::tempdir().unwrap();
    let config = run.path().join("environment.json");
    let python = std::process::Command::new("python")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = String::from_utf8(python.stdout).unwrap();
    let prior = std::env::var_os("ENTMOOT_PYTHON_ENVIRONMENT");
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("ENTMOOT_PYTHON_ENVIRONMENT", value),
                None => std::env::remove_var("ENTMOOT_PYTHON_ENVIRONMENT"),
            }
        }
    }
    let _restore = Restore(prior);
    std::env::set_var("ENTMOOT_PYTHON_ENVIRONMENT", &config);
    // Match sidecar request.root spelling (canonicalize + strip Windows \\?\).
    let ordinary = |path: &std::path::Path| -> std::path::PathBuf {
        let canonical = std::fs::canonicalize(path).unwrap();
        #[cfg(windows)]
        {
            use std::path::{Component, Prefix};
            let mut components = canonical.components();
            if let Some(Component::Prefix(prefix)) = components.next() {
                if let Prefix::VerbatimDisk(drive) = prefix.kind() {
                    let mut plain = std::path::PathBuf::from(format!("{}:\\", char::from(drive)));
                    for part in components {
                        if let Component::Normal(value) = part {
                            plain.push(value);
                        }
                    }
                    return plain;
                }
            }
        }
        canonical
    };
    let mut serial = 0;
    let mut native = |mut item: Fixture, package_root: &std::path::Path| {
        let source_root = ordinary(item._root.path());
        let package_root = ordinary(package_root);
        if source_root != package_root {
            // Operator evidence qualifies origin joins, not uv/installation support.
            let lock = b"opaque harness selection";
            std::fs::write(item._root.path().join("fixture.lock"), lock).unwrap();
            let mut instance = entmoot_contracts::package_api::LockedPackageInstance {
                instance_id: String::new(),
                purl: "pkg:pypi/native-python-fixture@1.0.0".into(),
                placement: "fixture/dependency".into(),
                entry: "dependency".into(),
                lock_path: "fixture.lock".into(),
                lock_digest: ContentDigest::from_trusted(entmoot_contracts::blake3_hex(lock)),
            };
            instance.instance_id = instance.derive_id();
            std::fs::create_dir(item._root.path().join(".entmoot")).unwrap();
            std::fs::write(
                item._root.path().join(".entmoot/lock_bindings.json"),
                serde_json::to_vec(&serde_json::json!({
                    "producer":"entmoot.test", "policy":"opaque_fixture.v1",
                    "packages":[{"root":&package_root,"instance":instance}]
                }))
                .unwrap(),
            )
            .unwrap();
            item.graph = entmoot_engine::build_topology_from_repo_with_options(
                item._root.path(),
                TopologyBuildOptions {
                    repository: Some("fixtures/package-api"),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        std::fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "interpreter": python.trim(), "import_roots": [&package_root],
                "packages": [{"name": "native-python-fixture", "module": "", "root": &package_root}]
            }))
            .unwrap(),
        )
        .unwrap();
        let graph_path = run.path().join(format!("{serial}.json"));
        entmoot_engine::write_graph_json(&item.graph, &graph_path).unwrap();
        let result = produce_python_resolver(NativeProduceRequest {
            canon: false,
            graph_path,
            // Ordinary spelling keeps sidecar request.root aligned with environment.json.
            source_root: source_root.clone(),
            output_dir: run.path().join(format!("producer-{serial}")),
            budget: ProducerBudget {
                wall: std::time::Duration::from_secs(90),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy::Safe,
            rust_profile: Default::default(),
        })
        .unwrap();
        assert!(
            !result.raw.invalid_environment,
            "{:?}",
            result.raw.diagnostics
        );
        assert!(!result.raw.budget_exceeded);
        if source_root == package_root {
            assert!(
                result
                    .artifact
                    .package_api
                    .as_ref()
                    .is_some_and(|api| !api.surfaces.is_empty()),
                "same-root package_api surfaces missing; diagnostics={:?} provider_errors={:?} api={:?}",
                result.raw.diagnostics,
                result.raw.build_context.get("provider_errors"),
                result.artifact.package_api
            );
        }
        item.artifact = result.artifact;
        serial += 1;
        item
    };
    for (shape, original, consumer_source, changed, body) in [
        (
            "function",
            "def sanitize(x: int) -> int:\n    return x\n",
            "from api import sanitize\n\ndef caller():\n    return sanitize(1)\n",
            "def sanitize(x: str) -> int:\n    return x\n",
            "def sanitize(x: int) -> int:\n    return 3\n",
        ),
        (
            "method",
            "class Client:\n    def get(self, x: int):\n        return x\n",
            "from api import Client\n\ndef caller(client: Client):\n    return client.get(1)\n",
            "class Client:\n    def get(self, x: str):\n        return x\n",
            "class Client:\n    def get(self, x: int):\n        return 3\n",
        ),
        (
            "constructor",
            "class Client:\n    def __init__(self, x: int):\n        pass\n",
            "from api import Client\n\ndef caller():\n    return Client(1)\n",
            "class Client:\n    def __init__(self, x: str):\n        pass\n",
            "class Client:\n    def __init__(self, x: int):\n        print(x)\n",
        ),
        (
            "function_closed_all",
            "def sanitize(x: int) -> int:\n    return x\n__all__ = [\"sanitize\"]\n",
            "from api import sanitize\n\ndef caller():\n    return sanitize(1)\n",
            "def sanitize(x: str) -> int:\n    return x\n__all__ = [\"sanitize\"]\n",
            "def sanitize(x: int) -> int:\n    return 3\n__all__ = [\"sanitize\"]\n",
        ),
    ] {
        let baseline = fixture("python", "api.py", original);
        let consumer = native(
            fixture("python", "main.py", consumer_source),
            baseline._root.path(),
        );
        for (case, source, verdict) in [
            ("unchanged", original, Verdict::Fits),
            ("removed", "# removed\n", Verdict::BrokenAt),
            ("boundary", changed, Verdict::BrokenAt),
            ("body", body, Verdict::Fits),
            ("structural_only", original, Verdict::IncompleteBind),
        ] {
            let provider = fixture("python", "api.py", source);
            let root = provider._root.path().to_owned();
            let candidate = if case == "structural_only" {
                PackagePublication::from_graph(
                    &provider.graph,
                    blake3_hex(&serde_json::to_vec(&provider.graph).unwrap()),
                    |_| panic!("structural read opened artifact"),
                )
                .unwrap()
            } else {
                materialize(native(provider, &root)).0
            };
            let copy = Fixture {
                _root: tempfile::tempdir().unwrap(),
                graph: consumer.graph.clone(),
                artifact: consumer.artifact.clone(),
            };
            let catalog = CompatibilityCatalog::new(materialize(copy).0, vec![candidate]).unwrap();
            let result = catalog.assess(catalog.request()).unwrap();
            assert_eq!(verdict, result.verdict, "{shape}/{case}: {result:?}");
        }
    }
}

#[test]
fn absent_export_is_not_negative_under_partial_coverage() {
    let target = PackageApiTarget {
        package: PackageApiPackage {
            ecosystem: PackageApiEcosystem::Rust,
            name: "api".into(),
        },
        symbol: ProviderSymbolSpelling {
            scheme: "rust.fixture".into(),
            spelling: "api::sanitize".into(),
        },
    };
    let expected = blake3_hex(b"observed-boundary");
    let (consumer, _) = materialize(consumer(
        "rust",
        "lib.rs",
        "fn caller() { api::sanitize(); }",
        &target,
        &expected,
    ));
    let (provider, _) = materialize(provider(
        "rust",
        "lib.rs",
        "// export unassessed\n",
        &target,
        true,
    ));
    let catalog = CompatibilityCatalog::new(consumer, vec![provider]).unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.verdict, Verdict::IncompleteBind);
    assert_eq!(
        result.sites[0].reason,
        CompatibilityReason::ProviderExportScopeIncomplete
    );
}

fn rust_target() -> PackageApiTarget {
    PackageApiTarget {
        package: PackageApiPackage {
            ecosystem: PackageApiEcosystem::Rust,
            name: "api".into(),
        },
        symbol: ProviderSymbolSpelling {
            scheme: "rust.fixture".into(),
            spelling: "api::sanitize".into(),
        },
    }
}

#[test]
fn legacy_unassessed_bindings_do_not_break_on_absent_exports() {
    let target = rust_target();
    let mut consumer_fixture = fixture("rust", "lib.rs", "fn caller() { api::sanitize(); }");
    let node = consumer_fixture
        .graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::CallStub)
        .unwrap()
        .clone();
    consumer_fixture.artifact.package_api = None;
    consumer_fixture
        .artifact
        .observations
        .push(ResolverObservation {
            dispatch: CallDispatch::Unassessed,
            occurrence_id: node.id.clone(),
            source: ResolverCallSource {
                call_site: topology_node_address(&node.id),
                relation_source: topology_node_address(node.parent.as_ref().unwrap()),
                syntactic_callee: node.label.clone(),
            },
            outcome: ResolverObservationOutcome::ExternalSymbol,
            targets: vec![ResolverTarget {
                address: Address::Opaque {
                    system: target.symbol.scheme.clone(),
                    value: target.symbol.spelling.clone(),
                },
                relation_target: None,
                provider_pointer: "/fixture/target".into(),
                provider_symbol: Some(target.symbol.clone()),
                provider_callable: Some(ProviderCallableRef {
                    schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
                    kind: ProviderCallableKind::Callable,
                    package: ProviderPackageRef {
                        role: ProviderPackageRole::Module,
                        purl: Some(format!("pkg:cargo/{}", target.package.name)),
                        ecosystem: Some("rust".into()),
                    },
                    path: vec![],
                    display: Some(target.symbol.spelling.clone()),
                    native: target.symbol.clone(),
                }),
                declaration: None,
            }],
            limitation: None,
        });
    let (consumer, _) = materialize(consumer_fixture);
    let (provider, _) = materialize(provider(
        "rust",
        "lib.rs",
        "pub fn other() {}",
        &PackageApiTarget {
            package: target.package.clone(),
            symbol: ProviderSymbolSpelling {
                scheme: target.symbol.scheme,
                spelling: "api::other".into(),
            },
        },
        false,
    ));
    let catalog = CompatibilityCatalog::new(consumer, vec![provider]).unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.verdict, Verdict::IncompleteBind);
    assert_eq!(result.sites.len(), 1);
    assert_eq!(
        result.sites[0].reason,
        CompatibilityReason::ConsumerBoundaryUnassessed
    );
}

#[test]
fn unassessed_refs_stitch_when_provider_export_is_located() {
    let target = rust_target();
    let provider_fixture = provider("rust", "lib.rs", "pub fn sanitize() {}", &target, false);
    let expected = provider_fixture
        .artifact
        .package_api
        .as_ref()
        .unwrap()
        .surfaces[0]
        .exports[0]
        .boundary_digest
        .clone();
    let mut input = consumer(
        "rust",
        "lib.rs",
        "fn caller() { api::sanitize(); }",
        &target,
        &expected,
    );
    let reference = &mut input.artifact.package_api.as_mut().unwrap().references[0];
    reference.boundary_policy = "unassessed".into();
    reference.expected_boundary = None;
    reference.lock_instance = None;
    let catalog =
        CompatibilityCatalog::new(materialize(input).0, vec![materialize(provider_fixture).0])
            .unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.verdict, Verdict::Bound);
    assert_eq!(result.sites.len(), 1);
    assert_eq!(
        result.sites[0].reason,
        CompatibilityReason::ProviderExportLocatedContractUnassessed
    );
    assert_eq!(result.sites[0].verdict, Verdict::Bound);
    assert!(result.sites[0].observed_boundary.is_some());
    assert!(result.sites[0].provider_publication.is_some());
}

#[test]
fn mapped_package_root_missing_is_loud_without_provider_stitch() {
    let target = rust_target();
    let mut input = consumer(
        "rust",
        "lib.rs",
        "fn caller() { api::sanitize(); }",
        &target,
        &blake3_hex(b"unused"),
    );
    let api = input.artifact.package_api.as_mut().unwrap();
    api.references[0].boundary_policy = "unassessed".into();
    api.references[0].expected_boundary = None;
    api.references[0].lock_instance = None;
    api.limitations.push(
        CompatibilityLimitation::coded(CompatibilityLimitationCode::MappedPackageRootMissing)
            .with_detail(target.package.name),
    );
    let catalog = CompatibilityCatalog::new(materialize(input).0, vec![]).unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.verdict, Verdict::BrokenAt);
    assert_eq!(
        result.sites[0].reason,
        CompatibilityReason::MappedPackageRootMissing
    );
}

#[test]
fn ambiguous_providers_and_policy_drift_are_unknown() {
    let target = rust_target();
    for duplicate in [false, true] {
        let (consumer, _) = materialize(consumer(
            "rust",
            "lib.rs",
            "fn caller(){ api::sanitize(); }",
            &target,
            &blake3_hex(b"old"),
        ));
        let mut first = provider("rust", "lib.rs", "pub fn sanitize() {}", &target, false);
        if !duplicate {
            first.artifact.package_api.as_mut().unwrap().surfaces[0].exports[0].boundary_policy =
                "different-policy".into();
        }
        let mut providers = vec![materialize(first).0];
        if duplicate {
            providers.push(
                materialize(provider(
                    "rust",
                    "lib.rs",
                    "pub fn sanitize() { let x=1; }",
                    &target,
                    false,
                ))
                .0,
            );
        }
        let catalog = CompatibilityCatalog::new(consumer, providers).unwrap();
        let result = catalog.assess(catalog.request()).unwrap();
        assert_eq!(result.verdict, Verdict::Unknown);
    }
}

#[test]
fn binding_mismatch_bad_digest_and_out_of_inventory_exports_are_rejected() {
    let target = rust_target();
    let mut fixture = consumer(
        "rust",
        "lib.rs",
        "fn caller(){ api::sanitize(); }",
        &target,
        &blake3_hex(b"old"),
    );
    fixture.artifact.package_api.as_mut().unwrap().references[0]
        .target
        .symbol
        .spelling = "different::symbol".into();
    assert!(fixture.artifact.validate().is_err());
    fixture.artifact.package_api.as_mut().unwrap().references[0].target = target.clone();
    fixture.artifact.package_api.as_mut().unwrap().references[0].expected_boundary =
        Some("bad-digest".into());
    assert!(fixture.artifact.validate().is_err());

    let mut fixture = provider("rust", "lib.rs", "pub fn sanitize() {}", &target, false);
    fixture.artifact.package_api.as_mut().unwrap().surfaces[0].exports[0].declaration =
        topology_node_address("absent");
    // The artifact is syntactically valid; inventory validation rejects it before attach.
    fixture.artifact.validate().unwrap();
    let bytes = serde_json::to_vec(&fixture.artifact).unwrap();
    let manifest = ArtifactManifest::build(
        ArtifactFormat::Resolver,
        ArtifactProducer {
            name: fixture.artifact.producer.name.clone(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        fixture.graph.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "fixture",
            vec![],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        ),
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    fixture.graph.artifacts.push(manifest);
    assert!(
        PackagePublication::from_graph(&fixture.graph, blake3_hex(b"graph"), |_| Ok(Some(
            bytes.clone()
        )))
        .is_err()
    );
    assert!(
        PackagePublication::from_graph(&fixture.graph, blake3_hex(b"graph"), |_| Ok(Some(
            b"corrupt".to_vec()
        )))
        .is_err()
    );
}

#[test]
fn missing_artifact_bytes_remain_an_explicit_gap() {
    let target = rust_target();
    let (_, graph) = materialize(provider(
        "rust",
        "lib.rs",
        "pub fn sanitize() {}",
        &target,
        false,
    ));
    let consumer =
        PackagePublication::from_graph(&graph, blake3_hex(b"missing"), |_| Ok(None)).unwrap();
    let catalog = CompatibilityCatalog::new(consumer, vec![]).unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.verdict, Verdict::IncompleteBind);
    assert!(result.limitations.iter().any(|gap| {
        gap.code == CompatibilityLimitationCode::ArtifactUnavailable && gap.artifact_id.is_some()
    }));
}

#[test]
fn package_evidence_attach_is_idempotent_and_failure_atomic() {
    let target = rust_target();
    let mut fixture = provider("rust", "lib.rs", "pub fn sanitize() {}", &target, false);
    let directory = tempfile::tempdir().unwrap();
    let artifact_path = directory.path().join("resolver.json");
    let manifest_path = directory.path().join("resolver.json.manifest.json");
    let write = |artifact: &ResolverArtifact| {
        std::fs::write(&artifact_path, serde_json::to_vec(artifact).unwrap()).unwrap();
        let manifest = entmoot_engine::prepare_resolver_manifest(
            &fixture.graph,
            &artifact_path,
            &artifact.producer.name,
            &artifact.producer.version,
            "rust",
            &[],
        )
        .unwrap();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    };
    write(&fixture.artifact);
    let mut graph = fixture.graph.clone();
    entmoot_engine::ingest_artifact_manifest(&mut graph, &manifest_path, true).unwrap();
    let committed = serde_json::to_vec(&graph).unwrap();
    let retry = entmoot_engine::ingest_artifact_manifest(&mut graph, &manifest_path, true).unwrap();
    assert!(retry.already_present);
    assert_eq!(serde_json::to_vec(&graph).unwrap(), committed);
    fixture.artifact.package_api.as_mut().unwrap().surfaces[0].exports[0].declaration =
        topology_node_address("absent");
    write(&fixture.artifact);
    assert!(entmoot_engine::ingest_artifact_manifest(&mut graph, &manifest_path, true).is_err());
    assert_eq!(serde_json::to_vec(&graph).unwrap(), committed);
}

#[test]
fn missing_roots_are_scoped_to_consumer_reference_artifacts_across_families() {
    for (language, file, ecosystem, source, definition) in [
        (
            "rust",
            "api.rs",
            PackageApiEcosystem::Rust,
            "fn caller(){sanitize();}",
            "pub fn sanitize(){}",
        ),
        (
            "python",
            "api.py",
            PackageApiEcosystem::Python,
            "def caller():\n    sanitize()\n",
            "def sanitize():\n    pass\n",
        ),
        (
            "javascript",
            "api.js",
            PackageApiEcosystem::Javascript,
            "function caller(){sanitize();}",
            "function sanitize(){}",
        ),
        (
            "typescript",
            "api.ts",
            PackageApiEcosystem::Javascript,
            "function caller(){sanitize();}",
            "function sanitize(){}",
        ),
        (
            "tsx",
            "api.tsx",
            PackageApiEcosystem::Javascript,
            "function caller(){sanitize();}",
            "function sanitize(){}",
        ),
        (
            "go",
            "api.go",
            PackageApiEcosystem::Go,
            "package api\nfunc Caller(){ Sanitize() }",
            "package api\nfunc Sanitize(){}",
        ),
        (
            "java",
            "Api.java",
            PackageApiEcosystem::Java,
            "class Api { void caller(){ sanitize(); } }",
            "class Api { void sanitize(){} }",
        ),
        (
            "csharp",
            "Api.cs",
            PackageApiEcosystem::Csharp,
            "class Api { void caller(){ sanitize(); } }",
            "class Api { void sanitize(){} }",
        ),
        (
            "c",
            "api.c",
            PackageApiEcosystem::Native,
            "void caller(){sanitize();}",
            "void sanitize(){}",
        ),
        (
            "cpp",
            "api.cpp",
            PackageApiEcosystem::Native,
            "void caller(){sanitize();}",
            "void sanitize(){}",
        ),
    ] {
        let mut target = rust_target();
        target.package.ecosystem = ecosystem;
        if ecosystem == PackageApiEcosystem::Native {
            target.package.name = "generic:api".into();
        }
        for consumer_missing in [false, true] {
            for stitched in [false, true] {
                let mut input = consumer(language, file, source, &target, &blake3_hex(b"unused"));
                let api = input.artifact.package_api.as_mut().unwrap();
                api.references[0].boundary_policy = "unassessed".into();
                api.references[0].expected_boundary = None;
                api.references[0].lock_instance = None;
                let warning = CompatibilityLimitation::coded(
                    CompatibilityLimitationCode::MappedPackageRootMissing,
                )
                .with_detail(target.package.name.clone());
                if consumer_missing {
                    api.limitations.push(warning.clone());
                }
                let mut offered = if stitched {
                    provider(language, file, definition, &target, false)
                } else {
                    fixture(language, file, definition)
                };
                offered
                    .artifact
                    .package_api
                    .as_mut()
                    .unwrap()
                    .limitations
                    .push(warning);
                let catalog =
                    CompatibilityCatalog::new(materialize(input).0, vec![materialize(offered).0])
                        .unwrap();
                let result = catalog.assess(catalog.request()).unwrap();
                assert_eq!(
                    result.sites[0].verdict,
                    if stitched {
                        Verdict::Bound
                    } else if consumer_missing {
                        Verdict::BrokenAt
                    } else {
                        Verdict::IncompleteBind
                    },
                    "{language} missing={consumer_missing} stitched={stitched}: {result:?}"
                );
                assert!(result.limitations.iter().any(|gap| gap.code
                    == CompatibilityLimitationCode::MappedPackageRootMissing
                    && gap.publication.is_some()
                    && gap.artifact_id.is_some()));
            }
        }
    }
}

#[test]
fn another_consumer_artifact_cannot_supply_a_reference_missing_root_claim() {
    let target = rust_target();
    let mut input = consumer(
        "rust",
        "api.rs",
        "fn caller(){sanitize();}",
        &target,
        &blake3_hex(b"unused"),
    );
    let api = input.artifact.package_api.as_mut().unwrap();
    api.references[0].boundary_policy = "unassessed".into();
    api.references[0].expected_boundary = None;
    api.references[0].lock_instance = None;
    let first_bytes = serde_json::to_vec(&input.artifact).unwrap();
    let mut unrelated = input.artifact.clone();
    unrelated.producer.id = "fixture.unrelated".into();
    unrelated.observations.clear();
    let api = unrelated.package_api.as_mut().unwrap();
    api.references.clear();
    api.limitations.push(
        CompatibilityLimitation::coded(CompatibilityLimitationCode::MappedPackageRootMissing)
            .with_detail(target.package.name),
    );
    let second_bytes = serde_json::to_vec(&unrelated).unwrap();
    let (_, mut graph) = materialize(input);
    let first = &graph.artifacts[0];
    let first_id = first.artifact_id.clone();
    let second = ArtifactManifest::build(
        ArtifactFormat::Resolver,
        first.producer.clone(),
        first.snapshot.clone(),
        first.scope.clone(),
        blake3_hex(&second_bytes),
        None,
    )
    .unwrap();
    graph.artifacts.push(second);
    let digest = blake3_hex(&serde_json::to_vec(&graph).unwrap());
    let publication = PackagePublication::from_graph(&graph, digest, |manifest| {
        Ok(Some(if manifest.artifact_id == first_id {
            first_bytes.clone()
        } else {
            second_bytes.clone()
        }))
    })
    .unwrap();
    let catalog = CompatibilityCatalog::new(publication, vec![]).unwrap();
    let result = catalog.assess(catalog.request()).unwrap();
    assert_eq!(result.sites[0].verdict, Verdict::IncompleteBind);
    assert_eq!(
        result.sites[0].reason,
        CompatibilityReason::ConsumerBoundaryUnassessed
    );
}
