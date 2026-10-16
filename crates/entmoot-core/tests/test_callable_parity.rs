//! Wave C Segment 8 — JS/TS assigned arrow / function-expression callables.
#![cfg(feature = "full-tests")]

use entmoot_core::StructuralHasher;
use std::fs;
use std::path::PathBuf;

#[test]
fn named_declaration_inventory_has_equivalent_identity_controls_in_every_language() {
    let hasher = StructuralHasher::new();
    for (language, source) in [
        ("rust", "pub struct Thing { pub value: i32 }"),
        ("go", "package fixture\ntype Thing struct { Value int }\n"),
        ("python", "class Thing:\n    pass\n"),
        ("javascript", "class Thing {}"),
        ("typescript", "class Thing {}"),
        ("tsx", "class Thing {}"),
        ("java", "public class Thing {}"),
        ("csharp", "public class Thing {}"),
        ("c", "struct Thing { int value; };"),
        ("cpp", "class Thing { public: int value; };"),
    ] {
        let with = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap_or_else(|error| panic!("{language}: {error}"));
        let hash_only = hasher.compute_granular_hash(source, language, None).unwrap();
        assert_eq!(
            with.file_hash, hash_only.file_hash,
            "{language}: inventory must not alter structural hashing"
        );
        let declarations = &with.markers.as_ref().unwrap().declarations;
        let declared: Vec<_> = declarations
            .iter()
            .filter(|declaration| declaration.name == "Thing")
            .collect();
        assert_eq!(declared.len(), 1, "{language}: {declarations:?}");
        let declaration = declared[0];
        declaration.validate().unwrap();
        let wire = serde_json::to_value(declaration).unwrap();
        assert_eq!(
            serde_json::from_value::<entmoot_core::DeclarationMarker>(wire.clone()).unwrap(),
            *declaration
        );
        for mutation in 0..7 {
            let mut invalid = wire.clone();
            match mutation {
                0 => {
                    invalid["site"]["occurrence"]
                        .as_object_mut()
                        .unwrap()
                        .remove("address_id");
                }
                1 => {
                    invalid["site"]["occurrence"]
                        .as_object_mut()
                        .unwrap()
                        .remove("logic_address");
                }
                2 => {
                    invalid["site"]["span"].as_object_mut().unwrap().remove("end_byte");
                }
                3 => invalid["callable_address_id"] = serde_json::json!("legacy"),
                4 => invalid["site"]["occurrence"]["logic_address"] = serde_json::json!("forged"),
                5 => invalid["site"]["span"]["end_byte"] = serde_json::json!(0),
                _ => invalid["callable"] = wire["site"].clone(),
            }
            let accepted = serde_json::from_value::<entmoot_core::DeclarationMarker>(invalid)
                .is_ok_and(|value| value.validate().is_ok());
            assert!(!accepted, "{language}: mutation {mutation}");
        }
        assert!(
            declaration.callable.is_none(),
            "{language}: named type is not a callable body"
        );
        assert!(declaration.site.span.end_byte > declaration.site.span.start_byte);
    }
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/entmoot-core/tests/fixtures/callable_parity")
}

#[test]
fn named_type_and_alias_kinds_have_syntax_owned_addresses() {
    let hasher = StructuralHasher::new();
    for (language, source, kind) in [
        ("rust", "pub type Thing = u64;", "type_alias"),
        ("go", "package fixture\ntype Thing int\n", "type"),
        ("go", "package fixture\ntype Thing = int\n", "type_alias"),
        ("typescript", "export type Thing = string | number;", "type_alias"),
        ("tsx", "export type Thing = string | number;", "type_alias"),
        ("c", "typedef unsigned long Thing;", "type_alias"),
        ("cpp", "using Thing = unsigned long;", "type_alias"),
    ] {
        let value = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let markers = value.markers.unwrap();
        let declaration = markers
            .declarations
            .iter()
            .find(|row| row.name == "Thing")
            .unwrap_or_else(|| panic!("{language}: {:?}", markers.declarations));
        assert_eq!(declaration.kind, kind, "{language}");
        assert!(declaration.callable.is_none());
        declaration.validate().unwrap();
        assert!(value.callables.is_empty(), "{language}");
    }
}

fn assert_callable_named(lang: &str, source: &str, name: &str) {
    let hasher = StructuralHasher::new();
    let with = hasher
        .compute_granular_hash_with_analysis(source, lang, None)
        .unwrap_or_else(|e| panic!("{lang} analysis: {e}"));
    assert!(
        with.callables.contains_key(name),
        "{lang}: expected callable `{name}`, got keys {:?}",
        with.callables.keys().collect::<Vec<_>>()
    );
    let Some(markers) = with.markers else {
        panic!("{lang}: expected analysis markers");
    };
    assert!(
        markers.definitions.iter().any(|d| d.name == name),
        "{lang}: expected DefMarker `{name}`, got {:?}",
        markers.definitions
    );
}

#[test]
fn export_const_arrow_emits_lhs_callable_js() {
    let source = fs::read_to_string(fixtures_dir().join("export_const_arrow.js")).unwrap();
    assert_callable_named("javascript", &source, "handler");
}

#[test]
fn export_const_arrow_emits_lhs_callable_ts() {
    let source = fs::read_to_string(fixtures_dir().join("export_const_arrow.ts")).unwrap();
    assert_callable_named("typescript", &source, "handler");
}

#[test]
fn export_const_arrow_emits_lhs_callable_tsx() {
    let source = fs::read_to_string(fixtures_dir().join("export_const_arrow.ts")).unwrap();
    assert_callable_named("tsx", &source, "handler");
}

#[test]
fn const_arrow_emits_lhs_callable() {
    let source = fs::read_to_string(fixtures_dir().join("const_arrow.js")).unwrap();
    assert_callable_named("javascript", &source, "local");
}

#[test]
fn const_function_expression_emits_lhs_callable() {
    let source = fs::read_to_string(fixtures_dir().join("const_function_expr.js")).unwrap();
    assert_callable_named("javascript", &source, "named");
}

#[test]
fn export_function_declaration_still_tracked() {
    let source = fs::read_to_string(fixtures_dir().join("export_function.ts")).unwrap();
    assert_callable_named("typescript", &source, "keep");
}

#[test]
fn function_declaration_hash_isolation_unchanged() {
    let hasher = StructuralHasher::new();
    let source = "function already() { return 1; }\n";
    let with = hasher
        .compute_granular_hash_with_analysis(source, "javascript", None)
        .unwrap();
    let without = hasher.compute_granular_hash(source, "javascript", None).unwrap();
    assert_eq!(with.file_hash, without.file_hash);
    assert!(with.callables.contains_key("already"));
}

#[test]
fn assigned_lambda_emits_lhs_callable_python() {
    let source = fs::read_to_string(fixtures_dir().join("assigned_lambda.py")).unwrap();
    let hasher = StructuralHasher::new();
    let result = hasher
        .compute_granular_hash_with_analysis_at(
            &source,
            "python",
            Some("assigned_lambda.py"),
            Some("example/repo"),
            None,
        )
        .unwrap();
    let callable = &result.callables["handler"];
    let declarations = &result.markers.as_ref().unwrap().declarations;
    let declaration = declarations
        .iter()
        .find(|candidate| candidate.name == "handler")
        .unwrap();
    assert_eq!(declaration.kind, "assigned_callable");
    assert_eq!(
        declaration.callable.as_ref().unwrap().occurrence.address_id.as_str(),
        callable.address_id.as_str()
    );
    assert_ne!(declaration.site.occurrence.logic_address, callable.logic_address);
    assert!(declaration
        .site
        .occurrence
        .logic_address
        .contains("|assigned_callable|"));
}

#[test]
fn python_class_is_an_addressable_non_callable_declaration() {
    let hasher = StructuralHasher::new();
    let result = hasher
        .compute_granular_hash_with_analysis_at(
            "class Widget:\n    def run(self):\n        return 1\n",
            "python",
            Some("widget.py"),
            Some("example/repo"),
            None,
        )
        .unwrap();
    assert!(!result.callables.contains_key("Widget"));
    let declaration = result
        .markers
        .as_ref()
        .unwrap()
        .declarations
        .iter()
        .find(|candidate| candidate.name == "Widget")
        .unwrap();
    assert_eq!(declaration.kind, "class");
    assert!(declaration.site.occurrence.logic_address.contains("|Widget|class|"));
    assert!(declaration.callable.is_none());
}

#[test]
fn def_function_still_tracked_python() {
    let source = fs::read_to_string(fixtures_dir().join("def_function.py")).unwrap();
    assert_callable_named("python", &source, "keep");
}

#[test]
fn unbound_lambda_does_not_emit_callable_python() {
    let hasher = StructuralHasher::new();
    let source = "xs = [lambda x: x]\n";
    let with = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .unwrap();
    assert!(
        with.callables.is_empty(),
        "expected no callables for list-embedded lambda, got {:?}",
        with.callables.keys().collect::<Vec<_>>()
    );
}

fn assert_no_callable_named(lang: &str, source: &str, forbidden: &str) {
    let hasher = StructuralHasher::new();
    let with = hasher
        .compute_granular_hash_with_analysis(source, lang, None)
        .unwrap_or_else(|e| panic!("{lang} analysis: {e}"));
    assert!(
        !with.callables.contains_key(forbidden),
        "{lang}: nested binder `{forbidden}` must stay hash-only, got keys {:?}",
        with.callables.keys().collect::<Vec<_>>()
    );
    if let Some(markers) = &with.markers {
        assert!(
            markers.definitions.iter().all(|d| d.name != forbidden),
            "{lang}: nested binder `{forbidden}` must not emit DefMarker, got {:?}",
            markers.definitions
        );
    }
}

#[test]
fn nested_const_arrow_stays_hash_only_js() {
    let source = fs::read_to_string(fixtures_dir().join("nested_const_arrow.js")).unwrap();
    assert_callable_named("javascript", &source, "outer");
    assert_no_callable_named("javascript", &source, "nested");
}

#[test]
fn nested_assigned_lambda_stays_hash_only_python() {
    let source = fs::read_to_string(fixtures_dir().join("nested_assigned_lambda.py")).unwrap();
    assert_callable_named("python", &source, "outer");
    assert_no_callable_named("python", &source, "nested");
}

#[test]
fn def_hash_isolation_unchanged_python() {
    let hasher = StructuralHasher::new();
    let source = "def already():\n    return 1\n";
    let with = hasher
        .compute_granular_hash_with_analysis(source, "python", None)
        .unwrap();
    let without = hasher.compute_granular_hash(source, "python", None).unwrap();
    assert_eq!(with.file_hash, without.file_hash);
    assert!(with.callables.contains_key("already"));
}

#[test]
fn python_global_model_reference_changes_callable_structural_hash() {
    let hasher = StructuralHasher::new();
    let first = hasher
        .compute_granular_hash(
            "def get_queryset(self):\n    return Announcement.objects.all().order_by('id')\n",
            "python",
            None,
        )
        .unwrap();
    let second = hasher
        .compute_granular_hash(
            "def get_queryset(self):\n    return Sonarqube_Issue.objects.all().order_by('id')\n",
            "python",
            None,
        )
        .unwrap();
    assert_ne!(
        first.callables["get_queryset"].structural_hash, second.callables["get_queryset"].structural_hash,
        "unbound global/model names are semantic and must not be alpha-renamed"
    );
}

/// Receiver spelling and arguments remain structural inputs; chained sites retain syntax.
#[test]
fn python_chained_call_keeps_receiver_arguments_and_local_spelling() {
    let hasher = StructuralHasher::with_policy(entmoot_core::StructuralHashPolicy::v1());
    let build = |source: &str| {
        hasher
            .compute_granular_hash_with_analysis(source, "python", None)
            .unwrap()
    };
    let first = build("def make():\n    builder = Factory('A')\n    return builder.configure(1).build()\n");
    let renamed = build("def make():\n    worker = Factory('A')\n    return worker.configure(1).build()\n");
    let changed_argument = build("def make():\n    builder = Factory('B')\n    return builder.configure(1).build()\n");
    assert_ne!(
        first.callables["make"].structural_hash,
        renamed.callables["make"].structural_hash
    );
    assert_ne!(
        first.callables["make"].structural_hash,
        changed_argument.callables["make"].structural_hash
    );
    let markers = first.markers.unwrap();
    assert!(markers
        .call_sites
        .iter()
        .any(|call| call.callee == "builder.configure(1).build"));

    let sound = StructuralHasher::new()
        .compute_granular_hash_with_analysis(
            "def make():\n    builder = Factory('A')\n    return builder.configure(1).build()\n",
            "python",
            None,
        )
        .unwrap();
    assert!(sound
        .markers
        .unwrap()
        .call_sites
        .iter()
        .any(|call| call.callee == "builder.configure(1).build"));
}

#[test]
fn static_property_bodies_are_inventoried_with_stable_spans_and_body_sensitive_hashes() {
    let hasher = StructuralHasher::new();
    for language in ["javascript", "typescript", "tsx"] {
        for body in ["function(x) { return x + 1; }", "(x) => x + 1"] {
            let source = format!("const req = {{}}; function outer() {{ req.first = req.second = ({body}); }}");
            let result = hasher
                .compute_granular_hash_with_analysis(&source, language, None)
                .unwrap();
            assert_eq!(result.callables.len(), 2);
            let callable = &result.callables["req.second"];
            assert_eq!(&source[callable.start_byte..callable.end_byte], body);
            assert!(!callable.address_id.is_empty());
            assert!(callable.exact_body_id.is_some());
            let changed = hasher
                .compute_granular_hash(&source.replace("x + 1", "x - 1"), language, None)
                .unwrap();
            assert_ne!(
                callable.structural_hash,
                changed.callables["req.second"].structural_hash
            );
        }
        for assignment in [
            "req[key] = function() {}",
            "factory().method = function() {}",
            "const callbacks = [function() {}]",
        ] {
            let result = hasher.compute_granular_hash(assignment, language, None).unwrap();
            assert!(result.callables.is_empty(), "{language}: {assignment}");
        }
    }
}
