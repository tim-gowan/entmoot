//! Compiler input owns source commitments and exact structural sites. Relations,
//! claims, display strings, syntax trees and retained artifacts are not inputs to
//! a resolver. The compiler still chooses its ecosystem-sized analysis session.
use std::collections::HashSet;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize, Serializer as _};

use super::pipeline::NativeProducerError;
use crate::{NodeKind, SnapshotFileDigest, TopologyGraph, TopologyNode};

const SCHEMA: &str = "entmoot.native_input.v1";

/// Minimal structural join context, with identities copied unchanged from syntax.
/// This internal transport is fingerprint-qualified and never a query graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeInventory {
    schema_version: String,
    pub implementation: entmoot_contracts::ImplementationFingerprint,
    pub snapshot: entmoot_contracts::SourceSnapshot,
    pub snapshot_files: Vec<SnapshotFileDigest>,
    pub nodes: Vec<NativeSite>,
}

/// Only the fields consumed by exact-span attachment and compiler site selection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSite {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub path: Option<String>,
    pub logic_address: Option<String>,
    pub parent: Option<String>,
    pub start_byte: Option<usize>,
    pub end_byte: Option<usize>,
}

impl From<&TopologyNode> for NativeSite {
    fn from(node: &TopologyNode) -> Self {
        Self {
            id: node.id.clone(),
            kind: node.kind,
            label: node.label.clone(),
            path: node.path.clone(),
            logic_address: node.logic_address.clone(),
            parent: node.parent.clone(),
            start_byte: node.start_byte,
            end_byte: node.end_byte,
        }
    }
}

fn is_site(node: &TopologyNode) -> bool {
    matches!(
        node.kind,
        NodeKind::Callable | NodeKind::Declaration | NodeKind::CallStub
    )
}

impl From<&TopologyGraph> for NativeInventory {
    fn from(graph: &TopologyGraph) -> Self {
        Self {
            schema_version: SCHEMA.into(),
            implementation: graph.implementation.clone(),
            snapshot: graph.snapshot.clone(),
            snapshot_files: graph.snapshot_files.to_vec(),
            nodes: graph
                .nodes
                .iter()
                .filter(|node| is_site(node))
                .map(NativeSite::from)
                .collect(),
        }
    }
}

impl NativeInventory {
    /// Open the compact input produced by init/bind. Standalone commands still
    /// accept explicit graph exports, validated before their one-time projection.
    pub fn load(path: &Path) -> Result<Self, NativeProducerError> {
        let input = if path.to_string_lossy().ends_with(".native.json") {
            serde_json::from_reader(BufReader::new(std::fs::File::open(path)?))?
        } else {
            let graph = crate::load_graph_json(path)
                .map_err(|error| NativeProducerError::Graph(error.to_string()))?;
            Self::from(&graph)
        };
        input.validate()?;
        Ok(input)
    }

    pub fn validate(&self) -> Result<(), NativeProducerError> {
        let invalid = |message: &str| NativeProducerError::Graph(message.into());
        if self.schema_version != SCHEMA
            || self.implementation != crate::implementation_fingerprint()
        {
            return Err(invalid(
                "incompatible native input; regenerate from structural inventory",
            ));
        }
        self.snapshot
            .validate()
            .map_err(|error| NativeProducerError::Graph(error.to_string()))?;
        let mut paths = HashSet::new();
        for file in &self.snapshot_files {
            if !paths.insert(file.path.as_str())
                || !entmoot_contracts::normalize_repo_path(&file.path)
                    .is_ok_and(|path| path == file.path)
            {
                return Err(invalid("invalid or duplicate native input source path"));
            }
            entmoot_contracts::ContentDigest::parse(file.content_digest.clone())
                .map_err(|error| NativeProducerError::Graph(error.to_string()))?;
        }
        let mut ids = HashSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id.as_str())
                || node.id.is_empty()
                || !matches!(
                    node.kind,
                    NodeKind::Callable | NodeKind::Declaration | NodeKind::CallStub
                )
                || !node
                    .path
                    .as_deref()
                    .is_some_and(|path| paths.contains(path))
                || node
                    .start_byte
                    .zip(node.end_byte)
                    .is_none_or(|(start, end)| start > end)
            {
                return Err(invalid("invalid native input structural site"));
            }
            if let Some(address) = &node.logic_address {
                entmoot_contracts::OccurrenceRef::new(address.clone())
                    .map_err(NativeProducerError::Graph)?;
            }
        }
        for node in self
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::CallStub)
        {
            let file_parent = node.path.as_deref().map(crate::graph::file_node_id);
            if !node.parent.as_deref().is_some_and(|parent| {
                ids.contains(parent) || Some(parent) == file_parent.as_deref()
            }) {
                return Err(invalid("native call site has no inventoried owner"));
            }
        }
        Ok(())
    }
}

/// Write one site at a time, without allocating a second repository inventory.
/// The caller supplies a validated structural graph and a private staging path.
pub fn write_native_input(graph: &TopologyGraph, path: &Path) -> Result<(), NativeProducerError> {
    use serde::ser::{SerializeSeq, SerializeStruct};
    struct Sites<'a>(&'a [TopologyNode]);
    impl Serialize for Sites<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut seq = serializer.serialize_seq(None)?;
            for node in self.0.iter().filter(|node| is_site(node)) {
                seq.serialize_element(&NativeSite::from(node))?;
            }
            seq.end()
        }
    }
    graph
        .validate()
        .map_err(|error| NativeProducerError::Graph(error.to_string()))?;
    let mut output = BufWriter::new(std::fs::File::create(path)?);
    let mut serializer = serde_json::Serializer::new(&mut output);
    let mut object = (&mut serializer).serialize_struct("NativeInventory", 5)?;
    object.serialize_field("schema_version", SCHEMA)?;
    object.serialize_field("implementation", &graph.implementation)?;
    object.serialize_field("snapshot", &graph.snapshot)?;
    object.serialize_field("snapshot_files", &graph.snapshot_files)?;
    object.serialize_field("nodes", &Sites(&graph.nodes))?;
    SerializeStruct::end(object)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_input_preserves_structural_join_sites_and_rejects_corruption() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("a.py"),
            "def leaf():\n    pass\ndef caller():\n    leaf()\n",
        )
        .unwrap();
        let graph =
            crate::build_topology_from_repo_with_options(temp.path(), Default::default()).unwrap();
        let path = temp.path().join("input.native.json");
        write_native_input(&graph, &path).unwrap();
        let mut input = NativeInventory::load(&path).unwrap();
        assert_eq!(input.nodes.len(), 3);
        assert_eq!(input.snapshot, graph.snapshot);
        assert_eq!(input.snapshot_files, *graph.snapshot_files);
        let call = input
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::CallStub)
            .unwrap();
        assert_eq!(call.label, "leaf");
        let original = graph.nodes.iter().find(|node| node.id == call.id).unwrap();
        assert_eq!(
            (call.start_byte, call.end_byte, &call.parent),
            (original.start_byte, original.end_byte, &original.parent)
        );
        let encoded: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(encoded.get("relation_edges").is_none());
        assert!(encoded["nodes"][0].get("call_site").is_none());
        assert!(encoded["nodes"][0].get("display_ref").is_none());
        input.nodes.push(input.nodes[0].clone());
        assert!(input.validate().is_err());
        input.nodes.pop();
        input.nodes[0].path = Some("../outside.py".into());
        assert!(input.validate().is_err());
        input.nodes[0].path = Some("a.py".into());
        input.snapshot_files[0].content_digest = "blake3:bad".into();
        assert!(input.validate().is_err());
    }
}
