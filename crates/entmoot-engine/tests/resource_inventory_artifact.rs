#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use entmoot_contracts::{
    blake3_hex, resource_address, ArtifactFormat, ArtifactManifest, ArtifactProducer, EntityKind,
    Locator, RelationKind, ResourceArtifactRecord, ResourceArtifactRelation,
    ResourceInventoryArtifact, ResourceObservationKind, ScopeCompleteness, ScopeUniverse,
    RESOURCE_INVENTORY_ARTIFACT_SCHEMA,
};
use entmoot_engine::{
    ingest_artifact_manifest, ArtifactAdapter, ResourceInventoryAdapter, TopologyBuildOptions,
};

fn artifact(
    desired: entmoot_contracts::Address,
    suffix: &str,
) -> (
    ResourceInventoryArtifact,
    entmoot_contracts::Address,
    entmoot_contracts::Address,
) {
    let workload = resource_address(
        "kubernetes",
        "cluster/prod/kubernetes/namespace/default",
        "apps/v1/Deployment",
        "django",
    )
    .unwrap();
    let database = resource_address(
        "kubernetes",
        "cluster/prod/kubernetes/namespace/default",
        "v1/Service",
        "postgresql",
    )
    .unwrap();
    let document = ResourceInventoryArtifact {
        schema_version: RESOURCE_INVENTORY_ARTIFACT_SCHEMA.into(),
        environment_id: "cluster/prod".into(),
        observation_context_id: format!("cluster/prod/observation/{suffix}"),
        resources: vec![
            ResourceArtifactRecord {
                provider_pointer: "/resources/django".into(),
                entity_kind: EntityKind::Workload,
                logical_address: workload.clone(),
                display_name: "Deployment/default/django".into(),
                native_type: "apps/v1/Deployment".into(),
                observation_kind: ResourceObservationKind::Observed,
                body_digest: None,
                locators: vec![Locator::K8sObject {
                    cluster: "prod".into(),
                    namespace: "default".into(),
                    api_version: "apps/v1".into(),
                    resource_kind: "Deployment".into(),
                    name: "django".into(),
                }],
                identity_labels: BTreeMap::from([("app".into(), "django".into())]),
            },
            ResourceArtifactRecord {
                provider_pointer: "/resources/postgresql".into(),
                entity_kind: EntityKind::NetworkEndpoint,
                logical_address: database.clone(),
                display_name: "Service/default/postgresql".into(),
                native_type: "v1/Service".into(),
                observation_kind: ResourceObservationKind::Observed,
                body_digest: None,
                locators: Vec::new(),
                identity_labels: BTreeMap::new(),
            },
        ],
        relations: vec![
            ResourceArtifactRelation {
                provider_pointer: "/relations/desired-django".into(),
                subject: desired,
                predicate: RelationKind::ObservedAs,
                object: workload.clone(),
            },
            ResourceArtifactRelation {
                provider_pointer: "/relations/django-postgresql".into(),
                subject: workload.clone(),
                predicate: RelationKind::ConnectsTo,
                object: database.clone(),
            },
        ],
    };
    (document, workload, database)
}

fn graph_with_desired_deployment() -> (tempfile::TempDir, entmoot_engine::TopologyGraph) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("deployment.yaml"),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: django\nspec:\n  selector:\n    matchLabels:\n      app: django\n  template:\n    metadata:\n      labels:\n        app: django\n    spec:\n      containers:\n        - name: django\n          image: example.invalid/django:1\n",
    )
    .unwrap();
    let graph = entmoot_engine::build_topology_from_repo_with_options(
        temp.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("example/defectdojo"),
        },
    )
    .unwrap();
    (temp, graph)
}

fn write_manifest(
    root: &std::path::Path,
    graph: &entmoot_engine::TopologyGraph,
    document: &ResourceInventoryArtifact,
    observed_at: Option<&str>,
) -> std::path::PathBuf {
    let bytes = serde_json::to_vec_pretty(document).unwrap();
    std::fs::write(root.join("runtime-resources.json"), &bytes).unwrap();
    let scope = ScopeUniverse::from_selectors(
        "runtime-observation",
        vec![],
        vec!["deployment.yaml".into()],
        vec![],
        ScopeCompleteness::Partial,
        None,
    );
    let mut manifest = ArtifactManifest::build(
        ArtifactFormat::ResourceInventory,
        ArtifactProducer {
            name: "runtime-fixture".into(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: Some("prod-cluster".into()),
        },
        graph.snapshot.clone(),
        scope,
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    manifest.artifact_path = Some("runtime-resources.json".into());
    manifest.observed_at = observed_at.map(str::to_string);
    let manifest_path = root.join("runtime-resources.manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    manifest_path
}

#[test]
fn observed_resources_require_an_observation_time() {
    let (_temp, graph) = graph_with_desired_deployment();
    let desired = graph.resources[0].logical_address.clone();
    let (document, _, _) = artifact(desired, "one");
    let bytes = serde_json::to_vec(&document).unwrap();
    let scope = ScopeUniverse::from_selectors(
        "runtime",
        vec![],
        vec!["deployment.yaml".into()],
        vec![],
        ScopeCompleteness::Partial,
        None,
    );
    let manifest = ArtifactManifest::build(
        ArtifactFormat::ResourceInventory,
        ArtifactProducer {
            name: "fixture".into(),
            version: None,
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        graph.snapshot,
        scope,
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    let error = ResourceInventoryAdapter
        .normalize(&bytes, &manifest)
        .expect_err("observed resources without observed_at must fail");
    assert!(error.to_string().contains("manifest.observed_at"));
}

#[test]
fn generic_runtime_artifact_stitches_desired_and_observed_resources() {
    let (temp, mut graph) = graph_with_desired_deployment();
    let desired = graph.resources[0].logical_address.clone();
    let (mut document, workload, database) = artifact(desired.clone(), "one");
    let mut duplicate_connection = document.relations[1].clone();
    duplicate_connection.provider_pointer = "/relations/django-postgresql-duplicate".into();
    document.relations.push(duplicate_connection);
    let manifest_path =
        write_manifest(temp.path(), &graph, &document, Some("2026-09-11T12:00:00Z"));

    let stats = ingest_artifact_manifest(&mut graph, &manifest_path, false).unwrap();
    assert_eq!(stats.parsed, 4);
    assert_eq!(graph.resources.len(), 3);
    assert!(graph.relation_edges.iter().any(|edge| {
        edge.subject == desired
            && edge.predicate == RelationKind::ObservedAs
            && edge.object == workload
    }));
    assert!(graph.relation_edges.iter().any(|edge| {
        edge.subject == workload
            && edge.predicate == RelationKind::ConnectsTo
            && edge.object == database
    }));
    assert_eq!(
        graph
            .relation_claims
            .iter()
            .filter(|claim| !claim.evidence.is_empty())
            .count(),
        2
    );
    let connection_edge = graph
        .relation_edges
        .iter()
        .find(|edge| edge.predicate == RelationKind::ConnectsTo)
        .unwrap();
    let connection_claim = graph
        .relation_claims
        .iter()
        .find(|claim| {
            matches!(
                &claim.subject,
                entmoot_contracts::SubjectId::RelationEdge { edge_id } if edge_id == &connection_edge.id
            )
        })
        .unwrap();
    assert_eq!(connection_claim.evidence.len(), 2);
    graph.validate().unwrap();

    let repeated = ingest_artifact_manifest(&mut graph, &manifest_path, false).unwrap();
    assert_eq!(repeated.parsed, 4);
    assert_eq!(graph.resources.len(), 3, "reingestion must be idempotent");

    // A provider ledger retains observed resources and their relationships,
    // while the desired source resource remains owned by structural inventory.
    let manifest = &graph.artifacts[0];
    let declaration = entmoot_contracts::RevalidationDeclaration {
        policy_producer: serde_json::from_value(
            serde_json::json!({"id":"fixture.inputs","name":"Fixture inputs"}),
        )
        .unwrap(),
        method_id: "fixture.resources.v1".into(),
        input_scope: ScopeUniverse::from_selectors(
            "resource-inputs",
            vec![],
            vec!["deployment.yaml".into()],
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&["deployment.yaml".into()]),
        ),
        assumptions: vec!["fixed environment and observation context".into()],
        evidence_refs: vec![entmoot_contracts::EvidenceRef {
            artifact_id: manifest.artifact_id.clone(),
            provider_pointer: "/".into(),
            record_id: None,
        }],
    };
    let mut ledger = entmoot_engine::SqliteKnowledgeRepository::open_in_memory().unwrap();
    entmoot_engine::publish_artifact_evidence(&mut ledger, &graph, manifest, &declaration).unwrap();
    let retained = entmoot_engine::KnowledgeRepository::load(&ledger).unwrap();
    assert_eq!(retained.resources.len(), 2);
    assert!(retained
        .resources
        .iter()
        .all(|r| r.logical_address != desired));
    assert!(retained
        .resources
        .iter()
        .any(|r| r.logical_address == workload && r.native_type == "apps/v1/Deployment"));
    assert_eq!(retained.relation_claims.len(), 2);
    assert_eq!(retained.relation_edges.len(), 2);
    assert_eq!(retained.propositions.len(), 2);
    assert_eq!(
        retained
            .relation_claims
            .iter()
            .map(|c| c.evidence.len())
            .sum::<usize>(),
        3
    );
    assert!(retained.relation_claims.iter().all(|c| c
        .evidence
        .iter()
        .all(|e| e.artifact_id == manifest.artifact_id)));
    assert!(retained.coverages.is_empty() && retained.structural_edges.is_empty());
}

#[test]
fn malformed_runtime_relation_is_rejected_before_graph_mutation() {
    let (temp, mut graph) = graph_with_desired_deployment();
    let before = entmoot_engine::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint();
    let desired = graph.resources[0].logical_address.clone();
    let (mut document, _, _) = artifact(desired, "bad");
    document.relations[0].predicate = RelationKind::Calls;
    let manifest_path =
        write_manifest(temp.path(), &graph, &document, Some("2026-09-11T12:00:00Z"));

    let error = ingest_artifact_manifest(&mut graph, &manifest_path, false)
        .expect_err("non-resource relation predicate must fail");
    assert!(error.to_string().contains("unsupported resource relation"));
    assert_eq!(
        before,
        entmoot_engine::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint()
    );
}

#[test]
fn runtime_relation_with_unknown_endpoint_is_rejected_atomically() {
    let (temp, mut graph) = graph_with_desired_deployment();
    let before = entmoot_engine::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint();
    let desired = graph.resources[0].logical_address.clone();
    let (mut document, _, _) = artifact(desired, "missing");
    document.relations[1].object = resource_address(
        "kubernetes",
        "cluster/prod/kubernetes/namespace/default",
        "v1/Service",
        "missing",
    )
    .unwrap();
    let manifest_path =
        write_manifest(temp.path(), &graph, &document, Some("2026-09-11T12:00:00Z"));

    let error = ingest_artifact_manifest(&mut graph, &manifest_path, false)
        .expect_err("an unknown resource endpoint must fail graph validation");
    assert!(error.to_string().contains("unknown logical resource"));
    assert_eq!(
        before,
        entmoot_engine::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint()
    );
}
