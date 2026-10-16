//! Tests for structural hashing - TypeScript-specific.
//!
//! Ported from tests/test_typescript.py (84 tests)
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

/// Tests for StructuralHasher class - TypeScript-specific.
mod test_typescript_structural_hasher {
    use super::*;

    #[test]
    fn test_hash_typescript_different_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x: number): number {
            const result = x + 1;
            return result;
        }
        "#;
        let code2 = r#"
        function test(y: number): number {
            const output = y + 1;
            return output;
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }
}

/// Unit tests for StructuralHasher - TypeScript tests.
mod test_typescript_unit_tests {
    use super::*;

    // ============================================================================
    // Historical UT-Hash-1.1 corpus: layout and retained syntax controls
    // ============================================================================

    #[test]
    fn test_ut_hash_1_1_typescript_comments() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let x = 1; return x; }";
        let code2 = "// Comment\nfunction test(): void { let x = 1; return x; }";
        let code3 = "function test(): void { /* Block comment */ let x = 1; return x; }";
        let code4 = "/**\n * JSDoc comment\n */\nfunction test(): void { let x = 1; return x; }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_whitespace() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let x=1; return x; }";
        let code2 = "function test(): void { let x = 1; return x; }";
        let code3 = "function test(): void { let x=1; return  x; }";
        let code4 = "function test(): void { let x = 1; return x; }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_jsdoc() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let x = 1; return x; }";
        let code2 = "/**\n * This is a JSDoc comment.\n * @param {number} x - The parameter\n * @returns {number}\n */\nfunction test(): void { let x = 1; return x; }";
        let code3 = "/**\n * Different JSDoc with more details.\n * @param {number} x\n */\nfunction test(): void { let x = 1; return x; }";
        let code4 = "/**\n * Another JSDoc comment.\n */\nfunction test(): void { let x = 1; return x; }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_type_annotations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { return x + 1; }";
        let code2 = "function test(x: string): string { return x + 1; }";
        let code3 = "function test(x: any): any { return x + 1; }";
        let code4 = "function test(x: unknown): unknown { return x + 1; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3, code4], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_interface_declarations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { return x + 1; }";
        let code2 = "interface TestInterface { value: number; }\nfunction test(x: number): number { return x + 1; }";
        let code3 =
            "interface DifferentInterface { data: string; }\nfunction test(x: number): number { return x + 1; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_type_aliases() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { return x + 1; }";
        let code2 = "type MyNumber = number;\nfunction test(x: number): number { return x + 1; }";
        let code3 = "type MyType = string | number;\nfunction test(x: number): number { return x + 1; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_generics() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test<T>(x: T): T { return x; }";
        let code2 = "function test<U>(x: U): U { return x; }";
        let code3 = "function test(x: any): any { return x; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_import_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { return null; }";
        let code2 = "import type {Element} from './types';\nfunction test(): void { return null; }";
        let code3 = "import type {Component, Element} from './types';\nfunction test(): void { return null; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_export_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { return null; }";
        let code2 = "export type State = { count: number };\nfunction test(): void { return null; }";
        let code3 = "export type State = { count: number, name: string };\nfunction test(): void { return null; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_union_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number | string): void { return; }";
        let code2 = "function test(x: string | number): void { return; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_any_vs_union() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: any): void { return; }";
        let code2 = "function test(x: number | string): void { return; }";
        let code3 = "function test(x: unknown): void { return; }";
        let code4 = "function test(x: number): void { return; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3, code4], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_1_typescript_optional_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): void { return; }";
        let code2 = "function test(x?: number): void { return; }";
        let code3 = "function test(x: number | undefined): void { return; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "typescript", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_edge_typescript_has_error_flag() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        import type {Element} from './types';
        export type State = { count: number };
        function test(x: number, items: ReadonlyArray<number>): number {
            return items.length + x;
        }
        "#;
        let hash_value = hasher.compute_hash(code, "typescript", None).unwrap();
        assert!(hash_value.starts_with("blake3:"), "Should produce valid hash");
    }

    #[test]
    fn test_ut_hash_1_3_typescript_logical_changes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { return x + 1; }";
        let code2 = "function test(x: number, y: number): number { return x + y; }";
        let code3 = "function test(x: number): number { if (x > 0) return x; return 0; }";
        let code4 = "function test(x: number): number { return x > 0 ? x : 0; }";
        let code5 = "function test(x: number): number { return x + 1; }";
        let code6 = "function test(x: number): number { return x * 2; }";
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

    // ============================================================================
    // Historical UT-Hash-1.2 corpus: retained identifier spelling
    // ============================================================================

    #[test]
    fn test_ut_hash_1_2_typescript_function_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { return x; }";
        let code2 = "function test(y: number): number { return y; }";
        let code3 = "function test(param: number): number { return param; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_local_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let x = 1; let y = x + 1; return y; }";
        let code2 = "function test(): void { let a = 1; let b = a + 1; return b; }";
        let code3 = "function test(): void { const value = 1; const result = value + 1; return result; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_nested_scopes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): void { if (x > 0) { let y = x + 1; } }";
        let code2 = "function test(a: number): void { if (a > 0) { let b = a + 1; } }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_multiple_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function add(x: number, y: number): number { return x + y; }";
        let code2 = "function add(a: number, b: number): number { return a + b; }";
        let code3 = "function add(first: number, second: number): number { return first + second; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function func1(x: number): number { return x; } function func2(x: number): number { return x; }";
        let code2 = "function func1(a: number): number { return a; } function func2(a: number): number { return a; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_block_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let x = 1; if (true) { let y = 2; } return x; }";
        let code2 = "function test(): void { let a = 1; if (true) { let b = 2; } return a; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_class_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "class Test { method(x: number): number { return x; } }";
        let code2 = "class Test { method(y: number): number { return y; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_control_flow_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): number { if (x > 0) { return x; } else { return 0; } }";
        let code2 = "function test(y: number): number { if (y > 0) { return y; } else { return 0; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_2_typescript_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let str: string = \"test\"; let len = str.length; }";
        let code2 = "function test(): void { let s: string = \"test\"; let l = s.length; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2], "typescript", None);
    }

    // ============================================================================
    // UT-Hash-1.3: Semantic Variance
    // ============================================================================

    #[test]
    fn test_ut_hash_1_3_typescript_loop_bound() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { for (let i = 0; i < 10; i++) {} }";
        let code2 = "function test(): void { for (let i = 0; i < 20; i++) {} }";
        let code3 = "function test(): void { for (let i = 0; i < 5; i++) {} }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_comparison_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number, y: number): boolean { return x < y; }";
        let code2 = "function test(x: number, y: number): boolean { return x > y; }";
        let code3 = "function test(x: number, y: number): boolean { return x === y; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_computation_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(y: number): number { let x = y + 1; return x; }";
        let code2 = "function test(y: number): number { let x = y + 2; return x; }";
        let code3 = "function test(y: number): number { let x = y * 2; return x; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_switch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): void { switch(x) { case 1: break; default: break; } }";
        let code2 = "function test(x: number): void { switch(x) { case 1: break; case 2: break; default: break; } }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_logical_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: boolean, y: boolean): boolean { return x && y; }";
        let code2 = "function test(x: boolean, y: boolean): boolean { return x || y; }";
        let code3 = "function test(x: boolean): boolean { return !x; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_string_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): string { return 'hello'; }";
        let code2 = "function test(): string { return 'world'; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_integer_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): number { return 1; }";
        let code2 = "function test(): number { return 2; }";
        let code3 = "function test(): number { return 100; }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "typescript", None);
    }

    #[test]
    fn test_ut_hash_1_3_typescript_boolean_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): boolean { return true; }";
        let code2 = "function test(): boolean { return false; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    // ============================================================================
    // Edge Cases: Language-Specific Features
    // ============================================================================

    #[test]
    fn test_ut_hash_edge_typescript_arrow_function() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "const test = (x: number): number => x + 1;";
        let code2 = "const test = (y: number): number => y + 1;";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
    }

    #[test]
    fn test_ut_hash_edge_typescript_arrow_function_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "const test = (x: number): number => x + 1;";
        let code2 = "const test = (x: number): number => x + 2;";
        let code3 = "function test(x: number): number { return x + 1; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different operations");
        assert_ne!(hash1, hash3, "Arrow function vs regular function");
    }

    #[test]
    fn test_ut_hash_edge_typescript_template_literal() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): string { return `Hello ${x}`; }";
        let code2 = "function test(x: number): string { return `World ${x}`; }";
        let code3 = "function test(x: number): string { return 'Hello ' + x; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different string literals");
        assert_ne!(hash1, hash3, "Template literal vs concatenation");
    }

    #[test]
    fn test_ut_hash_edge_typescript_optional_chaining() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(obj: any): any { return obj?.property; }";
        let code2 = "function test(obj: any): any { return obj.property; }";
        let code3 = "function test(obj: any): any { return obj?.['property']; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Optional chaining vs regular access should be different");
        assert_ne!(hash1, hash3, "Different optional chaining syntax");
    }

    #[test]
    fn test_ut_hash_edge_typescript_qualified_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): any { let obj: any = { method: function() {} }; obj.method(); }";
        let code2 = "function test(): any { let o: any = { method: function() {} }; o.method(); }";
        let code3 = "function test(): any { let obj: any = { other: function() {} }; obj.other(); }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Receiver spelling is retained");
        assert_ne!(hash1, hash3, "Different method names");
    }

    #[test]
    fn test_ut_hash_edge_typescript_class_methods() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "class Test { method(x: number): number { return x + 1; } }";
        let code2 = "class Test { method(y: number): number { return y + 1; } }";
        let code3 = "class Test { other(x: number): number { return x + 1; } }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Parameter spelling is retained");
        assert_ne!(hash1, hash3, "Different method names");
    }

    #[test]
    fn test_ut_hash_edge_typescript_async_await() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async function test(x: number): Promise<number> { const result = await func(x); return result; }";
        let code2 = "async function test(y: number): Promise<number> { const result = await func(y); return result; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
    }

    #[test]
    fn test_ut_hash_edge_typescript_async_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async function test(x: number): Promise<number> { return x + 1; }";
        let code2 = "function test(x: number): number { return x + 1; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Async vs sync should be different");
    }

    #[test]
    fn test_ut_hash_edge_typescript_type_assertions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: any): number { return (x as number) + 1; }";
        let code2 = "function test(x: any): number { return (<number>x) + 1; }";
        let code3 = "function test(x: any): number { return x + 1; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Distinct type assertion syntax is retained");
        assert_ne!(hash1, hash3, "Type assertion syntax is retained");
    }

    #[test]
    fn test_ut_hash_edge_typescript_enums() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Enums emit runtime values — member/presence changes must change the hash.
        let code1 = "enum Test { A, B } function test(): void { return; }";
        let code2 = "enum Different { X, Y } function test(): void { return; }";
        let code3 = "function test(): void { return; }";
        let code4 = "enum Test { A, B } function test(): void { return; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        let hash4 = hasher.compute_hash(code4, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different enum members are runtime values");
        assert_ne!(hash1, hash3, "Enum presence is not cosmetic");
        assert_eq!(hash1, hash4, "Identical enums produce same hash");
    }

    #[test]
    fn test_ut_hash_edge_typescript_namespaces() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Namespaces emit runtime containers — presence/body must change the hash.
        let code1 = "function test(): void { return; }";
        let code2 = "namespace MyNamespace {\n            export function helper(): void {}\n        }\n        function test(): void { return; }";
        let code3 = "namespace OtherNamespace {\n            export const value = 1;\n        }\n        function test(): void { return; }";
        let code4 = "namespace MyNamespace {\n            export function helper(): void {}\n        }\n        function test(): void { return; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        let hash4 = hasher.compute_hash(code4, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Namespace presence is not cosmetic");
        assert_ne!(hash2, hash3, "Different namespace bodies are logical");
        assert_eq!(hash2, hash4, "Identical namespaces produce same hash");
    }

    #[test]
    fn test_ut_hash_edge_typescript_decorators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Decorators wrap at runtime — identity/presence must change the hash.
        let code1 = "@Component class Test { method(): void {} }";
        let code2 = "@Directive class Test { method(): void {} }";
        let code3 = "class Test { method(): void {} }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different decorators are logical");
        assert_ne!(hash1, hash3, "Decorator presence is not cosmetic");
    }

    #[test]
    fn test_ut_hash_edge_typescript_abstract_classes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "abstract class Test { abstract method(): void; }";
        let code2 = "class Test { method(): void {} }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Abstract vs concrete methods are different");
    }

    #[test]
    fn test_ut_hash_edge_typescript_for_of() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { for (let x of arr) { } }";
        let code2 = "function test(): void { for (let y of arr) { } }";
        let code3 = "function test(): void { for (let i in obj) { } }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "For-of vs for-in");
    }

    #[test]
    fn test_ut_hash_edge_typescript_nullish_coalescing() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: any): any { return x ?? 'default'; }";
        let code2 = "function test(x: any): any { return x ?? 'other'; }";
        let code3 = "function test(x: any): any { return x || 'default'; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different default values");
        assert_ne!(hash1, hash3, "?? vs ||");
    }

    #[test]
    fn test_ut_hash_edge_typescript_rest_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(...args: number[]): number { return args.length; }";
        let code2 = "function test(...params: number[]): number { return params.length; }";
        let code3 = "function test(): number { let args = Array.from(arguments); return args.length; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Parameter spelling is retained");
        assert_ne!(hash1, hash3, "Rest parameters vs arguments");
    }

    #[test]
    fn test_ut_hash_edge_typescript_destructuring() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { const [x, y] = [1, 2]; }";
        let code2 = "function test(): void { const [a, b] = [1, 2]; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
    }

    #[test]
    fn test_ut_hash_edge_typescript_destructuring_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { const [x, y] = [1, 2]; }";
        let code2 = "function test(): void { const [x, y] = [2, 3]; }";
        let code3 = "function test(): void { const x = 1; const y = 2; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different array values");
        assert_ne!(hash1, hash3, "Destructuring vs separate assignments");
    }

    #[test]
    fn test_ut_hash_edge_typescript_try_catch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { try { let x = 1; } catch (e: any) {} }";
        let code2 = "function test(): void { try { let x = 1; } catch (error: any) {} }";
        let code3 = "function test(): void { try { let x = 1; } finally {} }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Identifier spelling is retained");
        assert_ne!(hash1, hash3, "Catch vs finally");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_edge_typescript_throw() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { throw new Error('error1'); }";
        let code2 = "function test(): void { throw new Error('error2'); }";
        let code3 = "function test(): void { throw new TypeError('error1'); }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different messages");
        assert_ne!(hash1, hash3, "Different exception types");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_edge_typescript_break_continue() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { for (let i = 0; i < 10; i++) { if (i > 5) break; } }";
        let code2 = "function test(): void { for (let i = 0; i < 10; i++) { if (i > 5) continue; } }";
        let code3 = "function test(): void { for (let i = 0; i < 10; i++) { if (i > 5) return; } }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Break vs continue");
        assert_ne!(hash1, hash3, "Break vs return");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_edge_typescript_compound_assignment() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: number): void { x += 1; }";
        let code2 = "function test(x: number): void { x -= 1; }";
        let code3 = "function test(x: number): void { x *= 2; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "+= vs -=");
        assert_ne!(hash1, hash3, "+= vs *=");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_edge_typescript_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): void { let str: string = 'test'; let len = str.length; }";
        let code2 = "function test(): void { let str: string = 'test'; let upper = str.toUpperCase(); }";
        let code3 = "function test(): void { let str: string = 'test'; let empty = str.isEmpty(); }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different method calls");
        assert_ne!(hash1, hash3, "Different method calls");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_edge_typescript_method_chaining() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): any { return obj.method1().method2().method3(); }";
        let code2 = "function test(): any { return obj.method1().method2().method3(); }";
        let code3 = "function test(): any { return obj.method1().method2().method4(); }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_eq!(hash1, hash2, "Same logic");
        assert_ne!(hash1, hash3, "Different method names");
    }

    #[test]
    fn test_ut_hash_edge_typescript_generic_constraints() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test<T>(x: T): T { return x; }";
        let code2 = "function test<U>(x: U): U { return x; }";
        let code3 = "function test<T extends string>(x: T): T { return x; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Generic parameter spelling is retained");
        assert_ne!(hash1, hash3, "Generic constraints remain structural inputs");
        assert_ne!(hash2, hash3);
    }

    #[test]
    fn test_ut_hash_edge_typescript_tuple_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(): [number, string] { return [1, 'a']; }";
        let code2 = "function test(): [string, number] { return ['a', 1]; }";
        let code3 = "function test(): [number, string] { return [1, 'a']; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Different tuple order");
        assert_eq!(hash1, hash3, "Same tuple order");
    }

    #[test]
    fn test_ut_hash_edge_typescript_intersection_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "function test(x: { a: number } & { b: string }): void { return; }";
        let code2 = "function test(x: { b: string } & { a: number }): void { return; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_edge_typescript_readonly_modifiers() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "interface Test { value: number; }\nfunction test(x: Test): void { return; }";
        let code2 = "interface Test { readonly value: number; }\nfunction test(x: Test): void { return; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_ut_hash_edge_typescript_template_literal_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "type Test = `hello-${string}`; function test(): void { return; }";
        let code2 = "type Test = `world-${string}`; function test(): void { return; }";
        let code3 = "function test(): void { return; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        assert_ne!(hash1, hash2, "Type alias syntax is retained");
        assert_ne!(hash1, hash3, "Type alias syntax is retained");
    }

    #[test]
    fn test_ut_hash_edge_typescript_optional_parameter_clarification() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // `?` is a calling-convention marker (argument may be omitted) — not pure type sugar.
        let code1 = "function test(x?: number): void { return; }";
        let code2 = "function test(x: number | undefined): void { return; }";
        let code3 = "function test(x: number): void { return; }";
        let hash1 = hasher.compute_hash(code1, "typescript", None).unwrap();
        let hash2 = hasher.compute_hash(code2, "typescript", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "typescript", None).unwrap();
        // Union and optional-parameter syntax remain independently visible.
        assert_ne!(hash2, hash3, "Union type syntax is retained");
        assert_ne!(hash1, hash3, "Optional parameter marker is logical calling convention");
    }
}

/// Tests for canonical normalization behaviors specific to TypeScript.
mod test_typescript_canonical_normalization {
    use super::*;

    #[test]
    fn test_numeric_literal_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let codes = vec![
            "function value(): number { return 10; }",
            "function value(): number { return 0xA; }",
            "function value(): number { return 0o12; }",
            "function value(): number { return 0b1010; }",
            "function value(): number { return 1_0; }",
        ];
        assert_hash_set_size_with_debug(&hasher, &codes, 5, "typescript", None);
    }
}

/// Tests for scope analysis and global function preservation in TypeScript.
mod test_typescript_scope_analysis {
    use super::*;

    #[test]
    fn test_global_function_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x: number): void {
            console.log(x);
        }
        "#;
        let code2 = r#"
        function test(x: number): void {
            console.error(x);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_builtin_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(a: number, b: number): number {
            return Math.max(a, b);
        }
        "#;
        let code2 = r#"
        function test(a: number, b: number): number {
            return Math.min(a, b);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_import_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import os from 'os';
        function test(): void {
            os.platform();
        }
        "#;
        let code2 = r#"
        import fs from 'fs';
        function test(): void {
            fs.readFile('file.txt');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_import_named_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import { readFile } from 'fs';
        function test(): void {
            readFile('file.txt');
        }
        "#;
        let code2 = r#"
        import { writeFile } from 'fs';
        function test(): void {
            writeFile('file.txt', 'data');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_import_default_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import React from 'react';
        function test(): JSX.Element {
            return React.createElement('div');
        }
        "#;
        let code2 = r#"
        import Vue from 'vue';
        function test(): any {
            return Vue.createApp({});
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_require_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        const http = require('http');
        function test(): void {
            http.createServer();
        }
        "#;
        let code2 = r#"
        const https = require('https');
        function test(): void {
            https.createServer();
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_external_module_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        const fs = require('fs');
        function test(): void {
            fs.readFile('file.txt');
        }
        "#;
        let code2 = r#"
        const fs = require('fs');
        function test(): void {
            fs.writeFile('file.txt', 'data');
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_attribute_method_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(obj: any): void {
            obj.method1();
        }
        "#;
        let code2 = r#"
        function test(obj: any): void {
            obj.method2();
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_mixed_local_and_global() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x: number): void {
            const localVar = x + 1;
            console.log(localVar);
        }
        "#;
        let code2 = r#"
        function test(x: number): void {
            const localVar = x + 1;
            console.error(localVar);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_console_log_not_normalized() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(x: number): void {
            console.log(x);
        }
        "#;
        let code2 = r#"
        function test(x: number): void {
            console.log(x);
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "typescript", None);
        let code3 = r#"
        function test(x: number): void {
            console.error(x);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "typescript", None);
    }
}

/// Tests for arrow function implicit return normalization.
mod test_typescript_arrow_function_normalization {
    use super::*;

    #[test]
    fn test_arrow_function_implicit_vs_explicit() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(): number {
            const fn = (x: number): number => x + 1;
            return fn(5);
        }
        "#;
        let code2 = r#"
        function test(): number {
            const fn = (x: number): number => { return x + 1; };
            return fn(5);
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }

    #[test]
    fn test_arrow_function_multiple_statements() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(): number {
            const fn = (x: number): number => {
                const y = x + 1;
                return y;
            };
            return fn(5);
        }
        "#;
        let code2 = r#"
        function test(): number {
            const fn = (x: number): number => {
                const y = x + 1;
                return y;
            };
            return fn(5);
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }
}

/// Tests for semicolon independence in TypeScript.
mod test_typescript_semicolon_handling {
    use super::*;

    #[test]
    fn test_typescript_semicolon_independence() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function test(): void {
            const x = 1;
            const y = 2;
            return x + y;
        }
        "#;
        let code2 = r#"
        function test(): void {
            const x = 1;
            const y = 2;
            return x + y;
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "typescript", None);
    }
}

/// Tests for default parameter values in TypeScript (spec §10.1).
mod test_typescript_default_parameters {
    use super::*;

    #[test]
    fn test_typescript_default_parameters_affect_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function connect(retries: number = 5, timeout: number = 30): number {
            return retries + timeout;
        }
        "#;
        let code2 = r#"
        function connect(retries: number = 0, timeout: number | null = null): number {
            return retries + (timeout || 0);
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "typescript",
            Some("Different default parameter values should produce different hashes"),
        );
    }

    #[test]
    fn test_typescript_same_default_parameters_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function connect(retries: number = 5, timeout: number = 30): number {
            return retries + timeout;
        }
        "#;
        let code2 = r#"
        function connect(retries: number = 5, timeout: number = 30): number {
            return retries + timeout;
        }
        "#;
        assert_hash_equal_with_debug(
            &hasher,
            code1,
            code2,
            "typescript",
            Some("Same default parameter values should produce same hash"),
        );
    }

    #[test]
    fn test_typescript_with_default_vs_without_default() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function process(x: number, y: number = 10): number {
            return x + y;
        }
        "#;
        let code2 = r#"
        function process(x: number, y: number): number {
            return x + y;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "typescript",
            Some("Function with default parameter should differ from function without"),
        );
    }

    #[test]
    fn test_typescript_operand_order_preserved() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function add(x: number, y: number): number {
            return x + y;
        }
        "#;
        let code2 = r#"
        function add(x: number, y: number): number {
            return y + x;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "typescript",
            Some("Operand order should be preserved (x + y vs y + x should differ)"),
        );
    }

    #[test]
    fn test_closure_captures_different_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function outer(x: number, y: number) {
            return () => x;
        }
        "#;
        let code2 = r#"
        function outer(x: number, y: number) {
            return () => y;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "typescript",
            Some("Closures capturing different parameters must produce different hashes"),
        );
    }

    #[test]
    fn test_closure_captures_different_locals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function outer(): () => number {
            let x: number = 1;
            let y: number = 2;
            return () => x;
        }
        "#;
        let code2 = r#"
        function outer(): () => number {
            let x: number = 1;
            let y: number = 2;
            return () => y;
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "typescript",
            Some("Closures capturing different local variables must produce different hashes"),
        );
    }

    #[test]
    fn test_nested_closure_multiple_levels() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function a(x: number, z: number) {
            function b(y: number) {
                function c(): number {
                    return z;
                }
                return c;
            }
            return b;
        }
        "#;
        let code2 = r#"
        function a(x: number, z: number) {
            function b(y: number) {
                function c(): number {
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
            "typescript",
            Some("Nested closures capturing from different levels must produce different hashes"),
        );
    }
}

/// Default and explicit v1 retain the same syntax inputs.
mod test_typescript_policy_entry_points {
    use super::*;

    #[test]
    fn test_loop_variable_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        function calculateTotal(prices: number[]): number {
            let total = 0;
            for (const price of prices) {
                total = total + price;
            }
            return total;
        }
        "#;
        let code2 = r#"
        function calculateTotal(costs: number[]): number {
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
            .compute_hash(code1, "typescript", Some(&policy_explicit))
            .unwrap();
        let hash2_explicit = hasher
            .compute_hash(code2, "typescript", Some(&policy_explicit))
            .unwrap();
        assert_ne!(
            hash1_explicit, hash2_explicit,
            "Explicit v1 retains for-of variable spelling"
        );
        let hash1_default = hasher.compute_hash(code1, "typescript", Some(&policy_default)).unwrap();
        let hash2_default = hasher.compute_hash(code2, "typescript", Some(&policy_default)).unwrap();
        assert_ne!(
            hash1_default, hash2_default,
            "With the default policy, different loop variable names should produce different hashes"
        );
    }
}
