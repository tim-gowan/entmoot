//! Opt-in Clang raw-fact qualification; missing or invalid tool pins fail.

use std::time::Duration;

use entmoot_engine::native_resolver::{
    produce_clang_resolver, resolve_sidecar_family, ExecutionPolicy, NativeProduceRequest,
    ProducerBudget,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, write_graph_json, TopologyBuildOptions,
};
use tempfile::tempdir;

#[test]
fn clang_sidecar_reports_environment_or_resolves() {
    let _adapter = resolve_sidecar_family("clang")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_CLANG_ADAPTER", &_adapter);

    for (filename, source, missing_header) in [
        ("main.c", "int help(int x) { return x + 1; }\nint run(void) { return help(2); }\n", false),
        ("main.cpp", "namespace ns { int help(int x) { return x + 1; } int run() { return help(2); } }\n", false),
        ("main.c", "#include \"absent-fixture.h\"\nint help(int x) { return x + 1; }\nint run(void) { return help(2); }\n", true),
    ] {
    let fixture = tempdir().expect("fixture");
    let root = fixture.path().to_path_buf();
    std::fs::write(
        root.join(filename),
        source,
    )
    .expect("source");
    let commands = serde_json::json!([{
        "directory":root, "file":filename, "arguments":["clang","-c",filename]
    }]);
    std::fs::write(
        root.join("compile_commands.json"),
        serde_json::to_vec(&commands).expect("json"),
    )
    .expect("commands");
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/cxx-clang"),
        },
    )
    .expect("index fixture");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let out = dir.path().join("resolver-out");

    let result = produce_clang_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: out,
        budget: ProducerBudget {
            wall: Duration::from_secs(60),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .expect("produce clang resolver");

    assert_eq!(
        result.raw.invalid_environment, missing_header,
        "{:?}",
        result.raw.diagnostics
    );
    assert!(!result.raw.budget_exceeded);
    assert_eq!(result.receipt.inventory_calls, 1);
    assert_eq!(result.artifact.observations.len(), 1);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.kind == entmoot_engine::NodeKind::Callable)
            .count(),
        2
    );
    if missing_header {
        assert_eq!(result.receipt.semantic_status.as_str(), "failed");
        assert!(!result.raw.diagnostics.is_empty());
        // Fatal compiler recovery cannot establish a binding or boundary.
        // The native producer may omit the site; inventory still emits an
        // explicit failed environment attempt rather than reducing coverage.
        assert!(result.raw.calls.is_empty());
        assert!(result.artifact.observations.iter().all(|observation| {
            observation.targets.is_empty()
                && observation.outcome == entmoot_contracts::ResolverObservationOutcome::Failed
                && observation.dispatch == entmoot_contracts::CallDispatch::Open {
                    reason: entmoot_contracts::DispatchGap::InvalidEnvironment
                }
                && observation.limitation.is_some()
        }));
    } else {
        assert_eq!(result.receipt.semantic_status.as_str(), "ok");
        assert!(!result.raw.calls.is_empty(), "healthy native C/C++ call must survive");
        assert!(result.artifact.observations.iter().any(|observation|
            observation.outcome == entmoot_contracts::ResolverObservationOutcome::RepositoryCallable),
            "native binding must join the pre-existing inventory: {:?}", result.artifact.observations);
    }

    }
}
