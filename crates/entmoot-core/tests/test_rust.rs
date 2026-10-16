//! Tests for structural hashing - Rust-specific.
//!
//! Authored source controls for the retained-CST v1 policy (ADR-0024).
//! These are structural fixtures; compiler qualification is a separate lane.
#![cfg(feature = "full-tests")]

use entmoot_contracts::StructuralHashPolicy;
use entmoot_core::StructuralHasher;
#[path = "common/mod.rs"]
mod common;
use common::{assert_hash_equal_with_debug, assert_hash_not_equal_with_debug};

const LANG: &str = "rust";

/// Parameter and local spelling remain structural inputs.
mod test_retained_identifier_spelling {
    use super::*;

    #[test]
    fn test_different_param_names_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn add(x: i32, y: i32) -> i32 {
            x + y
        }
        "#;
        let code2 = r#"
        fn add(a: i32, b: i32) -> i32 {
            a + b
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_different_local_names_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            let result = x + 1;
            result
        }
        "#;
        let code2 = r#"
        fn method(x: i32) -> i32 {
            let output = x + 1;
            output
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_param_and_local_rename_combined() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn process(input: i32) -> i32 {
            let doubled = input * 2;
            doubled
        }
        "#;
        let code2 = r#"
        fn process(value: i32) -> i32 {
            let total = value * 2;
            total
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_for_loop_variable_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn sum(n: i32) -> i32 {
            let mut total = 0;
            for i in 0..n {
                total = total + i;
            }
            total
        }
        "#;
        let code2 = r#"
        fn sum(n: i32) -> i32 {
            let mut total = 0;
            for idx in 0..n {
                total = total + idx;
            }
            total
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_match_arm_binding_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn classify(x: i32) -> i32 {
            match x {
                n if n > 0 => n,
                _ => 0,
            }
        }
        "#;
        let code2 = r#"
        fn classify(x: i32) -> i32 {
            match x {
                v if v > 0 => v,
                _ => 0,
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_tuple_destructure_rename() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn parts() -> i32 {
            let (a, b) = (1, 2);
            a + b
        }
        "#;
        let code2 = r#"
        fn parts() -> i32 {
            let (x, y) = (1, 2);
            x + y
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Ordinary comments are cosmetic; doc attributes are retained.
mod test_comments {
    use super::*;

    #[test]
    fn test_line_comments_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            x + 1
        }
        "#;
        let code2 = r#"
        // leading
        fn method(x: i32) -> i32 {
            // inline
            x + 1 // trailing
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_doc_attributes_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            x
        }
        "#;
        let code2 = r#"
        /// Documentation for method
        fn method(x: i32) -> i32 {
            x
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_block_comment_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            x
        }
        "#;
        let code2 = r#"
        /* block comment */
        fn method(x: i32) -> i32 {
            x
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_whitespace_ignored() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x:i32)->i32{x+1}";
        let code2 = r#"
        fn method(x: i32) -> i32 {
            x + 1
        }
        "#;
        assert_hash_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Different operators / control flow / match → different hash
mod test_control_flow_differs {
    use super::*;

    #[test]
    fn test_different_operators() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) -> i32 { x + 1 }";
        let code2 = "fn method(x: i32) -> i32 { x - 1 }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_if_vs_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) {
            if x > 0 {
                return;
            }
        }
        "#;
        let code2 = r#"
        fn method(x: i32) {
            while x > 0 {
                return;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_if_condition_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            if x > 0 { 1 } else { 0 }
        }
        "#;
        let code2 = r#"
        fn method(x: i32) -> i32 {
            if x < 0 { 1 } else { 0 }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_match_arms_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            match x {
                1 => 1,
                _ => 0,
            }
        }
        "#;
        let code2 = r#"
        fn method(x: i32) -> i32 {
            match x {
                2 => 1,
                _ => 0,
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_match_vs_if() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            if x == 1 { 1 } else { 0 }
        }
        "#;
        let code2 = r#"
        fn method(x: i32) -> i32 {
            match x {
                1 => 1,
                _ => 0,
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_for_vs_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(n: i32) {
            for i in 0..n {}
        }
        "#;
        let code2 = r#"
        fn method(n: i32) {
            let mut i = 0;
            while i < n {
                i += 1;
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_extra_statement_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) -> i32 { x }";
        let code2 = "fn method(x: i32) -> i32 { let y = x; y }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_loop_vs_while() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method() { loop { break; } }";
        let code2 = "fn method() { while true { break; } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Type / lifetime / mut / return contracts
mod test_type_contracts {
    use super::*;

    #[test]
    fn test_different_param_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) {}";
        let code2 = "fn method(x: i64) {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_mut_param_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) {}";
        let code2 = "fn method(mut x: i32) {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_ref_mut_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: &i32) {}";
        let code2 = "fn method(x: &mut i32) {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_lifetime_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method<'a>(x: &'a str) -> &'a str { x }";
        let code2 = "fn method<'b>(x: &'b str) -> &'b str { x }";
        // Lifetimes are contracts retained as written — different lifetime names differ.
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_return_type_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method() -> i32 { 1 }";
        let code2 = "fn method() -> i64 { 1 }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_generic_type_args_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: Vec<i32>) {}";
        let code2 = "fn method(x: Vec<i64>) {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_same_types_different_names_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(items: Vec<i32>) -> i32 { items.len() as i32 }";
        let code2 = "fn method(values: Vec<i32>) -> i32 { values.len() as i32 }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_unit_vs_i32_return() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) {}";
        let code2 = "fn method(x: i32) -> i32 { x }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// impl methods vs free functions
mod test_impl_vs_free {
    use super::*;

    #[test]
    fn test_impl_method_vs_free_function() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let free_fn = "fn bar() -> i32 { 1 }";
        let impl_method = r#"
        impl Foo {
            fn bar() -> i32 { 1 }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, free_fn, impl_method, LANG, None);
    }

    #[test]
    fn test_impl_method_parameter_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        impl Foo {
            fn bar(&self, x: i32) -> i32 {
                x + 1
            }
        }
        "#;
        let code2 = r#"
        impl Foo {
            fn bar(&self, y: i32) -> i32 {
                y + 1
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_self_not_renamed() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        impl Foo {
            fn bar(&self) -> i32 {
                self.get()
            }
            fn get(&self) -> i32 { 1 }
        }
        "#;
        let hash = hasher.compute_hash(code1, LANG, None).expect("hash");
        assert!(hash.starts_with("blake3:"));
    }
}

/// async vs sync
mod test_async {
    use super::*;

    #[test]
    fn test_async_vs_sync() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let sync_code = "fn method() {}";
        let async_code = "async fn method() {}";
        assert_hash_not_equal_with_debug(&hasher, sync_code, async_code, LANG, None);
    }

    #[test]
    fn test_async_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "async fn method(x: i32) -> i32 { let y = x; y }";
        let code2 = "async fn method(a: i32) -> i32 { let b = a; b }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Attributes kept
mod test_attributes {
    use super::*;

    #[test]
    fn test_attribute_vs_none() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let with_attr = "#[inline]\nfn method() {}";
        let without = "fn method() {}";
        assert_hash_not_equal_with_debug(&hasher, with_attr, without, LANG, None);
    }

    #[test]
    fn test_different_attributes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "#[inline]\nfn method() {}";
        let code2 = "#[cold]\nfn method() {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_same_attribute_parameter_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "#[inline]\nfn method(x: i32) -> i32 { x }";
        let code2 = "#[inline]\nfn method(y: i32) -> i32 { y }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_attribute_order_prefer_miss() {
        // Attribute order is not proven cosmetic — prefer miss over false hit.
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "#[inline]\n#[cold]\nfn method() {}";
        let code2 = "#[cold]\n#[inline]\nfn method() {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// use path differences
mod test_use_paths {
    use super::*;

    #[test]
    fn test_different_use_paths_differ() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        use foo::bar;
        fn method() { bar(); }
        "#;
        let code2 = r#"
        use foo::baz;
        fn method() { baz(); }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_use_alias_target_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        use foo::Bar as Alias;
        fn method() {}
        "#;
        let code2 = r#"
        use foo::Baz as Alias;
        fn method() {}
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_use_alias_parameter_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        use foo::Bar as Alias;
        fn method(x: i32) -> i32 { x }
        "#;
        let code2 = r#"
        use foo::Bar as Alias;
        fn method(y: i32) -> i32 { y }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_call_target_change() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method() { foo(); }";
        let code2 = "fn method() { bar(); }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Signature type syntax is retained
mod test_retained_type_syntax {
    use super::*;

    #[test]
    fn test_retains_param_types() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = "fn method(x: i32) {}";
        let code2 = "fn method(x: i64) {}";
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "param types must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_mut() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = "fn method(x: &i32) {}";
        let code2 = "fn method(x: &mut i32) {}";
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "mut must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_lifetimes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = "fn method<'a>(x: &'a str) -> &'a str { x }";
        let code2 = "fn method(x: &str) -> &str { x }";
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "lifetimes must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_return_type() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = "fn method() -> i32 { 1 }";
        let code2 = "fn method() -> u32 { 1 }";
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "return types must differ under the retained CST policy");
    }

    #[test]
    fn test_retains_generics() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let policy = StructuralHashPolicy::v1();

        let code1 = "fn method(x: Vec<i32>) {}";
        let code2 = "fn method(x: Vec<String>) {}";
        let h1 = hasher.compute_hash(code1, LANG, Some(&policy)).unwrap();
        let h2 = hasher.compute_hash(code2, LANG, Some(&policy)).unwrap();
        assert_ne!(h1, h2, "generics must differ under the retained CST policy");
    }
}

/// Nested scopes / shadowing
mod test_nested_scopes {
    use super::*;

    #[test]
    fn test_same_names_different_functions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn first(x: i32) -> i32 {
            let y = x + 1;
            y
        }
        fn second(x: i32) -> i32 {
            let y = x + 2;
            y
        }
        "#;
        let code2 = r#"
        fn first(a: i32) -> i32 {
            let b = a + 1;
            b
        }
        fn second(a: i32) -> i32 {
            let b = a + 2;
            b
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_nested_blocks_independent() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            {
                let a = x;
            }
            {
                let a = x + 1;
                a
            }
        }
        "#;
        let code2 = r#"
        fn method(x: i32) -> i32 {
            {
                let first = x;
            }
            {
                let second = x + 1;
                second
            }
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_shadowing_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        fn method(x: i32) -> i32 {
            let x = x + 1;
            x
        }
        "#;
        let code2 = r#"
        fn method(a: i32) -> i32 {
            let a = a + 1;
            a
        }
        "#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// unsafe retained
mod test_unsafe {
    use super::*;

    #[test]
    fn test_unsafe_vs_safe() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let safe = "fn method() { let u = 1; }";
        let uns = "fn method() { unsafe { let u = 1; } }";
        assert_hash_not_equal_with_debug(&hasher, safe, uns, LANG, None);
    }

    #[test]
    fn test_unsafe_rename_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method() { unsafe { let u = 1; u } }";
        let code2 = "fn method() { unsafe { let v = 1; v } }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}

/// Misc
mod test_misc {
    use super::*;

    #[test]
    fn test_literal_spelling_retained() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method() -> i32 { 10 }";
        let code2 = "fn method() -> i32 { 0xA }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_string_literal_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"fn method() -> &'static str { "a" }"#;
        let code2 = r#"fn method() -> &'static str { "b" }"#;
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_param_count_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) {}";
        let code2 = "fn method(x: i32, y: i32) {}";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_field_access_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(s: S) { s.x; }";
        let code2 = "fn method(s: S) { s.y; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_type_cast_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method(x: i32) { let y = x as u32; }";
        let code2 = "fn method(x: i32) { let y = x as u64; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }

    #[test]
    fn test_basic_hash_computes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = "fn method(x: i32) -> i32 { x + 1 }";
        let hash = hasher.compute_hash(code, LANG, None).expect("hash");
        assert!(hash.starts_with("blake3:"), "unexpected hash format: {hash}");
        assert_eq!(hash.len(), 71);
    }

    #[test]
    fn test_compound_assignment_differs() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = "fn method() { let mut x = 1; x += 1; }";
        let code2 = "fn method() { let mut x = 1; x -= 1; }";
        assert_hash_not_equal_with_debug(&hasher, code1, code2, LANG, None);
    }
}
