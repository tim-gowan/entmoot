//! Tests for structural hashing - Python-specific.
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

/// Python structural integration controls.
///
/// These tests validate the implementation for Python,
/// checking retained syntax without claiming semantic equivalence.
mod test_python_unit_tests {
    use super::*;

    #[test]
    fn test_ut_hash_1_1_python_whitespace() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    x=1\n    return x";
        let code2 = "def test():\n    x = 1\n    return x";
        let code3 = "def test():\n    x=1\n    return  x";
        let code4 = "def test():\n    x = 1\n    return x";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "python", None);
    }
}

/// Tests for canonical normalization behaviors specific to Python.
mod test_python_canonical_normalization {
    use super::*;

    #[test]
    fn test_numeric_literal_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let codes = vec![
            "def value():\n    return 10",
            "def value():\n    return 0xA",
            "def value():\n    return 0o12",
            "def value():\n    return 0b1010",
            "def value():\n    return 1_0",
        ];
        assert_hash_set_size_with_debug(&hasher, &codes, 5, "python", None);
    }

    // ============================================================================
    // Historical UT-Hash-1.2 corpus: retained identifier spelling
    // ============================================================================

    #[test]
    fn test_ut_hash_1_2_python_local_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    x = 1\n    y = x + 1\n    return y";
        let code2 = "def test():\n    a = 1\n    b = a + 1\n    return b";
        let code3 = "def test():\n    value = 1\n    result = value + 1\n    return result";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_2_python_multiple_args() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def add(x, y):\n    return x + y";
        let code2 = "def add(a, b):\n    return a + b";
        let code3 = "def add(first, second):\n    return first + second";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_2_python_nested_functions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def outer(x):
            def inner(y):
                return x + y
            return inner(1)
        "#;
        let code2 = r#"
        def outer(a):
            def inner(b):
                return a + b
            return inner(1)
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_1_2_python_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def func1(x):
            return x

        def func2(x):
            return x
        "#;
        let code2 = r#"
        def func1(a):
            return a

        def func2(a):
            return a
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_1_2_python_block_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def test():
            x = 1
            if True:
                y = 2
            return x
        "#;
        let code2 = r#"
        def test():
            a = 1
            if True:
                b = 2
            return a
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_1_2_python_class_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        class Test:
            def method(self, x):
                return x
        "#;
        let code2 = r#"
        class Test:
            def method(self, y):
                return y
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_1_2_python_control_flow_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x):\n    if x > 0:\n        return x\n    else:\n        return 0";
        let code2 = "def test(y):\n    if y > 0:\n        return y\n    else:\n        return 0";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    // ============================================================================
    // UT-Hash-1.3: Semantic Variance
    // ============================================================================

    #[test]
    fn test_ut_hash_1_3_python_loop_bound() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    for i in range(10):\n        pass";
        let code2 = "def test():\n    for i in range(20):\n        pass";
        let code3 = "def test():\n    for i in range(5):\n        pass";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_comparison_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x, y):\n    return x < y";
        let code2 = "def test(x, y):\n    return x > y";
        let code3 = "def test(x, y):\n    return x == y";
        let code4 = "def test(x, y):\n    return x <= y";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3, code4], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_computation_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(y):\n    x = y + 1\n    return x";
        let code2 = "def test(y):\n    x = y + 2\n    return x";
        let code3 = "def test(y):\n    x = y * 2\n    return x";
        let code4 = "def test(y):\n    x = y - 1\n    return x";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3, code4], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_try_except() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    try:\n        x = 1\n    except:\n        pass";
        let code2 = "def test():\n    try:\n        x = 1\n    except ValueError:\n        pass";
        let code3 = "def test():\n    try:\n        x = 1\n    finally:\n        pass";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_with() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    with open('file1') as f:\n        data = f.read()";
        let code2 = "def test():\n    with open('file2') as f:\n        data = f.read()";
        let code3 = "def test():\n    f = open('file1')\n    data = f.read()";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_raise() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    raise ValueError('error1')";
        let code2 = "def test():\n    raise ValueError('error2')";
        let code3 = "def test():\n    raise RuntimeError('error1')";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_assert() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x):\n    assert x > 0";
        let code2 = "def test(x):\n    assert x < 0";
        let code3 = "def test(x):\n    assert x == 0";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_break_continue() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    for i in range(10):\n        if i > 5:\n            break";
        let code2 = "def test():\n    for i in range(10):\n        if i > 5:\n            continue";
        let code3 = "def test():\n    for i in range(10):\n        if i > 5:\n            return";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    while x > 0:\n        x -= 1";
        let code2 = "def test():\n    while x < 0:\n        x += 1";
        let code3 = "def test():\n    for i in range(10):\n        pass";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_logical_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x, y):\n    return x and y";
        let code2 = "def test(x, y):\n    return x or y";
        let code3 = "def test(x):\n    return not x";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_membership_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x):\n    return x in [1, 2, 3]";
        let code2 = "def test(x):\n    return x not in [1, 2, 3]";
        let code3 = "def test(x):\n    return x is None";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_compound_assignment() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x):\n    x += 1";
        let code2 = "def test(x):\n    x -= 1";
        let code3 = "def test(x):\n    x *= 2";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    connection = sqlite3.connect('db1')\n    cursor = connection.cursor()";
        let code2 = "def test():\n    connection = sqlite3.connect('db2')\n    cursor = connection.cursor()";
        let code3 = "def test():\n    connection = sqlite3.connect('db1')\n    cursor = connection.execute()";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_boolean_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return True";
        let code2 = "def test():\n    return False";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_string_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return 'hello'";
        let code2 = "def test():\n    return 'world'";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_1_1_python_string_quote_equivalence() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return 'hello'";
        let code2 = "def test():\n    return \"hello\"";
        let code3 = "def test():\n    return '''hello'''";
        let code4 = "def test():\n    return \"\"\"hello\"\"\"";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3, code4], "python", None);
    }

    #[test]
    fn test_ut_hash_1_3_python_none_literal() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return None";
        let code2 = "def test():\n    return None";
        let code3 = "def test():\n    return 0";
        assert_hash_equal_with_debug(&hasher, code1, code2, "python", None); // Same None value
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // None vs integer
    }

    #[test]
    fn test_ut_hash_1_2_python_tuple_unpacking() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    x, y = (1, 2)";
        let code2 = "def test():\n    a, b = (1, 2)";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_1_3_python_tuple_unpacking_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    x, y = (1, 2)";
        let code2 = "def test():\n    x, y = (2, 3)";
        let code3 = "def test():\n    x = 1\n    y = 2";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None); // Different tuple values
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // Unpacking vs separate assignments
    }

    #[test]
    fn test_ut_hash_edge_python_list_comprehension() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return [x * 2 for x in range(10)]";
        let code2 = "def test():\n    return [y * 2 for y in range(10)]";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_dict_comprehension() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return {x: x * 2 for x in range(10)}";
        let code2 = "def test():\n    return {y: y * 2 for y in range(10)}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_set_comprehension() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return {x * 2 for x in range(10)}";
        let code2 = "def test():\n    return {y * 2 for y in range(10)}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_generator_expression() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return (x * 2 for x in range(10))";
        let code2 = "def test():\n    return (y * 2 for y in range(10))";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_lambda() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    f = lambda x: x + 1\n    return f(5)";
        let code2 = "def test():\n    f = lambda y: y + 1\n    return f(5)";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_lambda_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    f = lambda x: x + 1\n    return f(5)";
        let code2 = "def test():\n    f = lambda x: x + 2\n    return f(5)";
        let code3 = "def test():\n    def f(x):\n        return x + 1\n    return f(5)";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None); // Different operations
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // Lambda vs regular function
    }

    #[test]
    fn test_ut_hash_edge_python_decorator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "@decorator\ndef test(x):\n    return x + 1";
        let code2 = "@decorator\ndef test(y):\n    return y + 1";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_decorator_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "@decorator1\ndef test(x):\n    return x + 1";
        let code2 = "@decorator2\ndef test(x):\n    return x + 1";
        let code3 = "def test(x):\n    return x + 1";
        // Decorators affect logic, so different decorators should produce different hashes
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Different decorators should produce different hashes"),
        );
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code3,
            "python",
            Some("Function with decorator should differ from function without"),
        );
        assert_hash_not_equal_with_debug(
            &hasher,
            code2,
            code3,
            "python",
            Some("Function with decorator should differ from function without"),
        );
    }

    #[test]
    fn test_ut_hash_edge_python_async_await() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async def test(x):\n    result = await func(x)\n    return result";
        let code2 = "async def test(y):\n    result = await func(y)\n    return result";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_async_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async def test(x):\n    return x + 1";
        let code2 = "def test(x):\n    return x + 1";
        // Async vs sync should be different (structural difference)
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_ut_hash_edge_python_nested_comprehension() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return [[x * y for y in range(5)] for x in range(10)]";
        let code2 = "def test():\n    return [[a * b for b in range(5)] for a in range(10)]";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_lambda_in_comprehension() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    return [lambda x: x + y for y in range(10)]";
        let code2 = "def test():\n    return [lambda a: a + b for b in range(10)]";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        // Identifier spelling is retained.
    }

    #[test]
    fn test_ut_hash_edge_python_method_chaining() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    result = obj.method1().method2().method3()";
        let code2 = "def test():\n    result = obj.method1().method2().method3()";
        let code3 = "def test():\n    result = obj.method1().method2().method4()";
        assert_hash_equal_with_debug(&hasher, code1, code2, "python", None); // Same logic
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // Different method names
    }

    #[test]
    fn test_ut_hash_edge_python_f_strings() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x):\n    return f'Hello {x}'";
        let code2 = "def test(x):\n    return f'World {x}'";
        let code3 = "def test(x):\n    return 'Hello ' + str(x)";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None); // Different string literals
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // F-string vs concatenation
    }

    #[test]
    fn test_ut_hash_edge_python_walrus_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    if (x := 1) > 0:\n        return x";
        let code2 = "def test():\n    if (y := 1) > 0:\n        return y";
        let code3 = "def test():\n    x = 1\n    if x > 0:\n        return x";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None); // Identifier spelling is retained.
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // Walrus operator vs separate assignment
    }

    // ============================================================================
    // UT-Hash-1.5: Structural Completeness Verification
    // ============================================================================

    #[test]
    fn test_ut_hash_1_5_python_control_flow_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_if = "def test(x):\n    if x > 0:\n        return 1";
        let code_while = "def test(x):\n    while x > 0:\n        return 1";
        let code_for = "def test():\n    for i in range(10):\n        pass";
        let code_try = "def test():\n    try:\n        pass\n    except:\n        pass";
        let code_with = "def test():\n    with open('file') as f:\n        pass";
        let code_raise = "def test():\n    raise ValueError()";
        let code_assert = "def test(x):\n    assert x > 0";
        let codes = vec![
            code_if,
            code_while,
            code_for,
            code_try,
            code_with,
            code_raise,
            code_assert,
        ];
        assert_hash_set_size_with_debug(
            &hasher,
            &codes,
            7,
            "python",
            Some("All control flow statements should produce distinct hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_python_operator_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_add = "def test(x, y):\n    return x + y";
        let code_sub = "def test(x, y):\n    return x - y";
        let code_mul = "def test(x, y):\n    return x * y";
        let code_and = "def test(x, y):\n    return x and y";
        let code_or = "def test(x, y):\n    return x or y";
        let code_in = "def test(x):\n    return x in [1, 2, 3]";
        let code_is = "def test(x):\n    return x is None";
        let codes = vec![code_add, code_sub, code_mul, code_and, code_or, code_in, code_is];
        assert_hash_set_size_with_debug(
            &hasher,
            &codes,
            7,
            "python",
            Some("All operators should produce distinct hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_python_external_call_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test():\n    connection = sqlite3.connect('db')\n    cursor = connection.cursor()";
        let code2 = "def test():\n    conn = sqlite3.connect('db')\n    cur = conn.cursor()";
        let code3 = "def test():\n    connection = sqlite3.connect('db')\n    cursor = connection.execute()";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
    }

    #[test]
    fn test_ut_hash_1_5_python_scope_isolation_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_nested = r#"
        def outer(x):
            def inner(y):
                return x + y
            return inner(1)
        "#;
        let code_block = r#"
        def test():
            x = 1
            if True:
                y = 2
            return x
        "#;
        let code_class = r#"
        class Test:
            def method(self, x):
                return x
        "#;
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_nested, code_block, code_class],
            "python",
            Some("Different scope structures should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_edge_python_match_case() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "def test(x):\n    match x:\n        case 1:\n            return 1";
        let code2 =
            "def test(x):\n    match x:\n        case 1:\n            return 1\n        case 2:\n            return 2";
        let code3 = "def test(x):\n    if x == 1:\n        return 1";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None); // Different cases
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "python", None);
        // Match/case vs if
    }
}

/// Tests for scope analysis and global function preservation.
mod test_python_scope_analysis {
    use super::*;

    #[test]
    fn test_global_function_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def test(x):
            print(x)
        "#;
        let code2 = r#"
        def test(x):
            exit(x)
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_builtin_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def test(x):
            return len(x)
        "#;
        let code2 = r#"
        def test(x):
            return str(x)
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_import_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import os

        def test():
            os.path.exists('/tmp')
        "#;
        let code2 = r#"
        import sys

        def test():
            sys.exit(0)
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_import_from_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        from os import path

        def test():
            path.exists('/tmp')
        "#;
        let code2 = r#"
        from sys import path

        def test():
            path.append('/tmp')
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_external_module_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import requests

        def test():
            requests.get('http://example.com')
        "#;
        let code2 = r#"
        import urllib.request

        def test():
            urllib.request.urlopen('http://example.com')
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_attribute_method_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def test(obj):
            obj.method1()
        "#;
        let code2 = r#"
        def test(obj):
            obj.method2()
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_mixed_local_and_global() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def test(x):
            local_var = x + 1
            print(local_var)
            return local_var
        "#;
        let code2 = r#"
        def test(x):
            local_var = x + 1
            exit(local_var)
            return local_var
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_nested_function_closure() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def outer(x):
            def inner():
                print(x)
            return inner
        "#;
        let code2 = r#"
        def outer(x):
            def inner():
                exit(x)
            return inner
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_global_keyword_handling() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        x = 10
        def func():
            global x
            x = 20
        "#;
        let code2 = r#"
        x = 10
        def func():
            y = 20
        "#;
        // x in code1 should be preserved (global), y in code2 should be normalized
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_class_field_vs_local() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        class Test:
            def method(self):
                local_var = 1
                return self.field + local_var
        "#;
        let code2 = r#"
        class Test:
            def method(self):
                local_var = 1
                return self.other_field + local_var
        "#;
        // Different field names should produce different hashes
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_default_parameters_affect_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def connect(retries=5, timeout=30):
            return retries + timeout
        "#;
        let code2 = r#"
        def connect(retries=0, timeout=None):
            return retries + timeout
        "#;
        // Different default values should produce different hashes
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_same_default_parameters_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def connect(retries=5, timeout=30):
            return retries + timeout
        "#;
        let code2 = r#"
        def connect(retries=5, timeout=30):
            return retries + timeout
        "#;
        // Same default values should produce same hash
        assert_hash_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_positional_only_separator_affects_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_sep = r#"
        def f(a, /):
            return a
        "#;
        let code_without = r#"
        def f(a):
            return a
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code_with_sep,
            code_without,
            "python",
            Some("Positional-only '/' separator changes calling convention"),
        );
    }

    #[test]
    fn test_keyword_only_separator_affects_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_sep = r#"
        def f(*, a):
            return a
        "#;
        let code_without = r#"
        def f(a):
            return a
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code_with_sep,
            code_without,
            "python",
            Some("Keyword-only '*' separator changes calling convention"),
        );
    }

    #[test]
    fn test_nested_def_parameter_changes_are_structural() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Nested param name must not leak into outer allowed_locals (would rename global `collision`).
        let code1 = r#"
        def outer():
            def inner(collision):
                return collision
            return helper(collision)
        "#;
        let code2 = r#"
        def outer():
            def inner(other):
                return other
            return helper(collision)
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Nested parameter changes remain structural inputs"),
        );
    }

    #[test]
    fn test_args_kwargs_affect_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def process(*args, **kwargs):
            return len(args) + len(kwargs)
        "#;
        let code2 = r#"
        def process():
            return 0
        "#;
        // Function WITH *args/**kwargs should differ from function WITHOUT
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_function_with_args_vs_without() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def process(x, *args):
            return x + len(args)
        "#;
        let code2 = r#"
        def process(x):
            return x
        "#;
        // Function with *args should differ from function without
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_function_with_kwargs_vs_without() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def process(x, **kwargs):
            return x + len(kwargs)
        "#;
        let code2 = r#"
        def process(x):
            return x
        "#;
        // Function with **kwargs should differ from function without
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "python", None);
    }

    #[test]
    fn test_operand_order_preserved() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def test(x, y):
            return x + y
        "#;
        let code2 = r#"
        def test(x, y):
            return y + x
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Operand order should be preserved (x + y vs y + x should differ)"),
        );
    }
}

/// Tests for expected behavior that is not yet implemented.
///
/// These tests document the desired behavior for features that need to be implemented.
mod test_python_known_issues {
    use super::*;

    #[test]
    fn test_args_kwargs_parameter_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Test *args renaming
        let code1 = r#"
        def func(x, *args):
            return sum(args) + x
        "#;
        let code2 = r#"
        def func(x, *items):
            return sum(items) + x
        "#;
        // Parameter spelling is retained.
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Renaming *args to *items must remain distinct"),
        );
    }

    #[test]
    fn test_args_parameter_renaming() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def func(*args):
            return args
        "#;
        let code2 = r#"
        def func(*items):
            return items
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Renaming *args to *items must remain distinct"),
        );
    }

    #[test]
    fn test_kwargs_parameter_renaming() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def func(**kwargs):
            return kwargs
        "#;
        let code2 = r#"
        def func(**options):
            return options
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Renaming **kwargs to **options must remain distinct"),
        );
    }

    #[test]
    fn test_args_and_kwargs_parameter_renaming() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def func(x, *args, **kwargs):
            return x + sum(args) + len(kwargs)
        "#;
        let code2 = r#"
        def func(x, *items, **options):
            return x + sum(items) + len(options)
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Renaming both *args and **kwargs must remain distinct"),
        );
    }

    #[test]
    fn test_intermediate_variable_simplification() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Non-literal temporary elimination needs semantic analysis, not use counting.
        let code1 = r#"
        def calculate(x, y):
            result = x + y
            return result
        "#;
        // Preserve the temporary in clone identity under the literal-only policy.
        let code2 = r#"
        def calculate(x, y):
            return x + y
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Non-literal temporaries remain explicit"),
        );
    }

    #[test]
    fn test_intermediate_variable_multiple_uses_should_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Variable used multiple times - should NOT be inlined
        let code1 = r#"
        def calculate(x, y):
            result = x + y
            return result + result
        "#;
        // Direct expression - different logic
        let code2 = r#"
        def calculate(x, y):
            return x + y + x + y
        "#;
        // These should produce DIFFERENT hashes (different logic)
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Multi-use intermediate variable should NOT be inlined (different logic)"),
        );
    }

    #[test]
    fn test_intermediate_variable_nested_usage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Call initializers must NOT be inlined when used twice — would duplicate side effects.
        let code1 = r#"
        def process(data):
            value = data.get('key')
            return value if value else 0
        "#;
        let code2 = r#"
        def process(data):
            return data.get('key') if data.get('key') else 0
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Call inlining with multi-use would duplicate side effects (prefer miss)"),
        );
    }

    #[test]
    fn test_closure_captures_different_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def outer(x, y):
            return lambda: x
        "#;
        let code2 = r#"
        def outer(x, y):
            return lambda: y
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Closures capturing different parameters must produce different hashes"),
        );
    }

    #[test]
    fn test_closure_captures_different_locals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def outer():
            x = 1
            y = 2
            return lambda: x
        "#;
        let code2 = r#"
        def outer():
            x = 1
            y = 2
            return lambda: y
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Closures capturing different local variables must produce different hashes"),
        );
    }

    #[test]
    fn test_nested_closure_multiple_levels() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def a(x, z):
            def b(y):
                def c():
                    return z
                return c
            return b
        "#;
        let code2 = r#"
        def a(x, z):
            def b(y):
                def c():
                    return y
                return c
            return b
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "python",
            Some("Nested closures capturing from different levels must produce different hashes"),
        );
    }
}
