//! Go sidecar resolver qualification against the mini fixture.

use std::path::PathBuf;
use std::time::Duration;

use entmoot_contracts::{CallDispatch, ResolverObservationOutcome};
use entmoot_engine::native_resolver::{
    produce_go_resolver, resolve_sidecar_family, ExecutionPolicy, NativeProduceRequest,
    ProducerBudget,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, write_graph_json, TopologyBuildOptions,
};
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/entmoot-engine/tests/fixtures/callsites/go/smoke")
}

#[test]
fn go_interface_signature_does_not_select_a_same_named_implementation() {
    let _adapter = resolve_sidecar_family("go")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_GO_ADAPTER", &_adapter);
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("go.mod"), "module example/interface\n\ngo 1.18\n").unwrap();
    let source = "package api\ntype Client interface { GetCart() int }\ntype Concrete struct{}\nfunc (Concrete) GetCart() int { return 1 }\nfunc through(c Client) int { return c.GetCart() }\nfunc direct(c Concrete) int { return c.GetCart() }\nfunc indirect(cb func() int) int { return cb() }\n";
    std::fs::write(root.join("api.go"), source).unwrap();
    std::fs::write(
        root.join("free.go"),
        "package api\nfunc leaf() int { return 3 }\nfunc free() int { return leaf() }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_go_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget {
            wall: Duration::from_secs(120),
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
    assert!(result
        .artifact
        .observations
        .iter()
        .any(|o| o.source.syntactic_callee == "leaf" && o.dispatch.is_closed()));
    let indirect = result
        .artifact
        .observations
        .iter()
        .find(|o| o.source.syntactic_callee == "cb")
        .unwrap();
    assert!(indirect.targets.is_empty());
    assert_eq!(
        indirect.dispatch,
        CallDispatch::Open {
            reason: entmoot_contracts::DispatchGap::IndirectCall("go.indirect_call".into())
        }
    );
    let signature = result
        .artifact
        .observations
        .iter()
        .find(|o| {
            o.targets.first().is_some_and(|t| {
                t.provider_symbol
                    .as_ref()
                    .is_some_and(|s| s.spelling.contains("Client).GetCart"))
            })
        })
        .unwrap();
    assert_eq!(
        signature.outcome,
        ResolverObservationOutcome::RepositorySymbol
    );
    assert_eq!(
        signature.dispatch,
        CallDispatch::Open {
            reason: entmoot_contracts::DispatchGap::IndirectCall("go.dynamic_dispatch".into())
        }
    );
    let target = &signature.targets[0];
    assert!(
        target.relation_target.is_none(),
        "interface declaration is not a concrete body"
    );
    let declaration = target.declaration.as_ref().unwrap();
    assert_eq!(
        declaration.kind,
        entmoot_contracts::ResolverDeclarationKind::Callable
    );
    assert_eq!(
        declaration.callable_association,
        entmoot_contracts::ResolverCallableAssociation::Unknown
    );
    assert_eq!(
        target.provider_callable.as_ref().unwrap().kind,
        entmoot_contracts::ProviderCallableKind::Callable
    );
    let raw = result
        .raw
        .calls
        .iter()
        .find(|c| {
            c.targets
                .first()
                .is_some_and(|t| t.kind == "callable_signature")
        })
        .unwrap();
    assert_eq!(
        &source[raw.targets[0].location.span().start_byte..raw.targets[0].location.span().end_byte],
        "GetCart() int"
    );
    // Unique concrete-method bind must remain present; Closed + policy fences
    // live in dispatch_expectations (ignored until the producer emits Closed).
    assert!(
        result.artifact.observations.iter().any(|o| {
            o.outcome == ResolverObservationOutcome::RepositoryCallable
                && o.targets[0]
                    .provider_symbol
                    .as_ref()
                    .is_some_and(|s| s.spelling.contains("Concrete).GetCart"))
        }),
        "concrete method Candidate bind missing: {:?}",
        result.artifact.observations
    );
}

#[test]
fn go_test_variants_do_not_duplicate_dependency_references() {
    let _adapter = resolve_sidecar_family("go")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_GO_ADAPTER", &_adapter);
    let dir = tempdir().unwrap();
    let root = dir.path().join("consumer");
    let dependency = dir.path().join("dependency");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&dependency).unwrap();
    std::fs::write(
        dependency.join("go.mod"),
        "module example.test/dependency\n\ngo 1.18\n",
    )
    .unwrap();
    std::fs::write(
        dependency.join("api.go"),
        "package dependency\ntype Version int\nfunc Message() string { return \"ok\" }\n",
    )
    .unwrap();
    std::fs::write(root.join("go.mod"), "module example.test/consumer\n\ngo 1.18\nrequire example.test/dependency v0.0.0\nreplace example.test/dependency => ../dependency\n").unwrap();
    std::fs::write(root.join("api.go"), "package consumer\nimport \"example.test/dependency\"\nvar Initial = dependency.Message()\nconst Version = dependency.Version(1)\nfunc Leaf() string { return Initial }\n").unwrap();
    std::fs::write(
        root.join("api_test.go"),
        "package consumer\nfunc check() { Leaf() }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_go_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget {
            wall: Duration::from_secs(120),
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
    assert_eq!(
        result.raw.build_context["package_api"]["references"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let api = result.artifact.package_api.as_ref().unwrap();
    assert_eq!(api.references.len(), 1);
    assert_eq!(
        api.references[0].target.symbol.spelling,
        "example.test/dependency.Message"
    );
    assert!(api.references[0].expected_boundary.is_some());
    let initial = result
        .artifact
        .observations
        .iter()
        .find(|o| o.source.syntactic_callee == "dependency.Message")
        .unwrap();
    assert_eq!(initial.outcome, ResolverObservationOutcome::ExternalSymbol);
    assert!(result
        .raw
        .calls
        .iter()
        .find(|c| c.targets.iter().any(|t| t.symbol.ends_with(".Message")))
        .unwrap()
        .owner
        .is_none());
    let version = result
        .artifact
        .observations
        .iter()
        .find(|o| o.source.syntactic_callee == "dependency.Version")
        .unwrap();
    assert_eq!(version.outcome, ResolverObservationOutcome::Intrinsic);
    assert_eq!(version.dispatch, CallDispatch::Unassessed);
}

#[test]
fn go_test_calls_and_conversions_survive_inventory_join() {
    let _adapter = resolve_sidecar_family("go")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_GO_ADAPTER", &_adapter);
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("go.mod"), "module example/testscope\n\ngo 1.18\n").unwrap();
    std::fs::write(root.join("api.go"), "package api\ntype stringer string\nvar initial = stringer(\"x\")\nfunc prepare() {}\nfunc convert(s stringer) string { return string(s) }\n").unwrap();
    std::fs::write(
        root.join("api_test.go"),
        "package api\nfunc check() { prepare() }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_go_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget {
            wall: Duration::from_secs(120),
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
    assert_eq!(result.raw.build_context["test_scope"], "include");
    for (name, expected, closed) in [
        (
            "prepare",
            ResolverObservationOutcome::RepositoryCallable,
            true,
        ),
        ("stringer", ResolverObservationOutcome::Intrinsic, false),
        ("string", ResolverObservationOutcome::Intrinsic, false),
    ] {
        let site = result
            .artifact
            .observations
            .iter()
            .find(|site| site.source.syntactic_callee == name)
            .unwrap();
        assert_eq!(site.outcome, expected, "{site:?}");
        assert_eq!(site.dispatch.is_closed(), closed, "{site:?}");
        if !closed {
            assert_eq!(site.dispatch, CallDispatch::Unassessed);
        }
        assert_eq!(site.targets[0].relation_target.is_some(), name == "prepare");
    }
}

#[test]
fn go_fixture_retains_sidecar_raw_and_declaration() {
    let _adapter = resolve_sidecar_family("go")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_GO_ADAPTER", &_adapter);

    let root = fixture_root();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/go-native"),
        },
    )
    .expect("index fixture");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let out = dir.path().join("resolver-out");

    let result = produce_go_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: out,
        budget: ProducerBudget {
            wall: Duration::from_secs(120),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .expect("produce go resolver");

    assert_ne!(result.receipt.semantic_status.as_str(), "failed");
    let retained: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            result
                .artifact_path
                .parent()
                .unwrap()
                .join("native.raw.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(retained["sidecar_output"]["stdout"]["encoding"], "utf8");
    let provider: serde_json::Value = serde_json::from_str(
        retained["sidecar_output"]["stdout"]["value"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(provider.get("calls").is_some());
    if let Some(context) = provider.get("build_context") {
        assert_eq!(&retained["build_context"], context);
    }

    let help = result
        .artifact
        .observations
        .iter()
        .find(|observation| {
            observation
                .source
                .syntactic_callee
                .to_ascii_lowercase()
                .contains("help")
                && observation.outcome == ResolverObservationOutcome::RepositoryCallable
        })
        .expect("expected repository_callable for sibling.Help");
    assert!(
        help.targets
            .first()
            .and_then(|target| target.declaration.as_ref())
            .is_some(),
        "repository_callable must carry ResolverDeclaration"
    );
    assert_eq!(help.targets.len(), 1);
    // Closed + policy fences for this free-fn bind: dispatch_expectations
    // (go smoke / go/free_fn), not re-asserted here.
}

#[test]
fn go_modules_with_identical_positions_keep_distinct_bindings() {
    let _adapter = resolve_sidecar_family("go")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_GO_ADAPTER", &_adapter);
    let root = tempdir().unwrap();
    for module in ["a", "b"] {
        std::fs::create_dir(root.path().join(module)).unwrap();
        std::fs::write(
            root.path().join(module).join("go.mod"),
            format!("module example/{module}\n\ngo 1.22\n"),
        )
        .unwrap();
        std::fs::write(
            root.path().join(module).join("main.go"),
            format!("package {module}\nfunc Help() {{}}\nfunc Root() {{ Help() }}\n"),
        )
        .unwrap();
    }
    let raw = entmoot_engine::native_resolver::producers::go::collect_go_observation(
        root.path(),
        Duration::from_secs(120),
        false,
    );
    assert!(!raw.invalid_environment, "{:?}", raw.diagnostics);
    assert!(!raw.budget_exceeded);
    assert_eq!(raw.sidecar_runs.len(), 2);
    assert_eq!(raw.calls.len(), 2);
    for module in ["a", "b"] {
        let call = raw
            .calls
            .iter()
            .find(|call| call.site.path == format!("{module}/main.go"))
            .unwrap();
        assert_eq!(call.targets.len(), 1);
        assert_eq!(call.targets[0].symbol, format!("example/{module}.Help"));
        assert_eq!(
            call.targets[0]
                .location
                .repository()
                .map(|site| site.path.as_str()),
            Some(format!("{module}/main.go").as_str())
        );
        assert!(raw
            .sidecar_runs
            .iter()
            .any(|run| run.source_prefix == module && run.output.is_some()));
    }
}
