//! Authored native evidence over production inventory; no compiler qualification implied.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::{query::*, *};
use entmoot_engine::*;
use serde_json::{json, Value};

#[test]
fn source_boundaries_keep_counts_context_attribution_and_unknowns() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.py"),
        "def run(object, other):\n    object.toString()\n    other.toString()\n    missing()\n",
    )
    .unwrap();
    let mut graph =
        build_topology_from_repo_with_options(root.path(), TopologyBuildOptions::default())
            .unwrap();
    let context = ContentDigest::parse(blake3_hex(b"authored-context")).unwrap();
    let artifact = ResolverArtifact {
        schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
        implementation: implementation_fingerprint(),
        producer: ResolverArtifactProducer {
            id: "fixture".into(),
            name: "fixture".into(),
            version: "1".into(),
            method_id: "fixture.v1".into(),
        },
        snapshot: graph.snapshot.clone(),
        language: "python".into(),
        language_version: None,
        build_context_digest: context.clone(),
        assessments: vec![],
        package_api: None,
        observations: graph
            .nodes
            .iter()
            .filter(|n| {
                n.call_site
                    .as_ref()
                    .is_some_and(|s| s.callee.ends_with(".toString"))
            })
            .map(|n| ResolverObservation {
                occurrence_id: n.id.clone(),
                source: ResolverCallSource {
                    call_site: topology_node_address(&n.id),
                    relation_source: topology_node_address(n.parent.as_ref().unwrap()),
                    syntactic_callee: n.label.clone(),
                },
                outcome: ResolverObservationOutcome::Unsupported,
                targets: vec![],
                dispatch: CallDispatch::Open {
                    reason: DispatchGap::IndirectCall("python.future_mechanism".into()),
                },
                limitation: Some("authored unsupported control".into()),
            })
            .collect(),
    };
    assert_eq!(artifact.observations.len(), 2);
    let bytes = serde_json::to_vec(&artifact).unwrap();
    let manifest = ArtifactManifest::build(
        ArtifactFormat::Resolver,
        ArtifactProducer {
            name: "fixture".into(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        graph.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "fixture",
            vec!["python".into()],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        ),
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    let artifact_id = manifest.artifact_id.clone();
    graph.artifacts.push(manifest);
    // A retained attempt from another environment must not satisfy a selected
    // context's reason filter. Context-free structural attempts remain visible.
    let mut other_attempt = graph
        .unresolved_relations
        .iter()
        .find(|a| a.predicate == RelationKind::Calls)
        .unwrap()
        .clone();
    other_attempt.scope.context_id = Some(blake3_hex(b"another-environment"));
    other_attempt.reason = UnresolvedReason::InvalidEnvironment;
    other_attempt.id = compute_unresolved_relation_id(
        &other_attempt.source,
        other_attempt.predicate,
        &other_attempt.target_constraint,
        other_attempt.reason,
        &other_attempt.producer,
        &other_attempt.scope,
        other_attempt.occurrence_id.as_deref(),
    );
    graph.unresolved_relations.push(other_attempt);
    let store = root.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let objects =
        FilesystemArtifactObjectRepository::new(workspace::artifact_store_path(&store).unwrap());
    let digest = ContentDigest::parse(blake3_hex(&bytes)).unwrap();
    let object_path = objects.put(&digest, &bytes).unwrap();
    assert!(objects.get_bounded(&digest, 1).is_err());
    let engine = QueryEngine::new(graph, &store);
    let query = |filter: Value, tier: &str| -> Result<Value, QueryServiceError> {
        engine.execute(serde_json::from_value(json!({"operation":"find", "request":{"kind":"call_boundary","request":filter}, "output":{"tier":tier}})).unwrap()).map(|v| v.into_value().unwrap())
    };
    let result = query(json!({"callee":"*.toString()", "outcome":"unsupported", "reason":"indirect_call", "limit":1}), "export").unwrap();
    let payload = &result["result"];
    assert_eq!(payload["bounds"]["total_matches"], 2, "{result}");
    assert_eq!(payload["items"].as_array().unwrap().len(), 1);
    assert_eq!(payload["boundary_counts"].as_array().unwrap().len(), 2);
    assert_eq!(
        payload["items"][0]["boundary"]["evaluations"][0]["artifact_id"],
        artifact_id
    );
    assert!(payload["items"][0]["boundary"]["content_digest"]
        .as_str()
        .unwrap()
        .starts_with("blake3:"));
    assert_eq!(
        payload["completeness"]["repository"]["status"],
        "not_established"
    );
    let exact = query(
        json!({"callee":"object.toString()", "build_context":context}),
        "compact",
    )
    .unwrap();
    assert_eq!(exact["result_bounds"]["total_matches"], 1);
    let other_environment = query(
        json!({"build_context":context, "reason":"invalid_environment"}),
        "compact",
    )
    .unwrap();
    assert_eq!(other_environment["result_bounds"]["total_matches"], 0);
    let absent = query(
        json!({"callee":"missing", "outcome":"unreported"}),
        "compact",
    )
    .unwrap();
    assert_eq!(absent["result_bounds"]["total_matches"], 1);
    assert!(query(json!({"build_context":blake3_hex(b"unknown")}), "compact").is_err());
    std::fs::remove_file(&object_path).unwrap();
    let unavailable = query(json!({"outcome":"unsupported"}), "compact").unwrap();
    assert_eq!(unavailable["result_bounds"]["total_matches"], 0);
    assert!(!unavailable["limitations"].as_array().unwrap().is_empty());
    std::fs::write(&object_path, b"altered").unwrap();
    assert!(query(json!({}), "compact").is_err());
}
