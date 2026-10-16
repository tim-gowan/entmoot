//! Tests for structural hashing - Go (Golang).
//!
//! Ported from tests/test_go.py (111 tests)
//!
//! Authored source controls for the retained-CST v1 policy (ADR-0024).
//! These are structural fixtures; compiler qualification is a separate lane.
#![cfg(feature = "full-tests")]

use entmoot_contracts::StructuralHashPolicy;
use entmoot_core::StructuralHasher;
#[path = "common/mod.rs"]
mod common;
use common::{
    assert_hash_equal_with_debug, assert_hash_not_equal_with_debug, assert_hash_set_size_with_debug,
    assert_hashes_equal_with_debug, assert_hashes_not_equal_with_debug,
};

/// Tests for StructuralHasher class - Go-specific.
mod test_go_structural_hasher {
    use super::*;

    #[test]
    fn test_hash_go_different_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func method(x int) int {
            result := x + 1
            return result
        }
        "#;
        let code2 = r#"
        package main

        func method(y int) int {
            output := y + 1
            return output
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }
}

/// Unit tests for StructuralHasher - Go tests.
mod test_go_unit_tests {
    use super::*;

    // ============================================================================
    // Historical UT-Hash-1.1 corpus: layout and retained syntax controls
    // ============================================================================

    #[test]
    fn test_ut_hash_1_1_go_comments() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { x := 1 }";
        let code2 = "// Comment\npackage main\n\nfunc test() { x := 1 }";
        let code3 = "package main\n\n/* Block comment */\nfunc test() { x := 1 }";
        let code4 = "package main\n\n// Line comment\nfunc test() { x := 1 }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_whitespace() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\nfunc test(){x:=1}";
        let code2 = "package main\n\nfunc test() {\n    x := 1\n}";
        let code3 = "package main\nfunc test() { x := 1 }";
        let code4 = "package main\n\nfunc test() {\n\tx := 1\n}";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_package_comments() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { x := 1 }";
        let code2 = "// Package comment\npackage main\n\nfunc test() { x := 1 }";
        let code3 = "/*\n * Package comment\n */\npackage main\n\nfunc test() { x := 1 }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_type_annotations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int) int { return x }";
        let code2 = "package main\n\nfunc test(x float64) float64 { return x }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_numeric_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_decimal = "package main\n\nfunc value() int { return 10 }";
        let code_hex = "package main\n\nfunc value() int { return 0xA }";
        let code_octal = "package main\n\nfunc value() int { return 0o12 }";
        let code_binary = "package main\n\nfunc value() int { return 0b1010 }";
        let code_underscored = "package main\n\nfunc value() int { return 1_0 }";
        let codes = vec![code_decimal, code_hex, code_octal, code_binary, code_underscored];
        assert_hash_set_size_with_debug(&hasher, &codes, 5, "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_string_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_interpreted = "package main\n\nfunc text() string { return \"Hello\\nWorld\" }";
        let code_raw = "package main\n\nfunc text() string { return `Hello\nWorld` }";
        assert_hash_not_equal_with_debug(&hasher, code_interpreted, code_raw, "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_block_wrapping() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_compact = "package main\n\nfunc maybe(x int) int { if x > 0 { return x }; return 0 }";
        let code_wrapped = "package main\n\nfunc maybe(x int) int { if x > 0 { { return x } }; return 0 }";
        assert_hash_not_equal_with_debug(&hasher, code_compact, code_wrapped, "go", None);
    }

    #[test]
    fn test_ut_hash_1_1_go_package_aliases() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_alias = "package main\n\nimport f \"fmt\"\n\nfunc log() {\n    f.Println(\"ok\")\n}\n";
        let code_direct = "package main\n\nimport \"fmt\"\n\nfunc log() {\n    fmt.Println(\"ok\")\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_alias, code_direct, "go", None);
    }

    // ============================================================================
    // Historical UT-Hash-1.2 corpus: retained identifier spelling
    // ============================================================================

    #[test]
    fn test_ut_hash_1_2_go_function_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc add(x int, y int) int { return x + y }";
        let code2 = "package main\n\nfunc add(a int, b int) int { return a + b }";
        let code3 = "package main\n\nfunc add(first int, second int) int { return first + second }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_local_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { x := 1; y := x + 1 }";
        let code2 = "package main\n\nfunc test() { a := 1; b := a + 1 }";
        let code3 = "package main\n\nfunc test() { value := 1; result := value + 1 }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_method_body_locals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\ntype T struct{}\n\nfunc (r T) method() int {\n    x := 1\n    y := x + 1\n    return y\n}\n";
        let code2 = "package main\n\ntype T struct{}\n\nfunc (r T) method() int {\n    a := 1\n    b := a + 1\n    return b\n}\n";
        let code3 = "package main\n\ntype T struct{}\n\nfunc (r T) method() int {\n    value := 1\n    result := value + 1\n    return result\n}\n";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_method_receiver() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\ntype T struct{}\n\nfunc (r T) method() { r.value = 1 }";
        let code2 = "package main\n\ntype T struct{}\n\nfunc (t T) method() { t.value = 1 }";
        let code3 = "package main\n\ntype T struct{}\n\nfunc (self T) method() { self.value = 1 }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_nested_scopes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int) { if x > 0 { y := x + 1 } }";
        let code2 = "package main\n\nfunc test(a int) { if a > 0 { b := a + 1 } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc method1(x int) int { return x }\nfunc method2(x int) int { return x }";
        let code2 = "package main\n\nfunc method1(a int) int { return a }\nfunc method2(a int) int { return a }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_blank_identifier() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { _, err := doSomething() }";
        let code2 = "package main\n\nfunc test() { _, e := doSomething() }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_pointer_receiver_renaming() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\ntype Counter struct{ value int }\nfunc (c *Counter) Inc() { c.value++ }\n";
        let code2 =
            "package main\n\ntype Counter struct{ value int }\nfunc (counter *Counter) Inc() { counter.value++ }\n";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_pointer_vs_value_receiver() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_pointer = "package main\n\ntype Counter struct{ value int }\nfunc (c *Counter) Touch() int { c.value++; return c.value }\n";
        let code_value = "package main\n\ntype Counter struct{ value int }\nfunc (c Counter) Touch() int { c.value++; return c.value }\n";
        assert_hash_not_equal_with_debug(&hasher, code_pointer, code_value, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_package_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nvar shared = 10\nfunc useShared() int { return shared }\n";
        let code2 = "package main\n\nvar shared = 10\nfunc useShared() int { local := shared; return local }\n";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_package_vs_local_semantics() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_pkg = "package main\n\nvar counter = 0\nfunc bump() int { counter++; return counter }\n";
        let code_local = "package main\n\nfunc bump() int { counter := 0; counter++; return counter }\n";
        assert_hash_not_equal_with_debug(&hasher, code_pkg, code_local, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_range_loop_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc walk(values []int) int {\n    total := 0\n    for i, v := range values {\n        total += i + v\n    }\n    return total\n}\n";
        let code2 = "package main\n\nfunc walk(values []int) int {\n    sum := 0\n    for idx, val := range values {\n        sum += idx + val\n    }\n    return sum\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_type_assert_bindings() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc cast(any interface{}) int {\n    str, ok := any.(string)\n    if ok { return len(str) }\n    return 0\n}\n";
        let code2 = "package main\n\nfunc cast(any interface{}) int {\n    name, ok := any.(string)\n    if ok { return len(name) }\n    return 0\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_named_return_values() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc compute(a, b int) (result int) {\n    result = a + b\n    return\n}\n";
        let code2 = "package main\n\nfunc compute(a, b int) (output int) {\n    output = a + b\n    return\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_variadic_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc sum(nums ...int) int {\n    total := 0\n    for _, v := range nums { total += v }\n    return total\n}\n";
        let code2 = "package main\n\nfunc sum(values ...int) int {\n    total := 0\n    for _, val := range values { total += val }\n    return total\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    // ============================================================================
    // UT-Hash-1.3: Semantic Variance
    // ============================================================================

    #[test]
    fn test_ut_hash_1_3_go_loop_bound() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { for i := 0; i < 10; i++ {} }";
        let code2 = "package main\n\nfunc test() { for i := 0; i < 20; i++ {} }";
        let code3 = "package main\n\nfunc test() { for i := 0; i < 5; i++ {} }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_comparison_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int, y int) bool { return x < y }";
        let code2 = "package main\n\nfunc test(x int, y int) bool { return x > y }";
        let code3 = "package main\n\nfunc test(x int, y int) bool { return x == y }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_computation_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(y int) int { x := y + 1; return x }";
        let code2 = "package main\n\nfunc test(y int) int { x := y + 2; return x }";
        let code3 = "package main\n\nfunc test(y int) int { x := y * 2; return x }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_switch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int) { switch x { case 1: default: } }";
        let code2 = "package main\n\nfunc test(x int) { switch x { case 1: case 2: default: } }";
        let code3 = "package main\n\nfunc test(x int) { if x == 1 {} }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_return_value() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() int { return 1 }";
        let code2 = "package main\n\nfunc test() int { return 2 }";
        let code3 = "package main\n\nfunc test() int { return 0 }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_break_continue() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { for i := 0; i < 10; i++ { if i > 5 { break } } }";
        let code2 = "package main\n\nfunc test() { for i := 0; i < 10; i++ { if i > 5 { continue } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_logical_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x bool, y bool) bool { return x && y }";
        let code2 = "package main\n\nfunc test(x bool, y bool) bool { return x || y }";
        let code3 = "package main\n\nfunc test(x bool) bool { return !x }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_arithmetic_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int, y int) int { return x + y }";
        let code2 = "package main\n\nfunc test(x int, y int) int { return x - y }";
        let code3 = "package main\n\nfunc test(x int, y int) int { return x * y }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_struct_literal_styles() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_keyed =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{X: 1, Y: 2} }\n";
        let code_positional =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{1, 2} }\n";
        assert_hash_not_equal_with_debug(&hasher, code_keyed, code_positional, "go", None);
    }

    #[test]
    fn test_ut_hash_1_2_go_struct_literal_field_order() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_keyed_order1 =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{X: 1, Y: 2} }\n";
        let code_keyed_order2 =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{Y: 2, X: 1} }\n";
        assert_hash_not_equal_with_debug(&hasher, code_keyed_order1, code_keyed_order2, "go", None);
        let code_positional_order1 =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{1, 2} }\n";
        let code_positional_order2 =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{2, 1} }\n";
        assert_hash_not_equal_with_debug(&hasher, code_positional_order1, code_positional_order2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_composite_literal_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_struct =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{X: 1, Y: 2} }\n";
        let code_struct_diff =
            "package main\n\ntype Point struct{X int; Y int}\nfunc p() Point { return Point{X: 2, Y: 1} }\n";
        let code_slice = "package main\n\nfunc p() []int { return []int{1, 2} }\n";
        assert_hashes_not_equal_with_debug(&hasher, &[code_struct, code_struct_diff, code_slice], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_interface_dispatch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_interface = "package main\n\ntype Runner interface{ Run() int }\ntype impl struct{}\nfunc (impl) Run() int { return 1 }\nfunc call() int {\n    var r Runner = impl{}\n    return r.Run()\n}\n";
        let code_direct = "package main\n\ntype impl struct{}\nfunc (impl) Run() int { return 1 }\nfunc call() int {\n    obj := impl{}\n    return obj.Run()\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_interface, code_direct, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_interface_method_sets() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_single_method = "package main\n\ntype Writer interface {\n    Write([]byte) (int, error)\n}\n";
        let code_two_methods =
            "package main\n\ntype Writer interface {\n    Write([]byte) (int, error)\n    Close() error\n}\n";
        let code_different_signature = "package main\n\ntype Writer interface {\n    Write([]byte) int\n}\n";
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_single_method, code_two_methods, code_different_signature],
            "go",
            None,
        );
    }

    #[test]
    fn test_ut_hash_1_2_go_generics_type_parameter() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_t = "package main\n\nfunc Map[T any](values []T) []T {\n    result := make([]T, len(values))\n    copy(result, values)\n    return result\n}\n";
        let code_u = "package main\n\nfunc Map[U any](values []U) []U {\n    result := make([]U, len(values))\n    copy(result, values)\n    return result\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_t, code_u, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_generics_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_comparable =
            "package main\n\nfunc Max[T comparable](a, b T) T {\n    if a > b { return a }\n    return b\n}\n";
        let code_ordered = "package main\n\nfunc Max[T ~int](a, b T) T {\n    _ = a\n    return b\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_comparable, code_ordered, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_generics_constraint_any_vs_comparable() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_any = "package main\n\nfunc Process[T any](items []T) []T {\n    return items\n}\n";
        let code_comparable = "package main\n\nfunc Process[T comparable](items []T) []T {\n    return items\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_any, code_comparable, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_generics_multiple_type_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_any_comparable =
            "package main\n\nfunc Map[T any, U comparable](items []T, keys []U) []T {\n    return items\n}\n";
        let code_comparable_any =
            "package main\n\nfunc Map[T comparable, U any](items []T, keys []U) []T {\n    return items\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_any_comparable, code_comparable_any, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_generics_constraint_combinations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_single_constraint =
            "package main\n\nfunc Max[T comparable](a, b T) T {\n    if a > b { return a }\n    return b\n}\n";
        let code_union_constraint =
            "package main\n\nfunc Max[T int | float64](a, b T) T {\n    if a > b { return a }\n    return b\n}\n";
        let code_approximation =
            "package main\n\nfunc Max[T ~int](a, b T) T {\n    if a > b { return a }\n    return b\n}\n";
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_single_constraint, code_union_constraint, code_approximation],
            "go",
            None,
        );
    }

    #[test]
    fn test_ut_hash_1_3_go_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nimport \"fmt\"\n\nfunc test() { fmt.Printf(\"test\") }";
        let code2 = "package main\n\nimport \"fmt\"\n\nfunc test() { fmt.Println(\"test\") }";
        let code3 = "package main\n\nimport \"os\"\n\nfunc test() { os.Open(\"file\") }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_defer() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { defer cleanup() }";
        let code2 = "package main\n\nfunc test() { cleanup() }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_goroutine() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { go doWork() }";
        let code2 = "package main\n\nfunc test() { doWork() }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_channel_operations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(ch chan int) { ch <- 1 }";
        let code2 = "package main\n\nfunc test(ch chan int) { <-ch }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_channel_direction_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_bidirectional = "package main\n\nfunc test(ch chan int) { ch <- 1 }";
        let code_receive_only = "package main\n\nfunc test(ch <-chan int) { <-ch }";
        let code_send_only = "package main\n\nfunc test(ch chan<- int) { ch <- 1 }";
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_bidirectional, code_receive_only, code_send_only],
            "go",
            None,
        );
    }

    #[test]
    fn test_ut_hash_1_3_go_type_assertions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc kind(any interface{}) int {\n    if _, ok := any.(string); ok { return 1 }\n    return 0\n}\n";
        let code2 = "package main\n\nfunc kind(any interface{}) int {\n    if _, ok := any.(int); ok { return 1 }\n    return 0\n}\n";
        let code3 = "package main\n\nfunc kind(any interface{}) int {\n    switch any.(type) {\n    case string:\n        return 1\n    default:\n        return 0\n    }\n}\n";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_type_assert_single_vs_double() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_single =
            "package main\n\nfunc length(any interface{}) int {\n    value := any.(string)\n    return len(value)\n}\n";
        let code_double = "package main\n\nfunc length(any interface{}) int {\n    if value, ok := any.(string); ok { return len(value) }\n    return 0\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_single, code_double, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_type_switch_multiple_matches() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_multi = "package main\n\nfunc describe(any interface{}) string {\n    switch v := any.(type) {\n    case string, error:\n        _ = v\n        return \"text\"\n    case int:\n        return \"int\"\n    default:\n        return \"unknown\"\n    }\n}\n";
        let code_simple = "package main\n\nfunc describe(any interface{}) string {\n    switch any.(type) {\n    case string:\n        return \"text\"\n    default:\n        return \"unknown\"\n    }\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_multi, code_simple, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_range_container_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_slice = "package main\n\nfunc sum(values []int) int {\n    total := 0\n    for _, v := range values { total += v }\n    return total\n}\n";
        let code_map = "package main\n\nfunc sum(values map[string]int) int {\n    total := 0\n    for _, v := range values { total += v }\n    return total\n}\n";
        let code_channel = "package main\n\nfunc drain(ch chan int) int {\n    total := 0\n    for v := range ch { total += v }\n    return total\n}\n";
        assert_hashes_not_equal_with_debug(&hasher, &[code_slice, code_map, code_channel], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_range_vs_index_loop() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_range = "package main\n\nfunc total(values []int) int {\n    sum := 0\n    for _, v := range values { sum += v }\n    return sum\n}\n";
        let code_index = "package main\n\nfunc total(values []int) int {\n    sum := 0\n    for i := 0; i < len(values); i++ { sum += values[i] }\n    return sum\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_range, code_index, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_select_statement() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_recv =
            "package main\n\nfunc mux(ch1, ch2 chan int) {\n    select {\n    case <-ch1:\n    case <-ch2:\n    }\n}\n";
        let code_send = "package main\n\nfunc mux(ch1, ch2 chan int) {\n    select {\n    case ch1 <- 1:\n    case ch2 <- 2:\n    }\n}\n";
        let code_default =
            "package main\n\nfunc mux(ch1, ch2 chan int) {\n    select {\n    case <-ch1:\n    default:\n    }\n}\n";
        assert_hashes_not_equal_with_debug(&hasher, &[code_recv, code_send, code_default], "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_type_switch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_cases = "package main\n\nfunc classify(any interface{}) int {\n    switch any.(type) {\n    case string:\n        return 1\n    case int:\n        return 2\n    default:\n        return 0\n    }\n}\n";
        let code_single_case = "package main\n\nfunc classify(any interface{}) int {\n    switch any.(type) {\n    case string:\n        return 1\n    default:\n        return 0\n    }\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_cases, code_single_case, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_switch_fallthrough() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_fallthrough = "package main\n\nfunc flag(x int) int {\n    switch x {\n    case 1:\n        fallthrough\n    case 2:\n        return 1\n    default:\n        return 0\n    }\n}\n";
        let code_break = "package main\n\nfunc flag(x int) int {\n    switch x {\n    case 1:\n        return 2\n    case 2:\n        return 1\n    default:\n        return 0\n    }\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_fallthrough, code_break, "go", None);
    }

    #[test]
    fn test_ut_hash_1_3_go_make_vs_new() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_make_len = "package main\n\nfunc build() []int {\n    return make([]int, 10)\n}\n";
        let code_make_other_len = "package main\n\nfunc build() []int {\n    return make([]int, 20)\n}\n";
        let code_new = "package main\n\nfunc build() *int {\n    return new(int)\n}\n";
        assert_hashes_not_equal_with_debug(&hasher, &[code_make_len, code_make_other_len, code_new], "go", None);
    }

    // ============================================================================
    // UT-Hash-1.4: Cross-Language Isolation
    // ============================================================================

    #[test]
    fn test_ut_hash_1_4_go_simple_loop() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let go_code = "package main\n\nfunc test() { for i := 0; i < 10; i++ {} }";
        let python_code = "def test():\n    for i in range(10):\n        pass";
        let java_code = "public class Test { public void method() { for (int i = 0; i < 10; i++) {} } }";
        let go_hash = hasher.compute_hash(go_code, "go", None).unwrap();
        let python_hash = hasher.compute_hash(python_code, "python", None).unwrap();
        let java_hash = hasher.compute_hash(java_code, "java", None).unwrap();
        assert_ne!(go_hash, python_hash, "Go and Python hashes should differ");
        assert_ne!(go_hash, java_hash, "Go and Java hashes should differ");
    }

    #[test]
    fn test_ut_hash_1_4_go_function_addition() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let go_code = "package main\n\nfunc add(x int, y int) int { return x + y }";
        let python_code = "def add(x, y):\n    return x + y";
        let java_code = "public class Test { public int add(int x, int y) { return x + y; } }";
        let go_hash = hasher.compute_hash(go_code, "go", None).unwrap();
        let python_hash = hasher.compute_hash(python_code, "python", None).unwrap();
        let java_hash = hasher.compute_hash(java_code, "java", None).unwrap();
        assert_ne!(go_hash, python_hash, "Go and Python hashes should differ");
        assert_ne!(go_hash, java_hash, "Go and Java hashes should differ");
    }

    #[test]
    fn test_ut_hash_1_4_go_conditional() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let go_code = "package main\n\nfunc test(x int) int { if x > 0 { return 1 } else { return 0 } }";
        let python_code = "def test(x):\n    if x > 0:\n        return 1\n    else:\n        return 0";
        let java_code =
            "public class Test { public int method(int x) { if (x > 0) { return 1; } else { return 0; } } }";
        let go_hash = hasher.compute_hash(go_code, "go", None).unwrap();
        let python_hash = hasher.compute_hash(python_code, "python", None).unwrap();
        let java_hash = hasher.compute_hash(java_code, "java", None).unwrap();
        assert_ne!(go_hash, python_hash, "Go and Python hashes should differ");
        assert_ne!(go_hash, java_hash, "Go and Java hashes should differ");
    }

    // ============================================================================
    // UT-Hash-1.5: Structural Completeness Verification
    // ============================================================================

    #[test]
    fn test_ut_hash_1_5_go_control_flow_if() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int) { if x > 0 {} }";
        let code2 = "package main\n\nfunc test(x int) { if x < 0 {} }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different conditions should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_control_flow_for() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { for i := 0; i < 10; i++ {} }";
        let code2 = "package main\n\nfunc test() { for i := 0; i < 20; i++ {} }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different loop bounds should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_control_flow_switch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int) { switch x { case 1: } }";
        let code2 = "package main\n\nfunc test(x int) { switch x { case 2: } }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different cases should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_control_flow_select() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(ch chan int) { select { case <-ch: } }";
        let code2 = "package main\n\nfunc test(ch chan int) { select { case ch <- 1: } }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different select cases should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_control_flow_return() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() int { return 1 }";
        let code2 = "package main\n\nfunc test() int { return 2 }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different return values should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_operators_arithmetic() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int, y int) int { return x + y }";
        let code2 = "package main\n\nfunc test(x int, y int) int { return x - y }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different operators should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_operators_comparison() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(x int, y int) bool { return x < y }";
        let code2 = "package main\n\nfunc test(x int, y int) bool { return x > y }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different operators should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nimport \"fmt\"\n\nfunc test() { fmt.Printf(\"test\") }";
        let code2 = "package main\n\nimport \"fmt\"\n\nfunc test() { fmt.Println(\"test\") }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different method calls should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc method1(x int) int { return x }\nfunc method2(y int) int { return y }";
        let code2 = "package main\n\nfunc method1(a int) int { return a }\nfunc method2(a int) int { return a }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Identifier spelling remains distinct in each scope"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_go_statement() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { go doWork() }";
        let code2 = "package main\n\nfunc test() { doWork() }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Go statement should produce different hash"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_defer_statement() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test() { defer cleanup() }";
        let code2 = "package main\n\nfunc test() { cleanup() }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Defer statement should produce different hash"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_channel_operations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "package main\n\nfunc test(ch chan int) { ch <- 1 }";
        let code2 = "package main\n\nfunc test(ch chan int) { x := <-ch }";
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Send vs receive should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_channel_direction_captured() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_receive_only = "package main\n\nfunc test(ch <-chan int) { <-ch }";
        let code_send_only = "package main\n\nfunc test(ch chan<- int) { ch <- 1 }";
        let code_bidirectional = "package main\n\nfunc test(ch chan int) { ch <- 1 }";
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_receive_only, code_send_only, code_bidirectional],
            "go",
            Some("Different channel direction types should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_go_type_assertion_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_string =
            "package main\n\nfunc detect(any interface{}) bool {\n    _, ok := any.(string)\n    return ok\n}\n";
        let code_int =
            "package main\n\nfunc detect(any interface{}) bool {\n    _, ok := any.(int)\n    return ok\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_string, code_int, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_type_switch_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_two_cases = "package main\n\nfunc classify(any interface{}) int {\n    switch any.(type) {\n    case string:\n        return 1\n    case int:\n        return 2\n    default:\n        return 0\n    }\n}\n";
        let code_single_case = "package main\n\nfunc classify(any interface{}) int {\n    switch any.(type) {\n    case string:\n        return 1\n    default:\n        return 0\n    }\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_two_cases, code_single_case, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_select_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_receive = "package main\n\nfunc wait(ch chan int) {\n    select {\n    case <-ch:\n    }\n}\n";
        let code_send = "package main\n\nfunc wait(ch chan int) {\n    select {\n    case ch <- 1:\n    }\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_receive, code_send, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_range_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_slice = "package main\n\nfunc sum(values []int) int {\n    s := 0\n    for _, v := range values { s += v }\n    return s\n}\n";
        let code_map = "package main\n\nfunc sum(values map[int]int) int {\n    s := 0\n    for _, v := range values { s += v }\n    return s\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_slice, code_map, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_composite_literal_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_struct = "package main\n\ntype data struct { X int }\nfunc build() data { return data{X: 1} }\n";
        let code_slice = "package main\n\nfunc build() []int { return []int{1} }\n";
        assert_hash_not_equal_with_debug(&hasher, code_struct, code_slice, "go", None);
    }

    #[test]
    fn test_ut_hash_1_5_go_side_effect_imports() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_blank_import = "package main\n\nimport _ \"image/png\"\n\nfunc test() int {\n    return 42\n}\n";
        let code_without_import = "package main\n\nfunc test() int {\n    return 42\n}\n";
        let code_different_blank_import =
            "package main\n\nimport _ \"image/jpeg\"\n\nfunc test() int {\n    return 42\n}\n";
        assert_hash_not_equal_with_debug(
            &hasher,
            code_with_blank_import,
            code_without_import,
            "go",
            Some("Blank import should affect hash"),
        );
        assert_hash_not_equal_with_debug(
            &hasher,
            code_with_blank_import,
            code_different_blank_import,
            "go",
            Some("Different blank imports should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_go_comprehensive_invariance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let baseline = "package main\n\nimport \"fmt\"\n\nfunc compute(x int, y int) int {\n    sum := x + y\n    if sum > 10 {\n        msg := \"value\"\n        fmt.Println(msg)\n        return len(msg)\n    }\n    return sum\n}\n";
        let with_comments = "package main\n\nimport \"fmt\"\n\n// Comment about compute\nfunc compute(first int, second int) int {\n    // addition\n    total := first + second\n    if total > 10 {\n        msg := \"value\"\n        fmt.Println(msg)\n        return len(msg)\n    }\n    return total\n}\n";
        let with_alias = "package main\n\nimport printer \"fmt\"\n\nfunc compute(a int, b int) int {\n    result := a + b\n    if result > 10 {\n        text := \"value\"\n        printer.Println(text)\n        return len(text)\n    }\n    return result\n}\n";
        let compact_whitespace = "package main\nimport \"fmt\"\nfunc compute(x int,y int)int{sum:=x+y;if sum>10{msg:=\"value\";fmt.Println(msg);return len(msg)};return sum}";
        let with_block_wrapper = "package main\n\nimport \"fmt\"\n\nfunc compute(x int, y int) int {\n    sum := x + y\n    if sum > 10 {\n        {\n            msg := \"value\"\n            fmt.Println(msg)\n            return len(msg)\n        }\n    }\n    return sum\n}\n";
        let codes = vec![
            baseline,
            with_comments,
            with_alias,
            compact_whitespace,
            with_block_wrapper,
        ];
        assert_hash_set_size_with_debug(&hasher, &codes, 5, "go", None);
    }

    #[test]
    fn test_ut_hash_go_expression_normalization_single_use() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_var = "package main\n\nfunc length() int {\n    value := \"text\"\n    return len(value)\n}\n";
        let code_inline = "package main\n\nfunc length() int {\n    return len(\"text\")\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_var, code_inline, "go", None);
    }

    #[test]
    fn test_ut_hash_go_expression_normalization_multi_use() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_multi =
            "package main\n\nfunc dup() int {\n    word := \"text\"\n    return len(word) + len(word)\n}\n";
        let hash = hasher.compute_hash(code_multi, "go", None).unwrap();
        assert!(hash.starts_with("blake3:"), "Should produce valid hash");
    }

    #[test]
    fn test_ut_hash_go_expression_normalization_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_shadow = "package main\n\nfunc shadow() int {\n    value := \"outer\"\n    if true {\n        value := \"inner\"\n        return len(value)\n    }\n    return len(value)\n}\n";
        let hash = hasher.compute_hash(code_shadow, "go", None).unwrap();
        assert!(hash.starts_with("blake3:"), "Should produce valid hash");
    }

    #[test]
    fn test_ut_hash_go_expression_normalization_side_effects() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_call = "package main\n\nfunc eval() int {\n    value := builder()\n    return len(value)\n}\n\nfunc builder() string { return \"x\" }\n";
        let code_literal = "package main\n\nfunc eval() int {\n    value := \"x\"\n    return len(value)\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_call, code_literal, "go", None);
    }

    #[test]
    fn test_ut_hash_go_expression_normalization_selector_literal() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_var = "package main\n\nvar config = struct{ Default int }{ Default: 5 }\nfunc timeout() int {\n    value := config.Default\n    return value\n}\n";
        let code_inline = "package main\n\nvar config = struct{ Default int }{ Default: 5 }\nfunc timeout() int {\n    return config.Default\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_var, code_inline, "go", None);
    }

    #[test]
    fn test_ut_hash_go_expression_normalization_selector_call() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_call = "package main\n\nfunc next() int { return 1 }\nvar config = struct{ Default func() int }{ Default: next }\nfunc timeout() int {\n    value := config.Default()\n    return value\n}\n";
        let code_inline = "package main\n\nfunc next() int { return 1 }\nvar config = struct{ Default func() int }{ Default: next }\nfunc timeout() int {\n    return config.Default()\n}\n";
        assert_hash_not_equal_with_debug(&hasher, code_call, code_inline, "go", None);
    }
}

/// Tests for scope analysis and global function preservation in Go.
mod test_go_scope_analysis {
    use super::*;

    #[test]
    fn test_package_function_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        import "fmt"
        func test(x int) {
            fmt.Println(x)
        }
        "#;
        let code2 = r#"
        package main
        import "fmt"
        func test(x int) {
            fmt.Printf("%d", x)
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_builtin_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        func test(arr []int) int {
            return len(arr)
        }
        "#;
        let code2 = r#"
        package main
        func test(arr []int) int {
            return cap(arr)
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_import_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        import "fmt"
        func test() {
            fmt.Println("test")
        }
        "#;
        let code2 = r#"
        package main
        import "os"
        func test() {
            os.Exit(0)
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_import_alias_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        import f "fmt"
        func test() {
            f.Println("test")
        }
        "#;
        let code2 = r#"
        package main
        import o "os"
        func test() {
            o.Exit(0)
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_external_package_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        import "fmt"
        func test() {
            fmt.Println("test")
        }
        "#;
        let code2 = r#"
        package main
        import "os"
        func test() {
            os.Exit(0)
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_package_qualified_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        import "strings"
        func test(s string) bool {
            return strings.HasPrefix(s, "prefix")
        }
        "#;
        let code2 = r#"
        package main
        import "strings"
        func test(s string) bool {
            return strings.HasSuffix(s, "suffix")
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_attribute_method_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        type Test struct {}
        func (t *Test) Method1() {}
        func test(t *Test) {
            t.Method1()
        }
        "#;
        let code2 = r#"
        package main
        type Test struct {}
        func (t *Test) Method2() {}
        func test(t *Test) {
            t.Method2()
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_mixed_local_and_global() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        import "fmt"
        func test(x int) {
            localVar := x + 1
            fmt.Println(localVar)
        }
        "#;
        let code2 = r#"
        package main
        import "fmt"
        func test(x int) {
            localVar := x + 1
            fmt.Printf("%d", localVar)
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_interface_method_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main
        type Writer interface {
            Write1([]byte) (int, error)
        }
        func test(w Writer) {
            w.Write1([]byte("test"))
        }
        "#;
        let code2 = r#"
        package main
        type Writer interface {
            Write2([]byte) (int, error)
        }
        func test(w Writer) {
            w.Write2([]byte("test"))
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_field_names_are_logic() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        type Config struct {
            Timeout int
            Retries int
        }

        func getConfig() Config {
            return Config{Timeout: 100}
        }
        "#;
        let code2 = r#"
        package main

        type Config struct {
            Timeout int
            Retries int
        }

        func getConfig() Config {
            return Config{Retries: 100}
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }
}

/// Tests for Go concurrency primitives (go and defer statements).
mod test_go_concurrency_primitives {
    use super::*;

    #[test]
    fn test_go_statement_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func test() {
            go func() {
                println("goroutine")
            }()
        }
        "#;
        let code2 = r#"
        package main

        func test() {
            go func() {
                println("goroutine")
            }()
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_defer_statement_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func test() {
            defer cleanup()
        }
        "#;
        let code2 = r#"
        package main

        func test() {
            defer cleanup()
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "go", None);
    }

    #[test]
    fn test_go_vs_defer_different_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func test() {
            go func() {
                println("async")
            }()
        }
        "#;
        let code2 = r#"
        package main

        func test() {
            defer func() {
                println("async")
            }()
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "go", None);
    }
}

/// Tests for qualified type extraction in Go (spec §10.2).
mod test_go_qualified_type_extraction {
    use super::*;

    #[test]
    fn test_go_qualified_interface_type_parameter() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func Handle(w storage.Reader) {
            w.Read(nil)
        }
        "#;
        let code2 = r#"
        package main

        func Handle(w storage.ReadWriter) {
            w.Read(nil)
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different qualified interface types should produce different hashes"),
        );
    }

    #[test]
    fn test_go_qualified_interface_type_parameter_io() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        import "io"

        func Process(r io.Reader) {
            r.Read(nil)
        }
        "#;
        let code2 = r#"
        package main

        import "io"

        func Process(w io.Writer) {
            w.Write(nil)
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different io interface types should produce different hashes"),
        );
    }

    #[test]
    fn test_go_qualified_type_same_interface_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func Handle(r storage.Reader) {
            r.Read(nil)
        }
        "#;
        let code2 = r#"
        package main

        func Handle(reader storage.Reader) {
            reader.Read(nil)
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Same interface type with different parameter names must remain distinct"),
        );
    }

    #[test]
    fn test_go_qualified_return_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func GetReader() storage.Reader {
            return nil
        }
        "#;
        let code2 = r#"
        package main

        func GetReader() storage.ReadWriter {
            return nil
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different qualified return types should produce different hashes"),
        );
    }

    #[test]
    fn test_go_qualified_return_type_io() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        import "io"

        func CreateReader() io.Reader {
            return nil
        }
        "#;
        let code2 = r#"
        package main

        import "io"

        func CreateReader() io.Writer {
            return nil
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Different io return types should produce different hashes"),
        );
    }
}

/// Tests for type preservation in Go (spec §2.2, §6.2).
mod test_go_type_preservation {
    use super::*;

    #[test]
    fn test_go_interface_type_vs_primitive() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func Process(r io.Reader) {
            r.Read(nil)
        }
        "#;
        let code2 = r#"
        package main

        func Process(id int) {
            _ = id
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Interface type vs primitive type should produce different hashes"),
        );
    }

    #[test]
    fn test_go_interface_type_vs_struct() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        type Handler struct{}

        func Process(r io.Reader) {
            r.Read(nil)
        }
        "#;
        let code2 = r#"
        package main

        type Handler struct{}

        func Process(h Handler) {
            _ = h
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Interface type vs struct type should produce different hashes"),
        );
    }

    #[test]
    fn test_go_any_vs_interface_brace_different_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func test(x any) {
            _ = x
        }
        "#;
        let code2 = r#"
        package main

        func test(x interface{}) {
            _ = x
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("any vs interface{} should produce different hashes for soundness"),
        );
    }

    #[test]
    fn test_go_operand_order_preserved() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func add(x int, y int) int {
            return x + y
        }
        "#;
        let code2 = r#"
        package main

        func add(x int, y int) int {
            return y + x
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Operand order should be preserved (x + y vs y + x should differ)"),
        );
    }

    #[test]
    fn test_closure_captures_different_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func outer(x int, y int) func() int {
            return func() int {
                return x
            }
        }
        "#;
        let code2 = r#"
        package main

        func outer(x int, y int) func() int {
            return func() int {
                return y
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Closures capturing different parameters must produce different hashes"),
        );
    }

    #[test]
    fn test_closure_captures_different_locals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func outer() func() int {
            x := 1
            y := 2
            return func() int {
                return x
            }
        }
        "#;
        let code2 = r#"
        package main

        func outer() func() int {
            x := 1
            y := 2
            return func() int {
                return y
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Closures capturing different local variables must produce different hashes"),
        );
    }

    #[test]
    fn test_nested_closure_multiple_levels() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func a(x int, z int) func(int) func() int {
            return func(y int) func() int {
                return func() int {
                    return z
                }
            }
        }
        "#;
        let code2 = r#"
        package main

        func a(x int, z int) func(int) func() int {
            return func(y int) func() int {
                return func() int {
                    return y
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "go",
            Some("Nested closures capturing from different levels must produce different hashes"),
        );
    }
}

/// Default and explicit v1 retain the same syntax inputs.
mod test_go_policy_entry_points {
    use super::*;

    #[test]
    fn test_loop_variable_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func calculateTotal(prices []int) int {
            total := 0
            for _, price := range prices {
                total = total + price
            }
            return total
        }
        "#;
        let code2 = r#"
        package main

        func calculateTotal(costs []int) int {
            sum := 0
            for _, cost := range costs {
                sum = sum + cost
            }
            return sum
        }
        "#;
        let policy_explicit = StructuralHashPolicy::v1();
        let policy_default = StructuralHashPolicy::default();
        let hash1_explicit = hasher.compute_hash(code1, "go", Some(&policy_explicit)).unwrap();
        let hash2_explicit = hasher.compute_hash(code2, "go", Some(&policy_explicit)).unwrap();
        assert_ne!(
            hash1_explicit, hash2_explicit,
            "Explicit v1 retains range variable spelling"
        );
        let hash1_default = hasher.compute_hash(code1, "go", Some(&policy_default)).unwrap();
        let hash2_default = hasher.compute_hash(code2, "go", Some(&policy_default)).unwrap();
        assert_ne!(
            hash1_default, hash2_default,
            "With the default policy, different loop variable names should produce different hashes"
        );
    }

    #[test]
    fn test_type_syntax_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        package main

        func Process(name string) {
            _ = name
        }
        "#;
        let code2 = r#"
        package main

        func Process(id int) {
            _ = id
        }
        "#;
        let policy_explicit = StructuralHashPolicy::v1();
        let policy_default = StructuralHashPolicy::default();
        let hash1_explicit = hasher.compute_hash(code1, "go", Some(&policy_explicit)).unwrap();
        let hash2_explicit = hasher.compute_hash(code2, "go", Some(&policy_explicit)).unwrap();
        assert_ne!(hash1_explicit, hash2_explicit, "Explicit v1 retains type syntax");
        let hash1_default = hasher.compute_hash(code1, "go", Some(&policy_default)).unwrap();
        let hash2_default = hasher.compute_hash(code2, "go", Some(&policy_default)).unwrap();
        assert_ne!(hash1_default, hash2_default, "Default policy retains type syntax");
        assert_eq!(hash1_explicit, hash1_default, "Default and explicit v1 agree");
        assert_eq!(hash2_explicit, hash2_default, "Default and explicit v1 agree");
    }
}
