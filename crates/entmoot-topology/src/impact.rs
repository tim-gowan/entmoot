//! Deterministic two-snapshot impact comparison.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use entmoot_contracts::query::{
    BindingImpact, BindingImpactKind, BindingImpactReason, BreakingChangeSignal,
    BreakingChangeSignalKind, BreakingChangeSignals, CallableImpact, CallableImpactKind,
    EvidenceImpact, EvidenceImpactKind, FileImpact, FileImpactKind, ImpactNonSignals,
    IntroducedEvidence, RescanRequirement, RevisionImpactManifest, REVISION_IMPACT_SCHEMA,
};
use entmoot_contracts::{
    ArtifactManifest, CorrespondenceMode, CorrespondencePolicy, CorrespondenceReceipt,
    ResolvedIdentityCompleteness, ResolvedIdentityRecord, ScopeCompleteness, SourceSubject,
};
use thiserror::Error;

use crate::{NodeKind, TopologyGraph, TopologyNode};

/// Failure to compare two graph snapshots soundly.
#[derive(Debug, Error)]
pub enum ImpactError {
    /// One input graph violates its own contract.
    #[error("{side} graph is invalid: {message}")]
    InvalidGraph {
        /// Input side (`base` or `head`).
        side: &'static str,
        /// Validation failure.
        message: String,
    },
    /// No shared subject and no correspondence policy for a cross-materialization compare.
    #[error(
        "subject correspondence required: base_repository={base}, head_repository={head}; \
declare matching snapshot.subject (or subject_aliases), or pass a CorrespondencePolicy \
(--correspondence-mode relative_occurrence|explicit_alias with --subject / --alias)"
    )]
    RepositoryMismatch {
        /// Base repository identity.
        base: String,
        /// Head repository identity.
        head: String,
    },
    /// Correspondence policy is present but incomplete or contradictory.
    #[error("correspondence policy invalid: {message}")]
    CorrespondenceInvalid {
        /// Human-readable reason.
        message: String,
    },
}

/// Compare validated graph snapshots and emit a typed, conservative impact manifest.
///
/// Evidence absent from the head remains stale/unknown, never disproven. Provider
/// series are considered satisfied only when an artifact from that exact series is
/// attached to the head snapshot.
pub fn compare_revision_impact(
    base: &TopologyGraph,
    head: &TopologyGraph,
) -> Result<RevisionImpactManifest, ImpactError> {
    compare_revision_impact_with_correspondence(base, head, None)
}

/// Same as [`compare_revision_impact`], with an optional compare-time correspondence policy.
pub fn compare_revision_impact_with_correspondence(
    base: &TopologyGraph,
    head: &TopologyGraph,
    correspondence: Option<&CorrespondencePolicy>,
) -> Result<RevisionImpactManifest, ImpactError> {
    base.validate().map_err(|error| ImpactError::InvalidGraph {
        side: "base",
        message: error.to_string(),
    })?;
    head.validate().map_err(|error| ImpactError::InvalidGraph {
        side: "head",
        message: error.to_string(),
    })?;
    if let Some(policy) = correspondence {
        policy
            .validate()
            .map_err(|error| ImpactError::CorrespondenceInvalid {
                message: error.to_string(),
            })?;
    }

    let join = resolve_correspondence(base, head, correspondence)?;
    let relative = join.use_relative_occurrence;

    let files = compare_files(base, head);
    let changed_paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    let callables = compare_callables(base, head, relative)?;
    let declarations = compare_declarations(base, head);
    let (evidence, introduced_evidence) = compare_evidence(base, head);
    let bindings = compare_bindings(base, head);
    let rescans = compare_rescans(base, head, &changed_paths);
    let mut gaps = vec![
        "materialized_snapshot_comparison_only; missing relations are unknown, not non-impacts".into(),
        "applicability_ledger_not_compared; evidence staleness is not durable invalidation".into(),
        "binding_equality_is_not_reuse_authorization; build and dependency roots require revalidation".into(),
        "inventory_delta_includes_bounded_declaration_syntax; resource and component identity transitions are not compared".into(),
        "declaration_inventory_is_partial; simple_named_binding_syntax_only; no read_write_alias_runtime_value_or_absence_proof".into(),
    ];
    if relative {
        gaps.push(
            "callable_join_uses_subject_relative_occurrence_keys; inventorial logic_address hashes are unchanged"
                .into(),
        );
    }
    if base.resolved_identities.is_empty() || head.resolved_identities.is_empty() {
        gaps.push("resolved_identity_evidence_missing_on_one_or_both_snapshots".into());
    }
    if bindings
        .iter()
        .any(|row| row.kind == BindingImpactKind::NotComparable)
    {
        gaps.push(
            "binding_comparison_incomplete; consult per-occurrence reasons and source records"
                .into(),
        );
    }
    if base.policy_ids != head.policy_ids {
        gaps.push(
            "structural_policy_set_changed; callable comparisons are not proof-equivalent".into(),
        );
    }
    if base.artifacts.is_empty() {
        gaps.push("base snapshot has no attached producer artifacts".into());
    }
    if head.artifacts.is_empty() && !base.artifacts.is_empty() {
        gaps.push(
            "head snapshot has no attached producer artifacts; base evidence remains stale".into(),
        );
    }
    if base
        .nodes
        .iter()
        .chain(&head.nodes)
        .any(|node| node.kind == NodeKind::Callable && node.signature_hash.is_none())
    {
        gaps.push(
            "at least one callable lacks signature_hash; boundary-only impact may be incomplete"
                .into(),
        );
    }

    let unmatched_base = callables
        .iter()
        .filter(|row| row.kind == CallableImpactKind::Removed)
        .count();
    let unmatched_head = callables
        .iter()
        .filter(|row| row.kind == CallableImpactKind::Introduced)
        .count();
    let correspondence_receipt = Some(CorrespondenceReceipt {
        mode: join.mode,
        subject: join.subject,
        aliases_applied: join.aliases_applied,
        unmatched_base,
        unmatched_head,
    });

    Ok(RevisionImpactManifest {
        contracts: compare_code_contracts(base, head),
        schema_version: REVISION_IMPACT_SCHEMA.into(),
        implementation: base.implementation.clone(),
        base_snapshot: base.snapshot.clone(),
        head_snapshot: head.snapshot.clone(),
        correspondence: correspondence.cloned(),
        correspondence_receipt,
        base_policy_ids: base.policy_ids.clone(),
        head_policy_ids: head.policy_ids.clone(),
        breaking_change_signals: breaking_change_signals(
            base, head, &callables, &bindings, &evidence,
        ),
        files,
        callables,
        declarations,
        bindings,
        evidence,
        introduced_evidence,
        rescans,
        gaps,
    })
}

struct ResolvedJoin {
    mode: CorrespondenceMode,
    subject: Option<SourceSubject>,
    aliases_applied: Vec<String>,
    use_relative_occurrence: bool,
}

fn snapshot_subjects(snapshot: &entmoot_contracts::SourceSnapshot) -> Vec<SourceSubject> {
    let mut subjects = Vec::new();
    if let Some(subject) = &snapshot.subject {
        subjects.push(subject.clone());
    }
    subjects.extend(snapshot.subject_aliases.iter().cloned());
    subjects
}

fn apply_aliases(
    subject: &SourceSubject,
    aliases: &[entmoot_contracts::SubjectAlias],
    applied: &mut Vec<String>,
) -> SourceSubject {
    let mut current = subject.clone();
    // Fixed-point over asserted aliases (small N).
    for _ in 0..aliases.len().saturating_add(1) {
        let mut advanced = false;
        for alias in aliases {
            if current == alias.from {
                applied.push(format!("{}→{}", alias.from.display(), alias.to.display()));
                current = alias.to.clone();
                advanced = true;
                break;
            }
        }
        if !advanced {
            break;
        }
    }
    current
}

fn subjects_correspond(
    base: &entmoot_contracts::SourceSnapshot,
    head: &entmoot_contracts::SourceSnapshot,
    aliases: &[entmoot_contracts::SubjectAlias],
) -> (bool, Option<SourceSubject>, Vec<String>) {
    let mut applied = Vec::new();
    let base_subjects: Vec<_> = snapshot_subjects(base)
        .into_iter()
        .map(|s| apply_aliases(&s, aliases, &mut applied))
        .collect();
    let head_subjects: Vec<_> = snapshot_subjects(head)
        .into_iter()
        .map(|s| apply_aliases(&s, aliases, &mut applied))
        .collect();
    for left in &base_subjects {
        for right in &head_subjects {
            if left == right {
                return (true, Some(left.clone()), applied);
            }
        }
    }
    (false, None, applied)
}

fn resolve_correspondence(
    base: &TopologyGraph,
    head: &TopologyGraph,
    correspondence: Option<&CorrespondencePolicy>,
) -> Result<ResolvedJoin, ImpactError> {
    let same_repo = base.snapshot.repository_id == head.snapshot.repository_id;
    let aliases = correspondence
        .map(|policy| policy.aliases.as_slice())
        .unwrap_or(&[]);
    let (subjects_match, matched_subject, mut aliases_applied) =
        subjects_correspond(&base.snapshot, &head.snapshot, aliases);

    if let Some(override_subject) = correspondence.and_then(|p| p.subject.as_ref()) {
        return Ok(ResolvedJoin {
            mode: correspondence
                .map(|p| p.mode)
                .unwrap_or(CorrespondenceMode::RelativeOccurrence),
            subject: Some(override_subject.clone()),
            aliases_applied,
            use_relative_occurrence: !same_repo || correspondence.is_some(),
        });
    }

    if same_repo && correspondence.is_none() {
        return Ok(ResolvedJoin {
            mode: CorrespondenceMode::SubjectMatch,
            subject: matched_subject.or_else(|| base.snapshot.subject.clone()),
            aliases_applied,
            use_relative_occurrence: false,
        });
    }

    if subjects_match && correspondence.is_none() {
        return Ok(ResolvedJoin {
            mode: CorrespondenceMode::SubjectMatch,
            subject: matched_subject,
            aliases_applied,
            use_relative_occurrence: !same_repo,
        });
    }

    let Some(policy) = correspondence else {
        if subjects_match {
            return Ok(ResolvedJoin {
                mode: CorrespondenceMode::SubjectMatch,
                subject: matched_subject,
                aliases_applied,
                use_relative_occurrence: !same_repo,
            });
        }
        return Err(ImpactError::RepositoryMismatch {
            base: base.snapshot.repository_id.clone(),
            head: head.snapshot.repository_id.clone(),
        });
    };

    // Declared subjects already correspond (possibly after aliases); honor requested mode.
    if subjects_match {
        return Ok(ResolvedJoin {
            mode: policy.mode,
            subject: matched_subject,
            aliases_applied,
            use_relative_occurrence: !same_repo || !aliases.is_empty(),
        });
    }

    match policy.mode {
        CorrespondenceMode::SubjectMatch => Err(ImpactError::CorrespondenceInvalid {
            message: format!(
                "subjects do not match (base={:?}, head={:?}); use relative_occurrence or explicit_alias",
                base.snapshot.subject.as_ref().map(SourceSubject::display),
                head.snapshot.subject.as_ref().map(SourceSubject::display)
            ),
        }),
        CorrespondenceMode::RelativeOccurrence => {
            let subject = base
                .snapshot
                .subject
                .clone()
                .or_else(|| head.snapshot.subject.clone());
            if subject.is_none() && !same_repo {
                return Err(ImpactError::CorrespondenceInvalid {
                    message: "relative_occurrence requires declared subject on at least one snapshot or --subject override"
                        .into(),
                });
            }
            Ok(ResolvedJoin {
                mode: CorrespondenceMode::RelativeOccurrence,
                subject,
                aliases_applied,
                use_relative_occurrence: true,
            })
        }
        CorrespondenceMode::ExplicitAlias => {
            if policy.aliases.is_empty() {
                return Err(ImpactError::CorrespondenceInvalid {
                    message: "explicit_alias requires at least one --alias from=to pair".into(),
                });
            }
            let (ok, subject, applied) =
                subjects_correspond(&base.snapshot, &head.snapshot, &policy.aliases);
            aliases_applied = applied;
            if !ok && !same_repo {
                // Allow relative join under asserted aliases even when snapshots omit subject
                // when aliases alone map repository-derived subjects — still require a to-side.
                if let Some(to) = policy.aliases.first().map(|a| a.to.clone()) {
                    return Ok(ResolvedJoin {
                        mode: CorrespondenceMode::ExplicitAlias,
                        subject: Some(to),
                        aliases_applied,
                        use_relative_occurrence: true,
                    });
                }
                return Err(ImpactError::CorrespondenceInvalid {
                    message: "explicit_alias did not produce a shared subject".into(),
                });
            }
            Ok(ResolvedJoin {
                mode: CorrespondenceMode::ExplicitAlias,
                subject,
                aliases_applied,
                use_relative_occurrence: true,
            })
        }
    }
}

/// Subject-relative occurrence key: `lang|path|scope|symbol|kind` (repo segment stripped).
pub fn relative_occurrence_key(logic_address: &str) -> Option<String> {
    let parts: Vec<&str> = logic_address.split('|').collect();
    if parts.len() < 7 || parts[0] != "logic" {
        return None;
    }
    // logic|repo|lang|path|scope|symbol|kind[|start|end]
    Some(parts[2..7].join("|"))
}

fn callable_join_key(logic_address: &str, relative: bool) -> String {
    if relative {
        relative_occurrence_key(logic_address).unwrap_or_else(|| logic_address.to_owned())
    } else {
        logic_address.to_owned()
    }
}

fn compare_code_contracts(
    base: &TopologyGraph,
    head: &TopologyGraph,
) -> Vec<entmoot_contracts::query::CodeContractImpact> {
    use entmoot_contracts::query::{CodeContractImpact, CodeContractImpactKind};
    fn map(graph: &TopologyGraph) -> BTreeMap<&str, &TopologyNode> {
        graph
            .nodes
            .iter()
            .filter_map(|n| n.address_id.as_deref().map(|id| (id, n)))
            .collect()
    }
    let before = map(base);
    let after = map(head);
    let mut incoming: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for edge in &head.edges {
        if edge.kind == crate::EdgeKind::Calls {
            incoming
                .entry(&edge.to)
                .or_default()
                .insert(edge.from.clone());
        }
    }
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| {
            let left = before.get(id);
            let right = after.get(id);
            let a = left.and_then(|n| n.code_contract.as_deref());
            let b = right.and_then(|n| n.code_contract.as_deref());
            let kind = match (a, b) {
                (None, None) => return None,
                (Some(a), Some(b)) if a.boundary_key() == b.boundary_key() => return None,
                (Some(_), Some(_)) => CodeContractImpactKind::Changed,
                (None, Some(_)) if left.is_none() => CodeContractImpactKind::Introduced,
                (Some(_), None) if right.is_none() => CodeContractImpactKind::Removed,
                _ => CodeContractImpactKind::NotComparable,
            };
            let source = b.or(a)?;
            Some(CodeContractImpact {
                occurrence: source.site.occurrence.clone(),
                path: right.or(left)?.path.clone().unwrap_or_default(),
                kind,
                base: a.cloned(),
                head: b.cloned(),
                related_callers: right
                    .and_then(|n| incoming.get(n.id.as_str()))
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn compare_declarations(
    base: &TopologyGraph,
    head: &TopologyGraph,
) -> Vec<entmoot_contracts::query::DeclarationImpact> {
    use entmoot_contracts::query::{DeclarationImpact, DeclarationImpactKind};
    fn entries(
        graph: &TopologyGraph,
    ) -> BTreeMap<&str, (&str, &entmoot_contracts::DeclarationMarker)> {
        graph
            .nodes
            .iter()
            .filter_map(|node| {
                Some((
                    node.address_id.as_deref()?,
                    (node.path.as_deref()?, node.declaration.as_deref()?),
                ))
            })
            .collect()
    }
    let left = entries(base);
    let right = entries(head);
    left.keys()
        .chain(right.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| {
            let before = left.get(id);
            let after = right.get(id);
            let kind = match (before, after) {
                (Some((_, b)), Some((_, h))) => {
                    // Relocation is not an initializer edit. Compare content commitments,
                    // not snapshot-local span positions or file bytes outside the initializer.
                    let digest = |marker: &entmoot_contracts::DeclarationMarker| {
                        marker
                            .syntax
                            .as_ref()
                            .and_then(|s| s.initializer.as_ref())
                            .map(|i| i.content_digest.clone())
                    };
                    if digest(b) == digest(h) {
                        return None;
                    }
                    DeclarationImpactKind::InitializerChanged
                }
                (Some(_), None) => DeclarationImpactKind::ObservedOnlyInBase,
                (None, Some(_)) => DeclarationImpactKind::ObservedOnlyInHead,
                (None, None) => return None,
            };
            Some(DeclarationImpact {
                address_id: id.to_owned(),
                path: before.or(after)?.0.to_owned(),
                kind,
                base: before.map(|(_, d)| (*d).clone()),
                head: after.map(|(_, d)| (*d).clone()),
            })
        })
        .collect()
}

fn breaking_change_signals(
    base: &TopologyGraph,
    head: &TopologyGraph,
    callables: &[CallableImpact],
    bindings: &[BindingImpact],
    evidence: &[EvidenceImpact],
) -> BreakingChangeSignals {
    let mut limitations = vec![
        "signals_are_review_hints_not_semver_compatibility_exploitability_or_vulnerability_fixed_verdicts".into(),
        "consumer_exposure_and_public_export_contracts_not_established".into(),
        "removed_occurrence_is_not_a_proven_rename_or_cross_language_move".into(),
        "empty_signals_do_not_establish_compatibility; missing_binding_evidence_remains_unknown".into(),
    ];
    let policy_comparable = !base.policy_ids.is_empty() && base.policy_ids == head.policy_ids;
    if !policy_comparable {
        limitations.push("signals_suppressed: structural_policy_missing_or_changed".into());
    }
    let unavailable = |graph: &TopologyGraph, path: &str| {
        graph
            .diagnostics
            .iter()
            .any(|d| d.code.starts_with("source.") && d.provider_pointer.as_deref() == Some(path))
    };
    let mut signals = Vec::new();
    for row in callables {
        let kind = match row.kind {
            CallableImpactKind::BoundaryChanged | CallableImpactKind::BodyAndBoundaryChanged
                if row.base_signature_hash.is_some() && row.head_signature_hash.is_some() =>
            {
                Some(BreakingChangeSignalKind::BoundaryChanged)
            }
            CallableImpactKind::Removed if row.base_signature_hash.is_some() => {
                Some(BreakingChangeSignalKind::CallableRemoved)
            }
            _ => None,
        };
        if let Some(kind) = kind {
            if !policy_comparable || unavailable(base, &row.path) || unavailable(head, &row.path) {
                limitations.push(format!(
                    "signal_unknown_due_to_inventory_or_policy_gap:{}",
                    row.occurrence.logic_address
                ));
                continue;
            }
            let description = match kind {
                BreakingChangeSignalKind::BoundaryChanged => "Callable boundary changed",
                BreakingChangeSignalKind::CallableRemoved => "Callable occurrence removed",
            };
            signals.push(BreakingChangeSignal {
                kind,
                summary: format!(
                    "{description}: {}; inspect consumer contracts",
                    row.occurrence.logic_address
                ),
                callable: row.clone(),
            });
        }
    }
    BreakingChangeSignals {
        derivation_policy: "entmoot.impact.boundary_signals.v1".into(),
        signals,
        non_signals: ImpactNonSignals {
            body_only_changes: callables
                .iter()
                .filter(|r| r.kind == CallableImpactKind::BodyChanged)
                .count(),
            binding_changes: bindings
                .iter()
                .filter(|r| r.kind == BindingImpactKind::Changed)
                .count(),
            bindings_not_comparable: bindings
                .iter()
                .filter(|r| r.kind == BindingImpactKind::NotComparable)
                .count(),
            stale_evidence_records: evidence
                .iter()
                .filter(|r| r.disposition != EvidenceImpactKind::Retained)
                .count(),
            base_unresolved_attempts: base.unresolved_relations.len(),
            head_unresolved_attempts: head.unresolved_relations.len(),
        },
        limitations,
    }
}

fn compare_files(base: &TopologyGraph, head: &TopologyGraph) -> Vec<FileImpact> {
    let base_files = base
        .snapshot_files
        .iter()
        .map(|file| (file.path.as_str(), file.content_digest.as_str()))
        .collect::<BTreeMap<_, _>>();
    let head_files = head
        .snapshot_files
        .iter()
        .map(|file| (file.path.as_str(), file.content_digest.as_str()))
        .collect::<BTreeMap<_, _>>();
    base_files
        .keys()
        .chain(head_files.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|path| {
            let base_digest = base_files.get(path).copied();
            let head_digest = head_files.get(path).copied();
            let kind = match (base_digest, head_digest) {
                (None, Some(_)) => FileImpactKind::Added,
                (Some(_), None) => FileImpactKind::Removed,
                (Some(left), Some(right)) if left != right => FileImpactKind::Modified,
                _ => return None,
            };
            Some(FileImpact {
                path: path.into(),
                kind,
                base_content_digest: base_digest.map(str::to_owned),
                head_content_digest: head_digest.map(str::to_owned),
            })
        })
        .collect()
}

fn callable_map(graph: &TopologyGraph, relative: bool) -> BTreeMap<String, &TopologyNode> {
    graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Callable)
        .filter_map(|node| {
            let address = node.logic_address.as_deref()?;
            Some((callable_join_key(address, relative), node))
        })
        .collect()
}

fn compare_callables(
    base: &TopologyGraph,
    head: &TopologyGraph,
    relative: bool,
) -> Result<Vec<CallableImpact>, ImpactError> {
    let base_nodes = callable_map(base, relative);
    let head_nodes = callable_map(head, relative);
    base_nodes
        .keys()
        .chain(head_nodes.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|join_key| {
            let base_node = base_nodes.get(&join_key).copied();
            let head_node = head_nodes.get(&join_key).copied();
            let kind = match (base_node, head_node) {
                (None, Some(_)) => CallableImpactKind::Introduced,
                (Some(_), None) => CallableImpactKind::Removed,
                (Some(left), Some(right))
                    if left.structural_hash != right.structural_hash
                        && left.signature_hash != right.signature_hash =>
                {
                    CallableImpactKind::BodyAndBoundaryChanged
                }
                (Some(left), Some(right)) if left.structural_hash != right.structural_hash => {
                    CallableImpactKind::BodyChanged
                }
                (Some(left), Some(right)) if left.signature_hash != right.signature_hash => {
                    CallableImpactKind::BoundaryChanged
                }
                _ => return None,
            };
            let path = head_node
                .and_then(|node| node.path.clone())
                .or_else(|| base_node.and_then(|node| node.path.clone()))
                .unwrap_or_default();
            let source = head_node.or(base_node);
            let logic_address = source
                .and_then(|node| node.logic_address.clone())
                .unwrap_or(join_key);
            let occurrence = entmoot_contracts::AddressId::parse(
                source
                    .and_then(|node| node.address_id.as_deref())
                    .unwrap_or_default(),
            )
            .map(|address_id| entmoot_contracts::OccurrenceRef {
                address_id,
                logic_address,
            })
            .map_err(|error| ImpactError::InvalidGraph {
                side: if head_node.is_some() { "head" } else { "base" },
                message: format!("invalid callable occurrence: {error}"),
            });
            Some(occurrence.map(|occurrence| CallableImpact {
                occurrence,
                path,
                kind,
                base_node_id: base_node.map(|node| node.id.clone()),
                head_node_id: head_node.map(|node| node.id.clone()),
                base_structural_hash: base_node.and_then(|node| node.structural_hash.clone()),
                head_structural_hash: head_node.and_then(|node| node.structural_hash.clone()),
                base_signature_hash: base_node.and_then(|node| node.signature_hash.clone()),
                head_signature_hash: head_node.and_then(|node| node.signature_hash.clone()),
            }))
        })
        .collect()
}

type EvidenceKey = (String, String);

fn evidence_ids(graph: &TopologyGraph) -> BTreeSet<EvidenceKey> {
    let mut ids = BTreeSet::new();
    macro_rules! records {
        ($kind:literal, $rows:expr, $id:ident) => {
            ids.extend($rows.iter().map(|row| ($kind.into(), row.$id.to_string())));
        };
    }
    records!("finding_observation", graph.findings, observation_id);
    records!("location_attachment", graph.attachments, attachment_id);
    records!(
        "component_vulnerability",
        graph.component_vulnerabilities,
        observation_id
    );
    records!("assertion", graph.assertions, assertion_id);
    records!("relation_edge", graph.relation_edges, id);
    records!("relation_claim", graph.relation_claims, id);
    records!("unresolved_relation", graph.unresolved_relations, id);
    records!("resolution", graph.resolutions, resolution_id);
    records!("resolver_coverage", graph.resolver_coverages, coverage_id);
    records!("artifact", graph.artifacts, artifact_id);
    ids.extend(
        graph
            .resolved_identities
            .iter()
            .map(|row| ("resolved_identity".into(), row.record_id())),
    );
    ids
}

fn compare_evidence(
    base: &TopologyGraph,
    head: &TopologyGraph,
) -> (Vec<EvidenceImpact>, Vec<IntroducedEvidence>) {
    let base_ids = evidence_ids(base);
    let head_ids = evidence_ids(head);
    let head_propositions = head
        .relation_edges
        .iter()
        .map(|row| row.proposition_id.as_str())
        .collect::<HashSet<_>>();
    let base_propositions = base
        .relation_edges
        .iter()
        .map(|row| (row.id.as_str(), row.proposition_id.as_str()))
        .collect::<HashMap<_, _>>();
    let evidence = base_ids
        .iter()
        .map(|(kind, id)| {
            let proposition = (kind == "relation_edge")
                .then(|| base_propositions.get(id.as_str()).copied())
                .flatten()
                .filter(|value| head_propositions.contains(value));
            EvidenceImpact {
                record_kind: kind.clone(),
                base_record_id: id.clone(),
                disposition: if head_ids.contains(&(kind.clone(), id.clone())) {
                    EvidenceImpactKind::Retained
                } else if proposition.is_some() {
                    EvidenceImpactKind::PropositionRetainedClaimStale
                } else {
                    EvidenceImpactKind::Stale
                },
                proposition_id: proposition.map(str::to_owned),
            }
        })
        .collect();
    let introduced = head_ids
        .difference(&base_ids)
        .map(|(kind, id)| IntroducedEvidence {
            record_kind: kind.clone(),
            head_record_id: id.clone(),
        })
        .collect();
    (evidence, introduced)
}

fn binding_groups(graph: &TopologyGraph) -> BTreeMap<&str, Vec<ResolvedIdentityRecord>> {
    let mut groups: BTreeMap<&str, Vec<ResolvedIdentityRecord>> = BTreeMap::new();
    for row in &graph.resolved_identities {
        groups
            .entry(row.address_id.as_str())
            .or_default()
            .push(row.clone());
    }
    for rows in groups.values_mut() {
        rows.sort_by_cached_key(ResolvedIdentityRecord::record_id);
    }
    groups
}

// A series commits the declared configuration class/scope, not all environment
// inputs; artifact IDs themselves change per snapshot. Equality is not reuse.
// Do not select a winner among multiple observations from one series.
type BindingValues<'a> = BTreeMap<(&'a str, &'a str, Option<&'a str>, &'a str), &'a str>;

fn binding_values<'a>(
    rows: &'a [ResolvedIdentityRecord],
    series: &HashMap<&str, &'a str>,
) -> Option<BindingValues<'a>> {
    let mut values = BTreeMap::new();
    for row in rows {
        let source = row.binding.source()?;
        let series_id = series.get(source.artifact.artifact_id.as_str())?;
        let key = (
            *series_id,
            source.producer.producer_id.as_str(),
            source.producer.version.as_deref(),
            row.resolution_policy_id.as_str(),
        );
        if values.insert(key, row.resolved_hash.as_str()).is_some() {
            return None;
        }
    }
    Some(values)
}

fn compare_bindings(base: &TopologyGraph, head: &TopologyGraph) -> Vec<BindingImpact> {
    let mut base_groups = binding_groups(base);
    let mut head_groups = binding_groups(head);
    let addresses = base_groups
        .keys()
        .chain(head_groups.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let base_series = base
        .artifacts
        .iter()
        .map(|row| (row.artifact_id.as_str(), row.artifact_series_id.as_str()))
        .collect();
    let head_series = head
        .artifacts
        .iter()
        .map(|row| (row.artifact_id.as_str(), row.artifact_series_id.as_str()))
        .collect();
    addresses
        .into_iter()
        .map(|address| {
            let left = base_groups.remove(address).unwrap_or_default();
            let right = head_groups.remove(address).unwrap_or_default();
            let mut reasons = Vec::new();
            if left.is_empty() || right.is_empty() {
                reasons.push(BindingImpactReason::MissingEvidence);
            }
            if left
                .iter()
                .chain(&right)
                .any(|row| row.binding.completeness() != ResolvedIdentityCompleteness::Complete)
            {
                reasons.push(BindingImpactReason::IncompleteEvidence);
            }
            if base.policy_ids != head.policy_ids {
                reasons.push(BindingImpactReason::StructuralPolicySetChanged);
            }
            let left_values = binding_values(&left, &base_series);
            let right_values = binding_values(&right, &head_series);
            let same_series = matches!((&left_values, &right_values), (Some(l), Some(r)) if l.keys().eq(r.keys()));
            // Missing/partial assessments are not evidence of a producer change.
            // Only compare series when both sides actually supply records.
            if !left.is_empty() && !right.is_empty() && !same_series {
                reasons.push(BindingImpactReason::ProducerSeriesOrPolicyNotComparable);
            }
            let kind = match (&left_values, &right_values) {
                (Some(l), Some(r)) if same_series && reasons.is_empty() => {
                    if l == r {
                        BindingImpactKind::Unchanged
                    } else {
                        BindingImpactKind::Changed
                    }
                }
                _ => BindingImpactKind::NotComparable,
            };
            BindingImpact {
                address_id: address.into(),
                kind,
                base: left,
                head: right,
                reasons,
            }
        })
        .collect()
}

fn compare_rescans(
    base: &TopologyGraph,
    head: &TopologyGraph,
    changed_paths: &[&str],
) -> Vec<RescanRequirement> {
    let mut head_by_series: HashMap<&str, &ArtifactManifest> = HashMap::new();
    for artifact in &head.artifacts {
        if artifact.scope.completeness != ScopeCompleteness::CompleteRelativeToUniverse {
            continue;
        }
        head_by_series
            .entry(artifact.artifact_series_id.as_str())
            .and_modify(|selected| {
                if artifact.artifact_id < selected.artifact_id {
                    *selected = artifact;
                }
            })
            .or_insert(artifact);
    }
    let mut result = base
        .artifacts
        .iter()
        .filter_map(|artifact| {
            let affected_paths = changed_paths
                .iter()
                .filter(|path| artifact.scope.path_in_scope(path).unwrap_or(false))
                .map(|path| (*path).to_owned())
                .collect::<Vec<_>>();
            if affected_paths.is_empty() {
                return None;
            }
            let head_artifact_id = head_by_series
                .get(artifact.artifact_series_id.as_str())
                .map(|value| value.artifact_id.clone());
            Some(RescanRequirement {
                artifact_series_id: artifact.artifact_series_id.clone(),
                producer_name: artifact.producer.name.clone(),
                format: artifact.format,
                affected_paths,
                satisfied_by_head_artifact: head_artifact_id.is_some(),
                head_artifact_id,
            })
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.artifact_series_id.cmp(&right.artifact_series_id));
    result.dedup_by(|left, right| left.artifact_series_id == right.artifact_series_id);
    result
}
