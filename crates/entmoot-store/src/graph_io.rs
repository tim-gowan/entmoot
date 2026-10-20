//! JSON serialization for topology graphs.

use std::path::Path;

use crate::graph::TopologyGraph;

pub fn write_graph_json(
    graph: &TopologyGraph,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = crate::atomic_write::AtomicGraphWrite::begin(path)?;
    writer.write_graph(graph)?;
    writer.commit()?;
    Ok(())
}

/// Load inventory nodes whose paths equal or nest under the supplied prefixes.
/// Empty prefixes fall back to a complete inventory projection. Store locators
/// and SQLite publications page from membership indexes; JSON exports filter
/// after a full deserialize.
pub fn load_inventory_for_paths(
    path: &Path,
    path_prefixes: &[String],
) -> Result<TopologyGraph, Box<dyn std::error::Error>> {
    if path_prefixes.is_empty() {
        return load_graph_json(path);
    }
    let is_store = crate::KnowledgeViewLocator::read(path)?.is_some()
        || path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sqlite3"));
    if !is_store {
        let mut graph = load_graph_json(path)?;
        graph.nodes.retain(|node| {
            let Some(node_path) = node.path.as_deref() else {
                return false;
            };
            path_prefixes.iter().any(|prefix| {
                node_path == prefix.as_str() || node_path.starts_with(&(prefix.clone() + "/"))
            })
        });
        graph.snapshot_files = graph
            .snapshot_files
            .iter()
            .filter(|file| {
                path_prefixes.iter().any(|prefix| {
                    file.path == *prefix || file.path.starts_with(&(prefix.clone() + "/"))
                })
            })
            .cloned()
            .collect::<Vec<_>>()
            .into();
        graph.edges.clear();
        for node in &mut graph.nodes {
            node.parent = None;
        }
        graph.rebuild_indexes();
        return Ok(graph);
    }
    let repository = crate::SqliteKnowledgeRepository::open_read_snapshot(path)?;
    let metadata = repository
        .inventory_metadata()?
        .ok_or("publication has no retained inventory")?;
    let mut graph = TopologyGraph::new(metadata.root);
    graph.snapshot = metadata.snapshot;
    graph.policy_ids = metadata.policy_ids;
    // Keep only selected-path source digests so validation matches the scoped
    // node page; missing unselected files are not absence of inventory.
    graph.snapshot_files = metadata
        .snapshot_files
        .into_iter()
        .filter(|file| {
            path_prefixes.iter().any(|prefix| {
                file.path == *prefix || file.path.starts_with(&(prefix.clone() + "/"))
            })
        })
        .collect::<Vec<_>>()
        .into();
    let mut after = None;
    loop {
        let page = repository
            .inventory_nodes_under_prefixes(path_prefixes, after, 256)?
            .ok_or("publication has no retained inventory")?;
        graph.nodes.extend(page.nodes);
        after = page.next_ordinal;
        if after.is_none() {
            break;
        }
    }
    // Path pages omit off-path parents. Overlay compares callable presence only,
    // so drop containment links rather than pretending the chain is complete.
    for node in &mut graph.nodes {
        node.parent = None;
    }
    graph.rebuild_indexes();
    Ok(graph)
}

/// Load an explicit graph export or project a pinned store locator on demand.
pub fn load_graph_json(path: &Path) -> Result<TopologyGraph, Box<dyn std::error::Error>> {
    if crate::KnowledgeViewLocator::read(path)?.is_some() {
        return crate::SqliteKnowledgeRepository::open_read_snapshot(path)?
            .load_inventory(false)?
            .ok_or_else(|| "publication has no retained inventory".into());
    }
    let file = std::fs::File::open(path)?;
    let mut graph: TopologyGraph = match serde_json::from_reader(std::io::BufReader::new(file)) {
        Ok(graph) => graph,
        Err(error) => {
            #[derive(serde::Deserialize)]
            struct GraphHeader {
                schema_version: Option<String>,
            }
            let file = std::fs::File::open(path)?;
            let header: Result<GraphHeader, _> =
                serde_json::from_reader(std::io::BufReader::new(file));
            if matches!(
                header,
                Ok(GraphHeader {
                    schema_version: Some(ref version)
                }) if version == "topology.graph.v1"
            ) {
                return Err(
                        "obsolete topology.graph.v1 contract; regenerate with `entmoot-cli index` (expect logic.graph.v1)"
                            .into(),
                    );
            }
            return Err(error.into());
        }
    };
    graph.rebuild_indexes();
    graph.validate()?;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_db::KnowledgeRepository;

    #[test]
    fn rejects_obsolete_topology_graph_v1_with_regeneration_guidance() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("graph.json");
        std::fs::write(&path, r#"{"schema_version":"topology.graph.v1"}"#).unwrap();
        let error = load_graph_json(&path).unwrap_err().to_string();
        assert!(error.contains("regenerate"));
        assert!(error.contains("logic.graph.v1"));
    }

    #[test]
    fn atomic_writer_replaces_an_existing_graph() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("graph.json");
        std::fs::write(&path, "old bytes").unwrap();
        let graph = TopologyGraph::new(temp.path().to_string_lossy());
        write_graph_json(&graph, &path).unwrap();
        let loaded = load_graph_json(&path).unwrap();
        assert_eq!(loaded.schema_version, crate::TOPOLOGY_SCHEMA_VERSION);
    }

    #[test]
    fn atomic_writer_creates_nested_output_parent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/output/graph.json");
        let graph = TopologyGraph::new(temp.path().to_string_lossy());
        write_graph_json(&graph, &path).unwrap();
        assert!(path.is_file());
        assert_eq!(
            load_graph_json(&path).unwrap().schema_version,
            crate::TOPOLOGY_SCHEMA_VERSION
        );
    }

    #[test]
    fn graph_file_uses_compact_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("graph.json");
        let graph = TopologyGraph::new(temp.path().to_string_lossy());
        write_graph_json(&graph, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert!(!bytes.windows(3).any(|window| window == b"\n  "));
    }

    #[test]
    fn path_scoped_inventory_load_keeps_selected_callables_only() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("keep.py"), "def keep():\n    return 1\n").unwrap();
        std::fs::write(source.path().join("drop.py"), "def drop():\n    return 2\n").unwrap();
        let graph = crate::build_topology_from_repo_with_options(source.path(), Default::default())
            .unwrap();
        let database = data.path().join("knowledge.sqlite3");
        let mut repository = crate::SqliteKnowledgeRepository::open(&database).unwrap();
        repository
            .replace_all(&crate::KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        drop(repository);
        let scoped = load_inventory_for_paths(&database, &["keep.py".into()]).unwrap();
        let labels: Vec<_> = scoped
            .nodes
            .iter()
            .filter(|node| node.kind == crate::NodeKind::Callable)
            .map(|node| node.label.as_str())
            .collect();
        assert!(labels.contains(&"keep"));
        assert!(!labels.contains(&"drop"));
        let full = crate::SqliteKnowledgeRepository::open_read_snapshot(&database)
            .unwrap()
            .load_inventory(false)
            .unwrap()
            .unwrap();
        assert!(full.nodes.len() > scoped.nodes.len());
    }

    #[test]
    fn path_scoped_overlay_matches_full_load_under_the_same_prefix() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("keep.py"), "def keep():\n    return 1\n").unwrap();
        std::fs::write(source.path().join("drop.py"), "def drop():\n    return 2\n").unwrap();
        let base_graph =
            crate::build_topology_from_repo_with_options(source.path(), Default::default())
                .unwrap();
        std::fs::write(
            source.path().join("keep.py"),
            "def keep():\n    return 1\ndef keep_added():\n    return 3\n",
        )
        .unwrap();
        std::fs::write(source.path().join("drop.py"), "def drop():\n    return 9\n").unwrap();
        let head_graph =
            crate::build_topology_from_repo_with_options(source.path(), Default::default())
                .unwrap();

        let store = |graph: &crate::TopologyGraph| {
            let data = tempfile::tempdir().unwrap();
            let database = data.path().join("knowledge.sqlite3");
            let mut repository = crate::SqliteKnowledgeRepository::open(&database).unwrap();
            repository
                .replace_all(&crate::KnowledgeStore::extract_from_graph(graph))
                .unwrap();
            repository.store_inventory(graph, None).unwrap();
            drop(repository);
            (data, database)
        };
        let (_base_data, base_db) = store(&base_graph);
        let (_head_data, head_db) = store(&head_graph);
        let prefixes = vec!["keep.py".into()];
        let request = entmoot_contracts::query::InventoryOverlayRequest {
            base_publication: "base".into(),
            head_publication: "head".into(),
            name_filters: Vec::new(),
            path_prefixes: prefixes.clone(),
        };
        let full_base = crate::SqliteKnowledgeRepository::open_read_snapshot(&base_db)
            .unwrap()
            .load_inventory(false)
            .unwrap()
            .unwrap();
        let full_head = crate::SqliteKnowledgeRepository::open_read_snapshot(&head_db)
            .unwrap()
            .load_inventory(false)
            .unwrap()
            .unwrap();
        let full = crate::compare_inventory_overlay(&full_base, &full_head, &request).unwrap();
        let scoped = crate::compare_inventory_overlay(
            &load_inventory_for_paths(&base_db, &prefixes).unwrap(),
            &load_inventory_for_paths(&head_db, &prefixes).unwrap(),
            &request,
        )
        .unwrap();
        assert_eq!(
            (scoped.introduced, scoped.removed, scoped.retained),
            (full.introduced, full.removed, full.retained)
        );
        assert_eq!(
            serde_json::to_value(&scoped.deltas).unwrap(),
            serde_json::to_value(&full.deltas).unwrap()
        );
        assert!(
            scoped.deltas.iter().all(|delta| {
                delta
                    .path
                    .as_deref()
                    .is_some_and(|path| path == "keep.py" || path.starts_with("keep.py/"))
            }),
            "path filter must exclude drop.py symbols"
        );
        assert!(
            scoped.introduced >= 1,
            "keep_added must appear under keep.py"
        );
    }
}
