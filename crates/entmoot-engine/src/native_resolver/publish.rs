//! Immutable publication generations for dogfood / standard Rust publish.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use entmoot_contracts::blake3_hex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::export::{load_graph_json, write_graph_json};
use crate::ingest::ingest_artifact_manifest;
use crate::knowledge::KnowledgeStore;
use crate::knowledge_db::{
    KnowledgeRepository, KnowledgeStoreBatches, PublicationBuffer, SqliteKnowledgeRepository,
};
use crate::native_resolver::budget::ProducerBudget;
use crate::native_resolver::pipeline::{
    produce_rust_resolver, NativeProduceRequest, NativeProducerError,
};
use crate::native_resolver::RA_AP_PIN;
use crate::{build_topology_from_repo_with_options, TopologyBuildOptions};
use entmoot_contracts::ExecutionPolicy;

pub const PUBLICATION_SCHEMA: &str = "entmoot.publication.v1";

#[derive(Debug, Clone)]
pub struct DogfoodPublishRequest {
    pub rust_profile: entmoot_contracts::RustBuildProfile,
    pub source_root: PathBuf,
    pub repository_id: String,
    pub publications_dir: PathBuf,
    pub current_pointer: PathBuf,
    pub budget: ProducerBudget,
    pub execution_policy: ExecutionPolicy,
    pub force: bool,
    pub require_native: bool,
    pub producer_bin_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationManifest {
    pub rust_profile: entmoot_contracts::RustBuildProfile,
    pub schema_version: String,
    pub freshness_key: String,
    pub repository_id: String,
    pub source_root: String,
    pub graph: String,
    pub store: String,
    pub resolver_dir: Option<String>,
    pub snapshot: serde_json::Value,
    pub ra_ap_pin: String,
    pub execution_policy: ExecutionPolicy,
    pub semantic_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DogfoodPublishResult {
    pub generation_dir: PathBuf,
    pub publication: PublicationManifest,
    pub reused: bool,
}

#[derive(Debug, Error)]
pub enum PublicationError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Produce(#[from] NativeProducerError),
    #[error("topology build failed: {0}")]
    Topology(String),
    #[error("attach failed: {0}")]
    Attach(String),
    #[error("publication failed: {0}")]
    Message(String),
}

fn git_head(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let head = text.trim();
    (!head.is_empty()).then(|| head.to_owned())
}

fn freshness_key(
    snapshot_digest: &str,
    producer_digest: Option<&str>,
    execution_policy: ExecutionPolicy,
    rust_profile: &entmoot_contracts::RustBuildProfile,
) -> Result<String, serde_json::Error> {
    let material = format!(
        "ra_ap={RA_AP_PIN}|policy={execution_policy:?}|snapshot={snapshot_digest}|producer={}",
        producer_digest.unwrap_or("unknown")
    );
    let digest = blake3_hex(&serde_json::to_vec(&(material, rust_profile))?);
    Ok(digest
        .strip_prefix("blake3:")
        .unwrap_or(&digest)
        .chars()
        .take(32)
        .collect())
}

fn write_current_pointer(
    path: &Path,
    publication: &PublicationManifest,
) -> Result<(), PublicationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staging = path.with_extension("json.staging");
    fs::write(&staging, serde_json::to_vec_pretty(publication)?)?;
    fs::rename(&staging, path)?;
    Ok(())
}

fn sync_knowledge(
    graph: &crate::graph::TopologyGraph,
    store_path: &Path,
) -> Result<(), PublicationError> {
    let mut repository = SqliteKnowledgeRepository::open(store_path)
        .map_err(|error| PublicationError::Message(error.to_string()))?;
    let buffer = PublicationBuffer::default();
    let mut batches = KnowledgeStoreBatches::new(KnowledgeStore::extract_from_graph(graph), buffer)
        .map_err(|error| PublicationError::Message(error.to_string()))?;
    repository
        .replace_owner_buffered(
            &format!("snapshot:{}", graph.snapshot.repository_id),
            &mut batches,
            buffer,
        )
        .map_err(|error| PublicationError::Message(error.to_string()))?;
    Ok(())
}

fn attach_resolver(
    graph: &mut crate::graph::TopologyGraph,
    store_path: &Path,
    manifest_path: &Path,
) -> Result<(), PublicationError> {
    ingest_artifact_manifest(graph, manifest_path, true)
        .map_err(|error| PublicationError::Attach(error.to_string()))?;
    sync_knowledge(graph, store_path)?;
    Ok(())
}

/// structural index → native produce → attach → immutable generation + current pointer.
pub fn publish_dogfood(
    request: DogfoodPublishRequest,
) -> Result<DogfoodPublishResult, PublicationError> {
    request
        .rust_profile
        .validate()
        .map_err(|error| PublicationError::Message(error.into()))?;
    fs::create_dir_all(&request.publications_dir)?;
    let root = fs::canonicalize(&request.source_root)?;
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some(request.repository_id.as_str()),
        },
    )
    .map_err(|error| PublicationError::Topology(error.to_string()))?;
    let snapshot_digest = graph
        .snapshot
        .snapshot_digest
        .clone()
        .unwrap_or_else(|| format!("rev:{}", graph.snapshot.revision));
    let key = freshness_key(
        &snapshot_digest,
        request.producer_bin_digest.as_deref(),
        request.execution_policy,
        &request.rust_profile,
    )?;
    let generation = request.publications_dir.join(&key);
    let pointer_exists = request.current_pointer.is_file();
    if !request.force && generation.is_dir() && pointer_exists {
        if let Ok(existing) = fs::read_to_string(&request.current_pointer) {
            if let Ok(publication) = serde_json::from_str::<PublicationManifest>(&existing) {
                if publication.freshness_key == key {
                    return Ok(DogfoodPublishResult {
                        generation_dir: generation,
                        publication,
                        reused: true,
                    });
                }
            }
        }
    }

    let staging = tempfile::Builder::new()
        .prefix(&format!("{key}.staging-"))
        .tempdir_in(&request.publications_dir)?;
    let staging_path = staging.path().to_path_buf();
    let graph_path = staging_path.join("graph.json");
    let store_path = PathBuf::from(format!("{}.knowledge.sqlite3", graph_path.display()));
    write_graph_json(&graph, &graph_path)
        .map_err(|error| PublicationError::Topology(error.to_string()))?;
    sync_knowledge(&graph, &store_path)?;

    let resolver_dir = staging_path.join("resolver");
    let produce_request = NativeProduceRequest {
        canon: false,
        graph_path: graph_path.clone(),
        source_root: root.clone(),
        output_dir: resolver_dir.clone(),
        budget: request.budget,
        execution_policy: request.execution_policy,
        rust_profile: request.rust_profile.clone(),
    };

    let mut graph = load_graph_json(&graph_path)
        .map_err(|error| PublicationError::Topology(error.to_string()))?;
    let (semantic_status, limitations) = match produce_rust_resolver(produce_request) {
        Ok(result) => {
            if let Err(error) = attach_resolver(&mut graph, &store_path, &result.manifest_path) {
                if request.require_native {
                    return Err(error);
                }
                let mut limitations = result.receipt.limitations;
                limitations.push(error.to_string());
                (
                    format!("attach_failed:{}", result.receipt.semantic_status),
                    limitations,
                )
            } else {
                write_graph_json(&graph, &graph_path)
                    .map_err(|error| PublicationError::Attach(error.to_string()))?;
                (
                    result.receipt.semantic_status.to_string(),
                    result.receipt.limitations,
                )
            }
        }
        Err(error) => {
            if request.require_native {
                return Err(PublicationError::Produce(error));
            }
            ("structural_only".into(), vec![error.to_string()])
        }
    };

    let publication = PublicationManifest {
        rust_profile: request.rust_profile,
        schema_version: PUBLICATION_SCHEMA.into(),
        freshness_key: key.clone(),
        repository_id: request.repository_id.clone(),
        source_root: root.to_string_lossy().into_owned(),
        graph: "graph.json".into(),
        store: "graph.json.knowledge.sqlite3".into(),
        resolver_dir: resolver_dir.exists().then(|| "resolver".to_owned()),
        snapshot: serde_json::to_value(&graph.snapshot)?,
        ra_ap_pin: RA_AP_PIN.into(),
        execution_policy: request.execution_policy,
        semantic_status,
        limitations,
    };
    fs::write(
        staging_path.join("publication.json"),
        serde_json::to_vec_pretty(&publication)?,
    )?;

    if generation.exists() {
        fs::remove_dir_all(&generation)?;
    }
    let staging_persist = staging.keep();
    fs::rename(&staging_persist, &generation).map_err(|error| {
        let _ = fs::remove_dir_all(&staging_persist);
        PublicationError::Io(error)
    })?;
    write_current_pointer(&request.current_pointer, &publication)?;

    // Compatibility copies for existing MCP configs that point at dogfood.graph.json.
    if let Some(parent) = request.current_pointer.parent() {
        let legacy_graph = parent.join("dogfood.graph.json");
        let legacy_store = PathBuf::from(format!("{}.knowledge.sqlite3", legacy_graph.display()));
        let published_graph = generation.join("graph.json");
        let published_store = generation.join("graph.json.knowledge.sqlite3");
        let _ = fs::remove_file(&legacy_graph);
        let _ = fs::remove_file(&legacy_store);
        fs::copy(&published_graph, &legacy_graph)?;
        if published_store.exists() {
            fs::copy(&published_store, &legacy_store)?;
        }
        if let Some(head) = git_head(&root) {
            let marker = parent.join("dogfood.head");
            let payload = serde_json::json!({
                "head": head,
                "graph": legacy_graph,
                "store": legacy_store,
                "source_root": root,
                "repository_id": request.repository_id,
                "publication": request.current_pointer,
                "freshness_key": key,
                "snapshot": publication.snapshot,
            });
            fs::write(marker, serde_json::to_vec_pretty(&payload)?)?;
        }
    }

    Ok(DogfoodPublishResult {
        generation_dir: generation,
        publication,
        reused: false,
    })
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn publication_identity_includes_requested_rust_profile() {
        let default = entmoot_contracts::RustBuildProfile::default();
        let mut selected = default.clone();
        selected.analysis = entmoot_contracts::RustAnalysisMode::Test {
            crate_root: "src/lib.rs".into(),
        };
        let key = |profile: &entmoot_contracts::RustBuildProfile| {
            freshness_key("snapshot", Some("producer"), ExecutionPolicy::Safe, profile).unwrap()
        };
        assert_ne!(key(&default), key(&selected));
        selected.analysis = entmoot_contracts::RustAnalysisMode::Workspace {};
        selected.features = entmoot_contracts::RustFeatureSelection::All {};
        assert_ne!(key(&default), key(&selected));
        selected = default.clone();
        selected.target_triple = Some("x86_64-unknown-linux-gnu".into());
        assert_ne!(key(&default), key(&selected));
    }
}
