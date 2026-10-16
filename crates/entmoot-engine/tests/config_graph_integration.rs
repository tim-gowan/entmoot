#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_contracts::{
    blake3_hex, ArtifactFormat, ArtifactManifest, ArtifactProducer, AttachmentTargetKind,
    ScopeCompleteness, ScopeUniverse,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, ingest_artifact_manifest, KnowledgeRepository,
    KnowledgeStore, NodeKind, SqliteKnowledgeRepository, TopologyBuildOptions,
};

fn fixture_repository() -> (tempfile::TempDir, entmoot_engine::TopologyGraph) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("deploy")).unwrap();
    std::fs::write(
        directory.path().join("deploy/app.yaml"),
        r#"apiVersion: v1
kind: ConfigMap
metadata: { name: app-config, namespace: prod }
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: api, namespace: prod }
spec:
  template:
    metadata:
      labels: { app: api }
    spec:
      containers:
        - name: api
          image: example/api:1
          envFrom:
            - configMapRef: { name: app-config }
---
apiVersion: v1
kind: Service
metadata: { name: api, namespace: prod }
spec:
  selector: { app: api }
"#,
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        directory.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("github.com/example/service"),
        },
    )
    .unwrap();
    (directory, graph)
}

#[test]
fn repository_index_materializes_resources_relations_and_durable_queries() {
    let (_directory, graph) = fixture_repository();
    assert_eq!(graph.resources.len(), 3);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Resource)
            .count(),
        3
    );
    assert_eq!(graph.relation_edges.len(), 2);
    assert_eq!(graph.relation_claims.len(), 2);
    graph.validate().unwrap();

    let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
    repository
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let resource_id = graph.resources[0].resource_id.clone();
    assert_eq!(
        repository
            .resources_by_logical_id(&resource_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn checkov_sarif_location_attaches_to_smallest_configuration_resource() {
    let (directory, mut graph) = fixture_repository();
    let artifact = serde_json::to_vec(&serde_json::json!({
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "Checkov",
                "version": "3.2.0-test",
                "informationUri": "https://www.checkov.io/"
            }},
            "results": [{
                "ruleId": "CKV_K8S_14",
                "level": "error",
                "message": {"text": "Image tag should be immutable"},
                "fingerprints": {"checkov/resource": "Deployment.prod.api"},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": "deploy/app.yaml"},
                    "region": {"startLine": 6, "startColumn": 1, "endLine": 17, "endColumn": 1}
                }}]
            }]
        }]
    }))
    .unwrap();
    std::fs::write(directory.path().join("checkov.sarif"), &artifact).unwrap();
    let scope = ScopeUniverse::from_selectors(
        "iac",
        vec!["yaml".into()],
        vec!["deploy/app.yaml".into()],
        vec![],
        ScopeCompleteness::Partial,
        None,
    );
    let mut manifest = ArtifactManifest::build(
        ArtifactFormat::Sarif21,
        ArtifactProducer {
            name: "Checkov".into(),
            version: Some("3.2.0-test".into()),
            information_uri: Some("https://www.checkov.io/".into()),
            ruleset_id: Some("checkov-default".into()),
            config_class: Some("iac".into()),
        },
        graph.snapshot.clone(),
        scope,
        blake3_hex(&artifact),
        None,
    )
    .unwrap();
    manifest.artifact_path = Some("checkov.sarif".into());
    let manifest_path = directory.path().join("checkov.manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let stats = ingest_artifact_manifest(&mut graph, &manifest_path, false).unwrap();
    assert_eq!(stats.resource_attached, 1);
    let target = graph.attachments[0].selected_target.as_ref().unwrap();
    assert_eq!(target.kind, AttachmentTargetKind::Resource);
    let selected = graph.node(target.node_id.as_deref().unwrap()).unwrap();
    assert_eq!(selected.kind, NodeKind::Resource);
    assert!(selected.label.contains("Deployment"));
    graph.validate().unwrap();
    // Configuration-only transition with a real adapter fixture. The provider
    // observation is historical evidence, not a claim that the new image is safe.
    let head_directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(head_directory.path().join("deploy")).unwrap();
    let source = std::fs::read_to_string(directory.path().join("deploy/app.yaml")).unwrap();
    std::fs::write(
        head_directory.path().join("deploy/app.yaml"),
        source.replace("example/api:1", "example/api:2"),
    )
    .unwrap();
    let head = build_topology_from_repo_with_options(
        head_directory.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("github.com/example/service"),
        },
    )
    .unwrap();
    let impact = entmoot_engine::compare_revision_impact(&graph, &head).unwrap();
    assert_eq!(impact.files.len(), 1);
    assert_eq!(impact.files[0].path, "deploy/app.yaml");
    assert!(impact.callables.is_empty());
    assert!(impact.bindings.is_empty());
    assert_eq!(head.resources.len(), graph.resources.len());
    let stale_finding = impact
        .evidence
        .iter()
        .find(|e| e.record_kind == "finding_observation")
        .unwrap();
    assert_eq!(
        stale_finding.disposition,
        entmoot_contracts::query::EvidenceImpactKind::Stale
    );
    assert!(!impact.rescans.is_empty());
    assert!(
        head.findings.is_empty(),
        "missing scanner evidence is not a new negative finding"
    );
}

#[test]
fn repository_merge_resolves_cross_file_terraform_and_kubernetes_links_deterministically() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("infra")).unwrap();
    std::fs::create_dir_all(directory.path().join("deploy")).unwrap();
    std::fs::write(
        directory.path().join("infra/network.tf"),
        "resource \"aws_vpc\" \"main\" { cidr_block = \"10.0.0.0/16\" }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("infra/app.tf"),
        "resource \"aws_subnet\" \"app\" { vpc_id = aws_vpc.main.id }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("deploy/service.yaml"),
        "apiVersion: v1\nkind: Service\nmetadata: { name: api }\nspec:\n  selector: { app: api }\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("deploy/workload.yaml"),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata: { name: api }\nspec:\n  template:\n    metadata:\n      labels: { app: api }\n",
    )
    .unwrap();

    let build = || {
        build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("github.com/example/cross-file"),
            },
        )
        .unwrap()
    };
    let first = build();
    let second = build();
    assert_eq!(first.resources.len(), 4);
    assert_eq!(first.relation_edges.len(), 2);
    assert!(first.unresolved_relations.is_empty());
    assert_eq!(
        first
            .relation_edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect::<Vec<_>>(),
        second
            .relation_edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect::<Vec<_>>()
    );
}
