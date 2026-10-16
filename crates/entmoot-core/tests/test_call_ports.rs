//! Shared P1 syntax controls. These do not assert parameter binding or value flow.
#![cfg(feature = "full-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_core::StructuralHasher;
use serde_json::Value;

fn calls(language: &str, source: &str) -> Vec<Value> {
    let hasher = StructuralHasher::new();
    let record = hasher
        .compute_granular_hash_with_analysis(source, language, None)
        .unwrap();
    assert_eq!(
        record.file_hash,
        hasher.compute_granular_hash(source, language, None).unwrap().file_hash
    );
    record
        .markers
        .unwrap()
        .call_sites
        .iter()
        .map(|call| {
            call.validate().unwrap();
            serde_json::to_value(call).unwrap()
        })
        .collect()
}

fn text<'a>(source: &'a str, span: &Value) -> &'a str {
    &source[usize::try_from(span["start_byte"].as_u64().unwrap()).unwrap()
        ..usize::try_from(span["end_byte"].as_u64().unwrap()).unwrap()]
}

#[test]
fn claimed_families_preserve_order_receiver_result_and_exact_callee() {
    for (language, source) in [
        (
            "rust",
            "fn caller() { obj.run(first, callback); obj.run(second, callback); }",
        ),
        (
            "python",
            "def caller():\n    obj.run(first, callback)\n    obj.run(second, callback)\n",
        ),
        (
            "javascript",
            "function caller() { obj.run(first, callback); obj.run(second, callback); }",
        ),
        (
            "typescript",
            "function caller() { obj.run(first, callback); obj.run(second, callback); }",
        ),
        (
            "tsx",
            "function caller() { obj.run(first, callback); obj.run(second, callback); }",
        ),
        (
            "go",
            "package p\nfunc caller() { obj.run(first, callback); obj.run(second, callback) }",
        ),
        (
            "java",
            "class C { void caller() { obj.run(first, callback); obj.run(second, callback); } }",
        ),
        (
            "csharp",
            "class C { void caller() { obj.run(first, callback); obj.run(second, callback); } }",
        ),
        (
            "c",
            "void caller() { obj.run(first, callback); obj.run(second, callback); }",
        ),
    ] {
        let rows = calls(language, source);
        assert_eq!(rows.len(), 2, "{language}: {rows:?}");
        for (index, row) in rows.iter().enumerate() {
            let ports = &row["ports"];
            assert!(ports.is_object(), "{language}: {row}");
            assert_eq!(ports["completeness"], "complete", "{language}: {row}");
            assert_eq!(text(source, &ports["callee"]), "obj.run", "{language}");
            assert_eq!(text(source, &ports["receiver"]), "obj", "{language}");
            assert_eq!(ports["arguments"].as_array().unwrap().len(), 2, "{language}");
            for (position, arg) in ports["arguments"].as_array().unwrap().iter().enumerate() {
                assert_eq!(arg["position"], position);
                assert_eq!(arg["kind"], "positional");
            }
            assert_eq!(
                text(source, &ports["arguments"][0]["span"]),
                if index == 0 { "first" } else { "second" }
            );
            assert_eq!(text(source, &ports["arguments"][1]["span"]), "callback");
            assert_eq!(
                text(source, &ports["result"]),
                if index == 0 {
                    "obj.run(first, callback)"
                } else {
                    "obj.run(second, callback)"
                }
            );
            assert_eq!(row["owner"]["name"], "caller", "{language}");
        }
        assert_ne!(rows[0]["start_byte"], rows[1]["start_byte"]);
    }
}

#[test]
fn keyword_and_spread_syntax_do_not_claim_parameter_positions() {
    for (language, source, expected) in [
        (
            "python",
            "def caller():\n    run(first, named=callback, *more, **keywords)\n",
            vec!["positional", "named", "spread", "keyword_spread"],
        ),
        (
            "javascript",
            "function caller() { run(first, ...more); }",
            vec!["positional", "spread"],
        ),
        (
            "typescript",
            "function caller() { run(first, ...more); }",
            vec!["positional", "spread"],
        ),
        (
            "tsx",
            "function caller() { run(first, ...more); }",
            vec!["positional", "spread"],
        ),
        (
            "csharp",
            "class C { void caller() { run(first, named: callback); } }",
            vec!["positional", "named"],
        ),
    ] {
        let rows = calls(language, source);
        let args = rows[0]["ports"]["arguments"].as_array().unwrap();
        assert_eq!(
            args.iter().map(|a| a["kind"].as_str().unwrap()).collect::<Vec<_>>(),
            expected,
            "{language}"
        );
        if matches!(language, "python" | "csharp") {
            assert_eq!(args[1]["name"], "named");
        }
        assert!(args.iter().all(|a| a.get("parameter").is_none()));
    }
}

#[test]
fn file_scope_nested_dynamic_and_oversized_calls_remain_distinct() {
    let source = "run(inner(value), callback); table[key](other);";
    let rows = calls("javascript", source);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.get("owner").is_none()));
    let outer = rows.iter().find(|r| r["callee"] == "run").unwrap();
    assert_eq!(text(source, &outer["ports"]["arguments"][0]["span"]), "inner(value)");
    let long = format!("{}(value);", "x".repeat(1100));
    let rows = calls("javascript", &long);
    assert!(rows[0]["callee"].as_str().unwrap().starts_with("<oversized-callee:"));
    assert_eq!(text(&long, &rows[0]["ports"]["callee"]).len(), 1100);
}

#[test]
fn malformed_argument_lists_never_claim_complete_ports() {
    for (language, prefix, suffix) in [
        ("rust", "fn caller() {", "}"),
        ("python", "def caller():\n    ", "\n"),
        ("javascript", "function caller() {", "}"),
        ("typescript", "function caller() {", "}"),
        ("tsx", "function caller() {", "}"),
        ("go", "package p\nfunc caller() {", "}"),
        ("java", "class C { void caller() {", "}}"),
        ("csharp", "class C { void caller() {", "}}"),
        ("c", "void caller() {", "}"),
    ] {
        let source = format!("{prefix}run(first, , last);{suffix}");
        match StructuralHasher::new().compute_granular_hash_with_analysis(&source, language, None) {
            Err(entmoot_core::CoreError::Parse(_)) => {}
            Ok(record) => {
                // Recovery-enabled grammars must preserve invalid-site status.
                let rows = record.markers.unwrap().call_sites;
                assert!(!rows.is_empty(), "{language}: malformed site was lost");
                for row in rows {
                    row.validate().unwrap();
                    let ports = row.ports.unwrap();
                    assert_eq!(
                        ports.completeness,
                        entmoot_contracts::SiteCompleteness::Partial,
                        "{language}: {ports:?}"
                    );
                    assert!(ports
                        .limitations
                        .contains(&entmoot_contracts::SiteLimitation::InvalidSyntax));
                }
            }
            Err(error) => panic!("{language}: unexpected error: {error}"),
        }
    }
}

#[test]
fn argument_budget_preserves_prefix_and_explicit_partial_outcome() {
    for (language, prefix, suffix) in [
        ("rust", "fn caller() {", "}"),
        ("python", "def caller():\n    ", "\n"),
        ("javascript", "function caller() {", "}"),
        ("typescript", "function caller() {", "}"),
        ("tsx", "function caller() {", "}"),
        ("go", "package p\nfunc caller() {", "}"),
        ("java", "class C { void caller() {", "}}"),
        ("csharp", "class C { void caller() {", "}}"),
        ("c", "void caller() {", "}"),
    ] {
        let source = format!("{prefix}run({});{suffix}", vec!["value"; 300].join(","));
        let rows = calls(language, &source);
        assert_eq!(rows[0]["ports"]["arguments"].as_array().unwrap().len(), 256);
        assert_eq!(rows[0]["ports"]["completeness"], "partial");
        assert_eq!(rows[0]["ports"]["limitations"], serde_json::json!(["argument_budget"]));
        let source = format!("{prefix}{}(value);{suffix}", "x".repeat(1100));
        let rows = calls(language, &source);
        assert!(
            rows[0]["callee"].as_str().unwrap().starts_with("<oversized-callee:"),
            "{language}"
        );
    }
}
