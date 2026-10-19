//! Durable knowledge records separable from topology view materialization (ADR-0001 / INV-19).

use std::collections::BTreeMap;

use entmoot_contracts::{
    Assertion, BomComposition, CapabilityObservation, ComponentDependency, ComponentOccurrence,
    ComponentVulnerability, FindingObservation, LocationAttachment, LocationRecord, Proposition,
    RelationClaim, RelationEdge, Resolution, ResolvedIdentityRecord, ResolverCoverageObservation,
    ResourceRecord, TraceRecord, TraceStepRecord,
};
use serde::{Deserialize, Serialize};

use crate::graph::{StructuralEdge, TopologyGraph};
use entmoot_contracts::ArtifactDiagnostic;
use entmoot_contracts::ArtifactManifest;
mod artifact_projection;

pub fn validate_declaration_site(
    site: &entmoot_contracts::DeclarationSiteRecord,
) -> Result<(), String> {
    site.validate()?;
    let address =
        crate::display_ref::parse_logic_address(&site.marker.site.occurrence.logic_address)
            .ok_or("invalid declaration logic address")?;
    if address.path != site.path
        || address.scope != site.marker.scope
        || address.symbol != site.marker.name
        || address.kind != site.marker.kind
        || address
            .start_byte
            .is_some_and(|start| start != site.marker.site.span.start_byte)
    {
        return Err("declaration address disagrees with source observation".into());
    }
    Ok(())
}

pub fn validate_code_contract(
    record: &entmoot_contracts::CodeContractRecord,
) -> Result<(), String> {
    record.validate()?;
    let address =
        crate::display_ref::parse_logic_address(&record.contract.site.occurrence.logic_address)
            .ok_or("invalid code contract logic address")?;
    if address.path != record.path
        || address.symbol != record.contract.name
        || address
            .start_byte
            .is_some_and(|start| start != record.contract.site.span.start_byte)
    {
        return Err("code contract address disagrees with source observation".into());
    }
    Ok(())
}

/// Knowledge plane records that must survive topology cache deletion.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeStore {
    /// Source-of-record signatures and object boundaries, scoped to exact source bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_contracts: Vec<entmoot_contracts::CodeContractRecord>,
    /// Immutable source-committed named declaration/initializer observations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declaration_sites: Vec<entmoot_contracts::DeclarationSiteRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverages: Vec<entmoot_contracts::CoverageObservation>,
    /// Source-committed syntax sites, retained independently of aggregate edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub call_sites: Vec<entmoot_contracts::CallSiteRecord>,
    /// Exact dependency roots used by owner-sized publication. They are ledger
    /// records, not topology nodes or positive assertions about source behavior.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_roots: Vec<entmoot_contracts::AnalysisInputRoot>,
    pub artifacts: Vec<ArtifactManifest>,
    pub capabilities: Vec<CapabilityObservation>,
    pub findings: Vec<FindingObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<ResourceRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentOccurrence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_dependencies: Vec<ComponentDependency>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_vulnerabilities: Vec<ComponentVulnerability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bom_compositions: Vec<BomComposition>,
    pub locations: Vec<LocationRecord>,
    pub traces: Vec<TraceRecord>,
    pub trace_steps: Vec<TraceStepRecord>,
    pub attachments: Vec<LocationAttachment>,
    pub assertions: Vec<Assertion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<Resolution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolver_coverages: Vec<ResolverCoverageObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_identities: Vec<ResolvedIdentityRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub propositions: Vec<Proposition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_edges: Vec<RelationEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_claims: Vec<RelationClaim>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_relations: Vec<entmoot_contracts::UnresolvedRelation>,
    pub diagnostics: Vec<ArtifactDiagnostic>,
    /// Exact inventory edges (contains/imports/calls/has_control_site) preserved for rebuild.
    pub structural_edges: Vec<StructuralEdge>,
}

impl KnowledgeStore {
    /// Extract durable knowledge + exact edges from a materialized topology view.
    pub fn extract_from_graph(graph: &TopologyGraph) -> Self {
        let digests = graph
            .snapshot_files
            .iter()
            .map(|f| (f.path.as_str(), &f.content_digest))
            .collect();
        let Self {
            call_sites,
            declaration_sites,
            code_contracts,
            ..
        } = Self::syntax_from_nodes(graph, &graph.nodes, &digests);
        let propositions = graph
            .relation_edges
            .iter()
            .map(RelationEdge::proposition)
            .map(|proposition| (proposition.id.as_str().to_owned(), proposition))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        Self {
            declaration_sites,
            call_sites,
            coverages: graph.coverages.clone(),
            input_roots: graph.input_roots.clone(),
            artifacts: graph.artifacts.clone(),
            capabilities: graph.capabilities.clone(),
            findings: graph.findings.clone(),
            resources: graph.resources.clone(),
            components: graph.components.clone(),
            component_dependencies: graph.component_dependencies.clone(),
            component_vulnerabilities: graph.component_vulnerabilities.clone(),
            bom_compositions: graph.bom_compositions.clone(),
            locations: graph.locations.clone(),
            code_contracts,
            traces: graph.traces.clone(),
            trace_steps: graph.trace_steps.clone(),
            attachments: graph.attachments.clone(),
            assertions: graph.assertions.clone(),
            resolutions: graph.resolutions.clone(),
            resolver_coverages: graph.resolver_coverages.clone(),
            resolved_identities: graph.resolved_identities.clone(),
            propositions,
            relation_edges: graph.relation_edges.clone(),
            relation_claims: graph.relation_claims.clone(),
            unresolved_relations: graph.unresolved_relations.clone(),
            diagnostics: graph.diagnostics.clone(),
            structural_edges: graph.edges.to_vec(),
        }
    }

    /// Project only the requested syntax nodes using the snapshot's complete
    /// path/digest index. Chunking does not change occurrence or record identity.
    pub fn syntax_from_nodes(
        graph: &TopologyGraph,
        nodes: &[crate::TopologyNode],
        digests: &BTreeMap<&str, &String>,
    ) -> Self {
        let mut call_sites: Vec<_> = nodes
            .iter()
            .filter_map(|node| {
                let path = node.path.as_deref()?;
                let marker = node.call_site.as_deref()?.clone();
                let mut site = entmoot_contracts::CallSiteRecord {
                    site_id: String::new(),
                    snapshot: graph.snapshot.clone(),
                    path: path.to_owned(),
                    content_digest: entmoot_contracts::ContentDigest::from_trusted(
                        (*digests.get(path)?).clone(),
                    ),
                    marker,
                };
                site.site_id = site.derive_id();
                Some(site)
            })
            .collect();
        call_sites.sort_by(|a, b| a.site_id.cmp(&b.site_id));
        let mut declaration_sites: Vec<_> = nodes
            .iter()
            .filter_map(|node| {
                let marker = node.declaration.as_deref()?.clone();
                let path = node.path.as_deref()?;
                let mut site = entmoot_contracts::DeclarationSiteRecord {
                    site_id: String::new(),
                    snapshot: graph.snapshot.clone(),
                    path: path.to_owned(),
                    content_digest: entmoot_contracts::ContentDigest::from_trusted(
                        (*digests.get(path)?).clone(),
                    ),
                    marker,
                };
                site.site_id = site.derive_id();
                Some(site)
            })
            .collect();
        declaration_sites.sort_by(|a, b| a.site_id.cmp(&b.site_id));
        let mut code_contracts: Vec<_> = nodes
            .iter()
            .filter_map(|node| {
                let path = node.path.as_deref()?;
                let mut record = entmoot_contracts::CodeContractRecord {
                    record_id: String::new(),
                    snapshot: graph.snapshot.clone(),
                    path: path.into(),
                    content_digest: entmoot_contracts::ContentDigest::from_trusted(
                        (*digests.get(path)?).clone(),
                    ),
                    contract: node.code_contract.as_deref()?.clone(),
                };
                record.record_id = record.derive_id();
                Some(record)
            })
            .collect();
        code_contracts.sort_by(|a, b| a.record_id.cmp(&b.record_id));
        Self {
            code_contracts,
            declaration_sites,
            call_sites,
            ..Default::default()
        }
    }

    /// Apply knowledge onto an inventory topology skeleton (nodes + snapshot chrome).
    ///
    /// Replaces evidence collections and exact edges. Display-only fields on nodes are left
    /// as provided by the inventory builder.
    pub fn apply_to_inventory(&self, inventory: TopologyGraph) -> TopologyGraph {
        let inventory = self.project_syntax(inventory);
        Self {
            input_roots: self.input_roots.clone(),
            artifacts: self.artifacts.clone(),
            capabilities: self.capabilities.clone(),
            findings: self.findings.clone(),
            resources: self.resources.clone(),
            components: self.components.clone(),
            component_dependencies: self.component_dependencies.clone(),
            component_vulnerabilities: self.component_vulnerabilities.clone(),
            bom_compositions: self.bom_compositions.clone(),
            locations: self.locations.clone(),
            traces: self.traces.clone(),
            trace_steps: self.trace_steps.clone(),
            attachments: self.attachments.clone(),
            assertions: self.assertions.clone(),
            resolutions: self.resolutions.clone(),
            resolver_coverages: self.resolver_coverages.clone(),
            resolved_identities: self.resolved_identities.clone(),
            relation_edges: self.relation_edges.clone(),
            relation_claims: self.relation_claims.clone(),
            unresolved_relations: self.unresolved_relations.clone(),
            diagnostics: self.diagnostics.clone(),
            structural_edges: self.structural_edges.clone(),
            ..Default::default()
        }
        .attach_collections(inventory)
    }

    /// Consume verified knowledge into a disposable inventory without cloning
    /// the evidence collections. Snapshot-qualified syntax is reattached below.
    pub fn into_inventory(self, inventory: TopologyGraph) -> TopologyGraph {
        let inventory = self.project_syntax(inventory);
        self.attach_collections(inventory)
    }

    fn project_syntax(&self, mut inventory: TopologyGraph) -> TopologyGraph {
        // A disposable projection cannot restore inactive observations. Rehydrate
        // syntax only from visible, snapshot-matched source-of-record rows below.
        for node in &mut inventory.nodes {
            node.declaration = None;
            node.code_contract = None;
        }
        inventory.coverages = self
            .coverages
            .iter()
            .filter(|c| c.snapshot == inventory.snapshot)
            .cloned()
            .collect();
        let digests: BTreeMap<_, _> = inventory
            .snapshot_files
            .iter()
            .map(|f| (f.path.clone(), f.content_digest.clone()))
            .collect();
        inventory.rebuild_indexes();
        for site in &self.declaration_sites {
            if site.snapshot != inventory.snapshot
                || validate_declaration_site(site).is_err()
                || digests.get(&site.path).map(String::as_str) != Some(site.content_digest.as_str())
            {
                continue;
            }
            let marker = &site.marker;
            let id = crate::graph::declaration_node_id(
                &site.path,
                &marker.name,
                marker.site.span.start_byte,
            );
            let mut node = inventory.node(&id).cloned().unwrap_or_else(|| {
                crate::TopologyNode::shell(
                    id,
                    crate::NodeKind::Declaration,
                    &marker.name,
                    Some(crate::graph::file_node_id(&site.path)),
                )
            });
            node.path = Some(site.path.clone());
            node.start_byte = Some(marker.site.span.start_byte);
            node.end_byte = Some(marker.site.span.end_byte);
            node.scope = marker.scope.clone();
            node.logic_kind = Some(marker.kind.clone());
            node.address_id = Some(marker.site.occurrence.address_id.as_str().to_owned());
            node.logic_address = Some(marker.site.occurrence.logic_address.clone());
            node.fqn = crate::display_ref::build_fqn(&site.path, &marker.scope, &marker.name);
            node.display_ref = node.fqn.clone().unwrap_or_else(|| site.path.clone());
            node.declaration = Some(Box::new(marker.clone()));
            inventory.upsert_node(node);
        }
        for site in &self.call_sites {
            if site.snapshot != inventory.snapshot
                || digests.get(&site.path).map(String::as_str) != Some(site.content_digest.as_str())
            {
                continue;
            }
            let marker = &site.marker;
            let owner = marker
                .owner
                .as_ref()
                .map(|owner| {
                    crate::graph::callable_node_id(&site.path, &owner.name, owner.start_byte)
                })
                .unwrap_or_else(|| crate::graph::file_node_id(&site.path));
            let id = crate::graph::call_stub_id(&site.path, marker.start_byte, &marker.callee);
            let mut node = inventory.node(&id).cloned().unwrap_or_else(|| {
                crate::TopologyNode::shell(
                    id,
                    crate::NodeKind::CallStub,
                    &marker.callee,
                    Some(owner),
                )
            });
            node.path = Some(site.path.clone());
            node.start_byte = Some(marker.start_byte);
            node.end_byte = Some(marker.end_byte);
            node.call_site = Some(Box::new(marker.clone()));
            inventory.upsert_node(node);
        }
        let mut contract_nodes: BTreeMap<_, Vec<usize>> = BTreeMap::new();
        for (index, node) in inventory.nodes.iter().enumerate() {
            if let (Some(path), Some(address), Some(start), Some(end)) =
                (&node.path, &node.address_id, node.start_byte, node.end_byte)
            {
                contract_nodes
                    .entry((path.clone(), address.clone(), start, end))
                    .or_default()
                    .push(index);
            }
        }
        let mut observations = BTreeMap::new();
        for record in &self.code_contracts {
            if record.snapshot != inventory.snapshot
                || validate_code_contract(record).is_err()
                || digests.get(&record.path).map(String::as_str)
                    != Some(record.content_digest.as_str())
            {
                continue;
            }
            let key = (
                record.path.clone(),
                record.contract.site.occurrence.address_id.to_string(),
                record.contract.site.span.start_byte,
                record.contract.site.span.end_byte,
            );
            let observation = observations.entry(key).or_insert(Some(&record.contract));
            if observation.is_some_and(|previous| previous != &record.contract) {
                // Retain competing durable observations, but do not choose one
                // as the signature used for selection on this projection.
                *observation = None;
            }
        }
        for (key, observation) in observations {
            if let Some(indices) = contract_nodes.get(&key) {
                for &index in indices {
                    inventory.nodes[index].code_contract = observation.cloned().map(Box::new);
                }
            }
        }
        inventory
    }

    fn attach_collections(self, mut inventory: TopologyGraph) -> TopologyGraph {
        drop(self.code_contracts);
        drop(self.declaration_sites);
        drop(self.call_sites);
        inventory.input_roots = self.input_roots;
        inventory.artifacts = self.artifacts;
        inventory.capabilities = self.capabilities;
        inventory.findings = self.findings;
        inventory.resources = self.resources;
        inventory.components = self.components;
        inventory.component_dependencies = self.component_dependencies;
        inventory.component_vulnerabilities = self.component_vulnerabilities;
        inventory.bom_compositions = self.bom_compositions;
        inventory.locations = self.locations;
        inventory.traces = self.traces;
        inventory.trace_steps = self.trace_steps;
        inventory.attachments = self.attachments;
        inventory.assertions = self.assertions;
        inventory.resolutions = self.resolutions;
        inventory.resolver_coverages = self.resolver_coverages;
        inventory.resolved_identities = self.resolved_identities;
        inventory.relation_edges = self.relation_edges;
        inventory.relation_claims = self.relation_claims;
        inventory.unresolved_relations = self.unresolved_relations;
        inventory.diagnostics = self.diagnostics;
        inventory.edges = self.structural_edges.into();
        inventory.canonicalize_inventory();
        inventory
    }

    /// Semantic fingerprint for PT-27 equality (ignores UI-only node display fields).
    pub fn semantic_fingerprint(&self) -> String {
        let mut parts = Vec::new();
        let mut contracts: Vec<_> = self
            .code_contracts
            .iter()
            .map(|r| r.record_id.as_str())
            .collect();
        contracts.sort_unstable();
        parts.push(format!("code_contracts:{}", contracts.join(",")));
        let mut declarations: Vec<_> = self
            .declaration_sites
            .iter()
            .map(|s| s.site_id.as_str())
            .collect();
        declarations.sort_unstable();
        parts.push(format!("declaration_sites:{}", declarations.join(",")));
        let mut sites: Vec<_> = self.call_sites.iter().map(|s| s.site_id.as_str()).collect();
        sites.sort_unstable();
        parts.push(format!("call_sites:{}", sites.join(",")));
        let mut coverage: Vec<_> = self
            .coverages
            .iter()
            .map(|s| s.coverage_id.as_str())
            .collect();
        coverage.sort_unstable();
        parts.push(format!("coverages:{}", coverage.join(",")));
        let mut roots = self
            .input_roots
            .iter()
            .map(|root| root.id.as_str())
            .collect::<Vec<_>>();
        roots.sort_unstable();
        parts.push(format!("input_roots:{}", roots.join(",")));
        parts.push(format!("artifacts:{}", self.artifacts.len()));
        parts.push(format!("findings:{}", self.findings.len()));
        parts.push(format!("resources:{}", self.resources.len()));
        parts.push(format!("components:{}", self.components.len()));
        parts.push(format!(
            "component_dependencies:{}",
            self.component_dependencies.len()
        ));
        parts.push(format!(
            "component_vulnerabilities:{}",
            self.component_vulnerabilities.len()
        ));
        parts.push(format!("bom_compositions:{}", self.bom_compositions.len()));
        parts.push(format!("locations:{}", self.locations.len()));
        parts.push(format!("attachments:{}", self.attachments.len()));
        parts.push(format!("assertions:{}", self.assertions.len()));
        parts.push(format!("resolutions:{}", self.resolutions.len()));
        parts.push(format!(
            "resolver_coverages:{}",
            self.resolver_coverages.len()
        ));
        let mut resolved_ids: Vec<_> = self
            .resolved_identities
            .iter()
            .map(ResolvedIdentityRecord::state_fingerprint)
            .collect();
        resolved_ids.sort_unstable();
        parts.push(format!("resolved_identities:{}", resolved_ids.join(",")));
        parts.push(format!("propositions:{}", self.propositions.len()));
        parts.push(format!("relation_edges:{}", self.relation_edges.len()));
        parts.push(format!("relation_claims:{}", self.relation_claims.len()));
        parts.push(format!(
            "unresolved_relations:{}",
            self.unresolved_relations.len()
        ));
        parts.push(format!("structural_edges:{}", self.structural_edges.len()));
        let mut assertion_ids: Vec<_> = self
            .assertions
            .iter()
            .map(|a| a.assertion_id.clone())
            .collect();
        assertion_ids.sort();
        parts.push(format!("assertion_ids:{}", assertion_ids.join(",")));
        let mut unresolved_relation_ids: Vec<_> = self
            .unresolved_relations
            .iter()
            .map(|relation| relation.id.as_str())
            .collect();
        unresolved_relation_ids.sort_unstable();
        parts.push(format!(
            "unresolved_relation_ids:{}",
            unresolved_relation_ids.join(",")
        ));
        let mut edge_keys: Vec<_> = self
            .structural_edges
            .iter()
            .map(|e| format!("{}>{}:{:?}", e.from, e.to, e.kind))
            .collect();
        edge_keys.sort();
        parts.push(format!("edge_keys:{}", edge_keys.join(";")));
        parts.join("|")
    }
}

/// Strip evidence collections from a graph, leaving inventory nodes/snapshot (view delete).
pub fn inventory_skeleton(graph: &TopologyGraph) -> TopologyGraph {
    let mut skeleton = TopologyGraph {
        schema_version: graph.schema_version.clone(),
        implementation: graph.implementation.clone(),
        root: graph.root.clone(),
        snapshot: graph.snapshot.clone(),
        snapshot_files: graph.snapshot_files.clone(),
        policy_ids: graph.policy_ids.clone(),
        nodes: graph.nodes.clone(),
        coverages: graph.coverages.clone(),
        input_roots: graph.input_roots.clone(),
        ..TopologyGraph::new(&graph.root)
    };
    skeleton.rebuild_indexes();
    skeleton
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind, TopologyNode, TOPOLOGY_SCHEMA_VERSION};
    use entmoot_contracts::{
        AssertionObject, ClaimScope, EntityKind, EntityRef, EpistemicStatus, SourceSnapshot,
        SourceState,
    };

    fn sample_graph() -> TopologyGraph {
        let mut g = TopologyGraph {
            implementation: crate::implementation_fingerprint(),
            coverages: Vec::new(),
            input_roots: Vec::new(),
            schema_version: TOPOLOGY_SCHEMA_VERSION.into(),
            root: "root".into(),
            snapshot: SourceSnapshot {
                repository_id: "local/test".into(),
                revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                source_state: SourceState::Committed,
                snapshot_digest: None,
                estate_id: None,
                subject: None,
                subject_aliases: Vec::new(),
            },
            snapshot_files: vec![].into(),
            policy_ids: vec![],
            nodes: vec![TopologyNode::shell(
                "n1",
                NodeKind::File,
                "a.py",
                Some("root".into()),
            )]
            .into(),
            edges: vec![StructuralEdge::structural("root", "n1", EdgeKind::Contains)].into(),
            artifacts: vec![],
            capabilities: vec![],
            findings: vec![],
            resources: vec![],
            components: vec![],
            component_dependencies: vec![],
            component_vulnerabilities: vec![],
            bom_compositions: vec![],
            locations: vec![],
            traces: vec![],
            trace_steps: vec![],
            resolutions: vec![],
            resolver_coverages: vec![],
            resolved_identities: vec![],
            attachments: vec![],
            assertions: vec![Assertion {
                assertion_id: "a1".into(),
                subject: EntityRef {
                    entity_id: "l1".into(),
                    entity_kind: EntityKind::Opaque,
                },
                predicate: "entmoot.finding_location".into(),
                object: AssertionObject {
                    entity: None,
                    value: None,
                },
                evidence_refs: vec![],
                epistemic_status: EpistemicStatus::Observed,
                derivation: None,
                supports: vec![],
                contradicts: vec![],
                observation_context_id: None,
                valid_from: None,
                valid_to: None,
                claim_scope: Some(ClaimScope::Occurrence),
                subject_address: None,
            }],
            relation_edges: vec![],
            relation_claims: vec![],
            unresolved_relations: vec![],
            diagnostics: vec![],
            node_by_id: Default::default(),
            edge_keys: Default::default(),
            relation_edge_by_id: Default::default(),
            claim_by_id: Default::default(),
            unresolved_relation_by_id: Default::default(),
        };
        g.rebuild_indexes();
        g
    }

    #[test]
    fn pt27_delete_topology_and_rematerialize_preserves_knowledge() {
        let original = sample_graph();
        let knowledge = KnowledgeStore::extract_from_graph(&original);
        let fp_before = knowledge.semantic_fingerprint();

        // Delete topology view / cache.
        let skeleton = inventory_skeleton(&original);
        assert!(skeleton.assertions.is_empty());
        assert!(skeleton.edges.is_empty());

        // Rematerialize from knowledge + inventory nodes.
        let rebuilt = knowledge.apply_to_inventory(skeleton);
        let fp_after = KnowledgeStore::extract_from_graph(&rebuilt).semantic_fingerprint();
        assert_eq!(fp_before, fp_after);
        assert_eq!(rebuilt.assertions.len(), 1);
        assert_eq!(rebuilt.edges.len(), 1);
        assert_eq!(rebuilt.nodes.len(), original.nodes.len());
    }
}
