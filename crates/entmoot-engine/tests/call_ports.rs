//! P1 durable site and attachment controls across the release claim set.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_engine::{KnowledgeRepository, KnowledgeStore, NodeKind, SqliteKnowledgeRepository};

#[test]
fn sites_survive_aggregation_and_knowledge_replay_for_every_claimed_family() {
    for (file, source) in [
        (
            "main.rs",
            "fn target(a: i32) {} fn caller() { target(1); target(2); }",
        ),
        (
            "main.py",
            "def target(a): pass\ndef caller():\n    target(1)\n    target(2)\n",
        ),
        (
            "main.js",
            "function target(a) {} function caller() { target(1); target(2); }",
        ),
        (
            "main.ts",
            "function target(a: number) {} function caller() { target(1); target(2); }",
        ),
        (
            "main.tsx",
            "function target(a: number) {} function caller() { target(1); target(2); }",
        ),
        (
            "main.go",
            "package p\nfunc target(a int) {}\nfunc caller() { target(1); target(2) }",
        ),
        (
            "C.java",
            "class C { void target(int a) {} void caller() { target(1); target(2); } }",
        ),
        (
            "C.cs",
            "class C { void target(int a) {} void caller() { target(1); target(2); } }",
        ),
        (
            "main.c",
            "void target(int a) {} void caller() { target(1); target(2); }",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file), source).unwrap();
        let graph =
            entmoot_engine::build_topology_from_repo_with_options(dir.path(), Default::default())
                .unwrap();
        let store = KnowledgeStore::extract_from_graph(&graph);
        assert_eq!(store.call_sites.len(), 2, "{file}");
        if graph
            .edges
            .iter()
            .any(|edge| edge.resolved && edge.kind == entmoot_engine::EdgeKind::Calls)
        {
            let occurrences: Vec<_> = graph
                .relation_edges
                .iter()
                .filter_map(|edge| edge.occurrence_id.as_ref())
                .collect();
            assert_eq!(
                occurrences.len(),
                2,
                "{file}: aggregate keeps both exact call references"
            );
            assert!(occurrences
                .iter()
                .all(|id| graph.node(id).is_some_and(|n| n.call_site.is_some())));
        }
        let (manifest, mut location) =
            location(&graph, file, source.find("target(1)").unwrap() + 7, 1);
        let sources = vec![(file.to_owned(), source.to_owned())];
        let indexes = entmoot_engine::location_join::AttachmentIndexes::new(&graph, &sources);
        let attachment =
            entmoot_engine::location_join::resolve_location(&location, &manifest, &indexes);
        assert!(attachment
            .reason_codes
            .contains(&entmoot_contracts::AttachmentReason::ExactPortSpan));
        assert_eq!(
            serde_json::to_value(attachment.selected_target.unwrap()).unwrap()["port"],
            serde_json::json!({"kind":"argument", "position":0}),
            "{file}"
        );
        let mut recovered = graph.clone();
        for node in &mut recovered.nodes {
            if let Some(ports) = node.call_site.as_mut().and_then(|m| m.ports.as_mut()) {
                ports.completeness = entmoot_contracts::SiteCompleteness::Partial;
                ports.limitations = vec![entmoot_contracts::SiteLimitation::InvalidSyntax];
            }
        }
        let recovered_index =
            entmoot_engine::location_join::AttachmentIndexes::new(&recovered, &sources);
        let recovered_attachment =
            entmoot_engine::location_join::resolve_location(&location, &manifest, &recovered_index);
        assert!(
            recovered_attachment.selected_target.unwrap().port.is_none(),
            "{file}: recovered invalid syntax cannot authorize an exact port"
        );
        assert!(recovered_attachment
            .candidate_targets
            .iter()
            .all(|target| target.port.is_none()));
        let stale_sources = vec![(file.to_owned(), source.replace("target(1)", "target(9)"))];
        let stale = entmoot_engine::location_join::AttachmentIndexes::new(&graph, &stale_sources);
        let attached =
            entmoot_engine::location_join::resolve_location(&location, &manifest, &stale);
        assert!(
            !attached
                .reason_codes
                .contains(&entmoot_contracts::AttachmentReason::ExactPortSpan),
            "{file}: stale source cannot authorize a port"
        );
        location.extent = entmoot_contracts::LocationExtent::File;
        let attached =
            entmoot_engine::location_join::resolve_location(&location, &manifest, &indexes);
        assert_eq!(
            attached.selected_target.unwrap().kind,
            entmoot_contracts::AttachmentTargetKind::File
        );
        assert_ne!(store.call_sites[0].site_id, store.call_sites[1].site_id);
        let database = dir.path().join("knowledge.sqlite3");
        let mut db = SqliteKnowledgeRepository::open(&database).unwrap();
        db.replace_all(&store).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(loaded.call_sites, store.call_sites);
        let buffer = entmoot_engine::PublicationBuffer {
            max_records: 2, // Relation occurrence + its proposition are one batch.
            max_bytes: 1024 * 1024,
        };
        let mut batches =
            entmoot_engine::KnowledgeStoreBatches::new(store.clone(), buffer).unwrap();
        let mut batched = SqliteKnowledgeRepository::open_in_memory().unwrap();
        batched
            .replace_owner_buffered("fixture/sites", &mut batches, buffer)
            .unwrap();
        assert_eq!(batched.load().unwrap().call_sites, loaded.call_sites);
        assert_eq!(batched.load().unwrap().coverages, loaded.coverages);
        let replay = loaded.apply_to_inventory(graph.clone());
        let query = entmoot_engine::QueryEngine::new(replay.clone(), &database);
        for node in replay.nodes.iter().filter(|node| node.call_site.is_some()) {
            let result = query
                .inspect(entmoot_contracts::query::InspectQuery::Subject {
                    id: node.id.clone(),
                    limit: Some(10),
                })
                .unwrap();
            assert_eq!(
                serde_json::to_value(result).unwrap()["call_site"],
                serde_json::to_value(&node.call_site).unwrap(),
                "{file}: typed port inspection"
            );
        }
        let status = query.status(Default::default()).unwrap();
        assert_eq!(
            status
                .scan_coverage_partitions
                .iter()
                .find(|row| row.name == "parsed")
                .unwrap()
                .count
                .value,
            Some(1)
        );
        assert_eq!(
            serde_json::to_value(&replay.nodes).unwrap(),
            serde_json::to_value(&graph.nodes).unwrap(),
            "{file}: node metadata survives replay"
        );
        // A topology cache loses its sites and edges; authoritative site records
        // rematerialize them behind the aggregated caller-to-callee projection.
        let mut skeleton = graph.clone();
        skeleton.nodes.retain(|n| n.kind != NodeKind::CallStub);
        skeleton.edges.clear();
        skeleton.rebuild_indexes();
        let rebuilt = loaded.apply_to_inventory(skeleton);
        assert_eq!(
            rebuilt
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::CallStub)
                .count(),
            2
        );
        assert_eq!(
            KnowledgeStore::extract_from_graph(&rebuilt).call_sites,
            store.call_sites
        );
        let mut invalid = loaded.clone();
        invalid.call_sites[0]
            .marker
            .ports
            .as_mut()
            .unwrap()
            .arguments[0]
            .span
            .end_byte = usize::MAX;
        assert!(db.replace_all(&invalid).is_err());
        assert_eq!(
            db.load().unwrap().call_sites,
            loaded.call_sites,
            "{file}: atomic rejection"
        );
    }
}

fn location(
    graph: &entmoot_engine::TopologyGraph,
    file: &str,
    start: usize,
    len: usize,
) -> (
    entmoot_contracts::ArtifactManifest,
    entmoot_contracts::LocationRecord,
) {
    use entmoot_contracts::*;
    let manifest = ArtifactManifest::build(
        ArtifactFormat::Sarif21,
        ArtifactProducer {
            name: "fixture".into(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        graph.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "fixture",
            vec![],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        ),
        blake3_hex(b"fixture"),
        None,
    )
    .unwrap();
    let location = LocationRecord {
        location_id: "location".into(),
        artifact_id: manifest.artifact_id.clone(),
        path: file.into(),
        extent: LocationExtent::Region {
            region: SourceRegion::Bytes {
                span: entmoot_contracts::SiteSpan {
                    start_byte: start,
                    end_byte: start + len,
                },
            },
        },

        provider_pointer: "/fixture".into(),
        provider_confidence: None,
    };
    (manifest, location)
}

#[test]
fn coincident_nested_ports_preserve_candidates_with_file_attachment() {
    use entmoot_contracts::{AttachmentReason, AttachmentTargetKind};
    for (file, source) in [
        ("a.rs", "fn caller() { run(inner(x)); }"),
        ("a.py", "def caller():\n    run(inner(x))\n"),
        ("a.js", "function caller() { run(inner(x)); }"),
        ("a.ts", "function caller() { run(inner(x)); }"),
        ("a.tsx", "function caller() { run(inner(x)); }"),
        ("a.go", "package p\nfunc caller() { run(inner(x)) }"),
        ("A.java", "class A { void caller() { run(inner(x)); } }"),
        ("A.cs", "class A { void caller() { run(inner(x)); } }"),
        ("a.c", "void caller() { run(inner(x)); }"),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(file), source).unwrap();
        let graph =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let (manifest, location) = location(&graph, file, source.find("inner(x)").unwrap(), 8);
        let sources = vec![(file.into(), source.into())];
        let indexes = entmoot_engine::location_join::AttachmentIndexes::new(&graph, &sources);
        let attached =
            entmoot_engine::location_join::resolve_location(&location, &manifest, &indexes);
        assert_eq!(
            attached.selected_target.unwrap().kind,
            AttachmentTargetKind::File
        );
        assert_eq!(attached.candidate_targets.len(), 2);
        assert!(attached
            .reason_codes
            .contains(&AttachmentReason::AmbiguousCandidates));
    }
}

#[test]
fn sarif_coordinates_and_source_commitments_control_attachment_across_claimed_families() {
    use entmoot_contracts::{
        blake3_hex, AttachmentDisposition, AttachmentReason, AttachmentTargetKind, SiteSpan,
    };
    use entmoot_engine::artifact::{ArtifactAdapter, SarifAdapter};
    use entmoot_engine::location_join::{resolve_location, AttachmentIndexes};

    for (file, source) in [
        ("a.rs", "fn caller() { /*😀*/ target(); }"),
        ("a.py", "def caller():\n    '😀'; target()\n"),
        ("a.js", "function caller() { /*😀*/ target(); }"),
        ("a.ts", "function caller() { /*😀*/ target(); }"),
        ("a.tsx", "function caller() { /*😀*/ target(); }"),
        ("a.go", "package p\nfunc caller() { /*😀*/ target() }"),
        ("A.java", "class A { void caller() { /*😀*/ target(); } }"),
        ("A.cs", "class A { void caller() { /*😀*/ target(); } }"),
        ("a.c", "void caller() { /*😀*/ target(); }"),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(file), source).unwrap();
        let graph =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        // Independently specified occurrence selector and literal source width.
        let start = source.find("target()").unwrap();
        assert_eq!(source.matches("target()").count(), 1);
        let expected = SiteSpan {
            start_byte: start,
            end_byte: start + 8,
        };
        let line = u32::try_from(
            source[..start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count(),
        )
        .unwrap()
            + 1;
        let prefix = source[..start].rsplit('\n').next().unwrap();
        let sources = vec![(file.into(), source.into())];
        let indexes = AttachmentIndexes::new(&graph, &sources);
        for (kind, column) in [
            (
                "utf16CodeUnits",
                u32::try_from(prefix.encode_utf16().count()).unwrap() + 1,
            ),
            (
                "unicodeCodePoints",
                u32::try_from(prefix.chars().count()).unwrap() + 1,
            ),
        ] {
            let doc = serde_json::json!({"version":"2.1.0","runs":[{
                "columnKind":kind,"tool":{"driver":{"name":"fixture"}},
                "results":[{"ruleId":"coordinate-control","locations":[{"physicalLocation":{
                    "artifactLocation":{"uri":file},
                    // Omitted endLine must default to startLine, not ignore endColumn.
                    "region":{"startLine":line,"startColumn":column,"endColumn":column+8}
                }}]}]
            }]});
            let bytes = serde_json::to_vec(&doc).unwrap();
            let (manifest, _) = location(&graph, file, start, 8);
            // Rebuild identity after changing the exact provider bytes.
            let manifest = entmoot_contracts::ArtifactManifest::build(
                manifest.format,
                manifest.producer,
                manifest.snapshot,
                manifest.scope,
                blake3_hex(&bytes),
                None,
            )
            .unwrap();
            let normalized = SarifAdapter.normalize(&bytes, &manifest).unwrap();
            assert!(
                normalized.diagnostics.is_empty(),
                "{file}: {:?}",
                normalized.diagnostics
            );
            let provider = normalized.locations[0].clone();
            let attached = resolve_location(&provider, &manifest, &indexes);
            assert_eq!(
                attached.disposition,
                AttachmentDisposition::Attached,
                "{file}/{kind}: {attached:?}"
            );
            assert_eq!(
                attached.selected_target.as_ref().unwrap().kind,
                AttachmentTargetKind::CallSite
            );
            assert_eq!(attached.verified_span.as_ref().unwrap().span, expected);
            assert_eq!(
                attached
                    .verified_span
                    .as_ref()
                    .unwrap()
                    .source_digest
                    .as_str(),
                blake3_hex(source.as_bytes())
            );
            assert_eq!(normalized.locations[0], provider);
            let mut publication = graph.clone();
            publication.artifacts.push(manifest.clone());
            publication.locations.push(provider.clone());
            publication.attachments.push(attached.clone());
            publication.validate().unwrap();
            let database = root.path().join(format!("{kind}.sqlite3"));
            let mut repository = SqliteKnowledgeRepository::open(&database).unwrap();
            let knowledge = KnowledgeStore::extract_from_graph(&publication);
            check_buffered_attachment_publication(&knowledge);
            repository.replace_all(&knowledge).unwrap();
            repository.store_inventory(&publication, None).unwrap();
            assert_eq!(
                repository.load().unwrap().attachments,
                publication.attachments
            );
            for damage in 0..3 {
                let mut invalid = knowledge.clone();
                let attachment = &mut invalid.attachments[0];
                match damage {
                    0 => attachment.verified_span = None,
                    1 => attachment.disposition = AttachmentDisposition::Unresolved,
                    _ => attachment.artifact_id = "0".repeat(64),
                }
                assert!(
                    repository.replace_all(&invalid).is_err(),
                    "{file}/{kind}/{damage}"
                );
                assert_eq!(
                    repository.load().unwrap().attachments,
                    publication.attachments
                );
            }
            let mut invalid = publication.clone();
            invalid.attachments[0]
                .verified_span
                .as_mut()
                .unwrap()
                .source_digest =
                entmoot_contracts::ContentDigest::parse(format!("blake3:{}", "0".repeat(64)))
                    .unwrap();
            assert!(invalid.validate().is_err());
            assert!(repository.store_inventory(&invalid, None).is_err());
            assert_eq!(
                repository.load().unwrap().attachments,
                publication.attachments
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                doc
            );

            let mut unknown = provider.clone();
            if let entmoot_contracts::LocationExtent::Region {
                region: entmoot_contracts::SourceRegion::Text { encoding, .. },
            } = &mut unknown.extent
            {
                *encoding = entmoot_contracts::ColumnEncoding::Unknown;
            }
            let unresolved = resolve_location(&unknown, &manifest, &indexes);
            assert_eq!(unresolved.disposition, AttachmentDisposition::Unresolved);
            assert_eq!(
                unresolved.reason_codes,
                vec![AttachmentReason::CoordinateEncodingUnknown]
            );
            assert!(unresolved.verified_span.is_none());
            assert!(unresolved.selected_target.is_none());

            let mut forged = provider.clone();
            forged.artifact_id = "0".repeat(64);
            let invalid = resolve_location(&forged, &manifest, &indexes);
            assert_eq!(
                invalid.reason_codes,
                vec![AttachmentReason::ArtifactIdentityMismatch]
            );
            assert!(invalid.selected_target.is_none());
            assert!(invalid.verified_span.is_none());

            for changed_source in [source.replace("target()", "other ()"), String::new()] {
                let changed_sources = vec![(file.into(), changed_source)];
                let changed = AttachmentIndexes::new(&graph, &changed_sources);
                let stale = resolve_location(&provider, &manifest, &changed);
                assert_eq!(
                    stale.reason_codes,
                    vec![AttachmentReason::SourceDigestMismatch]
                );
                assert_eq!(stale.disposition, AttachmentDisposition::Unresolved);
                assert!(stale.verified_span.is_none());
                assert!(stale.selected_target.is_none());
            }

            let mut missing = graph.clone();
            missing.snapshot_files.clear();
            let missing_indexes = AttachmentIndexes::new(&missing, &sources);
            let unresolved = resolve_location(&provider, &manifest, &missing_indexes);
            assert_eq!(
                unresolved.reason_codes,
                vec![AttachmentReason::SourceDigestUnavailable]
            );
            assert_eq!(unresolved.disposition, AttachmentDisposition::Unresolved);
        }
    }
}

/// Exercise the actual publication iterator using provider records and joins
/// produced above. One-record batches force every schema reference across a
/// batch boundary; reverse order also checks forward references.
fn check_buffered_attachment_publication(knowledge: &KnowledgeStore) {
    use entmoot_engine::{KnowledgeStoreBatches, PublicationBuffer};

    let store = KnowledgeStore {
        artifacts: knowledge.artifacts.clone(),
        locations: knowledge.locations.clone(),
        attachments: knowledge.attachments.clone(),
        ..Default::default()
    };
    let buffer = PublicationBuffer {
        max_records: 1,
        max_bytes: 1024 * 1024,
    };
    for reverse in [false, true] {
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let publish = |repository: &mut SqliteKnowledgeRepository, store: KnowledgeStore| {
            let mut batches = KnowledgeStoreBatches::new(store, buffer)
                .unwrap()
                .collect::<Vec<_>>();
            if reverse {
                batches.reverse();
            }
            repository.replace_owner_buffered(
                "coordinate-fixture",
                &mut batches.into_iter(),
                buffer,
            )
        };
        let first = publish(&mut repository, store.clone()).unwrap();
        assert_eq!(first.batches, 3);
        assert_eq!(first.peak_batch_records, 1);
        let before = serde_json::to_value(repository.load().unwrap()).unwrap();
        let visibility = repository.visibility_changes(None, 100).unwrap();
        assert!(publish(&mut repository, store.clone())
            .unwrap()
            .outcome
            .is_unchanged());
        for damage in 0..5 {
            let mut invalid = store.clone();
            match damage {
                0 => invalid.artifacts.clear(),
                1 => invalid.locations.clear(),
                2 => invalid.attachments[0].verified_span = None,
                3 => invalid.attachments[0].artifact_id = "0".repeat(64),
                _ => {
                    invalid.attachments[0].disposition =
                        entmoot_contracts::AttachmentDisposition::Unresolved
                }
            }
            assert!(
                publish(&mut repository, invalid).is_err(),
                "damage={damage}, reverse={reverse}"
            );
            assert_eq!(
                serde_json::to_value(repository.load().unwrap()).unwrap(),
                before
            );
            assert_eq!(
                repository.visibility_changes(None, 100).unwrap(),
                visibility
            );
        }
        assert!(publish(&mut repository, store.clone())
            .unwrap()
            .outcome
            .is_unchanged());
    }
}
