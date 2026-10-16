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

pub fn load_graph_json(path: &Path) -> Result<TopologyGraph, Box<dyn std::error::Error>> {
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
}
