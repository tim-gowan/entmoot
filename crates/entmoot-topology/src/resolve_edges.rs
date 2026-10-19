//! Deterministic resolve post-pass: parallel callable→callable `calls` edges.

mod summary;
pub use summary::CallSummary;
use summary::{CallDecision, CallOutcome, Lookup};

use std::collections::{HashMap, HashSet};

use entmoot_contracts::{FileIndexRecord, TargetConstraint, UnresolvedReason};
use entmoot_core::{resolve_repo_calls, ResolveConfidence, ResolvedCall};

use crate::graph::{call_stub_id, file_node_id, NodeKind, StructuralEdge, TopologyGraph};
use crate::relation_claims::{record_structural_resolved_call, record_structural_unresolved_call};

/// Emit resolved `calls` edges for same-file / import-local targets present in-graph.
///
/// Syntactic `call_stub` edges are retained for structural navigation.
pub fn apply_resolved_call_edges(graph: &mut TopologyGraph, records: &[(String, FileIndexRecord)]) {
    let index = StructuralCallIndex::new(graph);
    let file_refs: Vec<_> = records
        .iter()
        .map(|(path, record)| (path.as_str(), record))
        .collect();
    index.apply(graph, resolve_repo_calls(&file_refs));
}

/// A disposable callable index over the complete syntax inventory. It can stitch
/// independent file frontiers without rebuilding indexes or changing resolution.
#[derive(Default)]
pub struct StructuralCallIndex {
    by_path_name: HashMap<(String, String), Vec<String>>,
    by_fqn: HashMap<String, UniqueNodeId>,
    callable_ids: HashSet<String>,
}
impl StructuralCallIndex {
    pub fn new(graph: &TopologyGraph) -> Self {
        let mut index = Self::default();
        index.extend_nodes(&graph.nodes);
        index
    }
    /// Add one file's callable inventory before any call decisions are made.
    /// Import lookups still see the complete repository index after this barrier.
    pub fn extend_nodes(&mut self, nodes: &[crate::TopologyNode]) {
        for node in nodes.iter().filter(|node| node.kind == NodeKind::Callable) {
            self.callable_ids.insert(node.id.clone());
            if let Some(path) = &node.path {
                self.by_path_name
                    .entry((path.clone(), node.label.clone()))
                    .or_default()
                    .push(node.id.clone());
            }
            if let Some(fqn) = &node.fqn {
                insert_unique_node(&mut self.by_fqn, fqn, &node.id);
            }
        }
    }
    fn apply(&self, graph: &mut TopologyGraph, resolved: Vec<ResolvedCall>) {
        let mut lookup = Lookup::plain(self);
        for call in resolved {
            decide_call(&call, &mut lookup).apply(graph);
        }
    }
}

fn decide_call(call: &ResolvedCall, lookup: &mut Lookup<'_>) -> CallDecision {
    let caller = call
        .owner
        .as_ref()
        .map(|owner| crate::graph::callable_node_id(&call.file, &owner.name, owner.start_byte))
        .filter(|id| lookup.callable(id))
        .unwrap_or_else(|| file_node_id(&call.file));
    let occurrence = call_stub_id(&call.file, call.start_byte, &call.syntactic_callee);
    if matches!(
        call.confidence,
        ResolveConfidence::SameFileDef | ResolveConfidence::ImportLocal
    ) {
        match resolve_callee_node(call, lookup) {
            CalleeNodeMatch::One(target) => {
                return CallDecision {
                    caller,
                    occurrence,
                    outcome: CallOutcome::Resolved {
                        target,
                        confidence: call.confidence,
                    },
                }
            }
            CalleeNodeMatch::Ambiguous(candidates) => {
                return CallDecision {
                    caller,
                    occurrence,
                    outcome: CallOutcome::Unresolved {
                        target: TargetConstraint::Candidates {
                            addresses: candidates
                                .into_iter()
                                .map(|id| entmoot_contracts::topology_node_address(&id))
                                .collect(),
                        },
                        reason: UnresolvedReason::AmbiguousTarget,
                    },
                }
            }
            CalleeNodeMatch::None => {}
        }
    }
    let (target, reason) = unresolved_target(call, lookup);
    CallDecision {
        caller,
        occurrence,
        outcome: CallOutcome::Unresolved { target, reason },
    }
}

fn unresolved_target(
    call: &ResolvedCall,
    lookup: &mut Lookup<'_>,
) -> (TargetConstraint, UnresolvedReason) {
    if call.confidence == ResolveConfidence::Ambiguous {
        let head = callee_head(&call.syntactic_callee);
        if let Some(candidates) = lookup.path_name(&call.file, head) {
            if !candidates.is_empty() {
                return (
                    TargetConstraint::Candidates {
                        addresses: candidates
                            .iter()
                            .map(|id| entmoot_contracts::topology_node_address(id))
                            .collect(),
                    },
                    UnresolvedReason::AmbiguousTarget,
                );
            }
        }
        return (
            TargetConstraint::Syntactic {
                callee: call.syntactic_callee.clone(),
            },
            UnresolvedReason::AmbiguousTarget,
        );
    }

    let reason = if matches!(
        call.confidence,
        ResolveConfidence::SameFileDef | ResolveConfidence::ImportLocal
    ) {
        UnresolvedReason::TargetNotInInventory
    } else {
        UnresolvedReason::UnresolvedBinding
    };
    (
        TargetConstraint::Syntactic {
            callee: call.syntactic_callee.clone(),
        },
        reason,
    )
}

fn callee_head(callee: &str) -> &str {
    callee
        .split(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(callee)
}

fn resolve_callee_node(call: &ResolvedCall, lookup: &mut Lookup<'_>) -> CalleeNodeMatch {
    match call.confidence {
        ResolveConfidence::SameFileDef => {
            let head = callee_head(&call.syntactic_callee);
            // Prefer exact start_byte match via callable_node_id when unique name in file.
            let Some(candidates) = lookup.path_name(&call.file, head) else {
                return CalleeNodeMatch::None;
            };
            if candidates.len() == 1 {
                return CalleeNodeMatch::One(candidates[0].clone());
            }
            // Multiple same-name defs: use resolved_fqn path::scope.symbol if present.
            if let Some(fqn) = call.resolved_fqn.as_ref() {
                if let Some(id) = lookup.fqn(fqn) {
                    return CalleeNodeMatch::One(id.to_string());
                }
            }
            CalleeNodeMatch::Ambiguous(candidates.clone())
        }
        ResolveConfidence::ImportLocal => {
            let local = repository_local_import_nodes(call, lookup);
            if local.len() == 1 {
                return CalleeNodeMatch::One(local[0].clone());
            }
            if !local.is_empty() {
                return CalleeNodeMatch::Ambiguous(local);
            }
            let Some(fqn) = call.resolved_fqn.as_ref() else {
                return CalleeNodeMatch::None;
            };
            // A unique leaf spelling is not binding evidence. Package/module
            // identity must match exactly or an external Resolver must attach it.
            lookup.fqn(fqn).map_or(CalleeNodeMatch::None, |id| {
                CalleeNodeMatch::One(id.to_string())
            })
        }
        _ => CalleeNodeMatch::None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CalleeNodeMatch {
    One(String),
    Ambiguous(Vec<String>),
    None,
}

/// Resolve only import forms whose spelling itself proves a repository-relative
/// module location. Package aliases and absolute package names remain open.
fn repository_local_import_nodes(call: &ResolvedCall, lookup: &mut Lookup<'_>) -> Vec<String> {
    let Some(target) = call.resolved_fqn.as_deref() else {
        return Vec::new();
    };
    let mut path_symbols = Vec::new();
    if call.file.ends_with(".rs") {
        if target.starts_with("crate::")
            || target.starts_with("self::")
            || target.starts_with("super::")
        {
            let mut parts = target.split("::").collect::<Vec<_>>();
            let anchor = parts.remove(0);
            let Some(symbol) = parts.pop().filter(|part| !part.is_empty()) else {
                return Vec::new();
            };
            if let Some(src) = crate_source_root(&call.file) {
                let mut modules = match anchor {
                    "crate" => Vec::new(),
                    "self" | "super" => rust_module_parts(&call.file, src),
                    _ => Vec::new(),
                };
                if anchor == "super" {
                    modules.pop();
                }
                modules.extend(parts.into_iter().map(str::to_string));
                let module = modules.join("/");
                if module.is_empty() {
                    for root in ["lib.rs", "main.rs"] {
                        path_symbols.push((format!("{src}/{root}"), symbol.to_string()));
                    }
                } else {
                    path_symbols.push((format!("{src}/{module}.rs"), symbol.to_string()));
                    path_symbols.push((format!("{src}/{module}/mod.rs"), symbol.to_string()));
                }
            }
        }
    } else if target.starts_with('.') {
        if let Some((module, symbol)) = target.rsplit_once('.') {
            if !symbol.is_empty() {
                let base = parent_path(&call.file);
                let module_path = if call.file.ends_with(".py") {
                    normalize_python_relative_module(base, module)
                } else {
                    normalize_relative_path(base, module)
                };
                if has_source_extension(&module_path) {
                    path_symbols.push((module_path, symbol.to_string()));
                } else {
                    for extension in source_extensions(&call.file) {
                        path_symbols
                            .push((format!("{module_path}.{extension}"), symbol.to_string()));
                        path_symbols.push((
                            format!("{module_path}/index.{extension}"),
                            symbol.to_string(),
                        ));
                    }
                }
            }
        }
    }
    let mut nodes = path_symbols
        .into_iter()
        .filter_map(|(path, name)| lookup.path_name(&path, &name))
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    nodes.sort();
    nodes.dedup();
    nodes
}

fn normalize_python_relative_module(base: &str, module: &str) -> String {
    let dot_count = module.chars().take_while(|value| *value == '.').count();
    let mut rooted = base.to_string();
    for _ in 1..dot_count {
        rooted = parent_path(&rooted).to_string();
    }
    let tail = module[dot_count..].replace('.', "/");
    if tail.is_empty() {
        rooted
    } else if rooted.is_empty() {
        tail
    } else {
        format!("{rooted}/{tail}")
    }
}

fn crate_source_root(path: &str) -> Option<&str> {
    path.rfind("/src/")
        .map(|index| &path[..index + 4])
        .or_else(|| path.strip_prefix("src/").map(|_| "src"))
}

fn rust_module_parts(path: &str, source_root: &str) -> Vec<String> {
    let relative = path
        .strip_prefix(source_root)
        .and_then(|value| value.strip_prefix('/'))
        .unwrap_or(path);
    let mut parts = relative.split('/').map(str::to_string).collect::<Vec<_>>();
    if let Some(file) = parts.pop() {
        let stem = file.strip_suffix(".rs").unwrap_or(&file);
        if !matches!(stem, "lib" | "main" | "mod") {
            parts.push(stem.to_string());
        }
    }
    parts
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn normalize_relative_path(base: &str, module: &str) -> String {
    let mut parts = base
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for part in module.replace('\\', "/").split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value.to_string()),
        }
    }
    parts.join("/")
}

fn source_extensions(path: &str) -> &'static [&'static str] {
    if path.ends_with(".py") {
        &["py"]
    } else if path.ends_with(".ts") || path.ends_with(".tsx") {
        &["ts", "tsx", "js", "jsx"]
    } else {
        &["js", "jsx", "mjs", "cjs", "ts", "tsx"]
    }
}

fn has_source_extension(path: &str) -> bool {
    ["py", "js", "jsx", "mjs", "cjs", "ts", "tsx"]
        .iter()
        .any(|extension| path.ends_with(&format!(".{extension}")))
}

#[derive(Clone, Debug)]
enum UniqueNodeId {
    One(String),
    Ambiguous,
}

fn insert_unique_node(index: &mut HashMap<String, UniqueNodeId>, key: &str, node_id: &str) {
    use std::collections::hash_map::Entry;

    match index.entry(key.to_string()) {
        Entry::Vacant(entry) => {
            entry.insert(UniqueNodeId::One(node_id.to_string()));
        }
        Entry::Occupied(mut entry) => {
            if !matches!(entry.get(), UniqueNodeId::One(existing) if existing == node_id) {
                entry.insert(UniqueNodeId::Ambiguous);
            }
        }
    }
}

fn unique_node<'a>(index: &'a HashMap<String, UniqueNodeId>, key: &str) -> Option<&'a str> {
    match index.get(key) {
        Some(UniqueNodeId::One(node_id)) => Some(node_id),
        Some(UniqueNodeId::Ambiguous) | None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{CallSiteMarker, DefMarker, FileMarkers, PolicyId};

    use crate::extractor::build_topology_from_records;
    use crate::graph::{EdgeKind, NodeKind};

    fn match_node(
        call: &ResolvedCall,
        by_path_name: &HashMap<(String, String), Vec<String>>,
        by_fqn: &HashMap<String, UniqueNodeId>,
    ) -> CalleeNodeMatch {
        let index = StructuralCallIndex {
            by_path_name: by_path_name.clone(),
            by_fqn: by_fqn.clone(),
            callable_ids: Default::default(),
        };
        resolve_callee_node(call, &mut Lookup::plain(&index))
    }

    #[test]
    fn unique_node_index_never_selects_one_of_multiple_targets() {
        for ids in [["node-a", "node-b"], ["node-b", "node-a"]] {
            let mut index = HashMap::new();
            insert_unique_node(&mut index, "helper", ids[0]);
            insert_unique_node(&mut index, "helper", ids[1]);
            assert_eq!(unique_node(&index, "helper"), None);
        }
    }

    #[test]
    fn duplicate_observation_of_same_node_remains_unique() {
        let mut index = HashMap::new();
        insert_unique_node(&mut index, "helper", "node-a");
        insert_unique_node(&mut index, "helper", "node-a");
        assert_eq!(unique_node(&index, "helper"), Some("node-a"));
    }

    #[test]
    fn import_local_never_binds_an_unrelated_unique_leaf_name() {
        let call = ResolvedCall {
            file: "caller.py".into(),
            start_byte: 10,
            end_byte: 18,
            syntactic_callee: "helper".into(),
            owner: None,
            occurrence_fqn: None,
            resolved_fqn: Some("dependency.helper".into()),
            confidence: ResolveConfidence::ImportLocal,
        };
        let by_path_name = HashMap::from([(
            ("unrelated.py".into(), "helper".into()),
            vec!["unique-but-unrelated".into()],
        )]);
        let by_fqn = HashMap::from([(
            "unrelated.py::helper".into(),
            UniqueNodeId::One("unique-but-unrelated".into()),
        )]);

        assert_eq!(
            match_node(&call, &by_path_name, &by_fqn),
            CalleeNodeMatch::None
        );
    }

    #[test]
    fn literal_sibling_imports_resolve_only_exact_inventory_paths() {
        let cases = [
            ("src/caller.py", ".sibling.helper", "src/sibling.py"),
            ("src/caller.ts", "./sibling.helper", "src/sibling.ts"),
            ("src/caller.js", "./sibling.helper", "src/sibling.js"),
            ("src/caller.js", "./sibling.js.helper", "src/sibling.js"),
            (
                "crates/example/src/caller.rs",
                "crate::sibling::helper",
                "crates/example/src/sibling.rs",
            ),
            (
                "crates/example/src/feature/caller.rs",
                "super::sibling::helper",
                "crates/example/src/feature/sibling.rs",
            ),
            (
                "crates/example/src/feature/mod.rs",
                "self::sibling::helper",
                "crates/example/src/feature/sibling.rs",
            ),
        ];
        for (caller, target, sibling) in cases {
            let call = ResolvedCall {
                file: caller.into(),
                start_byte: 10,
                end_byte: 16,
                syntactic_callee: "helper".into(),
                owner: None,
                occurrence_fqn: None,
                resolved_fqn: Some(target.into()),
                confidence: ResolveConfidence::ImportLocal,
            };
            let by_path_name = HashMap::from([(
                (sibling.into(), "helper".into()),
                vec!["sibling-helper".into()],
            )]);
            assert_eq!(
                match_node(&call, &by_path_name, &HashMap::new()),
                CalleeNodeMatch::One("sibling-helper".into()),
                "{caller} -> {target}"
            );
        }
    }

    #[test]
    fn sibling_extension_ambiguity_remains_explicit() {
        let call = ResolvedCall {
            file: "src/caller.ts".into(),
            start_byte: 10,
            end_byte: 16,
            syntactic_callee: "helper".into(),
            owner: None,
            occurrence_fqn: None,
            resolved_fqn: Some("./sibling.helper".into()),
            confidence: ResolveConfidence::ImportLocal,
        };
        let by_path_name = HashMap::from([
            (
                ("src/sibling.ts".into(), "helper".into()),
                vec!["ts".into()],
            ),
            (
                ("src/sibling.js".into(), "helper".into()),
                vec!["js".into()],
            ),
        ]);
        assert_eq!(
            match_node(&call, &by_path_name, &HashMap::new()),
            CalleeNodeMatch::Ambiguous(vec!["js".into(), "ts".into()])
        );
    }

    #[test]
    fn resolved_edge_does_not_remove_call_stub() {
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
                call_sites: vec![CallSiteMarker {
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
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::CallStub));
        assert!(graph.edges.iter().any(|e| e.resolved));
        assert!(graph
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && !e.resolved));
        // structural Structural claim on durable Calls RelationEdgeId after resolve.
        assert!(
            !graph.relation_edges.is_empty(),
            "expected durable relation_edges from structural resolve"
        );
        assert!(graph.relation_claims.iter().any(|c| matches!(
            &c.value,
            entmoot_contracts::RelationClaimValue::ResolutionState {
                state: entmoot_contracts::ResolutionState::Structural
            }
        )));
        assert!(graph
            .edges
            .iter()
            .any(|e| e.resolved && e.relation_edge_id.is_some()));
    }
}
