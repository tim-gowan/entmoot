//! Retain a qualified publication in a campaign's multi-view production store.
//! Uses repository publication APIs; evaluation grades never enter this store.
use entmoot_engine::{
    FilesystemArtifactObjectRepository, GixSourceStore, KnowledgeRepository, KnowledgeStoreBatches,
    KnowledgeViewLocator, PublicationBuffer, SqliteKnowledgeRepository,
};
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    graph: PathBuf,
    source_store: PathBuf,
    database: PathBuf,
    locator: PathBuf,
}

fn retain(request: &Request) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let graph = entmoot_engine::load_graph_json(&request.graph)?;
    graph.validate()?;
    let source = SqliteKnowledgeRepository::open_read_only(&request.source_store)?;
    if source.inventory_export_digest()?.is_none() {
        return Err("retention requires a qualified inventory export".into());
    }
    source.verify_inventory_export(&request.graph)?;
    let knowledge = source.load()?;
    let source_objects = FilesystemArtifactObjectRepository::new(
        entmoot_engine::workspace::artifact_store_path(&request.source_store)?,
    );
    let destination_objects = FilesystemArtifactObjectRepository::new(
        entmoot_engine::workspace::artifact_store_path(&request.database)?,
    );
    let mut unavailable_objects = Vec::new();
    for artifact in &knowledge.artifacts {
        let digest = entmoot_contracts::ContentDigest::parse(artifact.content_digest.clone())?;
        match source_objects.get(&digest) {
            Ok(bytes) => {
                destination_objects.put(&digest, &bytes)?;
            }
            Err(entmoot_engine::ArtifactStoreError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                unavailable_objects.push(artifact.artifact_id.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }
    let view = entmoot_contracts::blake3_hex(&serde_json::to_vec(&graph)?);
    let mut destination = SqliteKnowledgeRepository::open_view(&request.database, &view)?;
    let buffer = PublicationBuffer::default();
    let mut batches = KnowledgeStoreBatches::new(knowledge, buffer)?;
    let receipt = destination.replace_owner_buffered(
        &format!("snapshot:{}", graph.snapshot.repository_id),
        &mut batches,
        buffer,
    )?;
    let source_files = GixSourceStore::open(std::path::Path::new(&graph.root))?;
    destination.store_inventory(&graph, Some(&source_files))?;
    drop(destination);
    // Expose the locator only after knowledge and inventory have committed.
    let locator = KnowledgeViewLocator {
        database: request.database.clone(),
        view,
    };
    entmoot_engine::workspace::write_json(&request.locator, &locator)?;
    Ok(serde_json::json!({
        "locator": request.locator, "publication": locator,
        "snapshot": graph.snapshot, "nodes": graph.nodes.len(),
        "source_files": graph.snapshot_files.len(),
        "outcome": receipt.outcome.as_str(), "unavailable_raw_artifacts": unavailable_objects,
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request: Request = serde_json::from_slice(&fs::read(
        std::env::args()
            .nth(1)
            .ok_or("usage: retain_campaign_publication request.json")?,
    )?)?;
    println!("{}", retain(&request)?);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use entmoot_engine::{KnowledgeStore, TopologyGraph};

    #[test]
    fn refuses_a_store_without_qualified_inventory() {
        let temp = tempfile::tempdir().unwrap();
        let mut graph = TopologyGraph::new(temp.path().to_string_lossy().into_owned());
        graph.snapshot.repository_id = "fixture".into();
        graph.snapshot.revision = "a".repeat(40);
        graph.validate().unwrap();
        let graph_path = temp.path().join("graph.json");
        entmoot_engine::write_graph_json(&graph, &graph_path).unwrap();
        let source_path = temp.path().join("source.sqlite3");
        let mut source = SqliteKnowledgeRepository::open(&source_path).unwrap();
        source
            .synchronize(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        drop(source);
        assert!(retain(&Request {
            graph: graph_path,
            source_store: source_path,
            database: temp.path().join("retained.sqlite3"),
            locator: temp.path().join("retained.view.json"),
        })
        .is_err());
    }

    #[test]
    fn retaining_two_revisions_preserves_both_views_and_rejects_mismatched_export() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("all.sqlite3");
        let mut locators = Vec::new();
        let revisions = ["a".repeat(40), "b".repeat(40)];
        for revision in &revisions {
            let mut graph = TopologyGraph::new(temp.path().to_string_lossy().into_owned());
            graph.snapshot.repository_id = "fixture".into();
            graph.snapshot.revision = revision.clone();
            let graph_path = temp.path().join(format!("{revision}.json"));
            entmoot_engine::write_graph_json(&graph, &graph_path).unwrap();
            let source_path = temp.path().join(format!("{revision}.sqlite3"));
            let mut source = SqliteKnowledgeRepository::open(&source_path).unwrap();
            source
                .synchronize(&KnowledgeStore::extract_from_graph(&graph))
                .unwrap();
            source.store_inventory(&graph, None).unwrap();
            drop(source);
            let request = Request {
                graph: graph_path.clone(),
                source_store: source_path,
                database: database.clone(),
                locator: temp.path().join(format!("{revision}.view.json")),
            };
            retain(&request).unwrap();
            retain(&request).unwrap();
            locators.push(request.locator.clone());
            graph.snapshot.revision = "c".repeat(40);
            entmoot_engine::write_graph_json(&graph, &graph_path).unwrap();
            assert!(retain(&request).is_err());
        }
        for (locator, expected) in locators.iter().zip(revisions) {
            let store = SqliteKnowledgeRepository::open_read_only(locator).unwrap();
            assert_eq!(
                store
                    .load_inventory(false)
                    .unwrap()
                    .unwrap()
                    .snapshot
                    .revision,
                expected
            );
        }
    }
}
