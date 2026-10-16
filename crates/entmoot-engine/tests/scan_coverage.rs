#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_engine::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};

#[test]
fn gitignore_exclusions_survive_publication_and_incremental_reuse() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("generated")).unwrap();
    for path in ["keep.py", "drop.py", "generated/api.py"] {
        std::fs::write(root.path().join(path), "def leaf(): return 1\n").unwrap();
    }
    let base =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    let keep_ids = |graph: &entmoot_engine::TopologyGraph| {
        graph
            .nodes
            .iter()
            .filter(|node| node.path.as_deref() == Some("keep.py"))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>()
    };
    std::fs::write(
        root.path().join(".gitignore"),
        "generated/\ndrop.py\n.gitignore\n",
    )
    .unwrap();
    let head = entmoot_engine::build_topology_from_repo_reusing_base(
        root.path(),
        Default::default(),
        &base,
    )
    .unwrap();
    assert_eq!(keep_ids(&head), keep_ids(&base));
    assert!(!head
        .nodes
        .iter()
        .any(|node| matches!(node.path.as_deref(), Some("drop.py" | "generated/api.py"))));
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    db.replace_all(&KnowledgeStore::extract_from_graph(&head))
        .unwrap();
    let saved = serde_json::to_value(db.load().unwrap()).unwrap();
    let coverage = &saved["coverages"][0];
    assert_eq!(coverage["measurement"]["observed_files"], 2);
    assert_eq!(coverage["measurement"]["parsed"], 1);
    assert_eq!(coverage["measurement"]["failed"], 0);
    assert_eq!(
        coverage["measurement"]["excluded_directories"],
        serde_json::json!(["generated"])
    );
    let excludes = coverage["scope"]["excluded_paths"].as_array().unwrap();
    assert!(excludes.iter().any(|path| path == "drop.py"));
    assert!(excludes.iter().any(|path| path == "generated/**"));
    // The policy remains a snapshot input even when it matches its own rules.
    assert!(head
        .snapshot_files
        .iter()
        .any(|file| file.path == ".gitignore"));
    std::fs::write(root.path().join(".gitignore"), "").unwrap();
    let restored = entmoot_engine::build_topology_from_repo_reusing_base(
        root.path(),
        Default::default(),
        &head,
    )
    .unwrap();
    for path in ["drop.py", "generated/api.py"] {
        assert!(restored
            .nodes
            .iter()
            .any(|node| node.path.as_deref() == Some(path)));
    }
    assert_eq!(keep_ids(&restored), keep_ids(&base));
}

#[test]
fn dotnet_build_directories_do_not_drift_source_but_authored_siblings_remain() {
    let root = tempfile::tempdir().unwrap();
    let sources = [
        "dotnet/Api.cs",
        "dotnet/obj/Debug/Generated.cs",
        "dotnet/BIN/Release/Generated.cs",
        "authored/obj/Api.cs",
        "authored/bin/Api.cs",
        "dotnet/src/bin/Api.cs",
    ];
    for path in sources {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "class Api { int Run() { return 1; } }").unwrap();
    }
    std::fs::write(root.path().join("dotnet/Api.csproj"), "<Project />").unwrap();
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    let before = entmoot_core::scan::snapshot_file_digests(root.path()).unwrap();
    for path in sources {
        let present = graph.nodes.iter().any(|n| n.path.as_deref() == Some(path));
        assert_eq!(
            present,
            !path.contains("/Debug/") && !path.contains("/Release/"),
            "{path}"
        );
    }
    let store = KnowledgeStore::extract_from_graph(&graph);
    let coverage = serde_json::to_value(&store.coverages[0]).unwrap();
    let excluded = coverage["measurement"]["excluded_directories"]
        .as_array()
        .unwrap();
    assert!(excluded.iter().any(|path| path == "dotnet/obj"));
    assert!(excluded.iter().any(|path| path == "dotnet/BIN"));
    std::fs::write(root.path().join("dotnet/obj/Debug/Generated.cs"), "changed").unwrap();
    let after = entmoot_core::scan::snapshot_file_digests(root.path()).unwrap();
    let fingerprints = |files: Vec<entmoot_core::scan::SnapshotFile>| {
        files
            .into_iter()
            .map(|file| (file.path, file.content_digest))
            .collect::<Vec<_>>()
    };
    assert_eq!(fingerprints(before), fingerprints(after));
}

#[test]
fn shipped_build_inputs_and_exclusions_have_durable_replayable_coverage() {
    let root = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "dist/api.js",
            "export function api() { return use(value); }",
        ),
        ("build/api.py", "def api():\n    return use(value)\n"),
        ("generated/api.py", "def api():\n    return use(value)\n"),
        ("dist/api.min.js", "function api(){return use(value)}"),
        (
            "node_modules/dependency/api.js",
            "function api(){return use(value)}",
        ),
    ] {
        std::fs::create_dir_all(root.path().join(path).parent().unwrap()).unwrap();
        std::fs::write(root.path().join(path), source).unwrap();
    }
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    for path in [
        "dist/api.js",
        "build/api.py",
        "generated/api.py",
        "dist/api.min.js",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.path.as_deref() == Some(path)
                    && n.kind == entmoot_engine::NodeKind::Callable),
            "{path}"
        );
    }
    let store = KnowledgeStore::extract_from_graph(&graph);
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    db.replace_all(&store).unwrap();
    let saved = serde_json::to_value(db.load().unwrap()).unwrap();
    let coverage = &saved["coverages"][0];
    assert_eq!(coverage["measurement"]["kind"], "structural_scan");
    assert_eq!(coverage["measurement"]["parsed"], 4);
    assert_eq!(coverage["measurement"]["heuristic_skipped"], 0);
    assert!(coverage["scope"]["excluded_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "**/node_modules/**"));
    assert!(coverage["measurement"]["excluded_directories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "node_modules"));
    let incremental = entmoot_engine::build_topology_from_repo_reusing_base(
        root.path(),
        Default::default(),
        &graph,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(KnowledgeStore::extract_from_graph(&incremental)).unwrap()
            ["coverages"],
        saved["coverages"]
    );
}

#[test]
fn all_claimed_families_record_inclusions_exclusions_and_invalid_inputs() {
    for (file, source) in [
        ("a.rs", "fn caller() { run(1); }"),
        ("a.py", "def caller():\n    run(1)\n"),
        ("a.js", "function caller() { run(1); }"),
        ("a.ts", "function caller() { run(1); }"),
        ("a.tsx", "function caller() { run(1); }"),
        ("a.go", "package p\nfunc caller() { run(1) }"),
        ("A.java", "class A { void caller() { run(1); } }"),
        ("A.cs", "class A { void caller() { run(1); } }"),
        ("a.c", "void caller() { run(1); }"),
    ] {
        let root = tempfile::tempdir().unwrap();
        for directory in ["dist", "build", "generated", "node_modules"] {
            std::fs::create_dir(root.path().join(directory)).unwrap();
            let comment = if file.ends_with(".py") { "#" } else { "//" };
            let decorated = format!(
                "{comment} Code generated by fixture tool. DO NOT EDIT.\n{source}\n{comment} {}\n",
                "long line ".repeat(200)
            );
            std::fs::write(root.path().join(directory).join(file), decorated).unwrap();
        }
        std::fs::write(root.path().join(file), [0xffu8]).unwrap();
        let graph =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let store = KnowledgeStore::extract_from_graph(&graph);
        let coverage = serde_json::to_value(&store.coverages[0]).unwrap();
        assert_eq!(coverage["measurement"]["parsed"], 3, "{file}");
        assert_eq!(coverage["measurement"]["heuristic_skipped"], 0, "{file}");
        assert_eq!(coverage["measurement"]["failed"], 1, "{file}");
        let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
        db.replace_all(&store).unwrap();
        let mut invalid = store.clone();
        if let entmoot_contracts::CoverageMeasurement::StructuralScan { parsed, .. } =
            &mut invalid.coverages[0].measurement
        {
            *parsed += 1;
        }
        invalid.coverages[0].coverage_id = invalid.coverages[0].derive_id().unwrap();
        assert!(db.replace_all(&invalid).is_err());
        assert_eq!(db.load().unwrap().coverages, store.coverages);
        std::fs::write(root.path().join(file), source).unwrap();
        let changed = entmoot_engine::build_topology_from_repo_reusing_base(
            root.path(),
            Default::default(),
            &graph,
        )
        .unwrap();
        assert_ne!(
            changed.coverages[0].coverage_id,
            store.coverages[0].coverage_id
        );
        assert_eq!(
            serde_json::to_value(changed.coverages[0].clone()).unwrap()["measurement"]["parsed"],
            4
        );
    }
}

#[test]
fn old_heuristic_publication_cannot_hide_unchanged_source_on_reuse() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("api.pb.py"),
        "# generated by fixture\ndef api():\n    return 1\n",
    )
    .unwrap();
    let mut base =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    base.nodes
        .retain(|n| n.kind != entmoot_engine::NodeKind::Callable);
    base.edges.retain(|e| {
        base.nodes.iter().any(|n| n.id == e.from) && base.nodes.iter().any(|n| n.id == e.to)
    });
    for node in &mut base.nodes {
        node.file_hash = None;
    }
    if let entmoot_contracts::CoverageMeasurement::StructuralScan { method_id, .. } =
        &mut base.coverages[0].measurement
    {
        *method_id = "entmoot.scan.heuristic_structural_skip.v1".into();
    }
    base.rebuild_indexes();
    let head = entmoot_engine::build_topology_from_repo_reusing_base(
        root.path(),
        Default::default(),
        &base,
    )
    .unwrap();
    assert!(head
        .nodes
        .iter()
        .any(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == "api"));
}

#[test]
fn old_inventory_policy_forces_reparse_of_unchanged_property_functions() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("api.js"),
        "const obj = {}; obj.method = function() { return 1; };",
    )
    .unwrap();
    let fresh =
        entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    let mut old = fresh;
    for coverage in &mut old.coverages {
        if let entmoot_contracts::CoverageMeasurement::StructuralScan { method_id, .. } =
            &mut coverage.measurement
        {
            *method_id = "entmoot.scan.structural.v2".into();
        }
    }
    old.nodes
        .retain(|node| node.kind != entmoot_engine::NodeKind::Callable);
    let rebuilt = entmoot_engine::build_topology_from_repo_reusing_base(
        root.path(),
        Default::default(),
        &old,
    )
    .unwrap();
    assert!(rebuilt
        .nodes
        .iter()
        .any(|node| node.kind == entmoot_engine::NodeKind::Callable && node.label == "obj.method"));
}
