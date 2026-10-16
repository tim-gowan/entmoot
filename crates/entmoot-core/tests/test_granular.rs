//! Tests for granular (callable-scoped) hashing.
//!
//! Python suite ported from tests/test_granular.py; other languages are greenfield TDD.
//!
//! Authored source controls for the retained-CST v1 policy (ADR-0024).
//! These are structural fixtures; compiler qualification is a separate lane.
#![cfg(feature = "full-tests")]

use entmoot_core::granular::{compute_merkle_hash, compute_module_shell_hash, CallableHash};
use entmoot_core::{StructuralHashPolicy, StructuralHasher};
#[path = "common/mod.rs"]
mod common;

mod test_granular_hash_basic {
    use super::*;

    #[test]
    fn test_granular_hash_extracts_callables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        def foo(x):
            return x + 1
        
        def bar(y):
            return y * 2
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert_eq!(result.callables.len(), 2);
        assert!(result.callables.contains_key("foo"));
        assert!(result.callables.contains_key("bar"));
        assert_eq!(result.callables["foo"].node_type, "function");
        assert_eq!(result.callables["bar"].node_type, "function");
    }

    #[test]
    fn test_granular_hash_tracks_imports_per_callable() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        import os
        
        def uses_os():
            return os.path.join('a', 'b')
        
        def no_imports():
            return 42
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert!(result.callables.contains_key("uses_os"));
        assert!(result.callables.contains_key("no_imports"));
        assert!(
            result.callables["uses_os"].imports.contains(&"os".to_string())
                || result.imports.contains(&"os".to_string())
        );
        assert!(!result.callables["no_imports"].imports.contains(&"os".to_string()));
    }

    #[test]
    fn test_granular_hash_preserves_parameter_spelling_but_excludes_declared_name() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def foo(x):
            return x + 1
        "#;
        let code2 = r#"
        def bar(y):
            return y + 1
        "#;
        let result1 = hasher.compute_granular_hash(code1, "python", None).unwrap();
        let result2 = hasher.compute_granular_hash(code2, "python", None).unwrap();

        assert_ne!(
            result1.callables["foo"].structural_hash,
            result2.callables["bar"].structural_hash
        );
        let renamed = hasher
            .compute_granular_hash(&code1.replace("foo", "bar"), "python", None)
            .unwrap();
        assert_eq!(
            result1.callables["foo"].structural_hash,
            renamed.callables["bar"].structural_hash
        );
        assert_ne!(result1.callables["foo"].address_id, renamed.callables["bar"].address_id);
    }

    #[test]
    fn test_granular_hash_different_callable_different_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code1 = r#"
        def foo(x):
            return x + 1
        "#;
        let code2 = r#"
        def foo(x):
            return x * 2
        "#;
        let result1 = hasher.compute_granular_hash(code1, "python", None).unwrap();
        let result2 = hasher.compute_granular_hash(code2, "python", None).unwrap();

        assert_ne!(
            result1.callables["foo"].structural_hash,
            result2.callables["foo"].structural_hash
        );
    }

    #[test]
    fn test_granular_hash_merkle_tree_structure() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        def foo(): return 1
        def bar(): return 2
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert!(result.file_hash.starts_with("blake3:"));
        assert_eq!(result.callables.len(), 2);
        let (debug_hash, debug) = hasher.compute_hash_with_debug(code, "python").unwrap();
        assert_eq!(debug_hash, result.file_hash);
        assert_eq!(
            result.module_shell_hash,
            compute_module_shell_hash(&debug["serialized_ir"])
        );

        let callable_list: Vec<CallableHash> = result.callables.values().cloned().collect();
        let expected_hash = compute_merkle_hash(&callable_list, &result.imports, &result.module_shell_hash);
        assert_eq!(result.file_hash, expected_hash);
    }

    #[test]
    fn test_compute_hash_matches_granular_file_hash() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = "def foo(x):\n    return x + 1\n";
        let granular = hasher.compute_granular_hash(code, "python", None).unwrap();
        let file = hasher.compute_hash(code, "python", None).unwrap();
        assert_eq!(file, granular.file_hash);
        assert!(!granular.callables.is_empty());
    }

    #[test]
    fn test_granular_hash_with_classes() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        class MyClass:
            def method1(self):
                return 1
            
            def method2(self):
                return 2
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert!(result.callables.contains_key("MyClass") || !result.callables.is_empty());
    }

    #[test]
    fn test_granular_hash_file_level_imports() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        import os
        import sys
        
        def foo():
            return os.path.join('a', 'b')
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert!(
            result.imports.contains(&"os".to_string())
                || result
                    .callables
                    .get("foo")
                    .map(|c| c.imports.contains(&"os".to_string()))
                    .unwrap_or(false)
        );
    }

    #[test]
    fn test_granular_hash_nested_functions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        def outer(x):
            def inner(y):
                return y + 1
            return inner(x)
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert!(result.callables.contains_key("outer"));
    }

    #[test]
    fn test_granular_hash_import_from() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        from os import path
        
        def uses_path():
            return path.join('a', 'b')
        "#;
        let result = hasher.compute_granular_hash(code, "python", None).unwrap();

        assert!(result.callables.contains_key("uses_path"));
        assert!(
            result.imports.contains(&"path".to_string())
                || result.imports.contains(&"os.path".to_string())
                || result.callables["uses_path"].imports.contains(&"path".to_string())
        );
    }
}

mod test_module_shell {
    use super::*;

    #[test]
    fn test_module_shell_distinguishes_top_level_only_files() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let r1 = hasher.compute_granular_hash("x = 1\n", "python", None).unwrap();
        let r2 = hasher.compute_granular_hash("y = 2\n", "python", None).unwrap();

        assert!(r1.callables.is_empty());
        assert!(r2.callables.is_empty());
        assert_ne!(r1.module_shell_hash, r2.module_shell_hash);
        assert_ne!(r1.file_hash, r2.file_hash);
    }

    #[test]
    fn test_module_shell_change_with_stable_callables() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code_a = "TOP = 1\ndef foo():\n    return 1\n";
        let code_b = "TOP = 2\ndef foo():\n    return 1\n";

        let r1 = hasher.compute_granular_hash(code_a, "python", None).unwrap();
        let r2 = hasher.compute_granular_hash(code_b, "python", None).unwrap();

        assert_eq!(r1.callables["foo"].structural_hash, r2.callables["foo"].structural_hash);
        assert_ne!(r1.module_shell_hash, r2.module_shell_hash);
        assert_ne!(r1.file_hash, r2.file_hash);
    }
}

mod test_granular_duplicate_names {
    use super::*;

    #[test]
    fn test_java_overloaded_methods_in_merkle() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        class C {
            void foo() { }
            void foo(int x) { }
        }
        "#;
        let result = hasher.compute_granular_hash(code, "java", None).unwrap();

        assert_eq!(result.callables.len(), 2);
        let disambiguated = result.callables.keys().filter(|k| k.starts_with("foo@")).count();
        assert_eq!(disambiguated, 1);
        assert!(result.callables.contains_key("foo"));

        let code_one = r#"
        class C {
            void foo() { }
        }
        "#;
        let result_one = hasher.compute_granular_hash(code_one, "java", None).unwrap();
        assert_ne!(result.file_hash, result_one.file_hash);

        let mut modified = code.to_string();
        modified = modified.replace("void foo(int x)", "void foo(int x) { x++; }");
        let result_modified = hasher.compute_granular_hash(&modified, "java", None).unwrap();
        assert_ne!(result.file_hash, result_modified.file_hash);
    }

    #[test]
    fn test_javascript_same_named_nested_functions() {
        let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
        let code = r#"
        function outerA() {
            function inner() { return 1; }
        }
        function outerB() {
            function inner() { return 2; }
        }
        "#;
        let result = hasher.compute_granular_hash(code, "javascript", None).unwrap();

        assert_eq!(result.callables.len(), 4);
        let inner_keys: Vec<_> = result
            .callables
            .keys()
            .filter(|k| k == &"inner" || k.starts_with("inner@"))
            .collect();
        assert_eq!(inner_keys.len(), 2);

        let code_one_body = r#"
        function outerA() {
            function inner() { return 1; }
        }
        function outerB() {
            function inner() { return 3; }
        }
        "#;
        let result_modified = hasher.compute_granular_hash(code_one_body, "javascript", None).unwrap();
        assert_ne!(result.file_hash, result_modified.file_hash);
    }
}

macro_rules! granular_lang_suite {
    ($mod_name:ident, $lang:expr, $extract:expr, $same_a:expr, $same_b:expr, $diff_a:expr, $diff_b:expr, $merkle:expr, $nested:expr, $imports:expr, $uses_name:expr, $import_key:expr) => {
        mod $mod_name {
            use super::*;

            #[test]
            fn extracts_callables() {
                let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
                let result = hasher.compute_granular_hash($extract, $lang, None).unwrap();
                assert!(result.callables.contains_key("foo"));
                assert!(result.callables.contains_key("bar"));
                assert!(matches!(
                    result.callables["foo"].node_type.as_str(),
                    "function" | "method"
                ));
            }

            #[test]
            fn parameter_spelling_is_retained_but_declared_name_is_occurrence_only() {
                let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
                let r1 = hasher.compute_granular_hash($same_a, $lang, None).unwrap();
                let r2 = hasher.compute_granular_hash($same_b, $lang, None).unwrap();
                assert_ne!(
                    r1.callables["foo"].structural_hash,
                    r2.callables["bar"].structural_hash
                );
                let renamed = hasher
                    .compute_granular_hash(&$same_a.replace("foo", "bar"), $lang, None)
                    .unwrap();
                assert_eq!(
                    r1.callables["foo"].structural_hash,
                    renamed.callables["bar"].structural_hash
                );
                assert_ne!(
                    r1.callables["foo"].address_id,
                    renamed.callables["bar"].address_id
                );
            }

            #[test]
            fn different_callable_different_hash() {
                let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
                let r1 = hasher.compute_granular_hash($diff_a, $lang, None).unwrap();
                let r2 = hasher.compute_granular_hash($diff_b, $lang, None).unwrap();
                assert_ne!(
                    r1.callables["foo"].structural_hash,
                    r2.callables["foo"].structural_hash
                );
            }

            #[test]
            fn merkle_tree_structure() {
                let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
                let result = hasher.compute_granular_hash($merkle, $lang, None).unwrap();
                assert!(result.file_hash.starts_with("blake3:"));
                let callable_list: Vec<CallableHash> = result.callables.values().cloned().collect();
                assert_eq!(
                    result.file_hash,
                    compute_merkle_hash(&callable_list, &result.imports, &result.module_shell_hash)
                );
            }

            #[test]
            fn nested_functions() {
                let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
                let result = hasher.compute_granular_hash($nested, $lang, None).unwrap();
                assert!(result.callables.contains_key("outer"));
            }

            #[test]
            fn file_level_and_per_callable_imports() {
                let hasher = StructuralHasher::with_policy(StructuralHashPolicy::v1());
                let result = hasher.compute_granular_hash($imports, $lang, None).unwrap();
                assert!(result.callables.contains_key($uses_name));
                assert!(result.callables.contains_key("noImports") || result.callables.contains_key("no_imports"));
                let key = $import_key;
                assert!(
                    result.imports.iter().any(|i| i.contains(key))
                        || result.callables[$uses_name]
                            .imports
                            .iter()
                            .any(|i| i.contains(key))
                );
                let no_key = if result.callables.contains_key("noImports") {
                    "noImports"
                } else {
                    "no_imports"
                };
                assert!(!result.callables[no_key].imports.iter().any(|i| i.contains(key)));
            }
        }
    };
}

granular_lang_suite!(
    test_granular_javascript,
    "javascript",
    "function foo(x) { return x + 1; }\nfunction bar(y) { return y * 2; }\n",
    "function foo(x) { return x + 1; }\n",
    "function bar(y) { return y + 1; }\n",
    "function foo(x) { return x + 1; }\n",
    "function foo(x) { return x * 2; }\n",
    "function foo() { return 1; }\nfunction bar() { return 2; }\n",
    "function outer(x) { function inner(y) { return y + 1; } return inner(x); }\n",
    "import fs from 'fs';\nfunction usesFs() { return fs.readFileSync('a'); }\nfunction noImports() { return 42; }\n",
    "usesFs",
    "fs"
);

granular_lang_suite!(
    test_granular_typescript,
    "typescript",
    "function foo(x: number): number { return x + 1; }\nfunction bar(y: number): number { return y * 2; }\n",
    "function foo(x: number): number { return x + 1; }\n",
    "function bar(y: number): number { return y + 1; }\n",
    "function foo(x: number): number { return x + 1; }\n",
    "function foo(x: number): number { return x * 2; }\n",
    "function foo(): number { return 1; }\nfunction bar(): number { return 2; }\n",
    "function outer(x: number): number { function inner(y: number): number { return y + 1; } return inner(x); }\n",
    "import * as fs from 'fs';\nfunction usesFs(): number { return fs.readFileSync.length; }\nfunction noImports(): number { return 42; }\n",
    "usesFs",
    "fs"
);

granular_lang_suite!(
    test_granular_java,
    "java",
    "public class T { public int foo(int x) { return x + 1; } public int bar(int y) { return y * 2; } }\n",
    "public class T { public int foo(int x) { return x + 1; } }\n",
    "public class T { public int bar(int y) { return y + 1; } }\n",
    "public class T { public int foo(int x) { return x + 1; } }\n",
    "public class T { public int foo(int x) { return x * 2; } }\n",
    "public class T { public int foo() { return 1; } public int bar() { return 2; } }\n",
    "public class T { public int outer(int x) { class I { int inner(int y) { return y + 1; } } return new I().inner(x); } }\n",
    "import java.io.File;\npublic class T { public String usesFs() { return File.separator; } public int no_imports() { return 42; } }\n",
    "usesFs",
    "File"
);

granular_lang_suite!(
    test_granular_go,
    "go",
    "package main\nfunc foo(x int) int { return x + 1 }\nfunc bar(y int) int { return y * 2 }\n",
    "package main\nfunc foo(x int) int { return x + 1 }\n",
    "package main\nfunc bar(y int) int { return y + 1 }\n",
    "package main\nfunc foo(x int) int { return x + 1 }\n",
    "package main\nfunc foo(x int) int { return x * 2 }\n",
    "package main\nfunc foo() int { return 1 }\nfunc bar() int { return 2 }\n",
    "package main\nfunc outer(x int) int { f := func(y int) int { return y + 1 }; return f(x) }\n",
    "package main\nimport \"os\"\nfunc usesFs() string { return os.Getenv(\"A\") }\nfunc no_imports() int { return 42 }\n",
    "usesFs",
    "os"
);

granular_lang_suite!(
    test_granular_csharp,
    "csharp",
    "public class T { public int foo(int x) { return x + 1; } public int bar(int y) { return y * 2; } }\n",
    "public class T { public int foo(int x) { return x + 1; } }\n",
    "public class T { public int bar(int y) { return y + 1; } }\n",
    "public class T { public int foo(int x) { return x + 1; } }\n",
    "public class T { public int foo(int x) { return x * 2; } }\n",
    "public class T { public int foo() { return 1; } public int bar() { return 2; } }\n",
    "public class T { public int outer(int x) { int Inner(int y) { return y + 1; } return Inner(x); } }\n",
    "using System.IO;\npublic class T { public string usesFs() { return Path.DirectorySeparatorChar.ToString(); } public int no_imports() { return 42; } }\n",
    "usesFs",
    "Path"
);

granular_lang_suite!(
    test_granular_rust,
    "rust",
    "fn foo(x: i32) -> i32 { x + 1 }\nfn bar(y: i32) -> i32 { y * 2 }\n",
    "fn foo(x: i32) -> i32 { x + 1 }\n",
    "fn bar(y: i32) -> i32 { y + 1 }\n",
    "fn foo(x: i32) -> i32 { x + 1 }\n",
    "fn foo(x: i32) -> i32 { x * 2 }\n",
    "fn foo() -> i32 { 1 }\nfn bar() -> i32 { 2 }\n",
    "fn outer(x: i32) -> i32 { fn inner(y: i32) -> i32 { y + 1 } inner(x) }\n",
    "use std::fs;\nfn usesFs() -> usize { fs::read(\"a\").map(|v| v.len()).unwrap_or(0) }\nfn no_imports() -> i32 { 42 }\n",
    "usesFs",
    "fs"
);
