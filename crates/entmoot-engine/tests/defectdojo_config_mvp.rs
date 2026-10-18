#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use entmoot_contracts::{
    blake3_hex, resource_address, Address, ArtifactFormat, ArtifactManifest, ArtifactProducer,
    AttachmentTargetKind, Locator, RelationKind, ResourceArtifactRecord, ResourceArtifactRelation,
    ResourceInventoryArtifact, ResourceObservationKind, ScopeCompleteness, ScopeUniverse,
    RESOURCE_INVENTORY_ARTIFACT_SCHEMA,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, ingest_artifact_manifest, BoundedWalkBudget,
    InspectQuery, KnowledgeRead, KnowledgeRepository, KnowledgeStore, QueryEngine,
    SqliteKnowledgeRepository, TopologyBuildOptions, TraverseQuery,
};

const RENDERED: &str = include_str!(
    "../../../crates/entmoot-engine/tests/fixtures/config_inventory/defectdojo-rendered.yaml"
);

fn manifest_for(
    root: &std::path::Path,
    graph: &entmoot_engine::TopologyGraph,
    name: &str,
    bytes: &[u8],
    format: ArtifactFormat,
    producer: ArtifactProducer,
) -> std::path::PathBuf {
    std::fs::write(root.join(name), bytes).unwrap();
    let scope = ScopeUniverse::from_selectors(
        "defectdojo-mvp",
        vec!["yaml".into()],
        vec!["defectdojo-rendered.yaml".into()],
        vec![],
        ScopeCompleteness::Partial,
        None,
    );
    let mut manifest = ArtifactManifest::build(
        format,
        producer,
        graph.snapshot.clone(),
        scope,
        blake3_hex(bytes),
        None,
    )
    .unwrap();
    manifest.artifact_path = Some(name.into());
    if format == ArtifactFormat::ResourceInventory {
        manifest.observed_at = Some("2026-09-11T12:00:00Z".into());
    }
    let path = root.join(format!("{name}.manifest.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    path
}

fn observed_address(declared: &Address) -> Address {
    let Address::Resource {
        resource_type,
        logical_name,
        ..
    } = declared
    else {
        panic!("fixture resource must have a resource address");
    };
    resource_address(
        "kubernetes",
        "cluster/prod/kubernetes/namespace/defectdojo",
        resource_type,
        logical_name,
    )
    .unwrap()
}

fn resource_address_by_kind_and_name<'a>(
    resources: &'a [ResourceArtifactRecord],
    kind: &str,
    name: &str,
) -> &'a Address {
    let matches: Vec<_> = resources
        .iter()
        .filter(|resource| {
            let Address::Resource { logical_name, .. } = &resource.logical_address else {
                unreachable!()
            };
            logical_name == name && resource.native_type.rsplit('/').next() == Some(kind)
        })
        .collect();
    assert_eq!(matches.len(), 1, "fixture requires one {kind}/{name}");
    &matches[0].logical_address
}

const PROCESSES: &[(&str, &str)] = &[
    ("Deployment", "defectdojo-django"),
    ("Deployment", "defectdojo-celery-worker"),
    ("Deployment", "defectdojo-celery-beat"),
    ("Job", "defectdojo-initializer"),
];

#[test]
fn defectdojo_mvp_connects_every_deployed_process_to_database_and_broker() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("defectdojo-rendered.yaml"), RENDERED).unwrap();
    let mut graph = build_topology_from_repo_with_options(
        directory.path(),
        TopologyBuildOptions {
            policy: None,
            repository: Some("github.com/DefectDojo/django-DefectDojo"),
        },
    )
    .unwrap();

    assert_eq!(graph.resources.len(), 12);
    assert_eq!(graph.relation_edges.len(), 12);
    assert!(graph.unresolved_relations.is_empty());

    let observed_resources = graph
        .resources
        .iter()
        .map(|declared| ResourceArtifactRecord {
            provider_pointer: format!("/resources/{}", declared.resource_id),
            entity_kind: declared.entity_kind.clone(),
            logical_address: observed_address(&declared.logical_address),
            display_name: declared.display_name.clone(),
            native_type: declared.native_type.clone(),
            observation_kind: ResourceObservationKind::Observed,
            body_digest: None,
            locators: vec![Locator::Opaque {
                system: "runtime_fixture".into(),
                value: declared.display_name.clone(),
            }],
            identity_labels: BTreeMap::new(),
        })
        .collect::<Vec<_>>();
    let mut runtime_relations = graph
        .resources
        .iter()
        .map(|declared| ResourceArtifactRelation {
            provider_pointer: format!("/relations/stitches/{}", declared.resource_id),
            subject: declared.logical_address.clone(),
            predicate: RelationKind::ObservedAs,
            object: observed_address(&declared.logical_address),
        })
        .collect::<Vec<_>>();
    for &(kind, process) in PROCESSES {
        for endpoint in ["defectdojo-postgresql", "defectdojo-valkey"] {
            runtime_relations.push(ResourceArtifactRelation {
                provider_pointer: format!("/relations/connectors/{process}/{endpoint}"),
                subject: resource_address_by_kind_and_name(&observed_resources, kind, process)
                    .clone(),
                predicate: RelationKind::ConnectsTo,
                object: resource_address_by_kind_and_name(
                    &observed_resources,
                    "StatefulSet",
                    endpoint,
                )
                .clone(),
            });
        }
    }
    let runtime = ResourceInventoryArtifact {
        schema_version: RESOURCE_INVENTORY_ARTIFACT_SCHEMA.into(),
        environment_id: "cluster/prod".into(),
        observation_context_id: "cluster/prod/2026-09-11T12:00:00Z".into(),
        resources: observed_resources,
        relations: runtime_relations,
    };
    let runtime_bytes = serde_json::to_vec_pretty(&runtime).unwrap();
    let runtime_manifest = manifest_for(
        directory.path(),
        &graph,
        "runtime-resources.json",
        &runtime_bytes,
        ArtifactFormat::ResourceInventory,
        ArtifactProducer {
            name: "generic-runtime-fixture".into(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: Some("prod-cluster".into()),
        },
    );
    let runtime_stats = ingest_artifact_manifest(&mut graph, &runtime_manifest, false).unwrap();
    assert_eq!(runtime_stats.parsed, 32);
    assert_eq!(graph.resources.len(), 24);
    assert_eq!(
        graph
            .relation_edges
            .iter()
            .filter(|edge| edge.predicate == RelationKind::ObservedAs)
            .count(),
        12
    );
    assert_eq!(
        graph
            .relation_edges
            .iter()
            .filter(|edge| edge.predicate == RelationKind::ConnectsTo)
            .count(),
        8
    );

    let deployment_line = u32::try_from(
        RENDERED
            .lines()
            .position(|line| line == "kind: Deployment")
            .unwrap(),
    )
    .unwrap()
        + 1;
    let checkov = serde_json::to_vec_pretty(&serde_json::json!({
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "Checkov",
                "version": "fixture",
                "informationUri": "https://www.checkov.io/"
            }},
            "results": [{
                "ruleId": "CKV_K8S_FIXTURE",
                "level": "warning",
                "message": {"text": "Sanitized Checkov integration fixture"},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": "defectdojo-rendered.yaml"},
                    "region": {"startLine": deployment_line, "startColumn": 1}
                }}]
            }]
        }]
    }))
    .unwrap();
    let checkov_manifest = manifest_for(
        directory.path(),
        &graph,
        "checkov.sarif",
        &checkov,
        ArtifactFormat::Sarif21,
        ArtifactProducer {
            name: "Checkov".into(),
            version: Some("fixture".into()),
            information_uri: Some("https://www.checkov.io/".into()),
            ruleset_id: Some("fixture".into()),
            config_class: Some("iac".into()),
        },
    );
    let checkov_stats = ingest_artifact_manifest(&mut graph, &checkov_manifest, false).unwrap();
    assert_eq!(checkov_stats.resource_attached, 1);
    assert_eq!(
        graph
            .attachments
            .last()
            .unwrap()
            .selected_target
            .as_ref()
            .unwrap()
            .kind,
        AttachmentTargetKind::Resource
    );

    let mut store = SqliteKnowledgeRepository::open_in_memory().unwrap();
    store
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    for &(kind, process) in PROCESSES {
        let relations = store
            .relation_edges_for_subject(resource_address_by_kind_and_name(
                &runtime.resources,
                kind,
                process,
            ))
            .unwrap();
        assert_eq!(
            relations
                .iter()
                .filter(|edge| edge.predicate == RelationKind::ConnectsTo)
                .count(),
            2,
            "{process} must expose both deployed connector observations"
        );
    }

    let query_store_path = directory.path().join("query.sqlite3");
    let mut query_store = SqliteKnowledgeRepository::open(&query_store_path).unwrap();
    query_store
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    drop(query_store);
    let ingress = graph
        .resources
        .iter()
        .find(|resource| {
            resource.native_type.ends_with("/Ingress")
                && resource.observation_kind == ResourceObservationKind::Declared
        })
        .unwrap();
    let query = QueryEngine::new(graph.clone(), query_store_path);
    let context = query
        .inspect(InspectQuery::Subject {
            id: ingress.occurrence_id.clone(),
            limit: Some(20),
        })
        .unwrap()
        .into_value()
        .expect("serialize result");
    assert_eq!(context["status"], "known");
    assert!(context["relation_neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|neighbor| neighbor["predicate"] == "routes_to"
            && neighbor["subjects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|subject| subject["label"]
                    .as_str()
                    .is_some_and(|label| label.contains("defectdojo-django")))));
    let service_subject = context["relation_neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|neighbor| neighbor["predicate"] == "routes_to")
        .and_then(|neighbor| neighbor["subjects"].as_array())
        .and_then(|subjects| subjects.first())
        .and_then(|subject| subject["node_id"].as_str())
        .unwrap();
    let service_context = query
        .inspect(InspectQuery::Subject {
            id: service_subject.into(),
            limit: Some(20),
        })
        .unwrap()
        .into_value()
        .expect("serialize result");
    let deployment_subject = service_context["relation_neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|neighbor| neighbor["predicate"] == "selects")
        .and_then(|neighbor| neighbor["subjects"].as_array())
        .and_then(|subjects| {
            subjects.iter().find(|subject| {
                subject["label"]
                    .as_str()
                    .is_some_and(|label| label.contains("defectdojo-django"))
            })
        })
        .and_then(|subject| subject["node_id"].as_str())
        .unwrap();
    let deployment_context = query
        .inspect(InspectQuery::Subject {
            id: deployment_subject.into(),
            limit: Some(20),
        })
        .unwrap()
        .into_value()
        .expect("serialize result");
    let observed_subject = deployment_context["relation_neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|neighbor| neighbor["predicate"] == "observed_as")
        .and_then(|neighbor| neighbor["subjects"].as_array())
        .and_then(|subjects| subjects.first())
        .and_then(|subject| subject["node_id"].as_str())
        .unwrap();
    let observed_context = query
        .inspect(InspectQuery::Subject {
            id: observed_subject.into(),
            limit: Some(20),
        })
        .unwrap()
        .into_value()
        .expect("serialize result");
    let connected_labels: Vec<_> = observed_context["relation_neighbors"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("observed deployment has no relation neighbors: {observed_context}")
        })
        .iter()
        .filter(|neighbor| neighbor["predicate"] == "connects_to")
        .flat_map(|neighbor| neighbor["subjects"].as_array().into_iter().flatten())
        .filter_map(|subject| subject["label"].as_str())
        .collect();
    assert!(connected_labels
        .iter()
        .any(|label| label.contains("defectdojo-postgresql")));
    assert!(connected_labels
        .iter()
        .any(|label| label.contains("defectdojo-valkey")));

    let observed_postgresql = observed_context["relation_neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|neighbor| neighbor["predicate"] == "connects_to")
        .flat_map(|neighbor| neighbor["subjects"].as_array().into_iter().flatten())
        .find(|subject| {
            subject["label"]
                .as_str()
                .is_some_and(|label| label.contains("defectdojo-postgresql"))
        })
        .and_then(|subject| subject["node_id"].as_str())
        .unwrap();
    for max_hops in 1..=4 {
        let walk = query
            .traverse(TraverseQuery::Path {
                evidence_filter: Default::default(),
                prune: Default::default(),
                resumable: false,
                continuation: None,
                start: ingress.occurrence_id.clone(),
                target: observed_postgresql.into(),
                predicates: vec![],
                budget: BoundedWalkBudget {
                    max_hops,
                    max_expansions: 1_000,
                    max_edges_per_node: 50,
                    max_examined_edges: 100_000,
                    max_results: 10,
                    max_evidence_per_hop: 20,
                },
            })
            .unwrap()
            .into_value()
            .expect("serialize result");
        if max_hops < 4 {
            assert_eq!(walk["status"], "unknown", "budget {max_hops}");
            assert!(walk["paths"].as_array().unwrap().is_empty());
        } else {
            assert_eq!(walk["status"], "known");
            assert_eq!(walk["paths"][0]["hop_count"], 4);
            let predicates: Vec<_> = walk["paths"][0]["hops"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hop| hop["relation_edge"]["predicate"].as_str().unwrap())
                .collect();
            assert_eq!(
                predicates,
                ["routes_to", "selects", "observed_as", "connects_to"]
            );
        }
    }
    // Explicit declared-to-observed resource edges authorize a context boundary,
    // in either search direction, without weakening ordinary call composition.
    for (start, direction) in [
        (
            ingress.occurrence_id.as_str(),
            entmoot_contracts::query::CallDirection::Callees,
        ),
        (
            observed_postgresql,
            entmoot_contracts::query::CallDirection::Callers,
        ),
    ] {
        let neighborhood = query
            .traverse(TraverseQuery::Neighborhood {
                endpoint_selector: None,
                prune: Default::default(),
                resumable: false,
                continuation: None,
                start: start.into(),
                direction,
                predicates: vec![],
                evidence_filter: entmoot_contracts::query::HopEvidenceFilter::Any,
                endpoint_role: None,
                budget: BoundedWalkBudget {
                    max_hops: 4,
                    max_results: 20,
                    ..Default::default()
                },
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert!(
            neighborhood["items"].as_array().unwrap().iter().any(|hit| {
                let predicates: Vec<_> = hit["witness"]["hops"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|hop| hop["relation_edge"]["predicate"].as_str().unwrap())
                    .collect();
                predicates == ["routes_to", "selects", "observed_as", "connects_to"]
            }),
            "{direction:?}: {neighborhood}"
        );
    }
    for mutation in ["implicit", "foreign-snapshot", "different-observation"] {
        let mut incompatible = graph.clone();
        for edge in &mut incompatible.relation_edges {
            if edge.predicate == RelationKind::ObservedAs {
                if mutation == "implicit" {
                    edge.predicate = RelationKind::DependsOn;
                } else if mutation == "foreign-snapshot" {
                    edge.scope.snapshot_revision = Some("other-snapshot".into());
                }
            } else if edge.predicate == RelationKind::ConnectsTo
                && mutation == "different-observation"
            {
                edge.scope.context_id = Some("other-observation".into());
            }
            *edge = entmoot_contracts::relation_edge_with_id(
                &edge.schema_domain,
                edge.subject.clone(),
                edge.predicate,
                edge.object.clone(),
                edge.scope.clone(),
                edge.occurrence_id.clone(),
            );
        }
        let incompatible = QueryEngine::new(incompatible, directory.path().join("query.sqlite3"));
        let walk = incompatible
            .traverse(TraverseQuery::Path {
                evidence_filter: Default::default(),
                prune: Default::default(),
                resumable: false,
                continuation: None,
                start: ingress.occurrence_id.clone(),
                target: observed_postgresql.into(),
                predicates: vec![],
                budget: BoundedWalkBudget {
                    max_hops: 4,
                    ..Default::default()
                },
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert!(
            walk["paths"].as_array().unwrap().is_empty(),
            "{mutation}: {walk}"
        );
        for (start, direction) in [
            (
                ingress.occurrence_id.as_str(),
                entmoot_contracts::query::CallDirection::Callees,
            ),
            (
                observed_postgresql,
                entmoot_contracts::query::CallDirection::Callers,
            ),
        ] {
            let walk = incompatible
                .traverse(TraverseQuery::Neighborhood {
                    endpoint_selector: None,
                    prune: Default::default(),
                    resumable: false,
                    continuation: None,
                    start: start.into(),
                    direction,
                    predicates: vec![],
                    evidence_filter: entmoot_contracts::query::HopEvidenceFilter::Any,
                    endpoint_role: None,
                    budget: BoundedWalkBudget {
                        max_hops: 4,
                        max_results: 20,
                        ..Default::default()
                    },
                })
                .unwrap()
                .into_value()
                .unwrap();
            assert!(
                walk["items"].as_array().unwrap().iter().all(|hit| {
                    let predicates: Vec<_> = hit["witness"]["hops"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|hop| hop["relation_edge"]["predicate"].as_str().unwrap())
                        .collect();
                    predicates != ["routes_to", "selects", "observed_as", "connects_to"]
                }),
                "{mutation}: {walk}"
            );
        }
    }
    graph.validate().unwrap();
}
