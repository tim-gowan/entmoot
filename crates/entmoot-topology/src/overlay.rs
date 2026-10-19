//! Inventory-only cross-publication symbol overlay (no package_api).

use std::collections::BTreeMap;

use entmoot_contracts::query::{
    InventoryOverlayDelta, InventoryOverlayReport, InventoryOverlayRequest, OverlayChangeKind,
    INVENTORY_OVERLAY_SCHEMA,
};
use thiserror::Error;

use crate::{NodeKind, TopologyGraph};

/// Failure to build an inventory overlay.
#[derive(Debug, Error)]
pub enum OverlayError {
    /// One input graph violates its own contract.
    #[error("{side} graph is invalid: {message}")]
    InvalidGraph {
        /// Input side (`base` or `head`).
        side: &'static str,
        /// Validation failure.
        message: String,
    },
}

#[derive(Clone)]
struct InventorySymbol {
    name: String,
    path: Option<String>,
    language: Option<String>,
    logic_address: Option<String>,
}

fn language_from_logic_address(logic_address: &str) -> Option<String> {
    let parts: Vec<&str> = logic_address.split('|').collect();
    if parts.len() >= 3 && parts[0] == "logic" {
        Some(parts[2].to_owned())
    } else {
        None
    }
}

fn callable_name(node: &crate::TopologyNode) -> Option<String> {
    if let Some(address) = node.logic_address.as_deref() {
        let parts: Vec<&str> = address.split('|').collect();
        if parts.len() >= 6 && parts[0] == "logic" && parts[5] != "-" && !parts[5].is_empty() {
            return Some(parts[5].to_owned());
        }
    }
    if !node.label.is_empty() {
        return Some(node.label.clone());
    }
    None
}

fn symbol_key(node: &crate::TopologyNode) -> Option<String> {
    let name = callable_name(node)?;
    let path = node.path.as_deref().unwrap_or("");
    let lang = node
        .logic_address
        .as_deref()
        .and_then(language_from_logic_address)
        .unwrap_or_default();
    let occurrence = node
        .logic_address
        .as_deref()
        .and_then(crate::impact::relative_occurrence_key)
        .unwrap_or_else(|| {
            format!(
                "{lang}|{path}|{}|{name}|{}",
                node.scope.join("/"),
                node.logic_kind.as_deref().unwrap_or("")
            )
        });
    Some(format!(
        "{occurrence}|{}",
        node.signature_hash.as_deref().unwrap_or("")
    ))
}

fn inventory_map(graph: &TopologyGraph) -> BTreeMap<String, Vec<InventorySymbol>> {
    let mut inventory = BTreeMap::<String, Vec<InventorySymbol>>::new();
    for (key, symbol) in graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Callable)
        .filter_map(|node| {
            let key = symbol_key(node)?;
            Some((
                key,
                InventorySymbol {
                    name: callable_name(node).unwrap_or_default(),
                    path: node.path.clone(),
                    language: node
                        .logic_address
                        .as_deref()
                        .and_then(language_from_logic_address),
                    logic_address: node.logic_address.clone(),
                },
            ))
        })
    {
        inventory.entry(key).or_default().push(symbol);
    }
    inventory
}

fn passes_filters(symbol: &InventorySymbol, request: &InventoryOverlayRequest) -> bool {
    if !request.name_filters.is_empty() {
        let ok = request
            .name_filters
            .iter()
            .any(|filter| symbol.name.contains(filter));
        if !ok {
            return false;
        }
    }
    if !request.path_prefixes.is_empty() {
        let path = symbol.path.as_deref().unwrap_or("");
        let ok = request
            .path_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix));
        if !ok {
            return false;
        }
    }
    true
}

/// Compare callable/symbol presence across two publications (inventory only).
///
/// Not Bound/Fits, not SemVer. Missing inventory evidence does not prove absence.
pub fn compare_inventory_overlay(
    base: &TopologyGraph,
    head: &TopologyGraph,
    request: &InventoryOverlayRequest,
) -> Result<InventoryOverlayReport, OverlayError> {
    base.validate()
        .map_err(|error| OverlayError::InvalidGraph {
            side: "base",
            message: error.to_string(),
        })?;
    head.validate()
        .map_err(|error| OverlayError::InvalidGraph {
            side: "head",
            message: error.to_string(),
        })?;
    Ok(compare_inventory_overlay_unchecked(base, head, request))
}

fn compare_inventory_overlay_unchecked(
    base: &TopologyGraph,
    head: &TopologyGraph,
    request: &InventoryOverlayRequest,
) -> InventoryOverlayReport {
    let base_map = inventory_map(base);
    let head_map = inventory_map(head);
    let mut deltas = Vec::new();
    let keys: std::collections::BTreeSet<_> =
        base_map.keys().chain(head_map.keys()).cloned().collect();
    for key in keys {
        let left_group = base_map.get(&key).map_or(&[][..], Vec::as_slice);
        let right_group = head_map.get(&key).map_or(&[][..], Vec::as_slice);
        for index in 0..left_group.len().max(right_group.len()) {
            let left = left_group.get(index);
            let right = right_group.get(index);
            let Some(sample) = right.or(left) else {
                continue;
            };
            if !passes_filters(sample, request) {
                continue;
            }
            let change = match (left, right) {
                (None, Some(_)) => OverlayChangeKind::Introduced,
                (Some(_), None) => OverlayChangeKind::Removed,
                (Some(_), Some(_)) => OverlayChangeKind::Retained,
                (None, None) => continue,
            };
            deltas.push(InventoryOverlayDelta {
                name: sample.name.clone(),
                path: sample.path.clone(),
                language: sample.language.clone(),
                change,
                logic_address: sample.logic_address.clone(),
            });
        }
    }
    deltas.sort_by(|a, b| {
        (&a.name, a.path.as_deref(), &a.change).cmp(&(&b.name, b.path.as_deref(), &b.change))
    });
    let introduced = deltas
        .iter()
        .filter(|d| d.change == OverlayChangeKind::Introduced)
        .count();
    let removed = deltas
        .iter()
        .filter(|d| d.change == OverlayChangeKind::Removed)
        .count();
    let retained = deltas
        .iter()
        .filter(|d| d.change == OverlayChangeKind::Retained)
        .count();

    InventoryOverlayReport {
        schema: INVENTORY_OVERLAY_SCHEMA.into(),
        base_snapshot: base.snapshot.clone(),
        head_snapshot: head.snapshot.clone(),
        deltas,
        introduced,
        removed,
        retained,
        limitations: vec![
            "inventory_presence_only; not Bound/Fits and not SemVer".into(),
            "missing_evidence_is_not_absence".into(),
            "scope_kind_signature_presence; not body or API compatibility".into(),
            "identical_declarations_preserve_multiplicity; individual_correspondence_not_established".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{SourceSnapshot, SourceState};
    use entmoot_core::{build_file_index, StructuralHasher};

    use crate::extractor::MarkerTopologyExtractor;
    use crate::graph::file_node_id;

    fn graph(source: &str, revision: char) -> TopologyGraph {
        use crate::graph::SnapshotFileDigest;
        let repository_id = "example/repo";
        let result = StructuralHasher::new()
            .compute_granular_hash_with_analysis_at(
                source,
                "python",
                Some("app.py"),
                Some(repository_id),
                None,
            )
            .unwrap();
        let snapshot = SourceSnapshot {
            repository_id: repository_id.into(),
            revision: revision.to_string().repeat(40),
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
        let mut extractor = MarkerTopologyExtractor::with_snapshot(checkout, snapshot);
        extractor.on_repo_begin(checkout);
        extractor.on_file("app.py", &build_file_index(&result), Some(source));
        extractor.on_file_end(&file_node_id("app.py"));
        let mut graph = extractor.finish();
        graph.snapshot_files = vec![SnapshotFileDigest {
            path: "app.py".into(),
            content_digest: format!("blake3:{}", revision.to_string().repeat(64)),
        }]
        .into();
        graph
    }

    #[test]
    fn scoped_methods_and_identical_declarations_keep_their_multiplicity() {
        let base = graph(
            "class A:\n    def run(self): pass\nclass B:\n    def run(self): pass\n",
            'a',
        );
        let head = graph("class A:\n    def run(self): pass\n", 'b');
        let request = InventoryOverlayRequest {
            base_publication: "base".into(),
            head_publication: "head".into(),
            name_filters: vec![],
            path_prefixes: vec![],
        };
        let report = compare_inventory_overlay_unchecked(&base, &head, &request);
        assert_eq!(
            (report.introduced, report.removed, report.retained),
            (0, 1, 1)
        );
        assert!(report
            .deltas
            .iter()
            .any(|d| d.change == OverlayChangeKind::Removed
                && d.logic_address
                    .as_deref()
                    .is_some_and(|s| s.contains("|B|run|"))));
        let duplicate = graph("def run(): pass\ndef run(): pass\n", 'a');
        let single = graph("def run(): pass\n", 'b');
        let report = compare_inventory_overlay_unchecked(&duplicate, &single, &request);
        assert_eq!((report.removed, report.retained), (1, 1));
    }

    #[test]
    fn overlay_reports_introduced_and_removed_symbols() {
        let base = graph("def model_dump():\n    pass\n", 'a');
        let head = graph(
            "def model_dump():\n    pass\ndef model_validate():\n    pass\n",
            'b',
        );
        // Inventory overlay is presence-only; skip full topology validation so
        // fixture graphs need not satisfy code-contract join invariants.
        let report = compare_inventory_overlay_unchecked(
            &base,
            &head,
            &InventoryOverlayRequest {
                base_publication: "base".into(),
                head_publication: "head".into(),
                name_filters: Vec::new(),
                path_prefixes: Vec::new(),
            },
        );
        assert_eq!(report.introduced, 1);
        assert_eq!(report.retained, 1);
        assert!(report
            .deltas
            .iter()
            .any(|d| d.name == "model_validate" && d.change == OverlayChangeKind::Introduced));
    }
    #[test]
    fn anonymous_occurrences_do_not_collapse() {
        let base = graph("a = lambda: 1\nb = lambda: 1\n", 'a');
        let head = graph("a = lambda: 1\n", 'b');
        let report = compare_inventory_overlay_unchecked(
            &base,
            &head,
            &InventoryOverlayRequest {
                base_publication: "base".into(),
                head_publication: "head".into(),
                name_filters: vec![],
                path_prefixes: vec![],
            },
        );
        assert_eq!((report.removed, report.retained), (1, 1));
    }
}
