//! Build a snapshot-matched artifact manifest for attach-first dogfood.
//!
//! Usage:
//!   cargo run -p entmoot-engine --example write_matched_artifact_manifest -- \
//!     --graph path/to/topo.json \
//!     --artifact path/to/file.sarif|resources.json|resolver.json \
//!     --format sarif21|resource_inventory|resolver \
//!     --producer-name SnykCode \
//!     --out-dir path/to/out \
//!     [--include path]... [--exclude glob]...

use std::path::PathBuf;

use entmoot_contracts::{
    blake3_hex, ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness,
    ScopeUniverse,
};
use entmoot_engine::TopologyGraph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph_path = None::<PathBuf>;
    let mut artifact_path = None::<PathBuf>;
    let mut out_dir = None::<PathBuf>;
    let mut format = None::<String>;
    let mut producer_name = "fixture".to_string();
    let mut producer_version = None::<String>;
    let mut includes: Vec<String> = Vec::new();
    let mut excludes: Vec<String> = Vec::new();
    let mut languages: Vec<String> = Vec::new();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--graph" => {
                i += 1;
                graph_path = Some(PathBuf::from(&args[i]));
            }
            "--artifact" => {
                i += 1;
                artifact_path = Some(PathBuf::from(&args[i]));
            }
            "--out-dir" => {
                i += 1;
                out_dir = Some(PathBuf::from(&args[i]));
            }
            "--format" => {
                i += 1;
                format = Some(args[i].clone());
            }
            "--producer-name" => {
                i += 1;
                producer_name = args[i].clone();
            }
            "--producer-version" => {
                i += 1;
                producer_version = Some(args[i].clone());
            }
            "--include" => {
                i += 1;
                includes.push(args[i].clone());
            }
            "--exclude" => {
                i += 1;
                excludes.push(args[i].clone());
            }
            "--language" => {
                i += 1;
                languages.push(args[i].clone());
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
        i += 1;
    }

    let graph_path = graph_path.ok_or("--graph required")?;
    let artifact_path = artifact_path.ok_or("--artifact required")?;
    let out_dir = out_dir.ok_or("--out-dir required")?;
    let format = match format.ok_or("--format required")?.as_str() {
        "sarif21" | "sarif" => ArtifactFormat::Sarif21,
        "resource_inventory" => ArtifactFormat::ResourceInventory,
        "resolver" => ArtifactFormat::Resolver,
        "cyclonedx" | "cdx" => ArtifactFormat::CycloneDx,
        "osv_scanner" | "osv" => ArtifactFormat::OsvScanner,
        "semantic_cluster" => ArtifactFormat::SemanticCluster,
        other => return Err(format!("unsupported --format {other}").into()),
    };
    if languages.is_empty() {
        languages = match format {
            ArtifactFormat::Sarif21 => vec!["typescript".into(), "javascript".into()],
            ArtifactFormat::SemanticCluster => vec![],
            ArtifactFormat::ResourceInventory => vec![],
            ArtifactFormat::Resolver => vec!["python".into()],
            ArtifactFormat::CycloneDx => vec![],
            ArtifactFormat::OsvScanner => vec![],
            // This helper deliberately does not accept assessment_evidence:
            // that envelope owns its producer and build context.
            ArtifactFormat::AssessmentEvidence => vec![],
        };
    }

    std::fs::create_dir_all(&out_dir)?;
    let graph: TopologyGraph = serde_json::from_slice(&std::fs::read(&graph_path)?)?;
    let bytes = std::fs::read(&artifact_path)?;
    let file_name = artifact_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("artifact file name")?
        .to_string();
    let dest = out_dir.join(&file_name);
    if artifact_path.canonicalize().ok().as_ref() != dest.canonicalize().ok().as_ref() {
        std::fs::copy(&artifact_path, &dest)?;
    }

    let scope = ScopeUniverse::from_selectors(
        "review",
        languages,
        includes,
        excludes,
        ScopeCompleteness::Partial,
        None,
    );
    let mut manifest = ArtifactManifest::build(
        format,
        ArtifactProducer {
            name: producer_name,
            version: producer_version,
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        graph.snapshot,
        scope,
        blake3_hex(&bytes),
        None,
    )?;
    manifest.artifact_path = Some(file_name.clone());
    let manifest_path = out_dir.join(format!("{file_name}.manifest.json"));
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    eprintln!("wrote {}", manifest_path.display());
    eprintln!("wrote {}", dest.display());
    Ok(())
}
