//! Hash-relation assertions for the remaining whole-file language controls.
//! Production visitors own hashing; debug serialization is computed only on failure.
#![allow(dead_code)] // Each integration target imports only the helpers it needs.

use entmoot_contracts::StructuralHashRelation;
use entmoot_core::StructuralHasher;
use std::collections::HashSet;

fn assert_relation(
    hasher: &StructuralHasher,
    left: &str,
    right: &str,
    language: &str,
    relation: StructuralHashRelation,
    message: Option<&str>,
) {
    let unique = match relation {
        StructuralHashRelation::SameHash => 1,
        StructuralHashRelation::DifferentHash => 2,
    };
    assert_hash_set_size_with_debug(hasher, &[left, right], unique, language, message);
}

pub fn assert_hash_equal_with_debug(
    hasher: &StructuralHasher,
    left: &str,
    right: &str,
    language: &str,
    message: Option<&str>,
) {
    assert_relation(hasher, left, right, language, StructuralHashRelation::SameHash, message);
}

pub fn assert_hash_not_equal_with_debug(
    hasher: &StructuralHasher,
    left: &str,
    right: &str,
    language: &str,
    message: Option<&str>,
) {
    assert_relation(
        hasher,
        left,
        right,
        language,
        StructuralHashRelation::DifferentHash,
        message,
    );
}

pub fn assert_hashes_equal_with_debug(
    hasher: &StructuralHasher,
    sources: &[&str],
    language: &str,
    message: Option<&str>,
) {
    assert_hash_set_size_with_debug(hasher, sources, 1, language, message);
}

pub fn assert_hashes_not_equal_with_debug(
    hasher: &StructuralHasher,
    sources: &[&str],
    language: &str,
    message: Option<&str>,
) {
    assert_hash_set_size_with_debug(hasher, sources, sources.len(), language, message);
}

pub fn assert_hash_set_size_with_debug(
    hasher: &StructuralHasher,
    sources: &[&str],
    expected: usize,
    language: &str,
    message: Option<&str>,
) {
    let hashes: Vec<_> = sources
        .iter()
        .map(|source| {
            hasher
                .compute_hash(source, language, None)
                .unwrap_or_else(|error| panic!("{language}: {error}\n{source}"))
        })
        .collect();
    let actual = hashes.iter().collect::<HashSet<_>>().len();
    if actual == expected {
        return;
    }
    let details: Vec<_> = sources
        .iter()
        .zip(&hashes)
        .map(|(source, hash)| {
            let (_, debug) = hasher
                .compute_hash_with_debug(source, language)
                .unwrap_or_else(|error| panic!("{language}: debug failed: {error}"));
            // Character truncation keeps Unicode diagnostics valid at the limit.
            let ir: String = debug
                .get("serialized_ir")
                .map(String::as_str)
                .unwrap_or_default()
                .chars()
                .take(2000)
                .collect();
            format!("{hash}\n{source}\nRetained CST serialization (up to 2000 characters):\n{ir}")
        })
        .collect();
    panic!(
        "{language}: expected {expected} distinct hashes, got {actual}; {}\n{}",
        message.unwrap_or_default(),
        details.join("\n\n")
    );
}
