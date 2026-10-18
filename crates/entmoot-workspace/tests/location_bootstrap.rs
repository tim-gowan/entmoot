use entmoot_contracts::ENTMOOT_CONFIG_FILE;
use entmoot_workspace::workspace::*;
use std::fs;
#[test]
fn bootstrap_writes_home_namespace_not_source_tree() {
    const CHILD: &str = "ENTMOOT_LOCATION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let data = tempfile::tempdir().unwrap();
        let inherited = std::env::var_os("ENTMOOT_DATA_DIR");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "bootstrap_writes_home_namespace_not_source_tree"])
            .env(CHILD, "1")
            .env("ENTMOOT_DATA_DIR", data.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::env::var_os("ENTMOOT_DATA_DIR"), inherited);
        return;
    }
    let data = data_root();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("app.py"), "def leaf(): pass\n").unwrap();
    let first = prepare_workspace(source.path()).unwrap();
    let second = prepare_workspace(source.path()).unwrap();
    assert_eq!(first.current.graph, second.current.graph);
    assert!(!source.path().join(".entmoot").exists());
    let storage = storage_directory_in(source.path(), Some(&data)).unwrap();
    assert!(storage.join("current.json").is_file());
    assert!(workspace_namespace(source.path(), &data)
        .join(ENTMOOT_CONFIG_FILE)
        .is_file());
    let summary: WorkspaceSummary =
        serde_json::from_slice(&fs::read(&first.summary).unwrap()).unwrap();
    assert!(summary.structural_only);
    // Damaged pointer under the correct namespace must not be silently replaced.
    let pointer = storage.join("current.json");
    fs::write(&pointer, "invalid").unwrap();
    assert!(prepare_workspace(source.path()).is_err());
    assert_eq!(fs::read(&pointer).unwrap(), b"invalid");
}
