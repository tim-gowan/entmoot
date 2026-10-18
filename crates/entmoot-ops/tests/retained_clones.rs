use entmoot_contracts::{CanonAvailability, CloneLevel, CloneRequest};
use entmoot_ops::{KnowledgeRepository, QueryEngine};
use std::sync::Arc;
#[test]
fn groups_retained_occurrences_without_reparsing_the_checkout() {
    let root = tempfile::tempdir().unwrap();
    let source = "fn first(x:i32)->i32{x+x} fn second(y:i32)->i32{y+y}";
    std::fs::write(root.path().join("a.rs"), source).unwrap();
    let mut graph = entmoot_ops::build_topology_from_repo_with_options(
        root.path(),
        entmoot_ops::TopologyBuildOptions::default(),
    )
    .unwrap();
    entmoot_ops::canon::produce(&mut graph, root.path(), &[], &[], None).unwrap();
    let path = root.path().join("knowledge.sqlite3");
    entmoot_ops::SqliteKnowledgeRepository::open(&path)
        .unwrap()
        .replace_all(&entmoot_ops::KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let engine = QueryEngine::new(graph, path.clone()).with_source_store(Arc::new(
        entmoot_ops::GixSourceStore::open(root.path()).unwrap(),
    ));
    assert!(engine
        .complete_graph()
        .unwrap()
        .relation_claims
        .iter()
        .any(|claim| claim.predicate == "canon.file"));
    assert!(
        !entmoot_ops::SqliteKnowledgeRepository::open_read_snapshot(&path)
            .unwrap()
            .load_assessment()
            .unwrap()
            .relation_claims
            .iter()
            .any(|claim| claim.predicate.starts_with("canon."))
    );
    let request = CloneRequest {
        min_tokens: 5,
        ..Default::default()
    };
    let report = engine.clones(request.clone()).unwrap();
    assert_eq!(report.scanned_files, 1);
    let mut stale_graph = engine.graph().clone();
    stale_graph.snapshot.revision = "different-revision".into();
    let stale = QueryEngine::new(stale_graph, path)
        .clones(request.clone())
        .unwrap();
    assert_eq!(stale.canon, CanonAvailability::Stale);
    assert!(stale.nudge.is_some());
    assert!(stale.groups.is_empty());
    let group = report
        .groups
        .iter()
        .find(|g| g.level == CloneLevel::Alpha)
        .unwrap();
    assert_eq!(group.members.len(), 2);
    assert_ne!(group.members[0].node_id, group.members[1].node_id);
    assert!(group
        .members
        .iter()
        .all(|m| engine.graph().nodes.iter().any(|n| n.id == m.node_id)));
    let limited = engine
        .clones(CloneRequest {
            max_patterns: 1,
            ..request.clone()
        })
        .unwrap();
    assert!(limited.groups.is_empty());
    assert!(limited.bounds.truncated);
    assert!(limited.limitations.iter().any(|l| l.contains("exceeded")));
    std::fs::write(root.path().join("a.rs"), source.replace("x+x", "x-x")).unwrap();
    // Retained snapshot reads do not reparse the edited checkout.
    assert_eq!(
        engine.clones(request).unwrap().canon,
        CanonAvailability::Available
    );
}
