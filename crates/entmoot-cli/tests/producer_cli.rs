#![cfg(feature = "producer")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

#[test]
fn sidecar_commands_fail_loudly_and_preserve_unavailable_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    for (path, text) in [
        (
            "main.go",
            "package main\nfunc main() { help() }\nfunc help() {}\n",
        ),
        (
            "Main.cs",
            "class Main { void Run() { Help(); } void Help() {} }",
        ),
        (
            "Main.java",
            "class Main { void run() { help(); } void help() {} }",
        ),
        (
            "main.c",
            "void help(void) {}\nint main(void) { help(); return 0; }",
        ),
        (
            "main.ts",
            "export function leaf() {} export function run() { leaf(); }",
        ),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let graph =
        entmoot_engine::build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    entmoot_engine::write_graph_json(&graph, &graph_path).unwrap();
    let before = std::fs::read(&graph_path).unwrap();
    for (name, env) in [
        ("go", "ENTMOOT_GO_ADAPTER"),
        ("roslyn", "ENTMOOT_ROSLYN_ADAPTER"),
        ("java", "ENTMOOT_JAVA_ADAPTER"),
        ("clang", "ENTMOOT_CLANG_ADAPTER"),
        ("typescript", "ENTMOOT_TYPESCRIPT_ADAPTER"),
    ] {
        let out = dir.path().join(name);
        let output = Command::new(env!("CARGO_BIN_EXE_entmoot-producer"))
            .args([name, "produce", "--graph"])
            .arg(&graph_path)
            .arg("--root")
            .arg(&root)
            .arg("--out")
            .arg(&out)
            .env(env, dir.path().join("missing-solver"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{name} silently used a fallback");
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid_environment"));
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("native.raw.json")).unwrap()).unwrap();
        assert_eq!(raw["invalid_environment"], true);
        let artifact: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("resolver.json")).unwrap()).unwrap();
        assert!(
            !artifact["observations"].as_array().unwrap().is_empty(),
            "{name} silently lost inventory attempts"
        );
        assert!(artifact["observations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["outcome"] == "failed"));
        assert!(out.join("resolver.json.manifest.json").is_file());
    }
    assert_eq!(std::fs::read(graph_path).unwrap(), before);
}
