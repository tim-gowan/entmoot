//! One-shot fixture regenerator for Wave 0 BLAKE3 migration.

use std::fs;
use std::path::Path;

use entmoot_contracts::{
    blake3_hex, ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness,
    ScopeUniverse, SourceSnapshot, SourceState,
};

fn regen(
    artifact_rel: &str,
    format: ArtifactFormat,
    producer_name: &str,
    producer_version: Option<&str>,
    languages: &[&str],
    included: &[&str],
    excluded: &[&str],
) {
    let path = Path::new(artifact_rel);
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let snapshot = SourceSnapshot {
        repository_id: "github.com/acme/app".into(),
        revision: "f356a09207c7a9550eb6fc4c3945e081922cf998".into(),
        source_state: SourceState::Committed,
        snapshot_digest: None,
        estate_id: None,
        subject: None,
        subject_aliases: Vec::new(),
    };
    let scope = ScopeUniverse::from_selectors(
        "review",
        languages.iter().map(|s| (*s).to_string()).collect(),
        included.iter().map(|s| (*s).to_string()).collect(),
        excluded.iter().map(|s| (*s).to_string()).collect(),
        ScopeCompleteness::Partial,
        None,
    );
    let producer = ArtifactProducer {
        name: producer_name.into(),
        version: producer_version.map(str::to_string),
        information_uri: None,
        ruleset_id: None,
        config_class: None,
    };
    let mut m =
        ArtifactManifest::build(format, producer, snapshot, scope, blake3_hex(&bytes), None)
            .unwrap_or_else(|e| panic!("build {}: {e}", path.display()));
    m.artifact_path = Some(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    );
    let out = format!("{}.manifest.json", path.display());
    let json =
        serde_json::to_string_pretty(&m).unwrap_or_else(|e| panic!("json {}: {e}", path.display()));
    fs::write(&out, format!("{json}\n")).unwrap_or_else(|e| panic!("write {out}: {e}"));
    println!("wrote {out}");
}

fn main() {
    regen(
        "crates/entmoot-contracts/tests/fixtures/artifacts/sarif/snyk-code.min.sarif",
        ArtifactFormat::Sarif21,
        "SnykCode",
        Some("1.0.0-test"),
        &["typescript", "python"],
        &[],
        &["target/**"],
    );
    regen(
        "crates/entmoot-contracts/tests/fixtures/artifacts/sarif/opengrep.min.sarif",
        ArtifactFormat::Sarif21,
        "Opengrep OSS",
        Some("1.0.0-test"),
        &["typescript", "python"],
        &[],
        &[],
    );
    regen(
        "crates/entmoot-contracts/tests/fixtures/artifacts/sarif/semantic-cluster.min.jsonl",
        ArtifactFormat::SemanticCluster,
        "opengrep",
        None,
        &["typescript", "python"],
        &[],
        &[],
    );
}
