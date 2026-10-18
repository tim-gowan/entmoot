use entmoot_contracts::query::*;
use entmoot_store::*;
#[test]
fn overload_deletion_and_anonymous_occurrences_do_not_collapse() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("App.java");
    std::fs::write(
        &path,
        "class App { void run(int x) {} void run(String x) {} }",
    )
    .unwrap();
    let base =
        entmoot_store::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    std::fs::write(&path, "class App { void run(int x) {} }").unwrap();
    let head =
        entmoot_store::build_topology_from_repo_with_options(root.path(), Default::default())
            .unwrap();
    let report = overlay::compare_inventory_overlay(
        &base,
        &head,
        &InventoryOverlayRequest {
            base_publication: "base".into(),
            head_publication: "head".into(),
            name_filters: vec!["run".into()],
            path_prefixes: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        (report.introduced, report.removed, report.retained),
        (0, 1, 1)
    );
}
