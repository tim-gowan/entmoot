//! Hasher API controls; source relations belong to revision pairs.
#![cfg(feature = "full-tests")]

use entmoot_contracts::StructuralHashPolicy;
use entmoot_core::{CoreError, StructuralHasher};

#[allow(dead_code)]
#[path = "support/revision_pairs.rs"]
mod fixture;

#[test]
fn omitted_default_and_explicit_v1_select_the_same_policy_for_every_grammar() {
    assert_eq!(StructuralHashPolicy::default(), StructuralHashPolicy::v1());
    let corpus = fixture::load();
    let hasher = StructuralHasher::new();
    let policy = StructuralHashPolicy::v1();
    let mut languages = hasher.get_supported_languages();
    languages.sort();
    assert_eq!(
        languages,
        [
            "c",
            "cpp",
            "csharp",
            "go",
            "java",
            "javascript",
            "python",
            "rust",
            "tsx",
            "typescript"
        ]
    );
    for language in languages {
        // Experimental C++ shares this simple source with the C control; this
        // tests API policy selection, not C++ feature qualification.
        let fixture_language = if language == "cpp" { "c" } else { &language };
        let pair = corpus
            .pairs
            .iter()
            .find(|pair| pair.id == format!("{fixture_language}-unchanged"))
            .unwrap_or_else(|| panic!("missing authored policy control for {language}"));
        let implicit = hasher.compute_hash(&pair.base.source, &language, None).unwrap();
        let explicit = hasher
            .compute_hash(&pair.base.source, &language, Some(&policy))
            .unwrap();
        assert_eq!(implicit, explicit, "{language}: default must select retained-CST v1");
        let digest = implicit.strip_prefix("blake3:").expect("public hash encoding");
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

#[test]
fn unsupported_grammar_is_a_typed_failure() {
    let hasher = StructuralHasher::new();
    assert!(matches!(
        hasher.compute_hash("code", "unsupported_lang", None),
        Err(CoreError::UnsupportedLanguage(_))
    ));
}
