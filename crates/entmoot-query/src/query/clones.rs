//! Source-verified review candidates; similarity never changes identity or proves equivalence.
use std::collections::{BTreeMap, BTreeSet};

use crate::KnowledgeRead;
use entmoot_contracts::query::ResultBounds;
use entmoot_contracts::{
    Address, CanonAvailability, CanonObservation, RelationClaimValue, CANON_NUDGE,
};
use entmoot_contracts::{CloneGroup, CloneLevel, CloneOccurrence, CloneReport, CloneRequest};

use super::{QueryEngine, QueryError};

impl QueryEngine {
    /// Find bounded repeated syntax patterns in the indexed source snapshot.
    pub fn clones(&self, mut request: CloneRequest) -> Result<CloneReport, QueryError> {
        let path = super::selection::CompiledPath::from_request(
            request.path_selector.as_ref(),
            request.path.as_deref(),
        )?;
        request.min_tokens = request.min_tokens.clamp(5, 10_000);
        request.min_occurrences = request.min_occurrences.clamp(2, 100);
        request.max_files = request.max_files.clamp(1, 10_000);
        request.max_patterns = request.max_patterns.clamp(1, 100_000);
        request.limit = request.limit.clamp(1, 200);
        let mut report = CloneReport {
            canon: CanonAvailability::Available,
            nudge: None,
            method: "entmoot.syntax-clones.v1".into(),
            implementation: crate::implementation_fingerprint(),
            snapshot: self.graph.snapshot.clone(),
            request: request.clone(),
            groups: Vec::new(),
            bounds: ResultBounds::complete(0),
            scanned_files: 0,
            patterns: 0,
            alpha_unavailable_callables: 0,
            limitations: vec![
                "review_candidates_not_behavioral_equivalence".into(),
                "written_types_preserved_inferred_types_not_compared".into(),
            ],
        };
        let mut groups = BTreeMap::<(String, CloneLevel, String), Vec<usize>>::new();
        let mut occurrences: Vec<CloneOccurrence> = Vec::new();
        let mut exhausted = false;
        let unavailable = |mut report: CloneReport, reason| {
            report.canon = reason;
            report.nudge = Some(CANON_NUDGE.into());
            report.groups.clear();
            report
        };
        if !self.reader.exists() {
            return Ok(unavailable(report, CanonAvailability::Missing));
        }
        let repository = match self.repository() {
            Ok(repository) => repository,
            Err(QueryError::Repository(
                crate::KnowledgeRepositoryError::IncompatibleFingerprint { .. },
            )) => return Ok(unavailable(report, CanonAvailability::Incompatible)),
            Err(error) => return Err(error),
        };
        let manifests = repository.relation_claims_for_address(
            &Address::Opaque {
                system: "entmoot.canon".into(),
                value: "graph".into(),
            },
            "canon.graph",
            2,
        )?;
        let [manifest] = manifests.as_slice() else {
            return Ok(unavailable(report, CanonAvailability::Missing));
        };
        let RelationClaimValue::Canon { observation, .. } = &manifest.value else {
            return Ok(unavailable(report, CanonAvailability::Incompatible));
        };
        let CanonObservation::Graph { graph } = observation.as_ref() else {
            return Ok(unavailable(report, CanonAvailability::Incompatible));
        };
        if graph.implementation != crate::implementation_fingerprint() {
            return Ok(unavailable(report, CanonAvailability::Incompatible));
        }
        if graph.snapshot != self.graph.snapshot
            || manifest.scope.domain != graph.snapshot.repository_id
            || manifest.scope.snapshot_revision.as_ref() != Some(&graph.snapshot.revision)
            || manifest.scope.snapshot_digest != graph.snapshot.snapshot_digest
            || manifest.scope.context_id.as_deref() != Some(graph.context.as_str())
        {
            return Ok(unavailable(report, CanonAvailability::Stale));
        }
        report.limitations = vec!["review_candidates_not_behavioral_equivalence".into()];
        report.limitations.extend(
            graph
                .limitations
                .iter()
                .map(|l| format!("canon:{}", l.as_str())),
        );
        let sources: BTreeMap<_, _> = self
            .graph
            .snapshot_files
            .iter()
            .map(|f| (f.path.as_str(), f.content_digest.as_str()))
            .collect();
        exhausted |= graph
            .limitations
            .contains(&entmoot_contracts::CanonLimit::BudgetExceeded);
        for reference in &graph.files {
            if path.as_ref().is_some_and(|p| !p.matches(&reference.path)) {
                continue;
            }
            if report.scanned_files >= request.max_files || report.patterns >= request.max_patterns
            {
                report
                    .limitations
                    .push("pattern_or_file_budget_exceeded".into());
                exhausted = true;
                break;
            }
            let Some(claim) = repository.relation_claim_by_id(reference.claim_id.as_str())? else {
                return Ok(unavailable(report, CanonAvailability::Missing));
            };
            if claim.scope != manifest.scope {
                return Ok(unavailable(report, CanonAvailability::Stale));
            }
            let RelationClaimValue::Canon { observation, .. } = &claim.value else {
                return Ok(unavailable(report, CanonAvailability::Incompatible));
            };
            let CanonObservation::File { file } = observation.as_ref() else {
                return Ok(unavailable(report, CanonAvailability::Incompatible));
            };
            if path.as_ref().is_some_and(|p| !p.matches(&file.path)) {
                continue;
            }
            if file.path != reference.path {
                return Ok(unavailable(report, CanonAvailability::Incompatible));
            }
            if sources.get(file.path.as_str()).copied() != Some(file.source_digest.as_str()) {
                return Ok(unavailable(report, CanonAvailability::Stale));
            }
            report.scanned_files += 1;
            report.alpha_unavailable_callables += file.alpha_unavailable_callables;
            report.limitations.extend(
                file.limitations
                    .iter()
                    .map(|l| format!("{}:{}", file.path, l.as_str())),
            );
            exhausted |= file
                .limitations
                .contains(&entmoot_contracts::CanonLimit::BudgetExceeded);
            let canonical: BTreeMap<_, _> = file
                .forms
                .iter()
                .filter(|_| request.canonical)
                .filter(|f| f.complete())
                .map(|f| (f.span.start_byte, f.fingerprint().to_string()))
                .collect();
            for occurrence in &file.patterns {
                if report.patterns >= request.max_patterns {
                    report.limitations.push("pattern_budget_exceeded".into());
                    exhausted = true;
                    break;
                }
                report.patterns += 1;
                if occurrence.token_count < request.min_tokens
                    || (!request.include_statements
                        && occurrence.extent == entmoot_contracts::CloneExtent::Statements)
                {
                    continue;
                }
                let occurrence_id = occurrences.len();
                occurrences.push(occurrence.clone());
                let candidates = if request.canonical {
                    vec![(
                        CloneLevel::Canonical,
                        (occurrence.extent == entmoot_contracts::CloneExtent::Callable)
                            .then(|| canonical.get(&occurrence.span.start_byte))
                            .flatten(),
                    )]
                } else {
                    vec![
                        (CloneLevel::Exact, Some(&occurrence.fingerprints.exact)),
                        (CloneLevel::Syntax, Some(&occurrence.fingerprints.syntax)),
                        (CloneLevel::Alpha, occurrence.fingerprints.alpha.as_ref()),
                        (
                            CloneLevel::Parameterized,
                            occurrence.fingerprints.parameterized.as_ref(),
                        ),
                    ]
                };
                for (level, fingerprint) in candidates {
                    if let Some(fingerprint) = fingerprint {
                        groups
                            .entry((file.language.clone(), level, fingerprint.clone()))
                            .or_default()
                            .push(occurrence_id);
                    }
                }
            }
            if request.canonical && file.forms.iter().any(|f| !f.complete()) {
                report
                    .limitations
                    .push(format!("{}:partial_forms_excluded", file.path));
            }
        }
        // Rank compact membership records; only the delivered page owns member copies.
        let mut memberships = BTreeSet::new();
        let mut ranked = Vec::new();
        for ((language, level, fingerprint), mut members) in groups {
            let location = |i: usize| {
                let m = &occurrences[i];
                (m.path.as_str(), m.span.start_byte, m.span.end_byte)
            };
            members.sort_by(|a, b| location(*a).cmp(&location(*b)));
            members.dedup_by(|a, b| location(*a) == location(*b));
            if members.len() < request.min_occurrences {
                continue;
            }
            let signature: Vec<_> = members.iter().map(|&i| location(i)).collect();
            if !memberships.insert(signature) {
                continue;
            }
            let repeated = members
                .iter()
                .map(|&i| occurrences[i].token_count)
                .min()
                .unwrap_or(0)
                .saturating_mul(members.len() - 1);
            ranked.push((language, level, fingerprint, members, repeated));
        }
        ranked.sort_by(|a, b| b.4.cmp(&a.4).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let total = ranked.len();
        report.groups = ranked
            .into_iter()
            .skip(request.offset)
            .take(request.limit)
            .map(
                |(language, level, fingerprint, members, repeated_tokens)| CloneGroup {
                    language,
                    level,
                    fingerprint,
                    repeated_tokens,
                    members: members
                        .into_iter()
                        .map(|i| occurrences[i].clone())
                        .collect(),
                },
            )
            .collect();
        report.bounds =
            ResultBounds::window(request.offset, request.limit, report.groups.len(), total);
        report.bounds.truncated |= exhausted;
        report.limitations.sort();
        report.limitations.dedup();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_canon_is_unavailable_with_an_actionable_nudge() {
        let graph = crate::TopologyGraph::new("empty");
        let engine = QueryEngine::new(graph, std::path::PathBuf::from("absent.sqlite3"));
        let report = engine.clones(CloneRequest::default()).unwrap();
        assert_eq!(report.canon, entmoot_contracts::CanonAvailability::Missing);
        assert!(report.nudge.unwrap().contains("bind --experimental canon"));
        assert!(report.groups.is_empty());
    }
}
