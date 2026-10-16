//! Analysis foundation integration tests (call/import marker parity).

#![cfg(feature = "full-tests")]

use entmoot_contracts::{ControlSiteKind, ImportKind};
use entmoot_core::{resolve::resolve_file_calls, CoreError, StructuralHasher};

#[test]
fn bundled_iife_markers_are_bounded_without_changing_structural_hashes() {
    let source = format!("(() => {{ const data = \"{}\"; return data; }})();", "λ".repeat(3000));
    let hasher = StructuralHasher::new();
    for language in ["javascript", "typescript"] {
        let analyzed = hasher
            .compute_granular_hash_with_analysis(&source, language, None)
            .unwrap();
        let plain = hasher.compute_granular_hash(&source, language, None).unwrap();
        assert_eq!(analyzed.file_hash, plain.file_hash);
        let calls = analyzed.markers.unwrap().call_sites;
        let marker = calls.iter().find(|call| call.start_byte == 0).unwrap();
        assert!(marker.callee.starts_with("<oversized-callee:blake3:"));
        assert!(marker.callee.len() < 128);
        assert_eq!(marker.end_byte, source.len() - 1);
    }
}

#[test]
fn granular_hash_with_analysis_emits_markers() {
    let source = r#"
import os

def greet(name):
    return name
"#;
    let hasher = StructuralHasher::new();
    let with_markers = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .expect("analysis hash");
    let without_markers = hasher
        .compute_granular_hash(source, "python", None)
        .expect("plain hash");

    assert_eq!(with_markers.file_hash, without_markers.file_hash);
    assert!(with_markers.markers.is_some());
    let markers = with_markers.markers.unwrap();
    assert!(!markers.imports.is_empty(), "expected import markers");
    assert!(
        markers.definitions.iter().any(|d| d.name == "greet"),
        "expected greet definition marker"
    );
}

#[test]
fn granular_hash_with_analysis_emits_call_sites() {
    let source = r#"
def greet(name):
    return print(name)
"#;
    let hasher = StructuralHasher::new();
    let with_markers = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .expect("analysis hash");
    let markers = with_markers.markers.expect("markers");
    assert!(
        markers.call_sites.iter().any(|c| c.callee == "print"),
        "expected print call-site marker, got {:?}",
        markers.call_sites
    );
}

#[test]
fn call_owner_distinguishes_module_and_innermost_callable_scope() {
    let source = "outside()\ndef outer():\n    def inner():\n        inside()\n    inner()\n";
    let result = StructuralHasher::new()
        .compute_granular_hash_with_analysis(source, "python", None)
        .expect("analysis hash");
    let markers = result.markers.expect("markers");

    let outside = markers
        .call_sites
        .iter()
        .find(|call| call.callee == "outside")
        .expect("module call");
    assert!(outside.owner.is_none());

    let inside = markers
        .call_sites
        .iter()
        .find(|call| call.callee == "inside")
        .expect("nested call");
    let owner = inside.owner.as_ref().expect("nested owner");
    assert_eq!(owner.name, "inner");
    let inner_definition = markers
        .definitions
        .iter()
        .find(|definition| definition.name == "inner")
        .expect("inner definition");
    assert_eq!(owner.scope, inner_definition.scope);
    assert_eq!(owner.start_byte, inner_definition.start_byte);
}

#[test]
fn default_granular_hash_omits_markers() {
    let source = "def f(): pass\n";
    let hasher = StructuralHasher::new();
    let result = hasher.compute_granular_hash(source, "python", None).expect("hash");
    assert!(result.markers.is_none());
}

#[test]
fn binding_markers_do_not_claim_a_fake_zero_byte_location() {
    let source = "\ndef greet(name):\n    value = name\n    return value\n";
    let hasher = StructuralHasher::new();
    let result = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .expect("analysis hash");
    let markers = result.markers.expect("markers");

    assert!(!markers.bindings.is_empty(), "expected binding markers");
    assert!(
        markers
            .bindings
            .iter()
            .all(|binding| binding.start_byte.is_some_and(|offset| offset > 0)),
        "Python binding locations must be real declaration offsets: {:?}",
        markers.bindings
    );
}

struct CallParityCase {
    lang: &'static str,
    source: &'static str,
    simple_callee: &'static str,
    qualified_substr: &'static str,
}

fn call_parity_cases() -> Vec<CallParityCase> {
    vec![
        CallParityCase {
            lang: "python",
            source: "def f(obj):\n    foo()\n    obj.bar()\n",
            simple_callee: "foo",
            qualified_substr: "bar",
        },
        CallParityCase {
            lang: "javascript",
            source: "function f(obj) { foo(); obj.bar(); }\n",
            simple_callee: "foo",
            qualified_substr: "bar",
        },
        CallParityCase {
            lang: "typescript",
            source: "function f(obj: any) { foo(); obj.bar(); }\n",
            simple_callee: "foo",
            qualified_substr: "bar",
        },
        CallParityCase {
            lang: "tsx",
            source: "function f(obj: any) { foo(); obj.bar(); }\n",
            simple_callee: "foo",
            qualified_substr: "bar",
        },
        CallParityCase {
            lang: "java",
            source: "class C { void f(Obj obj) { foo(); obj.bar(); } }\n",
            simple_callee: "foo",
            qualified_substr: "bar",
        },
        CallParityCase {
            lang: "go",
            source: "package p\nfunc f(obj T) { Foo(); obj.Bar() }\n",
            simple_callee: "Foo",
            qualified_substr: "Bar",
        },
        CallParityCase {
            lang: "csharp",
            source: "class C { void F(Obj obj) { Foo(); obj.Bar(); } }\n",
            simple_callee: "Foo",
            qualified_substr: "Bar",
        },
        CallParityCase {
            lang: "rust",
            source: "fn f(obj: T) { foo(); obj.bar(); }\n",
            simple_callee: "foo",
            qualified_substr: "bar",
        },
    ]
}

#[test]
fn call_site_parity_simple_and_qualified_all_languages() {
    let hasher = StructuralHasher::new();
    for case in call_parity_cases() {
        let with_m = hasher
            .compute_granular_hash_with_analysis(case.source, case.lang, None)
            .unwrap_or_else(|e| panic!("{} analysis: {e}", case.lang));
        let without = hasher
            .compute_granular_hash(case.source, case.lang, None)
            .unwrap_or_else(|e| panic!("{} plain: {e}", case.lang));
        assert_eq!(with_m.file_hash, without.file_hash, "{} file_hash isolation", case.lang);
        for (name, a) in &with_m.callables {
            let b = without
                .callables
                .get(name)
                .unwrap_or_else(|| panic!("{} missing callable {}", case.lang, name));
            assert_eq!(
                a.structural_hash, b.structural_hash,
                "{} structural_hash isolation for {}",
                case.lang, a.name
            );
        }
        let markers = with_m
            .markers
            .unwrap_or_else(|| panic!("{} expected markers", case.lang));
        for call in &markers.call_sites {
            let owner = call
                .owner
                .as_ref()
                .unwrap_or_else(|| panic!("{} callable-scoped call lacks an owner: {call:?}", case.lang));
            assert!(
                markers.definitions.iter().any(|definition| {
                    definition.name == owner.name
                        && definition.scope == owner.scope
                        && definition.start_byte == owner.start_byte
                }),
                "{} call owner must identify an emitted definition: {call:?}",
                case.lang
            );
        }
        assert!(
            markers.call_sites.iter().all(|call| {
                call.owner
                    .as_ref()
                    .is_some_and(|owner| owner.name.eq_ignore_ascii_case("f"))
            }),
            "{} call markers must retain their visitor-time owner: {:?}",
            case.lang,
            markers.call_sites
        );
        assert!(
            markers
                .call_sites
                .iter()
                .any(|c| c.callee == case.simple_callee && c.start_byte < c.end_byte),
            "{} missing simple callee {:?}, got {:?}",
            case.lang,
            case.simple_callee,
            markers.call_sites
        );
        assert!(
            markers
                .call_sites
                .iter()
                .any(|c| { c.callee.contains(case.qualified_substr) && c.start_byte < c.end_byte }),
            "{} missing qualified containing {:?}, got {:?}",
            case.lang,
            case.qualified_substr,
            markers.call_sites
        );
    }
}

struct ControlParityCase {
    lang: &'static str,
    source: &'static str,
    expected: &'static [ControlSiteKind],
}

fn control_parity_cases() -> Vec<ControlParityCase> {
    use ControlSiteKind::{Break, Conditional, Continue, Loop, Match, Return, Switch, Throw};

    vec![
        ControlParityCase {
            lang: "python",
            source: "def f(xs):\n    for x in xs:\n        if x:\n            continue\n        break\n    match xs:\n        case []:\n            raise ValueError()\n    return 0\n",
            expected: &[Conditional, Loop, Match, Return, Break, Continue, Throw],
        },
        ControlParityCase {
            lang: "javascript",
            source: "function f(xs) { for (const x of xs) { if (x) continue; break; } switch (xs.length) { case 0: throw new Error(); } return 0; }\n",
            expected: &[Conditional, Loop, Switch, Return, Break, Continue, Throw],
        },
        ControlParityCase {
            lang: "typescript",
            source: "function f(xs: unknown[]) { for (const x of xs) { if (x) continue; break; } switch (xs.length) { case 0: throw new Error(); } return 0; }\n",
            expected: &[Conditional, Loop, Switch, Return, Break, Continue, Throw],
        },
        ControlParityCase {
            lang: "tsx",
            source: "function f(xs: unknown[]) { for (const x of xs) { if (x) continue; break; } switch (xs.length) { case 0: throw new Error(); } return 0; }\n",
            expected: &[Conditional, Loop, Switch, Return, Break, Continue, Throw],
        },
        ControlParityCase {
            lang: "java",
            source: "class C { int f(Object[] xs) { for (Object x : xs) { if (x != null) continue; break; } switch (xs.length) { case 0: throw new RuntimeException(); } return 0; } }\n",
            expected: &[Conditional, Loop, Switch, Return, Break, Continue, Throw],
        },
        ControlParityCase {
            lang: "go",
            source: "package p\nfunc f(xs []bool) int { for _, x := range xs { if x { continue }; break }; return 0 }\n",
            expected: &[Conditional, Loop, Return, Break, Continue],
        },
        ControlParityCase {
            lang: "csharp",
            source: "class C { int F(object[] xs) { foreach (object x in xs) { if (x != null) continue; break; } switch (xs.Length) { case 0: throw new System.Exception(); } return 0; } }\n",
            expected: &[Conditional, Loop, Switch, Return, Break, Continue, Throw],
        },
        ControlParityCase {
            lang: "rust",
            source: "fn f(xs: &[bool]) -> i32 { for x in xs { if *x { continue; } break; } match xs.len() { 0 => return 0, _ => 1 } }\n",
            expected: &[Conditional, Loop, Match, Return, Break, Continue],
        },
    ]
}

#[test]
fn control_site_parity_uses_each_ecosystems_native_tree_sitter_nodes() {
    let hasher = StructuralHasher::new();

    for case in control_parity_cases() {
        let result = hasher
            .compute_granular_hash_with_analysis(case.source, case.lang, None)
            .unwrap_or_else(|error| panic!("{} analysis: {error}", case.lang));
        let markers = result
            .markers
            .unwrap_or_else(|| panic!("{} expected markers", case.lang));

        for expected in case.expected {
            assert!(
                markers.control_sites.iter().any(|site| {
                    site.kind == *expected && site.from_byte < site.to_byte && site.to_byte <= case.source.len()
                }),
                "{} missing {expected:?}; got {:?}",
                case.lang,
                markers.control_sites
            );
        }
    }
}

#[test]
fn malformed_source_is_rejected_for_every_registered_ecosystem() {
    let cases = [
        ("python", "def f(x):\n    return x +"),
        ("javascript", "function f() { const value = ; return value; }"),
        ("typescript", "function f(): number { const value = ; return value; }"),
        ("tsx", "function f(): JSX.Element { return <div>"),
        ("java", "class C { int f() { return"),
        ("go", "package p\nfunc f() int { value := ; return value }"),
        ("csharp", "class C { int F() { return"),
        ("rust", "fn f() { let value = ; }"),
        ("c", "int f(void) { return"),
        ("cpp", "int f() { return"),
        // A different grammar accepting part of this source must not rescue it.
        (
            "javascript",
            "type State = { count: number }; function f() { const value = ; return value; }",
        ),
    ];
    let hasher = StructuralHasher::new();

    for (language, source) in cases {
        assert!(
            matches!(hasher.compute_hash(source, language, None), Err(CoreError::Parse(_))),
            "{language}: whole-file API must reject recovered syntax"
        );
        let error = match hasher.compute_granular_hash_with_analysis(source, language, None) {
            Ok(_) => panic!("{language} accepted malformed source as a stable identity"),
            Err(error) => error,
        };
        assert!(
            matches!(error, CoreError::Parse(_)),
            "{language} returned the wrong failure class: {error}"
        );
    }
}

struct ImportParityCase {
    lang: &'static str,
    source: &'static str,
    expect_kind: ImportKind,
    /// Local binding name or path substring that must appear on a marker.
    expect_path_or_alias: &'static str,
    /// If set, require alias == Some(this).
    expect_alias: Option<&'static str>,
}

fn import_parity_cases() -> Vec<ImportParityCase> {
    vec![
        ImportParityCase {
            lang: "python",
            source: "from m import x as z\ndef f():\n    z()\n",
            expect_kind: ImportKind::Named,
            expect_path_or_alias: "m",
            expect_alias: Some("z"),
        },
        ImportParityCase {
            lang: "python",
            source: "from m import *\ndef f():\n    pass\n",
            expect_kind: ImportKind::Star,
            expect_path_or_alias: "m",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "python",
            source: "import os\ndef f():\n    os.getcwd()\n",
            expect_kind: ImportKind::Module,
            expect_path_or_alias: "os",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "javascript",
            source: "import { a as b } from 'mod';\nfunction f() { b(); }\n",
            expect_kind: ImportKind::Named,
            expect_path_or_alias: "a",
            expect_alias: Some("b"),
        },
        ImportParityCase {
            lang: "javascript",
            source: "import defName from 'mod';\nfunction f() { defName(); }\n",
            expect_kind: ImportKind::Module,
            expect_path_or_alias: "mod",
            expect_alias: Some("defName"),
        },
        ImportParityCase {
            lang: "javascript",
            source: "import * as ns from 'mod';\nfunction f() { ns.x(); }\n",
            expect_kind: ImportKind::Alias,
            expect_path_or_alias: "mod",
            expect_alias: Some("ns"),
        },
        ImportParityCase {
            lang: "javascript",
            source: "import 'mod';\nfunction f() {}\n",
            expect_kind: ImportKind::Module,
            expect_path_or_alias: "mod",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "typescript",
            source: "import { a as b } from 'mod';\nfunction f() { b(); }\n",
            expect_kind: ImportKind::Named,
            expect_path_or_alias: "a",
            expect_alias: Some("b"),
        },
        ImportParityCase {
            lang: "typescript",
            source: "import defName from 'mod';\nfunction f() { defName(); }\n",
            expect_kind: ImportKind::Module,
            expect_path_or_alias: "mod",
            expect_alias: Some("defName"),
        },
        ImportParityCase {
            lang: "typescript",
            source: "import * as ns from 'mod';\nfunction f() { ns.x(); }\n",
            expect_kind: ImportKind::Alias,
            expect_path_or_alias: "mod",
            expect_alias: Some("ns"),
        },
        ImportParityCase {
            lang: "tsx",
            source: "import { a as b } from 'mod';\nfunction f() { b(); }\n",
            expect_kind: ImportKind::Named,
            expect_path_or_alias: "a",
            expect_alias: Some("b"),
        },
        ImportParityCase {
            lang: "tsx",
            source: "import * as ns from 'mod';\nfunction f() { ns.x(); }\n",
            expect_kind: ImportKind::Alias,
            expect_path_or_alias: "mod",
            expect_alias: Some("ns"),
        },
        ImportParityCase {
            lang: "java",
            source: "import pkg.Type;\nclass C { void f() { } }\n",
            expect_kind: ImportKind::Named,
            expect_path_or_alias: "pkg.Type",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "java",
            source: "import pkg.*;\nclass C { void f() { } }\n",
            expect_kind: ImportKind::Star,
            expect_path_or_alias: "pkg",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "java",
            source: "import static pkg.Util.method;\nclass C { void f() { } }\n",
            expect_kind: ImportKind::Static,
            expect_path_or_alias: "pkg.Util.method",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "go",
            source: "package p\nimport alias \"fmt\"\nfunc f() {}\n",
            expect_kind: ImportKind::Alias,
            expect_path_or_alias: "fmt",
            expect_alias: Some("alias"),
        },
        ImportParityCase {
            lang: "go",
            source: "package p\nimport \"fmt\"\nfunc f() {}\n",
            expect_kind: ImportKind::Module,
            expect_path_or_alias: "fmt",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "csharp",
            source: "using X = System.IO;\nclass C { void F() { } }\n",
            expect_kind: ImportKind::Alias,
            expect_path_or_alias: "System.IO",
            expect_alias: Some("X"),
        },
        ImportParityCase {
            lang: "csharp",
            source: "using System.IO;\nclass C { void F() { } }\n",
            expect_kind: ImportKind::Module,
            expect_path_or_alias: "System.IO",
            expect_alias: None,
        },
        ImportParityCase {
            lang: "rust",
            source: "use foo::bar as Baz;\nfn f() {}\n",
            expect_kind: ImportKind::Alias,
            expect_path_or_alias: "foo::bar",
            expect_alias: Some("Baz"),
        },
        ImportParityCase {
            lang: "rust",
            source: "use foo::*;\nfn f() {}\n",
            expect_kind: ImportKind::Star,
            expect_path_or_alias: "foo",
            expect_alias: None,
        },
    ]
}

#[test]
fn import_marker_parity_spans_kind_alias_all_languages() {
    let hasher = StructuralHasher::new();
    for case in import_parity_cases() {
        let with_m = hasher
            .compute_granular_hash_with_analysis(case.source, case.lang, None)
            .unwrap_or_else(|e| panic!("{} analysis: {e}", case.lang));
        let without = hasher
            .compute_granular_hash(case.source, case.lang, None)
            .unwrap_or_else(|e| panic!("{} plain: {e}", case.lang));
        assert_eq!(
            with_m.file_hash, without.file_hash,
            "{} import fixture file_hash isolation",
            case.lang
        );
        let markers = with_m
            .markers
            .unwrap_or_else(|| panic!("{} expected markers", case.lang));
        assert!(
            markers.imports.iter().any(|i| i.start_byte < i.end_byte),
            "{} expected nonzero import spans, got {:?}",
            case.lang,
            markers.imports
        );
        let matched = markers.imports.iter().any(|i| {
            if i.kind != case.expect_kind {
                return false;
            }
            if let Some(alias) = case.expect_alias {
                if i.alias.as_deref() != Some(alias) {
                    return false;
                }
            }
            i.path.contains(case.expect_path_or_alias)
                || i.alias.as_ref().is_some_and(|a| a.contains(case.expect_path_or_alias))
        });
        assert!(
            matched,
            "{} missing import kind={:?} path/alias containing {:?} alias={:?}, got {:?}",
            case.lang, case.expect_kind, case.expect_path_or_alias, case.expect_alias, markers.imports
        );
    }
}

#[test]
fn import_markers_preserve_named_targets_for_default_resolver() {
    let cases = [
        (
            "python",
            "from package.module import original as local\ndef f():\n    local()\n",
            "local",
            "package.module.original",
        ),
        (
            "javascript",
            "import { original as local } from 'package';\nfunction f() { local(); }\n",
            "local",
            "package.original",
        ),
        (
            "typescript",
            "import { original as local } from 'package';\nfunction f() { local(); }\n",
            "local",
            "package.original",
        ),
        (
            "tsx",
            "import { original as local } from 'package';\nfunction f() { local(); }\n",
            "local",
            "package.original",
        ),
        (
            "java",
            "import static package.Type.original;\nclass C { void f() { original(); } }\n",
            "original",
            "package.Type.original",
        ),
        (
            "go",
            "package p\nimport local \"example.com/package\"\nfunc f() { local.Original() }\n",
            "local.Original",
            "example.com/package.Original",
        ),
        (
            "csharp",
            "using Local = Package.Type;\nclass C { void F() { Local.Original(); } }\n",
            "Local.Original",
            "Package.Type.Original",
        ),
        (
            "rust",
            "use package::original as local;\nfn f() { local(); }\n",
            "local",
            "package::original",
        ),
    ];
    let hasher = StructuralHasher::new();

    for (lang, source, syntactic_callee, expected) in cases {
        let result = hasher
            .compute_granular_hash_with_analysis(source, lang, None)
            .unwrap_or_else(|error| panic!("{lang} analysis: {error}"));
        let record = entmoot_core::build_file_index(&result);
        let resolved = resolve_file_calls("fixture", &record);
        let call = resolved
            .iter()
            .find(|call| call.syntactic_callee == syntactic_callee)
            .unwrap_or_else(|| panic!("{lang} did not emit {syntactic_callee}(): {resolved:?}"));
        assert_eq!(call.resolved_fqn.as_deref(), Some(expected), "{lang}");
    }
}

#[test]
fn lexical_parameters_do_not_resolve_to_shadowed_imports_in_any_ecosystem() {
    let cases = [
        ("python", "from package import target\ndef f(target):\n    target()\n"),
        (
            "javascript",
            "import { target } from 'package';\nfunction f(target) { target(); }\n",
        ),
        (
            "typescript",
            "import { target } from 'package';\nfunction f(target: () => void) { target(); }\n",
        ),
        (
            "tsx",
            "import { target } from 'package';\nfunction f(target: () => void) { target(); return <div />; }\n",
        ),
        (
            "java",
            "import static package.Util.target;\nclass C { void f(Runnable target) { target.run(); } }\n",
        ),
        (
            "go",
            "package p\nimport target \"example.com/package\"\nfunc f(target func()) { target() }\n",
        ),
        (
            "csharp",
            "using target = Package.Target;\nclass C { void F(System.Action target) { target(); } }\n",
        ),
        ("rust", "use package::target;\nfn f(target: fn()) { target(); }\n"),
    ];
    let hasher = StructuralHasher::new();

    for (lang, source) in cases {
        let result = hasher
            .compute_granular_hash_with_analysis(source, lang, None)
            .unwrap_or_else(|error| panic!("{lang} analysis: {error}"));
        let record = entmoot_core::build_file_index(&result);
        let resolved = resolve_file_calls("fixture", &record);
        assert!(!resolved.is_empty(), "{lang} emitted no call site");
        assert!(
            resolved
                .iter()
                .all(|call| call.confidence != entmoot_core::ResolveConfidence::ImportLocal),
            "{lang} resolved a lexically shadowed import: {resolved:?}"
        );
    }
}

#[test]
fn chained_call_semantics_are_preserved_in_every_ecosystem() {
    struct Case {
        lang: &'static str,
        key: &'static str,
        baseline: &'static str,
        changed_receiver: &'static str,
        changed_argument: &'static str,
    }
    let cases = [
        Case {
            lang: "python",
            key: "f",
            baseline: "def f():\n    return Alpha.make(1).run()\n",
            changed_receiver: "def f():\n    return Beta.make(1).run()\n",
            changed_argument: "def f():\n    return Alpha.make(2).run()\n",
        },
        Case {
            lang: "javascript",
            key: "f",
            baseline: "function f() { return Alpha.make(1).run(); }",
            changed_receiver: "function f() { return Beta.make(1).run(); }",
            changed_argument: "function f() { return Alpha.make(2).run(); }",
        },
        Case {
            lang: "typescript",
            key: "f",
            baseline: "function f() { return Alpha.make(1).run(); }",
            changed_receiver: "function f() { return Beta.make(1).run(); }",
            changed_argument: "function f() { return Alpha.make(2).run(); }",
        },
        Case {
            lang: "tsx",
            key: "f",
            baseline: "function f() { return Alpha.make(1).run(); }",
            changed_receiver: "function f() { return Beta.make(1).run(); }",
            changed_argument: "function f() { return Alpha.make(2).run(); }",
        },
        Case {
            lang: "java",
            key: "f",
            baseline: "class C { Object f() { return Alpha.make(1).run(); } }",
            changed_receiver: "class C { Object f() { return Beta.make(1).run(); } }",
            changed_argument: "class C { Object f() { return Alpha.make(2).run(); } }",
        },
        Case {
            lang: "csharp",
            key: "f",
            baseline: "class C { object f() { return Alpha.Make(1).Run(); } }",
            changed_receiver: "class C { object f() { return Beta.Make(1).Run(); } }",
            changed_argument: "class C { object f() { return Alpha.Make(2).Run(); } }",
        },
        Case {
            lang: "go",
            key: "f",
            baseline: "package p\nfunc f() any { return Alpha(1).Run() }\n",
            changed_receiver: "package p\nfunc f() any { return Beta(1).Run() }\n",
            changed_argument: "package p\nfunc f() any { return Alpha(2).Run() }\n",
        },
        Case {
            lang: "rust",
            key: "f",
            baseline: "fn f() { Alpha::make(1).run(); }",
            changed_receiver: "fn f() { Beta::make(1).run(); }",
            changed_argument: "fn f() { Alpha::make(2).run(); }",
        },
    ];
    let hasher = StructuralHasher::new();
    for case in cases {
        let structural_hash = |source| {
            hasher
                .compute_granular_hash(source, case.lang, None)
                .unwrap_or_else(|error| panic!("{}: {error}", case.lang))
                .callables[case.key]
                .structural_hash
                .clone()
        };
        let baseline = structural_hash(case.baseline);
        assert_ne!(
            baseline,
            structural_hash(case.changed_receiver),
            "{} erased the chained-call receiver",
            case.lang
        );
        assert_ne!(
            baseline,
            structural_hash(case.changed_argument),
            "{} erased a chained-call receiver argument",
            case.lang
        );
    }
}

#[test]
fn javascript_default_import_exposes_its_local_binding_to_resolver() {
    let source = "import local from 'package';\nfunction f() { local(); }\n";
    let hasher = StructuralHasher::new();
    let result = hasher
        .compute_granular_hash_with_analysis(source, "javascript", None)
        .expect("javascript analysis");
    let record = entmoot_core::build_file_index(&result);
    let resolved = resolve_file_calls("fixture.js", &record);
    let call = resolved
        .iter()
        .find(|call| call.syntactic_callee == "local")
        .unwrap_or_else(|| panic!("did not emit local() call: {resolved:?}"));

    assert_eq!(call.resolved_fqn.as_deref(), Some("package"));
}

#[test]
fn python_dotted_module_import_binds_its_first_segment() {
    let source = "import package.module\ndef f():\n    package.module.run()\n    package.other()\n";
    let hasher = StructuralHasher::new();
    let result = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .expect("python analysis");
    let markers = result.markers.as_ref().expect("markers");
    assert!(markers
        .imports
        .iter()
        .any(|import| { import.path == "package.module" && import.alias.as_deref() == Some("package") }));

    let record = entmoot_core::build_file_index(&result);
    let resolved = resolve_file_calls("fixture.py", &record);
    let call = resolved
        .iter()
        .find(|call| call.syntactic_callee == "package.module.run")
        .unwrap_or_else(|| panic!("did not emit dotted call: {resolved:?}"));
    assert_eq!(call.resolved_fqn.as_deref(), Some("package.module.run"));
    let unrelated = resolved
        .iter()
        .find(|call| call.syntactic_callee == "package.other")
        .unwrap_or_else(|| panic!("did not emit unrelated dotted call: {resolved:?}"));
    assert_eq!(unrelated.confidence, entmoot_core::ResolveConfidence::SyntacticOnly);
    assert!(unrelated.resolved_fqn.is_none());
}

#[test]
fn python_plain_named_import_preserves_the_complete_target() {
    let source = "from package.module import original\ndef f():\n    original()\n";
    let hasher = StructuralHasher::new();
    let result = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .expect("python analysis");
    let record = entmoot_core::build_file_index(&result);
    let resolved = resolve_file_calls("fixture.py", &record);
    let call = resolved
        .iter()
        .find(|call| call.syntactic_callee == "original")
        .unwrap_or_else(|| panic!("did not emit original() call: {resolved:?}"));

    assert_eq!(call.resolved_fqn.as_deref(), Some("package.module.original"));
}
