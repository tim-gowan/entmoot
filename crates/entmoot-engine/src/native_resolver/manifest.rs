//! Build matched resolver artifact manifests for attach.

use std::fs;
use std::path::Path;

use entmoot_contracts::{
    blake3_hex, ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness,
    ScopeUniverse,
};
use thiserror::Error;

use crate::export::load_graph_json;
use crate::graph::TopologyGraph;

#[derive(Debug, Error)]
pub enum ManifestPrepError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("graph validation failed: {0}")]
    Graph(String),
    #[error("manifest construction failed: {0}")]
    Manifest(String),
}

/// Construct a complete-relative-to-universe resolver manifest for `artifact`.
pub fn prepare_resolver_manifest(
    graph: &TopologyGraph,
    artifact_path: &Path,
    producer_name: &str,
    producer_version: &str,
    language: &str,
    included_paths: &[String],
) -> Result<ArtifactManifest, ManifestPrepError> {
    let bytes = fs::read(artifact_path)?;
    let provisional = ScopeUniverse::from_selectors(
        "native-resolver",
        vec![],
        included_paths.to_vec(),
        vec![],
        if included_paths.is_empty() {
            ScopeCompleteness::CompleteRelativeToUniverse
        } else {
            ScopeCompleteness::Partial
        },
        None,
    );
    let paths = if included_paths.is_empty() {
        graph
            .snapshot_files
            .iter()
            .filter(|file| crate::native_resolver::join::language_matches(language, &file.path))
            .map(|file| file.path.clone())
            .collect::<Vec<_>>()
    } else {
        graph
            .snapshot_files
            .iter()
            .filter_map(|file| {
                provisional
                    .path_in_scope(&file.path)
                    .ok()
                    .filter(|yes| *yes)
                    .map(|_| file.path.clone())
            })
            .collect()
    };
    let scope = ScopeUniverse::from_selectors(
        provisional.profile,
        vec![],
        if included_paths.is_empty() {
            paths.clone()
        } else {
            provisional.included_paths
        },
        vec![],
        ScopeCompleteness::CompleteRelativeToUniverse,
        Some(&paths),
    );
    let mut manifest = ArtifactManifest::build(
        ArtifactFormat::Resolver,
        ArtifactProducer {
            name: producer_name.to_owned(),
            version: Some(producer_version.to_owned()),
            information_uri: None,
            ruleset_id: Some("entmoot.native_resolver.v1".into()),
            config_class: Some("entmoot.native_resolver.v1".into()),
        },
        graph.snapshot.clone(),
        scope,
        blake3_hex(&bytes),
        None,
    )
    .map_err(|error| ManifestPrepError::Manifest(error.to_string()))?;
    manifest.artifact_path = artifact_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    Ok(manifest)
}

/// Load graph JSON then prepare a resolver manifest.
pub fn prepare_resolver_manifest_from_paths(
    graph_path: &Path,
    artifact_path: &Path,
    producer_name: &str,
    producer_version: &str,
    included_paths: &[String],
) -> Result<ArtifactManifest, ManifestPrepError> {
    let graph = load_graph_json(graph_path).map_err(|error| {
        ManifestPrepError::Graph(format!("failed to load {}: {error}", graph_path.display()))
    })?;
    graph
        .validate()
        .map_err(|error| ManifestPrepError::Graph(error.to_string()))?;
    prepare_resolver_manifest(
        &graph,
        artifact_path,
        producer_name,
        producer_version,
        "rust",
        included_paths,
    )
}
