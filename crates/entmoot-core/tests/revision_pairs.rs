#[path = "support/revision_pairs.rs"]
mod fixture;

#[test]
fn production_visitors_preserve_revision_occurrence_contracts() {
    let corpus = fixture::load();
    for pair in &corpus.pairs {
        fixture::verify(pair, &corpus.repository);
    }
}

#[test]
fn explicit_debug_ir_and_streamed_inventory_have_identical_file_identity() {
    let corpus = fixture::load();
    let hasher = entmoot_core::StructuralHasher::new();
    for pair in &corpus.pairs {
        for revision in [&pair.base, &pair.head] {
            if revision.parse == fixture::ParseOutcome::Rejected {
                continue;
            }
            let result = hasher
                .compute_granular_hash_with_analysis(&revision.source, &pair.language, None)
                .unwrap_or_else(|error| panic!("{}: {error}", pair.id));
            let (debug_hash, debug) = hasher
                .compute_hash_with_debug(&revision.source, &pair.language)
                .unwrap_or_else(|error| panic!("{}: {error}", pair.id));
            assert_eq!(debug_hash, result.file_hash, "{}", pair.id);
            assert_eq!(
                entmoot_core::granular::compute_module_shell_hash(&debug["serialized_ir"]),
                result.module_shell_hash,
                "{}",
                pair.id
            );
        }
    }
}
