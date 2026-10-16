//! Deterministic length-prefixed identity hashing (BLAKE3).

pub const IDENTITY_SCHEMA_VERSION: &str = "logic.identity.v1";
pub const DIGEST_PREFIX: &str = "blake3:";

/// Hash ordered string parts with length prefixes (stable across join order).
pub fn hash_parts(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        let bytes = part.as_bytes();
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_hex().to_string()
}

pub fn blake3_hex(bytes: &[u8]) -> String {
    format!("{DIGEST_PREFIX}{}", blake3::hash(bytes).to_hex())
}

pub fn strip_digest_prefix(value: &str) -> &str {
    value.strip_prefix(DIGEST_PREFIX).unwrap_or(value)
}

/// True when revision looks like a full git SHA (40 hex) or similarly immutable id.
pub fn is_immutable_revision(revision: &str) -> bool {
    let rev = revision.trim();
    if rev.is_empty() || rev == "main" || rev == "master" || rev == "HEAD" {
        return false;
    }
    rev.len() >= 20 && rev.chars().all(|c| c.is_ascii_hexdigit())
}

/// Canonical content-manifest digest: sorted path → content digest over scope.
pub fn content_manifest_digest(entries: &[(String, String)]) -> String {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (path, digest) in &sorted {
        let p = path.as_bytes();
        let d = strip_digest_prefix(digest).as_bytes();
        hasher.update(&(p.len() as u64).to_le_bytes());
        hasher.update(p);
        hasher.update(&(d.len() as u64).to_le_bytes());
        hasher.update(d);
    }
    format!("{DIGEST_PREFIX}{}", hasher.finalize().to_hex())
}

pub struct ArtifactIdentityInput<'a> {
    pub format: &'a str,
    pub producer_name: &'a str,
    pub producer_version: &'a str,
    pub repository_id: &'a str,
    pub revision: &'a str,
    pub source_state: &'a str,
    pub snapshot_digest: &'a str,
    pub content_digest: &'a str,
    pub config_digest: &'a str,
    pub scope_universe_id: &'a str,
}

pub fn derive_artifact_id(input: &ArtifactIdentityInput<'_>) -> String {
    hash_parts(&[
        "artifact_instance",
        IDENTITY_SCHEMA_VERSION,
        input.format,
        input.producer_name,
        input.producer_version,
        input.repository_id,
        input.revision,
        input.source_state,
        strip_digest_prefix(input.snapshot_digest),
        strip_digest_prefix(input.content_digest),
        strip_digest_prefix(input.config_digest),
        input.scope_universe_id,
    ])
}

pub fn derive_artifact_series_id(
    format: &str,
    producer_name: &str,
    config_class: &str,
    scope_universe_id: &str,
    repository_id: &str,
) -> String {
    hash_parts(&[
        "artifact_series",
        IDENTITY_SCHEMA_VERSION,
        format,
        producer_name,
        config_class,
        scope_universe_id,
        repository_id,
    ])
}

pub fn finding_observation_id(
    artifact_id: &str,
    run_index: u32,
    fingerprint_or_result: &str,
) -> String {
    hash_parts(&[
        "finding_obs",
        artifact_id,
        &run_index.to_string(),
        fingerprint_or_result,
    ])
}

pub fn location_id(
    artifact_id: &str,
    path: &str,
    start_line: u32,
    start_column: Option<u32>,
    end_line: Option<u32>,
    end_column: Option<u32>,
    provider_pointer: &str,
) -> String {
    hash_parts(&[
        "location",
        artifact_id,
        path,
        &start_line.to_string(),
        &start_column.map(|c| c.to_string()).unwrap_or_default(),
        &end_line.map(|l| l.to_string()).unwrap_or_default(),
        &end_column.map(|c| c.to_string()).unwrap_or_default(),
        provider_pointer,
    ])
}

/// Identity for a file-grain location (no region coordinates).
pub fn file_location_id(artifact_id: &str, path: &str, provider_pointer: &str) -> String {
    hash_parts(&["location", "file", artifact_id, path, provider_pointer])
}

pub fn trace_id(finding_obs_id: &str, code_flow_index: u32, thread_index: u32) -> String {
    hash_parts(&[
        "trace",
        finding_obs_id,
        &code_flow_index.to_string(),
        &thread_index.to_string(),
    ])
}

pub fn step_id(trace_id: &str, ordinal: u32) -> String {
    hash_parts(&["step", trace_id, &ordinal.to_string()])
}

pub fn attachment_id(graph_snapshot_id: &str, location_id: &str, target: &str) -> String {
    hash_parts(&["attachment", graph_snapshot_id, location_id, target])
}

pub fn assertion_id(subject: &str, predicate: &str, object: &str, context: &str) -> String {
    hash_parts(&["assertion", subject, predicate, object, context])
}

/// Topology node IDs (length-prefixed; not FQN / display labels).
pub fn topo_repo_id(root_key: &str) -> String {
    hash_parts(&["topo_repo", root_key])
}

pub fn topo_file_id(path: &str) -> String {
    hash_parts(&["topo_file", path])
}

pub fn topo_callable_id(path: &str, name: &str, start_byte: usize) -> String {
    hash_parts(&["topo_callable", path, name, &start_byte.to_string()])
}

pub fn topo_declaration_id(path: &str, name: &str, start_byte: usize) -> String {
    hash_parts(&["topo_declaration", path, name, &start_byte.to_string()])
}

pub fn topo_import_id(path: &str, module_path: &str) -> String {
    hash_parts(&["topo_import", path, module_path])
}

pub fn topo_call_stub_id(path: &str, start_byte: usize, callee: &str) -> String {
    hash_parts(&["topo_call", path, &start_byte.to_string(), callee])
}

pub fn topo_control_site_id(path: &str, start_byte: usize) -> String {
    hash_parts(&["topo_control_site", path, &start_byte.to_string()])
}

pub fn topo_module_stub_id(module_path: &str) -> String {
    hash_parts(&["topo_module", module_path])
}

pub fn scope_universe_id(
    selector_language_version: &str,
    profile: &str,
    languages: &[String],
    includes: &[String],
    excludes: &[String],
) -> String {
    let mut parts = vec![
        "scope_universe".to_string(),
        selector_language_version.to_string(),
        profile.to_string(),
    ];
    let mut langs = languages.to_vec();
    langs.sort();
    let mut inc = includes.to_vec();
    inc.sort();
    let mut exc = excludes.to_vec();
    exc.sort();
    parts.extend(langs);
    parts.push("|".into());
    parts.extend(inc);
    parts.push("|".into());
    parts.extend(exc);
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    hash_parts(&refs)
}

pub fn observed_document_set_digest(paths: &[String]) -> String {
    let mut sorted = paths.to_vec();
    sorted.sort();
    let refs: Vec<(String, String)> = sorted.into_iter().map(|p| (p, "path".into())).collect();
    // Path-set only: digest of sorted paths (second field is a constant marker).
    content_manifest_digest(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_tree_digests_differ_for_different_files() {
        let a = content_manifest_digest(&[("a.rs".into(), "blake3:aa".into())]);
        let b = content_manifest_digest(&[("a.rs".into(), "blake3:bb".into())]);
        assert_ne!(a, b);
        assert!(a.starts_with(DIGEST_PREFIX));
    }

    #[test]
    fn artifact_instance_includes_snapshot() {
        let input = |snapshot_digest| ArtifactIdentityInput {
            format: "sarif21",
            producer_name: "Snyk",
            producer_version: "1",
            repository_id: "repo",
            revision: "abc",
            source_state: "working_tree",
            snapshot_digest,
            content_digest: "c1",
            config_digest: "cfg",
            scope_universe_id: "scope",
        };
        let x = derive_artifact_id(&input("snap1"));
        let y = derive_artifact_id(&input("snap2"));
        assert_ne!(x, y);
    }

    #[test]
    fn topo_ids_are_stable_and_opaque() {
        let a = topo_file_id("src/a.rs");
        let b = topo_file_id("src/a.rs");
        assert_eq!(a, b);
        assert!(!a.contains("src/a.rs"));
        assert_ne!(
            topo_callable_id("src/a.rs", "f", 0),
            topo_callable_id("src/a.rs", "f", 1)
        );
    }
}
