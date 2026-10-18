use entmoot_core::ResolveConfidence;
use entmoot_store::*;
use std::collections::HashSet;
#[test]
fn repository_build_connects_literal_sibling_import_calls() {
    let directory = tempfile::tempdir().unwrap();
    for path in ["src", "web", "py"] {
        std::fs::create_dir_all(directory.path().join(path)).unwrap();
    }
    std::fs::write(
        directory.path().join("src/lib.rs"),
        "mod sibling; use crate::sibling::rust_help; fn rust_caller() { rust_help(); }",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/sibling.rs"),
        "pub fn rust_help() {}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("web/caller.ts"),
        "import { tsHelp } from './sibling'; function tsCaller() { tsHelp(); }",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("web/sibling.ts"),
        "export function tsHelp() {}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("web/type_only.ts"),
        "import { type Ghost } from './ghost'; function typeOnlyCaller() { Ghost(); }",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("web/ghost.ts"),
        "export function Ghost() {}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("py/caller.py"),
        "from .sibling import py_help\ndef py_caller():\n    py_help()\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("py/sibling.py"),
        "def py_help():\n    pass\n",
    )
    .unwrap();

    let graph = entmoot_store::build_topology_from_repo_with_options(
        directory.path(),
        entmoot_store::TopologyBuildOptions::default(),
    )
    .unwrap();
    let pairs = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.resolved
                && edge.resolve_confidence == Some(ResolveConfidence::ImportLocal)
        })
        .filter_map(|edge| {
            Some((
                graph.node(&edge.from)?.label.as_str(),
                graph.node(&edge.to)?.label.as_str(),
            ))
        })
        .collect::<HashSet<_>>();
    assert!(pairs.contains(&("rust_caller", "rust_help")));
    assert!(pairs.contains(&("tsCaller", "tsHelp")));
    assert!(pairs.contains(&("py_caller", "py_help")));
    assert!(!pairs.contains(&("typeOnlyCaller", "Ghost")));
}
