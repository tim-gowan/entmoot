//! Transactional manifest-driven artifact ingestion.

use std::path::{Path, PathBuf};

use entmoot_contracts::ArtifactManifest;

use crate::{
    adapter_for_manifest,
    assertions::{derive_assertions_for_artifact, derive_provider_role_hint_assertions},
    resolve_location, ArtifactError, AttachmentIndexes, DiagnosticSeverity, TopologyGraph,
};

pub use entmoot_contracts::ArtifactIngestStats;

pub fn ingest_artifact_manifest(
    graph: &mut TopologyGraph,
    manifest_path: &Path,
    replace_series: bool,
) -> Result<ArtifactIngestStats, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(manifest_path)?;
    let manifest: ArtifactManifest = serde_json::from_str(&text)?;
    manifest.validate_structure()?;
    manifest.verify_against_snapshot(&graph.snapshot)?;
    let artifact_path = resolve_artifact_path(manifest_path, &manifest)?;
    let bytes = std::fs::read(&artifact_path)?;
    let verified = crate::ManifestUnverified::new(manifest).verify(&bytes, &graph.snapshot)?;
    let manifest = verified.into_manifest();
    let mut normalized = adapter_for_manifest(&manifest).normalize(&bytes, &manifest)?;
    crate::artifact::assessment::join(graph, &manifest, &mut normalized)?;
    crate::package_inventory::validate_package_inventory(
        graph,
        &manifest,
        normalized.resolver_package_api.as_ref(),
    )?;
    if let Some(coverage) = normalized.resolver_coverages.first() {
        crate::artifact::resolver::join_inventory_bindings(
            graph,
            &manifest,
            &coverage.language,
            &normalized.resolver_observations,
        )?
        .project(&mut normalized)?;
        crate::relation_claims::share_structural_call_projections(graph, &mut normalized);
    }
    let provider_role_hints = normalized.provider_role_hints.clone();
    if normalized
        .diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error)
    {
        return Err(ArtifactError::Message(
            "artifact normalization produced error diagnostics".into(),
        )
        .into());
    }

    let location_paths: Vec<&str> = normalized
        .locations
        .iter()
        .map(|location| location.path.as_str())
        .collect();
    let sources = crate::read_graph_sources_for_paths(graph, &location_paths)?;
    let indexes = AttachmentIndexes::new(graph, &sources);
    let attachments = normalized
        .locations
        .iter()
        .map(|location| resolve_location(location, &manifest, &indexes))
        .collect::<Vec<_>>();
    let mut stats = ArtifactIngestStats {
        artifact_id: manifest.artifact_id.clone(),
        artifact_digest: manifest.content_digest.clone(),
        resolver_observations: normalized.resolver_observations.len(),
        resolver_targets: normalized.resolutions.len(),
        unresolved: normalized.unresolved_relations.len(),
        parsed: if normalized.source_record_count > 0 {
            normalized.source_record_count
        } else {
            normalized.findings.len()
                + normalized.resources.len()
                + normalized.components.len()
                + normalized.component_dependencies.len()
                + normalized.component_vulnerabilities.len()
                + normalized.bom_compositions.len()
                + normalized.relation_edges.len()
                + normalized.resolutions.len()
        },
        ..Default::default()
    };
    for attachment in &attachments {
        use entmoot_contracts::{AttachmentDisposition::*, AttachmentTargetKind::*};
        match attachment.disposition {
            Excluded => stats.excluded += 1,
            Unresolved => stats.unresolved += 1,
            Invalid => stats.invalid += 1,
            Attached => match attachment.selected_target.as_ref().map(|t| t.kind) {
                Some(File) => stats.file_attached += 1,
                Some(Callable) => stats.callable_attached += 1,
                Some(CallSite) => stats.call_site_attached += 1,
                Some(Resource) => stats.resource_attached += 1,
                _ => stats.unresolved += 1,
            },
        }
    }

    if graph
        .artifacts
        .iter()
        .any(|a| a.artifact_id == manifest.artifact_id)
    {
        stats.already_present = true;
        return Ok(stats);
    }
    let mut candidate = graph.clone();
    candidate.rebuild_indexes();
    if replace_series {
        let replaced: Vec<String> = candidate
            .artifacts
            .iter()
            .filter(|a| a.artifact_series_id == manifest.artifact_series_id)
            .map(|a| a.artifact_id.clone())
            .collect();
        candidate.remove_artifacts(&replaced);
    }
    candidate.capabilities.extend(normalized.capabilities);
    candidate.components.extend(normalized.components);
    candidate
        .component_dependencies
        .extend(normalized.component_dependencies);
    candidate
        .component_vulnerabilities
        .extend(normalized.component_vulnerabilities);
    candidate
        .bom_compositions
        .extend(normalized.bom_compositions);
    candidate.findings.extend(normalized.findings);
    candidate.locations.extend(normalized.locations);
    candidate.traces.extend(normalized.traces);
    candidate.trace_steps.extend(normalized.trace_steps);
    candidate.diagnostics.extend(normalized.diagnostics);
    candidate.resolutions.extend(normalized.resolutions);
    candidate
        .resolver_coverages
        .extend(normalized.resolver_coverages);
    candidate.attachments.extend(attachments.clone());
    candidate.artifacts.push(manifest.clone());
    // Resolved identity (ADR-0024) is derived once the artifact's observations
    // are joined against the inventoried call stubs of this exact snapshot.
    if let Some(coverage) = candidate
        .resolver_coverages
        .iter()
        .find(|coverage| coverage.artifact_id == manifest.artifact_id)
    {
        let language = coverage.language.clone();
        let source = entmoot_contracts::ResolverEvidenceRef {
            producer: entmoot_contracts::ProducerIdentity {
                producer_id: coverage.producer_id.clone(),
                version: coverage.producer_version.clone(),
                tool_name: None,
            },
            artifact: entmoot_contracts::ArtifactRef::from_manifest(&manifest)?,
        };
        let records = crate::derive_resolved_identities_from_graph(
            &candidate,
            &manifest,
            &language,
            &normalized.resolver_observations,
            &source,
        );
        stats.resolved_identities = records.len();
        candidate.resolved_identities.extend(records);
    }
    crate::config_inventory::materialize_resource_records(&mut candidate, normalized.resources);
    for edge in normalized.relation_edges {
        candidate.upsert_relation_edge(edge);
    }
    for claim in normalized.relation_claims {
        if manifest.format == entmoot_contracts::ArtifactFormat::Resolver {
            crate::relation_claims::merge_native_claim(&mut candidate, claim)?;
        } else {
            candidate.insert_relation_claim(claim);
        }
    }
    for relation in normalized.unresolved_relations {
        if manifest.format == entmoot_contracts::ArtifactFormat::Resolver {
            crate::relation_claims::merge_native_unresolved(&mut candidate, relation)?;
        } else {
            candidate.insert_unresolved_relation(relation);
        }
    }
    let mut assertions =
        derive_assertions_for_artifact(&candidate, &manifest.artifact_id, &attachments);
    assertions.extend(derive_provider_role_hint_assertions(
        &manifest.artifact_id,
        &provider_role_hints,
        &attachments,
    ));
    stats.assertions = assertions.len();
    candidate.assertions.extend(assertions);
    candidate.validate()?;
    // Successful attach mutates the in-memory projection directly. PT-27
    // rematerialize (extract → skeleton → apply) remains for store reload and
    // dedicated rematerialize tests—not an identity round-trip on every attach.
    candidate.rebuild_indexes();
    *graph = candidate;
    Ok(stats)
}

pub fn resolve_artifact_path(
    manifest_path: &Path,
    manifest: &ArtifactManifest,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let relative = manifest
        .artifact_path
        .as_deref()
        .ok_or("manifest artifact_path is required for ingestion")?;
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(
            "artifact_path must be relative to its manifest without parent traversal".into(),
        );
    }
    Ok(manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        blake3_hex, ArtifactFormat, ArtifactProducer, ScopeCompleteness, ScopeUniverse,
    };
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    fn graph_and_manifest() -> (tempfile::TempDir, TopologyGraph, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(
            temp.path().join("src/a.rs"),
            "fn run() { println!(\"ok\"); }\n",
        )
        .unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            temp.path(),
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("example/repo"),
            },
        )
        .unwrap();
        let artifact = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[{"ruleId":"R1","message":{"text":"x"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/a.rs"},"region":{"startLine":1,"startColumn":4}}}]}]}]}"#;
        let artifact_path = temp.path().join("fixture.sarif");
        std::fs::write(&artifact_path, artifact).unwrap();
        let scope = ScopeUniverse::from_selectors(
            "test",
            vec!["rust".into()],
            vec!["src/a.rs".into()],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let mut manifest = ArtifactManifest::build(
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "fixture".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            scope,
            blake3_hex(artifact),
            None,
        )
        .unwrap();
        manifest.artifact_path = Some("fixture.sarif".into());
        let manifest_path = temp.path().join("fixture.manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        graph.validate().unwrap();
        (temp, graph, manifest_path)
    }

    #[test]
    fn native_resolver_ingestion_checks_inventory_and_is_failure_atomic() {
        use entmoot_contracts::{
            topology_node_address, ContentDigest, ResolverArtifact, ResolverArtifactProducer,
            ResolverCallSource, ResolverObservation, ResolverObservationOutcome, ResolverTarget,
            RESOLVER_ARTIFACT_SCHEMA,
        };
        let (temp, _, _) = graph_and_manifest();
        std::fs::write(
            temp.path().join("src/a.rs"),
            "fn target() {}\nfn run() { target(); }\n",
        )
        .unwrap();
        let mut graph = crate::build_topology_from_repo_with_options(
            temp.path(),
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("example/repo"),
            },
        )
        .unwrap();
        let site = graph
            .nodes
            .iter()
            .find(|node| node.kind == crate::NodeKind::CallStub)
            .unwrap();
        let owner = site.parent.clone().unwrap();
        let mut artifact = ResolverArtifact {
            assessments: vec![],
            package_api: None,
            schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
            implementation: crate::implementation_fingerprint(),
            producer: ResolverArtifactProducer {
                id: "native".into(),
                name: "native".into(),
                version: "1".into(),
                method_id: "fixture.definition.v1".into(),
            },
            snapshot: graph.snapshot.clone(),
            language: "rust".into(),
            language_version: None,
            build_context_digest: ContentDigest::parse(blake3_hex(b"environment")).unwrap(),
            observations: vec![ResolverObservation {
                dispatch: Default::default(),
                occurrence_id: site.id.clone(),
                source: ResolverCallSource {
                    call_site: topology_node_address(&site.id),
                    relation_source: topology_node_address(&owner),
                    syntactic_callee: site.label.clone(),
                },
                outcome: ResolverObservationOutcome::RepositoryCallable,
                targets: vec![ResolverTarget {
                    address: topology_node_address(&owner),
                    relation_target: Some(topology_node_address(&owner)),
                    provider_pointer: "/native/definition".into(),
                    provider_symbol: None,
                    provider_callable: None,
                    declaration: None,
                }],
                limitation: None,
            }],
        };
        let write = |artifact: &ResolverArtifact, name: &str, excludes: Vec<String>| {
            let bytes = serde_json::to_vec(artifact).unwrap();
            std::fs::write(temp.path().join(name), &bytes).unwrap();
            write_matched_manifest(
                temp.path(),
                ArtifactFormat::Resolver,
                ArtifactProducer {
                    name: artifact.producer.name.clone(),
                    version: Some("1".into()),
                    information_uri: None,
                    ruleset_id: None,
                    config_class: None,
                },
                artifact.snapshot.clone(),
                vec!["rust".into()],
                vec![],
                excludes,
                &bytes,
                name,
            )
        };
        let manifest = write(&artifact, "native.json", vec![]);
        let stats = ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        assert_eq!(stats.resolver_observations, 1);
        assert_eq!(stats.resolver_targets, 1);
        assert_eq!(graph.resolver_coverages.len(), 1);
        // Both in-scope Rust callables receive a resolved identity: `run` joins
        // its call stub, `target` has an empty (complete) binding set.
        assert_eq!(stats.resolved_identities, 2);
        assert_eq!(graph.resolved_identities.len(), 2);
        let native_hash_for = |graph: &TopologyGraph, address_id: &str| {
            graph
                .resolved_identities
                .iter()
                .find(|r| {
                    r.binding
                        .source()
                        .is_some_and(|source| source.producer.producer_id == "native")
                        && r.address_id == address_id
                })
                .map(|r| (r.resolved_hash.clone(), r.binding.completeness()))
        };
        let owner_address = graph
            .nodes
            .iter()
            .find(|node| node.id == owner)
            .and_then(|node| node.address_id.clone())
            .unwrap();
        let (complete_hash, completeness) = native_hash_for(&graph, &owner_address).unwrap();
        assert_eq!(
            completeness,
            entmoot_contracts::ResolvedIdentityCompleteness::Complete
        );
        assert!(!complete_hash.is_empty());
        for record in &graph.resolved_identities {
            record.validate().unwrap();
            assert!(graph.artifacts.iter().any(|a| record
                .binding
                .source()
                .is_some_and(|source| source.artifact.verify(a).is_ok())));
        }
        let mut forged = graph.clone();
        let source = match &mut forged.resolved_identities[0].binding {
            entmoot_contracts::ResolvedBindingEvidence::Complete { source, .. }
            | entmoot_contracts::ResolvedBindingEvidence::Partial { source, .. } => source,
            entmoot_contracts::ResolvedBindingEvidence::Unavailable { .. } => {
                panic!("attached evidence")
            }
        };
        source.artifact.content_digest =
            ContentDigest::from_trusted(blake3_hex(b"unrelated artifact bytes"));
        assert!(forged.validate().is_err());
        let raw = serde_json::to_value(&artifact).unwrap();
        let resolution = graph
            .resolutions
            .iter()
            .find(|r| r.producer_id == "native")
            .unwrap();
        assert!(raw
            .pointer(&resolution.evidence_refs[0].provider_pointer)
            .is_some());
        let before = crate::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint();
        ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        assert_eq!(
            before,
            crate::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint()
        );
        let excluded = write(&artifact, "excluded.json", vec!["src/a.rs".into()]);
        assert!(ingest_artifact_manifest(&mut graph, &excluded, false).is_err());
        artifact.observations[0].targets[0].relation_target =
            Some(topology_node_address("missing"));
        let invalid = write(&artifact, "invalid.json", vec![]);
        assert!(ingest_artifact_manifest(&mut graph, &invalid, false).is_err());
        assert_eq!(
            before,
            crate::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint()
        );
        artifact.observations[0].targets[0].relation_target = Some(topology_node_address(&owner));
        artifact.producer.id = "independent".into();
        artifact.producer.name = "independent".into();
        let independent = write(&artifact, "independent.json", vec![]);
        ingest_artifact_manifest(&mut graph, &independent, false).unwrap();
        assert_eq!(graph.resolver_coverages.len(), 2);
        assert_eq!(
            graph
                .resolutions
                .iter()
                .filter(|r| r.producer_id == "native")
                .count(),
            1
        );
        assert_eq!(
            graph
                .resolutions
                .iter()
                .filter(|r| r.producer_id == "independent")
                .count(),
            1
        );
        let target_id = graph
            .nodes
            .iter()
            .find(|node| node.kind == crate::NodeKind::Callable && node.label == "target")
            .unwrap()
            .id
            .clone();
        artifact.observations[0].targets[0].address = topology_node_address(&target_id);
        artifact.observations[0].targets[0].relation_target =
            Some(topology_node_address(&target_id));
        let replacement = write(&artifact, "independent-new.json", vec![]);
        ingest_artifact_manifest(&mut graph, &replacement, true).unwrap();
        assert_eq!(graph.resolver_coverages.len(), 2);
        assert_eq!(
            graph
                .resolutions
                .iter()
                .filter(|r| r.producer_id == "native")
                .count(),
            1
        );
        assert_eq!(
            graph
                .resolutions
                .iter()
                .filter(|r| r.producer_id == "independent")
                .count(),
            1
        );
        artifact.producer.id = "native".into();
        artifact.producer.name = "native".into();
        artifact.observations[0].outcome = ResolverObservationOutcome::Unresolved;
        artifact.observations[0].targets.clear();
        let replacement = write(&artifact, "native-new.json", vec![]);
        ingest_artifact_manifest(&mut graph, &replacement, true).unwrap();
        assert_eq!(graph.resolver_coverages.len(), 2);
        assert_eq!(
            graph
                .resolutions
                .iter()
                .filter(|r| r.producer_id == "independent")
                .count(),
            1
        );
        assert!(!graph
            .relation_edges
            .iter()
            .any(|edge| edge.subject == topology_node_address(&owner)
                && edge.object == topology_node_address(&owner)));
    }

    #[test]
    fn file_only_sarif_attaches_idempotently_without_fabricating_a_callsite() {
        let (temp, mut graph, original) = graph_and_manifest();
        let manifest: ArtifactManifest =
            serde_json::from_slice(&std::fs::read(original).unwrap()).unwrap();
        let mut artifact: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("fixture.sarif")).unwrap())
                .unwrap();
        artifact["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
            .as_object_mut()
            .unwrap()
            .remove("region");
        let bytes = serde_json::to_vec(&artifact).unwrap();
        std::fs::write(temp.path().join("file-only.sarif"), &bytes).unwrap();
        let path = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            manifest.producer.clone(),
            graph.snapshot.clone(),
            vec!["rust".into()],
            vec![],
            vec![],
            &bytes,
            "file-only.sarif",
        );
        let stats = ingest_artifact_manifest(&mut graph, &path, false).unwrap();
        assert_eq!(
            (
                stats.file_attached,
                stats.call_site_attached,
                stats.callable_attached
            ),
            (1, 0, 0)
        );
        assert!(graph.locations[0].extent.is_file());
        let before = crate::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint();
        ingest_artifact_manifest(&mut graph, &path, false).unwrap();
        assert_eq!(
            before,
            crate::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint()
        );
        artifact["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"] =
            serde_json::json!({"startLine": "wrong"});
        let bytes = serde_json::to_vec(&artifact).unwrap();
        std::fs::write(temp.path().join("invalid.sarif"), &bytes).unwrap();
        let path = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            manifest.producer,
            graph.snapshot.clone(),
            vec!["rust".into()],
            vec![],
            vec![],
            &bytes,
            "invalid.sarif",
        );
        assert!(ingest_artifact_manifest(&mut graph, &path, false).is_err());
        assert_eq!(
            before,
            crate::KnowledgeStore::extract_from_graph(&graph).semantic_fingerprint()
        );
    }

    #[test]
    fn file_only_sarif_attaches_to_snapshot_file_without_language_parser() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("dependencies.lock"), "opaque lock data\n").unwrap();
        let mut graph = crate::build_topology_from_repo_with_options(
            temp.path(),
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("example/dependencies"),
            },
        )
        .unwrap();
        let bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[{"ruleId":"OSV-1","message":{"text":"dependency finding"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"dependencies.lock"}}}]}]}]}"#;
        std::fs::write(temp.path().join("osv.sarif"), bytes).unwrap();
        let manifest = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "fixture-osv".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec![],
            vec!["dependencies.lock".into()],
            vec![],
            bytes,
            "osv.sarif",
        );

        let stats = ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        assert_eq!(stats.file_attached, 1);
        assert_eq!(stats.unresolved, 0);
        let attachment = graph.attachments.first().expect("attachment");
        assert_eq!(
            attachment
                .selected_target
                .as_ref()
                .map(|target| target.kind),
            Some(entmoot_contracts::AttachmentTargetKind::File)
        );
        graph.validate().unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn write_matched_manifest(
        dir: &Path,
        format: ArtifactFormat,
        producer: ArtifactProducer,
        snapshot: entmoot_contracts::SourceSnapshot,
        languages: Vec<String>,
        included: Vec<String>,
        excluded: Vec<String>,
        bytes: &[u8],
        artifact_rel: &str,
    ) -> PathBuf {
        let scope = ScopeUniverse::from_selectors(
            "review",
            languages,
            included,
            excluded,
            ScopeCompleteness::Partial,
            None,
        );
        let mut manifest =
            ArtifactManifest::build(format, producer, snapshot, scope, blake3_hex(bytes), None)
                .unwrap();
        manifest.artifact_path = Some(artifact_rel.into());
        let path = dir.join(format!("{artifact_rel}.manifest.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        path
    }

    fn multi_vendor_workspace() -> (tempfile::TempDir, TopologyGraph) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("routes")).unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("routes/login.ts"),
            "function login(user) {\n  const q = user.q;\n  console.log(q);\n  db.find(q);\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("sample.py"), "def vuln(x):\n    return x\n").unwrap();
        std::fs::write(
            root.join("app/views.py"),
            "def render(html):\n    return html\n",
        )
        .unwrap();
        // TypeScript fixture used by SARIF attach dogfood in this workspace.
        std::fs::write(root.join("src/lib.ts"), "export hello\n").unwrap();

        let graph = crate::build_topology_from_repo_with_options(
            root,
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("github.com/acme/app"),
            },
        )
        .unwrap();
        (temp, graph)
    }

    #[test]
    fn ingest_is_idempotent_and_attaches_locations() {
        let (_temp, mut graph, manifest) = graph_and_manifest();
        let stats = ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        assert_eq!(stats.parsed, 1);
        assert_eq!(graph.artifacts.len(), 1);
        assert_eq!(graph.findings.len(), 1);
        assert_eq!(graph.locations.len(), 1);
        assert_eq!(graph.attachments.len(), 1);
        assert!(!graph.assertions.is_empty());
        assert!(graph
            .assertions
            .iter()
            .all(|a| a.predicate == crate::assertions::PREDICATE_FINDING_LOCATION));
        ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        assert_eq!(graph.artifacts.len(), 1);
        assert_eq!(graph.attachments.len(), 1);
    }

    #[test]
    fn ingest_knowledge_survives_topology_view_delete() {
        let (_temp, mut graph, manifest) = graph_and_manifest();
        ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        let knowledge = crate::KnowledgeStore::extract_from_graph(&graph);
        let before = knowledge.semantic_fingerprint();
        let rebuilt = knowledge.apply_to_inventory(crate::inventory_skeleton(&graph));
        let after = crate::KnowledgeStore::extract_from_graph(&rebuilt).semantic_fingerprint();
        assert_eq!(before, after);
        assert!(!rebuilt.assertions.is_empty());
        assert_eq!(rebuilt.schema_version, crate::TOPOLOGY_SCHEMA_VERSION);
    }

    #[test]
    fn failed_ingest_leaves_graph_byte_for_byte_unchanged() {
        let (_temp, mut graph, manifest) = graph_and_manifest();
        ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        let before = serde_json::to_vec(&graph).unwrap();
        let mut invalid: ArtifactManifest =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        invalid.content_digest = "blake3:deadbeef".into();
        std::fs::write(&manifest, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        assert!(ingest_artifact_manifest(&mut graph, &manifest, false).is_err());
        assert_eq!(serde_json::to_vec(&graph).unwrap(), before);
    }

    #[test]
    fn attachment_rejects_source_changed_since_graph_indexing() {
        let (temp, mut graph, manifest) = graph_and_manifest();
        let before = serde_json::to_vec(&graph).unwrap();
        std::fs::write(temp.path().join("src/a.rs"), "fn changed() {}\n").unwrap();
        let error = ingest_artifact_manifest(&mut graph, &manifest, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("source snapshot changed"));
        assert_eq!(serde_json::to_vec(&graph).unwrap(), before);
    }

    #[test]
    fn artifact_paths_cannot_escape_manifest_directory() {
        let (_temp, graph, manifest_path) = graph_and_manifest();
        let mut manifest: ArtifactManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.artifact_path = Some("../outside.sarif".into());
        assert!(resolve_artifact_path(&manifest_path, &manifest).is_err());
        graph.validate().unwrap();
    }

    #[test]
    fn series_replace_preserves_other_vendors() {
        let (temp, mut graph) = multi_vendor_workspace();
        let fixtures = repo_root().join("crates/entmoot-contracts/tests/fixtures/artifacts");
        let snyk_bytes = std::fs::read(fixtures.join("sarif/snyk-code.min.sarif")).unwrap();
        let opengrep_bytes = std::fs::read(fixtures.join("sarif/opengrep.min.sarif")).unwrap();
        std::fs::write(temp.path().join("snyk.sarif"), &snyk_bytes).unwrap();
        std::fs::write(temp.path().join("opengrep.sarif"), &opengrep_bytes).unwrap();

        let snyk = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "SnykCode".into(),
                version: Some("1.0.0-test".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["typescript".into()],
            vec![],
            vec!["target/**".into()],
            &snyk_bytes,
            "snyk.sarif",
        );
        let opengrep = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "Opengrep OSS".into(),
                version: Some("1.0.0-test".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["python".into()],
            vec![],
            vec![],
            &opengrep_bytes,
            "opengrep.sarif",
        );
        ingest_artifact_manifest(&mut graph, &snyk, false).unwrap();
        ingest_artifact_manifest(&mut graph, &opengrep, false).unwrap();
        let opengrep_id = graph
            .artifacts
            .iter()
            .find(|a| a.producer.name.contains("Opengrep"))
            .unwrap()
            .artifact_id
            .clone();

        // Re-ingest a new Snyk series instance (different content digest → new artifact_id)
        // by rewriting bytes slightly while keeping series producer/scope shape via replace_series.
        let mut snyk_v2 = snyk_bytes;
        snyk_v2.extend_from_slice(b"\n");
        std::fs::write(temp.path().join("snyk.sarif"), &snyk_v2).unwrap();
        let snyk2 = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "SnykCode".into(),
                version: Some("1.0.0-test".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["typescript".into()],
            vec![],
            vec!["target/**".into()],
            &snyk_v2,
            "snyk.sarif",
        );
        // Force same series id as first Snyk by editing the written manifest.
        let mut m: ArtifactManifest =
            serde_json::from_slice(&std::fs::read(&snyk2).unwrap()).unwrap();
        let first_snyk = graph
            .artifacts
            .iter()
            .find(|a| a.producer.name == "SnykCode")
            .unwrap();
        m.artifact_series_id = first_snyk.artifact_series_id.clone();
        std::fs::write(&snyk2, serde_json::to_vec_pretty(&m).unwrap()).unwrap();

        ingest_artifact_manifest(&mut graph, &snyk2, true).unwrap();
        assert!(graph.artifacts.iter().any(|a| a.artifact_id == opengrep_id));
        assert_eq!(
            graph
                .artifacts
                .iter()
                .filter(|a| a.producer.name == "SnykCode")
                .count(),
            1
        );
        graph.validate().unwrap();
    }

    #[test]
    fn export_const_arrow_sarif_attaches_to_callable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("routes")).unwrap();
        let handler_src = std::fs::read_to_string(
            repo_root()
                .join("crates/entmoot-core/tests/fixtures/callable_parity/routes/handler.ts"),
        )
        .unwrap();
        std::fs::write(root.join("routes/handler.ts"), &handler_src).unwrap();
        let mut graph = crate::build_topology_from_repo_with_options(
            root,
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("local/callable-parity"),
            },
        )
        .unwrap();
        assert!(
            graph.nodes.iter().any(|n| {
                n.kind == crate::NodeKind::Callable && n.display_ref.contains("handler")
            }),
            "expected handler callable node, nodes={:?}",
            graph
                .nodes
                .iter()
                .filter(|n| n.kind == crate::NodeKind::Callable)
                .map(|n| &n.display_ref)
                .collect::<Vec<_>>()
        );

        let sarif_bytes = std::fs::read(
            repo_root()
                .join("crates/entmoot-core/tests/fixtures/callable_parity/handler_arrow.sarif"),
        )
        .unwrap();
        std::fs::write(root.join("handler_arrow.sarif"), &sarif_bytes).unwrap();
        let manifest = write_matched_manifest(
            root,
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "SyntheticCallableParity".into(),
                version: Some("1.0.0-test".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["typescript".into()],
            vec![],
            vec![],
            &sarif_bytes,
            "handler_arrow.sarif",
        );
        ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        let primary = graph
            .attachments
            .iter()
            .find(|a| {
                graph
                    .findings
                    .iter()
                    .any(|f| f.primary_location_ids.contains(&a.location_id))
            })
            .expect("primary attachment");
        assert_eq!(
            primary.selected_target.as_ref().map(|t| t.kind),
            Some(entmoot_contracts::AttachmentTargetKind::Callable),
            "reason={:?} target={:?}",
            primary.reason_codes,
            primary.selected_target
        );
        graph.validate().unwrap();
    }

    #[test]
    fn assigned_lambda_sarif_attaches_to_callable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("routes")).unwrap();
        let handler_src = std::fs::read_to_string(
            repo_root()
                .join("crates/entmoot-core/tests/fixtures/callable_parity/routes/handler.py"),
        )
        .unwrap();
        std::fs::write(root.join("routes/handler.py"), &handler_src).unwrap();
        let mut graph = crate::build_topology_from_repo_with_options(
            root,
            crate::TopologyBuildOptions {
                policy: None,
                repository: Some("local/callable-parity-py"),
            },
        )
        .unwrap();
        assert!(
            graph.nodes.iter().any(|n| {
                n.kind == crate::NodeKind::Callable && n.display_ref.contains("handler")
            }),
            "expected handler callable node, nodes={:?}",
            graph
                .nodes
                .iter()
                .filter(|n| n.kind == crate::NodeKind::Callable)
                .map(|n| &n.display_ref)
                .collect::<Vec<_>>()
        );

        let sarif_bytes = std::fs::read(
            repo_root()
                .join("crates/entmoot-core/tests/fixtures/callable_parity/handler_lambda.sarif"),
        )
        .unwrap();
        std::fs::write(root.join("handler_lambda.sarif"), &sarif_bytes).unwrap();
        let manifest = write_matched_manifest(
            root,
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "SyntheticCallableParity".into(),
                version: Some("1.0.0-test".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["python".into()],
            vec![],
            vec![],
            &sarif_bytes,
            "handler_lambda.sarif",
        );
        ingest_artifact_manifest(&mut graph, &manifest, false).unwrap();
        let primary = graph
            .attachments
            .iter()
            .find(|a| {
                graph
                    .findings
                    .iter()
                    .any(|f| f.primary_location_ids.contains(&a.location_id))
            })
            .expect("primary attachment");
        assert_eq!(
            primary.selected_target.as_ref().map(|t| t.kind),
            Some(entmoot_contracts::AttachmentTargetKind::Callable),
            "reason={:?} target={:?}",
            primary.reason_codes,
            primary.selected_target
        );
        graph.validate().unwrap();
    }

    #[test]
    fn provider_stated_kinds_emit_role_claims_and_generic_sarif_does_not() {
        let (temp, mut graph) = multi_vendor_workspace();
        let role_bytes = std::fs::read(repo_root().join(
            "crates/entmoot-contracts/tests/fixtures/artifacts/sarif/provider-roles.min.sarif",
        ))
        .unwrap();
        std::fs::write(temp.path().join("roles.sarif"), &role_bytes).unwrap();
        let roles = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "SyntheticRoleProducer".into(),
                version: Some("1.0.0-test".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["typescript".into()],
            vec![],
            vec![],
            &role_bytes,
            "roles.sarif",
        );
        ingest_artifact_manifest(&mut graph, &roles, false).unwrap();
        assert!(
            graph.assertions.iter().any(|a| {
                a.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE
                    && a.object.value.as_deref() == Some("sink")
            }),
            "expected sink role assertion, roles={:?}",
            graph
                .assertions
                .iter()
                .filter(|a| a.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE)
                .map(|a| &a.object.value)
                .collect::<Vec<_>>()
        );
        assert!(graph.assertions.iter().any(|a| {
            a.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE
                && a.object.value.as_deref() == Some("source")
        }));

        // Negative: role assertions always carry an explicit provider kind token.
        assert!(graph.assertions.iter().all(|a| {
            a.predicate != crate::assertions::PREDICATE_PROVIDER_ROLE || a.object.value.is_some()
        }));

        let opengrep_bytes = br#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"Opengrep OSS","rules":[{"id":"visible-sink","properties":{"tags":["entmoot.role:sink"]}}]}},"results":[{"ruleId":"visible-sink","message":{"text":"Visible sink"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"sample.py"},"region":{"startLine":1,"startColumn":5}}}]}]}]}"#;
        std::fs::write(temp.path().join("opengrep.sarif"), opengrep_bytes).unwrap();
        let opengrep = write_matched_manifest(
            temp.path(),
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "Opengrep OSS".into(),
                version: Some("fixture".into()),
                information_uri: None,
                ruleset_id: Some("visible-boundaries".into()),
                config_class: None,
            },
            graph.snapshot.clone(),
            vec!["python".into()],
            vec![],
            vec![],
            opengrep_bytes,
            "opengrep.sarif",
        );
        ingest_artifact_manifest(&mut graph, &opengrep, false).unwrap();
        assert!(graph.assertions.iter().any(|assertion| {
            assertion.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE
                && assertion.object.value.as_deref() == Some("sink")
                && assertion.derivation.as_ref().is_some_and(|derivation| {
                    derivation.method_id == "logic.sarif.opengrep.rule_role.v1"
                })
        }));
        graph.validate().unwrap();
    }
}
