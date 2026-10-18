//! Immutable provider-role postings and conservative attachment qualification.
use super::*;
use entmoot_contracts::query::SeedAnchor;
use entmoot_contracts::{
    AttachmentDisposition, AttachmentFreshness, AttachmentTargetKind, EpistemicStatus,
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(super) struct RoleIndex {
    by_role: HashMap<String, Vec<RoleEntry>>,
}
#[derive(Debug)]
struct RoleEntry {
    assertion: usize,
    mapping: Option<(String, Vec<usize>)>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RoleSelection {
    pub matching: usize,
    pub truncated: bool,
    pub excluded_hypotheses: usize,
    pub unmapped: Vec<String>,
    pub anchors: Vec<SeedAnchor>,
}

impl RoleIndex {
    pub fn new(graph: &TopologyGraph) -> Self {
        let nodes: HashMap<_, _> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let unsupported: HashSet<_> = graph
            .unresolved_relations
            .iter()
            .filter(|r| r.reason == entmoot_contracts::UnresolvedReason::UnsupportedConstruct)
            .filter_map(|r| r.occurrence_id.as_deref())
            .collect();
        let mut attachments: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, attachment) in graph.attachments.iter().enumerate() {
            attachments
                .entry(&attachment.location_id)
                .or_default()
                .push(index);
        }
        let mut result = Self::default();
        for (index, assertion) in graph.assertions.iter().enumerate() {
            if assertion.predicate != crate::assertions::PREDICATE_PROVIDER_ROLE {
                continue;
            }
            let Some(role) = assertion.object.value.as_ref() else {
                continue;
            };
            let artifacts: HashSet<_> = assertion
                .evidence_refs
                .iter()
                .chain(assertion.derivation.iter().flat_map(|d| &d.input_refs))
                .map(|reference| reference.artifact_id.as_str())
                .collect();
            let mut candidates: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for &index in attachments
                .get(assertion.subject.entity_id.as_str())
                .into_iter()
                .flatten()
            {
                let attachment = &graph.attachments[index];
                if attachment.disposition != AttachmentDisposition::Attached
                    || attachment.freshness != AttachmentFreshness::Current
                    || !attachment.conflicts.is_empty()
                    || (!artifacts.is_empty()
                        && !artifacts.contains(attachment.artifact_id.as_str()))
                {
                    continue;
                }
                let Some(id) = attachment
                    .selected_target
                    .as_ref()
                    .and_then(|t| t.node_id.as_deref())
                else {
                    continue;
                };
                let mut node = nodes.get(id).copied();
                let grain_matches =
                    attachment
                        .selected_target
                        .as_ref()
                        .zip(node)
                        .is_some_and(|(target, node)| {
                            matches!(
                                (target.kind, node.kind),
                                (AttachmentTargetKind::Callable, NodeKind::Callable)
                                    | (AttachmentTargetKind::CallSite, NodeKind::CallStub)
                            )
                        });
                if !grain_matches {
                    continue;
                }
                let mut visited = HashSet::new();
                while let Some(current) = node {
                    if !visited.insert(current.id.as_str())
                        || unsupported.contains(current.id.as_str())
                    {
                        break;
                    }
                    if current.kind == NodeKind::Callable {
                        candidates.entry(&current.id).or_default().push(index);
                        break;
                    }
                    if current.kind != NodeKind::CallStub {
                        break;
                    }
                    node = current
                        .parent
                        .as_deref()
                        .and_then(|id| nodes.get(id).copied());
                }
            }
            let mapping = if candidates.len() == 1 {
                candidates.into_iter().next().map(|(id, mut indexes)| {
                    indexes.sort_by_key(|&i| &graph.attachments[i].attachment_id);
                    indexes.dedup_by_key(|i| &graph.attachments[*i].attachment_id);
                    (id.to_owned(), indexes)
                })
            } else {
                None
            };
            result
                .by_role
                .entry(role.clone())
                .or_default()
                .push(RoleEntry {
                    assertion: index,
                    mapping,
                });
        }
        for entries in result.by_role.values_mut() {
            entries.sort_by_key(|e| &graph.assertions[e.assertion].assertion_id);
        }
        result
    }

    pub fn select(
        &self,
        graph: &TopologyGraph,
        roles: &[&str],
        limit: usize,
        hypotheses: bool,
    ) -> RoleSelection {
        // At most two already-sorted postings (rooted or all-seed query), not a graph scan.
        let mut postings: Vec<_> = roles
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|role| self.by_role.get(*role))
            .map(|entries| entries.iter().peekable())
            .collect();
        let matching = postings.iter().map(ExactSizeIterator::len).sum();
        let mut result = RoleSelection {
            matching,
            truncated: matching > limit,
            excluded_hypotheses: 0,
            unmapped: vec![],
            anchors: vec![],
        };
        for _ in 0..limit {
            let next = postings
                .iter_mut()
                .enumerate()
                .filter_map(|(i, entries)| {
                    entries
                        .peek()
                        .map(|e| (i, &graph.assertions[e.assertion].assertion_id))
                })
                .min_by(|a, b| a.1.cmp(b.1))
                .map(|(i, _)| i);
            let Some(entry) = next.and_then(|i| postings[i].next()) else {
                break;
            };
            let assertion = &graph.assertions[entry.assertion];
            if matches!(
                assertion.epistemic_status,
                EpistemicStatus::Possible | EpistemicStatus::HeuristicallyDerived
            ) && !hypotheses
            {
                result.excluded_hypotheses += 1;
                continue;
            }
            let eligible = !matches!(
                assertion.epistemic_status,
                EpistemicStatus::Unknown | EpistemicStatus::Contradicted
            ) && assertion.validate_epistemic().is_ok()
                && assertion.derivation.as_ref().is_none_or(|d| {
                    !d.method_id.trim().is_empty() && !d.policy_version.trim().is_empty()
                });
            if let Some((id, attachments)) = entry.mapping.as_ref().filter(|_| eligible) {
                result.anchors.push(SeedAnchor {
                    assertion: assertion.clone().into(),
                    assertion_id: assertion.assertion_id.clone(),
                    location_id: assertion.subject.entity_id.clone(),
                    role: assertion.object.value.clone().unwrap_or_default(),
                    callable_id: id.clone(),
                    attachment_ids: attachments
                        .iter()
                        .map(|&i| graph.attachments[i].attachment_id.clone())
                        .collect(),
                    attachments: attachments
                        .iter()
                        .map(|&i| graph.attachments[i].clone().into())
                        .collect(),
                });
            } else {
                result.unmapped.push(assertion.assertion_id.clone());
            }
        }
        result
    }
}
