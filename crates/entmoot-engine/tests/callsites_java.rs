//! Java sidecar deep checks (package API attach / compatibility).
//! Product bind/Open smoke rows: `dispatch_expectations` (`java_smoke`).

use std::path::PathBuf;
use std::time::Duration;

use entmoot_contracts::ResolverObservationOutcome;
use entmoot_engine::native_resolver::{
    produce_java_resolver, resolve_sidecar_family, ExecutionPolicy, NativeProduceRequest,
    ProducerBudget,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, write_graph_json, TopologyBuildOptions,
};
use tempfile::tempdir;

#[test]
fn java_smoke_attaches_package_api_for_compatibility() {
    let _adapter = resolve_sidecar_family("java")
        .expect("catalog adapter required; build with scripts/build_resolver_adapters.py");
    std::env::set_var("ENTMOOT_JAVA_ADAPTER", &_adapter);

    let mapping_dir = tempdir().expect("mapping directory");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/entmoot-engine/tests/fixtures/callsites/java/smoke");
    // This integration binary has one test; restore the process-local mapping
    // when it finishes. CLI/MCP tests use child environments independently.
    struct MappingGuard(Option<std::ffi::OsString>);
    impl Drop for MappingGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.0 {
                std::env::set_var("ENTMOOT_PACKAGE_API", value);
            } else {
                std::env::remove_var("ENTMOOT_PACKAGE_API");
            }
        }
    }
    let _guard = MappingGuard(std::env::var_os("ENTMOOT_PACKAGE_API"));
    let mapping = mapping_dir.path().join("packages.json");
    std::fs::write(
        &mapping,
        serde_json::to_vec(&serde_json::json!({"packages":[{
            "ecosystem":"java", "name":"fixture/java-api", "root":root
        }]}))
        .expect("mapping JSON"),
    )
    .expect("write mapping");
    std::env::set_var("ENTMOOT_PACKAGE_API", mapping);
    let mut graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/java"),
        },
    )
    .expect("index fixture");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let out = dir.path().join("resolver-out");

    let result = produce_java_resolver(NativeProduceRequest {
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
    .expect("produce java resolver");

    assert!(
        !result.raw.invalid_environment,
        "{:?}",
        result.raw.diagnostics
    );
    assert!(!result.raw.budget_exceeded);
    assert_eq!(result.receipt.semantic_status.as_str(), "partial");
    assert!(
        result.artifact.observations.iter().any(|observation| {
            observation
                .source
                .syntactic_callee
                .to_ascii_lowercase()
                .contains("help")
                && observation.outcome == ResolverObservationOutcome::RepositoryCallable
        }),
        "expected repository_callable for Sibling.help; status={} diagnostics={:?}",
        result.receipt.semantic_status.as_str(),
        result.receipt.limitations
    );
    let api = result
        .artifact
        .package_api
        .as_ref()
        .expect("normalized package API");
    assert!(!api.surfaces.is_empty());
    assert!(api
        .surfaces
        .iter()
        .any(|surface| !surface.exports.is_empty()));
    assert!(result.raw.build_context["package_api"]["surfaces"]
        .as_array()
        .is_some_and(|surfaces| !surfaces.is_empty()));
    let manifest: entmoot_contracts::ArtifactManifest =
        serde_json::from_slice(&std::fs::read(&result.manifest_path).expect("manifest"))
            .expect("typed manifest");
    let bytes = std::fs::read(&result.artifact_path).expect("artifact bytes");
    let normalized = entmoot_engine::adapter_for_manifest(&manifest)
        .normalize(&bytes, &manifest)
        .expect("normalize attachment");
    assert_eq!(normalized.resolver_package_api.as_ref(), Some(api));
    entmoot_engine::ingest_artifact_manifest(&mut graph, &result.manifest_path, false)
        .expect("attach Java artifact");
    let publication = entmoot_engine::compatibility::PackagePublication::from_graph(
        &graph,
        entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph).expect("graph JSON")),
        |_| Ok(Some(bytes.clone())),
    )
    .expect("read attached package API");
    let catalog = entmoot_engine::compatibility::CompatibilityCatalog::new(publication, vec![])
        .expect("catalog");
    let result = catalog
        .assess(catalog.request())
        .expect("compatibility read");
    assert!(!serde_json::to_string(&result)
        .expect("result")
        .contains("package_api_not_produced"));
}
