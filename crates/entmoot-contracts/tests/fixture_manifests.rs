//! Fixture manifests must validate as logic.artifact.v1 (Wave 0 gate).

#[cfg(test)]
mod tests {
    use entmoot_contracts::ArtifactManifest;
    use std::path::PathBuf;

    fn fixtures() -> Vec<PathBuf> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/entmoot-contracts/tests/fixtures/artifacts");
        vec![
            root.join("sarif/snyk-code.min.sarif.manifest.json"),
            root.join("sarif/opengrep.min.sarif.manifest.json"),
            root.join("sarif/semantic-cluster.min.jsonl.manifest.json"),
        ]
    }

    #[test]
    fn fixture_manifests_validate_and_match_digests() {
        for manifest_path in fixtures() {
            let text = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
            let m: ArtifactManifest = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("parse {}: {e}", manifest_path.display()));
            m.validate_structure()
                .unwrap_or_else(|e| panic!("validate {}: {e}", manifest_path.display()));
            let art = manifest_path
                .parent()
                .unwrap()
                .join(m.artifact_path.as_ref().expect("artifact_path"));
            let bytes = std::fs::read(&art).unwrap();
            m.verify_content_digest(&bytes)
                .unwrap_or_else(|e| panic!("digest {}: {e}", art.display()));
        }
    }
}
