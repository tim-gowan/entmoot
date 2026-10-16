//! Produce → ingest fills occurrence-scoped resolved identities (ADR-0024).

#![cfg(feature = "resolved-oxc")]

use std::path::PathBuf;
use std::time::Duration;

use entmoot_contracts::ResolvedIdentityCompleteness;
use entmoot_engine::native_resolver::{
    produce_oxc_resolver, ExecutionPolicy, NativeProduceRequest, ProducerBudget,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, ingest_artifact_manifest, write_graph_json,
    TopologyBuildOptions,
};
use tempfile::tempdir;

#[test]
fn oxc_produce_ingest_emits_resolved_identities() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/entmoot-engine/tests/fixtures/callsites/javascript/smoke");
    let mut graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/js-oxc-resolved"),
        },
    )
    .expect("index");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let out = dir.path().join("resolver-out");
    let produced = produce_oxc_resolver(NativeProduceRequest {
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
    .expect("produce");
    assert!(!produced.artifact.observations.is_empty());
    let stats = ingest_artifact_manifest(&mut graph, &produced.manifest_path, true)
        .expect("ingest resolver");
    assert!(
        stats.resolved_identities > 0,
        "expected resolved identities after Oxc ingest"
    );
    assert!(graph.resolved_identities.iter().any(|record| {
        record.binding.completeness() == ResolvedIdentityCompleteness::Complete
            || record.binding.completeness() == ResolvedIdentityCompleteness::Partial
    }));
}
