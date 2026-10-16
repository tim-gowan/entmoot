use entmoot_contracts::*;
use serde_json::Value;

pub fn lower(source: &str, language: &str, path: &str, output: &Value) {
    assert_ne!(output["invalid_environment"], true, "{output}");
    let rows = output["build_context"]["canon_expressions"]
        .as_array()
        .expect("native scalar sites");
    let facts: Vec<_> = rows
        .iter()
        .filter(|r| r["path"] == path)
        .map(|r| CanonExpressionFact {
            site: NativeSpanRef {
                path: path.into(),
                span: SiteSpan {
                    start_byte: usize::try_from(r["start_byte"].as_u64().unwrap()).unwrap(),
                    end_byte: usize::try_from(r["end_byte"].as_u64().unwrap()).unwrap(),
                },
            },
            source_digest: ContentDigest::from_trusted(blake3_hex(source.as_bytes())),
            sort: serde_json::from_value(r["sort"].clone()).unwrap(),
            overflow: serde_json::from_value(r["overflow"].clone()).unwrap(),
            limitations: vec![],
        })
        .collect();
    assert!(
        facts.iter().any(
            |f| source[f.site.span.start_byte..f.site.span.end_byte].contains('x')
                && f.sort == CanonSort::Bool
        ),
        "{output}"
    );
    let hasher = entmoot_core::StructuralHasher::new();
    let inventory = hasher
        .compute_granular_hash_with_analysis(source, language, None)
        .unwrap();
    let analysis = hasher
        .canon_file(
            source,
            language,
            &inventory.markers.unwrap().code_contracts,
            &facts,
            100,
        )
        .unwrap();
    assert_eq!(analysis.forms.len(), 1, "{language}");
    assert!(
        analysis.forms[0].complete(),
        "{language}: {:?}",
        analysis.forms[0].limitations
    );
    assert!(analysis.forms[0]
        .terms
        .iter()
        .any(|t| matches!(t.op, CanonOp::Input { slot: 0 })));
    analysis.forms[0].validate().unwrap();
}
