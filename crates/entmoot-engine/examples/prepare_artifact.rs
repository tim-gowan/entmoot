//! Evaluation helper: build a manifest with the authoritative Rust identity API.
use entmoot_contracts::{
    ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness, ScopeUniverse,
};
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    graph: PathBuf,
    artifact: PathBuf,
    format: ArtifactFormat,
    producer: String,
    version: String,
    included_paths: Vec<String>,
    #[serde(default)]
    config_digest: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request: Request = serde_json::from_slice(&fs::read(
        std::env::args()
            .nth(1)
            .ok_or("usage: prepare_artifact request.json")?,
    )?)?;
    let graph = entmoot_engine::load_graph_json(&request.graph)?;
    graph.validate()?;
    let provisional = ScopeUniverse::from_selectors(
        "evidence-exercise",
        vec![],
        request.included_paths,
        vec![],
        ScopeCompleteness::Partial,
        None,
    );
    let paths = graph
        .snapshot_files
        .iter()
        .filter_map(|f| {
            provisional
                .path_in_scope(&f.path)
                .ok()
                .filter(|yes| *yes)
                .map(|_| f.path.clone())
        })
        .collect::<Vec<_>>();
    let scope = ScopeUniverse::from_selectors(
        provisional.profile,
        vec![],
        provisional.included_paths,
        vec![],
        ScopeCompleteness::CompleteRelativeToUniverse,
        Some(&paths),
    );
    let mut manifest = ArtifactManifest::build(
        request.format,
        ArtifactProducer {
            name: request.producer,
            version: Some(request.version),
            information_uri: None,
            ruleset_id: Some("entmoot.evidence-exercise.v1".into()),
            config_class: Some("entmoot.evidence-exercise.v1".into()),
        },
        graph.snapshot,
        scope,
        entmoot_contracts::blake3_hex(&fs::read(&request.artifact)?),
        request.config_digest,
    )?;
    manifest.artifact_path = Some(
        request
            .artifact
            .file_name()
            .ok_or("artifact requires a filename")?
            .to_string_lossy()
            .into_owned(),
    );
    let output = request.artifact.with_file_name(format!(
        "{}.manifest.json",
        request
            .artifact
            .file_name()
            .ok_or("artifact filename")?
            .to_string_lossy()
    ));
    fs::write(&output, serde_json::to_vec_pretty(&manifest)?)?;
    println!("{}", output.display());
    Ok(())
}
