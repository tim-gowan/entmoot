//! Occurrence-scoped resolved identity (ADR-0024).
//!
//! Derives `resolved_hash` from inventory markers joined to Resolver observations.
//! Never rewrites `structural_hash`. Unavailable / Partial are never comparable
//! for reuse.

use std::collections::BTreeMap;

use entmoot_contracts::{
    address_identity_key, blake3_hex, ArtifactManifest, FileMarkers, ResolvedBindingEvidence,
    ResolvedIdentityRecord, ResolverEvidenceRef, ResolverObservation, ResolverObservationOutcome,
};

use crate::graph::{NodeKind, TopologyGraph};

/// Human-readable series; compatibility is established by the descriptor hash.
pub const RESOLUTION_POLICY_ID: &str = "entmoot.resolved_identity.v1";

/// Records carry this canonical descriptor's BLAKE3 fingerprint as bare hex.
const RESOLUTION_DERIVATION_DESCRIPTOR: &str = concat!(
    "entmoot.resolved_identity.v1\n",
    "reference-key=relative-offset,kind,spelling;target=canonical-occurrence,provider-symbol,purl;",
    "missing=partial;owner=visitor;callee-body=excluded"
);

/// One inventoried reference inside a callable that participates in resolved IR.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InventoriedReference {
    pub relative_offset: usize,
    pub spelling: String,
    pub kind: &'static str,
}

/// Build references owned by this callable, excluding nested callable bodies.
/// Span containment alone cannot establish ownership; the visitor records it.
pub fn inventoried_calls_for_callable(
    markers: &FileMarkers,
    callable_start: usize,
    callable_end: usize,
) -> Vec<InventoriedReference> {
    let mut refs: Vec<InventoriedReference> = markers
        .call_sites
        .iter()
        .filter(|site| {
            site.start_byte >= callable_start
                && site.end_byte <= callable_end
                && site
                    .owner
                    .as_ref()
                    .is_some_and(|owner| owner.start_byte == callable_start)
        })
        .map(|site| InventoriedReference {
            relative_offset: site.start_byte.saturating_sub(callable_start),
            spelling: site.callee.clone(),
            kind: "call",
        })
        .collect();
    refs.sort_by(|a, b| {
        a.relative_offset
            .cmp(&b.relative_offset)
            .then_with(|| a.spelling.cmp(&b.spelling))
            .then_with(|| a.kind.cmp(b.kind))
    });
    refs
}

fn outcome_token(outcome: ResolverObservationOutcome) -> &'static str {
    match outcome {
        ResolverObservationOutcome::RepositoryCallable => "repository_callable",
        ResolverObservationOutcome::RepositorySymbol => "repository_symbol",
        ResolverObservationOutcome::IndexedDependency => "indexed_dependency",
        ResolverObservationOutcome::ExternalSymbol => "external_symbol",
        ResolverObservationOutcome::Intrinsic => "intrinsic",
        ResolverObservationOutcome::Ambiguous => "ambiguous",
        ResolverObservationOutcome::Unsupported => "unsupported",
        ResolverObservationOutcome::Failed => "failed",
        ResolverObservationOutcome::Unresolved => "unresolved",
    }
}

fn binding_identity(
    observation: &ResolverObservation,
    canonical_targets: &BTreeMap<String, String>,
) -> String {
    if observation.targets.is_empty() {
        return format!(
            "none|{}|{}",
            outcome_token(observation.outcome),
            observation.limitation.as_deref().unwrap_or("")
        );
    }
    let mut parts: Vec<String> = observation
        .targets
        .iter()
        .map(|target| {
            // Projection node IDs include source spans. Commit the represented
            // occurrence address when joined, never a callee body or its position.
            // Provider addresses remain verbatim in the artifact and graph facts.
            let relation = target
                .relation_target
                .as_ref()
                .and_then(|address| canonical_targets.get(&address_identity_key(address)))
                .or_else(|| canonical_targets.get(&address_identity_key(&target.address)))
                .cloned()
                .unwrap_or_else(|| address_identity_key(&target.address));
            let spelling = target
                .provider_symbol
                .as_ref()
                .map(|s| format!("{}:{}", s.scheme, s.spelling))
                .unwrap_or_default();
            // A native spelling can be identical in different distributions or
            // versions. Its attributed package coordinate is part of the binding,
            // unlike toolchain pins, display text or the dependency's body.
            let package = target
                .provider_callable
                .as_ref()
                .and_then(|callable| callable.package.purl.as_deref())
                .unwrap_or("");
            format!(
                "{}:{relation}{}:{spelling}{}:{package}",
                relation.len(),
                spelling.len(),
                package.len()
            )
        })
        .collect();
    parts.sort();
    parts.join(";")
}

/// Derive a resolved identity for one occurrence from inventoried references and
/// observations keyed by relative offset + spelling.
pub fn derive_resolved_identity(
    address_id: &str,
    inventoried: &[InventoriedReference],
    by_reference: &BTreeMap<InventoriedReference, &ResolverObservation>,
    source: &ResolverEvidenceRef,
) -> ResolvedIdentityRecord {
    derive_with_targets(
        address_id,
        inventoried,
        by_reference,
        source,
        &BTreeMap::new(),
    )
}

fn derive_with_targets(
    address_id: &str,
    inventoried: &[InventoriedReference],
    by_reference: &BTreeMap<InventoriedReference, &ResolverObservation>,
    source: &ResolverEvidenceRef,
    canonical_targets: &BTreeMap<String, String>,
) -> ResolvedIdentityRecord {
    let mut unresolved = 0u64;
    let mut ambiguous = 0u64;
    let mut missing = 0u64;
    let mut lines = Vec::with_capacity(inventoried.len());
    for reference in inventoried {
        match by_reference.get(reference) {
            Some(observation) => {
                match observation.outcome {
                    ResolverObservationOutcome::Unresolved
                    | ResolverObservationOutcome::Unsupported
                    | ResolverObservationOutcome::Failed => unresolved += 1,
                    ResolverObservationOutcome::Ambiguous => ambiguous += 1,
                    _ => {}
                }
                lines.push(format!(
                    "{}|{}|{}|{}|{}",
                    reference.relative_offset,
                    reference.kind,
                    reference.spelling,
                    outcome_token(observation.outcome),
                    binding_identity(observation, canonical_targets)
                ));
            }
            None => {
                missing += 1;
                unresolved += 1;
                lines.push(format!(
                    "{}|{}|{}|missing|none",
                    reference.relative_offset, reference.kind, reference.spelling
                ));
            }
        }
    }
    lines.sort();
    let ir = format!(
        "resolved_identity.v1|{}|{}",
        RESOLUTION_DERIVATION_DESCRIPTOR,
        lines.join("\n")
    );
    let resolved_hash = blake3_hex(ir.as_bytes());
    // An in-scope callable with no inventoried references has a complete (empty)
    // binding set; only a missing artifact makes an identity unavailable.
    let mut reason_codes = Vec::new();
    if missing > 0 {
        reason_codes.push("inventory_reference_unassessed".into());
    }
    if unresolved > 0 {
        reason_codes.push("unresolved_or_failed_bindings".into());
    }
    if ambiguous > 0 {
        reason_codes.push("ambiguous_bindings".into());
    }
    ResolvedIdentityRecord {
        address_id: address_id.to_owned(),
        resolved_hash,
        resolution_policy_id: entmoot_contracts::strip_digest_prefix(&blake3_hex(
            RESOLUTION_DERIVATION_DESCRIPTOR.as_bytes(),
        ))
        .to_owned(),
        binding: if unresolved > 0 || ambiguous > 0 {
            ResolvedBindingEvidence::Partial {
                source: source.clone(),
                reference_count: inventoried.len() as u64,
                unresolved_count: unresolved,
                ambiguous_count: ambiguous,
                reason_codes,
            }
        } else {
            ResolvedBindingEvidence::Complete {
                source: source.clone(),
                reference_count: inventoried.len() as u64,
            }
        },
    }
}

/// Derive resolved identities for every inventoried callable inside a Resolver
/// artifact's scope, joining its observations by topology call-stub id.
///
/// Callables outside the manifest scope or in another language are skipped
/// rather than marked unavailable: absence of a record is "not assessed by this
/// artifact", which callers must not read as a negative conclusion.
pub fn derive_resolved_identities_from_graph(
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    language: &str,
    observations: &[ResolverObservation],
    source: &ResolverEvidenceRef,
) -> Vec<ResolvedIdentityRecord> {
    let canonical_targets = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let address = node
                .logic_address
                .as_ref()
                .map(|value| entmoot_contracts::Address::LogicAddress {
                    logic_address: value.clone(),
                })
                .or_else(|| {
                    node.address_id
                        .as_ref()
                        .map(|value| entmoot_contracts::Address::AddressId {
                            address_id: value.clone(),
                        })
                })?;
            Some((
                address_identity_key(&entmoot_contracts::topology_node_address(&node.id)),
                address_identity_key(&address),
            ))
        })
        .collect();
    let by_occurrence: BTreeMap<&str, &ResolverObservation> = observations
        .iter()
        .map(|observation| (observation.occurrence_id.as_str(), observation))
        .collect();

    let mut stubs_by_parent: BTreeMap<&str, Vec<&crate::graph::TopologyNode>> = BTreeMap::new();
    for node in &graph.nodes {
        if node.kind == NodeKind::CallStub {
            if let Some(parent) = node.parent.as_deref() {
                stubs_by_parent.entry(parent).or_default().push(node);
            }
        }
    }

    let mut records = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::Callable {
            continue;
        }
        let (Some(address_id), Some(path), Some(start)) = (
            node.address_id.as_deref(),
            node.path.as_deref(),
            node.start_byte,
        ) else {
            continue;
        };
        let in_language = entmoot_core::provider_language::language_matches(language, path);
        if !in_language || !manifest.path_in_scope(path).unwrap_or(false) {
            continue;
        }
        let mut inventoried = Vec::new();
        let mut by_reference: BTreeMap<InventoriedReference, &ResolverObservation> =
            BTreeMap::new();
        for stub in stubs_by_parent.get(node.id.as_str()).into_iter().flatten() {
            let Some(stub_start) = stub.start_byte else {
                continue;
            };
            let relative = stub_start.saturating_sub(start);
            let reference = InventoriedReference {
                relative_offset: relative,
                spelling: stub.label.clone(),
                kind: "call",
            };
            if let Some(observation) = by_occurrence.get(stub.id.as_str()) {
                by_reference.insert(reference.clone(), *observation);
            }
            inventoried.push(reference);
        }
        inventoried.sort_by(|a, b| {
            a.relative_offset
                .cmp(&b.relative_offset)
                .then_with(|| a.spelling.cmp(&b.spelling))
        });
        records.push(derive_with_targets(
            address_id,
            &inventoried,
            &by_reference,
            source,
            &canonical_targets,
        ));
    }
    records.sort_by(|a, b| a.address_id.cmp(&b.address_id));
    records
}

/// Unavailable record when no Resolver artifact exists for an occurrence.
#[cfg(test)]
pub fn unavailable_resolved_identity(
    address_id: &str,
    reason: impl Into<String>,
) -> ResolvedIdentityRecord {
    let reason = reason.into();
    let ir = format!(
        "resolved_identity.v1|{}|unavailable|{}",
        RESOLUTION_DERIVATION_DESCRIPTOR, reason
    );
    ResolvedIdentityRecord {
        address_id: address_id.to_owned(),
        resolved_hash: blake3_hex(ir.as_bytes()),
        resolution_policy_id: entmoot_contracts::strip_digest_prefix(&blake3_hex(
            RESOLUTION_DERIVATION_DESCRIPTOR.as_bytes(),
        ))
        .to_owned(),
        binding: ResolvedBindingEvidence::Unavailable {
            reason_codes: vec![reason],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        topology_node_address, Address, ProviderSymbolSpelling, ResolvedIdentityCompleteness,
        ResolverCallSource, ResolverTarget,
    };

    fn observation(
        id: &str,
        outcome: ResolverObservationOutcome,
        symbol: &str,
    ) -> ResolverObservation {
        let mut targets = Vec::new();
        if !matches!(
            outcome,
            ResolverObservationOutcome::Unresolved
                | ResolverObservationOutcome::Unsupported
                | ResolverObservationOutcome::Failed
        ) {
            targets.push(ResolverTarget {
                address: Address::Opaque {
                    system: "rust.compiler.symbol".into(),
                    value: symbol.into(),
                },
                relation_target: Some(topology_node_address("callee")),
                provider_pointer: "/observations/0/targets/0".into(),
                provider_symbol: Some(ProviderSymbolSpelling {
                    scheme: "rust.compiler.symbol".into(),
                    spelling: symbol.into(),
                }),
                provider_callable: None,
                declaration: None,
            });
        }
        ResolverObservation {
            dispatch: Default::default(),
            occurrence_id: id.into(),
            source: ResolverCallSource {
                call_site: topology_node_address(id),
                relation_source: topology_node_address("caller"),
                syntactic_callee: symbol.into(),
            },
            outcome,
            targets,
            limitation: None,
        }
    }

    #[test]
    fn binding_change_changes_resolved_hash() {
        let inventoried = vec![InventoriedReference {
            relative_offset: 4,
            spelling: "help".into(),
            kind: "call",
        }];
        let a = observation(
            "call",
            ResolverObservationOutcome::RepositoryCallable,
            "help",
        );
        let b = observation(
            "call",
            ResolverObservationOutcome::RepositoryCallable,
            "help_v2",
        );
        let mut map_a = BTreeMap::new();
        map_a.insert(inventoried[0].clone(), &a);
        let mut map_b = BTreeMap::new();
        map_b.insert(inventoried[0].clone(), &b);
        let ra = derive_resolved_identity(TEST_ADDRESS, &inventoried, &map_a, &source());
        let rb = derive_resolved_identity(TEST_ADDRESS, &inventoried, &map_b, &source());
        assert_eq!(
            ra.binding.completeness(),
            ResolvedIdentityCompleteness::Complete
        );
        assert_ne!(ra.resolved_hash, rb.resolved_hash);
        assert_eq!(ra.binding.source(), Some(&source()));
        ra.validate().unwrap();
    }

    const TEST_ADDRESS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn source() -> ResolverEvidenceRef {
        ResolverEvidenceRef {
            producer: entmoot_contracts::ProducerIdentity {
                producer_id: "fixture.rust".into(),
                version: Some("1".into()),
                tool_name: None,
            },
            artifact: entmoot_contracts::ArtifactRef {
                artifact_id: entmoot_contracts::ArtifactId::from_trusted("b".repeat(64)),
                content_digest: entmoot_contracts::ContentDigest::from_trusted(blake3_hex(
                    b"unit fixture observations",
                )),
            },
        }
    }

    #[test]
    fn callable_without_references_is_complete_and_empty() {
        let empty = BTreeMap::new();
        let record = derive_resolved_identity(TEST_ADDRESS, &[], &empty, &source());
        assert_eq!(
            record.binding.completeness(),
            ResolvedIdentityCompleteness::Complete
        );
        assert_eq!(record.binding.reference_count(), Some(0));
        assert_ne!(
            record.resolved_hash,
            unavailable_resolved_identity(TEST_ADDRESS, "no_artifact").resolved_hash
        );
    }

    #[test]
    fn unavailable_never_equals_complete() {
        let inventoried = vec![InventoriedReference {
            relative_offset: 0,
            spelling: "f".into(),
            kind: "call",
        }];
        let obs = observation("call", ResolverObservationOutcome::RepositoryCallable, "f");
        let mut map = BTreeMap::new();
        map.insert(inventoried[0].clone(), &obs);
        let complete = derive_resolved_identity(TEST_ADDRESS, &inventoried, &map, &source());
        let unavailable = unavailable_resolved_identity(TEST_ADDRESS, "no_artifact");
        unavailable.validate().unwrap();
        assert_ne!(complete.resolved_hash, unavailable.resolved_hash);
        assert_eq!(
            unavailable.binding.completeness(),
            ResolvedIdentityCompleteness::Unavailable
        );
    }

    #[test]
    fn missing_inventory_assessment_is_partial() {
        let inventoried = vec![InventoriedReference {
            relative_offset: 8,
            spelling: "g".into(),
            kind: "call",
        }];
        let empty = BTreeMap::new();
        let record = derive_resolved_identity(TEST_ADDRESS, &inventoried, &empty, &source());
        assert_eq!(
            record.binding.completeness(),
            ResolvedIdentityCompleteness::Partial
        );
        assert!(matches!(
            record.binding,
            ResolvedBindingEvidence::Partial {
                unresolved_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn resolved_identity_wire_digests_have_exactly_one_prefix() {
        let complete = derive_resolved_identity(TEST_ADDRESS, &[], &BTreeMap::new(), &source());
        let unavailable = unavailable_resolved_identity(TEST_ADDRESS, "no_artifact");
        for record in [complete, unavailable] {
            record.validate().unwrap();
            assert_eq!(record.resolved_hash.len(), 71);
            assert_eq!(record.resolution_policy_id.len(), 64);
            let mut doubled = record.clone();
            doubled.resolved_hash = format!("blake3:{}", doubled.resolved_hash);
            assert!(doubled.validate().unwrap_err().contains("resolved_hash"));
            let mut bad_policy = record;
            bad_policy.resolution_policy_id = format!("blake3:{}", bad_policy.resolution_policy_id);
            assert!(bad_policy
                .validate()
                .unwrap_err()
                .contains("resolution_policy_id"));
        }
    }

    #[test]
    fn binding_identity_ignores_projection_relocation_but_detects_new_occurrence() {
        let inventoried = vec![InventoriedReference {
            relative_offset: 4,
            spelling: "help".into(),
            kind: "call",
        }];
        let mut before = observation(
            "call",
            ResolverObservationOutcome::RepositoryCallable,
            "help",
        );
        before.targets[0].address = Address::LogicAddress {
            logic_address: "stable-target".into(),
        };
        let mut after = before.clone();
        after.targets[0].relation_target = Some(topology_node_address("callee-at-new-span"));
        let left = BTreeMap::from([(inventoried[0].clone(), &before)]);
        let right = BTreeMap::from([(inventoried[0].clone(), &after)]);
        assert_eq!(
            derive_resolved_identity(TEST_ADDRESS, &inventoried, &left, &source()).resolved_hash,
            derive_resolved_identity(TEST_ADDRESS, &inventoried, &right, &source()).resolved_hash
        );
        // Native RepoPath targets join through immutable inventory occurrence addresses.
        before.targets[0].address = Address::RepoPath {
            repository_id: Some("repo".into()),
            path: "app.py".into(),
            start_byte: Some(1),
            end_byte: Some(2),
        };
        after.targets[0].address = Address::RepoPath {
            repository_id: Some("repo".into()),
            path: "app.py".into(),
            start_byte: Some(100),
            end_byte: Some(200),
        };
        let canonical = BTreeMap::from([
            (
                address_identity_key(&topology_node_address("callee")),
                "logic_address:stable-target".into(),
            ),
            (
                address_identity_key(&topology_node_address("callee-at-new-span")),
                "logic_address:stable-target".into(),
            ),
        ]);
        let left = derive_with_targets(
            TEST_ADDRESS,
            &inventoried,
            &BTreeMap::from([(inventoried[0].clone(), &before)]),
            &source(),
            &canonical,
        );
        let right = derive_with_targets(
            TEST_ADDRESS,
            &inventoried,
            &BTreeMap::from([(inventoried[0].clone(), &after)]),
            &source(),
            &canonical,
        );
        assert_eq!(left.resolved_hash, right.resolved_hash);
        let changed = BTreeMap::from([(
            address_identity_key(&topology_node_address("callee-at-new-span")),
            "logic_address:different-target".into(),
        )]);
        assert_ne!(
            left.resolved_hash,
            derive_with_targets(
                TEST_ADDRESS,
                &inventoried,
                &BTreeMap::from([(inventoried[0].clone(), &after)]),
                &source(),
                &changed
            )
            .resolved_hash
        );
    }
}
