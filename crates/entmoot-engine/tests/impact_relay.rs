//! Impact relay preserves the shared walk's uncertainty and both paging axes.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::query::*;
use entmoot_engine::{
    build_topology_from_repo_with_options, KnowledgeRepository, KnowledgeStore, QueryEngine,
    SqliteKnowledgeRepository, TopologyBuildOptions, TopologyGraph,
};

fn pair(file: &str, before: &str, after: &str) -> (tempfile::TempDir, QueryEngine, TopologyGraph) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join(file), before).unwrap();
    let options = TopologyBuildOptions {
        repository: Some("fixture/relay"),
        ..Default::default()
    };
    let base = build_topology_from_repo_with_options(&source, options).unwrap();
    std::fs::write(source.join(file), after).unwrap();
    let head = build_topology_from_repo_with_options(
        &source,
        TopologyBuildOptions {
            repository: Some("fixture/relay"),
            ..Default::default()
        },
    )
    .unwrap();
    let store = directory.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&head))
        .unwrap();
    let engine = QueryEngine::new(head, store).with_compare_graph(base.clone());
    (directory, engine, base)
}

#[test]
fn relay_pages_changes_and_callers_without_erasing_gaps() {
    let source =
        "fn leaf() -> i32 { 1 }\nfn first() { leaf(); }\nfn second() { leaf(); }\nfn gone() {}\n";
    let (_dir, engine, _) = pair(
        "lib.rs",
        source,
        &source
            .replace("{ 1 }", "{ 2 }")
            .replace("fn gone() {}\n", ""),
    );
    let mut request = ImpactRelayRequest {
        evidence_filter: HopEvidenceFilter::Any,
        ..Default::default()
    };
    request.output.limit = 1;
    request.callers_output.limit = 1;
    let first = engine.relay_impact(request.clone()).unwrap();
    assert_eq!(first.page.total_items, 2);
    assert_eq!(first.items.len(), 1);
    request.output.after = first.page.next_cursor.clone();
    let second = engine.relay_impact(request.clone()).unwrap();
    assert!(second.page.next_cursor.is_none());
    let items: Vec<_> = first.items.iter().chain(&second.items).collect();
    let body = items
        .iter()
        .find(|item| item.callable.kind == CallableImpactKind::BodyChanged)
        .unwrap();
    let callers = body.callers.as_ref().unwrap();
    assert!(callers.page.total_items >= 2, "{callers:?}");
    assert!(callers.details_deferred);
    assert!(first
        .gaps
        .iter()
        .any(|gap| gap == "repository_call_graph_completeness_not_established"));
    assert!(first
        .gaps
        .iter()
        .any(|gap| gap == "dependency_call_graph_completeness_not_established"));
    assert!(first
        .gaps
        .iter()
        .any(|gap| gap == "native_library_call_graph_completeness_not_established"));
    let mut followup = (*callers.query).clone();
    followup.set_output(OutputOptions {
        limit: 1,
        after: callers.page.next_cursor.clone(),
        ..Default::default()
    });
    let QueryResponse::Compact(next_callers) = engine.execute(followup).unwrap() else {
        panic!("compact")
    };
    assert_eq!(next_callers.page.offset, 1);
    assert_ne!(callers.items, next_callers.items);
    let removed = items
        .iter()
        .find(|item| item.callable.kind == CallableImpactKind::Removed)
        .unwrap();
    assert!(removed.callers.is_none());
    assert_eq!(removed.gaps, ["removed_callable_no_head_neighborhood"]);

    let mut bounded = ImpactRelayRequest {
        evidence_filter: HopEvidenceFilter::Any,
        ..Default::default()
    };
    bounded.budget.max_results = 1;
    let truncated = engine.relay_impact(bounded).unwrap();
    assert!(truncated
        .items
        .iter()
        .filter_map(|item| item.callers.as_ref())
        .any(|page| page.search_truncated));

    request.budget.max_hops += 1;
    assert!(matches!(
        engine.relay_impact(request),
        Err(QueryServiceError::InvalidRequest(_))
    ));
    let resolved = engine.relay_impact(ImpactRelayRequest::default()).unwrap();
    assert!(resolved
        .items
        .iter()
        .filter_map(|item| item.callers.as_ref())
        .all(|page| page.items.is_empty()));
}

#[test]
fn relay_fences_pair_and_rejects_byte_and_search_overflows() {
    let (_dir, engine, mut base) = pair(
        "lib.rs",
        "fn leaf() -> i32 { 1 }\nfn gone() {}",
        "fn leaf() -> i32 { 2 }",
    );
    let request = ImpactRelayRequest {
        output: OutputOptions {
            limit: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let first = engine.relay_impact(request.clone()).unwrap();
    let mut resumed = request.clone();
    resumed.output.after = first.page.next_cursor;
    base.root.push_str("-another-base");
    let changed_pair = engine.with_compare_graph(base);
    assert!(matches!(
        changed_pair.relay_impact(resumed),
        Err(QueryServiceError::InvalidRequest(_))
    ));
    for invalid in [
        ImpactRelayRequest {
            output: OutputOptions {
                max_bytes: Some(128),
                ..Default::default()
            },
            ..request.clone()
        },
        ImpactRelayRequest {
            output: OutputOptions {
                tier: InformationTier::Explain,
                ..Default::default()
            },
            ..request.clone()
        },
        ImpactRelayRequest {
            output: OutputOptions {
                limit: 17,
                ..Default::default()
            },
            ..request.clone()
        },
        ImpactRelayRequest {
            budget: BoundedWalkBudget {
                max_hops: 0,
                ..Default::default()
            },
            ..request
        },
    ] {
        assert!(matches!(
            changed_pair.relay_impact(invalid),
            Err(QueryServiceError::InvalidRequest(_))
        ));
    }
}

#[test]
fn relay_is_language_neutral_and_preserves_empty_unknown_neighborhoods() {
    for (file, source) in [
        ("lib.rs", "fn leaf() -> i32 { 1 }"),
        ("api.py", "def leaf():\n    return 1\n"),
        ("api.js", "function leaf() { return 1; }"),
        ("api.ts", "function leaf() { return 1; }"),
        ("api.tsx", "function leaf() { return 1; }"),
        ("api.go", "package p\nfunc leaf() int { return 1 }"),
        ("Api.java", "class Api { int leaf() { return 1; } }"),
        ("Api.cs", "class Api { int Leaf() { return 1; } }"),
        ("api.c", "int leaf(void) { return 1; }"),
        ("api.cpp", "int leaf() { return 1; }"),
    ] {
        let (_dir, engine, _) = pair(file, source, &source.replace('1', "2"));
        let result = engine.relay_impact(ImpactRelayRequest::default()).unwrap();
        assert_eq!(result.items.len(), 1, "{file}");
        assert_eq!(
            result.items[0].callable.kind,
            CallableImpactKind::BodyChanged,
            "{file}"
        );
        assert!(
            result.items[0].callers.as_ref().unwrap().items.is_empty(),
            "{file}"
        );
        assert!(!result.gaps.is_empty());
    }
}

#[test]
fn relay_byte_pages_resume_every_changed_callable() {
    let before = "fn first() -> i32 { 1 }\nfn second() -> i32 { 1 }\nfn third() -> i32 { 1 }";
    let (_dir, engine, _) = pair("lib.rs", before, &before.replace('1', "2"));
    let mut request = ImpactRelayRequest::default();
    let mut complete = engine.relay_impact(request.clone()).unwrap();
    assert_eq!(complete.items.len(), 3);
    request.output.limit = 1;
    let single = engine.relay_impact(request.clone()).unwrap();
    // A little headroom accommodates differing names while remaining below
    // the complete envelope. Delivery pages must never drop a changed subject.
    request.output.max_bytes = Some(serde_json::to_vec(&single).unwrap().len() + 128);
    request.output.limit = 3;
    let mut delivered = Vec::new();
    loop {
        let page = engine.relay_impact(request.clone()).unwrap();
        assert!(!page.items.is_empty());
        assert!(page.items.len() < 3);
        assert!(serde_json::to_vec(&page).unwrap().len() <= request.output.max_bytes.unwrap());
        delivered.extend(page.items);
        let Some(next) = page.page.next_cursor else {
            break;
        };
        request.output.after = Some(next);
    }
    for item in delivered.iter_mut().chain(&mut complete.items) {
        if let Some(receipt) = item
            .callers
            .as_mut()
            .and_then(|c| c.search_receipt.as_mut())
        {
            receipt.elapsed_micros = 0;
            receipt.index_micros = 0;
            receipt.evidence_micros = 0;
        }
    }
    assert_eq!(delivered, complete.items);
}
