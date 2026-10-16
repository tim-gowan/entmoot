//! Tests for structural hashing - Java-specific.
//!
//! Ported from tests/test_java.py (105 tests)
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

/// Tests for StructuralHasher class - Java-specific.
mod test_java_structural_hasher {
    use super::*;

    #[test]
    fn test_hash_java_different_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method(int x) {
                int result = x + 1;
                return result;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method(int y) {
                int output = y + 1;
                return output;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }
}

/// Tests for canonical normalization behaviors on Java input.
mod test_java_canonical_normalization {
    use super::*;

    #[test]
    fn test_java_lang_import_resolution() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_short = r#"
        public class Test {
            public String value() {
                String result = "ok";
                return result;
            }
        }
        "#;
        let code_fully_qualified = r#"
        public class Test {
            public java.lang.String value() {
                java.lang.String result = "ok";
                return result;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code_short, code_fully_qualified, "java", None);
    }

    #[test]
    fn test_this_prefix_normalization() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_this = r#"
        public class Test {
            private int value;
            public int getValue() {
                return this.value;
            }
        }
        "#;
        let code_without_this = r#"
        public class Test {
            private int value;
            public int getValue() {
                return value;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code_with_this, code_without_this, "java", None);
    }

    #[test]
    fn test_implicit_block_normalization() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_without_block = r#"
        public class Test {
            public int maybe(int value) {
                if (value > 0)
                    return value;
                return 0;
            }
        }
        "#;
        let code_with_block = r#"
        public class Test {
            public int maybe(int value) {
                if (value > 0) {
                    return value;
                }
                return 0;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code_without_block, code_with_block, "java", None);
    }

    #[test]
    fn test_modifier_order_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let variants = vec![
            "public static final class Test {}",
            "static public final class Test {}",
            "final public static class Test {}",
        ];
        assert_hash_set_size_with_debug(&hasher, &variants, 3, "java", None);
    }

    #[test]
    fn test_array_style_normalization() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_type_brackets = r#"
        public class Test {
            public void method() {
                int[] values = new int[3];
                values[0] = 1;
            }
        }
        "#;
        let code_identifier_brackets = r#"
        public class Test {
            public void method() {
                int values[] = new int[3];
                values[0] = 1;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code_type_brackets, code_identifier_brackets, "java", None);
    }

    #[test]
    fn test_literal_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_decimal = "public class Test { public int v() { return 10; } }";
        let code_hex = "public class Test { public int v() { return 0xA; } }";
        let code_octal = "public class Test { public int v() { return 012; } }";
        let code_binary = "public class Test { public int v() { return 0b1010; } }";
        let code_underscored = "public class Test { public int v() { return 1_0; } }";
        let codes = vec![code_decimal, code_hex, code_octal, code_binary, code_underscored];
        assert_hash_set_size_with_debug(&hasher, &codes, 5, "java", None);
    }
}

/// Unit tests for StructuralHasher - Java tests.
///
/// These tests validate the implementation for Java,
/// checking retained syntax without claiming semantic equivalence.
mod test_java_unit_tests {
    use super::*;

    // ============================================================================
    // Historical UT-Hash-1.1 corpus: layout and retained syntax controls
    // ============================================================================

    #[test]
    fn test_ut_hash_1_1_java_comments() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { int x = 1; } }";
        let code2 = "// Comment\npublic class Test { public void method() { int x = 1; } }";
        let code3 = "public class Test { /* Block comment */ public void method() { int x = 1; } }";
        let code4 = "/**\n * Javadoc comment\n */\npublic class Test { public void method() { int x = 1; } }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_whitespace() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { int x=1; } }";
        let code2 = "public class Test { public void method() { int x = 1; } }";
        let code3 = "public class Test { public void method() { int x=1; } }";
        let code4 = "public class Test { public void method() { int x = 1; } }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_javadoc() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { int x = 1; } }";
        let code2 = "/**\n * This is a Javadoc comment.\n * @param x the parameter\n */\npublic class Test { public void method() { int x = 1; } }";
        let code3 = "/**\n * Different Javadoc with more details.\n * @return void\n */\npublic class Test { public void method() { int x = 1; } }";
        let code4 = "/**\n * Another Javadoc comment.\n */\npublic class Test { public void method() { int x = 1; } }";
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3, code4], "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_annotations() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { int x = 1; } }";
        let code2 = "@Override\npublic class Test { public void method() { int x = 1; } }";
        let code3 = "@Deprecated\npublic class Test { public void method() { int x = 1; } }";
        let code4 = "@SuppressWarnings(\"unused\")\npublic class Test { public void method() { int x = 1; } }";
        // Annotations are kept (prefer miss) — presence and annotation identity change the hash.
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3, code4], "java", None);
    }

    // ============================================================================
    // Historical UT-Hash-1.2 corpus: retained identifier spelling
    // ============================================================================

    #[test]
    fn test_ut_hash_1_2_java_method_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public int method(int x) { return x; } }";
        let code2 = "public class Test { public int method(int y) { return y; } }";
        let code3 = "public class Test { public int method(int param) { return param; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_local_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { int x = 1; int y = x + 1; } }";
        let code2 = "public class Test { public void method() { int a = 1; int b = a + 1; } }";
        let code3 = "public class Test { public void method() { int value = 1; int result = value + 1; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_nested_scopes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(int x) { if (x > 0) { int y = x + 1; } } }";
        let code2 = "public class Test { public void method(int a) { if (a > 0) { int b = a + 1; } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_multiple_params() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public int add(int x, int y) { return x + y; } }";
        let code2 = "public class Test { public int add(int a, int b) { return a + b; } }";
        let code3 = "public class Test { public int add(int first, int second) { return first + second; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_scope_isolation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 =
            "public class Test { public void method1(int x) { return x; } public void method2(int x) { return x; } }";
        let code2 =
            "public class Test { public void method1(int a) { return a; } public void method2(int a) { return a; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_block_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { int x = 1; if (true) { int y = 2; } return x; } }";
        let code2 = "public class Test { public void method() { int a = 1; if (true) { int b = 2; } return a; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_class_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(int x) { return x; } }";
        let code2 = "public class Test { public void method(int y) { return y; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_control_flow_variables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public int method(int x) { if (x > 0) { return x; } else { return 0; } } }";
        let code2 = "public class Test { public int method(int y) { if (y > 0) { return y; } else { return 0; } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_2_java_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { String str = \"test\"; int len = str.length(); } }";
        let code2 = "public class Test { public void method() { String s = \"test\"; int l = s.length(); } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    // ============================================================================
    // UT-Hash-1.3: Semantic Variance
    // ============================================================================

    #[test]
    fn test_ut_hash_1_3_java_loop_bound() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { for (int i = 0; i < 10; i++) {} } }";
        let code2 = "public class Test { public void method() { for (int i = 0; i < 20; i++) {} } }";
        let code3 = "public class Test { public void method() { for (int i = 0; i < 5; i++) {} } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_comparison_operator() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public boolean method(int x, int y) { return x < y; } }";
        let code2 = "public class Test { public boolean method(int x, int y) { return x > y; } }";
        let code3 = "public class Test { public boolean method(int x, int y) { return x == y; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_computation_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public int method(int y) { int x = y + 1; return x; } }";
        let code2 = "public class Test { public int method(int y) { int x = y + 2; return x; } }";
        let code3 = "public class Test { public int method(int y) { int x = y * 2; return x; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_switch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(int x) { switch(x) { case 1: break; default: break; } } }";
        let code2 = "public class Test { public void method(int x) { switch(x) { case 1: break; case 2: break; default: break; } } }";
        let code3 = "public class Test { public void method(int x) { if (x == 1) {} } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_try_catch() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { try { int x = 1; } catch (Exception e) {} } }";
        let code2 = "public class Test { public void method() { try { int x = 1; } catch (RuntimeException e) {} } }";
        let code3 = "public class Test { public void method() { try { int x = 1; } finally {} } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_throw() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { throw new Exception(); } }";
        let code2 = "public class Test { public void method() { throw new RuntimeException(); } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_assert() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(int x) { assert x > 0; } }";
        let code2 = "public class Test { public void method(int x) { assert x < 0; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_break_continue() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { for (int i = 0; i < 10; i++) { if (i > 5) break; } } }";
        let code2 =
            "public class Test { public void method() { for (int i = 0; i < 10; i++) { if (i > 5) continue; } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_do_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { do { x++; } while (x < 10); } }";
        let code2 = "public class Test { public void method() { do { y++; } while (y < 10); } }";
        let code3 = "public class Test { public void method() { while (x < 10) { x++; } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_enhanced_for() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { for (int x : list) { } } }";
        let code2 = "public class Test { public void method() { for (int y : list) { } } }";
        let code3 = "public class Test { public void method() { for (int i = 0; i < list.length; i++) { } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_null_literal() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public Object method() { return null; } }";
        let code2 = "public class Test { public Object method() { return (null); } }";
        let code3 = "public class Test { public Object method() { return 0; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { while (x > 0) { x--; } } }";
        let code2 = "public class Test { public void method() { while (x < 0) { x++; } } }";
        let code3 = "public class Test { public void method() { for (int i = 0; i < 10; i++) {} } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_logical_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public boolean method(boolean x, boolean y) { return x && y; } }";
        let code2 = "public class Test { public boolean method(boolean x, boolean y) { return x || y; } }";
        let code3 = "public class Test { public boolean method(boolean x) { return !x; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_membership_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public boolean method(Object x) { return x instanceof String; } }";
        let code2 = "public class Test { public boolean method(Object x) { return x instanceof Integer; } }";
        let code3 = "public class Test { public boolean method(Object x) { return x == null; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_compound_assignment() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(int x) { x += 1; } }";
        let code2 = "public class Test { public void method(int x) { x -= 1; } }";
        let code3 = "public class Test { public void method(int x) { x *= 2; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_external_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { String str = \"test\"; int len = str.length(); } }";
        let code2 =
            "public class Test { public void method() { String str = \"test\"; String upper = str.toUpperCase(); } }";
        let code3 =
            "public class Test { public void method() { String str = \"test\"; boolean empty = str.isEmpty(); } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_boolean_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public boolean method() { return true; } }";
        let code2 = "public class Test { public boolean method() { return false; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_string_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public String method() { return \"hello\"; } }";
        let code2 = "public class Test { public String method() { return \"world\"; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_integer_literals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public int method() { return 1; } }";
        let code2 = "public class Test { public int method() { return 2; } }";
        let code3 = "public class Test { public int method() { return 100; } }";
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    // ============================================================================
    // UT-Hash-1.x: Edge Cases & Modern Java Features
    // ============================================================================

    #[test]
    fn test_ut_hash_1_2_java_inner_class_renaming() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Outer { class Inner { void method(int x) { } } }";
        let code2 = "public class Outer { class Inner { void method(int y) { } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_qualified_method_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 =
            "public class Test { public void method() { String str = java.util.Arrays.toString(new int[]{1}); } }";
        let code2 =
            "public class Test { public void method() { String s = java.util.Arrays.toString(new int[]{1}); } }";
        let code3 = "public class Test { public void method() { String str = java.util.Collections.emptyList(); } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_static_imports() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "import static java.lang.Math.PI; public class Test { public double method() { return PI; } }";
        let code2 = "import static java.lang.Math.PI; public class Test { public double method() { return PI; } }";
        let code3 = "public class Test { public double method() { return java.lang.Math.PI; } }";
        assert_hash_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_annotations_kept() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "@Override\npublic void method() { }";
        let code2 = "@Deprecated\npublic void method() { }";
        let code3 = "public void method() { }";
        // Different annotations (and absence) must differ — prefer miss over false hit.
        assert_hashes_not_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_generics_pruned() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        // Type parameter spelling and concrete types remain distinct.
        let code1 = "public class Test<T> { public T method() { return null; } }";
        let code2 = "public class Test<U> { public U method() { return null; } }";
        let code3 = "public class Test { public Object method() { return null; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_goto_label() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { label: if (true) break label; } }";
        let code2 = "public class Test { public void method() { label: if (true) break label; } }";
        let code3 = "public class Test { public void method() { if (true) break; } }";
        assert_hash_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_lambda_expressions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { Runnable r = () -> System.out.println(\"test\"); } }";
        let code2 = "public class Test { public void method() { Runnable r = () -> System.out.println(\"other\"); } }";
        let code3 = "public class Test { public void method() { Runnable r = new Runnable() { public void run() { System.out.println(\"test\"); } }; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_method_references() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { Runnable r = Test::staticMethod; } }";
        let code2 = "public class Test { public void method() { Runnable r = Test::otherMethod; } }";
        let code3 = "public class Test { public void method() { Runnable r = () -> Test.staticMethod(); } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_1_java_var_keyword() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { var x = 1; } }";
        let code2 = "public class Test { public void method() { var y = 1; } }";
        let code3 = "public class Test { public void method() { int x = 1; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        let hash1 = hasher.compute_hash(code1, "java", None).unwrap();
        let hash3 = hasher.compute_hash(code3, "java", None).unwrap();
        assert_ne!(hash1, hash3); // Inferred and explicit type syntax remain distinct.
    }

    #[test]
    fn test_ut_hash_1_3_java_text_blocks() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { String text = \"\"\"\nHello\nWorld\n\"\"\"; } }";
        let code2 = "public class Test { public void method() { String text = \"\"\"\nGoodbye\nWorld\n\"\"\"; } }";
        let code3 = "public class Test { public void method() { String text = \"Hello\\nWorld\"; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_switch_expressions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 =
            "public class Test { public int method(int x) { return switch(x) { case 1 -> 1; default -> 0; }; } }";
        let code2 = "public class Test { public int method(int x) { return switch(x) { case 1 -> 1; case 2 -> 2; default -> 0; }; } }";
        let code3 = "public class Test { public int method(int x) { switch(x) { case 1: return 1; default: return 0; } return 0; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_records() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public record Test(int x, int y) { }";
        let code2 = "public record Test(int a, int b) { }";
        let code3 = "public class Test { private final int x; private final int y; public Test(int x, int y) { this.x = x; this.y = y; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    // ============================================================================
    // UT-Hash-1.4: Cross-Language Isolation
    // ============================================================================

    #[test]
    fn test_ut_hash_1_4_java_simple_loop() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_java = "public class Test { public void method() { for(int i=0; i<10; i++) { } } }";
        let code_py = "def method():\n    for i in range(10):\n        pass";
        let hash_java = hasher.compute_hash(code_java, "java", None).unwrap();
        let hash_py = hasher.compute_hash(code_py, "python", None).unwrap();
        assert_ne!(
            hash_java, hash_py,
            "Java and Python loops should produce different hashes"
        );
    }

    #[test]
    fn test_ut_hash_1_4_java_function_addition() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_java = "public class Test { public int add(int x, int y) { return x + y; } }";
        let code_py = "def add(x, y):\n    return x + y";
        let hash_java = hasher.compute_hash(code_java, "java", None).unwrap();
        let hash_py = hasher.compute_hash(code_py, "python", None).unwrap();
        assert_ne!(hash_java, hash_py, "Java and Python hashes unexpectedly match");
    }

    #[test]
    fn test_ut_hash_1_4_java_conditional() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_java = "public class Test { public void check(int x) { if (x > 10) { return; } } }";
        let code_py = "def check(x):\n    if x > 10:\n        return";
        let hash_java = hasher.compute_hash(code_java, "java", None).unwrap();
        let hash_py = hasher.compute_hash(code_py, "python", None).unwrap();
        assert_ne!(hash_java, hash_py, "Java and Python hashes unexpectedly match");
    }

    // ============================================================================
    // UT-Hash-1.5: Structural Completeness Verification
    // ============================================================================

    #[test]
    fn test_ut_hash_1_5_java_control_flow_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_if = "public class Test { public void method(int x) { if (x > 0) {} } }";
        let code_switch = "public class Test { public void method(int x) { switch(x) { case 1: break; } } }";
        let code_try = "public class Test { public void method() { try {} catch (Exception e) {} } }";
        let code_throw = "public class Test { public void method() { throw new Exception(); } }";
        let code_sync = "public class Test { public void method(Object o) { synchronized(o) {} } }";
        let codes = vec![code_if, code_switch, code_try, code_throw, code_sync];
        assert_hash_set_size_with_debug(
            &hasher,
            &codes,
            5,
            "java",
            Some("All control flow statements should produce distinct hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_java_synchronized_block() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(Object o) { synchronized(o) { int x = 1; } } }";
        let code2 = "public class Test { public void method(Object lock) { synchronized(lock) { int x = 1; } } }";
        let code3 = "public class Test { public void method(Object o) { { int x = 1; } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_5_java_try_with_resources() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 =
            "public class Test { public void method() { try (AutoCloseable r = null) { } catch (Exception e) {} } }";
        let code2 =
            "public class Test { public void method() { try (AutoCloseable res = null) { } catch (Exception e) {} } }";
        let code3 =
            "public class Test { public void method() { try { AutoCloseable r = null; } catch (Exception e) {} } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_1_5_java_operator_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_add = "public class Test { public int method(int x, int y) { return x + y; } }";
        let code_sub = "public class Test { public int method(int x, int y) { return x - y; } }";
        let code_mul = "public class Test { public int method(int x, int y) { return x * y; } }";
        let code_and = "public class Test { public boolean method(boolean x, boolean y) { return x && y; } }";
        let code_or = "public class Test { public boolean method(boolean x, boolean y) { return x || y; } }";
        let code_eq = "public class Test { public boolean method(int x, int y) { return x == y; } }";
        let codes = vec![code_add, code_sub, code_mul, code_and, code_or, code_eq];
        assert_hash_set_size_with_debug(
            &hasher,
            &codes,
            6,
            "java",
            Some("All operators should produce distinct hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_java_external_call_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method() { String str = \"test\"; int len = str.length(); } }";
        let code2 =
            "public class Test { public void method() { String str = \"test\"; String upper = str.toUpperCase(); } }";
        let code3 = "public class Test { public void method() { String str = \"test\"; int len = str.length(); } }";
        assert_hash_equal_with_debug(&hasher, code1, code3, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_ut_hash_1_5_java_scope_isolation_coverage() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_nested = "public class Outer { class Inner { void method(int x) { } } }";
        let code_block = "public class Test { public void method() { int x = 1; if (true) { int y = 2; } return x; } }";
        let code_class = "public class Test { public void method(int x) { return x; } }";
        assert_hashes_not_equal_with_debug(
            &hasher,
            &[code_nested, code_block, code_class],
            "java",
            Some("Different scope structures should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_1_5_java_unused_imports() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_unused_import =
            "import java.util.List;\n\npublic class Test {\n    public int method() {\n        return 42;\n    }\n}\n";
        let code_without_import = "public class Test {\n    public int method() {\n        return 42;\n    }\n}\n";
        let code_different_unused_import =
            "import java.util.Map;\n\npublic class Test {\n    public int method() {\n        return 42;\n    }\n}\n";
        assert_hash_not_equal_with_debug(
            &hasher,
            code_with_unused_import,
            code_without_import,
            "java",
            Some("Unused import should affect hash"),
        );
        assert_hash_not_equal_with_debug(
            &hasher,
            code_with_unused_import,
            code_different_unused_import,
            "java",
            Some("Different unused imports should produce different hashes"),
        );
    }

    #[test]
    fn test_ut_hash_java_comprehensive_invariance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public int method(int x, int y) { int result = x + y; if (result > 10) { String str = \"test\"; int len = str.length(); return len; } else { return result; } } }";
        let code2 = "/**\n * This is a comprehensive Javadoc comment.\n * @param x the first integer parameter\n * @param y the second integer parameter\n * @return the result of computation\n */\npublic class Test { // This method performs addition and conditional logic\npublic int method(int x, int y) { /* Calculate the sum */\nint result = x + y; // Check if result exceeds threshold\nif (result > 10) { /* Create string and get length */\nString str = \"test\"; int len = str.length(); return len; } else { // Return the sum directly\nreturn result; } } }";
        let code3 = "public class Test{public int method(int x,int y){int result=x+y;if(result>10){String str=\"test\";int len=str.length();return len;}else{return result;}}}";
        let code4 = "public class Test { public int method(int a, int b) { int sum = a + b; if (sum > 10) { String text = \"test\"; int length = text.length(); return length; } else { return sum; } } }";
        let code5 = "@Override\n@SuppressWarnings(\"unused\")\npublic class Test { @Deprecated\npublic int method(int x, int y) { int result = x + y; if (result > 10) { String str = \"test\"; int len = str.length(); return len; } else { return result; } } }";
        // Only comments and layout are cosmetic; names and annotations remain.
        assert_hashes_equal_with_debug(&hasher, &[code1, code2, code3], "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code4, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code5, "java", None);
    }

    #[test]
    fn test_ut_hash_1_3_java_pattern_matching() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test { public void method(Object x) { if (x instanceof String s) { int len = s.length(); } } }";
        let code2 = "public class Test { public void method(Object x) { if (x instanceof String str) { int len = str.length(); } } }";
        let code3 = "public class Test { public void method(Object x) { if (x instanceof String) { String s = (String) x; int len = s.length(); } } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }

    #[test]
    fn test_ut_hash_java_expression_normalization_single_use() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_var = "public class Test { public int method() { String str = \"test\"; return str.length(); } }";
        let code_direct = "public class Test { public int method() { return \"test\".length(); } }";
        let hash_with_var = hasher.compute_hash(code_with_var, "java", None).unwrap();
        let hash_direct = hasher.compute_hash(code_direct, "java", None).unwrap();
        assert_ne!(
            hash_with_var, hash_direct,
            "Temporary declarations remain structural inputs"
        );
    }

    #[test]
    fn test_ut_hash_java_expression_normalization_multi_use() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_multi_use = "public class Test { public int method() { String str = \"test\"; int len1 = str.length(); int len2 = str.length(); return len1 + len2; } }";
        let hash_result = hasher.compute_hash(code_multi_use, "java", None).unwrap();
        assert!(hash_result.starts_with("blake3:"));
    }

    #[test]
    fn test_ut_hash_java_expression_normalization_scope_aware() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_shadowing = "public class Test { public int method() { String str = \"outer\"; if (true) { String str = \"inner\"; return str.length(); } return str.length(); } }";
        let hash_result = hasher.compute_hash(code_shadowing, "java", None).unwrap();
        assert!(hash_result.starts_with("blake3:"));
    }

    #[test]
    fn test_ut_hash_java_expression_normalization_safe_to_inline() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_with_method =
            "public class Test { public int method() { String str = getString(); return str.length(); } }";
        let code_with_literal =
            "public class Test { public int method() { String str = \"test\"; return str.length(); } }";
        let hash_with_method = hasher.compute_hash(code_with_method, "java", None).unwrap();
        let hash_with_literal = hasher.compute_hash(code_with_literal, "java", None).unwrap();
        assert_ne!(hash_with_method, hash_with_literal, "Method call should not be inlined");
    }
}

/// Tests for scope analysis and global function preservation in Java.
mod test_java_scope_analysis {
    use super::*;

    #[test]
    fn test_global_function_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method(int x) {
                System.out.println(x);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method(int x) {
                System.err.println(x);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_builtin_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int method(int a, int b) {
                return Math.max(a, b);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int method(int a, int b) {
                return Math.min(a, b);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_import_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import java.util.List;
        public class Test {
            public void method() {
                List<String> list = new ArrayList<>();
            }
        }
        "#;
        let code2 = r#"
        import java.util.Map;
        public class Test {
            public void method() {
                Map<String, String> map = new HashMap<>();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_static_import_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import static java.lang.Math.PI;
        public class Test {
            public double method() {
                return PI;
            }
        }
        "#;
        let code2 = r#"
        import static java.lang.Math.E;
        public class Test {
            public double method() {
                return E;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_external_class_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public String method(int x) {
                return String.valueOf(x);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public String method(int x) {
                return Integer.toString(x);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_attribute_method_preservation() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method(Object obj) {
                obj.method1();
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method(Object obj) {
                obj.method2();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_mixed_local_and_global() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method(int x) {
                int localVar = x + 1;
                System.out.println(localVar);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method(int x) {
                int localVar = x + 1;
                System.err.println(localVar);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_package_qualified_calls() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method() {
                java.util.Arrays.asList(1, 2, 3);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method() {
                java.util.Collections.emptyList();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_system_out_not_normalized() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method(int x) {
                System.out.println(x);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method(int x) {
                System.out.println(x);
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "java", None);
        let code3 = r#"
        public class Test {
            public void method(int x) {
                System.err.println(x);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code3, "java", None);
    }
}

/// Tests for Java type preservation in method signatures.
mod test_java_type_preservation {
    use super::*;

    #[test]
    fn test_java_method_overloading_distinction() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(User u) {
                System.out.println(u);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(Order o) {
                System.out.println(o);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_java_type_precision_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public double calculate(int value) {
                return value * 1.0;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public double calculate(double value) {
                return value * 1.0;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_java_same_types_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(String name) {
                System.out.println(name);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(String value) {
                System.out.println(value);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_java_type_hashing_parameter_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(User u) {
                System.out.println(u);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(Order o) {
                System.out.println(o);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_java_type_hashing_primitive_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public double calculate(int value) {
                return value * 1.0;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public double calculate(double value) {
                return value * 1.0;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_java_unqualified_class_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(String s) {
                System.out.println(s);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(Integer i) {
                System.out.println(i);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different unqualified class types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_unqualified_class_types_same_type_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(String name) {
                System.out.println(name);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(String value) {
                System.out.println(value);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Same unqualified class type with different parameter names must remain distinct"),
        );
    }

    #[test]
    fn test_java_return_type_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public String method() {
                return null;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public Integer method() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different return types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_return_type_same_type_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public String getValue() {
                return null;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public String createValue() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different method names should produce different hashes"),
        );
    }

    #[test]
    fn test_java_return_type_primitive_vs_wrapper() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int method() {
                return 0;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public Integer method() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Primitive vs wrapper return types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_primitive_vs_wrapper_parameter() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(int x) {
                System.out.println(x);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(Integer x) {
                System.out.println(x);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Primitive vs wrapper parameter types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_array_dimension_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(int[] x) {
                System.out.println(x.length);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(int[][] x) {
                System.out.println(x.length);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different array dimensions should produce different hashes"),
        );
    }

    #[test]
    fn test_java_array_dimension_same_dimension_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(int[] values) {
                System.out.println(values.length);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(int[] array) {
                System.out.println(array.length);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Same array dimension with different parameter names must remain distinct"),
        );
    }

    #[test]
    fn test_java_array_dimension_return_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int[] method() {
                return null;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int[][] method() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different array dimension return types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_array_dimension_generic() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import java.util.List;
        
        public class Test {
            public void process(List<String>[] items) {
                System.out.println(items.length);
            }
        }
        "#;
        let code2 = r#"
        import java.util.List;
        
        public class Test {
            public void process(List<String>[][] items) {
                System.out.println(items.length);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different array dimensions with generics should produce different hashes"),
        );
    }
}

/// Tests for semicolon independence in Java.
mod test_java_semicolon_handling {
    use super::*;

    #[test]
    fn test_java_semicolon_independence() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void method() {
                int x = 1;
                int y = 2;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void method() {
                int x = 1;
                int y = 2;
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, "java", None);
    }

    #[test]
    fn test_java_operand_order_preserved() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int add(int x, int y) {
                return x + y;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int add(int x, int y) {
                return y + x;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Operand order should be preserved (x + y vs y + x should differ)"),
        );
    }
}

/// Tests for qualified type extraction in Java (spec §10.2).
///
/// Critical requirement: Must extract qualified type names from parameter declarations.
/// Missing qualified type extraction leads to false cache hits when interface contracts change.
mod test_java_qualified_type_extraction {
    use super::*;

    #[test]
    fn test_java_qualified_interface_type_parameter() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(java.io.Reader r) {
                r.read();
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(java.io.Writer w) {
                w.write(0);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different qualified interface types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_qualified_type_same_interface_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(java.io.Reader r) {
                r.read();
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(java.io.Reader reader) {
                reader.read();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Same interface type with different parameter names must remain distinct"),
        );
    }

    #[test]
    fn test_java_qualified_return_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public java.io.Reader getReader() {
                return null;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public java.io.Writer getWriter() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different qualified return types should produce different hashes"),
        );
    }

    #[test]
    fn test_java_qualified_return_type_same_interface_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public java.io.Reader getReader() {
                return null;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public java.io.Reader createReader() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different method names should produce different hashes"),
        );
    }

    #[test]
    fn test_java_generic_type_parameter_variance() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import java.util.List;

        public class Test {
            public void process(List<String> items) {
                items.size();
            }
        }
        "#;
        let code2 = r#"
        import java.util.List;

        public class Test {
            public void process(List<Integer> items) {
                items.size();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different generic type parameters should produce different hashes"),
        );
    }

    #[test]
    fn test_java_generic_type_parameter_same_type_same_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        import java.util.List;

        public class Test {
            public void process(List<String> items) {
                items.size();
            }
        }
        "#;
        let code2 = r#"
        import java.util.List;

        public class Test {
            public void process(List<String> list) {
                list.size();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Same generic type with different parameter names must remain distinct"),
        );
    }

    #[test]
    fn test_java_generic_type_parameter_qualified() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(java.util.List<String> items) {
                items.size();
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(java.util.List<Integer> items) {
                items.size();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Different qualified generic type parameters should produce different hashes"),
        );
    }

    #[test]
    fn test_closure_captures_different_parameters() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public java.util.function.Supplier<Integer> outer(int x, int y) {
                return () -> x;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public java.util.function.Supplier<Integer> outer(int x, int y) {
                return () -> y;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Closures capturing different parameters must produce different hashes"),
        );
    }

    #[test]
    fn test_closure_captures_different_locals() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public java.util.function.Supplier<Integer> outer() {
                int x = 1;
                int y = 2;
                return () -> x;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public java.util.function.Supplier<Integer> outer() {
                int x = 1;
                int y = 2;
                return () -> y;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Closures capturing different local variables must produce different hashes"),
        );
    }

    #[test]
    fn test_nested_closure_multiple_levels() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public java.util.function.Function<Integer, java.util.function.Supplier<Integer>> a(int x, int z) {
                return (Integer y) -> () -> z;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public java.util.function.Function<Integer, java.util.function.Supplier<Integer>> a(int x, int z) {
                return (Integer y) -> () -> y;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(
            &hasher,
            code1,
            code2,
            "java",
            Some("Nested closures capturing from different levels must produce different hashes"),
        );
    }
}

/// Default and explicit v1 retain the same syntax inputs.
mod test_java_policy_entry_points {
    use super::*;

    #[test]
    fn test_loop_variable_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Calculator {
            public int calculateTotal(List<Integer> prices) {
                int total = 0;
                for (Integer price : prices) {
                    total = total + price;
                }
                return total;
            }
        }
        "#;
        let code2 = r#"
        public class Calculator {
            public int calculateTotal(List<Integer> costs) {
                int sum = 0;
                for (Integer cost : costs) {
                    sum = sum + cost;
                }
                return sum;
            }
        }
        "#;
        let policy_explicit = StructuralHashPolicy::v1();
        let policy_default = StructuralHashPolicy::default();
        let hash1_explicit = hasher.compute_hash(code1, "java", Some(&policy_explicit)).unwrap();
        let hash2_explicit = hasher.compute_hash(code2, "java", Some(&policy_explicit)).unwrap();
        assert_ne!(
            hash1_explicit, hash2_explicit,
            "Explicit v1 retains enhanced-for variable spelling"
        );
        let hash1_default = hasher.compute_hash(code1, "java", Some(&policy_default)).unwrap();
        let hash2_default = hasher.compute_hash(code2, "java", Some(&policy_default)).unwrap();
        assert_ne!(
            hash1_default, hash2_default,
            "With the default policy, different loop variable names should produce different hashes"
        );
    }

    #[test]
    fn test_type_syntax_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void process(String name) {
                System.out.println(name);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void process(Integer id) {
                System.out.println(id);
            }
        }
        "#;
        let policy_explicit = StructuralHashPolicy::v1();
        let policy_default = StructuralHashPolicy::default();
        let hash1_explicit = hasher.compute_hash(code1, "java", Some(&policy_explicit)).unwrap();
        let hash2_explicit = hasher.compute_hash(code2, "java", Some(&policy_explicit)).unwrap();
        assert_ne!(hash1_explicit, hash2_explicit, "Explicit v1 retains type syntax");
        let hash1_default = hasher.compute_hash(code1, "java", Some(&policy_default)).unwrap();
        let hash2_default = hasher.compute_hash(code2, "java", Some(&policy_default)).unwrap();
        assert_ne!(hash1_default, hash2_default, "Default policy retains type syntax");
        assert_eq!(hash1_explicit, hash1_default, "Default and explicit v1 agree");
        assert_eq!(hash2_explicit, hash2_default, "Default and explicit v1 agree");
    }
}
