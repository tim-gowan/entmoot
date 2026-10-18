//! Pyright-shaped fixture evidence joined to production visitor identities.
//! This is an integration control, not a real Pyright qualification receipt.

use std::collections::BTreeMap;

use entmoot_contracts::{
    topology_node_address, Address, ProviderSymbolSpelling, ResolvedIdentityCompleteness,
    ResolverCallSource, ResolverObservation, ResolverObservationOutcome, ResolverTarget,
};
use entmoot_core::{CallSiteMarker, CallableHash};
use entmoot_engine::{
    derive_resolved_identity,
    graph::{call_stub_id, callable_node_id},
    inventoried_calls_for_callable,
};

// Core owns this shared helper's formatting as well as its fixture contract.
mod fixture {
    include!("../../entmoot-core/tests/support/revision_pairs.rs");
}

fn fixture_source() -> entmoot_contracts::ResolverEvidenceRef {
    entmoot_contracts::ResolverEvidenceRef {
        producer: entmoot_contracts::ProducerIdentity {
            producer_id: "fixture.pyright-shape".into(),
            version: None,
            tool_name: None,
        },
        artifact: entmoot_contracts::ArtifactRef {
            artifact_id: entmoot_contracts::ArtifactId::from_trusted("b".repeat(64)),
            content_digest: entmoot_contracts::ContentDigest::from_trusted(
                entmoot_contracts::blake3_hex(b"revision-pair fixture observations"),
            ),
        },
    }
}

#[test]
fn claimed_family_occurrences_keep_empty_and_missing_bindings_distinct_through_storage() {
    use entmoot_contracts::{
        ArtifactFormat, ArtifactManifest, ArtifactProducer, ArtifactRef, ContentDigest,
        ProducerIdentity, ResolvedBindingEvidence, ResolverEvidenceRef, ScopeCompleteness,
        ScopeUniverse, SourceSnapshot, SourceState,
    };
    use entmoot_engine::{
        KnowledgeRead, KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository,
    };

    let corpus = fixture::load();
    let ids = [
        "python-nested-unicode",
        "rust-call-owner",
        "javascript-call-owner",
        "typescript-call-owner",
        "tsx-call-owner",
        "go-call-owner",
        "java-call-owner",
        "csharp-call-owner",
        "c-call-owner",
    ];
    for id in ids {
        let pair = corpus
            .pairs
            .iter()
            .find(|pair| pair.id == id)
            .expect("authored call-owner fixture");
        fixture::verify(pair, &corpus.repository);
        let revision = &pair.base;
        let inventory =
            fixture::inventory(pair, revision, &corpus.repository).expect("accepted source");
        // Explicitly empty fixture observations exercise missing assessment, not a native producer.
        let bytes = b"[]";
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "fixture.empty-observations".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            SourceSnapshot {
                estate_id: None,
                repository_id: corpus.repository.clone(),
                revision: "a".repeat(40),
                source_state: SourceState::WorkingTree,
                snapshot_digest: Some(entmoot_contracts::blake3_hex(revision.source.as_bytes())),
                subject: None,
                subject_aliases: Vec::new(),
            },
            ScopeUniverse::from_selectors(
                "fixture",
                vec![pair.language.clone()],
                vec![revision.path.clone()],
                vec![],
                ScopeCompleteness::Partial,
                None,
            ),
            entmoot_contracts::blake3_hex(bytes),
            None,
        )
        .expect("valid fixture manifest");
        let source = ResolverEvidenceRef {
            artifact: ArtifactRef::from_manifest(&manifest).expect("complete artifact reference"),
            producer: ProducerIdentity {
                producer_id: "fixture.empty-observations".into(),
                version: Some("1".into()),
                tool_name: None,
            },
        };
        let mut records = Vec::new();
        for selector in &revision.occurrences {
            let owner = fixture::select_key(&inventory, revision, &selector.key);
            let references = inventoried_calls_for_callable(
                inventory.markers.as_ref().expect("visitor markers"),
                owner.start_byte,
                owner.end_byte,
            );
            let expected = revision
                .calls
                .iter()
                .filter(|call| call.owner == selector.key)
                .count() as u64;
            assert_eq!(references.len() as u64, expected, "{id}/{}", selector.key);
            let record =
                derive_resolved_identity(&owner.address_id, &references, &BTreeMap::new(), &source);
            record.validate().expect("qualified contract shape");
            assert_eq!(record.binding.reference_count(), Some(expected));
            assert_eq!(
                record.binding.completeness(),
                if expected == 0 {
                    ResolvedIdentityCompleteness::Complete
                } else {
                    ResolvedIdentityCompleteness::Partial
                }
            );
            assert_eq!(record.address_id, owner.address_id);
            records.push(record);
        }
        let store = KnowledgeStore {
            artifacts: vec![manifest],
            resolved_identities: records.clone(),
            ..Default::default()
        };
        let mut repository = SqliteKnowledgeRepository::open_in_memory().expect("database");
        repository
            .replace_all(&store)
            .expect("store current evidence");
        assert_eq!(
            repository
                .load()
                .expect("read evidence")
                .semantic_fingerprint(),
            store.semantic_fingerprint()
        );
        let cursor = repository.current_visibility_cursor().expect("cursor");
        for selected in 0..records.len() {
            let mut forged = store.clone();
            let source = match &mut forged.resolved_identities[selected].binding {
                ResolvedBindingEvidence::Complete { source, .. }
                | ResolvedBindingEvidence::Partial { source, .. } => source,
                ResolvedBindingEvidence::Unavailable { .. } => panic!("fixture has an artifact"),
            };
            source.artifact.content_digest =
                ContentDigest::from_trusted(entmoot_contracts::blake3_hex(b"other bytes"));
            assert!(
                repository.replace_all(&forged).is_err(),
                "{id}: mismatched artifact digest"
            );
            assert_eq!(
                repository
                    .current_visibility_cursor()
                    .expect("unchanged cursor"),
                cursor
            );
            assert_eq!(
                repository
                    .load()
                    .expect("retained evidence")
                    .semantic_fingerprint(),
                store.semantic_fingerprint()
            );
        }
    }
}

fn observation(
    path: &str,
    owner: &CallableHash,
    site: &CallSiteMarker,
    symbol: &str,
) -> ResolverObservation {
    let occurrence_id = call_stub_id(path, site.start_byte, &site.callee);
    ResolverObservation {
        dispatch: Default::default(),
        source: ResolverCallSource {
            call_site: topology_node_address(&occurrence_id),
            relation_source: topology_node_address(&callable_node_id(
                path,
                &owner.name,
                owner.start_byte,
            )),
            syntactic_callee: site.callee.clone(),
        },
        occurrence_id,
        outcome: ResolverObservationOutcome::ExternalSymbol,
        targets: vec![ResolverTarget {
            address: Address::Opaque {
                system: "pyright.lsp.definition".into(),
                value: symbol.into(),
            },
            relation_target: None,
            provider_pointer: "/observations/0/targets/0".into(),
            provider_symbol: Some(ProviderSymbolSpelling {
                scheme: "python.qualified_name".into(),
                spelling: symbol.into(),
            }),
            provider_callable: None,
            declaration: None,
        }],
        limitation: None,
    }
}

#[test]
fn binding_change_invalidates_resolved_hash_without_changing_structure_or_occurrence() {
    let corpus = fixture::load();
    let pair = corpus
        .pairs
        .iter()
        .find(|p| p.id == "python-import-binding")
        .expect("authored pair");
    fixture::verify(pair, &corpus.repository);
    let mut resolved = Vec::new();
    for (revision, symbol) in [(&pair.base, "one.helper"), (&pair.head, "two.helper")] {
        let inventory =
            fixture::inventory(pair, revision, &corpus.repository).expect("accepted source");
        let owner = fixture::select_key(&inventory, revision, "caller");
        let markers = inventory.markers.as_ref().expect("visitor markers");
        let references = inventoried_calls_for_callable(markers, owner.start_byte, owner.end_byte);
        assert_eq!(references.len(), 1);
        let sites: Vec<_> = markers
            .call_sites
            .iter()
            .filter(|s| s.callee == "helper")
            .collect();
        assert_eq!(sites.len(), 1);
        let evidence = observation(&revision.path, owner, sites[0], symbol);
        let map = BTreeMap::from([(references[0].clone(), &evidence)]);
        let record =
            derive_resolved_identity(&owner.address_id, &references, &map, &fixture_source());
        assert_eq!(record.address_id, owner.address_id);
        assert_eq!(
            record.binding.completeness(),
            ResolvedIdentityCompleteness::Complete
        );
        let missing = derive_resolved_identity(
            &owner.address_id,
            &references,
            &BTreeMap::new(),
            &fixture_source(),
        );
        assert_eq!(
            missing.binding.completeness(),
            ResolvedIdentityCompleteness::Partial
        );
        assert!(missing
            .binding
            .reason_codes()
            .iter()
            .any(|s| s == "inventory_reference_unassessed"));
        resolved.push(record);
    }
    assert_eq!(resolved[0].address_id, resolved[1].address_id);
    assert_ne!(resolved[0].resolved_hash, resolved[1].resolved_hash);
}

#[test]
fn nested_calls_belong_only_to_the_visitor_selected_occurrence() {
    let corpus = fixture::load();
    let pair = corpus
        .pairs
        .iter()
        .find(|p| p.id == "python-nested-unicode")
        .expect("authored pair");
    let inventory =
        fixture::inventory(pair, &pair.base, &corpus.repository).expect("accepted source");
    let markers = inventory.markers.as_ref().expect("visitor markers");
    for (key, expected_callee) in [("outer", "inner"), ("inner", "helper")] {
        let owner = fixture::select_key(&inventory, &pair.base, key);
        let calls = inventoried_calls_for_callable(markers, owner.start_byte, owner.end_byte);
        assert_eq!(calls.len(), 1, "{key} must not inherit nested references");
        assert_eq!(calls[0].spelling, expected_callee);
    }
}

#[test]
fn callable_reference_ownership_is_bounded_in_every_claimed_family() {
    let corpus = fixture::load();
    let pairs: Vec<_> = corpus
        .pairs
        .iter()
        .filter(|p| p.id.ends_with("-call-owner"))
        .collect();
    assert_eq!(pairs.len(), 9, "claimed families plus TS and TSX dialects");
    for pair in pairs {
        let inventory =
            fixture::inventory(pair, &pair.base, &corpus.repository).expect("accepted source");
        let markers = inventory.markers.as_ref().expect("visitor markers");
        let owner = fixture::select_key(&inventory, &pair.base, "focus");
        let sibling = fixture::select_key(&inventory, &pair.base, "sibling");
        let calls = inventoried_calls_for_callable(markers, owner.start_byte, owner.end_byte);
        assert_eq!(calls.len(), 1, "{}: owner", pair.id);
        assert_eq!(calls[0].spelling, "sibling");
        assert!(
            inventoried_calls_for_callable(markers, sibling.start_byte, sibling.end_byte)
                .is_empty(),
            "{}: unrelated sibling",
            pair.id
        );
    }
}

#[test]
fn chained_calls_at_the_same_byte_do_not_share_binding_evidence() {
    let source = "def caller():\n    return factory()()\n";
    let inventory = entmoot_core::StructuralHasher::new()
        .compute_granular_hash_with_analysis_at(
            source,
            "python",
            Some("chain.py"),
            Some("visitor-fixtures"),
            None,
        )
        .expect("accepted chained call");
    let owner = inventory.callables.get("caller").expect("explicit caller");
    let markers = inventory.markers.as_ref().expect("visitor markers");
    let references = inventoried_calls_for_callable(markers, owner.start_byte, owner.end_byte);
    assert_eq!(references.len(), 2);
    assert_eq!(references[0].relative_offset, references[1].relative_offset);
    assert_ne!(references[0].spelling, references[1].spelling);
    let inner: Vec<_> = markers
        .call_sites
        .iter()
        .filter(|s| s.callee == "factory")
        .collect();
    assert_eq!(inner.len(), 1);
    let evidence = observation("chain.py", owner, inner[0], "module.factory");
    let key = references
        .iter()
        .find(|r| r.spelling == "factory")
        .expect("inner reference");
    let map = BTreeMap::from([(key.clone(), &evidence)]);
    let resolved =
        derive_resolved_identity(&owner.address_id, &references, &map, &fixture_source());
    assert_eq!(
        resolved.binding.completeness(),
        ResolvedIdentityCompleteness::Partial
    );
    assert!(matches!(
        resolved.binding,
        entmoot_contracts::ResolvedBindingEvidence::Partial {
            unresolved_count: 1,
            ..
        }
    ));
    assert!(resolved
        .binding
        .reason_codes()
        .iter()
        .any(|s| s == "inventory_reference_unassessed"));
}
