//! Authored boundary expectations over production inventory; no compiler needed.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_core::{GranularHashResult, StructuralHasher};
use serde_json::Value;

fn inventory(source: &str, language: &str) -> GranularHashResult {
    StructuralHasher::new()
        .compute_granular_hash_with_analysis(source, language, None)
        .unwrap_or_else(|e| panic!("{language}: {e}"))
}

fn contract(result: &GranularHashResult, name: &str) -> Value {
    let markers = serde_json::to_value(result.markers.as_ref().unwrap()).unwrap();
    let records = markers["code_contracts"].as_array().expect("structured contracts");
    let selected: Vec<_> = records.iter().filter(|r| r["name"] == name).collect();
    assert_eq!(selected.len(), 1, "exact contract for {name}: {records:?}");
    selected[0].clone()
}

const CALLABLES: &[(&str, &str, &str)] = &[
    ("rust", "fn echo(value: i32) -> i32 { value }", "i32"),
    ("python", "def echo(value: int) -> int:\n    return value\n", "int"),
    ("javascript", "function echo(value) { return value; }", ""),
    (
        "typescript",
        "function echo(value: number): number { return value; }",
        "number",
    ),
    (
        "tsx",
        "function echo(value: number): number { return value; }",
        "number",
    ),
    ("go", "package p\nfunc echo(value int) int { return value }", "int"),
    ("java", "class Box { int echo(int value) { return value; } }", "int"),
    ("csharp", "class Box { int echo(int value) { return value; } }", "int"),
    ("c", "int echo(int value) { return value; }", "int"),
    ("cpp", "int echo(int value) { return value; }", "int"),
];

#[test]
fn declared_parameters_and_results_use_real_occurrence_ids() {
    for &(language, source, ty) in CALLABLES {
        let result = inventory(source, language);
        let record = contract(&result, "echo");
        let callable = &result.callables["echo"];
        assert_eq!(
            record["site"]["occurrence"]["address_id"], callable.address_id,
            "{language}"
        );
        assert_eq!(record["language"], language);
        assert_eq!(record["shape"]["kind"], "callable");
        let parameters = record["shape"]["parameters"].as_array().unwrap();
        assert_eq!(parameters.len(), 1, "{language}: {record}");
        assert_eq!(parameters[0]["name"], "value", "{language}");
        if ty.is_empty() {
            assert_eq!(parameters[0]["declared_type"]["kind"], "unspecified");
            assert_eq!(record["shape"]["result"]["kind"], "unspecified");
        } else {
            for t in [&parameters[0]["declared_type"], &record["shape"]["result"]] {
                assert_eq!(t["kind"], "explicit", "{language}: {t}");
                assert_eq!(t["syntax"], ty, "{language}");
                let start = usize::try_from(t["span"]["start_byte"].as_u64().unwrap()).unwrap();
                let end = usize::try_from(t["span"]["end_byte"].as_u64().unwrap()).unwrap();
                assert_eq!(&source[start..end], ty);
            }
        }
    }
}

#[test]
fn object_fields_are_declared_members_not_locals_or_inherited_guesses() {
    let cases = [
        ("rust", "struct Box { value: i32 }", "i32"),
        (
            "python",
            "class Box:\n    value: int\n    def method(self):\n        local: str = 'x'\n",
            "int",
        ),
        ("javascript", "class Box { value = 1; method() { let local = 2; } }", ""),
        (
            "typescript",
            "class Box { value: number; method() { let local: string; } }",
            "number",
        ),
        ("tsx", "class Box { value: number; }", "number"),
        ("go", "package p\ntype Box struct { value int }\n", "int"),
        ("java", "class Box { int value; void method() { int local; } }", "int"),
        ("csharp", "class Box { int value; void method() { int local; } }", "int"),
        ("c", "struct Box { int value; };", "int"),
        ("cpp", "struct Box { int value; };", "int"),
    ];
    for (language, source, ty) in cases {
        let result = inventory(source, language);
        let record = contract(&result, "Box");
        assert_eq!(record["shape"]["kind"], "object", "{language}");
        let members = record["shape"]["members"].as_array().unwrap();
        assert!(!members.iter().any(|m| m["name"] == "local"), "{language}");
        let field = members.iter().find(|m| m["name"] == "value").expect("field");
        assert_eq!(field["kind"], "field");
        if !ty.is_empty() {
            assert_eq!(field["declared_type"]["syntax"], ty, "{language}");
        }
    }
}

#[test]
fn nested_unicode_crlf_contracts_survive_relocation_and_body_edits() {
    let base =
        "# é🍃\r\nclass Box:\r\n    @staticmethod\r\n    def echo(value: int) -> int:\r\n        return value\r\n";
    let a = inventory(base, "python");
    let b = inventory(&format!("\r\n{base}"), "python");
    let c = inventory(&base.replace("return value", "return 42"), "python");
    for next in [&b, &c] {
        assert_eq!(a.callables["echo"].address_id, next.callables["echo"].address_id);
    }
    assert_eq!(a.callables["echo"].structural_hash, b.callables["echo"].structural_hash);
    assert_ne!(a.callables["echo"].structural_hash, c.callables["echo"].structural_hash);
    let a_contract = contract(&a, "echo");
    let b_contract = contract(&b, "echo");
    assert_eq!(a_contract["shape"], contract(&c, "echo")["shape"]);
    assert_eq!(
        a_contract["site"]["span"]["end_byte"],
        contract(&a, "Box")["site"]["span"]["end_byte"]
    );
    assert_eq!(
        b_contract["site"]["span"]["start_byte"].as_u64(),
        a_contract["site"]["span"]["start_byte"].as_u64().map(|v| v + 2)
    );
}

#[test]
fn unsupported_patterns_remain_visible_and_malformed_files_fail() {
    let result = inventory("function echo({value}) { return value; }", "javascript");
    let record = contract(&result, "echo");
    assert!(record["limitations"].as_array().is_some_and(|v| !v.is_empty()));
    assert!(record["shape"]["parameters"][0]["name"].is_null());
    assert!(StructuralHasher::new()
        .compute_granular_hash_with_analysis("def echo(:", "python", None)
        .is_err());
}

#[test]
fn python_parameter_modes_and_written_defaults_are_distinct() {
    let source =
        "def echo(a: int = 1, /, b: str = '', *args: float, flag: bool = False, **kw: object) -> None:\n    pass\n";
    let result = inventory(source, "python");
    let record = contract(&result, "echo");
    let params = record["shape"]["parameters"].as_array().unwrap();
    assert_eq!(
        params.iter().map(|p| p["name"].as_str().unwrap()).collect::<Vec<_>>(),
        ["a", "b", "args", "flag", "kw"]
    );
    assert_eq!(
        params.iter().map(|p| p["kind"].as_str().unwrap()).collect::<Vec<_>>(),
        [
            "positional_only",
            "ordinary",
            "variadic",
            "keyword_only",
            "keyword_variadic"
        ]
    );
    assert_eq!(params[0]["default"]["syntax"], "1");
    let modified = inventory(&source.replace("= 1,", "= 2,"), "python");
    let a = &result.markers.as_ref().unwrap().code_contracts[0];
    let b = &modified.markers.as_ref().unwrap().code_contracts[0];
    assert_ne!(a.boundary_key(), b.boundary_key());
}

#[test]
fn compound_c_declarators_do_not_claim_plain_scalar_types() {
    let result = inventory("int *echo(int *value) { return value; }", "c");
    let record = contract(&result, "echo");
    assert_eq!(record["shape"]["parameters"][0]["name"], "value");
    assert_eq!(record["shape"]["parameters"][0]["declared_type"]["kind"], "unsupported");
    assert_eq!(record["shape"]["result"]["kind"], "unsupported");
}

#[test]
fn trailing_java_dimensions_are_not_misreported_as_scalar_types() {
    let result = inventory("class Box { int echo(int value[])[] { return value; } }", "java");
    let record = contract(&result, "echo");
    assert_eq!(record["shape"]["parameters"][0]["declared_type"]["kind"], "unsupported");
    assert_eq!(record["shape"]["result"]["kind"], "unsupported");
}

#[test]
fn typescript_optional_parameters_change_the_declared_boundary() {
    let before = inventory("function echo(value: number) { return value; }", "typescript");
    let after = inventory("function echo(value?: number) { return value; }", "typescript");
    assert_ne!(
        before.markers.unwrap().code_contracts[0].boundary_key(),
        after.markers.unwrap().code_contracts[0].boundary_key()
    );
}
