//! Production inventory, authored graph facts and conservative assessment controls.
use entmoot_contracts::*;
use entmoot_contracts::{TestRunner, TestTarget, UnusedSignal};
use entmoot_engine::query::{assess_test_impact, assess_unused};
use entmoot_engine::QueryEngine;

#[allow(clippy::unwrap_used)]
fn graph(root: &std::path::Path, source: &str) -> entmoot_engine::TopologyGraph {
    std::fs::write(root.join("lib.rs"), source).unwrap();
    entmoot_engine::build_topology_from_repo_with_options(
        root,
        entmoot_engine::TopologyBuildOptions {
            repository: Some("local-assessment"),
            ..Default::default()
        },
    )
    .unwrap()
}
fn target(id: &str, owner: &str) -> TestTarget {
    TestTarget {
        id: id.into(),
        runner: TestRunner::Cargo,
        command: vec!["cargo".into(), "test".into(), "--test".into(), id.into()],
        owners: vec![owner.into()],
        fixtures: vec![],
        occurrences: vec![],
    }
}

#[allow(clippy::unwrap_used)]
fn declaration(graph: &entmoot_engine::TopologyGraph, label: &str) -> AssessmentDeclaration {
    let node = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == label)
        .unwrap();
    AssessmentDeclaration {
        path: node.path.clone().unwrap(),
        source_digest: ContentDigest::parse(
            graph
                .snapshot_files
                .iter()
                .find(|f| Some(&f.path) == node.path.as_ref())
                .unwrap()
                .content_digest
                .clone(),
        )
        .unwrap(),
        declaration: SourceOccurrenceRef {
            occurrence: OccurrenceRef::new(node.logic_address.clone().unwrap()).unwrap(),
            span: SiteSpan {
                start_byte: node.start_byte.unwrap(),
                end_byte: node.end_byte.unwrap(),
            },
        },
    }
}

#[allow(clippy::unwrap_used)]
fn attach_evidence(
    graph: &mut entmoot_engine::TopologyGraph,
    observations: Vec<AssessmentObservation>,
) {
    let output = tempfile::tempdir().unwrap();
    let producer = ProducerRef {
        id: "authored-provider".into(),
        name: "authored-provider".into(),
        version: Some("1".into()),
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![ProducerCapabilityKind::ArtifactAdapter],
    };
    let artifact = AssessmentEvidence {
        schema_version: "entmoot.assessment-evidence.v1".into(),
        implementation: entmoot_engine::implementation_fingerprint(),
        snapshot: graph.snapshot.clone(),
        producer,
        tools: vec![],
        build_context: ContentDigest::parse(format!("blake3:{}", "a".repeat(64))).unwrap(),
        observations,
    };
    let bytes = serde_json::to_vec(&artifact).unwrap();
    let mut manifest = ArtifactManifest::build(
        ArtifactFormat::AssessmentEvidence,
        ArtifactProducer {
            name: artifact.producer.name.clone(),
            version: artifact.producer.version.clone(),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        graph.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "assessment",
            vec![],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        ),
        blake3_hex(&bytes),
        Some(artifact.build_context.to_string()),
    )
    .unwrap();
    manifest.artifact_path = Some("evidence.json".into());
    std::fs::write(output.path().join("evidence.json"), bytes).unwrap();
    let path = output.path().join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    entmoot_engine::ingest_artifact_manifest(graph, &path, false).unwrap();
}

#[test]
fn observed_escapes_exposure_and_references_prevent_unreferenced_candidates() {
    let root = tempfile::tempdir().unwrap();
    for usage in [
        CodeUseKind::Escape,
        CodeUseKind::NonCallReference,
        CodeUseKind::Exposure,
        CodeUseKind::Entrypoint,
    ] {
        let mut graph = graph(
            root.path(),
            "fn callback() {} fn warning() {} fn sibling() {}",
        );
        let callback = declaration(&graph, "callback");
        let warning = declaration(&graph, "warning");
        attach_evidence(
            &mut graph,
            vec![
                AssessmentObservation::CodeUse {
                    subject: callback,
                    usage,
                },
                AssessmentObservation::CompilerUnused {
                    subject: warning,
                    diagnostic_code: "dead_code".into(),
                    message: "function is never used in selected build".into(),
                },
            ],
        );
        let report = assess_unused(&graph, UnusedRequest::default()).unwrap();
        assert!(!report
            .candidates
            .iter()
            .flat_map(|c| &c.members)
            .any(|m| m.label == "callback"));
        assert_eq!(
            report.candidates[0].signal,
            UnusedSignal::CompilerDiagnostic
        );
        assert!(!report.candidates[0].evidence.is_empty());
        assert!(!report.completeness.established());
    }
}

#[test]
fn shifted_declarations_and_stale_claims_cannot_change_assessments() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = graph(root.path(), "fn callback() {} fn sibling() {}");
    let mut subject = declaration(&graph, "callback");
    subject.declaration.span.start_byte += 1;
    attach_evidence(
        &mut graph,
        vec![AssessmentObservation::CodeUse {
            subject,
            usage: CodeUseKind::Escape,
        }],
    );
    assert!(graph
        .diagnostics
        .iter()
        .any(|d| d.code == "assessment_declaration_unmatched"));
    assert_eq!(
        assess_unused(&graph, UnusedRequest::default())
            .unwrap()
            .candidates
            .len(),
        2
    );
    let subject = declaration(&graph, "callback");
    attach_evidence(
        &mut graph,
        vec![AssessmentObservation::CodeUse {
            subject,
            usage: CodeUseKind::Escape,
        }],
    );
    for claim in &mut graph.relation_claims {
        claim.state = RelationClaimState::Retracted;
    }
    assert_eq!(
        assess_unused(&graph, UnusedRequest::default())
            .unwrap()
            .candidates
            .len(),
        2
    );
}

#[test]
fn execution_hits_add_targets_but_do_not_replace_ownership_or_assert_coverage() {
    let root = tempfile::tempdir().unwrap();
    let mut base = graph(root.path(), "fn value() { }");
    let covered = declaration(&base, "value");
    let observed = target("instrumented", "test.rs");
    attach_evidence(
        &mut base,
        vec![AssessmentObservation::TestExecution {
            target: observed.clone(),
            outcome: TestExecutionOutcome::Passed,
            covered: vec![covered],
            limitations: vec!["execution does not establish assertion coverage".into()],
        }],
    );
    let head = graph(root.path(), "fn value() { let x=1; }");
    let request = TestImpactRequest {
        targets: vec![
            target("owner", "lib.rs"),
            observed,
            target("sibling", "other.rs"),
        ],
        ..Default::default()
    };
    let report = assess_test_impact(&base, &head, request.clone()).unwrap();
    assert_eq!(
        report
            .selected
            .iter()
            .map(|s| s.target.id.as_str())
            .collect::<Vec<_>>(),
        ["instrumented", "owner"]
    );
    assert!(report.selected[0].reasons.iter().any(
        |r| matches!(r, TestSelectionReason::ExecutionCoverage { side, .. } if side == "base")
    ));
    let mut different_command = request;
    different_command.targets[1]
        .command
        .push("--ignored".into());
    assert_eq!(
        assess_test_impact(&base, &head, different_command)
            .unwrap()
            .selected
            .len(),
        1
    );
}

#[test]
fn explicit_runner_distinguishes_status_unavailability_and_source_drift() {
    use entmoot_engine::test_validation::{validate_tests, TestValidationError};
    let root = tempfile::tempdir().unwrap();
    let graph = graph(root.path(), "fn value() {}");
    let executable = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut passed = target("pass", "lib.rs");
    passed.command = vec![
        executable.clone(),
        "--exact".into(),
        "line_sensitive_changes_keep_owners_and_unaffected_siblings_stay_out".into(),
    ];
    let mut failed = target("fail", "lib.rs");
    failed.command = vec![executable, "--invalid-libtest-option".into()];
    let mut missing = target("missing", "lib.rs");
    missing.command = vec![root
        .path()
        .join("missing-tool.exe")
        .to_string_lossy()
        .into_owned()];
    let request = TestValidationRequest {
        targets: vec![passed, failed, missing],
        wall_seconds: 10,
        build_context: ContentDigest::parse(format!("blake3:{}", "a".repeat(64))).unwrap(),
    };
    let evidence = validate_tests(&graph, request.clone()).unwrap();
    for (observation, expected) in evidence.observations.iter().zip([
        TestExecutionOutcome::Passed,
        TestExecutionOutcome::Failed,
        TestExecutionOutcome::Unavailable,
    ]) {
        assert!(
            matches!(observation, AssessmentObservation::TestExecution { outcome, covered, limitations, .. } if *outcome == expected && covered.is_empty() && !limitations.is_empty())
        );
    }
    let output = tempfile::tempdir().unwrap();
    let manifest =
        entmoot_engine::test_validation::write_evidence(&evidence, output.path()).unwrap();
    let mut attached = graph.clone();
    entmoot_engine::ingest_artifact_manifest(&mut attached, &manifest, false).unwrap();
    assert_eq!(
        attached
            .relation_claims
            .iter()
            .filter(|c| matches!(c.value, RelationClaimValue::Assessment { .. }))
            .count(),
        3
    );
    std::fs::write(root.path().join("new.rs"), "fn added() {}").unwrap();
    assert!(matches!(
        validate_tests(&graph, request),
        Err(TestValidationError::SourceChanged)
    ));
}

#[test]
#[ignore = "child process fixture for bounded validation"]
fn validation_timeout_child() {
    std::thread::sleep(std::time::Duration::from_secs(30));
}

#[test]
fn explicit_validation_enforces_process_tree_deadline() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph(root.path(), "fn value() {}");
    let mut child = target("timeout", "lib.rs");
    child.command = vec![
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        "--exact".into(),
        "validation_timeout_child".into(),
        "--ignored".into(),
    ];
    let started = std::time::Instant::now();
    let evidence = entmoot_engine::test_validation::validate_tests(
        &graph,
        TestValidationRequest {
            targets: vec![child],
            wall_seconds: 1,
            build_context: ContentDigest::parse(format!("blake3:{}", "a".repeat(64))).unwrap(),
        },
    )
    .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(
        matches!(&evidence.observations[0], AssessmentObservation::TestExecution { outcome: TestExecutionOutcome::TimedOut, covered, .. } if covered.is_empty())
    );
}

#[test]
fn line_sensitive_changes_keep_owners_and_unaffected_siblings_stay_out() {
    let root = tempfile::tempdir().unwrap();
    let base = graph(root.path(), "fn value()->u32 { line!() }");
    let head = graph(root.path(), "\nfn value()->u32 { line!() }");
    let report = assess_test_impact(
        &base,
        &head,
        TestImpactRequest {
            targets: vec![target("value", "lib.rs"), target("unrelated", "other.rs")],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!report.full_suite_required);
    assert_eq!(report.selected.len(), 1);
    assert_eq!(report.selected[0].target.id, "value");
}

#[test]
fn removed_callees_keep_base_paths_and_new_edges_add_head_paths() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("test.rs"),
        "fn test_old(){ old() } fn test_new(){ new() }",
    )
    .unwrap();
    let base = graph(root.path(), "fn old() {} fn new() {} fn helper() {}");
    let head = graph(root.path(), "fn new() { helper() } fn helper() {}");
    // The structural binder does not promise cross-file Rust lookup. Author
    // those evidence edges independently, using the real occurrence IDs.
    fn add_edge(graph: &mut entmoot_engine::TopologyGraph, from: &str, to: &str) {
        let id = |name: &str| {
            graph
                .nodes
                .iter()
                .find(|n| n.kind == entmoot_engine::NodeKind::Callable && n.label == name)
                .unwrap()
                .id
                .clone()
        };
        graph.edges.push(entmoot_engine::StructuralEdge::structural(
            id(from),
            id(to),
            entmoot_engine::EdgeKind::Calls,
        ));
    }
    let mut base = base;
    let mut head = head;
    add_edge(&mut base, "test_old", "old");
    add_edge(&mut head, "test_new", "new");
    let report = assess_test_impact(
        &base,
        &head,
        TestImpactRequest {
            targets: vec![
                target("owner", "lib.rs"),
                target("old", "test.rs"),
                target("sibling", "unrelated.rs"),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    let selected = report
        .selected
        .iter()
        .find(|t| t.target.id == "old")
        .unwrap();
    for side in ["base", "head"] {
        assert!(selected.reasons.iter().any(|r| matches!(r, TestSelectionReason::ReverseRelationship { side: s, nodes, .. } if s == side && nodes.len() > 1)));
    }
    assert!(!report.selected.iter().any(|t| t.target.id == "sibling"));
}

#[test]
fn all_structural_families_keep_real_occurrences_and_unknown_uses() {
    for (file, source) in [
        ("sample.rs", "pub fn sample() {}"),
        ("sample.py", "def sample():\r\n    return 'é'\r\n"),
        ("sample.js", "export function sample() { return 1; }"),
        (
            "sample.ts",
            "export function sample(): number { return 1; }",
        ),
        ("sample.tsx", "export function sample() { return <div/>; }"),
        (
            "sample.go",
            "package sample\nfunc Sample() int { return 1 }",
        ),
        (
            "Sample.java",
            "class Sample { public static int sample() { return 1; } }",
        ),
        (
            "Sample.cs",
            "class Sample { public static int sample() { return 1; } }",
        ),
        ("sample.c", "int sample(void) { return 1; }"),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(file), source).unwrap();
        let graph =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let report = assess_unused(&graph, UnusedRequest::default()).unwrap();
        assert!(!report.candidates.is_empty(), "{file}");
        for subject in report.candidates.iter().flat_map(|c| &c.members) {
            let node = graph
                .nodes
                .iter()
                .find(|n| n.id == subject.node_id)
                .unwrap();
            assert_eq!(subject.path.as_deref(), Some(file));
            let span = subject.span.as_ref().unwrap();
            assert!(source.get(span.start_byte..span.end_byte).is_some());
            assert_eq!(Some(span.start_byte), node.start_byte);
        }
        assert!(
            !report.completeness.established(),
            "export/entrypoint absence is unqualified:{file}"
        );
        // Whitespace can be semantically significant to tools/macros. Every
        // family keeps its authored owner even when callable hashes agree.
        std::fs::write(root.path().join(file), format!("\n{source}")).unwrap();
        let head =
            entmoot_engine::build_topology_from_repo_with_options(root.path(), Default::default())
                .unwrap();
        let runner = match file.rsplit('.').next().unwrap() {
            "rs" => TestRunner::Cargo,
            "py" => TestRunner::Pytest,
            "js" | "ts" | "tsx" => TestRunner::Node,
            "go" => TestRunner::Go,
            "java" => TestRunner::Junit,
            "cs" => TestRunner::Dotnet,
            "c" => TestRunner::Ctest,
            _ => unreachable!(),
        };
        let mut owner = target("owned", file);
        owner.runner = runner;
        owner.command = vec!["authored-runner".into(), "owned".into()];
        let impact = assess_test_impact(
            &graph,
            &head,
            TestImpactRequest {
                targets: vec![owner.clone(), target("sibling", "unrelated/")],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(impact.selected.len(), 1, "{file}");
        assert_eq!(impact.selected[0].target, owner);
        assert!(!impact.full_suite_required, "{file}");
    }
}

#[test]
fn sqlite_lazy_eager_assessments_match() {
    use entmoot_engine::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};
    let root = tempfile::tempdir().unwrap();
    let mut graph = graph(root.path(), "fn a(){ b() } fn b() {} fn candidate() {}");
    let used = declaration(&graph, "candidate");
    let facts = vec![
        AssessmentObservation::CodeUse {
            subject: used.clone(),
            usage: CodeUseKind::Escape,
        },
        AssessmentObservation::TestExecution {
            target: target("covered", "tests/"),
            outcome: TestExecutionOutcome::Passed,
            covered: vec![used],
            limitations: vec!["authored instrumented hit".into()],
        },
    ];
    attach_evidence(&mut graph, facts.clone());
    let claims = graph.relation_claims.clone();
    attach_evidence(&mut graph, facts);
    assert_eq!(graph.relation_claims, claims, "replay is idempotent");
    let path = root.path().join("store.sqlite3");
    let mut repo = SqliteKnowledgeRepository::open(&path).unwrap();
    repo.replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    repo.store_inventory(&graph, None).unwrap();
    drop(repo);
    let repo = SqliteKnowledgeRepository::open_read_snapshot(&path).unwrap();
    let lazy = repo.load_inventory(true).unwrap().unwrap();
    let eager = repo.load_inventory(false).unwrap().unwrap();
    let keyed = |rows: &[RelationClaim]| {
        rows.iter()
            .map(|c| (c.id.to_string(), c.clone()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(
        keyed(&eager.relation_claims),
        keyed(&claims),
        "typed facts survive reopen"
    );
    drop(repo);
    let expected = assess_unused(&graph, UnusedRequest::default()).unwrap();
    let actual = QueryEngine::new(lazy, path)
        .unused(UnusedRequest::default())
        .unwrap();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    let head = self::graph(
        root.path(),
        "fn a(){ b() } fn b() {} fn candidate() { a() }",
    );
    let request = TestImpactRequest {
        targets: vec![target("owner", "lib.rs"), target("covered", "tests/")],
        ..Default::default()
    };
    let expected = assess_test_impact(&graph, &head, request.clone()).unwrap();
    let actual = assess_test_impact(&eager, &head, request).unwrap();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
}

#[test]
fn incoming_call_from_outside_inventory_is_not_reported_as_unreferenced() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = graph(root.path(), "fn callback() {}");
    let callback = graph
        .nodes
        .iter()
        .find(|n| n.kind == entmoot_engine::NodeKind::Callable)
        .unwrap();
    graph.relation_edges.push(relation_edge_with_id(
        "source",
        Address::Opaque {
            system: "external-entrypoint".into(),
            value: "event-loop".into(),
        },
        RelationKind::Calls,
        topology_node_address(&callback.id),
        Scope {
            domain: graph.snapshot.repository_id.clone(),
            snapshot_revision: Some(graph.snapshot.revision.clone()),
            snapshot_digest: graph.snapshot.snapshot_digest.clone(),
            occurrence_id: None,
            context_id: None,
        },
        None,
    ));
    let report = assess_unused(&graph, UnusedRequest::default()).unwrap();
    assert!(
        report.candidates.is_empty(),
        "an observed external caller is still a caller"
    );
}

#[test]
fn fixture_only_unmapped_and_exhausted_queries_widen_explicitly() {
    let root = tempfile::tempdir().unwrap();
    let base = graph(root.path(), "fn value() { }");
    let head = graph(root.path(), "fn value() { let x=1; }");
    let mut suite = target("fixture", "tests/");
    suite.fixtures = vec!["lib.rs".into()];
    let request = TestImpactRequest {
        targets: vec![suite, target("sibling", "other.rs")],
        ..Default::default()
    };
    let report = assess_test_impact(&base, &head, request.clone()).unwrap();
    assert!(!report.full_suite_required);
    assert_eq!(report.selected.len(), 1);
    let report = assess_test_impact(
        &base,
        &head,
        TestImpactRequest {
            max_nodes: 1,
            ..request
        },
    )
    .unwrap();
    assert!(report.full_suite_required);
    assert_eq!(report.selected.len(), 2);
    let report = assess_test_impact(
        &base,
        &head,
        TestImpactRequest {
            targets: vec![target("sibling", "other.rs")],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.full_suite_required);
    assert!(!report.selected[0].reasons.is_empty());
}

#[test]
fn unused_candidates_are_never_absence_proofs_and_keep_recursive_islands() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph(
        root.path(),
        "fn a(){b()} fn b(){a()} fn lonely(){} fn root(){leaf()} fn leaf(){}",
    );
    let report = assess_unused(&graph, UnusedRequest::default()).unwrap();
    assert!(!report.completeness.established());
    assert_eq!(report.summary, "completeness not established");
    assert!(report
        .candidates
        .iter()
        .any(|c| c.signal == UnusedSignal::MutuallyReferencingIsland && c.members.len() == 2));
    assert!(!report
        .candidates
        .iter()
        .any(|c| c.members.iter().any(|m| m.label == "leaf")));
    let bounded = assess_unused(
        &graph,
        UnusedRequest {
            max_edges: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(bounded.limitations.iter().any(|s| s.contains("budget")));
    assert!(
        bounded.bounds.truncated,
        "work exhaustion is visible in bounds"
    );
    assert!(
        bounded.candidates.is_empty(),
        "truncated edges cannot establish missing incoming calls"
    );
}
