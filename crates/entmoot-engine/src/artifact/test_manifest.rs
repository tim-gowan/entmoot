//! Shared helpers for building Wave 0 manifests in adapter tests.

use entmoot_contracts::{
    blake3_hex, ArtifactFormat, ArtifactManifest, ArtifactProducer, ScopeCompleteness,
    ScopeUniverse, SourceSnapshot, SourceState,
};

pub fn test_manifest(
    bytes: &[u8],
    format: ArtifactFormat,
    producer_name: &str,
    producer_version: Option<&str>,
    excluded: Vec<&str>,
    included: Vec<&str>,
) -> ArtifactManifest {
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
        vec!["typescript".into(), "python".into()],
        included.into_iter().map(str::to_string).collect(),
        excluded.into_iter().map(str::to_string).collect(),
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
    ArtifactManifest::build(format, producer, snapshot, scope, blake3_hex(bytes), None)
        .expect("test manifest")
}
