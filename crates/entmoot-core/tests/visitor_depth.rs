//! Compiler-free budget controls against real grammar trees and production visitors.
use std::collections::BTreeSet;

use entmoot_core::registry::LanguageRegistry;
use entmoot_core::{scan_repo_with_analysis, SkipReason, StructuralHasher, MAX_RECURSIVE_DEPTH};

struct Fixture {
    language: &'static str,
    extension: &'static str,
    before: &'static str,
    callable_start: usize,
    after: &'static str,
    trailer: &'static str,
    scope: &'static [&'static str],
}

impl Fixture {
    fn source(&self, depth: usize) -> String {
        format!(
            "{}{}1{}{}{}",
            self.before,
            "(".repeat(depth),
            ")".repeat(depth),
            self.after,
            self.trailer
        )
    }
}

fn fixtures() -> [Fixture; 9] {
    [
        Fixture {
            language: "python",
            extension: "py",
            before: "def focus():\n    return ",
            callable_start: 0,
            after: "",
            trailer: "\n",
            scope: &[],
        },
        Fixture {
            language: "javascript",
            extension: "js",
            before: "function focus() { return ",
            callable_start: 0,
            after: "; }",
            trailer: "\n",
            scope: &[],
        },
        Fixture {
            language: "typescript",
            extension: "ts",
            before: "function focus(): number { return ",
            callable_start: 0,
            after: "; }",
            trailer: "\n",
            scope: &[],
        },
        Fixture {
            language: "tsx",
            extension: "tsx",
            before: "function focus(): number { return ",
            callable_start: 0,
            after: "; }",
            trailer: "\n",
            scope: &[],
        },
        Fixture {
            language: "rust",
            extension: "rs",
            before: "fn focus() -> i32 { ",
            callable_start: 0,
            after: " }",
            trailer: "\n",
            scope: &[],
        },
        Fixture {
            language: "go",
            extension: "go",
            before: "package p\nfunc focus() int { return ",
            callable_start: 10,
            after: " }",
            trailer: "\n",
            scope: &[],
        },
        Fixture {
            language: "java",
            extension: "java",
            before: "class T { int focus() { return ",
            callable_start: 10,
            after: "; }",
            trailer: " }\n",
            scope: &["T"],
        },
        Fixture {
            language: "csharp",
            extension: "cs",
            before: "class T { int focus() { return ",
            callable_start: 10,
            after: "; }",
            trailer: " }\n",
            scope: &["T"],
        },
        Fixture {
            language: "c",
            extension: "c",
            before: "int focus(void) { return ",
            callable_start: 0,
            after: "; }",
            trailer: "\n",
            scope: &[],
        },
    ]
}

#[test]
fn production_visitors_reject_depth_exhaustion_in_valid_grammar_trees() -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = LanguageRegistry::new();
    entmoot_core::languages::register_all(&mut registry);
    for fixture in fixtures() {
        let plugin = registry
            .get_language(fixture.language)
            .ok_or("missing claimed grammar")?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&plugin.language)?;
        for depth in [32, MAX_RECURSIVE_DEPTH + 64] {
            let source = fixture.source(depth);
            let tree = parser.parse(&source, None).ok_or("missing parse tree")?;
            assert!(
                !tree.root_node().has_error(),
                "{}: fixture must reach the visitor guard",
                fixture.language
            );
            let result = plugin.normalizer.visit_tree(&tree, source.as_bytes(), None);
            if depth == 32 {
                let entmoot_core::normalizer::VisitorOutput { callables, .. } = result?;
                // The authored name, scope and byte range identify this occurrence.
                let selected: Vec<_> = callables
                    .iter()
                    .filter(|callable| {
                        callable.name == "focus"
                            && callable.scope == fixture.scope
                            && callable.start_byte == fixture.callable_start
                            && callable.end_byte == source.len() - fixture.trailer.len()
                    })
                    .collect();
                assert_eq!(
                    selected.len(),
                    1,
                    "{}: exact occurrence missing or duplicated",
                    fixture.language
                );
                assert_eq!(callables.len(), 1, "{}: unexpected inventory", fixture.language);
            } else {
                let error = result.err().ok_or("exhausted visitor budget was accepted")?;
                assert!(error.to_string().contains("depth"), "{}: {error}", fixture.language);
            }
        }
    }
    Ok(())
}

#[test]
fn repository_scan_preserves_failed_sources_and_unrelated_occurrences() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let mut good_paths = BTreeSet::new();
    let mut failed_paths = BTreeSet::new();
    for fixture in fixtures() {
        for (label, depth) in [("good", 32), ("deep", MAX_RECURSIVE_DEPTH + 64)] {
            let path = dir.path().join(format!("{label}.{}", fixture.extension));
            std::fs::write(&path, fixture.source(depth))?;
            if label == "good" {
                good_paths.insert(path);
            } else {
                failed_paths.insert(path);
            }
        }
    }
    let scan = scan_repo_with_analysis(&StructuralHasher::new(), dir.path(), None, Some("visitor-depth"))?;
    assert_eq!(scan.reused_files, 0);
    assert_eq!(
        scan.files.iter().map(|file| file.path.clone()).collect::<BTreeSet<_>>(),
        good_paths
    );
    assert_eq!(
        scan.skipped
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>(),
        failed_paths
    );
    assert_eq!(scan.snapshot_files.len(), 18);
    assert_eq!(
        scan.snapshot_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>(),
        good_paths.union(&failed_paths).cloned().collect()
    );
    for file in &scan.snapshot_files {
        assert_eq!(
            file.content_digest,
            entmoot_contracts::blake3_hex(&std::fs::read(&file.path)?)
        );
    }
    for file in &scan.skipped {
        assert!(
            matches!(&file.reason, SkipReason::HashError(reason) if reason.contains("depth")),
            "{file:?}"
        );
    }
    let mut addresses = BTreeSet::new();
    for file in &scan.files {
        assert_eq!(file.record.occurrences.len(), 1);
        for occurrence in &file.record.occurrences {
            let reference = entmoot_contracts::OccurrenceRef::new(occurrence.logic_address.clone())?;
            assert_eq!(reference.address_id.as_str(), occurrence.address_id);
            assert!(
                addresses.insert(&occurrence.address_id),
                "occurrences must remain file-scoped"
            );
            assert!(!occurrence.structural_hash.is_empty());
        }
    }
    assert_eq!(addresses.len(), 9);
    Ok(())
}
