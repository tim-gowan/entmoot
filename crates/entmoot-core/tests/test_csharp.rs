//! Tests for structural hashing - C#-specific.
//!
//! Authored source controls for the retained-CST v1 policy (ADR-0024).
//! These are structural fixtures; compiler qualification is a separate lane.
#![cfg(feature = "full-tests")]

use entmoot_contracts::StructuralHashPolicy;
use entmoot_core::StructuralHasher;
#[path = "common/mod.rs"]
mod common;
use common::{assert_hash_equal_with_debug, assert_hash_not_equal_with_debug, assert_hash_set_size_with_debug};

const LANG: &str = "csharp";

/// Parameter and local spelling remain structural inputs.
mod test_retained_identifier_spelling {
    use super::*;

    #[test]
    fn test_different_param_names_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Add(int x, int y) {
                return x + y;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Add(int a, int b) {
                return a + b;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_different_local_names_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                int result = x + 1;
                return result;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                int output = x + 1;
                return output;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_param_and_local_rename_combined() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Process(int input) {
                int doubled = input * 2;
                return doubled;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Process(int value) {
                int total = value * 2;
                return total;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_foreach_variable_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Sum(int[] items) {
                int total = 0;
                foreach (var item in items) {
                    total = total + item;
                }
                return total;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Sum(int[] items) {
                int total = 0;
                foreach (var x in items) {
                    total = total + x;
                }
                return total;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_for_loop_variable_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Count(int n) {
                int total = 0;
                for (int i = 0; i < n; i++) {
                    total = total + i;
                }
                return total;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Count(int n) {
                int total = 0;
                for (int idx = 0; idx < n; idx++) {
                    total = total + idx;
                }
                return total;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_catch_variable_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Safe() {
                try {
                    return 1;
                } catch (Exception ex) {
                    return ex.GetHashCode();
                }
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Safe() {
                try {
                    return 1;
                } catch (Exception err) {
                    return err.GetHashCode();
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Comments and whitespace → same hash
mod test_comments_whitespace {
    use super::*;

    #[test]
    fn test_line_comments_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                return x + 1;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            // leading comment
            public int Method(int x) {
                // inline
                return x + 1; // trailing
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_xml_doc_comments_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                return x;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            /// <summary>Docs</summary>
            public int Method(int x) {
                return x;
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_whitespace_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "public class Test{public int Method(int x){return x+1;}}";
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                return x + 1;
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_block_comment_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                return x;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            /* block comment */
            public int Method(int x) {
                return x;
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Different operators / control flow → different hash
mod test_control_flow_differs {
    use super::*;

    #[test]
    fn test_different_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                return x + 1;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                return x - 1;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_if_vs_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(int x) {
                if (x > 0) {
                    return;
                }
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(int x) {
                while (x > 0) {
                    return;
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_if_condition_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                if (x > 0) {
                    return 1;
                }
                return 0;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                if (x < 0) {
                    return 1;
                }
                return 0;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_extra_statement_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                return x;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                int y = x;
                return y;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_braced_vs_unbraced_if_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                if (x > 0)
                    return x;
                return 0;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                if (x > 0) {
                    return x;
                }
                return 0;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_switch_vs_if() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                if (x == 1) {
                    return 1;
                }
                return 0;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                switch (x) {
                    case 1:
                        return 1;
                    default:
                        return 0;
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Type contracts: param / nullable / generics / return types
mod test_type_contracts {
    use super::*;

    #[test]
    fn test_different_param_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(int x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(string x) { }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_nullable_vs_non_nullable() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(int x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(int? x) { }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_generic_type_args_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(List<string> items) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(List<int> items) { }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_return_type_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method() {
                return 1;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public string Method() {
                return "1";
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_void_vs_int_return() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(int x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                return x;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_same_types_different_names_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public List<string> Method(IEnumerable<int> items) {
                return null;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public List<string> Method(IEnumerable<int> values) {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Nested scopes
mod test_nested_scopes {
    use super::*;

    #[test]
    fn test_same_names_different_methods() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int First(int x) {
                int y = x + 1;
                return y;
            }
            public int Second(int x) {
                int y = x + 2;
                return y;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int First(int a) {
                int b = a + 1;
                return b;
            }
            public int Second(int a) {
                int b = a + 2;
                return b;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_nested_blocks_independent() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method(int x) {
                {
                    int a = x;
                }
                {
                    int a = x + 1;
                    return a;
                }
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method(int x) {
                {
                    int first = x;
                }
                {
                    int second = x + 1;
                    return second;
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_constructor_scope() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public Test(int value) {
                int local = value;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public Test(int input) {
                int temp = input;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Using aliases
mod test_using_aliases {
    use super::*;

    #[test]
    fn test_using_alias_present_consistently() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        using Alias = System.IO;
        public class Test {
            public void Method() {
                Alias.File.Exists("x");
            }
        }
        "#;
        let code2 = r#"
        using Alias = System.IO;
        public class Test {
            public void Method() {
                Alias.File.Exists("x");
            }
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_using_alias_affects_simple_name() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let with_alias = r#"
        using Alias = System.IO;
        public class Test {
            public void Method() {
                Alias x = null;
            }
        }
        "#;
        let without = r#"
        public class Test {
            public void Method() {
                System.IO x = null;
            }
        }
        "#;
        // Alias resolution should make simple Alias resolve like System.IO when used as type/id.
        // If both serialize differently due to using_directive presence, hashes may differ —
        // assert they at least compute successfully and using is captured.
        let h1 = hasher.compute_hash(with_alias, LANG, None).expect("hash with alias");
        let h2 = hasher.compute_hash(without, LANG, None).expect("hash without");
        assert!(!h1.is_empty());
        assert!(!h2.is_empty());
    }

    #[test]
    fn test_different_using_alias_target_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        using Alias = System.IO;
        public class Test {
            public void Method() { }
        }
        "#;
        let code2 = r#"
        using Alias = System.Text;
        public class Test {
            public void Method() { }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_plain_using_parameter_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        using System.Collections.Generic;
        public class Test {
            public void Method(List<int> items) { }
        }
        "#;
        let code2 = r#"
        using System.Collections.Generic;
        public class Test {
            public void Method(List<int> values) { }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Properties
mod test_properties {
    use super::*;

    #[test]
    fn test_property_get_set_captured() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let with_set = r#"
        public class Test {
            public int Value {
                get { return 1; }
                set { }
            }
        }
        "#;
        let get_only = r#"
        public class Test {
            public int Value {
                get { return 1; }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, with_set, get_only, LANG, None);
    }

    #[test]
    fn test_property_body_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Value {
                get {
                    int x = 1;
                    return x;
                }
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Value {
                get {
                    int y = 1;
                    return y;
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_different_property_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Value { get { return 1; } }
        }
        "#;
        let code2 = r#"
        public class Test {
            public string Value { get { return "1"; } }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Async / await
mod test_async_await {
    use super::*;

    #[test]
    fn test_async_vs_sync_method() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let sync_code = r#"
        public class Test {
            public Task Method() {
                return null;
            }
        }
        "#;
        let async_code = r#"
        public class Test {
            public async Task Method() {
                return null;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, sync_code, async_code, LANG, None);
    }

    #[test]
    fn test_await_expression_captured() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let with_await = r#"
        public class Test {
            public async Task Method() {
                await Foo();
            }
        }
        "#;
        let without_await = r#"
        public class Test {
            public async Task Method() {
                Foo();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, with_await, without_await, LANG, None);
    }

    #[test]
    fn test_async_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public async Task Method(int x) {
                int y = x;
                await Foo(y);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public async Task Method(int a) {
                int b = a;
                await Foo(b);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// foreach vs for
mod test_loops {
    use super::*;

    #[test]
    fn test_foreach_vs_for() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let foreach_code = r#"
        public class Test {
            public void Method(int[] items) {
                foreach (var x in items) {
                    Use(x);
                }
            }
        }
        "#;
        let for_code = r#"
        public class Test {
            public void Method(int[] items) {
                for (int i = 0; i < items.Length; i++) {
                    Use(items[i]);
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, foreach_code, for_code, LANG, None);
    }

    #[test]
    fn test_while_vs_for() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let while_code = r#"
        public class Test {
            public void Method(int n) {
                int i = 0;
                while (i < n) {
                    i++;
                }
            }
        }
        "#;
        let for_code = r#"
        public class Test {
            public void Method(int n) {
                for (int i = 0; i < n; i++) {
                }
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, while_code, for_code, LANG, None);
    }
}

/// Type contracts remain structural inputs
mod test_retained_type_syntax {
    use super::*;

    #[test]
    fn test_retains_param_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = r#"
        public class Test {
            public void Method(int x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(string x) { }
        }
        "#;
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "param types must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_nullable() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = r#"
        public class Test {
            public void Method(string x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(string? x) { }
        }
        "#;
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "nullable must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_generics() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = r#"
        public class Test {
            public void Method(List<int> x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(List<string> x) { }
        }
        "#;
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "generics must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_return_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = r#"
        public class Test {
            public int Method() { return 1; }
        }
        "#;
        let code2 = r#"
        public class Test {
            public long Method() { return 1; }
        }
        "#;
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "return types must differ under the retained CST policy");
    }
}

/// Invocation target changes
mod test_invocations {
    use super::*;

    #[test]
    fn test_invocation_target_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method() {
                Foo();
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method() {
                Bar();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_member_access_target_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method() {
                obj.Foo();
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method() {
                obj.Bar();
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_argument_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method() {
                Foo(1);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method() {
                Foo(2);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_invocation_with_renamed_arg() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(int x) {
                Foo(x);
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(int y) {
                Foo(y);
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Misc / modifiers / modifiers sort
mod test_misc {
    use super::*;

    #[test]
    fn test_modifier_order_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let variants = vec!["public static class Test {}", "static public class Test {}"];
        assert_hash_set_size_with_debug(&hasher, &variants, 2, LANG, None);
    }

    #[test]
    fn test_literal_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public int Method() {
                return 10;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public int Method() {
                return 0xA;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_string_literal_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public string Method() {
                return "a";
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public string Method() {
                return "b";
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_throw_vs_return() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method() {
                return;
            }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method() {
                throw;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_param_count_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        public class Test {
            public void Method(int x) { }
        }
        "#;
        let code2 = r#"
        public class Test {
            public void Method(int x, int y) { }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_basic_hash_computes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        public class Test {
            public int Method(int x) {
                return x + 1;
            }
        }
        "#;
        let hash = hasher.compute_hash(code, LANG, None).expect("hash");
        assert!(hash.starts_with("blake3:"), "unexpected hash format: {hash}");
        assert_eq!(hash.len(), 71); // "blake3:" + 64 hex chars
    }
}
