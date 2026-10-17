//! Topology extraction from per-file index records.

use std::collections::HashMap;
use std::path::Path;

use crate::display_ref::apply_display_refs_preserving;
use crate::resolve_edges::apply_resolved_call_edges;

use entmoot_contracts::{
    CallSiteMarker, ControlSite, DeclarationMarker, DefMarker, FileIndexRecord, ImportMarker,
    OccurrenceRecord, SourceSnapshot,
};
use entmoot_core::RepoScanResult;

use crate::graph::{
    call_stub_id, callable_node_id, control_site_id, declaration_node_id, file_node_id,
    import_node_id, module_stub_id, repo_node_id, EdgeKind, NodeKind, StructuralEdge,
    TopologyGraph, TopologyNode,
};
use crate::span_index::{definition_key_from_address, DefinitionIdentityKey, SpanIndex};

/// Reference-engine inventory builder; syntax callbacks are one cohesive implementation.
pub struct MarkerTopologyExtractor {
    reused_paths: std::collections::HashSet<String>,
    graph: TopologyGraph,
    span_index: SpanIndex,
    file_sources: HashMap<String, String>,
    current_file: Option<String>,
    occurrence_lookup: HashMap<DefinitionIdentityKey, OccurrenceRecord>,
    boundary_lookup: HashMap<String, String>,
    contract_lookup: HashMap<String, entmoot_contracts::CodeContract>,
    binding_counts: HashMap<String, usize>,
    call_counts: HashMap<String, usize>,
    control_site_counts: HashMap<String, usize>,
    pending_records: Vec<(String, FileIndexRecord)>,
}

impl MarkerTopologyExtractor {
    pub fn new(root: impl Into<String>) -> Self {
        Self::from_graph(TopologyGraph::new(root))
    }

    /// Construct an extractor whose derived records are scoped to an already
    /// observed source snapshot. The snapshot must be bound before `finish`,
    /// because resolution relation_claims are created during that method.
    pub fn with_snapshot(root: impl Into<String>, snapshot: SourceSnapshot) -> Self {
        let mut graph = TopologyGraph::new(root);
        graph.snapshot = snapshot;
        Self::from_graph(graph)
    }

    fn from_graph(graph: TopologyGraph) -> Self {
        Self {
            reused_paths: Default::default(),
            graph,
            span_index: SpanIndex::new(),
            file_sources: HashMap::new(),
            current_file: None,
            occurrence_lookup: HashMap::new(),
            boundary_lookup: HashMap::new(),
            contract_lookup: HashMap::new(),
            binding_counts: HashMap::new(),
            call_counts: HashMap::new(),
            control_site_counts: HashMap::new(),
            pending_records: Vec::new(),
        }
    }

    /// Continue extraction into an already-seeded structural inventory (file-incremental head).
    pub(crate) fn from_seeded_graph(graph: TopologyGraph) -> Self {
        let reused_paths = graph.nodes.iter().filter_map(|n| n.path.clone()).collect();
        let mut extractor = Self::from_graph(graph);
        extractor.reused_paths = reused_paths;
        extractor
    }

    fn owner_for_byte(&self, file: &str, byte: usize) -> Option<String> {
        self.span_index.resolve_byte(file, byte).map(|m| m.node_id)
    }
}

impl MarkerTopologyExtractor {
    pub fn on_repo_begin(&mut self, root: &str) {
        self.graph.upsert_node(TopologyNode::shell(
            repo_node_id(),
            NodeKind::Repo,
            root,
            None,
        ));
    }

    pub fn on_file(&mut self, path: &str, record: &FileIndexRecord, source: Option<&str>) {
        self.graph.record_policy_id(record.policy_id.as_str());
        self.current_file = Some(path.to_string());
        if let Some(src) = source {
            self.file_sources.insert(path.to_string(), src.to_string());
        }
        let file_id = file_node_id(path);
        self.pending_records
            .push((path.to_string(), record.clone()));

        self.span_index
            .ingest_file(path, record.markers.as_ref(), &record.occurrences, source);

        self.occurrence_lookup.clear();
        for occ in &record.occurrences {
            if let Some(key) = definition_key_from_address(&occ.logic_address) {
                self.occurrence_lookup.insert(key, occ.clone());
            }
        }
        self.boundary_lookup.clear();
        self.contract_lookup.clear();
        if let Some(markers) = &record.markers {
            self.contract_lookup
                .extend(markers.code_contracts.iter().map(|contract| {
                    (
                        contract.site.occurrence.address_id.to_string(),
                        contract.clone(),
                    )
                }));
        }
        self.boundary_lookup.extend(
            record
                .callable_boundaries
                .iter()
                .map(|boundary| (boundary.address_id.clone(), boundary.signature_hash.clone())),
        );

        if let Some(markers) = record.markers.as_ref() {
            for binding in &markers.bindings {
                if let Some(owner) = binding
                    .start_byte
                    .and_then(|byte| self.owner_for_byte(path, byte))
                {
                    *self.binding_counts.entry(owner).or_default() += 1;
                }
            }
        }

        let mut file_node = TopologyNode::shell(
            file_id.clone(),
            NodeKind::File,
            Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path),
            Some(repo_node_id()),
        );
        file_node.path = Some(path.to_string());
        file_node.file_hash = Some(record.file_hash.clone());
        self.graph.upsert_node(file_node);

        self.graph.add_edge(StructuralEdge::structural(
            repo_node_id(),
            &file_id,
            EdgeKind::Contains,
        ));

        if let Some(markers) = record.markers.as_ref() {
            for import in &markers.imports {
                self.on_import(&file_id, path, import);
            }
            for def in &markers.definitions {
                let occ = lookup_occurrence(def, &self.occurrence_lookup);
                self.on_definition(&file_id, path, def, occ.as_ref());
            }
            for declaration in &markers.declarations {
                self.on_declaration(&file_id, path, declaration);
            }
            for call in &markers.call_sites {
                let owner_id = call
                    .owner
                    .as_ref()
                    .map(|owner| callable_node_id(path, &owner.name, owner.start_byte))
                    .filter(|owner_id| self.graph.node(owner_id).is_some())
                    .unwrap_or_else(|| file_id.clone());
                self.on_call_site(&owner_id, path, call);
            }
            for site in &markers.control_sites {
                if let Some(owner) = self.owner_for_byte(path, site.from_byte) {
                    self.on_control_site(&owner, path, site);
                }
            }
        }
    }

    fn on_import(&mut self, file_id: &str, file_path: &str, import: &ImportMarker) {
        let stub_id = module_stub_id(&import.path);
        self.graph.upsert_node(TopologyNode::shell(
            stub_id,
            NodeKind::ModuleStub,
            &import.path,
            None,
        ));
        let import_id = import_node_id(file_path, &import.path);
        let mut import_node = TopologyNode::shell(
            import_id.clone(),
            NodeKind::ModuleStub,
            &import.path,
            Some(file_id.to_string()),
        );
        import_node.path = Some(file_path.to_string());
        import_node.start_byte = Some(import.start_byte);
        import_node.end_byte = Some(import.end_byte);
        self.graph.upsert_node(import_node);
        self.graph.add_edge(StructuralEdge {
            from: file_id.to_string(),
            to: import_id,
            kind: EdgeKind::Imports,
            resolved: false,
            resolve_confidence: None,
            relation_edge_id: None,
        });
    }

    fn on_definition(
        &mut self,
        file_id: &str,
        file_path: &str,
        def: &DefMarker,
        occurrence: Option<&OccurrenceRecord>,
    ) {
        let callable_id = callable_node_id(file_path, &def.name, def.start_byte);
        let mut callable_node = TopologyNode::shell(
            callable_id.clone(),
            NodeKind::Callable,
            &def.name,
            Some(file_id.to_string()),
        );
        callable_node.path = Some(file_path.to_string());
        callable_node.scope = def.scope.clone();
        callable_node.logic_kind = Some(def.kind.clone());
        callable_node.address_id = occurrence.map(|o| o.address_id.clone());
        callable_node.logic_address = occurrence.map(|o| o.logic_address.clone());
        callable_node.structural_hash = occurrence.map(|o| o.structural_hash.clone());
        callable_node.signature_hash = occurrence
            .and_then(|occurrence| self.boundary_lookup.get(&occurrence.address_id))
            .cloned();
        callable_node.exact_body_id = occurrence.and_then(|o| o.exact_body_id.clone());
        callable_node.code_contract = occurrence
            .and_then(|o| self.contract_lookup.get(&o.address_id))
            .cloned();
        callable_node.start_byte = Some(def.start_byte);
        callable_node.end_byte = Some(def.end_byte);
        self.graph.upsert_node(callable_node);
        self.graph.add_edge(StructuralEdge::structural(
            file_id,
            &callable_id,
            EdgeKind::Contains,
        ));
    }

    fn on_declaration(&mut self, file_id: &str, file_path: &str, declaration: &DeclarationMarker) {
        let node_id = declaration_node_id(
            file_path,
            &declaration.name,
            declaration.site.span.start_byte,
        );
        let mut node = TopologyNode::shell(
            node_id.clone(),
            NodeKind::Declaration,
            &declaration.name,
            Some(file_id.to_string()),
        );
        node.path = Some(file_path.to_string());
        node.scope = declaration.scope.clone();
        node.code_contract = self
            .contract_lookup
            .get(declaration.site.occurrence.address_id.as_str())
            .cloned();
        node.logic_kind = Some(declaration.kind.clone());
        if declaration.syntax.is_some() {
            node.declaration = Some(declaration.clone());
        }
        node.address_id = Some(declaration.site.occurrence.address_id.as_str().to_owned());
        node.logic_address = Some(declaration.site.occurrence.logic_address.clone());
        node.start_byte = Some(declaration.site.span.start_byte);
        node.end_byte = Some(declaration.site.span.end_byte);
        self.graph.upsert_node(node);
        self.graph.add_edge(StructuralEdge::structural(
            file_id,
            &node_id,
            EdgeKind::Contains,
        ));
        let target_id = declaration.callable.as_ref().and_then(|callable| {
            self.graph
                .nodes
                .iter()
                .find(|candidate| {
                    candidate.address_id.as_deref() == Some(callable.occurrence.address_id.as_str())
                })
                .map(|candidate| candidate.id.clone())
        });
        if let Some(target_id) = target_id {
            self.graph.add_edge(StructuralEdge::structural(
                &node_id,
                target_id,
                EdgeKind::BindsCallable,
            ));
        }
    }

    fn on_call_site(&mut self, owner_id: &str, file_path: &str, call: &CallSiteMarker) {
        let stub = call_stub_id(file_path, call.start_byte, &call.callee);
        *self.call_counts.entry(owner_id.to_string()).or_default() += 1;
        let mut call_node = TopologyNode::shell(
            stub.clone(),
            NodeKind::CallStub,
            &call.callee,
            Some(owner_id.to_string()),
        );
        call_node.path = Some(file_path.to_string());
        call_node.start_byte = Some(call.start_byte);
        call_node.end_byte = Some(call.end_byte);
        call_node.call_site = Some(call.clone());
        self.graph.upsert_node(call_node);
        self.graph.add_edge(StructuralEdge {
            from: owner_id.to_string(),
            to: stub,
            kind: EdgeKind::Calls,
            resolved: false,
            resolve_confidence: None,
            relation_edge_id: None,
        });
    }

    fn on_control_site(&mut self, owner_id: &str, file_path: &str, site: &ControlSite) {
        let site_id = control_site_id(file_path, site.from_byte);
        *self
            .control_site_counts
            .entry(owner_id.to_string())
            .or_default() += 1;
        let mut site_node = TopologyNode::shell(
            site_id.clone(),
            NodeKind::ControlSite,
            format!("{:?}", site.kind).to_lowercase(),
            Some(owner_id.to_string()),
        );
        site_node.path = Some(file_path.to_string());
        site_node.start_byte = Some(site.from_byte);
        site_node.end_byte = Some(site.to_byte);
        self.graph.upsert_node(site_node);
        self.graph.add_edge(StructuralEdge::structural(
            owner_id,
            &site_id,
            EdgeKind::HasControlSite,
        ));
    }

    pub fn on_file_end(&mut self, _file_id: &str) {
        self.current_file = None;
    }

    pub fn finish(self) -> TopologyGraph {
        let (mut graph, records) = self.finish_syntax();
        apply_resolved_call_edges(&mut graph, &records);
        graph
    }

    pub(crate) fn finish_syntax(mut self) -> (TopologyGraph, Vec<(String, FileIndexRecord)>) {
        for node in &mut self.graph.nodes {
            if let Some(c) = self.binding_counts.get(&node.id) {
                node.binding_count = *c;
            }
            if let Some(c) = self.call_counts.get(&node.id) {
                node.call_site_count = *c;
            }
            if let Some(c) = self.control_site_counts.get(&node.id) {
                node.control_site_count = *c;
            }
        }
        apply_display_refs_preserving(&mut self.graph, &self.file_sources, &self.reused_paths);
        let record_refs: Vec<(String, FileIndexRecord)> = self.pending_records;
        (self.graph, record_refs)
    }
}

pub fn build_topology_from_records(
    root: &str,
    records: &[(String, FileIndexRecord, Option<String>)],
) -> TopologyGraph {
    let mut extractor = MarkerTopologyExtractor::new(root);
    extractor.on_repo_begin(root);
    for (path, record, source) in records {
        let file_id = file_node_id(path);
        extractor.on_file(path, record, source.as_deref());
        extractor.on_file_end(&file_id);
    }
    extractor.finish()
}

pub fn build_topology_from_scan(root: &Path, scan: &RepoScanResult) -> TopologyGraph {
    build_topology_from_scan_with_snapshot(root, scan, None)
}

pub(crate) fn build_topology_from_scan_with_snapshot(
    root: &Path,
    scan: &RepoScanResult,
    snapshot: Option<SourceSnapshot>,
) -> TopologyGraph {
    let root_str = root.to_string_lossy();
    let records: Vec<(String, FileIndexRecord, Option<String>)> = scan
        .files
        .iter()
        .map(|f| {
            let rel = f
                .path
                .strip_prefix(root)
                .unwrap_or(&f.path)
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(&f.path).ok();
            (rel, f.record.clone(), source)
        })
        .collect();
    let mut extractor = match snapshot {
        Some(snapshot) => MarkerTopologyExtractor::with_snapshot(root_str.as_ref(), snapshot),
        None => MarkerTopologyExtractor::new(root_str.as_ref()),
    };
    extractor.on_repo_begin(&root_str);
    for (path, record, source) in &records {
        let file_id = file_node_id(path);
        extractor.on_file(path, record, source.as_deref());
        extractor.on_file_end(&file_id);
    }
    extractor.finish()
}

fn lookup_occurrence(
    def: &DefMarker,
    lookup: &HashMap<DefinitionIdentityKey, OccurrenceRecord>,
) -> Option<OccurrenceRecord> {
    lookup
        .get(&DefinitionIdentityKey::from_marker(def, true))
        .cloned()
        .or_else(|| {
            lookup
                .get(&DefinitionIdentityKey::from_marker(def, false))
                .cloned()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{DefMarker, FileMarkers, PolicyId, SourceState};
    use entmoot_core::{build_file_index, ResolveConfidence, StructuralHasher};

    #[test]
    fn resolver_records_use_the_bound_source_snapshot() {
        let record = FileIndexRecord {
            policy_id: PolicyId::from("a".repeat(64)),
            file_hash: "blake3:file".into(),
            module_shell_hash: "blake3:shell".into(),
            definitions: vec![],
            occurrences: vec![],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(FileMarkers {
                definitions: vec![DefMarker {
                    name: "main".into(),
                    kind: "function".into(),
                    scope: vec![],
                    start_byte: 0,
                    end_byte: 30,
                }],
                call_sites: vec![CallSiteMarker {
                    ports: None,
                    callee: "missing".into(),
                    start_byte: 10,
                    end_byte: 19,
                    owner: Some(entmoot_contracts::DefinitionSiteRef {
                        name: "main".into(),
                        scope: vec![],
                        start_byte: 0,
                    }),
                }],
                ..FileMarkers::new()
            }),
        };
        let snapshot = SourceSnapshot {
            repository_id: "https://github.com/example/repo".into(),
            revision: "1".repeat(40),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let checkout = if cfg!(windows) {
            "C:/checkout"
        } else {
            "/checkout"
        };
        let mut extractor = MarkerTopologyExtractor::with_snapshot(checkout, snapshot.clone());
        extractor.on_repo_begin(checkout);
        extractor.on_file("a.py", &record, None);
        extractor.on_file_end(&file_node_id("a.py"));
        let graph = extractor.finish();

        assert_eq!(graph.snapshot, snapshot);
        let unresolved = graph.unresolved_relations.first().unwrap();
        assert_eq!(unresolved.scope.domain, snapshot.repository_id);
        assert_eq!(
            unresolved.scope.snapshot_revision.as_deref(),
            Some(snapshot.revision.as_str())
        );
        assert_eq!(
            unresolved
                .provenance
                .as_ref()
                .and_then(|value| value.snapshot_revision.as_deref()),
            Some(snapshot.revision.as_str())
        );
        assert!(graph.validate().is_ok());

        let mut stale = graph;
        stale.snapshot.revision = "2".repeat(40);
        let error = stale.validate().unwrap_err().to_string();
        assert!(error.contains("scope disagrees with graph snapshot"));
    }

    #[test]
    fn materializes_class_and_assigned_callable_declarations() {
        let source = "class Widget:\n    pass\n\nhandler = lambda value: value\n";
        let result = StructuralHasher::new()
            .compute_granular_hash_with_analysis_at(
                source,
                "python",
                Some("a.py"),
                Some("example/repo"),
                None,
            )
            .unwrap();
        let graph = build_topology_from_records(
            "example/repo",
            &[(
                "a.py".into(),
                build_file_index(&result),
                Some(source.into()),
            )],
        );
        let declarations = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Declaration)
            .collect::<Vec<_>>();
        assert_eq!(declarations.len(), 2);
        assert!(declarations
            .iter()
            .any(|node| node.label == "Widget" && node.logic_kind.as_deref() == Some("class")));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == EdgeKind::BindsCallable
                && graph
                    .node(&edge.from)
                    .is_some_and(|node| node.label == "handler")
                && graph
                    .node(&edge.to)
                    .is_some_and(|node| node.kind == NodeKind::Callable)
        }));
    }

    #[test]
    fn extracts_file_and_callable_nodes() {
        let record = FileIndexRecord {
            policy_id: PolicyId::from("p"),
            file_hash: "blake3:abc".into(),
            module_shell_hash: "blake3:shell".into(),
            definitions: vec![],
            occurrences: vec![entmoot_contracts::OccurrenceRecord {
                address_id: "addr1".into(),
                logic_address: "logic|repo|python|a.py|-|f|function|0|10".into(),
                structural_hash: "blake3:body".into(),
                exact_body_id: None,
            }],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(FileMarkers {
                definitions: vec![DefMarker {
                    name: "f".into(),
                    kind: "function".into(),
                    scope: vec![],
                    start_byte: 0,
                    end_byte: 10,
                }],
                call_sites: vec![entmoot_contracts::CallSiteMarker {
                    ports: None,
                    callee: "print".into(),
                    start_byte: 5,
                    end_byte: 10,
                    owner: Some(entmoot_contracts::DefinitionSiteRef {
                        name: "f".into(),
                        scope: vec![],
                        start_byte: 0,
                    }),
                }],
                ..FileMarkers::new()
            }),
        };
        let graph = build_topology_from_records("repo", &[("a.py".into(), record, None)]);
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::Callable));
        let callable = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Callable)
            .unwrap();
        assert_eq!(callable.address_id.as_deref(), Some("addr1"));
        assert_eq!(callable.logic_kind.as_deref(), Some("function"));
        assert_eq!(callable.path.as_deref(), Some("a.py"));
        assert_eq!(callable.id, callable_node_id("a.py", "f", 0));
    }

    #[test]
    fn same_named_methods_keep_their_scoped_occurrences() {
        let definitions = vec![
            DefMarker {
                name: "handle".into(),
                kind: "method".into(),
                scope: vec!["Echo".into()],
                start_byte: 10,
                end_byte: 20,
            },
            DefMarker {
                name: "handle".into(),
                kind: "method".into(),
                scope: vec!["Upper".into()],
                start_byte: 30,
                end_byte: 40,
            },
        ];
        let record = FileIndexRecord {
            policy_id: PolicyId::from("p"),
            file_hash: "blake3:file".into(),
            module_shell_hash: "blake3:shell".into(),
            definitions: vec![],
            occurrences: vec![
                entmoot_contracts::OccurrenceRecord {
                    address_id: "echo-id".into(),
                    logic_address: "logic|repo|rust|lib.rs|Echo|handle|method|10|20".into(),
                    structural_hash: "echo-body".into(),
                    exact_body_id: None,
                },
                entmoot_contracts::OccurrenceRecord {
                    address_id: "upper-id".into(),
                    logic_address: "logic|repo|rust|lib.rs|Upper|handle|method|30|40".into(),
                    structural_hash: "upper-body".into(),
                    exact_body_id: None,
                },
            ],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(FileMarkers {
                definitions,
                ..FileMarkers::new()
            }),
        };

        let graph = build_topology_from_records("repo", &[("lib.rs".into(), record, None)]);
        let echo = graph.nodes.iter().find(|n| n.scope == ["Echo"]).unwrap();
        let upper = graph.nodes.iter().find(|n| n.scope == ["Upper"]).unwrap();
        assert_eq!(echo.address_id.as_deref(), Some("echo-id"));
        assert_eq!(upper.address_id.as_deref(), Some("upper-id"));
    }

    #[test]
    fn same_file_def_emits_resolved_call_edge() {
        let record = FileIndexRecord {
            policy_id: PolicyId::from("p"),
            file_hash: "blake3:file".into(),
            module_shell_hash: "blake3:shell".into(),
            definitions: vec![],
            occurrences: vec![],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(FileMarkers {
                definitions: vec![
                    DefMarker {
                        name: "helper".into(),
                        kind: "function".into(),
                        scope: vec![],
                        start_byte: 0,
                        end_byte: 10,
                    },
                    DefMarker {
                        name: "main".into(),
                        kind: "function".into(),
                        scope: vec![],
                        start_byte: 20,
                        end_byte: 40,
                    },
                ],
                call_sites: vec![entmoot_contracts::CallSiteMarker {
                    ports: None,
                    callee: "helper".into(),
                    start_byte: 25,
                    end_byte: 31,
                    owner: Some(entmoot_contracts::DefinitionSiteRef {
                        name: "main".into(),
                        scope: vec![],
                        start_byte: 20,
                    }),
                }],
                ..FileMarkers::new()
            }),
        };
        let graph = build_topology_from_records("repo", &[("a.py".into(), record, None)]);
        let caller = callable_node_id("a.py", "main", 20);
        let callee = callable_node_id("a.py", "helper", 0);
        let resolved = graph
            .edges
            .iter()
            .find(|e| e.from == caller && e.to == callee && e.resolved)
            .expect("resolved call edge");
        assert_eq!(
            resolved.resolve_confidence,
            Some(ResolveConfidence::SameFileDef)
        );
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::CallStub));
    }

    #[test]
    fn dynamic_callee_stays_unresolved_stub_only() {
        let record = FileIndexRecord {
            policy_id: PolicyId::from("p"),
            file_hash: "blake3:file".into(),
            module_shell_hash: "blake3:shell".into(),
            definitions: vec![],
            occurrences: vec![],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(FileMarkers {
                definitions: vec![DefMarker {
                    name: "main".into(),
                    kind: "function".into(),
                    scope: vec![],
                    start_byte: 0,
                    end_byte: 40,
                }],
                call_sites: vec![entmoot_contracts::CallSiteMarker {
                    ports: None,
                    callee: "obj.method".into(),
                    start_byte: 10,
                    end_byte: 20,
                    owner: Some(entmoot_contracts::DefinitionSiteRef {
                        name: "main".into(),
                        scope: vec![],
                        start_byte: 0,
                    }),
                }],
                ..FileMarkers::new()
            }),
        };
        let graph = build_topology_from_records("repo", &[("a.py".into(), record, None)]);
        assert!(graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .all(|e| !e.resolved));
        assert!(graph.relation_edges.is_empty());
        assert!(graph.relation_claims.is_empty());
        assert_eq!(graph.unresolved_relations.len(), 1);
        let unresolved = &graph.unresolved_relations[0];
        assert_eq!(
            unresolved.reason,
            entmoot_contracts::UnresolvedReason::UnresolvedBinding
        );
        assert_eq!(
            unresolved.occurrence_id.as_deref(),
            Some(call_stub_id("a.py", 10, "obj.method").as_str())
        );
    }

    #[test]
    fn module_level_call_is_retained_and_resolved_from_file_source() {
        let record = FileIndexRecord {
            policy_id: PolicyId::from("p"),
            file_hash: "blake3:file".into(),
            module_shell_hash: "blake3:shell".into(),
            definitions: vec![],
            occurrences: vec![],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(FileMarkers {
                call_sites: vec![entmoot_contracts::CallSiteMarker {
                    ports: None,
                    callee: "run".into(),
                    start_byte: 3,
                    end_byte: 8,
                    owner: None,
                }],
                ..FileMarkers::new()
            }),
        };
        let graph = build_topology_from_records("repo", &[("a.py".into(), record, None)]);
        let file = file_node_id("a.py");
        let stub = call_stub_id("a.py", 3, "run");
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.from == file && edge.to == stub && !edge.resolved));
        assert_eq!(graph.unresolved_relations.len(), 1);
        assert_eq!(
            graph.unresolved_relations[0].source,
            entmoot_contracts::topology_node_address(&file)
        );
    }
}
