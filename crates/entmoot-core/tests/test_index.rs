//! Index emission and signature/body hash tests.

use entmoot_core::{build_file_index, StructuralHasher};

#[test]
fn sound_boundaries_retain_types_arity_visibility_and_exclude_bodies() {
    let hasher = StructuralHasher::new();
    for (language, baseline, type_change, arity_change, body_change) in [
        (
            "python",
            "def f(x:int)->int:\n    return x\n",
            "def f(x:int)->str:\n    return x\n",
            "def f(x:int,y:int)->int:\n    return x\n",
            "def f(x:int)->int:\n    return x+1\n",
        ),
        (
            "java",
            "class C { public int f(int x) { return x; } }",
            "class C { public long f(int x) { return x; } }",
            "class C { public int f(int x, int y) { return x; } }",
            "class C { public int f(int x) { return x + 1; } }",
        ),
        (
            "c",
            "int f(int x) { return x; }",
            "long f(int x) { return x; }",
            "int f(int x, int y) { return x; }",
            "int f(int x) { return x + 1; }",
        ),
        (
            "cpp",
            "int f(int x) { return x; }",
            "long f(int x) { return x; }",
            "int f(int x, int y) { return x; }",
            "int f(int x) { return x + 1; }",
        ),
        (
            "rust",
            "pub fn f(x:i32)->i32{x}",
            "pub fn f(x:i64)->i64{x}",
            "pub fn f(x:i32,y:i32)->i32{x}",
            "pub fn f(x:i32)->i32{x+1}",
        ),
        (
            "csharp",
            "class C { public int f(int x) { return x; } }",
            "class C { public long f(long x) { return x; } }",
            "class C { public int f(int x, int y) { return x; } }",
            "class C { public int f(int x) { return x + 1; } }",
        ),
        (
            "go",
            "package p\nfunc f(x int) int{return x}",
            "package p\nfunc f(x string) string{return x}",
            "package p\nfunc f(x,y int) int{return x}",
            "package p\nfunc f(x int) int{return x+1}",
        ),
        (
            "typescript",
            "function f(x:number):number{return x}",
            "function f(x:string):string{return x}",
            "function f(x:number,y:number):number{return x}",
            "function f(x:number):number{return x+1}",
        ),
        (
            "javascript",
            "function f(x=1){return x}",
            "function f(x='a'){return x}",
            "function f(x=1,y=2){return x}",
            "function f(x=1){return x+1}",
        ),
    ] {
        let hash = |source: &str| {
            hasher.compute_granular_hash(source, language, None).unwrap().callables["f"]
                .signature_hash
                .clone()
        };
        assert_ne!(hash(baseline), hash(type_change), "{language}/type or default");
        assert_ne!(hash(baseline), hash(arity_change), "{language}/arity");
        assert_eq!(hash(baseline), hash(body_change), "{language}/body");
        assert_ne!(hash(baseline), entmoot_contracts::blake3_hex(b""));
    }
    let public = hasher.compute_granular_hash("pub fn f() {}", "rust", None).unwrap();
    let private = hasher.compute_granular_hash("fn f() {}", "rust", None).unwrap();
    assert_ne!(
        public.callables["f"].signature_hash,
        private.callables["f"].signature_hash
    );
}

#[test]
fn address_id_differs_from_structural_hash() {
    let hasher = StructuralHasher::new();
    let code = "def foo(x):\n    return x\n";
    let result = hasher
        .compute_granular_hash_at(code, "python", Some("a.py"), Some("acme/app"), None)
        .unwrap();
    let c = &result.callables["foo"];
    assert_ne!(c.address_id, c.structural_hash);
    assert!(!c.address_id.starts_with("blake3:"));
    assert!(c.structural_hash.starts_with("blake3:"));
}

#[test]
fn signature_hash_stable_on_body_only_edit() {
    let hasher = StructuralHasher::new();
    let a = hasher
        .compute_granular_hash("def foo(x):\n    return x\n", "python", None)
        .unwrap();
    let b = hasher
        .compute_granular_hash("def foo(x):\n    return x + 1\n", "python", None)
        .unwrap();
    assert_eq!(a.callables["foo"].signature_hash, b.callables["foo"].signature_hash);
    assert_ne!(a.callables["foo"].structural_hash, b.callables["foo"].structural_hash);
}

#[test]
fn signature_hash_changes_on_param_count_change() {
    let hasher = StructuralHasher::new();
    let a = hasher
        .compute_granular_hash("def foo(x):\n    return x\n", "python", None)
        .unwrap();
    let b = hasher
        .compute_granular_hash("def foo(x, y):\n    return x\n", "python", None)
        .unwrap();
    assert_ne!(a.callables["foo"].signature_hash, b.callables["foo"].signature_hash);
}

#[test]
fn policy_id_on_granular_result() {
    let hasher = StructuralHasher::new();
    use entmoot_core::StructuralHashPolicy;
    let policy = StructuralHashPolicy::v1();
    let pid = policy.fingerprint();
    let result = hasher
        .compute_granular_hash("def f(): pass", "python", Some(&policy))
        .unwrap();
    assert_eq!(result.policy_id, pid);
}

#[test]
fn build_file_index_dedupes_definitions_from_hash_output() {
    let hasher = StructuralHasher::new();
    let same = r#"
def a():
    return 1
def b():
    return 1
"#;
    let result = hasher.compute_granular_hash(same, "python", None).unwrap();
    let index = build_file_index(&result);
    assert!(!index.policy_id.is_empty());
    assert_eq!(index.occurrences.len(), result.callables.len());
    assert!(
        index.definitions.len() <= index.occurrences.len(),
        "definitions deduped by structural_hash"
    );
}

#[test]
fn property_store_key_roundtrip_json() {
    use entmoot_contracts::PropertyStoreKey;
    let key = PropertyStoreKey {
        policy_id: entmoot_contracts::PolicyId::parse("a".repeat(64)).unwrap(),
        structural_hash: entmoot_contracts::StructuralHash::parse(format!("blake3:{}", "b".repeat(64))).unwrap(),
        property_schema_id: "taint.v1".into(),
    };
    let json = serde_json::to_string(&key).unwrap();
    let back: PropertyStoreKey = serde_json::from_str(&json).unwrap();
    assert_eq!(key, back);
}
