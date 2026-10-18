//! Conservative configuration-plane inventory over upstream Tree-sitter grammars.
//!
//! Parsers preserve source spans and extract only identity/link semantics stated
//! by the native configuration format. They do not evaluate templates, HCL
//! functions, Helm, ARM expressions, or CloudFormation conditions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use entmoot_contracts::{
    blake3_hex, relation_claim_with_id, relation_edge_with_id, resource_address,
    unresolved_relation_with_id, Address, EntityKind, LocationExtent, ProducerCapabilityKind,
    ProducerRef, Provenance, RelationClaim, RelationClaimValue, RelationEdge, RelationKind,
    ResolutionState, ResourceObservationKind, ResourceOrigin, ResourceRecord, Scope,
    SourceLocation, SourceRegion, SubjectId, TargetConstraint, UnresolvedReason,
    UnresolvedRelation,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tree_sitter::{Language, Node, Parser};

use crate::graph::{
    EdgeKind, NodeKind, SnapshotFileDigest, StructuralEdge, TopologyGraph, TopologyNode,
};
use crate::{ArtifactDiagnostic, DiagnosticSeverity};

const CONFIG_PRODUCER_ID: &str = "entmoot.config_inventory";
const CONFIG_PRODUCER_VERSION: &str = "logic.config_inventory.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSyntax {
    KubernetesYaml,
    TerraformHcl,
    TerraformJson,
    ArmJson,
    CloudFormationYaml,
    CloudFormationJson,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigSource<'a> {
    pub repository_id: &'a str,
    pub revision: &'a str,
    pub snapshot_digest: Option<&'a str>,
    pub path: &'a str,
    pub syntax: ConfigSyntax,
    pub text: &'a str,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigInventory {
    pub resources: Vec<ResourceRecord>,
    pub relation_edges: Vec<RelationEdge>,
    pub relation_claims: Vec<RelationClaim>,
    pub unresolved_relations: Vec<UnresolvedRelation>,
    pub diagnostics: Vec<ConfigDiagnostic>,
    relation_edge_ids: HashSet<String>,
    unresolved_relation_ids: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigDiagnostic {
    pub code: String,
    pub message: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Error)]
pub enum ConfigInventoryError {
    #[error("configuration source fields must be non-empty")]
    EmptySourceField,
    #[error("invalid configuration source path: {0}")]
    InvalidPath(String),
    #[error("failed to load {0} Tree-sitter grammar")]
    Grammar(&'static str),
    #[error("{syntax} syntax error in {path}")]
    Syntax { syntax: &'static str, path: String },
    #[error("unsupported {syntax} structure in {path}: {message}")]
    Unsupported {
        syntax: &'static str,
        path: String,
        message: String,
    },
    #[error("invalid resource record in {path}: {message}")]
    InvalidResource { path: String, message: String },
}

#[derive(Debug, Clone)]
enum ValueKind {
    Map(BTreeMap<String, ParsedValue>),
    Seq(Vec<ParsedValue>),
    Scalar(String),
    Null,
}

#[derive(Debug, Clone)]
struct ParsedValue {
    kind: ValueKind,
    start_byte: usize,
    end_byte: usize,
    start_line: u32,
}

impl ParsedValue {
    fn map(&self) -> Option<&BTreeMap<String, ParsedValue>> {
        match &self.kind {
            ValueKind::Map(value) => Some(value),
            _ => None,
        }
    }

    fn seq(&self) -> Option<&[ParsedValue]> {
        match &self.kind {
            ValueKind::Seq(value) => Some(value),
            _ => None,
        }
    }

    fn scalar(&self) -> Option<&str> {
        match &self.kind {
            ValueKind::Scalar(value) => Some(value),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&ParsedValue> {
        self.map()?.get(key)
    }
}

/// Index one source document. No partial result is returned on parse/identity failure.
pub fn index_configuration_source(
    input: ConfigSource<'_>,
) -> Result<ConfigInventory, ConfigInventoryError> {
    validate_input(&input)?;
    match input.syntax {
        ConfigSyntax::KubernetesYaml => index_kubernetes(input),
        ConfigSyntax::TerraformHcl => index_terraform_hcl(input),
        ConfigSyntax::TerraformJson => index_terraform_json(input),
        ConfigSyntax::ArmJson => index_arm(input),
        ConfigSyntax::CloudFormationYaml => index_cloudformation(input, true),
        ConfigSyntax::CloudFormationJson => index_cloudformation(input, false),
    }
}

fn validate_input(input: &ConfigSource<'_>) -> Result<(), ConfigInventoryError> {
    if input.repository_id.trim().is_empty()
        || input.revision.trim().is_empty()
        || input.path.trim().is_empty()
    {
        return Err(ConfigInventoryError::EmptySourceField);
    }
    let path = entmoot_contracts::normalize_repo_path(input.path)
        .map_err(|error| ConfigInventoryError::InvalidPath(error.to_string()))?;
    if path != input.path {
        return Err(ConfigInventoryError::InvalidPath(input.path.into()));
    }
    Ok(())
}

fn parser_for(language: Language, label: &'static str) -> Result<Parser, ConfigInventoryError> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|_| ConfigInventoryError::Grammar(label))?;
    Ok(parser)
}

fn parse_yaml_documents(
    input: &ConfigSource<'_>,
) -> Result<Vec<ParsedValue>, ConfigInventoryError> {
    let mut parser = parser_for(tree_sitter_yaml::LANGUAGE.into(), "yaml")?;
    let tree = parser
        .parse(input.text, None)
        .ok_or_else(|| syntax_error(input, "yaml"))?;
    if tree.root_node().has_error() {
        return Err(syntax_error(input, "yaml"));
    }
    let mut cursor = tree.root_node().walk();
    tree.root_node()
        .named_children(&mut cursor)
        .filter(|node| node.kind() == "document")
        .map(|node| yaml_value(node, input))
        .collect()
}

fn parse_json_document(input: &ConfigSource<'_>) -> Result<ParsedValue, ConfigInventoryError> {
    let mut parser = parser_for(tree_sitter_json::LANGUAGE.into(), "json")?;
    let tree = parser
        .parse(input.text, None)
        .ok_or_else(|| syntax_error(input, "json"))?;
    if tree.root_node().has_error() {
        return Err(syntax_error(input, "json"));
    }
    json_value(tree.root_node(), input)
}

fn syntax_error(input: &ConfigSource<'_>, syntax: &'static str) -> ConfigInventoryError {
    ConfigInventoryError::Syntax {
        syntax,
        path: input.path.into(),
    }
}

fn unsupported(
    input: &ConfigSource<'_>,
    syntax: &'static str,
    message: impl Into<String>,
) -> ConfigInventoryError {
    ConfigInventoryError::Unsupported {
        syntax,
        path: input.path.into(),
        message: message.into(),
    }
}

fn yaml_value<'tree>(
    mut node: Node<'tree>,
    input: &ConfigSource<'_>,
) -> Result<ParsedValue, ConfigInventoryError> {
    while let "document" | "block_node" | "flow_node" | "block_sequence_item" = node.kind() {
        let wrapper = node;
        if let Some(tag) = named_child_of_kind(wrapper, "tag") {
            let tag_name = node_text(tag, input)?.trim();
            let key = match tag_name {
                "!Ref" => Some("Ref"),
                "!GetAtt" => Some("Fn::GetAtt"),
                "!Sub" => Some("Fn::Sub"),
                _ => None,
            };
            let Some(key) = key else {
                return Err(unsupported(
                    input,
                    "yaml",
                    format!("application tag `{tag_name}`"),
                ));
            };
            let value_node = only_value_child(wrapper)
                .ok_or_else(|| unsupported(input, "yaml", "tagged value missing"))?;
            let mut map = BTreeMap::new();
            map.insert(key.to_string(), yaml_value(value_node, input)?);
            return Ok(value_with_span(wrapper, ValueKind::Map(map)));
        }
        node = only_value_child(wrapper)
            .ok_or_else(|| unsupported(input, "yaml", "empty wrapper node"))?;
    }
    let kind = match node.kind() {
        "block_mapping" | "flow_mapping" => {
            let mut values = BTreeMap::new();
            let mut cursor = node.walk();
            for pair in node.named_children(&mut cursor) {
                if !matches!(pair.kind(), "block_mapping_pair" | "flow_pair") {
                    continue;
                }
                let key_node = pair
                    .child_by_field_name("key")
                    .ok_or_else(|| unsupported(input, "yaml", "mapping key missing"))?;
                let key_value = yaml_value(key_node, input)?;
                let key = key_value
                    .scalar()
                    .ok_or_else(|| unsupported(input, "yaml", "non-scalar mapping key"))?
                    .to_string();
                let value = match pair.child_by_field_name("value") {
                    Some(value_node) => yaml_value(value_node, input)?,
                    None => value_with_span(pair, ValueKind::Null),
                };
                if values.insert(key.clone(), value).is_some() {
                    return Err(unsupported(
                        input,
                        "yaml",
                        format!("duplicate mapping key `{key}`"),
                    ));
                }
            }
            ValueKind::Map(values)
        }
        "block_sequence" | "flow_sequence" => {
            let mut values = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if matches!(child.kind(), "block_sequence_item" | "flow_node") {
                    values.push(yaml_value(child, input)?);
                }
            }
            ValueKind::Seq(values)
        }
        "alias" => {
            return Err(unsupported(
                input,
                "yaml",
                "aliases require composition and are not indexed by the structural parser",
            ))
        }
        "null_scalar" => ValueKind::Null,
        kind if kind.ends_with("scalar") => {
            ValueKind::Scalar(decode_yaml_scalar(node_text(node, input)?, node.kind())?)
        }
        other => return Err(unsupported(input, "yaml", format!("node kind `{other}`"))),
    };
    Ok(value_with_span(node, kind))
}

fn only_value_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let child = node
        .named_children(&mut cursor)
        .find(|child| !matches!(child.kind(), "anchor" | "tag" | "yaml_directive"));
    child
}

fn named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let child = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == kind);
    child
}

fn decode_yaml_scalar(raw: &str, kind: &str) -> Result<String, ConfigInventoryError> {
    let trimmed = raw.trim();
    if kind == "single_quote_scalar" {
        return Ok(trimmed
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(trimmed)
            .replace("''", "'"));
    }
    if kind == "double_quote_scalar" {
        return serde_json::from_str(trimmed).map_err(|error| ConfigInventoryError::Unsupported {
            syntax: "yaml",
            path: "<scalar>".into(),
            message: format!("unsupported quoted scalar: {error}"),
        });
    }
    Ok(trimmed.to_string())
}

fn json_value<'tree>(
    mut node: Node<'tree>,
    input: &ConfigSource<'_>,
) -> Result<ParsedValue, ConfigInventoryError> {
    if node.kind() == "document" {
        node =
            only_value_child(node).ok_or_else(|| unsupported(input, "json", "empty document"))?;
    }
    let kind = match node.kind() {
        "object" => {
            let mut values = BTreeMap::new();
            let mut cursor = node.walk();
            for pair in node
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "pair")
            {
                let key_node = pair
                    .child_by_field_name("key")
                    .ok_or_else(|| unsupported(input, "json", "object key missing"))?;
                let value_node = pair
                    .child_by_field_name("value")
                    .ok_or_else(|| unsupported(input, "json", "object value missing"))?;
                let key: String = serde_json::from_str(node_text(key_node, input)?)
                    .map_err(|e| unsupported(input, "json", format!("invalid object key: {e}")))?;
                if values
                    .insert(key.clone(), json_value(value_node, input)?)
                    .is_some()
                {
                    return Err(unsupported(
                        input,
                        "json",
                        format!("duplicate object key `{key}`"),
                    ));
                }
            }
            ValueKind::Map(values)
        }
        "array" => {
            let mut cursor = node.walk();
            ValueKind::Seq(
                node.named_children(&mut cursor)
                    .map(|child| json_value(child, input))
                    .collect::<Result<_, _>>()?,
            )
        }
        "string" => {
            let value: String = serde_json::from_str(node_text(node, input)?)
                .map_err(|e| unsupported(input, "json", format!("invalid string: {e}")))?;
            ValueKind::Scalar(value)
        }
        "number" | "true" | "false" => ValueKind::Scalar(node_text(node, input)?.into()),
        "null" => ValueKind::Null,
        other => return Err(unsupported(input, "json", format!("node kind `{other}`"))),
    };
    Ok(value_with_span(node, kind))
}

fn node_text<'a>(
    node: Node<'_>,
    input: &'a ConfigSource<'_>,
) -> Result<&'a str, ConfigInventoryError> {
    input
        .text
        .get(node.byte_range())
        .ok_or_else(|| unsupported(input, "tree_sitter", "node span outside source"))
}

fn value_with_span(node: Node<'_>, kind: ValueKind) -> ParsedValue {
    let start = node.start_position();
    ParsedValue {
        kind,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: u32::try_from(start.row)
            .unwrap_or(u32::MAX)
            .saturating_add(1),
    }
}

fn index_kubernetes(input: ConfigSource<'_>) -> Result<ConfigInventory, ConfigInventoryError> {
    let documents = parse_yaml_documents(&input)?;
    let mut inventory = ConfigInventory::default();
    let mut models = Vec::new();
    for (document_index, document) in documents.iter().enumerate() {
        let Some(map) = document.map() else {
            continue;
        };
        let Some(api_version) = scalar_at(map, "apiVersion") else {
            continue;
        };
        let Some(kind) = scalar_at(map, "kind") else {
            continue;
        };
        let metadata = map.get("metadata").and_then(ParsedValue::map);
        let Some(name) = metadata.and_then(|value| scalar_at(value, "name")) else {
            inventory.diagnostics.push(ConfigDiagnostic {
                code: "config.resource_identity_unresolved".into(),
                message: format!("{kind} has no literal metadata.name"),
                path: input.path.into(),
                line: Some(document.start_line),
            });
            continue;
        };
        let namespace = metadata
            .and_then(|value| scalar_at(value, "namespace"))
            .unwrap_or("default");
        let address = resource_address(
            "kubernetes",
            format!("{}/kubernetes/namespace/{namespace}", input.repository_id),
            format!("{api_version}/{kind}"),
            name,
        )
        .map_err(|error| invalid_resource(&input, error))?;
        let mut record = build_source_record(
            &input,
            u32::try_from(document_index).map_err(|e| invalid_resource(&input, e))?,
            document,
            kubernetes_entity_kind(kind),
            address,
            format!("{kind}/{namespace}/{name}"),
            format!("{api_version}/{kind}"),
        )?;
        if let Some(labels) = metadata
            .and_then(|value| value.get("labels"))
            .and_then(map_scalars)
        {
            record.identity_labels = labels;
        }
        let selectable_labels = kubernetes_selectable_labels(document);
        if is_workload_kind(kind) || kind == "Pod" {
            record.identity_labels.extend(selectable_labels.clone());
        }
        models.push(KubernetesModel {
            document,
            record: record.clone(),
            namespace: namespace.to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
        });
        inventory.resources.push(record);
    }

    let by_name: HashMap<_, _> = models
        .iter()
        .map(|model| {
            (
                (
                    model.namespace.as_str(),
                    model.kind.as_str(),
                    model.name.as_str(),
                ),
                model,
            )
        })
        .collect();
    let resource_index = ResourceLookupIndex::build(&inventory.resources);
    let models_by_address: HashMap<_, _> = models
        .iter()
        .map(|model| {
            (
                entmoot_contracts::address_identity_key(&model.record.logical_address),
                model,
            )
        })
        .collect();
    for model in &models {
        match model.kind.as_str() {
            "Service" => {
                if let Some(selector) = model
                    .document
                    .get("spec")
                    .and_then(|value| value.get("selector"))
                    .and_then(map_scalars)
                {
                    let constraint = selector_constraint(&model.namespace, &selector);
                    let targets: Vec<_> = resource_index
                        .matching(&model.record.logical_address, &constraint)
                        .iter()
                        .filter_map(|address| {
                            models_by_address
                                .get(&entmoot_contracts::address_identity_key(address))
                                .copied()
                        })
                        .collect();
                    for target in &targets {
                        add_relation(
                            &mut inventory,
                            &input,
                            &model.record,
                            &target.record,
                            RelationKind::Selects,
                        );
                    }
                    if targets.is_empty() {
                        add_unresolved(
                            &mut inventory,
                            &input,
                            &model.record,
                            RelationKind::Selects,
                            constraint,
                        );
                    }
                }
            }
            "Ingress" => {
                let names = collect_values_at_key(model.document, "service")
                    .into_iter()
                    .filter_map(|service| service.get("name").and_then(ParsedValue::scalar));
                for name in names {
                    add_named_kubernetes_relation(
                        &mut inventory,
                        &input,
                        model,
                        &by_name,
                        "Service",
                        name,
                        RelationKind::RoutesTo,
                    );
                }
            }
            kind if is_workload_kind(kind) || kind == "Pod" || kind == "Job" => {
                for (target_kind, name) in kubernetes_configuration_references(model.document) {
                    add_named_kubernetes_relation(
                        &mut inventory,
                        &input,
                        model,
                        &by_name,
                        target_kind,
                        &name,
                        RelationKind::UsesConfiguration,
                    );
                }
            }
            "HorizontalPodAutoscaler" => {
                if let Some(target) = model
                    .document
                    .get("spec")
                    .and_then(|v| v.get("scaleTargetRef"))
                {
                    if let (Some(kind), Some(name)) = (
                        target.get("kind").and_then(ParsedValue::scalar),
                        target.get("name").and_then(ParsedValue::scalar),
                    ) {
                        add_named_kubernetes_relation(
                            &mut inventory,
                            &input,
                            model,
                            &by_name,
                            kind,
                            name,
                            RelationKind::DependsOn,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    finalize_inventory(&mut inventory);
    Ok(inventory)
}

struct KubernetesModel<'a> {
    document: &'a ParsedValue,
    record: ResourceRecord,
    namespace: String,
    kind: String,
    name: String,
}

fn kubernetes_entity_kind(kind: &str) -> EntityKind {
    match kind {
        "Deployment" => EntityKind::Deployment,
        "StatefulSet" | "DaemonSet" | "Job" | "CronJob" | "Pod" => EntityKind::Workload,
        "Service" | "Ingress" => EntityKind::NetworkEndpoint,
        "ConfigMap" | "Secret" => EntityKind::ConfigurationResource,
        _ => EntityKind::KubernetesResource,
    }
}

fn is_workload_kind(kind: &str) -> bool {
    matches!(kind, "Deployment" | "StatefulSet" | "DaemonSet")
}

fn kubernetes_selectable_labels(document: &ParsedValue) -> BTreeMap<String, String> {
    let from_template = document
        .get("spec")
        .and_then(|value| value.get("template"))
        .and_then(|value| value.get("metadata"))
        .and_then(|value| value.get("labels"))
        .and_then(map_scalars);
    from_template
        .or_else(|| {
            document
                .get("metadata")
                .and_then(|value| value.get("labels"))
                .and_then(map_scalars)
        })
        .unwrap_or_default()
}

fn selector_constraint(namespace: &str, selector: &BTreeMap<String, String>) -> String {
    let terms = selector
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("selector:{namespace}:{terms}")
}

fn kubernetes_configuration_references(document: &ParsedValue) -> BTreeSet<(&'static str, String)> {
    let mut output = BTreeSet::new();
    for value in collect_values_at_key(document, "configMapRef")
        .into_iter()
        .chain(collect_values_at_key(document, "configMapKeyRef"))
    {
        if let Some(name) = value.get("name").and_then(ParsedValue::scalar) {
            output.insert(("ConfigMap", name.to_string()));
        }
    }
    for value in collect_values_at_key(document, "secretRef")
        .into_iter()
        .chain(collect_values_at_key(document, "secretKeyRef"))
    {
        if let Some(name) = value.get("name").and_then(ParsedValue::scalar) {
            output.insert(("Secret", name.to_string()));
        }
    }
    for value in collect_values_at_key(document, "configMap") {
        if let Some(name) = value.get("name").and_then(ParsedValue::scalar) {
            output.insert(("ConfigMap", name.to_string()));
        }
    }
    for value in collect_values_at_key(document, "secret") {
        if let Some(name) = value.get("secretName").and_then(ParsedValue::scalar) {
            output.insert(("Secret", name.to_string()));
        }
    }
    output
}

fn collect_values_at_key<'a>(value: &'a ParsedValue, key: &str) -> Vec<&'a ParsedValue> {
    let mut output = Vec::new();
    collect_values_at_key_into(value, key, &mut output);
    output
}

fn collect_values_at_key_into<'a>(
    value: &'a ParsedValue,
    key: &str,
    output: &mut Vec<&'a ParsedValue>,
) {
    match &value.kind {
        ValueKind::Map(map) => {
            for (name, child) in map {
                if name == key {
                    output.push(child);
                }
                collect_values_at_key_into(child, key, output);
            }
        }
        ValueKind::Seq(values) => {
            for child in values {
                collect_values_at_key_into(child, key, output);
            }
        }
        ValueKind::Scalar(_) | ValueKind::Null => {}
    }
}

fn add_named_kubernetes_relation(
    inventory: &mut ConfigInventory,
    input: &ConfigSource<'_>,
    source: &KubernetesModel<'_>,
    by_name: &HashMap<(&str, &str, &str), &KubernetesModel<'_>>,
    target_kind: &str,
    target_name: &str,
    predicate: RelationKind,
) {
    if let Some(target) = by_name.get(&(source.namespace.as_str(), target_kind, target_name)) {
        add_relation(inventory, input, &source.record, &target.record, predicate);
    } else {
        add_unresolved(
            inventory,
            input,
            &source.record,
            predicate,
            format!("{target_kind}/{}/{target_name}", source.namespace),
        );
    }
}

fn index_terraform_hcl(input: ConfigSource<'_>) -> Result<ConfigInventory, ConfigInventoryError> {
    let mut parser = parser_for(tree_sitter_hcl::LANGUAGE.into(), "hcl")?;
    let tree = parser
        .parse(input.text, None)
        .ok_or_else(|| syntax_error(&input, "hcl"))?;
    if tree.root_node().has_error() {
        return Err(syntax_error(&input, "hcl"));
    }
    let mut blocks = Vec::new();
    collect_nodes(tree.root_node(), "block", &mut blocks);
    let mut inventory = ConfigInventory::default();
    let mut models = Vec::new();
    for block in blocks {
        let mut cursor = block.walk();
        let children: Vec<_> = block.named_children(&mut cursor).collect();
        let Some(block_type) = children
            .iter()
            .find(|child| child.kind() == "identifier")
            .and_then(|node| node_text(*node, &input).ok())
        else {
            continue;
        };
        if block_type != "resource" {
            continue;
        }
        let labels: Vec<String> = children
            .iter()
            .filter(|child| child.kind() == "string_lit")
            .map(|node| hcl_string(*node, &input))
            .collect::<Result<_, _>>()?;
        if labels.len() != 2 {
            return Err(unsupported(
                &input,
                "terraform",
                "resource block must have type and name labels",
            ));
        }
        let logical_name = format!("{}.{}", labels[0], labels[1]);
        let namespace = terraform_namespace(input.repository_id, input.path);
        let address = resource_address("terraform", namespace, &labels[0], &logical_name)
            .map_err(|error| invalid_resource(&input, error))?;
        let value = value_with_span(block, ValueKind::Null);
        let record = build_source_record(
            &input,
            0,
            &value,
            EntityKind::InfrastructureResource,
            address,
            logical_name.clone(),
            labels[0].clone(),
        )?;
        let references = terraform_references(block, &input)?;
        models.push((record.clone(), references));
        inventory.resources.push(record);
    }
    let by_name: HashMap<_, _> = models
        .iter()
        .map(|(record, _)| (record.display_name.as_str(), record))
        .collect();
    for (record, references) in &models {
        for target_name in references {
            if target_name == &record.display_name {
                continue;
            }
            if let Some(target) = by_name.get(target_name.as_str()) {
                add_relation(
                    &mut inventory,
                    &input,
                    record,
                    target,
                    RelationKind::DependsOn,
                );
            } else {
                add_unresolved(
                    &mut inventory,
                    &input,
                    record,
                    RelationKind::DependsOn,
                    target_name.clone(),
                );
            }
        }
    }
    finalize_inventory(&mut inventory);
    Ok(inventory)
}

fn hcl_string(node: Node<'_>, input: &ConfigSource<'_>) -> Result<String, ConfigInventoryError> {
    let raw = node_text(node, input)?;
    serde_json::from_str(raw)
        .map_err(|error| unsupported(input, "terraform", format!("non-literal label: {error}")))
}

fn terraform_references(
    node: Node<'_>,
    input: &ConfigSource<'_>,
) -> Result<BTreeSet<String>, ConfigInventoryError> {
    let mut expressions = Vec::new();
    collect_nodes(node, "expression", &mut expressions);
    let mut references = BTreeSet::new();
    for expression in expressions {
        let mut cursor = expression.walk();
        let children: Vec<_> = expression.named_children(&mut cursor).collect();
        if children
            .first()
            .is_some_and(|child| child.kind() == "variable_expr")
        {
            let mut components = Vec::new();
            for child in children {
                if matches!(child.kind(), "variable_expr" | "get_attr") {
                    let mut identifiers = Vec::new();
                    collect_nodes(child, "identifier", &mut identifiers);
                    for identifier in identifiers {
                        components.push(node_text(identifier, input)?.to_string());
                    }
                }
            }
            if components.len() >= 2
                && !matches!(
                    components[0].as_str(),
                    "var" | "local" | "module" | "data" | "path" | "terraform" | "count" | "each"
                )
            {
                references.insert(format!("{}.{}", components[0], components[1]));
            }
        }
    }
    Ok(references)
}

fn collect_nodes<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_nodes(child, kind, output);
    }
}

fn terraform_namespace(repository_id: &str, path: &str) -> String {
    let module = path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("root");
    format!("{repository_id}/terraform/module/{module}")
}

fn index_terraform_json(input: ConfigSource<'_>) -> Result<ConfigInventory, ConfigInventoryError> {
    let root = parse_json_document(&input)?;
    let mut inventory = ConfigInventory::default();
    let mut models = Vec::new();
    let Some(resource_types) = root.get("resource").and_then(ParsedValue::map) else {
        return Ok(inventory);
    };
    for (resource_type, names) in resource_types {
        let Some(names) = names.map() else { continue };
        for (name, body) in names {
            let logical_name = format!("{resource_type}.{name}");
            let address = resource_address(
                "terraform",
                terraform_namespace(input.repository_id, input.path),
                resource_type,
                &logical_name,
            )
            .map_err(|error| invalid_resource(&input, error))?;
            let record = build_source_record(
                &input,
                0,
                body,
                EntityKind::InfrastructureResource,
                address,
                logical_name.clone(),
                resource_type.clone(),
            )?;
            let references = terraform_json_references(body);
            models.push((record.clone(), references));
            inventory.resources.push(record);
        }
    }
    link_named_dependencies(&mut inventory, &input, &models);
    finalize_inventory(&mut inventory);
    Ok(inventory)
}

fn terraform_json_references(value: &ParsedValue) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    collect_scalars(value, &mut |text| {
        for candidate in text
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'))
            .filter(|part| part.matches('.').count() >= 1)
        {
            let mut components = candidate.split('.');
            if let (Some(kind), Some(name)) = (components.next(), components.next()) {
                if !matches!(kind, "var" | "local" | "module" | "data") {
                    output.insert(format!("{kind}.{name}"));
                }
            }
        }
    });
    output
}

fn link_named_dependencies(
    inventory: &mut ConfigInventory,
    input: &ConfigSource<'_>,
    models: &[(ResourceRecord, BTreeSet<String>)],
) {
    let by_name: HashMap<_, _> = models
        .iter()
        .map(|(record, _)| (record.display_name.as_str(), record))
        .collect();
    for (record, references) in models {
        for reference in references {
            if let Some(target) = by_name.get(reference.as_str()) {
                add_relation(inventory, input, record, target, RelationKind::DependsOn);
            } else {
                add_unresolved(
                    inventory,
                    input,
                    record,
                    RelationKind::DependsOn,
                    reference.clone(),
                );
            }
        }
    }
}

fn index_cloudformation(
    input: ConfigSource<'_>,
    yaml: bool,
) -> Result<ConfigInventory, ConfigInventoryError> {
    let roots = if yaml {
        parse_yaml_documents(&input)?
    } else {
        vec![parse_json_document(&input)?]
    };
    let mut inventory = ConfigInventory::default();
    let mut models = Vec::new();
    for (document_index, root) in roots.iter().enumerate() {
        let Some(resources) = root.get("Resources").and_then(ParsedValue::map) else {
            continue;
        };
        for (logical_name, body) in resources {
            let Some(native_type) = body.get("Type").and_then(ParsedValue::scalar) else {
                inventory.diagnostics.push(ConfigDiagnostic {
                    code: "config.resource_identity_unresolved".into(),
                    message: format!(
                        "CloudFormation resource `{logical_name}` has no literal Type"
                    ),
                    path: input.path.into(),
                    line: Some(body.start_line),
                });
                continue;
            };
            let address = resource_address(
                "cloudformation",
                format!(
                    "{}/cloudformation/template/{}",
                    input.repository_id, input.path
                ),
                native_type,
                logical_name,
            )
            .map_err(|error| invalid_resource(&input, error))?;
            let record = build_source_record(
                &input,
                u32::try_from(document_index).map_err(|e| invalid_resource(&input, e))?,
                body,
                EntityKind::InfrastructureResource,
                address,
                logical_name.clone(),
                native_type.into(),
            )?;
            let references = cloudformation_references(body);
            models.push((record.clone(), references));
            inventory.resources.push(record);
        }
    }
    let by_name: HashMap<_, _> = models
        .iter()
        .map(|(record, _)| (record.display_name.as_str(), record))
        .collect();
    for (record, references) in &models {
        for reference in &references.names {
            if let Some(target) = by_name.get(reference.as_str()) {
                add_relation(
                    &mut inventory,
                    &input,
                    record,
                    target,
                    RelationKind::DependsOn,
                );
            } else if references.required_resources.contains(reference) {
                add_unresolved(
                    &mut inventory,
                    &input,
                    record,
                    RelationKind::DependsOn,
                    reference.clone(),
                );
            }
        }
    }
    finalize_inventory(&mut inventory);
    Ok(inventory)
}

#[derive(Default)]
struct CloudFormationReferences {
    names: BTreeSet<String>,
    required_resources: BTreeSet<String>,
}

fn cloudformation_references(body: &ParsedValue) -> CloudFormationReferences {
    let mut output = CloudFormationReferences::default();
    if let Some(depends_on) = body.get("DependsOn") {
        match &depends_on.kind {
            ValueKind::Scalar(value) => {
                output.names.insert(value.clone());
                output.required_resources.insert(value.clone());
            }
            ValueKind::Seq(values) => {
                for value in values.iter().filter_map(ParsedValue::scalar) {
                    output.names.insert(value.to_string());
                    output.required_resources.insert(value.to_string());
                }
            }
            _ => {}
        }
    }
    for reference in collect_values_at_key(body, "Ref") {
        if let Some(value) = reference.scalar() {
            output.names.insert(value.to_string());
        }
    }
    for reference in collect_values_at_key(body, "Fn::GetAtt") {
        match &reference.kind {
            ValueKind::Scalar(value) => {
                if let Some((logical_name, _)) = value.split_once('.') {
                    output.names.insert(logical_name.to_string());
                }
            }
            ValueKind::Seq(values) => {
                if let Some(logical_name) = values.first().and_then(ParsedValue::scalar) {
                    output.names.insert(logical_name.to_string());
                }
            }
            _ => {}
        }
    }
    for substitution in collect_values_at_key(body, "Fn::Sub") {
        if let Some(template) = substitution.scalar().or_else(|| {
            substitution
                .seq()
                .and_then(|values| values.first())
                .and_then(ParsedValue::scalar)
        }) {
            output.names.extend(cloudformation_sub_references(template));
        }
    }
    output
}

fn cloudformation_sub_references(template: &str) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    let mut remaining = template;
    while let Some(start) = remaining.find("${") {
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        let variable = &remaining[..end];
        if !variable.starts_with('!') {
            let logical_name = variable.split('.').next().unwrap_or(variable);
            if !logical_name.is_empty() && !logical_name.contains(':') {
                output.insert(logical_name.to_string());
            }
        }
        remaining = &remaining[end + 1..];
    }
    output
}

fn index_arm(input: ConfigSource<'_>) -> Result<ConfigInventory, ConfigInventoryError> {
    let root = parse_json_document(&input)?;
    let mut inventory = ConfigInventory::default();
    let mut models = Vec::new();
    let language_v2 = root
        .get("languageVersion")
        .and_then(ParsedValue::scalar)
        .is_some_and(|version| version.starts_with("2."));
    let Some(resources) = root.get("resources") else {
        return Ok(inventory);
    };
    match &resources.kind {
        ValueKind::Map(values) if language_v2 => {
            for (symbolic_name, body) in values {
                if let Some(model) = arm_model(&input, symbolic_name, body, 0)? {
                    inventory.resources.push(model.0.clone());
                    models.push(model);
                }
            }
        }
        ValueKind::Seq(values) => {
            for (index, body) in values.iter().enumerate() {
                let logical_name = body
                    .get("name")
                    .and_then(ParsedValue::scalar)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("resource[{index}]"));
                if let Some(model) = arm_model(
                    &input,
                    &logical_name,
                    body,
                    u32::try_from(index).map_err(|e| invalid_resource(&input, e))?,
                )? {
                    inventory.resources.push(model.0.clone());
                    models.push(model);
                }
            }
        }
        _ => {
            return Err(unsupported(
                &input,
                "arm",
                "resources must be an array or a languageVersion 2 object",
            ))
        }
    }
    link_named_dependencies(&mut inventory, &input, &models);
    finalize_inventory(&mut inventory);
    Ok(inventory)
}

fn arm_model(
    input: &ConfigSource<'_>,
    logical_name: &str,
    body: &ParsedValue,
    document_index: u32,
) -> Result<Option<(ResourceRecord, BTreeSet<String>)>, ConfigInventoryError> {
    let Some(native_type) = body.get("type").and_then(ParsedValue::scalar) else {
        return Ok(None);
    };
    let address = resource_address(
        "arm",
        format!("{}/arm/template/{}", input.repository_id, input.path),
        native_type,
        logical_name,
    )
    .map_err(|error| invalid_resource(input, error))?;
    let record = build_source_record(
        input,
        document_index,
        body,
        EntityKind::InfrastructureResource,
        address,
        logical_name.into(),
        native_type.into(),
    )?;
    let mut references = BTreeSet::new();
    if let Some(depends_on) = body.get("dependsOn").and_then(ParsedValue::seq) {
        references.extend(
            depends_on
                .iter()
                .filter_map(ParsedValue::scalar)
                .map(|value| arm_dependency_target(value).unwrap_or_else(|| value.to_string())),
        );
    }
    Ok(Some((record, references)))
}

fn arm_dependency_target(value: &str) -> Option<String> {
    let expression = value.strip_prefix('[')?.strip_suffix(']')?;
    let arguments = expression.strip_prefix("resourceId(")?.strip_suffix(')')?;
    let mut quoted = Vec::new();
    let mut remaining = arguments;
    while let Some(start) = remaining.find('\'') {
        remaining = &remaining[start + 1..];
        let end = remaining.find('\'')?;
        quoted.push(&remaining[..end]);
        remaining = &remaining[end + 1..];
    }
    quoted.last().map(|value| (*value).to_string())
}

fn scalar_at<'a>(map: &'a BTreeMap<String, ParsedValue>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(ParsedValue::scalar)
}

fn map_scalars(value: &ParsedValue) -> Option<BTreeMap<String, String>> {
    Some(
        value
            .map()?
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.scalar()?.to_string())))
            .collect(),
    )
}

fn collect_scalars(value: &ParsedValue, visitor: &mut impl FnMut(&str)) {
    match &value.kind {
        ValueKind::Map(values) => {
            for child in values.values() {
                collect_scalars(child, visitor);
            }
        }
        ValueKind::Seq(values) => {
            for child in values {
                collect_scalars(child, visitor);
            }
        }
        ValueKind::Scalar(value) => visitor(value),
        ValueKind::Null => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn build_source_record(
    input: &ConfigSource<'_>,
    document_index: u32,
    body: &ParsedValue,
    entity_kind: EntityKind,
    address: Address,
    display_name: String,
    native_type: String,
) -> Result<ResourceRecord, ConfigInventoryError> {
    ResourceRecord::build(
        entity_kind,
        address,
        display_name,
        native_type,
        ResourceObservationKind::Declared,
        ResourceOrigin::Source {
            repository_id: input.repository_id.into(),
            revision: input.revision.into(),
            snapshot_digest: input.snapshot_digest.map(str::to_string),
            document_index,
            location: SourceLocation {
                path: input.path.into(),
                extent: LocationExtent::Region {
                    region: SourceRegion::Bytes {
                        span: entmoot_contracts::SiteSpan {
                            start_byte: body.start_byte,
                            end_byte: body.end_byte,
                        },
                    },
                },
            },
        },
        Some(blake3_hex(
            input
                .text
                .as_bytes()
                .get(body.start_byte..body.end_byte)
                .unwrap_or_default(),
        )),
    )
    .map_err(|error| invalid_resource(input, error))
}

fn invalid_resource(
    input: &ConfigSource<'_>,
    error: impl std::fmt::Display,
) -> ConfigInventoryError {
    ConfigInventoryError::InvalidResource {
        path: input.path.into(),
        message: error.to_string(),
    }
}

fn relation_scope(input: &ConfigSource<'_>) -> Scope {
    Scope {
        domain: input.repository_id.into(),
        snapshot_revision: Some(input.revision.into()),
        snapshot_digest: input.snapshot_digest.map(str::to_string),
        occurrence_id: None,
        context_id: Some("declared_configuration".into()),
    }
}

fn config_producer() -> ProducerRef {
    ProducerRef {
        id: CONFIG_PRODUCER_ID.into(),
        name: "Entmoot configuration inventory".into(),
        version: Some(CONFIG_PRODUCER_VERSION.into()),
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![ProducerCapabilityKind::Resolver],
    }
}

fn add_relation(
    inventory: &mut ConfigInventory,
    input: &ConfigSource<'_>,
    source: &ResourceRecord,
    target: &ResourceRecord,
    predicate: RelationKind,
) {
    let scope = relation_scope(input);
    let edge = relation_edge_with_id(
        "configuration",
        source.logical_address.clone(),
        predicate,
        target.logical_address.clone(),
        scope.clone(),
        Some(source.occurrence_id.clone()),
    );
    if !inventory
        .relation_edge_ids
        .insert(edge.id.as_str().to_string())
    {
        return;
    }
    let claim = relation_claim_with_id(
        SubjectId::RelationEdge {
            edge_id: edge.id.clone(),
        },
        "entmoot.relation_exists",
        RelationClaimValue::ResolutionState {
            state: ResolutionState::Structural,
        },
        config_producer(),
        scope,
        Vec::new(),
        Some(Provenance {
            snapshot_revision: Some(input.revision.into()),
            snapshot_digest: input.snapshot_digest.map(str::to_string),
            artifacts: Vec::new(),
            tool_digest: None,
            produced_at: None,
        }),
    );
    inventory.relation_edges.push(edge);
    inventory.relation_claims.push(claim);
}

fn add_unresolved(
    inventory: &mut ConfigInventory,
    input: &ConfigSource<'_>,
    source: &ResourceRecord,
    predicate: RelationKind,
    target: String,
) {
    let relation = unresolved_relation_with_id(
        source.logical_address.clone(),
        predicate,
        TargetConstraint::Syntactic { callee: target },
        UnresolvedReason::TargetNotInInventory,
        config_producer(),
        relation_scope(input),
        Some(source.occurrence_id.clone()),
        Vec::new(),
        Some(Provenance {
            snapshot_revision: Some(input.revision.into()),
            snapshot_digest: input.snapshot_digest.map(str::to_string),
            artifacts: Vec::new(),
            tool_digest: None,
            produced_at: None,
        }),
        Some(ProducerCapabilityKind::Resolver),
    );
    if inventory
        .unresolved_relation_ids
        .insert(relation.id.as_str().to_string())
    {
        inventory.unresolved_relations.push(relation);
    }
}

fn finalize_inventory(inventory: &mut ConfigInventory) {
    inventory
        .resources
        .sort_by(|left, right| left.occurrence_id.cmp(&right.occurrence_id));
    inventory
        .relation_edges
        .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    inventory
        .relation_edges
        .dedup_by(|left, right| left.id == right.id);
    inventory
        .relation_claims
        .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    inventory
        .relation_claims
        .dedup_by(|left, right| left.id == right.id);
    inventory
        .unresolved_relations
        .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    inventory
        .unresolved_relations
        .dedup_by(|left, right| left.id == right.id);
    inventory.diagnostics.sort_by(|left, right| {
        (&left.path, left.line, &left.code).cmp(&(&right.path, right.line, &right.code))
    });
    inventory.relation_edge_ids = inventory
        .relation_edges
        .iter()
        .map(|edge| edge.id.as_str().to_string())
        .collect();
    inventory.unresolved_relation_ids = inventory
        .unresolved_relations
        .iter()
        .map(|relation| relation.id.as_str().to_string())
        .collect();
}

/// Detect only supported configuration dialects. Extension alone never turns an
/// arbitrary YAML/JSON document into infrastructure inventory.
pub fn detect_configuration_syntax(
    path: &str,
    text: &str,
) -> Result<Option<ConfigSyntax>, ConfigInventoryError> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tf.json") {
        return Ok(Some(ConfigSyntax::TerraformJson));
    }
    if lower.ends_with(".tf") {
        return Ok(Some(ConfigSyntax::TerraformHcl));
    }
    if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        if text.contains("{{") || text.contains("{%") {
            return Ok(None);
        }
        let looks_kubernetes =
            text.contains("apiVersion:") && text.contains("kind:") && text.contains("metadata:");
        if looks_kubernetes {
            return Ok(Some(ConfigSyntax::KubernetesYaml));
        }
        if text.contains("AWSTemplateFormatVersion:")
            || (text.contains("Resources:") && text.contains("Type:"))
        {
            return Ok(Some(ConfigSyntax::CloudFormationYaml));
        }
        return Ok(None);
    }
    if lower.ends_with(".json") {
        if text.contains("deploymentTemplate.json#")
            || (text.contains("\"contentVersion\"") && text.contains("\"resources\""))
        {
            return Ok(Some(ConfigSyntax::ArmJson));
        }
        if text.contains("\"AWSTemplateFormatVersion\"")
            || (text.contains("\"Resources\"") && text.contains("\"Type\""))
        {
            return Ok(Some(ConfigSyntax::CloudFormationJson));
        }
    }
    Ok(None)
}

/// Parse all supported configuration files captured in a graph snapshot.
/// Candidate files are independent and parsed in parallel; results are stably
/// sorted before graph materialization.
pub fn index_repository_configurations(
    root: &std::path::Path,
    repository_id: &str,
    revision: &str,
    snapshot_digest: Option<&str>,
    snapshot_files: &[SnapshotFileDigest],
) -> Result<ConfigInventory, ConfigInventoryError> {
    let outcomes: Vec<Result<ConfigInventory, ConfigInventoryError>> = snapshot_files
        .par_iter()
        .filter(|file| configuration_candidate_path(&file.path))
        .map(|file| {
            let bytes = std::fs::read(root.join(&file.path)).map_err(|error| {
                ConfigInventoryError::InvalidPath(format!("{}: {error}", file.path))
            })?;
            if blake3_hex(&bytes) != file.content_digest {
                return Err(ConfigInventoryError::InvalidPath(format!(
                    "{} changed after snapshot capture",
                    file.path
                )));
            }
            let text = match std::str::from_utf8(&bytes) {
                Ok(text) => text,
                Err(error) => {
                    let mut skipped = ConfigInventory::default();
                    skipped.diagnostics.push(ConfigDiagnostic {
                        code: "config.parse_failed".into(),
                        message: format!(
                            "configuration inventory skipped for this file: source is not UTF-8: {error}"
                        ),
                        path: file.path.clone(),
                        line: None,
                    });
                    return Ok(skipped);
                }
            };
            let Some(syntax) = detect_configuration_syntax(&file.path, text)? else {
                let mut skipped = ConfigInventory::default();
                if (file.path.ends_with(".yaml") || file.path.ends_with(".yml"))
                    && (text.contains("{{") || text.contains("{%"))
                {
                    skipped.diagnostics.push(ConfigDiagnostic {
                        code: "config.render_required".into(),
                        message: "templated YAML requires an immutable rendered manifest; source template was not interpreted".into(),
                        path: file.path.clone(),
                        line: None,
                    });
                }
                return Ok(skipped);
            };
            let source = ConfigSource {
                repository_id,
                revision,
                snapshot_digest,
                path: &file.path,
                syntax,
                text,
            };
            match index_configuration_source(source) {
                Ok(inventory) => Ok(inventory),
                Err(error @ ConfigInventoryError::Syntax { .. })
                | Err(error @ ConfigInventoryError::Unsupported { .. }) => {
                    let mut skipped = ConfigInventory::default();
                    skipped.diagnostics.push(ConfigDiagnostic {
                        code: "config.parse_failed".into(),
                        message: format!(
                            "configuration inventory skipped for this file: {error}"
                        ),
                        path: file.path.clone(),
                        line: None,
                    });
                    Ok(skipped)
                }
                Err(error) => Err(error),
            }
        })
        .collect();

    let mut combined = ConfigInventory::default();
    for outcome in outcomes {
        let inventory = outcome?;
        combined.resources.extend(inventory.resources);
        combined.relation_edges.extend(inventory.relation_edges);
        combined.relation_claims.extend(inventory.relation_claims);
        combined
            .unresolved_relations
            .extend(inventory.unresolved_relations);
        combined.diagnostics.extend(inventory.diagnostics);
    }
    finalize_inventory(&mut combined);
    resolve_cross_file_relations(&mut combined);
    finalize_inventory(&mut combined);
    Ok(combined)
}

fn configuration_candidate_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".tf")
        || lower.ends_with(".tf.json")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".json")
}

fn resolve_cross_file_relations(inventory: &mut ConfigInventory) {
    let resource_index = ResourceLookupIndex::build(&inventory.resources);
    let mut remaining = Vec::new();
    for unresolved in std::mem::take(&mut inventory.unresolved_relations) {
        let TargetConstraint::Syntactic { callee } = &unresolved.target_constraint else {
            remaining.push(unresolved);
            continue;
        };
        let targets = resource_index.matching(&unresolved.source, callee);
        if targets.is_empty() {
            remaining.push(unresolved);
            continue;
        }
        for target in targets {
            let edge = relation_edge_with_id(
                "configuration",
                unresolved.source.clone(),
                unresolved.predicate,
                target,
                unresolved.scope.clone(),
                unresolved.occurrence_id.clone(),
            );
            if !inventory
                .relation_edge_ids
                .insert(edge.id.as_str().to_string())
            {
                continue;
            }
            let claim = relation_claim_with_id(
                SubjectId::RelationEdge {
                    edge_id: edge.id.clone(),
                },
                "entmoot.relation_exists",
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Structural,
                },
                unresolved.producer.clone(),
                unresolved.scope.clone(),
                unresolved.evidence.clone(),
                unresolved.provenance.clone(),
            );
            inventory.relation_edges.push(edge);
            inventory.relation_claims.push(claim);
        }
    }
    inventory.unresolved_relations = remaining;
}

#[derive(Default)]
struct ResourceLookupIndex {
    exact: HashMap<(String, String, String), Vec<String>>,
    kubernetes_exact: HashMap<(String, String, String), Vec<String>>,
    kubernetes_labels: HashMap<(String, String, String), BTreeSet<String>>,
    addresses: HashMap<String, Address>,
}

impl ResourceLookupIndex {
    fn build(resources: &[ResourceRecord]) -> Self {
        let mut index = Self::default();
        for resource in resources {
            let Address::Resource {
                resource_system,
                resource_namespace,
                resource_type,
                logical_name,
            } = &resource.logical_address
            else {
                continue;
            };
            let address_key = entmoot_contracts::address_identity_key(&resource.logical_address);
            index
                .addresses
                .insert(address_key.clone(), resource.logical_address.clone());
            index
                .exact
                .entry((
                    resource_system.clone(),
                    resource_namespace.clone(),
                    logical_name.clone(),
                ))
                .or_default()
                .push(address_key.clone());
            if resource_system == "kubernetes" {
                let namespace = kubernetes_namespace(resource_namespace);
                let kind = resource_type.rsplit('/').next().unwrap_or(resource_type);
                index
                    .kubernetes_exact
                    .entry((namespace.clone(), kind.to_string(), logical_name.clone()))
                    .or_default()
                    .push(address_key.clone());
                if is_workload_native_type(resource_type) {
                    for (label, value) in &resource.identity_labels {
                        index
                            .kubernetes_labels
                            .entry((namespace.clone(), label.clone(), value.clone()))
                            .or_default()
                            .insert(address_key.clone());
                    }
                }
            }
        }
        index
    }

    fn matching(&self, source: &Address, target: &str) -> Vec<Address> {
        self.matching_with_probe_count(source, target).0
    }

    fn matching_with_probe_count(&self, source: &Address, target: &str) -> (Vec<Address>, usize) {
        let Address::Resource {
            resource_system: source_system,
            resource_namespace: source_namespace,
            ..
        } = source
        else {
            return (Vec::new(), 0);
        };
        let selector = target.strip_prefix("selector:").and_then(|value| {
            let (namespace, terms) = value.split_once(':')?;
            let labels = terms
                .split(',')
                .filter_map(|term| term.split_once('='))
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>();
            Some((namespace, labels))
        });
        let mut probe_count = 0;
        let keys = if let Some((namespace, labels)) = selector {
            if source_system != "kubernetes" || labels.is_empty() {
                Vec::new()
            } else {
                let mut sets = labels
                    .iter()
                    .map(|(label, value)| {
                        self.kubernetes_labels.get(&(
                            namespace.to_string(),
                            label.clone(),
                            value.clone(),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()
                    .unwrap_or_default();
                sets.sort_by_key(|set| set.len());
                sets.first().map_or_else(Vec::new, |first| {
                    first
                        .iter()
                        .filter(|candidate| {
                            probe_count += 1;
                            sets[1..].iter().all(|set| {
                                probe_count += 1;
                                set.contains(*candidate)
                            })
                        })
                        .cloned()
                        .collect()
                })
            }
        } else if source_system == "kubernetes" {
            let mut parts = target.splitn(3, '/');
            let (Some(kind), Some(namespace), Some(name)) =
                (parts.next(), parts.next(), parts.next())
            else {
                return (Vec::new(), 0);
            };
            self.kubernetes_exact
                .get(&(namespace.to_string(), kind.to_string(), name.to_string()))
                .cloned()
                .unwrap_or_default()
        } else {
            self.exact
                .get(&(
                    source_system.clone(),
                    source_namespace.clone(),
                    target.to_string(),
                ))
                .cloned()
                .unwrap_or_default()
        };
        probe_count += keys.len();
        let mut addresses = keys
            .iter()
            .filter_map(|key| self.addresses.get(key).cloned())
            .collect::<Vec<_>>();
        addresses.sort_by_key(entmoot_contracts::address_identity_key);
        addresses.dedup_by(|left, right| {
            entmoot_contracts::address_identity_key(left)
                == entmoot_contracts::address_identity_key(right)
        });
        (addresses, probe_count)
    }
}

fn kubernetes_namespace(resource_namespace: &str) -> String {
    resource_namespace
        .rsplit_once("/namespace/")
        .map_or(resource_namespace, |(_, namespace)| namespace)
        .to_string()
}

fn is_workload_native_type(resource_type: &str) -> bool {
    ["/Deployment", "/StatefulSet", "/DaemonSet"]
        .iter()
        .any(|suffix| resource_type.ends_with(suffix))
}

/// Materialize resource occurrences as inventory nodes while keeping semantic
/// relations in the addressable RelationEdge/RelationClaim collections.
pub fn apply_configuration_inventory(graph: &mut TopologyGraph, inventory: ConfigInventory) {
    materialize_resource_records(graph, inventory.resources);
    for edge in inventory.relation_edges {
        graph.upsert_relation_edge(edge);
    }
    for claim in inventory.relation_claims {
        graph.insert_relation_claim(claim);
    }
    for relation in inventory.unresolved_relations {
        graph.insert_unresolved_relation(relation);
    }
    graph
        .diagnostics
        .extend(
            inventory
                .diagnostics
                .into_iter()
                .map(|diagnostic| ArtifactDiagnostic {
                    severity: if diagnostic.code == "config.parse_failed" {
                        DiagnosticSeverity::Error
                    } else {
                        DiagnosticSeverity::Info
                    },
                    code: diagnostic.code,
                    message: diagnostic.message,
                    artifact_id: None,
                    provider_pointer: Some(match diagnostic.line {
                        Some(line) => format!("{}:{line}", diagnostic.path),
                        None => diagnostic.path,
                    }),
                }),
        );
    graph.rebuild_indexes();
}

pub fn materialize_resource_records(graph: &mut TopologyGraph, resources: Vec<ResourceRecord>) {
    for resource in resources {
        let (path, start_line, start_byte, end_byte) = match &resource.origin {
            ResourceOrigin::Source { location, .. } => {
                let region = location.extent.region();
                (
                    Some(location.path.clone()),
                    region.and_then(|r| match r {
                        SourceRegion::Text { range, .. } => Some(range.start.line as usize),
                        SourceRegion::Bytes { .. } => None,
                    }),
                    region.and_then(|r| match r {
                        SourceRegion::Bytes { span } => Some(span.start_byte),
                        SourceRegion::Text { .. } => None,
                    }),
                    region.and_then(|r| match r {
                        SourceRegion::Bytes { span } => Some(span.end_byte),
                        SourceRegion::Text { .. } => None,
                    }),
                )
            }
            ResourceOrigin::Artifact { .. } => (None, None, None, None),
        };
        let parent = path.as_ref().map(|path| {
            let file_id = crate::graph::file_node_id(path);
            if graph.node(&file_id).is_none() {
                let label = std::path::Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path);
                let mut file = TopologyNode::shell(
                    &file_id,
                    NodeKind::File,
                    label,
                    Some(crate::graph::repo_node_id()),
                );
                file.path = Some(path.clone());
                file.display_ref = path.clone();
                graph.upsert_node(file);
                graph.add_edge(StructuralEdge::structural(
                    crate::graph::repo_node_id(),
                    &file_id,
                    EdgeKind::Contains,
                ));
            }
            file_id
        });
        let mut node = TopologyNode::shell(
            &resource.occurrence_id,
            NodeKind::Resource,
            &resource.display_name,
            parent.clone(),
        );
        node.path = path;
        node.display_ref = resource.display_name.clone();
        node.start_line = start_line;
        node.start_byte = start_byte;
        node.end_byte = end_byte;
        node.address_id = Some(resource.resource_id.clone());
        node.logic_kind = Some(resource.native_type.clone());
        graph.upsert_node(node);
        if let Some(parent) = parent {
            graph.add_edge(StructuralEdge::structural(
                parent,
                &resource.occurrence_id,
                EdgeKind::Contains,
            ));
        }
        graph.resources.push(resource);
    }
    graph
        .resources
        .sort_by(|left, right| left.occurrence_id.cmp(&right.occurrence_id));
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn selector_lookup_work_is_bounded_by_posting_lists_not_inventory_size() {
        const RESOURCE_COUNT: u32 = 16_384;
        let mut resources = Vec::with_capacity(RESOURCE_COUNT as usize);
        for index in 0..RESOURCE_COUNT {
            let address = resource_address(
                "kubernetes",
                "example/repo/kubernetes/namespace/default",
                "apps/v1/Deployment",
                format!("workload-{index}"),
            )
            .unwrap();
            let mut record = ResourceRecord::build(
                EntityKind::Workload,
                address,
                format!("Deployment/default/workload-{index}"),
                "apps/v1/Deployment".into(),
                ResourceObservationKind::Declared,
                ResourceOrigin::Source {
                    repository_id: "example/repo".into(),
                    revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    snapshot_digest: None,
                    document_index: index,
                    location: SourceLocation {
                        path: "deployment.yaml".into(),
                        extent: LocationExtent::Region {
                            region: SourceRegion::Bytes {
                                span: entmoot_contracts::SiteSpan {
                                    start_byte: usize::try_from(index).unwrap(),
                                    end_byte: usize::try_from(index).unwrap() + 1,
                                },
                            },
                        },
                    },
                },
                None,
            )
            .unwrap();
            record
                .identity_labels
                .insert("instance".into(), format!("workload-{index}"));
            resources.push(record);
        }
        let index = ResourceLookupIndex::build(&resources);
        let source = resource_address(
            "kubernetes",
            "example/repo/kubernetes/namespace/default",
            "v1/Service",
            "api",
        )
        .unwrap();
        let (matches, probes) =
            index.matching_with_probe_count(&source, "selector:default:instance=workload-8192");
        assert_eq!(matches.len(), 1);
        assert!(
            probes <= 2,
            "lookup inspected {probes} postings for {RESOURCE_COUNT} resources"
        );
    }
}
