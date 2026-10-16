//! Authored clone relationships over production declaration/occurrence inventory.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_core::StructuralHasher;

#[test]
fn alpha_does_not_confuse_hoisted_or_function_local_reads_with_free_names() {
    for (language, source) in [
        (
            "javascript",
            "function a(){return x; var x=1;} function b(){return x; var y=1;}",
        ),
        (
            "typescript",
            "function a(){return x; let x=1;} function b(){return x; let y=1;}",
        ),
        (
            "python",
            "def a():\n    return x\n    x=1\ndef b():\n    return x\n    y=1\n",
        ),
        (
            "java",
            "class C { int a(){int x=x; return x;} int b(){int y=x; return y;} }",
        ),
        (
            "csharp",
            "class C { int A(){int x=x; return x;} int B(){int y=x; return y;} }",
        ),
    ] {
        let analysis = patterns(source, language);
        let rows: Vec<_> = analysis
            .patterns
            .iter()
            .filter(|p| p.extent == entmoot_contracts::CloneExtent::Callable)
            .collect();
        assert_eq!(rows.len(), 2, "{language}");
        assert!(
            rows[0].fingerprints.alpha.is_none() || rows[0].fingerprints.alpha != rows[1].fingerprints.alpha,
            "free/local collision: {language}"
        );
    }
}

#[test]
fn canon_does_not_infer_dispatch_or_reflection_from_a_callee_spelling() {
    for source in ["fn f() { evaluate(); }", "fn f() { eval(); }", "fn f() { ordinary(); }"] {
        let hasher = StructuralHasher::new();
        let inventory = hasher
            .compute_granular_hash_with_analysis(source, "rust", None)
            .unwrap();
        let contracts = inventory.markers.unwrap().code_contracts;
        let result = hasher.canon_file(source, "rust", &contracts, &[], 100).unwrap();
        assert_eq!(
            result.forms[0].limitations,
            vec![entmoot_contracts::CanonLimit::UnsupportedSemantics]
        );
    }
}

#[test]
fn canon_boolean_returns_share_the_kernel_across_language_families() {
    for (language, source) in [
        ("rust", "fn a()->bool { true } fn b()->bool { false }"),
        ("python", "def a():\n    return True\ndef b():\n    return False\n"),
        ("javascript", "function a(){return true;} function b(){return false;}"),
        (
            "typescript",
            "function a():boolean{return true;} function b():boolean{return false;}",
        ),
        (
            "go",
            "package p\nfunc a() bool {return true}\nfunc b() bool {return false}\n",
        ),
        (
            "java",
            "class C { boolean a(){return true;} boolean b(){return false;} }",
        ),
        ("csharp", "class C { bool A(){return true;} bool B(){return false;} }"),
        ("c", "_Bool a(){return 1;} _Bool b(){return 0;}"),
    ] {
        let hasher = StructuralHasher::new();
        let inventory = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let contracts = inventory.markers.unwrap().code_contracts;
        let canon = hasher.canon_file(source, language, &contracts, &[], 1000).unwrap();
        assert_eq!(canon.forms.len(), 2, "{language}");
        assert_ne!(canon.forms[0].fingerprint(), canon.forms[1].fingerprint(), "{language}");
        for form in &canon.forms {
            form.validate().unwrap();
        }
    }
}

#[test]
fn canon_requires_native_types_and_keeps_assumptions_and_branch_order() {
    use entmoot_contracts::*;
    let source = "fn f(x:bool)->bool { if x { true } else { false } }";
    let hasher = StructuralHasher::new();
    let inventory = hasher
        .compute_granular_hash_with_analysis(source, "rust", None)
        .unwrap();
    let contracts = inventory.markers.unwrap().code_contracts;
    let unresolved = hasher.canon_file(source, "rust", &contracts, &[], 100).unwrap();
    assert!(!unresolved.forms[0].complete());
    let fact = CanonExpressionFact {
        site: NativeSpanRef {
            path: "a.rs".into(),
            span: SiteSpan {
                start_byte: 5,
                end_byte: 11,
            },
        },
        source_digest: ContentDigest::from_trusted(blake3_hex(source.as_bytes())),
        sort: CanonSort::Bool,
        overflow: None,
        limitations: vec![],
    };
    let qualified = hasher
        .canon_file(source, "rust", &contracts, std::slice::from_ref(&fact), 100)
        .unwrap();
    assert!(qualified.forms[0].complete(), "{:?}", qualified.forms[0].limitations);
    assert!(qualified.forms[0].terms.iter().any(|t| t.op == CanonOp::Ite));
    assert!(qualified.forms[0]
        .obligation
        .assumptions
        .contains(&CanonAssumption::ValidNativeProgram));
    let mut stale = fact;
    stale.source_digest = ContentDigest::from_trusted(blake3_hex(b"stale"));
    assert!(!hasher
        .canon_file(source, "rust", &contracts, &[stale], 100)
        .unwrap()
        .forms[0]
        .complete());
}

#[test]
fn canon_integer_models_preserve_width_operators_and_overflow_limits() {
    use entmoot_contracts::*;
    for (language, source, parameter, width, overflow) in [
        (
            "rust",
            "fn f(x:i32)->i32 { x+x }",
            "x:i32",
            Some(32),
            CanonOverflow::Checked,
        ),
        (
            "go",
            "package p\nfunc f(x int32) int32 { return x+x }",
            "x int32",
            Some(32),
            CanonOverflow::Wrap,
        ),
        (
            "java",
            "class C { int f(int x) { return x+x; } }",
            "int x",
            Some(32),
            CanonOverflow::Wrap,
        ),
        (
            "csharp",
            "class C { int F(int x) { return x+x; } }",
            "int x",
            Some(32),
            CanonOverflow::Wrap,
        ),
        (
            "c",
            "int f(int x) { return x+x; }",
            "int x",
            Some(32),
            CanonOverflow::Undefined,
        ),
        (
            "python",
            "def f(x: int):\n    return x+x\n",
            "x: int",
            None,
            CanonOverflow::Unbounded,
        ),
        (
            "typescript",
            "function f(x:bigint):bigint { return x+x; }",
            "x:bigint",
            None,
            CanonOverflow::Unbounded,
        ),
        (
            "javascript",
            "function f(x) { return x+x; }",
            "x",
            None,
            CanonOverflow::Unbounded,
        ),
    ] {
        let hasher = StructuralHasher::new();
        let contracts = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap()
            .markers
            .unwrap()
            .code_contracts;
        let facts: Vec<_> = [parameter, "x+x"]
            .iter()
            .map(|text| {
                let start = source.find(text).unwrap();
                CanonExpressionFact {
                    site: NativeSpanRef {
                        path: "fixture".into(),
                        span: SiteSpan {
                            start_byte: start,
                            end_byte: start + text.len(),
                        },
                    },
                    source_digest: ContentDigest::from_trusted(blake3_hex(source.as_bytes())),
                    sort: CanonSort::Integer {
                        bits: width,
                        signed: true,
                    },
                    overflow: Some(overflow),
                    limitations: vec![],
                }
            })
            .collect();
        let first = hasher
            .canon_file(source, language, &contracts, &facts, 100)
            .unwrap()
            .forms
            .remove(0);
        first.validate().unwrap();
        assert_eq!(
            first.complete(),
            matches!(overflow, CanonOverflow::Wrap | CanonOverflow::Unbounded),
            "{language}: {:?}",
            first.limitations
        );
        if first.complete() {
            let renamed = source.replace('x', "renamed");
            let renamed_contracts = hasher
                .compute_granular_hash_with_analysis(&renamed, language, None)
                .unwrap()
                .markers
                .unwrap()
                .code_contracts;
            let renamed_facts: Vec<_> = [parameter.replace('x', "renamed"), "renamed+renamed".into()]
                .iter()
                .map(|text| {
                    let start = renamed.find(text).unwrap();
                    let mut fact = facts[0].clone();
                    fact.site.span = SiteSpan {
                        start_byte: start,
                        end_byte: start + text.len(),
                    };
                    fact.source_digest = ContentDigest::from_trusted(blake3_hex(renamed.as_bytes()));
                    fact
                })
                .collect();
            let second = hasher
                .canon_file(&renamed, language, &renamed_contracts, &renamed_facts, 100)
                .unwrap()
                .forms
                .remove(0);
            assert_eq!(first.fingerprint(), second.fingerprint(), "{language}");
            let mut wide = facts.clone();
            for fact in &mut wide {
                fact.sort = CanonSort::Integer {
                    bits: Some(64),
                    signed: true,
                };
                fact.overflow = Some(CanonOverflow::Wrap);
            }
            let third = hasher
                .canon_file(source, language, &contracts, &wide, 100)
                .unwrap()
                .forms
                .remove(0);
            assert_ne!(first.fingerprint(), third.fingerprint(), "{language}");
        }
    }
}

#[test]
fn canon_does_not_erase_unmodeled_effects_or_dynamic_calls() {
    for source in [
        "fn f()->bool { side_effect(); true }",
        "fn f()->bool { loop {} true }",
        "fn f()->bool { eval(\"x\") }",
    ] {
        let hasher = StructuralHasher::new();
        let inventory = hasher
            .compute_granular_hash_with_analysis(source, "rust", None)
            .unwrap();
        let canon = hasher
            .canon_file(source, "rust", &inventory.markers.unwrap().code_contracts, &[], 100)
            .unwrap();
        assert!(!canon.forms[0].complete(), "{source}");
        canon.forms[0].validate().unwrap();
    }
}

#[test]
fn canon_merges_local_assignments_in_javascript_and_python() {
    use entmoot_contracts::*;
    for (language, source, parameter) in [
        (
            "javascript",
            "function f(x) { let y = true; if (x) { y = false; } else { y = true; } return y; }",
            SiteSpan {
                start_byte: 11,
                end_byte: 12,
            },
        ),
        (
            "python",
            "def f(x):\n    y = True\n    if x:\n        y = False\n    else:\n        y = True\n    return y\n",
            SiteSpan {
                start_byte: 6,
                end_byte: 7,
            },
        ),
    ] {
        let hasher = StructuralHasher::new();
        let inventory = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let native = CanonExpressionFact {
            site: NativeSpanRef {
                path: "fixture".into(),
                span: parameter,
            },
            source_digest: ContentDigest::from_trusted(blake3_hex(source.as_bytes())),
            sort: CanonSort::Bool,
            overflow: None,
            limitations: vec![],
        };
        let result = hasher
            .canon_file(
                source,
                language,
                &inventory.markers.unwrap().code_contracts,
                &[native],
                100,
            )
            .unwrap();
        assert!(
            result.forms[0].complete(),
            "{language}: {:?}",
            result.forms[0].limitations
        );
        assert!(result.forms[0].terms.iter().any(|t| t.op == CanonOp::Ite));
    }
}

fn patterns(source: &str, language: &str) -> entmoot_core::clones::CloneAnalysis {
    let hasher = StructuralHasher::new();
    let inventory = hasher
        .compute_granular_hash_with_analysis(source, language, None)
        .unwrap();
    hasher
        .clone_patterns(
            source,
            language,
            &inventory.markers.unwrap().code_contracts,
            1,
            true,
            500,
        )
        .unwrap()
}

#[test]
fn compositional_clone_derivation_has_pinned_fingerprints() {
    let rows = patterns("fn f(x:i32) { let y = 12; consume(x, y, 12, 13, \"a\"); }", "rust");
    let hashes = &rows.patterns[0].fingerprints;
    assert_eq!(
        (&hashes.syntax, hashes.alpha.as_deref(), hashes.parameterized.as_deref()),
        (
            &"blake3:7561b72be92cc9ea2e620658b9cdde2381beb8d6f4801f62c84103e189c1a3c9".into(),
            Some("blake3:e19d86da7ef48eca4e05765eb7713b2ec45387937f90f0f2c36b24b26dd28df5"),
            Some("blake3:9b63bfaa6db5f93aea0c7ae3adaca04a7737b287bfc090064fbb4823bc00575b")
        )
    );
}

#[test]
fn canon_preserves_async_decorators_and_shadowing_boundaries() {
    for (language, source) in [
        ("javascript", "async function f() { return true; }"),
        ("python", "@decorate\ndef f():\n    return True\n"),
        ("rust", "async fn f() -> bool { true }"),
        (
            "rust",
            "fn f() -> bool { let x = true; let y = { let x = false; x }; x }",
        ),
        ("go", "package p\nvar true = false\nfunc f() bool { return true }"),
    ] {
        let hasher = StructuralHasher::new();
        let inventory = hasher
            .compute_granular_hash_with_analysis(source, language, None)
            .unwrap();
        let result = hasher
            .canon_file(source, language, &inventory.markers.unwrap().code_contracts, &[], 100)
            .unwrap();
        assert!(!result.forms.is_empty(), "{language}");
        assert!(!result.forms[0].complete(), "{source}");
    }
}

#[test]
fn renamed_locals_keep_types_literals_external_names_and_identity_distinct() {
    let source="fn first(value: i32) -> i32 { let result = value + 1; result }\nfn second(input: i32) -> i32 { let output = input + 1; output }\nfn third(input: i64) -> i64 { let output = input + 1; output }\nfn fourth(input: i32) -> i32 { let output = input + 2; output }";
    let analysis = patterns(source, "rust");
    let rows: Vec<_> = analysis
        .patterns
        .iter()
        .filter(|p| p.extent == entmoot_contracts::CloneExtent::Callable)
        .collect();
    assert_eq!(rows.len(), 4);
    assert_ne!(rows[0].fingerprints.syntax, rows[1].fingerprints.syntax);
    assert!(rows[0].fingerprints.alpha.is_some());
    assert_eq!(rows[0].fingerprints.alpha, rows[1].fingerprints.alpha);
    assert_ne!(rows[1].fingerprints.alpha, rows[2].fingerprints.alpha);
    assert_ne!(rows[1].fingerprints.alpha, rows[3].fingerprints.alpha);
    assert_eq!(rows[1].fingerprints.parameterized, rows[3].fingerprints.parameterized);
    let other = patterns("fn a(x:i32)->i32 { left(x) } fn b(y:i32)->i32 { right(y) }", "rust");
    assert_ne!(
        other.patterns[0].fingerprints.alpha,
        other.patterns[1].fingerprints.alpha
    );
}

#[test]
fn nested_closures_macros_and_malformed_source_do_not_claim_alpha_equivalence() {
    for source in [
        "fn f(x:i32) { let y = || x; use_it(y); }",
        "fn f(x:i32) { println!(\"{}\",x); }",
    ] {
        let analysis = patterns(source, "rust");
        assert!(analysis.alpha_unavailable_callables > 0, "{source}");
        assert!(analysis.patterns[0].fingerprints.alpha.is_none(), "{source}");
    }
    assert!(StructuralHasher::new()
        .clone_patterns("fn broken(", "rust", &[], 1, true, 50)
        .is_err());
}

#[test]
fn repeated_statement_windows_have_exact_source_extents_and_budgets() {
    let source="fn a(x:i32) { first(x); second(x); third(x); extra(x); } fn b(y:i32) { before(y); first(y); second(y); third(y); }";
    let analysis = patterns(source, "rust");
    let matching: Vec<_> = analysis
        .patterns
        .iter()
        .filter(|p| {
            &source[p.span.start_byte..p.span.end_byte] == "first(x); second(x); third(x);"
                || &source[p.span.start_byte..p.span.end_byte] == "first(y); second(y); third(y);"
        })
        .collect();
    assert_eq!(matching.len(), 2);
    // Both windows capture a parameter outside their extent: its spelling stays.
    assert_ne!(matching[0].fingerprints.alpha, matching[1].fingerprints.alpha);
    let hasher = StructuralHasher::new();
    let inventory = hasher
        .compute_granular_hash_with_analysis(source, "rust", None)
        .unwrap();
    let limited = hasher
        .clone_patterns(source, "rust", &inventory.markers.unwrap().code_contracts, 1, true, 1)
        .unwrap();
    assert_eq!(limited.patterns.len(), 1);
    assert!(limited.truncated);
}

#[test]
fn clone_work_budget_counts_rejected_candidates_and_stops_before_scope_analysis() {
    let source = "fn a() { true } fn b(x:i32) { println!(\"{}\", x); }";
    let hasher = StructuralHasher::new();
    let contracts = hasher
        .compute_granular_hash_with_analysis(source, "rust", None)
        .unwrap()
        .markers
        .unwrap()
        .code_contracts;
    for budget in [0, 1] {
        let result = hasher
            .clone_patterns(source, "rust", &contracts, 1000, true, budget)
            .unwrap();
        assert!(result.patterns.is_empty());
        assert!(result.truncated, "budget {budget}");
        assert_eq!(
            result.alpha_unavailable_callables, 0,
            "unvisited callables are not analyzed"
        );
    }
}

#[test]
fn statement_windows_do_not_leak_out_of_nested_callables() {
    let source =
        "function outer(x) { function inner(y) { one(y); two(y); three(y); } if (x) { one(x); two(x); three(x); } }";
    let result = patterns(source, "javascript");
    let inner = source.find("function inner").unwrap();
    for (body, owner) in [("one(y); two(y); three(y);", inner), ("one(x); two(x); three(x);", 0)] {
        let windows: Vec<_> = result
            .patterns
            .iter()
            .filter(|p| {
                p.extent == entmoot_contracts::CloneExtent::Statements
                    && &source[p.span.start_byte..p.span.end_byte] == body
            })
            .collect();
        assert_eq!(windows.len(), 1, "{body}");
        assert_eq!(windows[0].owner_start, owner, "{body}");
    }
}

#[test]
fn supported_families_preserve_parameter_use_patterns() {
    for (language, source) in [
        (
            "python",
            "def a(x: int, /):\n    return x + x\ndef b(y: int, /):\n    return y + y\n",
        ),
        (
            "javascript",
            "function a(x) { return x + x; } function b(y) { return y + y; }",
        ),
        (
            "typescript",
            "function a(x: number) { return x + x; } function b(y: number) { return y + y; }",
        ),
        (
            "go",
            "package p\nfunc a(x int) int { return x+x }\nfunc b(y int) int { return y+y }",
        ),
        (
            "java",
            "class Box { int a(int x) { return x+x; } int b(int y) { return y+y; } }",
        ),
        ("c", "int a(int x) { return x+x; } int b(int y) { return y+y; }"),
    ] {
        let analysis = patterns(source, language);
        let rows: Vec<_> = analysis
            .patterns
            .iter()
            .filter(|p| p.extent == entmoot_contracts::CloneExtent::Callable)
            .collect();
        assert_eq!(rows.len(), 2, "{language}");
        assert!(rows[0].fingerprints.alpha.is_some(), "{language}");
        assert_eq!(rows[0].fingerprints.alpha, rows[1].fingerprints.alpha, "{language}");
    }
}

#[test]
fn alpha_preserves_names_callers_can_bind_and_still_renames_private_locals() {
    for (language, source, equal) in [
        ("python", "def a(x):\n    return x\ndef b(y):\n    return y\n", false),
        (
            "python",
            "def a(*, x):\n    return x\ndef b(*, y):\n    return y\n",
            false,
        ),
        ("python", "def a(*x):\n    return x\ndef b(*y):\n    return y\n", true),
        ("python", "def a(**x):\n    return x\ndef b(**y):\n    return y\n", true),
        (
            "python",
            "def a(x):\n    first = x\n    return first\ndef b(x):\n    second = x\n    return second\n",
            true,
        ),
        (
            "csharp",
            "class C { int A(int x) { return x; } int B(int y) { return y; } }",
            false,
        ),
        (
            "csharp",
            "class C { int A(int x) { int first = x; return first; } int B(int x) { int second = x; return second; } }",
            true,
        ),
    ] {
        let analysis = patterns(source, language);
        let rows: Vec<_> = analysis
            .patterns
            .iter()
            .filter(|p| p.extent == entmoot_contracts::CloneExtent::Callable)
            .collect();
        assert_eq!(rows.len(), 2, "{source}");
        assert!(rows.iter().all(|p| p.fingerprints.alpha.is_some()), "{source}");
        assert_eq!(
            rows[0].fingerprints.alpha == rows[1].fingerprints.alpha,
            equal,
            "{source}"
        );
        assert_eq!(
            rows[0].fingerprints.parameterized == rows[1].fingerprints.parameterized,
            equal,
            "{source}"
        );
    }
}

#[test]
fn alpha_declines_name_observation_or_uninventoried_parameters() {
    for (language, source) in [
        ("python", "def f(x, /):\n    return eval('x')\n"),
        ("python", "def f(x, /):\n    run = eval\n    return run('x')\n"),
        ("python", "def f(x, /):\n    exec('print(x)')\n"),
        ("python", "def f(x, /):\n    return locals()\n"),
        ("python", "def f(x, /):\n    return vars()\n"),
        ("python", "def f(x, /):\n    return globals()\n"),
        ("python", "def f(x, /):\n    return f'{x=}'\n"),
        ("javascript", "function f(x) { return eval('x'); }"),
        ("typescript", "function f(x: number) { return eval('x'); }"),
        ("tsx", "function f(x: number) { return eval('x'); }"),
        ("javascript", "function f(x) { return new Function('return x'); }"),
        ("csharp", "class C { string F() { int x = 1; return nameof(x); } }"),
        // This parameter shape is not inventoried yet; do not infer its binder.
        ("csharp", "class C { int[] F(params int[] x) { return x; } }"),
        ("c", "int f(int x) {\n#define STRINGIFY(x) #x\nreturn x;\n}"),
    ] {
        let analysis = patterns(source, language);
        assert!(!analysis.patterns.is_empty(), "{source}");
        assert_eq!(analysis.alpha_unavailable_callables, 1, "{source}");
        for pattern in analysis.patterns {
            assert!(pattern.fingerprints.alpha.is_none(), "{source}");
            assert!(pattern.fingerprints.parameterized.is_none(), "{source}");
            assert!(!pattern.fingerprints.syntax.is_empty(), "{source}");
        }
    }
    for (language, source) in [
        (
            "python",
            "def f(x, /):\n    return f'{x}' + 'eval locals vars globals exec'\n",
        ),
        ("javascript", "function f(x) { return 'eval Function' + x; }"),
        ("rust", "fn f(x: i32) { eval(x); }"),
    ] {
        assert!(
            patterns(source, language).patterns[0].fingerprints.alpha.is_some(),
            "{source}"
        );
    }
}

#[test]
fn interpolation_operations_and_nested_block_bindings_are_not_erased() {
    let qualified = patterns("fn a(x:i32){ module::x(x); } fn b(y:i32){ module::y(y); }", "rust");
    assert_ne!(
        qualified.patterns[0].fingerprints.alpha,
        qualified.patterns[1].fingerprints.alpha
    );
    let analysis = patterns(
        "def a(x):\n    return f'{external(x)}'\ndef b(y):\n    return f'{different(y)}'\n",
        "python",
    );
    assert_ne!(
        analysis.patterns[0].fingerprints.parameterized,
        analysis.patterns[1].fingerprints.parameterized
    );
    let analysis = patterns("function a(x) { { let y=x; use(y); } return y; }", "javascript");
    assert!(analysis.patterns[0].fingerprints.alpha.is_none());
}

#[test]
fn utf8_and_crlf_preserve_byte_spans_without_unicode_folding() {
    let source = "# 🍃\r\ndef first(é, /):\r\n    return é + 1\r\ndef second(值, /):\r\n    return 值 + 1\r\n";
    let analysis = patterns(source, "python");
    assert_eq!(analysis.patterns.len(), 2);
    for pattern in &analysis.patterns {
        assert!(source[pattern.span.start_byte..pattern.span.end_byte].starts_with("def "));
    }
    assert_eq!(
        analysis.patterns[0].fingerprints.alpha,
        analysis.patterns[1].fingerprints.alpha
    );
    assert_ne!(
        analysis.patterns[0].fingerprints.exact,
        analysis.patterns[1].fingerprints.exact
    );
}

#[test]
fn statement_alpha_keeps_captures_and_normalizes_only_included_declarations() {
    let source = "fn a(x:i32) { first(x); second(x); third(x); } fn b(y:i32) { first(y); second(y); third(y); }";
    let result = patterns(source, "rust");
    let windows: Vec<_> = result
        .patterns
        .iter()
        .filter(|p| p.extent == entmoot_contracts::CloneExtent::Statements)
        .collect();
    assert_eq!(windows.len(), 2);
    assert_ne!(windows[0].fingerprints.alpha, windows[1].fingerprints.alpha);
    assert_ne!(
        windows[0].fingerprints.parameterized,
        windows[1].fingerprints.parameterized
    );
    let source =
        "fn a(x:i32) { let one=x; first(one); second(one); } fn b(x:i32) { let two=x; first(two); second(two); }";
    let result = patterns(source, "rust");
    let windows: Vec<_> = result
        .patterns
        .iter()
        .filter(|p| p.extent == entmoot_contracts::CloneExtent::Statements)
        .collect();
    assert_eq!(windows.len(), 2);
    assert!(windows[0].fingerprints.alpha.is_some());
    assert_eq!(windows[0].fingerprints.alpha, windows[1].fingerprints.alpha);
}

#[test]
fn rust_alpha_resolves_sequential_shadowing_initializers_and_block_exit() {
    for (left, right, equal) in [
        (
            "fn a(x:i32){let x=x+1; use_it(x);}",
            "fn b(y:i32){let z=y+1; use_it(z);}",
            true,
        ),
        (
            "fn a(x:i32){let x=x+1; use_it(x);}",
            "fn b(y:i32){let z=z+1; use_it(z);}",
            false,
        ),
        (
            "fn a(x:i32){let value=x; {let value=value+1; use_it(value);} use_it(value);}",
            "fn b(seed:i32){let outer=seed; {let inner=outer+1; use_it(inner);} use_it(outer);}",
            true,
        ),
        (
            "fn a(x:i32){let value=x; {let value=value+1; use_it(value);} use_it(value);}",
            "fn b(seed:i32){let outer=seed; {let inner=outer+1; use_it(outer);} use_it(outer);}",
            false,
        ),
        (
            "fn a(x:i32){{let y=x; use_it(y);} use_it(y);}",
            "fn b(z:i32){{let y=z; use_it(y);} use_it(y);}",
            true,
        ),
        (
            "fn a(x:i32){{let y=x; use_it(y);} use_it(y);}",
            "fn b(z:i32){{let q=z; use_it(q);} use_it(q);}",
            false,
        ),
        (
            "fn a(x:i32){let y={let x=1; x}; use_it(x,y);}",
            "fn b(z:i32){let q={let v=1; v}; use_it(z,q);}",
            true,
        ),
    ] {
        let a = patterns(left, "rust");
        let b = patterns(right, "rust");
        assert_eq!(a.alpha_unavailable_callables, 0, "{left}");
        assert_eq!(b.alpha_unavailable_callables, 0, "{right}");
        assert_eq!(
            a.patterns[0].fingerprints.alpha == b.patterns[0].fingerprints.alpha,
            equal,
            "{left}\n{right}"
        );
    }
}

#[test]
fn alpha_declines_unlowered_local_and_pattern_binders() {
    for (language, source) in [
        ("rust", "fn f(x:i32){if let Some(x)=maybe(){use_it(x);}}"),
        ("rust", "fn f(x:i32){let (a,b)=pair(x); use_it(a,b);}"),
        ("rust", "fn f(x:i32){const x:i32=1; use_it(x);}"),
        ("go", "package p\nfunc f(x int){var y=x; use(y)}"),
        ("c", "int f(int x){{int x=1; use(x);} return x;}"),
        ("java", "class C { void f(int x) { for(int y: xs) { use(y); } } }"),
    ] {
        let result = patterns(source, language);
        assert!(!result.patterns.is_empty(), "{source}");
        assert!(
            result.patterns.iter().all(|p| p.fingerprints.alpha.is_none()),
            "{source}"
        );
    }
}

#[test]
fn parameterization_preserves_literal_partition_and_free_names() {
    for (left, right, equal) in [
        ("fn a(){call(1,2,1);}", "fn b(){call(7,9,7);}", true),
        ("fn a(){call(1,2,1);}", "fn b(){call(7,9,9);}", false),
        ("fn a(){call(1,2,1);}", "fn b(){call(7,7,7);}", false),
        ("fn a(){call(1,2,1);}", "fn b(){other(7,9,7);}", false),
        ("fn a(){call(1,2,1);}", "fn b(){call(7.0,9.0,7.0);}", false),
    ] {
        let a = patterns(left, "rust");
        let b = patterns(right, "rust");
        assert_eq!(
            a.patterns[0].fingerprints.parameterized == b.patterns[0].fingerprints.parameterized,
            equal,
            "{left}\n{right}"
        );
    }
}

#[test]
fn callable_summaries_are_independent_of_statement_admission() {
    let source = "fn f(x:i32){let x=x+1; {let y=x+2; use_it(y);} use_it(x);}";
    let hasher = StructuralHasher::new();
    let contracts = hasher
        .compute_granular_hash_with_analysis(source, "rust", None)
        .unwrap()
        .markers
        .unwrap()
        .code_contracts;
    let with_windows = hasher.clone_patterns(source, "rust", &contracts, 1, true, 100).unwrap();
    for budget in [1, 100] {
        let without = hasher
            .clone_patterns(source, "rust", &contracts, 1, false, budget)
            .unwrap();
        assert_eq!(with_windows.patterns[0].fingerprints, without.patterns[0].fingerprints);
    }
}

#[test]
fn source_windows_share_local_summaries_without_erasing_captures_across_families() {
    for (language, source, first, second) in [
        (
            "javascript",
            "function a(x){one(x); two(x); three(x);} function b(y){one(y); two(y); three(y);}",
            "one(x); two(x); three(x);",
            "one(y); two(y); three(y);",
        ),
        (
            "typescript",
            "function a(x:number){one(x); two(x); three(x);} function b(y:number){one(y); two(y); three(y);}",
            "one(x); two(x); three(x);",
            "one(y); two(y); three(y);",
        ),
        (
            "python",
            "def a(x, /):\n    one(x)\n    two(x)\n    three(x)\ndef b(y, /):\n    one(y)\n    two(y)\n    three(y)\n",
            "one(x)\n    two(x)\n    three(x)",
            "one(y)\n    two(y)\n    three(y)",
        ),
        (
            "java",
            "class C { void a(int x){one(x); two(x); three(x);} void b(int y){one(y); two(y); three(y);} }",
            "one(x); two(x); three(x);",
            "one(y); two(y); three(y);",
        ),
        (
            "c",
            "void a(int x){one(x); two(x); three(x);} void b(int y){one(y); two(y); three(y);}",
            "one(x); two(x); three(x);",
            "one(y); two(y); three(y);",
        ),
        (
            "go",
            "package p\nfunc a(x int){one(x); two(x); three(x)}\nfunc b(y int){one(y); two(y); three(y)}",
            "one(x); two(x); three(x)",
            "one(y); two(y); three(y)",
        ),
    ] {
        let result = patterns(source, language);
        let windows: Vec<_> = result
            .patterns
            .iter()
            .filter(|p| {
                p.extent == entmoot_contracts::CloneExtent::Statements
                    && [first, second].contains(&&source[p.span.start_byte..p.span.end_byte])
            })
            .collect();
        assert_eq!(windows.len(), 2, "{language}");
        assert!(windows.iter().all(|p| p.fingerprints.alpha.is_some()), "{language}");
        assert_ne!(
            windows[0].fingerprints.alpha, windows[1].fingerprints.alpha,
            "{language}"
        );
    }
    let source =
        "fn a(){before(8); let x=1; use_it(x,1); use_it(x,2);} fn b(){before(9); let y=7; use_it(y,7); use_it(y,8);}";
    let result = patterns(source, "rust");
    let windows: Vec<_> = result
        .patterns
        .iter()
        .filter(|p| {
            p.extent == entmoot_contracts::CloneExtent::Statements
                && source[p.span.start_byte..p.span.end_byte].starts_with("let ")
        })
        .collect();
    assert_eq!(windows.len(), 2);
    assert_ne!(windows[0].fingerprints.alpha, windows[1].fingerprints.alpha);
    assert_eq!(
        windows[0].fingerprints.parameterized,
        windows[1].fingerprints.parameterized
    );
}

#[test]
#[ignore = "local clone hashing cost probe; timing is not a correctness assertion"]
fn clone_summary_cost_probe() {
    use std::time::Instant;
    for size in [32, 256, 1024] {
        let mut source = String::from("fn sample(input: i32) {\n");
        for i in 0..size {
            source.push_str(&format!("let value{i} = input + {i}; use_it(value{i});\n"));
        }
        source.push('}');
        let hasher = StructuralHasher::new();
        let mut inventory_times = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            std::hint::black_box(
                hasher
                    .compute_granular_hash_with_analysis(&source, "rust", None)
                    .unwrap(),
            );
            inventory_times.push(start.elapsed().as_micros());
        }
        inventory_times.sort();
        eprintln!(
            "size={size} inventory_median_us={} samples={inventory_times:?}",
            inventory_times[3]
        );
        let contracts = hasher
            .compute_granular_hash_with_analysis(&source, "rust", None)
            .unwrap()
            .markers
            .unwrap()
            .code_contracts;
        for statements in [false, true] {
            let mut timings = Vec::new();
            for _ in 0..7 {
                let start = Instant::now();
                let result = hasher
                    .clone_patterns(&source, "rust", &contracts, 1, statements, 20000)
                    .unwrap();
                assert!(result.patterns[0].fingerprints.alpha.is_some());
                std::hint::black_box(result);
                timings.push(start.elapsed().as_micros());
            }
            timings.sort();
            eprintln!(
                "size={size} statements={statements} median_us={} samples={timings:?}",
                timings[3]
            );
            let mut workflow = Vec::new();
            for _ in 0..7 {
                let start = Instant::now();
                let inventory = hasher
                    .compute_granular_hash_with_analysis(&source, "rust", None)
                    .unwrap();
                std::hint::black_box(
                    hasher
                        .clone_patterns(
                            &source,
                            "rust",
                            &inventory.markers.unwrap().code_contracts,
                            1,
                            statements,
                            20000,
                        )
                        .unwrap(),
                );
                workflow.push(start.elapsed().as_micros());
            }
            workflow.sort();
            eprintln!(
                "size={size} statements={statements} workflow_median_us={} samples={workflow:?}",
                workflow[3]
            );
        }
    }
}
