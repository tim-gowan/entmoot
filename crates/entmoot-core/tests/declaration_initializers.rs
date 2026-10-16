//! Initializer edits are syntax evidence, never callable or runtime-value identity.

use entmoot_core::StructuralHasher;

const CASES: &[(&str, &str)] = &[
    ("rust", "const LIMIT: i32 = -1; fn get() -> i32 { LIMIT }"),
    ("python", "LIMIT = -1\ndef get():\n    return LIMIT\n"),
    ("javascript", "const LIMIT = -1; function get() { return LIMIT; }"),
    (
        "typescript",
        "const LIMIT: number = -1; function get() { return LIMIT; }",
    ),
    ("tsx", "const LIMIT: number = -1; function get() { return LIMIT; }"),
    ("go", "package p\nconst LIMIT = -1\nfunc get() int { return LIMIT }"),
    (
        "java",
        "class Config { static final int LIMIT = -1; int get() { return LIMIT; } }",
    ),
    (
        "csharp",
        "class Config { const int LIMIT = -1; int get() { return LIMIT; } }",
    ),
    ("c", "const int LIMIT = -1; int get(void) { return LIMIT; }"),
];

#[test]
fn constant_only_edits_are_addressable_in_every_claimed_family() {
    let hasher = StructuralHasher::new();
    for &(language, source) in CASES {
        let base = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let changed = source.replace("-1", "8192");
        let head = hasher
            .compute_granular_hash_with_analysis(&changed, language, None)
            .unwrap();
        assert_ne!(base.file_hash, head.file_hash, "{language}");
        assert!(!base.callables.is_empty(), "{language}");
        for (name, callable) in &base.callables {
            assert_eq!(
                callable.structural_hash, head.callables[name].structural_hash,
                "{language}:{name}"
            );
        }
        let declaration = |result: &entmoot_core::granular::GranularHashResult| {
            let markers = result.markers.as_ref().unwrap();
            let item = markers
                .declarations
                .iter()
                .find(|d| d.name == "LIMIT")
                .unwrap_or_else(|| panic!("missing LIMIT declaration for {language}"));
            serde_json::to_value(item).unwrap()
        };
        let before = declaration(&base);
        let after = declaration(&head);
        assert_eq!(before["site"]["occurrence"], after["site"]["occurrence"], "{language}");
        assert_eq!(
            before["syntax"]["initializer"]["value"],
            serde_json::json!({"kind": "decimal_integer", "text": "-1"}),
            "{language}"
        );
        assert_eq!(
            after["syntax"]["initializer"]["value"],
            serde_json::json!({"kind": "decimal_integer", "text": "8192"}),
            "{language}"
        );
        assert_ne!(
            before["syntax"]["initializer"]["content_digest"], after["syntax"]["initializer"]["content_digest"],
            "{language}"
        );
    }
}

#[test]
fn dynamic_initializers_never_become_literal_values() {
    let hasher = StructuralHasher::new();
    for &(language, source) in CASES {
        let source = source.replace("-1", "compute()");
        let result = hasher
            .compute_granular_hash_with_analysis(&source, language, None)
            .unwrap();
        let item = result
            .markers
            .unwrap()
            .declarations
            .into_iter()
            .find(|d| d.name == "LIMIT")
            .unwrap_or_else(|| panic!("missing dynamic declaration for {language}"));
        let item = serde_json::to_value(item).unwrap();
        assert_eq!(
            item["syntax"]["initializer"]["value"],
            serde_json::json!({"kind": "unevaluated"}),
            "{language}"
        );
        assert!(item["syntax"]["initializer"]["span"].is_object(), "{language}");
    }
}

#[test]
fn spans_are_exact_and_literal_observations_are_conservative() {
    let hasher = StructuralHasher::new();
    for &(language, original) in CASES {
        for value in ["17", "0", "- 1", "1 + 2", "0x10", "-0"] {
            let source = original.replace("-1", value);
            let result = hasher
                .compute_granular_hash_with_analysis(&source, language, None)
                .unwrap();
            let item = result
                .markers
                .unwrap()
                .declarations
                .into_iter()
                .find(|d| d.name == "LIMIT")
                .unwrap();
            let syntax = item.syntax.unwrap();
            syntax.validate(item.site.span).unwrap();
            assert_eq!(&source[syntax.name_span.start_byte..syntax.name_span.end_byte], "LIMIT");
            let init = syntax.initializer.unwrap();
            assert_eq!(&source[init.span.start_byte..init.span.end_byte], value, "{language}");
            let expected = match value {
                "17" => Some("17"),
                "0" => Some("0"),
                "- 1" => Some("-1"),
                _ => None,
            };
            assert_eq!(init.integer().as_deref(), expected, "{language}: {value}");
        }
    }
}

#[test]
fn complex_patterns_and_assignment_targets_do_not_invent_bindings() {
    for (language, source) in [
        ("python", "a, b = pair()\nobj.field = -1\n"),
        ("javascript", "const {a, b} = pair(); obj.field = -1;"),
        ("typescript", "const [a, b] = pair(); obj.field = -1;"),
        ("tsx", "const [a, b] = pair(); obj.field = -1;"),
        ("rust", "fn f() { let (a, b) = pair(); obj.field = -1; }"),
        ("go", "package p\nvar a, b = pair()\n"),
    ] {
        let result = StructuralHasher::new()
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        assert!(
            result.markers.unwrap().declarations.iter().all(|d| d.syntax.is_none()),
            "{language}"
        );
    }
}

#[test]
fn repeated_bindings_keep_distinct_addresses_and_class_scope() {
    let source = "class A:\n    LIMIT = -1\nclass B:\n    LIMIT = 2\nx = 3\nx = 4\n";
    let result = StructuralHasher::new()
        .compute_granular_hash_with_analysis(source, "python", None)
        .unwrap();
    let declarations = result.markers.unwrap().declarations;
    let limits: Vec<_> = declarations.iter().filter(|d| d.name == "LIMIT").collect();
    assert_eq!(limits[0].scope, ["A"]);
    assert_eq!(limits[1].scope, ["B"]);
    assert_ne!(limits[0].site.occurrence, limits[1].site.occurrence);
    let assignments: Vec<_> = declarations.iter().filter(|d| d.name == "x").collect();
    assert_eq!(assignments.len(), 2);
    assert_ne!(assignments[0].site.occurrence, assignments[1].site.occurrence);
}

#[test]
fn absent_initializer_and_fixed_buffer_size_are_not_initializers() {
    for (language, source) in [
        ("rust", "static LIMIT: i32;"),
        ("python", "LIMIT: int\n"),
        ("javascript", "let LIMIT;"),
        ("typescript", "let LIMIT: number;"),
        ("tsx", "let LIMIT: number;"),
        ("go", "package p\nvar LIMIT int\n"),
        ("java", "class Config { int LIMIT; }"),
        ("csharp", "class Config { int LIMIT; }"),
        ("csharp", "unsafe struct Config { public fixed int LIMIT[10]; }"),
        ("c", "int LIMIT;"),
    ] {
        let result = StructuralHasher::new()
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let item = result
            .markers
            .unwrap()
            .declarations
            .into_iter()
            .find(|d| d.name == "LIMIT")
            .unwrap_or_else(|| panic!("missing uninitialized syntax for {language}"));
        assert!(item.syntax.unwrap().initializer.is_none(), "{language}: {source}");
    }
}

#[test]
fn cpp_initializer_evidence_is_experimental_only() {
    let result = StructuralHasher::new()
        .compute_granular_hash_with_analysis("const int LIMIT = -1;", "cpp", None)
        .unwrap();
    let item = result
        .markers
        .unwrap()
        .declarations
        .into_iter()
        .find(|d| d.name == "LIMIT")
        .unwrap();
    assert_eq!(
        item.syntax.unwrap().initializer.unwrap().integer().as_deref(),
        Some("-1")
    );
}

#[test]
fn local_binding_scope_retains_the_inventoried_callable_owner() {
    for (language, source) in [
        ("rust", "fn owner() { let LIMIT = -1; }"),
        ("python", "def owner():\n    LIMIT = -1\n"),
        ("javascript", "function owner() { let LIMIT = -1; }"),
        ("typescript", "function owner() { let LIMIT = -1; }"),
        ("tsx", "function owner() { let LIMIT = -1; }"),
        ("go", "package p\nfunc owner() { LIMIT := -1 }"),
        ("java", "class Config { void owner() { int LIMIT = -1; } }"),
        ("csharp", "class Config { void owner() { int LIMIT = -1; } }"),
        ("c", "void owner(void) { int LIMIT = -1; }"),
    ] {
        let result = StructuralHasher::new()
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let markers = result.markers.unwrap();
        let declaration = markers.declarations.iter().find(|d| d.name == "LIMIT").unwrap();
        let owner = markers.definitions.iter().find(|d| d.name == "owner").unwrap();
        let mut expected = owner.scope.clone();
        expected.push(owner.name.clone());
        assert_eq!(declaration.scope, expected, "{language}");
    }
}

#[test]
fn recovered_javascript_trees_do_not_assert_initializer_absence() {
    // A typed declaration is qualified only by its declared language grammar.
    let result =
        StructuralHasher::new().compute_granular_hash_with_analysis("const LIMIT: number = -1;", "javascript", None);
    assert!(matches!(result, Err(entmoot_core::CoreError::Parse(_))));
}
