use entmoot_contracts::{
    resource_address, resource_occurrence_id, Address, EntityKind, LocationExtent,
    ResourceObservationKind, ResourceOrigin, ResourceRecord, SourceLocation, SourceRegion,
};

fn source_origin(path: &str, document_index: u32) -> ResourceOrigin {
    ResourceOrigin::Source {
        repository_id: "github.com/example/service".into(),
        revision: "a".repeat(40),
        snapshot_digest: None,
        document_index,
        location: SourceLocation {
            path: path.into(),
            extent: LocationExtent::Region {
                region: SourceRegion::Bytes {
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 0,
                        end_byte: 80,
                    },
                },
            },
        },
    }
}

#[test]
fn logical_resource_identity_is_producer_independent_but_occurrence_is_not() {
    let address = resource_address(
        "kubernetes",
        "github.com/example/service/namespace/default",
        "apps/v1/Deployment",
        "api",
    )
    .unwrap();
    let moved = source_origin("deploy/api.yaml", 0);
    let original = source_origin("k8s/api.yaml", 0);
    let mut shifted = source_origin("k8s/api.yaml", 0);
    let ResourceOrigin::Source { location, .. } = &mut shifted else {
        unreachable!()
    };
    let LocationExtent::Region { region } = &mut location.extent else {
        unreachable!()
    };
    let SourceRegion::Bytes { span } = region else {
        unreachable!()
    };
    span.start_byte = 2;
    let artifact = ResourceOrigin::Artifact {
        artifact_id: "b".repeat(64),
        provider_pointer: "/resources/17".into(),
    };

    let resource_id = entmoot_contracts::resource_id(&address).unwrap();
    assert_eq!(
        resource_id,
        entmoot_contracts::resource_id(&address).unwrap()
    );
    assert_ne!(
        resource_occurrence_id(&resource_id, &original).unwrap(),
        resource_occurrence_id(&resource_id, &moved).unwrap()
    );
    assert_ne!(
        resource_occurrence_id(&resource_id, &original).unwrap(),
        resource_occurrence_id(&resource_id, &shifted).unwrap()
    );
    assert_ne!(
        resource_occurrence_id(&resource_id, &original).unwrap(),
        resource_occurrence_id(&resource_id, &artifact).unwrap()
    );
}

#[test]
fn record_validates_canonical_ids_and_strict_wire_shape() {
    let address = resource_address(
        "terraform",
        "github.com/example/service/module/root",
        "aws_lb",
        "public",
    )
    .unwrap();
    let record = ResourceRecord::build(
        EntityKind::InfrastructureResource,
        address,
        "aws_lb.public".into(),
        "aws_lb".into(),
        ResourceObservationKind::Declared,
        source_origin("main.tf", 0),
        Some(format!("blake3:{}", "c".repeat(64))),
    )
    .unwrap();

    record.validate().unwrap();
    let mut value = serde_json::to_value(&record).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("vendor_payload".into(), serde_json::json!({"secret": "no"}));
    assert!(serde_json::from_value::<ResourceRecord>(value).is_err());
}

#[test]
fn resource_address_rejects_empty_or_noncanonical_components() {
    assert!(resource_address("", "scope", "type", "name").is_err());
    assert!(resource_address("kubernetes", "scope", "", "name").is_err());
    assert!(Address::Resource {
        resource_system: " kubernetes".into(),
        resource_namespace: "scope".into(),
        resource_type: "v1/Service".into(),
        logical_name: "api".into(),
    }
    .validate()
    .is_err());
}
