#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_engine::{
    KnowledgeRead, KnowledgeRepository, KnowledgeStore, NodeKind, SqliteKnowledgeRepository,
};

const CASES: &[(&str, &str)] = &[
    ("a.rs", "const LIMIT: i32 = -1; fn get() -> i32 { LIMIT }"),
    ("a.py", "LIMIT = -1\ndef get():\n    return LIMIT\n"),
    ("a.js", "const LIMIT = -1; function get() { return LIMIT; }"),
    (
        "a.ts",
        "const LIMIT: number = -1; function get() { return LIMIT; }",
    ),
    (
        "a.tsx",
        "const LIMIT: number = -1; function get() { return LIMIT; }",
    ),
    (
        "a.go",
        "package p\nconst LIMIT = -1\nfunc get() int { return LIMIT }",
    ),
    (
        "A.java",
        "class Config { static final int LIMIT = -1; int get() { return LIMIT; } }",
    ),
    (
        "A.cs",
        "class Config { const int LIMIT = -1; int get() { return LIMIT; } }",
    ),
    (
        "a.c",
        "const int LIMIT = -1; int get(void) { return LIMIT; }",
    ),
];

#[test]
fn revision_sequence_invalidates_negative_lookups_and_reuses_unchanged_targets() {
    use entmoot_engine::workspace::{initialize_workspace, BindOptions};
    let root = tempfile::tempdir().unwrap();
    let app = "import { leaf } from './leaf'; export function call() { return leaf(); }";
    let leaf = "export function leaf() { return 1; }";
    std::fs::write(root.path().join("app.ts"), app).unwrap();
    std::fs::write(
        root.path().join("stable.ts"),
        "function local() { return 0; } function stable() { return local(); }",
    )
    .unwrap();
    let options = BindOptions {
        structural_only: true,
        ..Default::default()
    };
    let mut prior_snapshot = None;
    // Independent expectations: (syntax reuse, call reuse, selected target, ambiguity).
    for (step, (reused, stitched, target, ambiguous)) in [
        (0, 0, None, false),
        (2, 2, None, false),
        (2, 1, Some("leaf.ts"), false),
        (2, 2, Some("leaf.ts"), false),
        (2, 1, Some("leaf.ts"), false),
        (3, 2, None, true),
        (3, 2, Some("leaf.js"), false),
        (2, 2, None, false), // Restore the complete first publication, including both call summaries.
        (1, 1, None, false),
        // Re-adding identical path/content reuses its historical sealed file unit.
        (3, 2, None, false),
        (2, 2, None, false),
    ]
    .into_iter()
    .enumerate()
    {
        match step {
            2 => std::fs::write(root.path().join("leaf.ts"), leaf).unwrap(),
            3 => std::fs::write(root.path().join("leaf.ts"), leaf.replace('1', "2")).unwrap(),
            4 => std::fs::write(
                root.path().join("leaf.ts"),
                format!("// moved definition\n{leaf}"),
            )
            .unwrap(),
            5 => std::fs::write(root.path().join("leaf.js"), leaf).unwrap(),
            6 => std::fs::remove_file(root.path().join("leaf.ts")).unwrap(),
            7 => std::fs::remove_file(root.path().join("leaf.js")).unwrap(),
            8 => std::fs::write(root.path().join("app.ts"), app.replace("./leaf", "./other"))
                .unwrap(),
            9 => std::fs::write(root.path().join("leaf.ts"), leaf).unwrap(),
            10 => {
                std::fs::rename(root.path().join("app.ts"), root.path().join("renamed.ts")).unwrap()
            }
            _ => {}
        }
        let summary = initialize_workspace(
            root.path(),
            Some("revision-trace"),
            &Default::default(),
            &options,
        )
        .unwrap();
        assert_eq!(summary.reused_syntax_files, reused, "parse step {step}");
        assert_eq!(
            summary.reused_extraction_files,
            [0, 0, 2, 2, 2, 3, 3, 0, 1, 3, 2][step],
            "extract step {step}",
        );
        assert_eq!(
            summary.reused_stitching_files, stitched,
            "stitch step {step}"
        );
        let graph = entmoot_engine::load_graph_json(&summary.publication.graph).unwrap();
        let cold = entmoot_engine::build_topology_from_repo_with_options(
            root.path(),
            entmoot_engine::TopologyBuildOptions {
                repository: Some("revision-trace"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&graph).unwrap(),
            serde_json::to_value(cold).unwrap(),
            "cold parity step {step}"
        );
        let caller = graph
            .nodes
            .iter()
            .find(|n| n.label == "call" && n.kind == NodeKind::Callable)
            .unwrap();
        let targets: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| {
                e.resolved && e.kind == entmoot_engine::EdgeKind::Calls && e.from == caller.id
            })
            .map(|e| {
                graph
                    .nodes
                    .iter()
                    .find(|n| n.id == e.to)
                    .unwrap()
                    .path
                    .as_deref()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            targets,
            target.into_iter().collect::<Vec<_>>(),
            "target step {step}"
        );
        assert_eq!(
            graph
                .unresolved_relations
                .iter()
                .any(|u| u.reason == entmoot_contracts::UnresolvedReason::AmbiguousTarget),
            ambiguous,
            "ambiguity step {step}"
        );
        if step > 1 {
            assert_ne!(
                prior_snapshot.as_ref(),
                Some(&graph.snapshot),
                "changed source must get fresh scope"
            );
        }
        prior_snapshot = Some(graph.snapshot);
    }
}

#[test]
fn workspace_syntax_cache_skips_unchanged_parses_and_reindexes_full_changed_files() {
    use entmoot_engine::workspace::{initialize_workspace, BindOptions};
    let root = tempfile::tempdir().unwrap();
    for &(path, source) in CASES {
        std::fs::write(root.path().join(path), source).unwrap();
    }
    let options = BindOptions {
        structural_only: true,
        ..Default::default()
    };
    let first = initialize_workspace(
        root.path(),
        Some("all-families"),
        &Default::default(),
        &options,
    )
    .unwrap();
    assert_eq!(first.indexed_files, CASES.len());
    let second = initialize_workspace(
        root.path(),
        Some("all-families"),
        &Default::default(),
        &options,
    )
    .unwrap();
    assert_eq!(second.indexed_files, 0);
    assert_eq!(second.reused_syntax_files, CASES.len());
    assert_eq!(second.reused_extraction_files, 0);
    assert_eq!(second.reused_stitching_files, CASES.len());
    assert_eq!(second.publication.graph, first.publication.graph);
    assert!(
        second.phases.iter().all(|phase| !matches!(
            phase.phase.as_str(),
            "syntax_projection" | "structural_resolution" | "sealed_publication"
        )),
        "unchanged inputs must reuse the sealed publication before graph construction"
    );
    // Equal source bytes do not imply equal scan coverage.
    std::fs::create_dir(root.path().join(".tmp")).unwrap();
    let exclusions = initialize_workspace(
        root.path(),
        Some("all-families"),
        &Default::default(),
        &options,
    )
    .unwrap();
    assert_eq!(exclusions.snapshot, second.snapshot);
    assert_eq!(exclusions.publication_outcome, "replaced");
    assert_ne!(exclusions.publication.graph, second.publication.graph);
    for &(path, source) in CASES {
        std::fs::write(root.path().join(path), source.replace("-1", "8192")).unwrap();
        let next = initialize_workspace(
            root.path(),
            Some("all-families"),
            &Default::default(),
            &options,
        )
        .unwrap();
        assert_eq!(next.indexed_files, 1, "{path}");
        assert_eq!(next.reused_syntax_files, CASES.len() - 1, "{path}");
        assert_eq!(next.reused_extraction_files, CASES.len() - 1, "{path}");
        assert_eq!(next.reused_stitching_files, CASES.len() - 1, "{path}");
        let graph = entmoot_engine::load_graph_json(&next.publication.graph).unwrap();
        let cold = entmoot_engine::build_topology_from_repo_with_options(
            root.path(),
            entmoot_engine::TopologyBuildOptions {
                repository: Some("all-families"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(graph).unwrap(),
            serde_json::to_value(cold).unwrap(),
            "{path}"
        );
    }
}

#[test]
fn graph_fragment_membership_is_verified_before_reuse_or_publication() {
    use entmoot_engine::workspace::{initialize_workspace, BindOptions};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("first.py"),
        "def first(): return missing()\n",
    )
    .unwrap();
    std::fs::write(root.path().join("second.py"), "def second(): return 2\n").unwrap();
    let options = BindOptions {
        structural_only: true,
        ..Default::default()
    };
    let initial = initialize_workspace(
        root.path(),
        Some("cache-integrity"),
        &Default::default(),
        &options,
    )
    .unwrap();
    let pointer = std::fs::read(
        entmoot_engine::workspace::storage_directory(root.path())
            .unwrap()
            .join("current.json"),
    )
    .unwrap();
    let locator: entmoot_engine::KnowledgeViewLocator =
        serde_json::from_slice(&std::fs::read(&initial.publication.store).unwrap()).unwrap();
    let db = rusqlite::Connection::open(&locator.database).unwrap();
    let first: i64 = db
        .query_row(
            "SELECT parts FROM file_fact_membership WHERE path='first.py'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let second: i64 = db
        .query_row(
            "SELECT parts FROM file_fact_membership WHERE path='second.py'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(
        first, second,
        "corruption control requires distinct call summaries"
    );
    for sql in [
        "UPDATE file_fact_membership SET parts=(SELECT parts FROM file_fact_membership WHERE path='second.py') WHERE path='first.py'",
        "UPDATE file_fact_membership SET parts=NULL WHERE path='first.py'",
    ] {
        db.execute(sql, []).unwrap();
        let error = initialize_workspace(root.path(), Some("cache-integrity"), &Default::default(), &options).unwrap_err();
        assert!(error.to_string().contains("membership commitment"), "{error}");
        assert_eq!(std::fs::read(entmoot_engine::workspace::storage_directory(root.path()).unwrap().join("current.json")).unwrap(), pointer);
        db.execute("UPDATE file_fact_membership SET parts=?1 WHERE path='first.py'", [first]).unwrap();
    }
    let restored = initialize_workspace(
        root.path(),
        Some("cache-integrity"),
        &Default::default(),
        &options,
    )
    .unwrap();
    assert_eq!(restored.reused_extraction_files, 0);
    assert_eq!(restored.publication.graph, initial.publication.graph);
}

#[test]
fn unchanged_caller_syntax_rebinds_after_sibling_module_ambiguity() {
    use entmoot_engine::workspace::{initialize_workspace, BindOptions};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.ts"),
        "import { leaf } from './leaf'; export function call() { return leaf(); }",
    )
    .unwrap();
    std::fs::write(
        root.path().join("leaf.ts"),
        "export function leaf() { return 1; }",
    )
    .unwrap();
    let options = BindOptions {
        structural_only: true,
        ..Default::default()
    };
    initialize_workspace(root.path(), Some("fixture"), &Default::default(), &options).unwrap();
    std::fs::write(
        root.path().join("leaf.js"),
        "export function leaf() { return 2; }",
    )
    .unwrap();
    let next =
        initialize_workspace(root.path(), Some("fixture"), &Default::default(), &options).unwrap();
    assert_eq!(next.indexed_files, 1);
    assert_eq!(next.reused_syntax_files, 2);
    assert_eq!(next.reused_extraction_files, 2);
    assert_eq!(next.reused_stitching_files, 1);
    let graph = entmoot_engine::load_graph_json(&next.publication.graph).unwrap();
    let cold = entmoot_engine::build_topology_from_repo_with_options(
        root.path(),
        entmoot_engine::TopologyBuildOptions {
            repository: Some("fixture"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(graph).unwrap(),
        serde_json::to_value(cold).unwrap()
    );
}

#[test]
fn initializers_survive_sqlite_rebuild_and_seed_impact_across_claimed_families() {
    for &(path, source) in CASES {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(path), source).unwrap();
        let base =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        base.validate().unwrap();
        let declaration = base
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Declaration && n.label == "LIMIT")
            .unwrap();
        let encoded = serde_json::to_value(declaration).unwrap();
        assert_eq!(
            encoded["declaration"]["syntax"]["initializer"]["value"],
            serde_json::json!({"kind":"decimal_integer", "text":"-1"}),
            "{path}"
        );
        let store = KnowledgeStore::extract_from_graph(&base);
        let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
        db.replace_all(&store).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap()["declaration_sites"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut skeleton = entmoot_engine::inventory_skeleton(&base);
        skeleton.nodes.retain(|n| n.declaration.is_none());
        skeleton.rebuild_indexes();
        let replay = loaded.apply_to_inventory(skeleton);
        replay.validate().unwrap();
        let restored = replay
            .nodes
            .iter()
            .find(|n| n.id == declaration.id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap()["declaration"],
            encoded["declaration"]
        );
        std::fs::write(root.path().join(path), source.replace("-1", "8192")).unwrap();
        let head = entmoot_engine::build_topology_from_repo_reusing_base(
            root.path(),
            Default::default(),
            &base,
        )
        .unwrap();
        let impact = entmoot_engine::compare_revision_impact(&base, &head).unwrap();
        assert!(impact.callables.is_empty(), "{path}");
        let impact = serde_json::to_value(impact).unwrap();
        let deltas = impact["declarations"].as_array().unwrap();
        assert_eq!(deltas.len(), 1, "{path}");
        assert_eq!(deltas[0]["kind"], "initializer_changed", "{path}");
        assert_eq!(
            deltas[0]["base"]["syntax"]["initializer"]["value"],
            serde_json::json!({"kind":"decimal_integer", "text":"-1"})
        );
        assert_eq!(
            deltas[0]["head"]["syntax"]["initializer"]["value"],
            serde_json::json!({"kind":"decimal_integer", "text":"8192"})
        );
        // The same file invalidation reparses full-file inventory; a subsequent
        // digest-gated splice must equal a cold build for every claimed syntax.
        let full =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let repeat = entmoot_engine::build_topology_from_repo_reusing_base(
            root.path(),
            Default::default(),
            &head,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&head).unwrap(),
            serde_json::to_value(&full).unwrap(),
            "{path}"
        );
        assert_eq!(
            serde_json::to_value(&repeat).unwrap(),
            serde_json::to_value(&full).unwrap(),
            "{path}"
        );
    }
}

#[test]
fn exact_integer_find_retains_all_occurrences_and_never_evaluates_expressions() {
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    for &(path, source) in CASES {
        std::fs::write(root.path().join(path), source).unwrap();
    }
    std::fs::write(
        root.path().join("dynamic.py"),
        "LIMIT = compute(-1)\nOTHER = -1 + 0\nTEXT = '-1'\n",
    )
    .unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    let db_path = root.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&db_path)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let engine = entmoot_engine::QueryEngine::new(graph, &db_path);
    let query =
        serde_json::from_value(json!({"kind":"initializer", "integer":"-1", "limit":2})).unwrap();
    let result = engine.find(query).unwrap().into_value().unwrap();
    assert_eq!(result["bounds"]["total_matches"], 9);
    assert_eq!(result["status"], "truncated");
    assert_eq!(result["items"].as_array().unwrap().len(), 2);
    assert!(!result["gaps"].as_array().unwrap().is_empty());
    let query =
        serde_json::from_value(json!({"kind":"initializer", "integer":"-1", "path":"A.java"}))
            .unwrap();
    let result = engine.find(query).unwrap().into_value().unwrap();
    assert_eq!(result["bounds"]["total_matches"], 1);
    let query =
        serde_json::from_value(json!({"kind":"subject", "query": "LIMIT", "path":"A.java"}))
            .unwrap();
    let subject = engine.find(query).unwrap().into_value().unwrap();
    assert_eq!(
        result["items"][0]["node_id"],
        subject["items"][0]["node_id"]
    );
    let inspect =
        serde_json::from_value(json!({"kind":"subject", "id":result["items"][0]["node_id"]}))
            .unwrap();
    let inspected = engine.inspect(inspect).unwrap().into_value().unwrap();
    assert_eq!(
        inspected["declaration"]["syntax"]["initializer"]["value"],
        serde_json::json!({"kind":"decimal_integer", "text":"-1"})
    );
    for integer in ["100", "1 + 2", "01", "-0"] {
        let query =
            serde_json::from_value(json!({"kind":"initializer", "integer":integer})).unwrap();
        let result = engine.find(query);
        if integer == "100" {
            assert_eq!(result.unwrap().into_value().unwrap()["status"], "unknown");
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn declaration_owners_validation_and_stale_replay_are_consistent_across_families() {
    for &(path, source) in CASES {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(path), source).unwrap();
        let graph =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let sites = KnowledgeStore::extract_from_graph(&graph).declaration_sites;
        let store = KnowledgeStore {
            declaration_sites: sites.clone(),
            ..Default::default()
        };
        let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut batches = entmoot_engine::KnowledgeStoreBatches::new(
            store.clone(),
            entmoot_engine::PublicationBuffer {
                max_records: 1,
                max_bytes: 16384,
            },
        )
        .unwrap();
        db.replace_owner_buffered(
            "first",
            &mut batches,
            entmoot_engine::PublicationBuffer {
                max_records: 1,
                max_bytes: 16384,
            },
        )
        .unwrap();
        db.replace_owner("second", &store, &[]).unwrap();
        db.remove_owner("first").unwrap();
        assert_eq!(db.load().unwrap().declaration_sites, sites, "{path}");
        let before = db.visibility_changes(None, 100).unwrap();
        for mutation in 0..7 {
            let mut invalid = store.clone();
            let site = &mut invalid.declaration_sites[0];
            match mutation {
                0 => {
                    site.marker
                        .syntax
                        .as_mut()
                        .unwrap()
                        .initializer
                        .as_mut()
                        .unwrap()
                        .value =
                        entmoot_contracts::InitializerValue::DecimalInteger { text: "999".into() }
                }
                1 => site.marker.scope.push("forged_scope".into()),
                2 => {
                    site.marker.syntax.as_mut().unwrap().name_span.end_byte =
                        site.marker.site.span.end_byte + 1
                }
                3 => site.content_digest = entmoot_contracts::ContentDigest::from_trusted("bad"),
                4 => site.marker.site.occurrence.logic_address.push('x'),
                5 => site.marker.site.span.end_byte = site.marker.site.span.start_byte,
                _ => site.marker.callable = Some(site.marker.site.clone()),
            }
            assert!(
                db.replace_owner("second", &invalid, &[]).is_err(),
                "{path}: mutation {mutation}"
            );
            let mut empty = SqliteKnowledgeRepository::open_in_memory().unwrap();
            assert!(
                empty.replace_all(&invalid).is_err(),
                "fresh {path}: mutation {mutation}"
            );
            assert_eq!(db.load().unwrap().declaration_sites, sites);
            assert_eq!(db.visibility_changes(None, 100).unwrap(), before);
        }
        let mut stale = entmoot_engine::inventory_skeleton(&graph);
        stale.nodes.retain(|n| n.declaration.is_none());
        stale.snapshot.revision = "other-revision".into();
        stale.rebuild_indexes();
        assert!(store
            .apply_to_inventory(stale)
            .nodes
            .iter()
            .all(|n| n.declaration.is_none()));
        db.remove_owner("second").unwrap();
        assert!(db.load().unwrap().declaration_sites.is_empty());
        let inactive = db
            .load()
            .unwrap()
            .apply_to_inventory(entmoot_engine::inventory_skeleton(&graph));
        assert!(
            inactive.nodes.iter().all(|node| node.declaration.is_none()),
            "inactive initializer resurrected for {path}"
        );
    }
}

#[test]
fn relocation_and_missing_inventory_never_become_runtime_or_breaking_claims() {
    for &(path, source) in CASES {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(path), source).unwrap();
        let base =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        std::fs::write(root.path().join(path), format!("\n\n{source}")).unwrap();
        let head =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let impact = entmoot_engine::compare_revision_impact(&base, &head).unwrap();
        assert!(impact.declarations.is_empty(), "{path}");
        std::fs::write(root.path().join(path), [0xff]).unwrap();
        let invalid =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let impact = entmoot_engine::compare_revision_impact(&base, &invalid).unwrap();
        assert_eq!(impact.declarations.len(), 1);
        assert_eq!(
            impact.declarations[0].kind,
            entmoot_contracts::query::DeclarationImpactKind::ObservedOnlyInBase
        );
        assert!(
            serde_json::to_value(&impact).unwrap()["breaking_change_signals"]["signals"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(impact
            .gaps
            .iter()
            .any(|g| g.contains("declaration_inventory_is_partial")));
    }
}

#[test]
fn old_scan_policy_rebuilds_initializer_inventory_without_changing_callable_identity() {
    for &(path, source) in CASES {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(path), source).unwrap();
        let fresh =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let mut old = fresh.clone();
        old.nodes.retain(|node| node.declaration.is_none());
        old.edges.retain(|edge| {
            old.nodes.iter().any(|n| n.id == edge.from) && old.nodes.iter().any(|n| n.id == edge.to)
        });
        for coverage in &mut old.coverages {
            if let entmoot_contracts::CoverageMeasurement::StructuralScan { method_id, .. } =
                &mut coverage.measurement
            {
                *method_id = "entmoot.scan.structural.v3".into();
                coverage.coverage_id = coverage.derive_id().unwrap();
            }
        }
        old.rebuild_indexes();
        let rebuilt = entmoot_engine::build_topology_from_repo_reusing_base(
            root.path(),
            Default::default(),
            &old,
        )
        .unwrap();
        assert_eq!(
            KnowledgeStore::extract_from_graph(&fresh).declaration_sites,
            KnowledgeStore::extract_from_graph(&rebuilt).declaration_sites,
            "{path}"
        );
        assert!(entmoot_engine::compare_revision_impact(&fresh, &rebuilt)
            .unwrap()
            .callables
            .is_empty());
    }
}
