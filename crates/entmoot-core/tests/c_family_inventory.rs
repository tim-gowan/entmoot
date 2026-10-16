use entmoot_core::{StructuralHashPolicy, StructuralHasher};

#[test]
fn c_and_cpp_inventory_retains_structure_and_never_selects_compiler_targets() {
    let hasher = StructuralHasher::new();
    for (language, path) in [("c", "sample.c"), ("cpp", "sample.cpp")] {
        let before = "int target(int x) { return x + 1; } int caller(void) { return target(2); }";
        let result = hasher
            .compute_granular_hash_with_analysis(before, language, None)
            .unwrap();
        let target = result.callables.values().find(|c| c.name == "target").unwrap();
        let changed = hasher
            .compute_granular_hash_with_analysis(&before.replace("x + 1", "x + 3"), language, None)
            .unwrap();
        let renamed = hasher
            .compute_granular_hash_with_analysis(&before.replace("target", "renamed"), language, None)
            .unwrap();
        let cosmetic = hasher
            .compute_granular_hash_with_analysis(&before.replace("return x", "/* comment */ return  x"), language, None)
            .unwrap();
        assert_ne!(
            target.structural_hash,
            changed
                .callables
                .values()
                .find(|c| c.name == "target")
                .unwrap()
                .structural_hash
        );
        assert_eq!(
            target.structural_hash,
            renamed
                .callables
                .values()
                .find(|c| c.name == "renamed")
                .unwrap()
                .structural_hash
        );
        assert_eq!(
            target.structural_hash,
            cosmetic
                .callables
                .values()
                .find(|c| c.name == "target")
                .unwrap()
                .structural_hash
        );
        let record = entmoot_core::index::build_file_index(&result);
        let calls = entmoot_core::resolve::resolve_file_calls(path, &record);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].resolved_fqn.is_none(), "C/C++ needs compiler evidence");
        assert_eq!(calls[0].owner.as_ref().unwrap().name, "caller");
        let explicit = hasher
            .compute_granular_hash_with_analysis(before, language, Some(&StructuralHashPolicy::v1()))
            .unwrap();
        assert_eq!(explicit.file_hash, result.file_hash);
        assert_eq!(explicit.policy_id, result.policy_id);
        assert!(hasher.compute_granular_hash("int broken( {", language, None).is_err());
    }
}

#[test]
fn cpp_namespaces_methods_overloads_templates_and_lambda_owners_are_distinct() {
    let code = "namespace ns { struct A { int f(int x) { return x; } int f() { return 2; } }; template<class T> T id(T x) { return x; } int g() { auto h = [] { return id(1); }; return h(); } }";
    let result = StructuralHasher::new()
        .compute_granular_hash_with_analysis(code, "cpp", None)
        .unwrap();
    assert_eq!(result.callables.len(), 5);
    let markers = result.markers.unwrap();
    let nested = markers.call_sites.iter().find(|c| c.callee == "id").unwrap();
    assert_eq!(nested.owner.as_ref().unwrap().name, "<lambda>");
    let methods: Vec<_> = result.callables.values().filter(|c| c.name == "f").collect();
    assert_eq!(methods.len(), 2);
    assert_ne!(methods[0].address_id, methods[1].address_id);
    assert!(methods
        .iter()
        .all(|c| c.logic_address.contains("ns") && c.logic_address.contains("A")));
}

#[test]
fn syntactic_includes_are_callable_dependencies_and_directives_are_retained() {
    let hasher = StructuralHasher::new();
    let a = "#include \"one.h\"\nint f(void) { return 1; }\n";
    let b = a.replace("one.h", "two.h");
    let a = hasher.compute_granular_hash(a, "c", None).unwrap();
    let b = hasher.compute_granular_hash(&b, "c", None).unwrap();
    assert_ne!(
        a.callables.values().next().unwrap().structural_hash,
        b.callables.values().next().unwrap().structural_hash
    );
    assert_ne!(
        hasher
            .compute_hash("int f(){/* NOLINT(x) */return 1;}", "c", None)
            .unwrap(),
        hasher
            .compute_hash("int f(){/* NOLINT(y) */return 1;}", "c", None)
            .unwrap()
    );
}

#[test]
fn cpp_template_boundary_changes_do_not_collapse() {
    let hasher = StructuralHasher::new();
    let a = hasher
        .compute_granular_hash("template<class T> int f() { return 1; }", "cpp", None)
        .unwrap();
    let b = hasher
        .compute_granular_hash("template<class T, class U> int f() { return 1; }", "cpp", None)
        .unwrap();
    assert_ne!(
        a.callables.values().next().unwrap().structural_hash,
        b.callables.values().next().unwrap().structural_hash
    );
}

#[test]
fn macro_shaped_bodies_are_not_invented_callables() {
    let hasher = StructuralHasher::new();
    for source in [
        "TEST_F(Suite, Case) { target(); }",
        "namespace ns { TEST_F(Suite, Case) { target(); } }",
    ] {
        assert!(hasher.compute_granular_hash(source, "cpp", None).is_err());
    }
    let constructors = hasher
        .compute_granular_hash("struct A { A() {} ~A() {} };", "cpp", None)
        .unwrap();
    assert_eq!(constructors.callables.len(), 2);
}
