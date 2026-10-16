#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_contracts::{blake3_hex, Address, RelationKind};
use entmoot_engine::{
    index_configuration_source, index_repository_configurations, ConfigSource, ConfigSyntax,
    SnapshotFileDigest,
};

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn source<'a>(path: &'a str, syntax: ConfigSyntax, text: &'a str) -> ConfigSource<'a> {
    ConfigSource {
        repository_id: "github.com/example/service",
        revision: REVISION,
        snapshot_digest: None,
        path,
        syntax,
        text,
    }
}

fn relation_count(result: &entmoot_engine::ConfigInventory, kind: RelationKind) -> usize {
    result
        .relation_edges
        .iter()
        .filter(|edge| edge.predicate == kind)
        .count()
}

#[test]
fn kubernetes_yaml_indexes_resources_and_native_links() {
    let text = r#"apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
  namespace: prod
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
  namespace: prod
spec:
  selector:
    matchLabels: { app: api }
  template:
    metadata:
      labels: { app: api }
    spec:
      containers:
        - name: api
          image: example/api:1
          envFrom:
            - configMapRef:
                name: app-config
---
apiVersion: v1
kind: Service
metadata: { name: api, namespace: prod }
spec:
  selector: { app: api }
  ports: [{ port: 80, targetPort: 8080 }]
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata: { name: public, namespace: prod }
spec:
  rules:
    - http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: api
                port: { number: 80 }
"#;
    let inventory = index_configuration_source(source(
        "deploy/application.yaml",
        ConfigSyntax::KubernetesYaml,
        text,
    ))
    .unwrap();

    assert_eq!(inventory.resources.len(), 4);
    assert_eq!(
        relation_count(&inventory, RelationKind::UsesConfiguration),
        1
    );
    assert_eq!(relation_count(&inventory, RelationKind::Selects), 1);
    assert_eq!(relation_count(&inventory, RelationKind::RoutesTo), 1);
    assert!(inventory.unresolved_relations.is_empty());
    assert!(inventory.resources.iter().all(|record| {
        record
            .body_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("blake3:"))
            && record.validate().is_ok()
    }));
}

#[test]
fn kubernetes_yaml_accepts_an_omitted_mapping_value_as_null() {
    let text = r#"apiVersion: v1
kind: Service
metadata:
  annotations:
    # Intentionally empty: YAML defines this omitted value as null.
  name: dns
spec:
  ports:
    - port: 53
"#;
    let inventory = index_configuration_source(source(
        "deploy/dns.yaml",
        ConfigSyntax::KubernetesYaml,
        text,
    ))
    .unwrap();

    assert_eq!(inventory.resources.len(), 1);
    assert_eq!(inventory.resources[0].display_name, "Service/default/dns");
}

#[test]
fn terraform_hcl_uses_declared_addresses_and_expression_dependencies() {
    let text = r#"resource "aws_vpc" "main" {
  cidr_block = "10.0.0.0/16"
}

resource "aws_security_group" "web" {
  vpc_id = aws_vpc.main.id
}

resource "aws_lb" "public" {
  security_groups = [aws_security_group.web.id]
  depends_on = [aws_vpc.main]
}
"#;
    let inventory =
        index_configuration_source(source("infra/main.tf", ConfigSyntax::TerraformHcl, text))
            .unwrap();

    assert_eq!(inventory.resources.len(), 3);
    assert_eq!(relation_count(&inventory, RelationKind::DependsOn), 3);
    let lb = inventory
        .resources
        .iter()
        .find(|resource| resource.display_name == "aws_lb.public")
        .unwrap();
    assert!(matches!(
        &lb.logical_address,
        Address::Resource { logical_name, .. } if logical_name == "aws_lb.public"
    ));
}

#[test]
fn cloudformation_yaml_resolves_explicit_and_intrinsic_dependencies() {
    let text = r#"AWSTemplateFormatVersion: '2010-09-09'
Resources:
  WebSecurityGroup:
    Type: AWS::EC2::SecurityGroup
  PublicLoadBalancer:
    Type: AWS::ElasticLoadBalancingV2::LoadBalancer
    DependsOn: WebSecurityGroup
    Properties:
      SecurityGroups:
        - Ref: WebSecurityGroup
"#;
    let inventory = index_configuration_source(source(
        "infra/template.yaml",
        ConfigSyntax::CloudFormationYaml,
        text,
    ))
    .unwrap();

    assert_eq!(inventory.resources.len(), 2);
    assert_eq!(relation_count(&inventory, RelationKind::DependsOn), 1);
}

#[test]
fn cloudformation_short_intrinsics_link_resources_but_parameter_refs_do_not() {
    let text = r#"Parameters:
  Stage:
    Type: String
Resources:
  Topic:
    Type: AWS::SNS::Topic
  Queue:
    Type: AWS::SQS::Queue
    Properties:
      QueueName: !Ref Stage
  Subscription:
    Type: AWS::SNS::Subscription
    Properties:
      TopicArn: !Ref Topic
      Endpoint: !GetAtt Queue.Arn
"#;
    let inventory = index_configuration_source(source(
        "infra/stack.yml",
        ConfigSyntax::CloudFormationYaml,
        text,
    ))
    .unwrap();

    assert_eq!(inventory.resources.len(), 3);
    assert_eq!(relation_count(&inventory, RelationKind::DependsOn), 2);
    assert!(inventory.unresolved_relations.is_empty());
}

#[test]
fn arm_v2_json_uses_symbolic_names_and_depends_on() {
    let text = r#"{
  "$schema": "https://schema.management.azure.com/schemas/2019-04-01/deploymentTemplate.json#",
  "languageVersion": "2.0",
  "resources": {
    "plan": {
      "type": "Microsoft.Web/serverfarms",
      "apiVersion": "2024-04-01",
      "name": "app-plan",
      "location": "eastus"
    },
    "site": {
      "type": "Microsoft.Web/sites",
      "apiVersion": "2024-04-01",
      "name": "api",
      "location": "eastus",
      "dependsOn": ["plan"]
    }
  }
}"#;
    let inventory = index_configuration_source(source(
        "infra/azuredeploy.json",
        ConfigSyntax::ArmJson,
        text,
    ))
    .unwrap();

    assert_eq!(inventory.resources.len(), 2);
    assert_eq!(relation_count(&inventory, RelationKind::DependsOn), 1);
}

#[test]
fn terraform_json_and_cloudformation_json_have_format_parity() {
    let terraform = r#"{
      "resource": {
        "aws_vpc": {"main": {"cidr_block": "10.0.0.0/16"}},
        "aws_subnet": {"app": {"vpc_id": "${aws_vpc.main.id}"}}
      }
    }"#;
    let tf_inventory = index_configuration_source(source(
        "infra/main.tf.json",
        ConfigSyntax::TerraformJson,
        terraform,
    ))
    .unwrap();
    assert_eq!(tf_inventory.resources.len(), 2);
    assert_eq!(relation_count(&tf_inventory, RelationKind::DependsOn), 1);

    let cloudformation = r#"{
      "Resources": {
        "Topic": {"Type": "AWS::SNS::Topic"},
        "Subscription": {
          "Type": "AWS::SNS::Subscription",
          "Properties": {"TopicArn": {"Ref": "Topic"}}
        }
      }
    }"#;
    let cfn_inventory = index_configuration_source(source(
        "infra/stack.json",
        ConfigSyntax::CloudFormationJson,
        cloudformation,
    ))
    .unwrap();
    assert_eq!(cfn_inventory.resources.len(), 2);
    assert_eq!(relation_count(&cfn_inventory, RelationKind::DependsOn), 1);
}

#[test]
fn arm_v1_resolves_literal_resource_id_dependency() {
    let text = r#"{
      "$schema": "https://schema.management.azure.com/schemas/2019-04-01/deploymentTemplate.json#",
      "resources": [
        {"type": "Microsoft.Web/serverfarms", "apiVersion": "2024-04-01", "name": "plan"},
        {
          "type": "Microsoft.Web/sites",
          "apiVersion": "2024-04-01",
          "name": "api",
          "dependsOn": ["[resourceId('Microsoft.Web/serverfarms', 'plan')]"]
        }
      ]
    }"#;
    let inventory = index_configuration_source(source(
        "infra/azuredeploy.json",
        ConfigSyntax::ArmJson,
        text,
    ))
    .unwrap();
    assert_eq!(inventory.resources.len(), 2);
    assert_eq!(relation_count(&inventory, RelationKind::DependsOn), 1);
    assert!(inventory.unresolved_relations.is_empty());
}

#[test]
fn yaml_aliases_are_rejected_instead_of_silently_miscomposed() {
    let text = "defaults: &defaults\n  namespace: prod\nmetadata: *defaults\n";
    let error = index_configuration_source(source(
        "deploy/alias.yaml",
        ConfigSyntax::KubernetesYaml,
        text,
    ))
    .unwrap_err();
    assert!(error.to_string().contains("aliases require composition"));
}

#[test]
fn malformed_input_fails_without_partial_inventory() {
    let error = index_configuration_source(source(
        "deploy/broken.yaml",
        ConfigSyntax::KubernetesYaml,
        "apiVersion: v1\nkind: [",
    ))
    .unwrap_err();
    assert!(error.to_string().contains("syntax"));
}

#[test]
fn repository_inventory_records_a_loud_parse_failure_and_continues() {
    let directory = tempfile::tempdir().unwrap();
    let path = "deploy/broken.yaml";
    let bytes = b"apiVersion: v1\nkind: Service\nmetadata: [";
    std::fs::create_dir_all(directory.path().join("deploy")).unwrap();
    std::fs::write(directory.path().join(path), bytes).unwrap();

    let inventory = index_repository_configurations(
        directory.path(),
        "github.com/example/service",
        REVISION,
        None,
        &[SnapshotFileDigest {
            path: path.into(),
            content_digest: blake3_hex(bytes),
        }],
    )
    .unwrap();

    assert!(inventory.resources.is_empty());
    assert_eq!(inventory.diagnostics.len(), 1);
    assert_eq!(inventory.diagnostics[0].code, "config.parse_failed");
    assert_eq!(inventory.diagnostics[0].path, path);
    assert!(inventory.diagnostics[0].message.contains("syntax error"));
}

#[test]
fn repository_inventory_records_a_loud_non_utf8_configuration_gap_and_continues() {
    let directory = tempfile::tempdir().unwrap();
    let path = "unittests/scans/binary-report.json";
    let bytes = [b'{', 0xff, b'}'];
    std::fs::create_dir_all(directory.path().join("unittests/scans")).unwrap();
    std::fs::write(directory.path().join(path), bytes).unwrap();

    let inventory = index_repository_configurations(
        directory.path(),
        "github.com/example/service",
        REVISION,
        None,
        &[SnapshotFileDigest {
            path: path.into(),
            content_digest: blake3_hex(&bytes),
        }],
    )
    .unwrap();

    assert!(inventory.resources.is_empty());
    assert_eq!(inventory.diagnostics.len(), 1);
    assert_eq!(inventory.diagnostics[0].code, "config.parse_failed");
    assert_eq!(inventory.diagnostics[0].path, path);
    assert!(inventory.diagnostics[0].message.contains("not UTF-8"));
}
