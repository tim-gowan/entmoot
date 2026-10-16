//! Tests for structural hashing - JavaScript-specific.
//!
//! Ported from tests/test_javascript.py (98 tests)
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

/// Tests for StructuralHasher class - JavaScript-specific.
mod test_javascript_structural_hasher {
    use super::*;

    #[test]
    fn test_hash_javascript_different_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x) {
            const result = x + 1;
            return result;
        }
        "#;
        let code2 = r#"
        function test(y) {
            const output = y + 1;
            return output;
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }
}

/// Unit tests for StructuralHasher - JavaScript tests.
mod test_javascript_unit_tests {
    use super::*;

    fn assert_typed_syntax_requires_typescript(hasher: &StructuralHasher, plain: &str, typed: &[&str]) {
        assert!(hasher.compute_hash(plain, "javascript", None).is_ok());
        for source in typed {
            assert!(matches!(
                hasher.compute_granular_hash_with_analysis(source, "javascript", None),
                Err(entmoot_core::CoreError::Parse(_))
            ));
            // Grammar acceptance only: unresolved Flow names are not TS semantic evidence.
            assert!(hasher.compute_hash(source, "typescript", None).is_ok());
        }
    }

    // ============================================================================
    // Historical UT-Hash-1.1 corpus: layout and retained syntax controls
    // ============================================================================

    #[test]
    fn test_ut_hash_1_1_javascript_comments() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x = 1; return x; }";
        let code2 = "// Comment\nfunction test() { let x = 1; return x; }";
        let code3 = "function test() { /* Block comment */ let x = 1; return x; }";
        let code4 = "/**\n * JSDoc comment\n */\nfunction test() { let x = 1; return x; }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_whitespace() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x=1; return x; }";
        let code2 = "function test() { let x = 1; return x; }";
        let code3 = "function test() { let x=1; return  x; }";
        let code4 = "function test() { let x = 1; return x; }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_jsdoc() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x = 1; return x; }";
        let code2 = "/**\n * This is a JSDoc comment.\n * @param {number} x - The parameter\n * @returns {number}\n */\nfunction test() { let x = 1; return x; }";
        let code3 = "/**\n * Different JSDoc with more details.\n * @param {number} x\n */\nfunction test() { let x = 1; return x; }";
        let code4 = "/**\n * Another JSDoc comment.\n */\nfunction test() { let x = 1; return x; }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_type_annotations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return x + 1; }";
        let code2 = "function test(x: number): number { return x + 1; }";
        let code3 = "function test(x: string): string { return x + 1; }";
        let code4 = "function test(x: any): any { return x + 1; }";
        assert_typed_syntax_requires_typescript(&hasher, code1, &[code2, code3, code4]);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_flow_annotations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return x + 1; }";
        let code2 = "function test(x: number): number { return x + 1; }";
        let code3 = "function test(x: string): string { return x + 1; }";
        let code4 = "function test(x: any): any { return x + 1; }";
        assert_typed_syntax_requires_typescript(&hasher, code1, &[code2, code3, code4]);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_flow_import_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return null; }";
        let code2 = "import type {Element} from './types';\nfunction test() { return null; }";
        let code3 = "import type {Component, Element} from './types';\nfunction test() { return null; }";
        assert_typed_syntax_requires_typescript(&hasher, code1, &[code2, code3]);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_flow_export_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return null; }";
        let code2 = "export type State = { count: number };\nfunction test() { return null; }";
        let code3 = "export type State = { count: number, name: string };\nfunction test() { return null; }";
        assert_typed_syntax_requires_typescript(&hasher, code1, &[code2, code3]);
    }

    #[test]
    fn test_ut_hash_1_1_javascript_flow_utility_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(items) { return items.length; }";
        let code2 = "function test(items: Array<number>) { return items.length; }";
        let code3 = "function test(items: $ReadOnlyArray<number>) { return items.length; }";
        assert_typed_syntax_requires_typescript(&hasher, code1, &[code2, code3]);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_flow_logical_changes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { return x + 1; }";
        let code2 = "function test(x: number, y: number): number { return x + y; }";
        let code3 = "function test(x: number): number { if (x > 0) return x; return 0; }";
        let code4 = "function test(x: number): number { return x > 0 ? x : 0; }";
        let code5 = "function test(x: number): number { return x + 1; }";
        let code6 = "function test(x: number): number { return x * 2; }";
        for source in [code1, code2, code3, code4, code5, code6] {
            assert!(matches!(
                hasher.compute_hash(source, "javascript", None),
                Err(entmoot_core::CoreError::Parse(_))
            ));
        }
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        let hash4 = hasher.compute_hash(code4, "typescript", None).unwrap();
        let hash5 = hasher.compute_hash(code5, "typescript", None).unwrap();
        let hash6 = hasher.compute_hash(code6, "typescript", None).unwrap();
        assert_ne!(
            hash1, hash2,
            "Different parameter counts should produce different hashes"
        );
        assert_ne!(hash3, hash4, "Different control flow should produce different hashes");
        assert_ne!(hash5, hash6, "Different expressions should produce different hashes");
    }

    #[test]
    fn test_ut_hash_edge_javascript_flow_complex_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return x || 0; }";
        let code2 = "function test(x: number | null) { return x || 0; }";
        let code3 = "function test(x: number | string | null) { return x || 0; }";
        assert_typed_syntax_requires_typescript(&hasher, code1, &[code2, code3]);
    }

    #[test]
    fn test_ut_hash_edge_javascript_flow_has_error_flag() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        import type {Element} from './types';
        export type State = { count: number };
        function test(x: number, items: $ReadOnlyArray<number>): number {
            return items.length + x;
        }
        "#;
        assert!(matches!(
            hasher.compute_hash(code, "javascript", None),
            Err(entmoot_core::CoreError::Parse(_))
        ));
    }

    #[test]
    fn test_javascript_visit_error_handles_error_nodes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x) {
            return x + 1;
        }
        "#;
        let code2 = r#"
        function test(x) {
            // Some code that might trigger ERROR nodes
            return x + 1;
        }
        "#;
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert!(hash1.starts_with("blake3:"), "Should produce valid hash");
        assert!(hash2.starts_with("blake3:"), "Should produce valid hash");
    }

    // ============================================================================
    // Historical UT-Hash-1.2 corpus: retained identifier spelling
    // ============================================================================

    #[test]
    fn test_ut_hash_1_2_javascript_function_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return x; }";
        let code2 = "function test(y) { return y; }";
        let code3 = "function test(param) { return param; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_local_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x = 1; let y = x + 1; return y; }";
        let code2 = "function test() { let a = 1; let b = a + 1; return b; }";
        let code3 = "function test() { const value = 1; const result = value + 1; return result; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_nested_scopes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { if (x > 0) { let y = x + 1; } }";
        let code2 = "function test(a) { if (a > 0) { let b = a + 1; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_multiple_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function add(x, y) { return x + y; }";
        let code2 = "function add(a, b) { return a + b; }";
        let code3 = "function add(first, second) { return first + second; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function func1(x) { return x; } function func2(x) { return x; }";
        let code2 = "function func1(a) { return a; } function func2(a) { return a; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_block_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x = 1; if (true) { let y = 2; } return x; }";
        let code2 = "function test() { let a = 1; if (true) { let b = 2; } return a; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_class_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "class Test { method(x) { return x; } }";
        let code2 = "class Test { method(y) { return y; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_control_flow_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { if (x > 0) { return x; } else { return 0; } }";
        let code2 = "function test(y) { if (y > 0) { return y; } else { return 0; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let str = \"test\"; let len = str.length; }";
        let code2 = "function test() { let s = \"test\"; let l = s.length; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_2_javascript_destructuring() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { const [x, y] = [1, 2]; }";
        let code2 = "function test() { const [a, b] = [1, 2]; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
    }

    // ============================================================================
    // UT-Hash-1.3: Semantic Variance
    // ============================================================================

    #[test]
    fn test_ut_hash_1_3_javascript_loop_bound() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { for (let i = 0; i < 10; i++) {} }";
        let code2 = "function test() { for (let i = 0; i < 20; i++) {} }";
        let code3 = "function test() { for (let i = 0; i < 5; i++) {} }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_comparison_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x, y) { return x < y; }";
        let code2 = "function test(x, y) { return x > y; }";
        let code3 = "function test(x, y) { return x === y; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_computation_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(y) { let x = y + 1; return x; }";
        let code2 = "function test(y) { let x = y + 2; return x; }";
        let code3 = "function test(y) { let x = y * 2; return x; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_switch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { switch(x) { case 1: break; default: break; } }";
        let code2 = "function test(x) { switch(x) { case 1: break; case 2: break; default: break; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_do_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x = 0; do { x++; } while (x < 10); }";
        let code2 = "function test() { let y = 0; do { y++; } while (y < 10); }";
        let code3 = "function test() { let x = 0; while (x < 10) { x++; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "Do-while vs while");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_for_in() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { for (let x in obj) { } }";
        let code2 = "function test() { for (let y in obj) { } }";
        let code3 = "function test() { for (let i = 0; i < 10; i++) { } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "For-in vs traditional for");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_for_of() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { for (let x of arr) { } }";
        let code2 = "function test() { for (let y of arr) { } }";
        let code3 = "function test() { for (let i in obj) { } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "For-of vs for-in");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_logical_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x, y) { return x && y; }";
        let code2 = "function test(x, y) { return x || y; }";
        let code3 = "function test(x) { return !x; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_null_literal() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return null; }";
        let code2 = "function test() { return null; }";
        let code3 = "function test() { return undefined; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_eq!(hash1, hash2, "Same null value");
        assert_ne!(hash1, hash3, "Null vs undefined");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_destructuring_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { const [x, y] = [1, 2]; }";
        let code2 = "function test() { const [x, y] = [2, 3]; }";
        let code3 = "function test() { const x = 1; const y = 2; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different array values");
        assert_ne!(hash1, hash3, "Destructuring vs separate assignments");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let x = 10; while (x > 0) { x--; } }";
        let code2 = "function test() { let x = 10; while (x < 0) { x++; } }";
        let code3 = "function test() { for (let i = 0; i < 10; i++) {} }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different conditions");
        assert_ne!(hash1, hash3, "While vs for");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_try_catch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { try { let x = 1; } catch (e) {} }";
        let code2 = "function test() { try { let x = 1; } catch (error) {} }";
        let code3 = "function test() { try { let x = 1; } finally {} }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "Catch vs finally");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_throw() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { throw new Error('error1'); }";
        let code2 = "function test() { throw new Error('error2'); }";
        let code3 = "function test() { throw new TypeError('error1'); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different messages");
        assert_ne!(hash1, hash3, "Different exception types");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_assert() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { console.assert(x > 0); }";
        let code2 = "function test(x) { console.assert(x < 0); }";
        let code3 = "function test(x) { if (!(x > 0)) throw new Error(); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different conditions");
        assert_ne!(hash1, hash3, "Assert vs if/throw");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_break_continue() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { for (let i = 0; i < 10; i++) { if (i > 5) break; } }";
        let code2 = "function test() { for (let i = 0; i < 10; i++) { if (i > 5) continue; } }";
        let code3 = "function test() { for (let i = 0; i < 10; i++) { if (i > 5) return; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Break vs continue");
        assert_ne!(hash1, hash3, "Break vs return");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_membership_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return 'prop' in x; }";
        let code2 = "function test(x) { return 'other' in x; }";
        let code3 = "function test(x) { return x.hasOwnProperty('prop'); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different property names");
        assert_ne!(hash1, hash3, "in vs hasOwnProperty");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_instanceof() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return x instanceof Array; }";
        let code2 = "function test(x) { return x instanceof Object; }";
        let code3 = "function test(x) { return Array.isArray(x); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different instanceof types");
        assert_ne!(hash1, hash3, "instanceof vs method call");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_typeof() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return typeof x; }";
        let code2 = "function test(x) { return typeof x === 'string'; }";
        let code3 = "function test(x) { return x.constructor.name; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "typeof vs typeof with comparison");
        assert_ne!(hash1, hash3, "typeof vs property access");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_compound_assignment() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { x += 1; }";
        let code2 = "function test(x) { x -= 1; }";
        let code3 = "function test(x) { x *= 2; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "+= vs -=");
        assert_ne!(hash1, hash3, "+= vs *=");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let str = 'test'; let len = str.length; }";
        let code2 = "function test() { let str = 'test'; let upper = str.toUpperCase(); }";
        let code3 = "function test() { let str = 'test'; let empty = str.isEmpty(); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different method calls");
        assert_ne!(hash1, hash3, "Different method calls");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_parameter_order() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x, y) { return x + y; }";
        let code2 = "function test(y, x) { return y + x; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_boolean_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return true; }";
        let code2 = "function test() { return false; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_string_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return 'hello'; }";
        let code2 = "function test() { return 'world'; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_integer_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return 1; }";
        let code2 = "function test() { return 2; }";
        let code3 = "function test() { return 100; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "javascript", None);
    }

    // ============================================================================
    // Edge Cases: Language-Specific Features
    // ============================================================================

    #[test]
    fn test_ut_hash_edge_javascript_arrow_function() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "const test = (x) => x + 1;";
        let code2 = "const test = (y) => y + 1;";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
    }

    #[test]
    fn test_ut_hash_edge_javascript_arrow_function_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "const test = (x) => x + 1;";
        let code2 = "const test = (x) => x + 2;";
        let code3 = "function test(x) { return x + 1; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different operations");
        assert_ne!(hash1, hash3, "Arrow function vs regular function");
    }

    #[test]
    fn test_ut_hash_edge_javascript_template_literal() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return `Hello ${x}`; }";
        let code2 = "function test(x) { return `World ${x}`; }";
        let code3 = "function test(x) { return 'Hello ' + x; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different string literals");
        assert_ne!(hash1, hash3, "Template literal vs concatenation");
    }

    #[test]
    fn test_ut_hash_edge_javascript_optional_chaining() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(obj) { return obj?.property; }";
        let code2 = "function test(obj) { return obj.property; }";
        let code3 = "function test(obj) { return obj?.['property']; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Optional chaining vs regular access should be different");
        assert_ne!(hash1, hash3, "Different optional chaining syntax");
    }

    #[test]
    fn test_ut_hash_edge_javascript_qualified_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let obj = { method: function() {} }; obj.method(); }";
        let code2 = "function test() { let o = { method: function() {} }; o.method(); }";
        let code3 = "function test() { let obj = { other: function() {} }; obj.other(); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Receiver spelling is retained");
        assert_ne!(hash1, hash3, "Different method names");
    }

    #[test]
    fn test_ut_hash_edge_javascript_spread_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let arr = [1, 2, 3]; let newArr = [...arr, 4]; }";
        let code2 = "function test() { let arr = [1, 2, 3]; let newArr = [...arr, 5]; }";
        let code3 = "function test() { let arr = [1, 2, 3]; let newArr = arr.concat([4]); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different values");
        assert_ne!(hash1, hash3, "Spread operator vs concat");
    }

    #[test]
    fn test_ut_hash_edge_javascript_rest_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(...args) { return args.length; }";
        let code2 = "function test(...params) { return params.length; }";
        let code3 = "function test() { let args = Array.from(arguments); return args.length; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Parameter spelling is retained");
        assert_ne!(hash1, hash3, "Rest parameters vs arguments");
    }

    #[test]
    fn test_ut_hash_edge_javascript_class_methods() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "class Test { method(x) { return x + 1; } }";
        let code2 = "class Test { method(y) { return y + 1; } }";
        let code3 = "class Test { other(x) { return x + 1; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Parameter spelling is retained");
        assert_ne!(hash1, hash3, "Different method names");
    }

    #[test]
    fn test_ut_hash_edge_javascript_static_methods() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "class Test { static method(x) { return x + 1; } }";
        let code2 = "class Test { static method(y) { return y + 1; } }";
        let code3 = "class Test { method(x) { return x + 1; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Parameter spelling is retained");
        assert_ne!(hash1, hash3, "Static vs instance method");
    }

    #[test]
    fn test_ut_hash_edge_javascript_label() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { label: for (let i = 0; i < 10; i++) { break label; } }";
        let code2 = "function test() { label: for (let i = 0; i < 10; i++) { break label; } }";
        let code3 = "function test() { for (let i = 0; i < 10; i++) { break; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_eq!(hash1, hash2, "Same logic");
        assert_ne!(hash1, hash3, "Labeled break vs unlabeled break");
    }

    #[test]
    fn test_ut_hash_edge_javascript_async_await() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async function test(x) { const result = await func(x); return result; }";
        let code2 = "async function test(y) { const result = await func(y); return result; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
    }

    #[test]
    fn test_ut_hash_edge_javascript_async_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async function test(x) { return x + 1; }";
        let code2 = "function test(x) { return x + 1; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Async vs sync should be different");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_generator_function() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function* gen(x) { yield x; }";
        let code2 = "function* gen(y) { yield y; }";
        let code3 = "function gen(x) { return x; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "Generator vs regular function");
    }

    #[test]
    fn test_ut_hash_edge_javascript_method_chaining() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { return obj.method1().method2().method3(); }";
        let code2 = "function test() { return obj.method1().method2().method3(); }";
        let code3 = "function test() { return obj.method1().method2().method4(); }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_eq!(hash1, hash2, "Same logic");
        assert_ne!(hash1, hash3, "Different method names");
    }

    #[test]
    fn test_ut_hash_edge_javascript_nullish_coalescing() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { return x ?? 'default'; }";
        let code2 = "function test(x) { return x ?? 'other'; }";
        let code3 = "function test(x) { return x || 'default'; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different default values");
        assert_ne!(hash1, hash3, "?? vs ||");
    }

    #[test]
    fn test_ut_hash_edge_javascript_logical_assignment() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x) { x &&= 1; }";
        let code2 = "function test(x) { x ||= 1; }";
        let code3 = "function test(x) { if (x) x = 1; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "&&= vs ||=");
        assert_ne!(hash1, hash3, "Logical assignment vs if");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_computed_property() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { const key = 'prop'; const obj = { [key]: 'value' }; }";
        let code2 = "function test() { const other = 'prop'; const obj = { [other]: 'value' }; }";
        let code3 = "function test() { const obj = { key: 'value' }; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "Computed property vs literal property");
    }

    #[test]
    fn test_ut_hash_1_3_javascript_delete_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(obj) { delete obj.prop; }";
        let code2 = "function test(obj) { delete obj.other; }";
        let code3 = "function test(obj) { obj.prop = undefined; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different property names");
        assert_ne!(hash1, hash3, "delete vs assignment");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_1_3_javascript_void_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { void expression; }";
        let code2 = "function test() { expression; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "void operator vs plain expression");
    }

    #[test]
    fn test_ut_hash_edge_javascript_with_statement() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(obj) { with (obj) { prop = value; } }";
        let code2 = "function test(obj) { obj.prop = value; }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "with statement vs direct property access");
    }

    // ============================================================================
    // UT-Hash-1.5: Structural Completeness Verification
    // ============================================================================

    #[test]
    fn test_ut_hash_1_5_javascript_control_flow_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_if = "function test(x) { if (x > 0) {} }";
        let code_switch = "function test(x) { switch(x) { case 1: break; } }";
        let code_try = "function test() { try {} catch (e) {} }";
        let code_throw = "function test() { throw new Error(); }";
        let code_for = "function test() { for (let i = 0; i < 10; i++) {} }";
        let code_while = "function test() { while (x > 0) {} }";
        let code_do_while = "function test() { do {} while (x > 0); }";
        let codes = vec![
            code_if,
            code_switch,
            code_try,
            code_throw,
            code_for,
            code_while,
            code_do_while,
        ];
        assert_hash_set_size_with_debug(
            &hasher,
            &codes,
            7,
            "javascript",
            Some("All control flow statements should produce distinct hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_javascript_operator_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_add = "function test(x, y) { return x + y; }";
        let code_sub = "function test(x, y) { return x - y; }";
        let code_mul = "function test(x, y) { return x * y; }";
        let code_and = "function test(x, y) { return x && y; }";
        let code_or = "function test(x, y) { return x || y; }";
        let code_eq = "function test(x, y) { return x === y; }";
        let code_in = "function test(x) { return 'prop' in x; }";
        let codes = vec![code_add, code_sub, code_mul, code_and, code_or, code_eq, code_in];
        assert_hash_set_size_with_debug(
            &hasher,
            &codes,
            7,
            "javascript",
            Some("All operators should produce distinct hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_javascript_external_call_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test() { let str = 'test'; let len = str.length; }";
        let code2 = "function test() { let s = 'test'; let l = s.length; }";
        let code3 = "function test() { let str = 'test'; let upper = str.toUpperCase(); }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "javascript", None);
    }

    #[test]
    fn test_ut_hash_1_5_javascript_scope_isolation_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_nested = "function outer(x) { function inner(y) { return x + y; } return inner(1); }";
        let code_block = "function test() { let x = 1; if (true) { let y = 2; } return x; }";
        let code_class = "class Test { method(x) { return x; } }";
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_nested, code_block, code_class],
            "javascript",
            Some("Different scope structures should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_edge_javascript_private_fields() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "class Test { #private = 1; method() { return this.#private; } }";
        let code2 = "class Test { #private = 2; method() { return this.#private; } }";
        let code3 = "class Test { private = 1; method() { return this.private; } }";
        let hash1 = hasher.compute_hash(code1, "javascript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "javascript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "javascript", None).unwrap();
        assert_ne!(hash1, hash2, "Different values");
        assert_ne!(hash1, hash3, "Private vs public field");
    }
}

/// Tests for canonical normalization behaviors specific to JavaScript.
mod test_javascript_canonical_normalization {
    use super::*;

    #[test]
    fn test_numeric_literal_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let codes = vec![
            "function value() { return 10; }",
            "function value() { return 0xA; }",
            "function value() { return 0o12; }",
            "function value() { return 0b1010; }",
            "function value() { return 1_0; }",
        ];
        assert_hash_set_size_with_debug(&hasher, &codes, 5, "javascript", None);
    }
}

/// Tests for scope analysis and global function preservation in JavaScript.
mod test_javascript_scope_analysis {
    use super::*;

    #[test]
    fn test_global_function_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x) {
            console.log(x);
        }
        "#;
        let code2 = r#"
        function test(x) {
            console.error(x);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_builtin_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(a, b) {
            return Math.max(a, b);
        }
        "#;
        let code2 = r#"
        function test(a, b) {
            return Math.min(a, b);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_import_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import os from 'os';
        function test() {
            os.platform();
        }
        "#;
        let code2 = r#"
        import fs from 'fs';
        function test() {
            fs.readFile('file.txt');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_import_named_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import { readFile } from 'fs';
        function test() {
            readFile('file.txt');
        }
        "#;
        let code2 = r#"
        import { writeFile } from 'fs';
        function test() {
            writeFile('file.txt', 'data');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_import_default_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import React from 'react';
        function test() {
            React.createElement('div');
        }
        "#;
        let code2 = r#"
        import Vue from 'vue';
        function test() {
            Vue.createApp({});
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_require_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        const http = require('http');
        function test() {
            http.createServer();
        }
        "#;
        let code2 = r#"
        const https = require('https');
        function test() {
            https.createServer();
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_external_module_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        const fs = require('fs');
        function test() {
            fs.readFile('file.txt');
        }
        "#;
        let code2 = r#"
        const fs = require('fs');
        function test() {
            fs.writeFile('file.txt', 'data');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_attribute_method_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(obj) {
            obj.method1();
        }
        "#;
        let code2 = r#"
        function test(obj) {
            obj.method2();
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_mixed_local_and_global() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x) {
            const localVar = x + 1;
            console.log(localVar);
        }
        "#;
        let code2 = r#"
        function test(x) {
            const localVar = x + 1;
            console.error(localVar);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_window_global_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test() {
            window.alert('test');
        }
        "#;
        let code2 = r#"
        function test() {
            window.confirm('test');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_console_log_not_normalized() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x) {
            console.log(x);
        }
        "#;
        let code2 = r#"
        function test(x) {
            console.log(x);
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "javascript", None);
        let code3 = r#"
        function test(x) {
            console.error(x);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "javascript", None);
    }
}

/// Tests for JSX support in JavaScript visitor.
mod test_javascript_jsx_support {
    use super::*;

    #[test]
    fn test_jsx_attribute_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function Component() {
            const handler = () => {};
            return <Button onClick={handler} />;
        }
        "#;
        let code2 = r#"
        function Component() {
            const handler = () => {};
            return <Button onClick={handler} />;
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_jsx_props_normalization() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function Component() {
            const x = () => {};
            return <Button onClick={x} disabled={true} />;
        }
        "#;
        let code2 = r#"
        function Component() {
            const y = () => {};
            return <Button onClick={y} disabled={true} />;
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_jsx_different_attributes_different_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function Component() {
            return <Button onClick={() => {}} />;
        }
        "#;
        let code2 = r#"
        function Component() {
            return <Button onPress={() => {}} />;
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }
}

/// Tests for arrow function implicit return normalization.
mod test_javascript_arrow_function_normalization {
    use super::*;

    #[test]
    fn test_arrow_function_implicit_vs_explicit() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test() {
            const fn = (x) => x + 1;
            return fn(5);
        }
        "#;
        let code2 = r#"
        function test() {
            const fn = (x) => { return x + 1; };
            return fn(5);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }

    #[test]
    fn test_arrow_function_multiple_statements() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test() {
            const fn = (x) => {
                const y = x + 1;
                return y;
            };
            return fn(5);
        }
        "#;
        let code2 = r#"
        function test() {
            const fn = (x) => {
                const y = x + 1;
                return y;
            };
            return fn(5);
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }
}

/// Tests for semicolon independence in JavaScript.
mod test_javascript_semicolon_handling {
    use super::*;

    #[test]
    fn test_javascript_semicolon_independence() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test() {
            const x = 1;
            const y = 2;
            return x + y;
        }
        "#;
        let code2 = r#"
        function test() {
            const x = 1;
            const y = 2;
            return x + y;
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "javascript", None);
    }
}

/// Tests for default parameter values in JavaScript (spec §10.1).
mod test_javascript_default_parameters {
    use super::*;

    #[test]
    fn test_javascript_default_parameters_affect_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function connect(retries = 5, timeout = 30) {
            return retries + timeout;
        }
        "#;
        let code2 = r#"
        function connect(retries = 0, timeout = null) {
            return retries + timeout;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Different default parameter values should produce different hashes"),
        );
    }

    #[test]
    fn test_javascript_same_default_parameters_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function connect(retries = 5, timeout = 30) {
            return retries + timeout;
        }
        "#;
        let code2 = r#"
        function connect(retries = 5, timeout = 30) {
            return retries + timeout;
        }
        "#;
        assert_hash_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Same default parameter values should produce same hash"),
        );
    }

    #[test]
    fn test_javascript_with_default_vs_without_default() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function process(x, y = 10) {
            return x + y;
        }
        "#;
        let code2 = r#"
        function process(x, y) {
            return x + y;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Function with default parameter should differ from function without"),
        );
    }

    #[test]
    fn test_javascript_operand_order_preserved() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function add(x, y) {
            return x + y;
        }
        "#;
        let code2 = r#"
        function add(x, y) {
            return y + x;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Operand order should be preserved (x + y vs y + x should differ)"),
        );
    }

    #[test]
    fn test_closure_captures_different_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function outer(x, y) {
            return () => x;
        }
        "#;
        let code2 = r#"
        function outer(x, y) {
            return () => y;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Closures capturing different parameters must produce different hashes"),
        );
    }

    #[test]
    fn test_closure_captures_different_locals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function outer() {
            let x = 1;
            let y = 2;
            return () => x;
        }
        "#;
        let code2 = r#"
        function outer() {
            let x = 1;
            let y = 2;
            return () => y;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Closures capturing different local variables must produce different hashes"),
        );
    }

    #[test]
    fn test_nested_closure_multiple_levels() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function a(x, z) {
            function b(y) {
                function c() {
                    return z;
                }
                return c;
            }
            return b;
        }
        "#;
        let code2 = r#"
        function a(x, z) {
            function b(y) {
                function c() {
                    return y;
                }
                return c;
            }
            return b;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "javascript",
            Some("Nested closures capturing from different levels must produce different hashes"),
        );
    }
}

/// Default and explicit v1 retain the same syntax inputs.
mod test_javascript_policy_entry_points {
    use super::*;

    #[test]
    fn test_loop_variable_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function calculateTotal(prices) {
            let total = 0;
            for (const price of prices) {
                total = total + price;
            }
            return total;
        }
        "#;
        let code2 = r#"
        function calculateTotal(costs) {
            let sum = 0;
            for (const cost of costs) {
                sum = sum + cost;
            }
            return sum;
        }
        "#;
        let policy_explicit = StructuralHashPolicy::v1();
        let policy_default = StructuralHashPolicy::default();
        let hash1_explicit = hasher
            .compute_hash(code1, "javascript", Some(&policy_explicit))
            .unwrap();
        let hash2_explicit = hasher
            .compute_hash(code2, "javascript", Some(&policy_explicit))
            .unwrap();
        assert_ne!(
            hash1_explicit, hash2_explicit,
            "Explicit v1 retains for-of variable spelling"
        );
        let hash1_default = hasher.compute_hash(code1, "javascript", Some(&policy_default)).unwrap();
        let hash2_default = hasher.compute_hash(code2, "javascript", Some(&policy_default)).unwrap();
        assert_ne!(
            hash1_default, hash2_default,
            "With the default policy, different loop variable names should produce different hashes"
        );
    }
}
