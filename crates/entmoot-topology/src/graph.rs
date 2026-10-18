//! Topology graph schema (POC JSON interchange).

use std::collections::{HashMap, HashSet};
use std::fmt;

use entmoot_contracts::{
    topo_call_stub_id, topo_callable_id, topo_control_site_id, topo_declaration_id, topo_file_id,
    topo_import_id, topo_module_stub_id, topo_repo_id, ArtifactManifest, Assertion, BomComposition,
    CapabilityObservation, ComponentDependency, ComponentOccurrence, ComponentVulnerability,
    FindingObservation, LocationAttachment, LocationRecord, Resolution,
    ResolverCoverageObservation, ResourceRecord, SourceSnapshot, SourceState, TraceRecord,
    TraceStepRecord,
};
use entmoot_core::ResolveConfidence;
use serde::{Deserialize, Serialize};

use crate::display_ref::DisplayRefKind;

/// Current graph export contract for this workspace. The discarded POC name
/// `topology.graph.v1` is not part of this lineage. A later Entmoot publish may
/// bump versions; until then this greenfield repo stays on `logic.graph.v1`.
pub const TOPOLOGY_SCHEMA_VERSION: &str = "logic.graph.v1";

pub use entmoot_contracts::query::InventoryKind as NodeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Imports,
    Calls,
    HasControlSite,
    BindsCallable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyNode {
    /// Source-declared relationship boundary; retained with the inventory node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_contract: Option<entmoot_contracts::CodeContract>,
    /// Source-committed declaration syntax; no runtime value or access relation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<entmoot_contracts::DeclarationMarker>,
    /// Disposable projection of the durable snapshot-local syntax observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_site: Option<entmoot_contracts::CallSiteMarker>,
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    /// Repository-relative POSIX path for file-scoped nodes (not part of identity key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Fully qualified name when scope/symbol/path are sufficient (display / stitch aid only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fqn: Option<String>,
    /// Human-readable location: FQN, or `path:line`, or `path#hash`, or path.
    pub display_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_ref_kind: Option<DisplayRefKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logic_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structural_hash: Option<String>,
    /// Parser-derived callable boundary digest used for revision invalidation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_hash: Option<String>,
    /// Exact observed unit bytes (`blake3:…`); distinct from `structural_hash` / LogicId.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_body_id: Option<String>,
    /// LogicKind string for callables (`function`, `method`, …) from DefMarker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logic_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<usize>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub binding_count: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub call_site_count: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub control_site_count: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default)]
    pub resolved: bool,
    /// Present on parallel callable→callable `calls` edges from deterministic resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_confidence: Option<ResolveConfidence>,
    /// Durable ADR-0009 RelationEdgeId when projected from relation_edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_edge_id: Option<String>,
}

impl StructuralEdge {
    pub fn structural(from: impl Into<String>, to: impl Into<String>, kind: EdgeKind) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind,
            resolved: matches!(
                kind,
                EdgeKind::Contains | EdgeKind::HasControlSite | EdgeKind::BindsCallable
            ),
            resolve_confidence: None,
            relation_edge_id: None,
        }
    }

    pub fn resolved_call(
        from: impl Into<String>,
        to: impl Into<String>,
        confidence: ResolveConfidence,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Calls,
            resolved: true,
            resolve_confidence: Some(confidence),
            relation_edge_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFileDigest {
    pub path: String,
    pub content_digest: String,
}

/// Disposable normalization-survey counters for empirical review.
///
/// Exact-body or label divergence identifies groups worth reviewing; it does
/// not by itself prove a structural-hash collision because normalization is
/// intentionally insensitive to some exact-source differences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallableDedupSummary {
    pub callable_occurrences: usize,
    pub unique_structural_hashes: usize,
    pub deduplicated_callable_occurrences: usize,
    pub duplicate_structural_hash_groups: usize,
    pub exact_body_divergence_groups: usize,
    pub cross_label_structural_hash_groups: usize,
    pub max_structural_hash_group_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSummary {
    pub schema_version: String,
    pub graph_schema_version: String,
    pub snapshot: SourceSnapshot,
    pub snapshot_files: Vec<SnapshotFileDigest>,
    /// BLAKE3 commitment to the sorted repository-relative path set.
    pub observed_document_set_digest: String,
    /// BLAKE3 commitment to sorted `(path, content_digest)` entries.
    pub content_manifest_digest: String,
    pub callable_dedup: CallableDedupSummary,
    pub policy_ids: Vec<String>,
    pub nodes: usize,
    pub structural_edges: usize,
    pub artifacts: usize,
    pub findings: usize,
    pub resources: usize,
    pub relation_edges: usize,
    pub relation_claims: usize,
    pub unresolved_relations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyGraph {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverages: Vec<entmoot_contracts::CoverageObservation>,
    pub schema_version: String,
    pub implementation: entmoot_contracts::ImplementationFingerprint,
    pub root: String,
    pub snapshot: SourceSnapshot,
    pub snapshot_files: Vec<SnapshotFileDigest>,
    /// Structural hash policies used by file records in this projection.
    #[serde(default)]
    pub policy_ids: Vec<String>,
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<StructuralEdge>,
    pub artifacts: Vec<ArtifactManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_roots: Vec<entmoot_contracts::AnalysisInputRoot>,
    pub capabilities: Vec<CapabilityObservation>,
    pub findings: Vec<FindingObservation>,
    /// Configuration declarations and runtime observations of logical resources.
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<Resolution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolver_coverages: Vec<ResolverCoverageObservation>,
    /// Occurrence-scoped resolved-binding identities derived per Resolver artifact (ADR-0024).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_identities: Vec<entmoot_contracts::ResolvedIdentityRecord>,
    pub attachments: Vec<LocationAttachment>,
    pub assertions: Vec<Assertion>,
    /// Durable addressable relations (ADR-0009).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_edges: Vec<entmoot_contracts::RelationEdge>,
    /// RelationClaims about edges/addresses (ADR-0009); resolution state is query-derived.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_claims: Vec<entmoot_contracts::RelationClaim>,
    /// Attributable relation attempts with no soundly addressable target (ADR-0009).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_relations: Vec<entmoot_contracts::UnresolvedRelation>,
    pub diagnostics: Vec<entmoot_contracts::ArtifactDiagnostic>,
    /// Runtime index: node id → index into `nodes`. Rebuilt after load.
    #[serde(skip)]
    pub node_by_id: HashMap<String, usize>,
    /// Runtime index: (from, to, kind) → index into `edges`.
    #[serde(skip)]
    pub edge_keys: HashMap<(String, String, EdgeKind), usize>,
    /// Runtime index: durable relation edge id → index into `relation_edges`.
    #[serde(skip)]
    pub relation_edge_by_id: HashMap<String, usize>,
    /// Runtime index for O(1) claim-id deduplication.
    #[serde(skip)]
    pub claim_by_id: HashMap<String, usize>,
    /// Runtime index for O(1) unresolved-relation idempotency.
    #[serde(skip)]
    pub unresolved_relation_by_id: HashMap<String, usize>,
}

fn callable_dedup_summary(nodes: &[TopologyNode]) -> CallableDedupSummary {
    #[derive(Default)]
    struct Group<'a> {
        count: usize,
        exact_body_ids: HashSet<&'a str>,
        labels: HashSet<&'a str>,
    }

    let mut groups: HashMap<&str, Group<'_>> = HashMap::new();
    for node in nodes.iter().filter(|node| node.kind == NodeKind::Callable) {
        let Some(structural_hash) = node.structural_hash.as_deref() else {
            continue;
        };
        let group = groups.entry(structural_hash).or_default();
        group.count += 1;
        group.labels.insert(&node.label);
        if let Some(exact_body_id) = node.exact_body_id.as_deref() {
            group.exact_body_ids.insert(exact_body_id);
        }
    }

    let callable_occurrences = groups.values().map(|group| group.count).sum();
    let unique_structural_hashes = groups.len();
    CallableDedupSummary {
        callable_occurrences,
        unique_structural_hashes,
        deduplicated_callable_occurrences: callable_occurrences
            .saturating_sub(unique_structural_hashes),
        duplicate_structural_hash_groups: groups.values().filter(|group| group.count > 1).count(),
        exact_body_divergence_groups: groups
            .values()
            .filter(|group| group.count > 1 && group.exact_body_ids.len() > 1)
            .count(),
        cross_label_structural_hash_groups: groups
            .values()
            .filter(|group| group.count > 1 && group.labels.len() > 1)
            .count(),
        max_structural_hash_group_size: groups.values().map(|group| group.count).max().unwrap_or(0),
    }
}

/// Structural or identity errors that make a topology graph unsafe to consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyValidationError {
    pub violations: Vec<String>,
}

impl fmt::Display for TopologyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "topology graph failed validation: {}",
            self.violations.join("; ")
        )
    }
}

impl std::error::Error for TopologyValidationError {}

impl TopologyNode {
    pub fn shell(
        id: impl Into<String>,
        kind: NodeKind,
        label: impl Into<String>,
        parent: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            code_contract: None,
            call_site: None,
            declaration: None,
            kind,
            label: label.into(),
            path: None,
            parent,
            fqn: None,
            display_ref: String::new(),
            display_ref_kind: None,
            start_line: None,
            scope: Vec::new(),
            file_hash: None,
            address_id: None,
            logic_address: None,
            structural_hash: None,
            signature_hash: None,
            exact_body_id: None,
            logic_kind: None,
            start_byte: None,
            end_byte: None,
            binding_count: 0,
            call_site_count: 0,
            control_site_count: 0,
        }
    }
}

impl TopologyGraph {
    pub fn new(root: impl Into<String>) -> Self {
        let root = root.into();
        let placeholder = entmoot_contracts::hash_parts(&["unbound_graph_snapshot", &root]);
        Self {
            schema_version: TOPOLOGY_SCHEMA_VERSION.into(),
            implementation: crate::implementation_fingerprint(),
            coverages: Vec::new(),
            root: root.clone(),
            snapshot: SourceSnapshot {
                repository_id: root,
                revision: placeholder.clone(),
                source_state: SourceState::WorkingTree,
                snapshot_digest: Some(format!("blake3:{placeholder}")),
                estate_id: None,
                subject: None,
                subject_aliases: Vec::new(),
            },
            snapshot_files: Vec::new(),
            policy_ids: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            artifacts: Vec::new(),
            input_roots: Vec::new(),
            capabilities: Vec::new(),
            findings: Vec::new(),
            resources: Vec::new(),
            components: Vec::new(),
            component_dependencies: Vec::new(),
            component_vulnerabilities: Vec::new(),
            bom_compositions: Vec::new(),
            locations: Vec::new(),
            traces: Vec::new(),
            trace_steps: Vec::new(),
            resolutions: Vec::new(),
            resolver_coverages: Vec::new(),
            resolved_identities: Vec::new(),
            attachments: Vec::new(),
            assertions: Vec::new(),
            relation_edges: Vec::new(),
            relation_claims: Vec::new(),
            unresolved_relations: Vec::new(),
            diagnostics: Vec::new(),
            node_by_id: HashMap::new(),
            edge_keys: HashMap::new(),
            relation_edge_by_id: HashMap::new(),
            claim_by_id: HashMap::new(),
            unresolved_relation_by_id: HashMap::new(),
        }
    }

    /// Rebuild runtime indexes after deserialize or bulk mutation.
    pub fn rebuild_indexes(&mut self) {
        self.node_by_id.clear();
        for (idx, node) in self.nodes.iter().enumerate() {
            self.node_by_id.insert(node.id.clone(), idx);
        }
        self.edge_keys.clear();
        for (index, edge) in self.edges.iter().enumerate() {
            self.edge_keys
                .insert((edge.from.clone(), edge.to.clone(), edge.kind), index);
        }
        self.relation_edge_by_id.clear();
        for (index, edge) in self.relation_edges.iter().enumerate() {
            self.relation_edge_by_id
                .insert(edge.id.as_str().to_string(), index);
        }
        self.claim_by_id.clear();
        self.claim_by_id.extend(
            self.relation_claims
                .iter()
                .enumerate()
                .map(|(index, claim)| (claim.id.as_str().to_string(), index)),
        );
        self.unresolved_relation_by_id.clear();
        self.unresolved_relation_by_id.extend(
            self.unresolved_relations
                .iter()
                .enumerate()
                .map(|(index, relation)| (relation.id.as_str().to_string(), index)),
        );
    }

    pub fn record_policy_id(&mut self, policy_id: &str) {
        match self
            .policy_ids
            .binary_search_by(|existing| existing.as_str().cmp(policy_id))
        {
            Ok(_) => {}
            Err(index) => self.policy_ids.insert(index, policy_id.to_string()),
        }
    }

    pub fn summary(&self) -> GraphSummary {
        let observed_paths: Vec<String> = self
            .snapshot_files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let content_entries: Vec<(String, String)> = self
            .snapshot_files
            .iter()
            .map(|file| (file.path.clone(), file.content_digest.clone()))
            .collect();
        GraphSummary {
            schema_version: "entmoot.graph.summary.v1".into(),
            graph_schema_version: self.schema_version.clone(),
            snapshot: self.snapshot.clone(),
            snapshot_files: self.snapshot_files.clone(),
            observed_document_set_digest: entmoot_contracts::observed_document_set_digest(
                &observed_paths,
            ),
            content_manifest_digest: entmoot_contracts::content_manifest_digest(&content_entries),
            callable_dedup: callable_dedup_summary(&self.nodes),
            policy_ids: self.policy_ids.clone(),
            nodes: self.nodes.len(),
            structural_edges: self.edges.len(),
            artifacts: self.artifacts.len(),
            findings: self.findings.len(),
            resources: self.resources.len(),
            relation_edges: self.relation_edges.len(),
            relation_claims: self.relation_claims.len(),
            unresolved_relations: self.unresolved_relations.len(),
        }
    }

    pub fn node(&self, id: &str) -> Option<&TopologyNode> {
        self.node_by_id.get(id).and_then(|idx| self.nodes.get(*idx))
    }

    pub fn upsert_node(&mut self, node: TopologyNode) {
        if let Some(&idx) = self.node_by_id.get(&node.id) {
            self.nodes[idx] = node;
        } else {
            let id = node.id.clone();
            let idx = self.nodes.len();
            self.nodes.push(node);
            self.node_by_id.insert(id, idx);
        }
    }

    pub fn add_edge(&mut self, edge: StructuralEdge) {
        self.try_add_edge(edge);
    }

    /// Add an edge and report whether its structural key was new.
    pub fn try_add_edge(&mut self, edge: StructuralEdge) -> bool {
        let key = (edge.from.clone(), edge.to.clone(), edge.kind);
        if self.edge_keys.contains_key(&key) {
            return false;
        }
        self.edge_keys.insert(key, self.edges.len());
        self.edges.push(edge);
        true
    }

    pub fn set_resolved_call_relation_edge_id(
        &mut self,
        from_node: &str,
        to_node: &str,
        relation_edge_id: String,
    ) {
        let key = (from_node.to_string(), to_node.to_string(), EdgeKind::Calls);
        let Some(&index) = self.edge_keys.get(&key) else {
            return;
        };
        if let Some(edge) = self.edges.get_mut(index).filter(|edge| edge.resolved) {
            edge.relation_edge_id = Some(relation_edge_id);
        }
    }

    pub fn upsert_relation_edge(&mut self, edge: entmoot_contracts::RelationEdge) -> String {
        let id = edge.id.as_str().to_string();
        if let Some(&index) = self.relation_edge_by_id.get(&id) {
            self.relation_edges[index] = edge;
        } else {
            let index = self.relation_edges.len();
            self.relation_edges.push(edge);
            self.relation_edge_by_id.insert(id.clone(), index);
        }
        id
    }

    pub fn insert_relation_claim(&mut self, claim: entmoot_contracts::RelationClaim) -> bool {
        let id = claim.id.as_str().to_string();
        if self.claim_by_id.contains_key(&id) {
            return false;
        }
        self.claim_by_id.insert(id, self.relation_claims.len());
        self.relation_claims.push(claim);
        true
    }

    pub fn insert_unresolved_relation(
        &mut self,
        relation: entmoot_contracts::UnresolvedRelation,
    ) -> bool {
        let id = relation.id.as_str().to_string();
        if self.unresolved_relation_by_id.contains_key(&id) {
            return false;
        }
        self.unresolved_relation_by_id
            .insert(id, self.unresolved_relations.len());
        self.unresolved_relations.push(relation);
        true
    }

    pub fn remove_artifacts(&mut self, artifact_ids: &[String]) {
        let owns = |id: &str| artifact_ids.iter().any(|candidate| candidate == id);
        let removed_edge_support: HashSet<_> = self
            .relation_claims
            .iter()
            .filter(|claim| {
                claim
                    .evidence
                    .iter()
                    .any(|evidence| owns(&evidence.artifact_id))
            })
            .filter_map(|claim| match &claim.subject {
                entmoot_contracts::SubjectId::RelationEdge { edge_id } => {
                    Some(edge_id.as_str().to_string())
                }
                _ => None,
            })
            .collect();
        let removed_resource_nodes: HashSet<_> = self
            .resources
            .iter()
            .filter_map(|resource| match &resource.origin {
                entmoot_contracts::ResourceOrigin::Artifact { artifact_id, .. }
                    if owns(artifact_id) =>
                {
                    Some(resource.occurrence_id.clone())
                }
                _ => None,
            })
            .collect();
        self.resources.retain(|resource| {
            !matches!(
                &resource.origin,
                entmoot_contracts::ResourceOrigin::Artifact { artifact_id, .. } if owns(artifact_id)
            )
        });
        self.components.retain(|record| !owns(&record.artifact_id));
        self.component_dependencies
            .retain(|record| !owns(&record.artifact_id));
        self.component_vulnerabilities
            .retain(|record| !owns(&record.artifact_id));
        self.bom_compositions
            .retain(|record| !owns(&record.artifact_id));
        self.nodes
            .retain(|node| !removed_resource_nodes.contains(&node.id));
        self.edges.retain(|edge| {
            !removed_resource_nodes.contains(&edge.from)
                && !removed_resource_nodes.contains(&edge.to)
        });
        self.artifacts.retain(|a| !owns(&a.artifact_id));
        self.capabilities.retain(|c| !owns(&c.artifact_id));
        self.resolver_coverages
            .retain(|coverage| !owns(&coverage.artifact_id));
        self.resolved_identities.retain(|record| {
            !record
                .binding
                .source()
                .is_some_and(|source| owns(source.artifact.artifact_id.as_str()))
        });
        self.findings.retain(|f| !owns(&f.artifact_id));
        self.locations.retain(|l| !owns(&l.artifact_id));
        self.resolutions.retain(|r| {
            !r.evidence_refs
                .iter()
                .any(|evidence| owns(&evidence.artifact_id))
        });
        self.attachments.retain(|a| !owns(&a.artifact_id));
        self.assertions.retain(|assertion| {
            !assertion
                .evidence_refs
                .iter()
                .any(|evidence| owns(&evidence.artifact_id))
        });
        self.relation_claims.retain_mut(|claim| {
            // Derived claims depend on all their inputs. Direct producer claims
            // can retain independent evidence of the very same assertion.
            if claim.derivation.is_some() {
                return !claim.evidence.iter().any(|e| owns(&e.artifact_id));
            }
            crate::relation_claims::retain_independent_evidence(
                &mut claim.evidence,
                &mut claim.provenance,
                owns,
            )
        });
        self.unresolved_relations.retain_mut(|relation| {
            crate::relation_claims::retain_independent_evidence(
                &mut relation.evidence,
                &mut relation.provenance,
                owns,
            )
        });
        // Remove occurrences whose artifact support was invalidated unless
        // independent support survives. Never retain all `calls` merely because
        // structural also emits that predicate.
        let claimed_edges: HashSet<_> = self
            .relation_claims
            .iter()
            .filter_map(|c| match &c.subject {
                entmoot_contracts::SubjectId::RelationEdge { edge_id } => {
                    Some(edge_id.as_str().to_string())
                }
                _ => None,
            })
            .collect();
        self.relation_edges.retain(|e| {
            claimed_edges.contains(e.id.as_str()) || !removed_edge_support.contains(e.id.as_str())
        });
        self.diagnostics
            .retain(|d| d.artifact_id.as_deref().is_none_or(|id| !owns(id)));
        let finding_ids: HashSet<_> = self
            .findings
            .iter()
            .map(|f| f.observation_id.as_str())
            .collect();
        self.traces
            .retain(|t| finding_ids.contains(t.finding_observation_id.as_str()));
        let trace_ids: HashSet<_> = self.traces.iter().map(|t| t.trace_id.as_str()).collect();
        self.trace_steps
            .retain(|s| trace_ids.contains(s.trace_id.as_str()));
        self.rebuild_indexes();
    }

    /// Validate referential integrity and callable identity agreement.
    pub fn validate(&self) -> Result<(), TopologyValidationError> {
        let mut violations = Vec::new();
        if self.implementation != crate::implementation_fingerprint() {
            violations.push("incompatible graph implementation fingerprint; regenerate the projection from retained inputs".into());
        }
        for coverage in &self.coverages {
            if let Err(error) = coverage.validate() {
                violations.push(error);
            }
            if coverage.snapshot != self.snapshot {
                violations.push("coverage snapshot mismatch".into());
            }
        }
        for root in &self.input_roots {
            if let Err(error) = root.validate() {
                violations.push(error.to_string());
            }
            if root.repository_id != self.snapshot.repository_id {
                violations.push("input root repository mismatch".into());
            }
        }
        if self.schema_version != TOPOLOGY_SCHEMA_VERSION {
            violations.push(format!(
                "unsupported schema_version `{}` (expected `{TOPOLOGY_SCHEMA_VERSION}`)",
                self.schema_version
            ));
        }
        if !std::path::Path::new(&self.root).is_absolute() {
            violations.push("graph root must be an absolute path".into());
        }
        if let Err(error) = self.snapshot.validate() {
            violations.push(format!("invalid graph snapshot: {error}"));
        }
        if self.policy_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            violations.push("graph policy_ids must be sorted and unique".into());
        }
        for policy_id in &self.policy_ids {
            if let Err(error) = entmoot_contracts::PolicyId::parse(policy_id.clone()) {
                violations.push(format!("invalid graph policy_id `{policy_id}`: {error}"));
            }
        }
        let mut snapshot_paths = HashSet::new();
        for file in &self.snapshot_files {
            if !snapshot_paths.insert(file.path.as_str()) {
                violations.push(format!("duplicate snapshot file `{}`", file.path));
            }
            if entmoot_contracts::normalize_repo_path(&file.path).is_err() {
                violations.push(format!("invalid snapshot file path `{}`", file.path));
            }
            if !file.content_digest.starts_with("blake3:") {
                violations.push(format!(
                    "snapshot file `{}` has a non-blake3 digest",
                    file.path
                ));
            }
        }
        let mut ids = HashSet::with_capacity(self.nodes.len());
        let mut declaration_addresses = HashSet::new();
        let source_digests: HashMap<_, _> = self
            .snapshot_files
            .iter()
            .map(|file| (file.path.as_str(), file.content_digest.as_str()))
            .collect();
        let mut file_nodes_by_id = HashMap::new();
        for node in &self.nodes {
            if let Some(contract) = &node.code_contract {
                if let Err(error) = contract.validate() {
                    violations.push(error);
                }
                if !matches!(node.kind, NodeKind::Callable | NodeKind::Declaration)
                    || node.label != contract.name
                    || node.address_id.as_deref()
                        != Some(contract.site.occurrence.address_id.as_str())
                    || node.logic_address.as_deref()
                        != Some(&contract.site.occurrence.logic_address)
                    || node.start_byte != Some(contract.site.span.start_byte)
                    || node.end_byte != Some(contract.site.span.end_byte)
                    || !node
                        .path
                        .as_deref()
                        .is_some_and(|path| source_digests.contains_key(path))
                {
                    violations
                        .push("code contract disagrees with source inventory occurrence".into());
                }
            }
            if let Some(marker) = &node.declaration {
                if !declaration_addresses.insert(&marker.site.occurrence.address_id) {
                    violations.push("ambiguous declaration observation address".into());
                }
                if let Some((path, digest)) = node
                    .path
                    .as_deref()
                    .and_then(|path| source_digests.get(path).map(|digest| (path, *digest)))
                {
                    let mut site = entmoot_contracts::DeclarationSiteRecord {
                        site_id: String::new(),
                        snapshot: self.snapshot.clone(),
                        path: path.to_owned(),
                        content_digest: entmoot_contracts::ContentDigest::from_trusted(digest),
                        marker: marker.clone(),
                    };
                    site.site_id = site.derive_id();
                    if let Err(error) = crate::knowledge::validate_declaration_site(&site) {
                        violations.push(error);
                    }
                } else {
                    violations.push("declaration observation lacks a source commitment".into());
                }
                if node.kind != NodeKind::Declaration
                    || node.label != marker.name
                    || node.start_byte != Some(marker.site.span.start_byte)
                    || node.end_byte != Some(marker.site.span.end_byte)
                    || node.address_id.as_deref()
                        != Some(marker.site.occurrence.address_id.as_str())
                    || node.logic_address.as_deref() != Some(&marker.site.occurrence.logic_address)
                    || node.scope != marker.scope
                    || node.logic_kind.as_deref() != Some(&marker.kind)
                {
                    violations.push("declaration marker disagrees with inventory node".into());
                }
                match &marker.syntax {
                    Some(syntax) => {
                        if let Err(error) = syntax.validate(marker.site.span) {
                            violations.push(error);
                        }
                    }
                    None => violations.push("declaration observation has no syntax".into()),
                }
            }
            if let Some(marker) = &node.call_site {
                if node.kind != NodeKind::CallStub
                    || node.start_byte != Some(marker.start_byte)
                    || node.end_byte != Some(marker.end_byte)
                    || node.label != marker.callee
                {
                    violations.push("callsite marker disagrees with inventory node".into());
                }
                if let Err(error) = marker.validate() {
                    violations.push(error.into());
                }
            }
            if !ids.insert(node.id.as_str()) {
                violations.push(format!("duplicate node id `{}`", node.id));
            }
            if node.kind == NodeKind::File {
                file_nodes_by_id.insert(node.id.as_str(), node);
            }
        }
        for snapshot_file in &self.snapshot_files {
            let expected_id = file_node_id(&snapshot_file.path);
            match file_nodes_by_id.get(expected_id.as_str()) {
                Some(node) if node.path.as_deref() == Some(snapshot_file.path.as_str()) => {}
                Some(_) => violations.push(format!(
                    "snapshot file `{}` has a mismatched file node",
                    snapshot_file.path
                )),
                None => violations.push(format!(
                    "snapshot file `{}` has no file node",
                    snapshot_file.path
                )),
            }
        }
        let artifact_ids: HashSet<_> = self
            .artifacts
            .iter()
            .map(|a| a.artifact_id.as_str())
            .collect();
        if artifact_ids.len() != self.artifacts.len() {
            violations.push("duplicate artifact_id".into());
        }
        for artifact in &self.artifacts {
            if let Err(error) = artifact.validate_structure() {
                violations.push(format!(
                    "artifact `{}` has an invalid manifest: {error}",
                    artifact.artifact_id
                ));
            }
            if let Err(error) = artifact.verify_against_snapshot(&self.snapshot) {
                violations.push(format!(
                    "artifact `{}` does not match graph snapshot: {error}",
                    artifact.artifact_id
                ));
            }
        }
        for capability in &self.capabilities {
            if !artifact_ids.contains(capability.artifact_id.as_str()) {
                violations.push(format!(
                    "capability references missing artifact `{}`",
                    capability.artifact_id
                ));
            }
        }
        let capability_keys: HashSet<_> = self
            .capabilities
            .iter()
            .map(|capability| {
                (
                    capability.artifact_id.as_str(),
                    capability.scope_universe_id.as_str(),
                    capability.capability,
                    capability.basis.as_str(),
                )
            })
            .collect();
        if capability_keys.len() != self.capabilities.len() {
            violations.push("duplicate capability observation".into());
        }
        let location_by_id: HashMap<_, _> = self
            .locations
            .iter()
            .map(|location| (location.location_id.as_str(), location))
            .collect();
        let location_ids: HashSet<_> = location_by_id.keys().copied().collect();
        if location_ids.len() != self.locations.len() {
            violations.push("duplicate location_id".into());
        }
        for location in &self.locations {
            if let Some(region) = location.extent.region() {
                if let Err(error) = region.validate() {
                    violations.push(format!(
                        "location `{}` has invalid coordinates: {error}",
                        location.location_id
                    ));
                }
            }
            if !artifact_ids.contains(location.artifact_id.as_str()) {
                violations.push(format!(
                    "location references missing artifact `{}`",
                    location.artifact_id
                ));
            }
        }
        let finding_ids: HashSet<_> = self
            .findings
            .iter()
            .map(|f| f.observation_id.as_str())
            .collect();
        if finding_ids.len() != self.findings.len() {
            violations.push("duplicate finding observation_id".into());
        }
        let resource_occurrence_ids: HashSet<_> = self
            .resources
            .iter()
            .map(|resource| resource.occurrence_id.as_str())
            .collect();
        if resource_occurrence_ids.len() != self.resources.len() {
            violations.push("duplicate resource occurrence_id".into());
        }
        for resource in &self.resources {
            if let Err(error) = resource.validate() {
                violations.push(format!(
                    "resource `{}` is invalid: {error}",
                    resource.occurrence_id
                ));
            }
            match &resource.origin {
                entmoot_contracts::ResourceOrigin::Source {
                    repository_id,
                    revision,
                    snapshot_digest,
                    location,
                    ..
                } => {
                    if repository_id != &self.snapshot.repository_id
                        || revision != &self.snapshot.revision
                        || snapshot_digest != &self.snapshot.snapshot_digest
                    {
                        violations.push(format!(
                            "resource `{}` source snapshot disagrees with graph",
                            resource.occurrence_id
                        ));
                    }
                    if !snapshot_paths.contains(location.path.as_str()) {
                        violations.push(format!(
                            "resource `{}` source path is absent from graph snapshot",
                            resource.occurrence_id
                        ));
                    }
                }
                entmoot_contracts::ResourceOrigin::Artifact { artifact_id, .. } => {
                    if !artifact_ids.contains(artifact_id.as_str()) {
                        violations.push(format!(
                            "resource `{}` references missing artifact `{artifact_id}`",
                            resource.occurrence_id
                        ));
                    }
                }
            }
            match self.node(&resource.occurrence_id) {
                Some(node) if node.kind == NodeKind::Resource => {
                    let expected_path = match &resource.origin {
                        entmoot_contracts::ResourceOrigin::Source { location, .. } => {
                            Some(location.path.as_str())
                        }
                        entmoot_contracts::ResourceOrigin::Artifact { .. } => None,
                    };
                    if node.address_id.as_deref() != Some(resource.resource_id.as_str())
                        || node.path.as_deref() != expected_path
                        || node.logic_kind.as_deref() != Some(resource.native_type.as_str())
                    {
                        violations.push(format!(
                            "resource `{}` node projection disagrees with its record",
                            resource.occurrence_id
                        ));
                    }
                }
                Some(_) => violations.push(format!(
                    "resource `{}` projects to a non-resource node",
                    resource.occurrence_id
                )),
                None => violations.push(format!(
                    "resource `{}` has no projected node",
                    resource.occurrence_id
                )),
            }
        }
        let component_ids: HashSet<_> = self
            .components
            .iter()
            .map(|record| record.occurrence_id.as_str())
            .collect();
        if component_ids.len() != self.components.len() {
            violations.push("duplicate component occurrence_id".into());
        }
        for component in &self.components {
            if let Err(error) = component.validate() {
                violations.push(format!(
                    "component `{}` is invalid: {error}",
                    component.occurrence_id
                ));
            }
            if !artifact_ids.contains(component.artifact_id.as_str()) {
                violations.push(format!(
                    "component references missing artifact `{}`",
                    component.artifact_id
                ));
            }
        }
        let dependency_ids: HashSet<_> = self
            .component_dependencies
            .iter()
            .map(|record| record.dependency_id.as_str())
            .collect();
        if dependency_ids.len() != self.component_dependencies.len() {
            violations.push("duplicate component dependency_id".into());
        }
        for dependency in &self.component_dependencies {
            if let Err(error) = dependency.validate() {
                violations.push(format!(
                    "component dependency `{}` is invalid: {error}",
                    dependency.dependency_id
                ));
            }
            if !artifact_ids.contains(dependency.artifact_id.as_str()) {
                violations.push(format!(
                    "component dependency references missing artifact `{}`",
                    dependency.artifact_id
                ));
            }
            for occurrence_id in [
                &dependency.subject_occurrence_id,
                &dependency.object_occurrence_id,
            ]
            .into_iter()
            .flatten()
            {
                if !component_ids.contains(occurrence_id.as_str()) {
                    violations.push(format!(
                        "component dependency `{}` references missing component `{occurrence_id}`",
                        dependency.dependency_id
                    ));
                }
            }
        }
        let vulnerability_ids: HashSet<_> = self
            .component_vulnerabilities
            .iter()
            .map(|record| record.observation_id.as_str())
            .collect();
        if vulnerability_ids.len() != self.component_vulnerabilities.len() {
            violations.push("duplicate component vulnerability observation_id".into());
        }
        for vulnerability in &self.component_vulnerabilities {
            if let Err(error) = vulnerability.validate() {
                violations.push(format!(
                    "component vulnerability `{}` is invalid: {error}",
                    vulnerability.observation_id
                ));
            }
            if !artifact_ids.contains(vulnerability.artifact_id.as_str()) {
                violations.push(format!(
                    "component vulnerability references missing artifact `{}`",
                    vulnerability.artifact_id
                ));
            }
            for affect in &vulnerability.affects {
                if let Some(occurrence_id) = &affect.component_occurrence_id {
                    if !component_ids.contains(occurrence_id.as_str()) {
                        violations.push(format!(
                            "component affect `{}` references missing component `{occurrence_id}`",
                            affect.affect_id
                        ));
                    }
                }
            }
        }
        let composition_ids: HashSet<_> = self
            .bom_compositions
            .iter()
            .map(|record| record.composition_id.as_str())
            .collect();
        if composition_ids.len() != self.bom_compositions.len() {
            violations.push("duplicate BOM composition_id".into());
        }
        for composition in &self.bom_compositions {
            if let Err(error) = composition.validate() {
                violations.push(format!(
                    "BOM composition `{}` is invalid: {error}",
                    composition.composition_id
                ));
            }
            if !artifact_ids.contains(composition.artifact_id.as_str()) {
                violations.push(format!(
                    "BOM composition references missing artifact `{}`",
                    composition.artifact_id
                ));
            }
        }
        for finding in &self.findings {
            if !artifact_ids.contains(finding.artifact_id.as_str()) {
                violations.push(format!(
                    "finding references missing artifact `{}`",
                    finding.artifact_id
                ));
            }
            for id in finding
                .primary_location_ids
                .iter()
                .chain(finding.related_location_ids.iter())
            {
                if !location_ids.contains(id.as_str()) {
                    violations.push(format!("finding references missing location `{id}`"));
                }
            }
        }
        let trace_ids: HashSet<_> = self.traces.iter().map(|t| t.trace_id.as_str()).collect();
        if trace_ids.len() != self.traces.len() {
            violations.push("duplicate trace_id".into());
        }
        for finding in &self.findings {
            for trace_id in &finding.trace_ids {
                if !trace_ids.contains(trace_id.as_str()) {
                    violations.push(format!("finding references missing trace `{trace_id}`"));
                }
            }
        }
        for trace in &self.traces {
            if !finding_ids.contains(trace.finding_observation_id.as_str()) {
                violations.push(format!(
                    "trace references missing finding `{}`",
                    trace.finding_observation_id
                ));
            }
        }
        for step in &self.trace_steps {
            if !trace_ids.contains(step.trace_id.as_str()) {
                violations.push(format!(
                    "trace step references missing trace `{}`",
                    step.trace_id
                ));
            }
            if !location_ids.contains(step.location_id.as_str()) {
                violations.push(format!(
                    "trace step references missing location `{}`",
                    step.location_id
                ));
            }
        }
        let step_by_id: HashMap<_, _> = self
            .trace_steps
            .iter()
            .map(|step| (step.step_id.as_str(), step))
            .collect();
        if step_by_id.len() != self.trace_steps.len() {
            violations.push("duplicate trace step_id".into());
        }
        for trace in &self.traces {
            let mut expected_ordinal = 0_u32;
            for step_id in &trace.step_ids {
                match step_by_id
                    .get(step_id.as_str())
                    .copied()
                    .filter(|step| step.trace_id == trace.trace_id)
                {
                    Some(step) if step.ordinal == expected_ordinal => expected_ordinal += 1,
                    Some(step) => violations.push(format!(
                        "trace `{}` step `{}` has ordinal {}, expected {}",
                        trace.trace_id, step.step_id, step.ordinal, expected_ordinal
                    )),
                    None => violations.push(format!(
                        "trace `{}` references missing step `{step_id}`",
                        trace.trace_id
                    )),
                }
            }
        }
        for attachment in &self.attachments {
            if let Some(location) = location_by_id.get(attachment.location_id.as_str()) {
                if let Err(error) = attachment.validate_location(location) {
                    violations.push(format!(
                        "attachment `{}` is invalid: {error}",
                        attachment.attachment_id
                    ));
                }
                if let Some(verified) = &attachment.verified_span {
                    if source_digests.get(location.path.as_str()).copied()
                        != Some(verified.source_digest.as_str())
                    {
                        violations.push(format!(
                            "attachment `{}` source digest disagrees with snapshot",
                            attachment.attachment_id
                        ));
                    }
                }
            }
            if !artifact_ids.contains(attachment.artifact_id.as_str()) {
                violations.push(format!(
                    "attachment references missing artifact `{}`",
                    attachment.artifact_id
                ));
            }
            if !location_ids.contains(attachment.location_id.as_str()) {
                violations.push(format!(
                    "attachment references missing location `{}`",
                    attachment.location_id
                ));
            }
            if let Some(target) = &attachment.selected_target {
                if let Some(node) = &target.node_id {
                    if !ids.contains(node.as_str()) {
                        violations.push(format!("attachment references missing node `{node}`"));
                    }
                }
            }
            for target in &attachment.candidate_targets {
                if let Some(node) = &target.node_id {
                    if !ids.contains(node.as_str()) {
                        violations.push(format!(
                            "attachment candidate references missing node `{node}`"
                        ));
                    }
                }
            }
            for target in attachment
                .selected_target
                .iter()
                .chain(&attachment.candidate_targets)
            {
                if let Some(port) = &target.port {
                    let node = target.node_id.as_ref().and_then(|id| self.node(id));
                    let ports = node
                        .and_then(|n| n.call_site.as_ref())
                        .and_then(|m| m.ports.as_ref());
                    let valid = target.kind == entmoot_contracts::AttachmentTargetKind::CallSite
                        && node.is_some_and(|n| n.kind == NodeKind::CallStub)
                        && ports.is_some_and(|p| match port {
                            entmoot_contracts::CallPort::Callee
                            | entmoot_contracts::CallPort::Result => true,
                            entmoot_contracts::CallPort::Receiver => p.receiver.is_some(),
                            entmoot_contracts::CallPort::Argument { position } => p
                                .arguments
                                .get(*position)
                                .is_some_and(|a| a.position == *position),
                        });
                    if !valid {
                        violations.push(format!(
                            "attachment `{}` has an invalid call port target",
                            attachment.attachment_id
                        ));
                    }
                }
            }
        }
        let attachment_ids: HashSet<_> = self
            .attachments
            .iter()
            .map(|attachment| attachment.attachment_id.as_str())
            .collect();
        if attachment_ids.len() != self.attachments.len() {
            violations.push("duplicate attachment_id".into());
        }

        // Every location has exactly one attachment; every attachment targets a known location.
        let mut attachments_by_location: HashMap<&str, usize> = HashMap::new();
        for attachment in &self.attachments {
            *attachments_by_location
                .entry(attachment.location_id.as_str())
                .or_default() += 1;
        }
        for location in &self.locations {
            match attachments_by_location
                .get(location.location_id.as_str())
                .copied()
            {
                Some(1) => {}
                Some(n) => violations.push(format!(
                    "location `{}` has {n} attachments; expected exactly one",
                    location.location_id
                )),
                None => violations.push(format!(
                    "location `{}` has no attachment",
                    location.location_id
                )),
            }
        }

        for attachment in &self.attachments {
            if let Some(target) = &attachment.selected_target {
                if let Some(node_id) = &target.node_id {
                    if let Some(node) = self.node(node_id) {
                        let expected_file = entmoot_contracts::topo_file_id(
                            &location_by_id
                                .get(attachment.location_id.as_str())
                                .map(|location| location.path.replace('\\', "/"))
                                .unwrap_or_default(),
                        );
                        if !expected_file.is_empty()
                            && !attachment_target_in_file_ancestry(self, node, &expected_file)
                        {
                            violations.push(format!(
                                "attachment `{}` target `{}` is outside location file ancestry",
                                attachment.attachment_id, node_id
                            ));
                        }
                    }
                }
            }
        }

        let assertion_ids: HashSet<_> = self
            .assertions
            .iter()
            .map(|assertion| assertion.assertion_id.as_str())
            .collect();
        if assertion_ids.len() != self.assertions.len() {
            violations.push("duplicate assertion_id".into());
        }
        let claim_ids: HashSet<_> = self.relation_claims.iter().map(|c| c.id.as_str()).collect();
        if claim_ids.len() != self.relation_claims.len() {
            violations.push("duplicate claim id".into());
        }
        for claim in &self.relation_claims {
            if let Err(error) = claim.validate() {
                violations.push(format!("claim `{}` is invalid: {error}", claim.id));
            }
            for evidence in &claim.evidence {
                validate_evidence_artifact(evidence, &artifact_ids, "claim", &mut violations);
            }
            if let Some(provenance) = &claim.provenance {
                validate_provenance_artifacts(
                    &provenance.artifacts,
                    &artifact_ids,
                    "claim",
                    &mut violations,
                );
            }
            if let Some(derivation) = &claim.derivation {
                for evidence in &derivation.input_refs {
                    validate_evidence_artifact(
                        evidence,
                        &artifact_ids,
                        "claim derivation",
                        &mut violations,
                    );
                }
                if let Some(evidence) = &derivation.calibration_evidence_ref {
                    validate_evidence_artifact(
                        evidence,
                        &artifact_ids,
                        "claim calibration",
                        &mut violations,
                    );
                }
            }
        }
        let unresolved_ids: HashSet<_> = self
            .unresolved_relations
            .iter()
            .map(|relation| relation.id.as_str())
            .collect();
        if unresolved_ids.len() != self.unresolved_relations.len() {
            violations.push("duplicate unresolved relation id".into());
        }
        for relation in &self.unresolved_relations {
            if let Err(error) = relation.validate() {
                violations.push(format!(
                    "unresolved relation `{}` is invalid: {error}",
                    relation.id
                ));
            }
            for evidence in &relation.evidence {
                validate_evidence_artifact(
                    evidence,
                    &artifact_ids,
                    "unresolved relation",
                    &mut violations,
                );
            }
            if let Some(provenance) = &relation.provenance {
                validate_provenance_artifacts(
                    &provenance.artifacts,
                    &artifact_ids,
                    "unresolved relation",
                    &mut violations,
                );
            }
            if relation.scope.domain == self.snapshot.repository_id
                && !scope_matches_snapshot(&relation.scope, &self.snapshot)
            {
                violations.push(format!(
                    "unresolved relation `{}` scope disagrees with graph snapshot",
                    relation.id
                ));
            }
        }
        let relation_edge_ids: HashSet<_> =
            self.relation_edges.iter().map(|e| e.id.as_str()).collect();
        if relation_edge_ids.len() != self.relation_edges.len() {
            violations.push("duplicate relation edge id".into());
        }
        let relation_edge_by_id: HashMap<_, _> = self
            .relation_edges
            .iter()
            .map(|edge| (edge.id.as_str(), edge))
            .collect();
        let proposition_ids: HashSet<_> = self
            .relation_edges
            .iter()
            .map(|edge| edge.proposition_id.as_str())
            .collect();
        let resource_addresses: HashSet<_> = self
            .resources
            .iter()
            .map(|resource| entmoot_contracts::address_identity_key(&resource.logical_address))
            .collect();
        for edge in &self.relation_edges {
            if let Err(error) = edge.validate() {
                violations.push(format!("relation edge `{}` is invalid: {error}", edge.id));
            }
            if (matches!(edge.schema_domain.as_str(), "source" | "configuration")
                && !scope_matches_snapshot(&edge.scope, &self.snapshot))
                || (edge.schema_domain == "resource_observation"
                    && !scope_snapshot_coordinates_match(&edge.scope, &self.snapshot))
            {
                violations.push(format!(
                    "snapshot-scoped relation edge `{}` scope disagrees with graph snapshot",
                    edge.id
                ));
            }
            if matches!(
                edge.schema_domain.as_str(),
                "configuration" | "resource_observation"
            ) && (!resource_addresses
                .contains(&entmoot_contracts::address_identity_key(&edge.subject))
                || !resource_addresses
                    .contains(&entmoot_contracts::address_identity_key(&edge.object)))
            {
                violations.push(format!(
                    "resource relation edge `{}` references an unknown logical resource",
                    edge.id
                ));
            }
        }
        for claim in &self.relation_claims {
            match &claim.subject {
                entmoot_contracts::SubjectId::Proposition { proposition_id } => {
                    if !proposition_ids.contains(proposition_id.as_str()) {
                        violations.push(format!(
                            "claim `{}` references missing proposition `{proposition_id}`",
                            claim.id
                        ));
                    }
                }
                entmoot_contracts::SubjectId::RelationEdge { edge_id } => {
                    match relation_edge_by_id.get(edge_id.as_str()) {
                        Some(edge) if !edge.accepts_claim_scope(&claim.scope) => {
                            violations.push(format!(
                                "claim `{}` scope disagrees with subject edge `{edge_id}`",
                                claim.id
                            ))
                        }
                        Some(_) => {}
                        None => violations.push(format!(
                            "claim `{}` references missing edge `{edge_id}`",
                            claim.id
                        )),
                    }
                }
                entmoot_contracts::SubjectId::RelationClaim { claim_id } => {
                    if !claim_ids.contains(claim_id.as_str()) {
                        violations.push(format!(
                            "claim `{}` references missing claim `{claim_id}`",
                            claim.id
                        ));
                    }
                }
                entmoot_contracts::SubjectId::Artifact { artifact_id } => {
                    if !artifact_ids.contains(artifact_id.as_str()) {
                        violations.push(format!(
                            "claim `{}` references missing artifact `{artifact_id}`",
                            claim.id
                        ));
                    }
                }
                entmoot_contracts::SubjectId::Address { .. } => {}
            }
            for linked in claim.supports.iter().chain(&claim.contradicts) {
                if !claim_ids.contains(linked.as_str()) {
                    violations.push(format!(
                        "claim `{}` links missing claim `{linked}`",
                        claim.id
                    ));
                }
            }
        }
        for assertion in &self.assertions {
            if assertion.claim_scope.is_none() {
                violations.push(format!(
                    "assertion `{}` missing claim_scope",
                    assertion.assertion_id
                ));
            }
            for evidence in &assertion.evidence_refs {
                validate_evidence_artifact(evidence, &artifact_ids, "assertion", &mut violations);
            }
            if let Some(derivation) = &assertion.derivation {
                for evidence in &derivation.input_refs {
                    validate_evidence_artifact(
                        evidence,
                        &artifact_ids,
                        "assertion derivation",
                        &mut violations,
                    );
                }
                if let Some(evidence) = &derivation.calibration_evidence_ref {
                    validate_evidence_artifact(
                        evidence,
                        &artifact_ids,
                        "assertion calibration",
                        &mut violations,
                    );
                }
            }
            if let Err(error) = assertion.validate_epistemic() {
                violations.push(error);
            }
            if let Some(subject) = &assertion.subject_address {
                if let Err(error) = subject.address.validate() {
                    violations.push(format!(
                        "assertion `{}` has invalid subject address: {error}",
                        assertion.assertion_id
                    ));
                }
            }
        }

        let resolution_ids: HashSet<_> = self
            .resolutions
            .iter()
            .map(|r| r.resolution_id.as_str())
            .collect();
        if resolution_ids.len() != self.resolutions.len() {
            violations.push("duplicate resolution_id".into());
        }
        for resolution in &self.resolutions {
            if let Err(error) = resolution.validate() {
                violations.push(format!(
                    "resolution `{}` is invalid: {error}",
                    resolution.resolution_id
                ));
            }
            for evidence in &resolution.evidence_refs {
                validate_evidence_artifact(evidence, &artifact_ids, "resolution", &mut violations);
            }
        }
        let coverage_ids: HashSet<_> = self
            .resolver_coverages
            .iter()
            .map(|coverage| coverage.coverage_id.as_str())
            .collect();
        if coverage_ids.len() != self.resolver_coverages.len() {
            violations.push("duplicate resolver coverage_id".into());
        }
        for coverage in &self.resolver_coverages {
            if let Err(error) = coverage.validate() {
                violations.push(format!(
                    "resolver coverage `{}` is invalid: {error}",
                    coverage.coverage_id
                ));
            }
            if !artifact_ids.contains(coverage.artifact_id.as_str()) {
                violations.push(format!(
                    "resolver coverage references missing artifact `{}`",
                    coverage.artifact_id
                ));
            }
        }
        let resolved_ids: HashSet<_> = self
            .resolved_identities
            .iter()
            .map(entmoot_contracts::ResolvedIdentityRecord::record_id)
            .collect();
        if resolved_ids.len() != self.resolved_identities.len() {
            violations.push("duplicate resolved identity for (address_id, artifact_id)".into());
        }
        let resolved_artifacts: HashMap<_, _> = self
            .artifacts
            .iter()
            .map(|manifest| (manifest.artifact_id.as_str(), manifest))
            .collect();
        for record in &self.resolved_identities {
            if let Err(error) = record.validate() {
                violations.push(format!(
                    "resolved identity `{}` is invalid: {error}",
                    record.address_id
                ));
            }
            if let Some(source) = record.binding.source() {
                let artifact_id = source.artifact.artifact_id.as_str();
                match resolved_artifacts.get(artifact_id) {
                    Some(manifest) => {
                        if let Err(error) = source.artifact.verify(manifest) {
                            violations.push(format!(
                                "resolved identity `{}`: {error}",
                                record.address_id
                            ));
                        }
                    }
                    None => violations.push(format!(
                        "resolved identity references missing artifact `{artifact_id}`"
                    )),
                }
            }
        }

        // (assertions and resolutions validated above)

        for node in &self.nodes {
            if let Some(parent) = &node.parent {
                if !ids.contains(parent.as_str()) {
                    violations.push(format!("node `{}` has missing parent `{parent}`", node.id));
                }
            }
            validate_definition_address(node, &mut violations);
        }

        let mut edge_keys = HashSet::with_capacity(self.edges.len());
        for edge in &self.edges {
            if !edge_keys.insert((edge.from.as_str(), edge.to.as_str(), edge.kind)) {
                violations.push(format!(
                    "duplicate {:?} edge `{}` -> `{}`",
                    edge.kind, edge.from, edge.to
                ));
            }
            if !ids.contains(edge.from.as_str()) {
                violations.push(format!("edge has missing source `{}`", edge.from));
            }
            if !ids.contains(edge.to.as_str()) {
                violations.push(format!("edge has missing target `{}`", edge.to));
            }
            if edge.kind == EdgeKind::BindsCallable {
                let source_kind = self.node(&edge.from).map(|node| node.kind);
                let target_kind = self.node(&edge.to).map(|node| node.kind);
                if source_kind != Some(NodeKind::Declaration)
                    || target_kind != Some(NodeKind::Callable)
                    || !edge.resolved
                {
                    violations.push(format!(
                        "binds_callable edge `{}` -> `{}` must be resolved declaration-to-callable",
                        edge.from, edge.to
                    ));
                }
            }
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(TopologyValidationError { violations })
        }
    }
}

fn scope_matches_snapshot(scope: &entmoot_contracts::Scope, snapshot: &SourceSnapshot) -> bool {
    scope.domain == snapshot.repository_id && scope_snapshot_coordinates_match(scope, snapshot)
}

fn scope_snapshot_coordinates_match(
    scope: &entmoot_contracts::Scope,
    snapshot: &SourceSnapshot,
) -> bool {
    scope.snapshot_revision.as_deref() == Some(snapshot.revision.as_str())
        && scope.snapshot_digest == snapshot.snapshot_digest
}

fn validate_evidence_artifact(
    evidence: &entmoot_contracts::EvidenceRef,
    artifact_ids: &HashSet<&str>,
    record_kind: &str,
    violations: &mut Vec<String>,
) {
    if !artifact_ids.contains(evidence.artifact_id.as_str()) {
        violations.push(format!(
            "{record_kind} references missing artifact `{}`",
            evidence.artifact_id
        ));
    }
}

fn validate_provenance_artifacts(
    provenance_artifacts: &[String],
    artifact_ids: &HashSet<&str>,
    record_kind: &str,
    violations: &mut Vec<String>,
) {
    for artifact_id in provenance_artifacts {
        if !artifact_ids.contains(artifact_id.as_str()) {
            violations.push(format!(
                "{record_kind} provenance references missing artifact `{artifact_id}`"
            ));
        }
    }
}

fn attachment_target_in_file_ancestry(
    graph: &TopologyGraph,
    node: &TopologyNode,
    file_id: &str,
) -> bool {
    if node.id == file_id {
        return true;
    }
    let mut parent = node.parent.as_deref();
    while let Some(parent_id) = parent {
        if parent_id == file_id {
            return true;
        }
        parent = graph
            .node(parent_id)
            .and_then(|candidate| candidate.parent.as_deref());
    }
    false
}

fn validate_definition_address(node: &TopologyNode, violations: &mut Vec<String>) {
    if !matches!(node.kind, NodeKind::Callable | NodeKind::Declaration) {
        return;
    }
    let Some(address) = node.logic_address.as_deref() else {
        if node.kind == NodeKind::Declaration {
            violations.push(format!("declaration `{}` has no logic_address", node.id));
        }
        return;
    };
    let Some(parsed) = crate::display_ref::parse_logic_address(address) else {
        violations.push(format!(
            "definition `{}` has invalid logic_address",
            node.id
        ));
        return;
    };
    match node.address_id.as_deref() {
        Some(address_id) if address_id == entmoot_core::hash::hash_address_envelope(address) => {}
        Some(_) => violations.push(format!(
            "definition `{}` address_id disagrees with logic_address",
            node.id
        )),
        None => violations.push(format!(
            "definition `{}` has logic_address without address_id",
            node.id
        )),
    }

    let expected_path = node.path.as_deref();
    if expected_path != Some(parsed.path.as_str()) {
        violations.push(format!(
            "definition `{}` path disagrees with logic_address",
            node.id
        ));
    }
    if parsed.scope != node.scope {
        violations.push(format!(
            "definition `{}` scope disagrees with logic_address",
            node.id
        ));
    }
    if parsed.symbol != node.label {
        violations.push(format!(
            "definition `{}` label disagrees with logic_address",
            node.id
        ));
    }
    if node.logic_kind.as_deref() != Some(parsed.kind.as_str()) {
        violations.push(format!(
            "definition `{}` kind disagrees with logic_address",
            node.id
        ));
    }
    let expected_fqn = crate::display_ref::build_fqn(&parsed.path, &parsed.scope, &parsed.symbol);
    if node.fqn != expected_fqn {
        violations.push(format!(
            "definition `{}` FQN disagrees with logic_address",
            node.id
        ));
    }
}

pub fn file_node_id(path: &str) -> String {
    topo_file_id(path)
}

pub fn callable_node_id(path: &str, name: &str, start_byte: usize) -> String {
    topo_callable_id(path, name, start_byte)
}

pub fn declaration_node_id(path: &str, name: &str, start_byte: usize) -> String {
    topo_declaration_id(path, name, start_byte)
}

pub fn import_node_id(path: &str, module_path: &str) -> String {
    topo_import_id(path, module_path)
}

pub fn call_stub_id(path: &str, start_byte: usize, callee: &str) -> String {
    topo_call_stub_id(path, start_byte, callee)
}

pub fn control_site_id(path: &str, start_byte: usize) -> String {
    topo_control_site_id(path, start_byte)
}

pub fn module_stub_id(module_path: &str) -> String {
    topo_module_stub_id(module_path)
}

pub fn repo_node_id() -> String {
    topo_repo_id("root")
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_second_unvalidated_children_hierarchy_is_rejected() {
        let node = super::TopologyNode::shell("a", super::NodeKind::Callable, "a", None);
        let mut value = serde_json::to_value(node).unwrap();
        value["children"] = serde_json::json!(["nonexistent-child"]);
        assert!(serde_json::from_value::<super::TopologyNode>(value).is_err());
    }
    use super::*;

    #[test]
    fn validation_rejects_dangling_edges_and_conflicting_identity() {
        let mut graph = TopologyGraph::new("C:/repo");
        let file_id = file_node_id("src/lib.rs");
        let callable_id = callable_node_id("src/lib.rs", "run", 0);
        graph.nodes.push(TopologyNode::shell(
            repo_node_id(),
            NodeKind::Repo,
            "repo",
            None,
        ));
        let mut file = TopologyNode::shell(
            file_id.clone(),
            NodeKind::File,
            "lib.rs",
            Some(repo_node_id()),
        );
        file.path = Some("src/lib.rs".into());
        graph.nodes.push(file);
        let mut callable = TopologyNode::shell(
            callable_id.clone(),
            NodeKind::Callable,
            "wrong",
            Some(file_id),
        );
        callable.path = Some("src/lib.rs".into());
        callable.logic_address = Some("logic|repo|rust|src/lib.rs|-|run|function|0|10".into());
        callable.logic_kind = Some("function".into());
        callable.start_byte = Some(0);
        callable.fqn = Some("src/lib.rs::wrong".into());
        graph.nodes.push(callable);
        graph.edges.push(StructuralEdge {
            from: callable_id,
            to: "missing".into(),
            kind: EdgeKind::Calls,
            resolved: false,
            resolve_confidence: None,
            relation_edge_id: None,
        });

        let error = graph.validate().unwrap_err();
        assert!(error.to_string().contains("label disagrees"));
        assert!(error.to_string().contains("missing target"));
    }

    #[test]
    fn v1_envelopes_require_matching_contract_and_derivation_fingerprints() {
        let root = tempfile::tempdir().unwrap();
        let graph = TopologyGraph::new(root.path().to_string_lossy());
        graph.validate().unwrap();
        let value = serde_json::to_value(&graph).unwrap();
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("implementation");
        assert!(serde_json::from_value::<TopologyGraph>(missing)
            .unwrap_err()
            .to_string()
            .contains("missing field `implementation`"));
        for field in ["contracts", "derivation"] {
            let mut mismatch = value.clone();
            mismatch["implementation"][field] =
                serde_json::json!(format!("blake3:{}", "a".repeat(64)));
            let decoded: TopologyGraph = serde_json::from_value(mismatch).unwrap();
            assert!(decoded
                .validate()
                .unwrap_err()
                .to_string()
                .contains("incompatible graph implementation fingerprint"));
        }
        let decoded: TopologyGraph = serde_json::from_value(value).unwrap();
        decoded.validate().unwrap();
    }

    #[test]
    fn schema_is_versioned_and_rejects_obsolete_fields() {
        let graph = TopologyGraph::new("C:/repo");
        let json = serde_json::to_value(&graph).unwrap();
        assert_eq!(json["schema_version"], TOPOLOGY_SCHEMA_VERSION);
        assert!(json.get("orphans").is_none());

        let obsolete = serde_json::json!({
            "schema_version": TOPOLOGY_SCHEMA_VERSION,
            "root": "C:/repo",
            "nodes": [],
            "edges": [],
            "orphans": []
        });
        let error = serde_json::from_value::<TopologyGraph>(obsolete).unwrap_err();
        assert!(error.to_string().contains("unknown field `orphans`"));
        for field in ["symbols", "symbol_occurrences", "semantic_symbols"] {
            let mut obsolete = json.clone();
            obsolete[field] = serde_json::json!([]);
            assert!(serde_json::from_value::<TopologyGraph>(obsolete)
                .unwrap_err()
                .to_string()
                .contains(&format!("unknown field `{field}`")));
            let mut knowledge = serde_json::to_value(crate::KnowledgeStore::default()).unwrap();
            knowledge[field] = serde_json::json!([]);
            assert!(serde_json::from_value::<crate::KnowledgeStore>(knowledge)
                .unwrap_err()
                .to_string()
                .contains(&format!("unknown field `{field}`")));
        }
    }

    #[test]
    fn validation_rejects_malformed_resolution_addresses() {
        let mut graph = TopologyGraph::new("repo");
        graph.resolutions.push(entmoot_contracts::Resolution {
            resolution_id: "resolution-1".into(),
            from: entmoot_contracts::Address::Opaque {
                system: String::new(),
                value: "source".into(),
            },
            to: entmoot_contracts::topology_node_address("target"),
            producer_id: "resolver".into(),
            producer_version: Some("1".into()),
            evidence_refs: vec![],
            validity: None,
            method_id: Some("resolver.v1".into()),
        });

        let error = graph.validate().unwrap_err();
        assert!(error
            .to_string()
            .contains("resolution `resolution-1` is invalid"));
    }

    #[test]
    fn summary_commits_to_observed_paths_and_contents_independently_of_order() {
        let mut first = TopologyGraph::new("C:/repo");
        first.snapshot_files = vec![
            SnapshotFileDigest {
                path: "src/b.rs".into(),
                content_digest: format!("blake3:{}", "b".repeat(64)),
            },
            SnapshotFileDigest {
                path: "src/a.rs".into(),
                content_digest: format!("blake3:{}", "a".repeat(64)),
            },
        ];
        let mut reordered = first.clone();
        reordered.snapshot_files.reverse();

        let summary = first.summary();
        let reordered_summary = reordered.summary();
        assert_eq!(
            summary.observed_document_set_digest,
            reordered_summary.observed_document_set_digest
        );
        assert_eq!(
            summary.content_manifest_digest,
            reordered_summary.content_manifest_digest
        );
        assert!(summary.observed_document_set_digest.starts_with("blake3:"));
        assert!(summary.content_manifest_digest.starts_with("blake3:"));

        reordered.snapshot_files[0].content_digest = format!("blake3:{}", "c".repeat(64));
        let changed = reordered.summary();
        assert_eq!(
            summary.observed_document_set_digest,
            changed.observed_document_set_digest
        );
        assert_ne!(
            summary.content_manifest_digest,
            changed.content_manifest_digest
        );
    }

    #[test]
    fn summary_reports_callable_dedup_review_signals() {
        let mut graph = TopologyGraph::new("C:/repo");
        for (index, label, body, exact) in [
            (0, "first", "same", "exact-a"),
            (1, "second", "same", "exact-b"),
            (2, "second", "same", "exact-b"),
            (3, "third", "unique", "exact-c"),
        ] {
            let mut node =
                TopologyNode::shell(format!("node-{index}"), NodeKind::Callable, label, None);
            node.structural_hash = Some(body.into());
            node.exact_body_id = Some(exact.into());
            graph.nodes.push(node);
        }

        let survey = graph.summary().callable_dedup;
        assert_eq!(survey.callable_occurrences, 4);
        assert_eq!(survey.unique_structural_hashes, 2);
        assert_eq!(survey.deduplicated_callable_occurrences, 2);
        assert_eq!(survey.duplicate_structural_hash_groups, 1);
        assert_eq!(survey.exact_body_divergence_groups, 1);
        assert_eq!(survey.cross_label_structural_hash_groups, 1);
        assert_eq!(survey.max_structural_hash_group_size, 3);
    }
}
