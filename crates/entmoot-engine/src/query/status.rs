//! Publication status and high-signal facet finds (ADR-0022 / PLANNING D5).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use entmoot_contracts::query::{
    BindingImpactKind, CallableImpactKind, CountedStat, FanDirection, FindQuery, NamedCount,
    PublicationHealth, QueryRequest, StatusReport, StatusRequest, SuggestedSelector,
    DEFAULT_STATUS_SAMPLE_LIMIT,
};
use entmoot_contracts::{
    address_identity_key, RelationClaim, RelationClaimState, RelationClaimValue, RelationKind,
    ResolutionState, SubjectId, UnresolvedReason,
};
use serde_json::{json, Value};

use super::{bounded_limit, QueryEngine, QueryError, ResultBounds, QUERY_RESULT_SCHEMA};
use crate::{compare_revision_impact, NodeKind};

const DEFAULT_MIN_FAN_DEGREE: usize = 8;

impl QueryEngine {
    /// Optional compare publication for `changed_subject` status/find facets.
    pub fn with_compare_graph(mut self, graph: crate::TopologyGraph) -> Self {
        self.compare_graph = Some(std::sync::Arc::new(graph));
        self
    }

    /// Report publication health and bounded high-signal find selectors.
    pub fn status(&self, request: StatusRequest) -> Result<StatusReport, QueryError> {
        let max_samples = request
            .max_samples_per_suggestion
            .clamp(1, DEFAULT_STATUS_SAMPLE_LIMIT.max(100));
        let max_suggestions = request.max_suggestions.clamp(1, 64);

        let store_health = match self.repository() {
            Ok(_) => CountedStat::available(1, 1),
            Err(error) => CountedStat::unavailable(format!("store_unavailable:{error}")),
        };
        let source_health = if self.source_store.is_some() {
            CountedStat::available(1, 1)
        } else {
            CountedStat::unavailable("source_store_not_configured")
        };
        let health = PublicationHealth {
            graph: CountedStat::available(1, 1),
            store: store_health,
            source: source_health,
        };

        let mut report = StatusReport::new(self.graph.snapshot.clone(), request.clone(), health);
        report.policy_id = self.graph.policy_ids.first().cloned();
        report.adapter_profile = crate::native_resolver::adapter_profile::load_profile(
            std::path::Path::new(&self.graph.root),
        )
        .ok()
        .flatten()
        .or_else(|| {
            crate::native_resolver::adapter_profile::find_workspace_root(std::path::Path::new(
                &self.graph.root,
            ))
            .and_then(|root| {
                crate::native_resolver::adapter_profile::load_profile(&root)
                    .ok()
                    .flatten()
            })
        });
        if report.adapter_profile.is_none() {
            report.limitations.push(
                "adapter_profile_unavailable; run entmoot init to refresh the workspace adapter profile"
                    .into(),
            );
        }

        let mut entity_kinds: BTreeMap<String, u64> = BTreeMap::new();
        for node in &self.graph.nodes {
            *entity_kinds
                .entry(node_kind_name(node.kind).into())
                .or_default() += 1;
        }
        let entity_total = self.graph.nodes.len() as u64;
        report.entity_kind_counts = entity_kinds
            .into_iter()
            .map(|(name, value)| NamedCount {
                name,
                count: CountedStat::available(value, entity_total.max(1)),
            })
            .collect();

        let mut relation_kinds: BTreeMap<String, u64> = BTreeMap::new();
        for edge in &self.graph.relation_edges {
            *relation_kinds
                .entry(relation_kind_name(edge.predicate).into())
                .or_default() += 1;
        }
        let relation_total = self.graph.relation_edges.len() as u64;
        report.relation_kind_counts = relation_kinds
            .into_iter()
            .map(|(name, value)| NamedCount {
                name,
                count: CountedStat::available(value, relation_total.max(1)),
            })
            .collect();

        let mut producers: BTreeMap<String, u64> = BTreeMap::new();
        for artifact in &self.graph.artifacts {
            let key = format!(
                "{}@{}",
                artifact.producer.name,
                artifact.producer.version.as_deref().unwrap_or("unknown")
            );
            *producers.entry(key).or_default() += 1;
        }
        let producer_total = self.graph.artifacts.len() as u64;
        report.producer_partitions = producers
            .into_iter()
            .map(|(name, value)| NamedCount {
                name,
                count: CountedStat::available(value, producer_total.max(1)),
            })
            .collect();

        report.resolver_coverage_partitions = self.resolver_coverage_partitions();
        report.scan_coverage_partitions = self.scan_coverage_partitions();

        let findings_total = self.graph.findings.len() as u64;
        report.findings = CountedStat::available(findings_total, findings_total.max(1));

        let role_total = self
            .graph
            .assertions
            .iter()
            .filter(|a| a.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE)
            .count() as u64;
        report.evidence_roles = CountedStat::available(role_total, role_total.max(1));

        let mut unresolved_reasons: BTreeMap<String, u64> = BTreeMap::new();
        for attempt in &self.graph.unresolved_relations {
            *unresolved_reasons
                .entry(entmoot_contracts::unresolved_reason_key(attempt.reason).into())
                .or_default() += 1;
        }
        let unresolved_total = self.graph.unresolved_relations.len() as u64;
        report.unresolved_reasons = unresolved_reasons
            .into_iter()
            .map(|(name, value)| NamedCount {
                name,
                count: CountedStat::available(value, unresolved_total.max(1)),
            })
            .collect();

        let (conflicted, stale) = self.claim_edge_partitions();
        report.conflicted_relations =
            CountedStat::available(conflicted.len() as u64, relation_total.max(1));
        report.stale_evidence = CountedStat::available(stale.len() as u64, relation_total.max(1));
        report.queued_work = CountedStat::unavailable("queue_index_unavailable");
        report.active_budgets = vec![
            NamedCount {
                name: "default_find_page".into(),
                count: CountedStat::available(20, 100),
            },
            NamedCount {
                name: "default_walk_max_hops".into(),
                count: CountedStat::available(6, 256),
            },
        ];

        let mut suggestions = Vec::new();
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "finding",
            "indexed_finding_observations",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::Finding {
                    query: None,
                    path: None,
                    limit: Some(max_samples),
                },
            },
            self.graph
                .findings
                .iter()
                .map(|f| f.observation_id.clone())
                .take(max_samples)
                .collect(),
            findings_total,
        );
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "evidence_role",
            "provider_stated_roles",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::EvidenceRole {
                    role: None,
                    path: None,
                    limit: Some(max_samples),
                },
            },
            self.graph
                .assertions
                .iter()
                .filter(|a| a.predicate == crate::assertions::PREDICATE_PROVIDER_ROLE)
                .map(|a| a.assertion_id.clone())
                .take(max_samples)
                .collect(),
            role_total,
        );
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "unresolved_boundary",
            "open_unresolved_relation_attempts",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::UnresolvedBoundary {
                    reason: None,
                    path: None,
                    limit: Some(max_samples),
                },
            },
            self.graph
                .unresolved_relations
                .iter()
                .map(|u| u.id.to_string())
                .take(max_samples)
                .collect(),
            unresolved_total,
        );
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "conflicted_relation",
            "active_claim_disagreement",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::ConflictedRelation {
                    path: None,
                    limit: Some(max_samples),
                },
            },
            conflicted.iter().take(max_samples).cloned().collect(),
            conflicted.len() as u64,
        );
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "stale_evidence",
            "active_stale_resolution_claims",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::StaleEvidence {
                    path: None,
                    limit: Some(max_samples),
                },
            },
            stale.iter().take(max_samples).cloned().collect(),
            stale.len() as u64,
        );
        let external_ids = self.external_boundary_ids(None);
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "external_boundary",
            "cross_plane_or_dependency_relations",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::ExternalBoundary {
                    path: None,
                    limit: Some(max_samples),
                },
            },
            external_ids.iter().take(max_samples).cloned().collect(),
            external_ids.len() as u64,
        );
        let fan_ids = self.high_fan_ids(FanDirection::Either, DEFAULT_MIN_FAN_DEGREE, None);
        push_suggestion(
            &mut suggestions,
            max_suggestions,
            "high_fan_in_or_out",
            "relation_degree_above_threshold",
            QueryRequest::Find {
                output: Default::default(),
                request: FindQuery::HighFanInOrOut {
                    direction: FanDirection::Either,
                    min_degree: Some(DEFAULT_MIN_FAN_DEGREE),
                    path: None,
                    limit: Some(max_samples),
                },
            },
            fan_ids
                .iter()
                .map(|(id, _)| id.clone())
                .take(max_samples)
                .collect(),
            fan_ids.len() as u64,
        );
        match self.producer_gap_ids(None) {
            Ok(ids) => {
                push_suggestion(
                    &mut suggestions,
                    max_suggestions,
                    "producer_gap",
                    "artifacts_without_resolver_coverage_observations",
                    QueryRequest::Find {
                        output: Default::default(),
                        request: FindQuery::ProducerGap {
                            path: None,
                            limit: Some(max_samples),
                        },
                    },
                    ids.iter().take(max_samples).cloned().collect(),
                    ids.len() as u64,
                );
            }
            Err(reason) => report.limitations.push(format!("producer_gap:{reason}")),
        }

        match self.changed_subject_ids(&request, None) {
            Ok((ids, total)) => {
                if let Some(compare_revision) = request.compare_revision.clone() {
                    push_suggestion(
                        &mut suggestions,
                        max_suggestions,
                        "changed_subject",
                        "callable_body_or_boundary_differs_from_compare",
                        QueryRequest::Find {
                            output: Default::default(),
                            request: FindQuery::ChangedSubject {
                                compare_revision,
                                compare_snapshot_digest: request.compare_snapshot_digest.clone(),
                                path: None,
                                limit: Some(max_samples),
                            },
                        },
                        ids.into_iter().take(max_samples).collect(),
                        total,
                    );
                }
            }
            Err(reason) => {
                report.limitations.push(format!("changed_subject:{reason}"));
            }
        }

        if self.graph.resolved_identities.is_empty() {
            report.limitations.push("binding_evidence_unavailable: run entmoot init or entmoot bind with available solvers; external artifacts may be attached explicitly".into());
        }
        report.suggested_selectors = suggestions;
        Ok(report)
    }

    pub(super) fn find_facet(&self, request: FindQuery) -> Result<Value, QueryError> {
        match request {
            FindQuery::ChangedSubject {
                compare_revision,
                compare_snapshot_digest,
                path,
                limit,
            } => {
                let status_request = StatusRequest {
                    compare_revision: Some(compare_revision),
                    compare_snapshot_digest,
                    ..StatusRequest::default()
                };
                let (ids, total) = self
                    .changed_subject_ids(&status_request, path.as_deref())
                    .map_err(QueryError::InvalidInput)?;
                self.facet_result(
                    "changed_subject",
                    ids.into_iter()
                        .map(|id| facet_hit(id, "callable", "callable_changed_vs_compare", None))
                        .collect(),
                    total,
                    limit,
                    Vec::new(),
                )
            }
            FindQuery::UnresolvedBoundary {
                reason,
                path,
                limit,
            } => {
                let reason_filter = reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_ascii_lowercase);
                let path = normalize_path(path);
                let mut items = Vec::new();
                for attempt in &self.graph.unresolved_relations {
                    let reason_name = entmoot_contracts::unresolved_reason_key(attempt.reason);
                    if reason_filter
                        .as_ref()
                        .is_some_and(|expected| reason_name != expected)
                    {
                        continue;
                    }
                    if let Some(expected) = path.as_ref() {
                        let matches_path = self
                            .node_for_address(&attempt.source)
                            .and_then(|node| node.path.as_ref())
                            .is_some_and(|p| p == expected);
                        if !matches_path {
                            continue;
                        }
                    }
                    items.push(facet_hit(
                        attempt.id.to_string(),
                        "unresolved_relation",
                        reason_name,
                        None,
                    ));
                }
                let total = items.len() as u64;
                self.facet_result("unresolved_boundary", items, total, limit, Vec::new())
            }
            FindQuery::ExternalBoundary { path, limit } => {
                let ids = self.external_boundary_ids(path.as_deref());
                let total = ids.len() as u64;
                self.facet_result(
                    "external_boundary",
                    ids.into_iter()
                        .map(|id| {
                            facet_hit(id, "relation_edge", "external_or_dependency_boundary", None)
                        })
                        .collect(),
                    total,
                    limit,
                    Vec::new(),
                )
            }
            FindQuery::ConflictedRelation { path, limit } => {
                let (conflicted, _) = self.claim_edge_partitions();
                let path = normalize_path(path);
                let items: Vec<_> = conflicted
                    .into_iter()
                    .filter(|id| path_filter_edge(self, id, path.as_deref()))
                    .map(|id| facet_hit(id, "relation_edge", "active_claim_disagreement", None))
                    .collect();
                let total = items.len() as u64;
                self.facet_result("conflicted_relation", items, total, limit, Vec::new())
            }
            FindQuery::StaleEvidence { path, limit } => {
                let (_, stale) = self.claim_edge_partitions();
                let path = normalize_path(path);
                let items: Vec<_> = stale
                    .into_iter()
                    .filter(|id| path_filter_edge(self, id, path.as_deref()))
                    .map(|id| facet_hit(id, "relation_edge", "active_stale_claim", None))
                    .collect();
                let total = items.len() as u64;
                self.facet_result("stale_evidence", items, total, limit, Vec::new())
            }
            FindQuery::ProducerGap { path, limit } => {
                let ids = self
                    .producer_gap_ids(path.as_deref())
                    .map_err(QueryError::InvalidInput)?;
                let total = ids.len() as u64;
                self.facet_result(
                    "producer_gap",
                    ids.into_iter()
                        .map(|id| {
                            facet_hit(
                                id,
                                "callable",
                                "artifacts_without_resolver_coverage_observations",
                                None,
                            )
                        })
                        .collect(),
                    total,
                    limit,
                    Vec::new(),
                )
            }
            FindQuery::HighFanInOrOut {
                direction,
                min_degree,
                path,
                limit,
            } => {
                let min_degree = min_degree.unwrap_or(DEFAULT_MIN_FAN_DEGREE).max(1);
                let scored = self.high_fan_ids(direction, min_degree, path.as_deref());
                let total = scored.len() as u64;
                self.facet_result(
                    "high_fan_in_or_out",
                    scored
                        .into_iter()
                        .map(|(id, degree)| {
                            facet_hit(
                                id,
                                "subject",
                                format!("degree_ge_{min_degree}"),
                                Some(degree),
                            )
                        })
                        .collect(),
                    total,
                    limit,
                    Vec::new(),
                )
            }
            _ => Err(QueryError::InvalidInput(
                "find_facet requires a high-signal facet kind".into(),
            )),
        }
    }

    fn facet_result(
        &self,
        facet: &str,
        mut items: Vec<Value>,
        total: u64,
        limit: Option<usize>,
        gaps: Vec<&str>,
    ) -> Result<Value, QueryError> {
        let limit = bounded_limit(limit);
        items.sort_by(|left, right| {
            left["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["id"].as_str().unwrap_or_default())
        });
        items.dedup_by(|left, right| left["id"] == right["id"]);
        let truncated = items.len() > limit;
        items.truncate(limit);
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "find",
            "view": "facet",
            "facet": facet,
            "snapshot": self.graph.snapshot,
            "status": if truncated { "truncated" } else if total == 0 { "unknown" } else { "known" },
            "items": items,
            "gaps": gaps,
            "bounds": ResultBounds::page(limit, limit.min(usize::try_from(total).unwrap_or(usize::MAX)), usize::try_from(total).unwrap_or(usize::MAX)),
        }))
    }

    fn scan_coverage_partitions(&self) -> Vec<NamedCount> {
        let mut totals = BTreeMap::<&str, u64>::new();
        for coverage in &self.graph.coverages {
            if let entmoot_contracts::CoverageMeasurement::StructuralScan {
                parsed,
                heuristic_skipped,
                unsupported,
                failed,
                ..
            } = &coverage.measurement
            {
                for (name, count) in [
                    ("parsed", parsed),
                    ("heuristic_skipped", heuristic_skipped),
                    ("unsupported", unsupported),
                    ("failed", failed),
                ] {
                    *totals.entry(name).or_default() += count;
                }
            }
        }
        if totals.is_empty() {
            return vec![NamedCount {
                name: "structural_scan".into(),
                count: CountedStat::unavailable("no_scan_coverage_observations"),
            }];
        }
        let observed = totals.values().sum::<u64>();
        totals
            .into_iter()
            .map(|(name, count)| NamedCount {
                name: name.into(),
                count: CountedStat::available(count, observed),
            })
            .collect()
    }

    fn resolver_coverage_partitions(&self) -> Vec<NamedCount> {
        if self.graph.resolver_coverages.is_empty() {
            return vec![NamedCount {
                name: "resolver_coverage".into(),
                count: CountedStat::unavailable("no_resolver_coverage_observations"),
            }];
        }
        let mut totals = BTreeMap::<&str, u64>::new();
        let mut observed = 0u64;
        for coverage in &self.graph.resolver_coverages {
            observed = observed.saturating_add(coverage.call_sites_observed);
            *totals.entry("repository").or_default() += coverage.repository_occurrences;
            *totals.entry("indexed_dependency").or_default() +=
                coverage.indexed_dependency_occurrences;
            *totals.entry("external_without_source").or_default() +=
                coverage.external_symbols_without_source;
            *totals.entry("intrinsic").or_default() += coverage.intrinsics_without_callable_target;
            *totals.entry("unresolved_or_ambiguous").or_default() +=
                coverage.unresolved_or_ambiguous;
            *totals.entry("unsupported").or_default() += coverage.unsupported;
            *totals.entry("failed").or_default() += coverage.failed;
        }
        totals
            .into_iter()
            .map(|(name, value)| NamedCount {
                name: name.into(),
                count: CountedStat::available(value, observed.max(1)),
            })
            .collect()
    }

    fn claim_edge_partitions(&self) -> (Vec<String>, Vec<String>) {
        let mut conflicted =
            super::conflicts::explicit_conflicts(&self.graph, &self.graph.relation_claims);
        let mut stale = BTreeSet::new();
        for claim in &self.graph.relation_claims {
            if claim.state != RelationClaimState::Active {
                continue;
            }
            let Some(edge_id) = claim_edge_id(claim) else {
                continue;
            };
            match &claim.value {
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Refuted,
                } => {
                    conflicted.insert(edge_id);
                }
                RelationClaimValue::ResolutionState {
                    state: ResolutionState::Stale,
                } => {
                    stale.insert(edge_id);
                }
                _ => {}
            }
        }
        (
            conflicted.into_iter().collect(),
            stale.into_iter().collect(),
        )
    }

    fn external_boundary_ids(&self, path: Option<&str>) -> Vec<String> {
        let path = normalize_path(path.map(str::to_owned));
        let mut ids = Vec::new();
        for edge in &self.graph.relation_edges {
            if !matches!(
                edge.predicate,
                RelationKind::DependsOn
                    | RelationKind::ConnectsTo
                    | RelationKind::UsesConfiguration
                    | RelationKind::ObservedAs
            ) {
                continue;
            }
            if let Some(expected) = path.as_deref() {
                if !path_filter_edge(self, edge.id.as_str(), Some(expected)) {
                    continue;
                }
            }
            ids.push(edge.id.to_string());
        }
        for attempt in &self.graph.unresolved_relations {
            if !matches!(
                attempt.reason,
                UnresolvedReason::OutOfScope
                    | UnresolvedReason::TargetNotInInventory
                    | UnresolvedReason::ProducerNotInvoked
            ) {
                continue;
            }
            if let Some(expected) = path.as_deref() {
                let matches_path = self
                    .node_for_address(&attempt.source)
                    .and_then(|node| node.path.as_ref())
                    .is_some_and(|p| p == expected);
                if !matches_path {
                    continue;
                }
            }
            ids.push(attempt.id.to_string());
        }
        ids.sort();
        ids.dedup();
        ids
    }

    fn high_fan_ids(
        &self,
        direction: FanDirection,
        min_degree: usize,
        path: Option<&str>,
    ) -> Vec<(String, u64)> {
        let path = normalize_path(path.map(str::to_owned));
        let mut degrees: HashMap<String, (usize, usize)> = HashMap::new();
        for (key, indexes) in self.indexes.relations.iter(true) {
            degrees.entry(key.clone()).or_default().0 = indexes.len();
        }
        for (key, indexes) in self.indexes.relations.iter(false) {
            degrees.entry(key.clone()).or_default().1 = indexes.len();
        }
        let mut scored = Vec::new();
        for (key, (out_degree, in_degree)) in degrees {
            let degree = match direction {
                FanDirection::Out => out_degree,
                FanDirection::In => in_degree,
                FanDirection::Either => out_degree.max(in_degree),
            };
            if degree < min_degree {
                continue;
            }
            let Some(node) = self.node_for_address_key(&key) else {
                continue;
            };
            if path
                .as_ref()
                .is_some_and(|expected| node.path.as_ref() != Some(expected))
            {
                continue;
            }
            scored.push((node.id.clone(), degree as u64));
        }
        scored.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        scored
    }

    fn producer_gap_ids(&self, path: Option<&str>) -> Result<Vec<String>, String> {
        if self.graph.artifacts.is_empty() {
            return Err("no_attached_producer_artifacts".into());
        }
        if !self.graph.resolver_coverages.is_empty() {
            return Err("resolver_coverage_is_aggregate_not_subject_grained".into());
        }
        let path = normalize_path(path.map(str::to_owned));
        let mut ids = Vec::new();
        for node in &self.graph.nodes {
            if node.kind != NodeKind::Callable {
                continue;
            }
            if path
                .as_ref()
                .is_some_and(|expected| node.path.as_ref() != Some(expected))
            {
                continue;
            }
            ids.push(node.id.clone());
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn changed_subject_ids(
        &self,
        request: &StatusRequest,
        path: Option<&str>,
    ) -> Result<(Vec<String>, u64), String> {
        let compare_revision = request
            .compare_revision
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "compare_revision_not_provided".to_owned())?;
        let compare = self
            .compare_graph
            .as_ref()
            .ok_or_else(|| "compare_graph_not_configured".to_owned())?;
        if compare.snapshot.revision != compare_revision {
            return Err(format!(
                "compare_revision_mismatch:requested={compare_revision},configured={}",
                compare.snapshot.revision
            ));
        }
        if let Some(digest) = request.compare_snapshot_digest.as_deref() {
            if compare.snapshot.snapshot_digest.as_deref() != Some(digest) {
                return Err("compare_snapshot_digest_mismatch".into());
            }
        }
        let head = self
            .complete_graph()
            .map_err(|error| format!("compare_failed:{error}"))?;
        let impact = compare_revision_impact(compare, &head)
            .map_err(|error| format!("compare_failed:{error}"))?;
        let path = normalize_path(path.map(str::to_owned));
        let mut ids = Vec::new();
        for callable in &impact.callables {
            if matches!(
                callable.kind,
                CallableImpactKind::Introduced
                    | CallableImpactKind::Removed
                    | CallableImpactKind::BodyChanged
                    | CallableImpactKind::BoundaryChanged
                    | CallableImpactKind::BodyAndBoundaryChanged
            ) {
                if path
                    .as_ref()
                    .is_some_and(|expected| &callable.path != expected)
                {
                    continue;
                }
                if let Some(id) = callable
                    .head_node_id
                    .clone()
                    .or_else(|| callable.base_node_id.clone())
                {
                    ids.push(id);
                }
            }
        }
        let binding_changes = impact
            .bindings
            .iter()
            .filter(|row| row.kind == BindingImpactKind::Changed)
            .map(|row| row.address_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        ids.extend(
            self.graph
                .nodes
                .iter()
                .filter(|node| {
                    node.address_id
                        .as_deref()
                        .is_some_and(|address| binding_changes.contains(address))
                        && path
                            .as_ref()
                            .is_none_or(|expected| node.path.as_ref() == Some(expected))
                })
                .map(|node| node.id.clone()),
        );
        ids.sort();
        ids.dedup();
        let total = ids.len() as u64;
        Ok((ids, total))
    }

    pub(super) fn node_for_address(
        &self,
        address: &entmoot_contracts::Address,
    ) -> Option<&crate::TopologyNode> {
        self.node_for_address_key(&address_identity_key(address))
    }

    pub(super) fn node_for_address_key(&self, key: &str) -> Option<&crate::TopologyNode> {
        self.indexes
            .nodes_by_address
            .get(key)
            .and_then(|indexes| indexes.first())
            .and_then(|index| self.graph.nodes.get(*index))
    }
}

fn push_suggestion(
    suggestions: &mut Vec<SuggestedSelector>,
    max_suggestions: usize,
    facet: &str,
    reason: &str,
    request: QueryRequest,
    sample_ids: Vec<String>,
    total: u64,
) {
    if suggestions.len() >= max_suggestions || total == 0 {
        return;
    }
    suggestions.push(SuggestedSelector {
        facet: facet.into(),
        reason: reason.into(),
        request,
        sample_ids,
        match_count: CountedStat::available(total, total.max(1)),
    });
}

fn facet_hit(id: String, kind: &str, reason: impl Into<String>, score: Option<u64>) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "reason": reason.into(),
        "score": score,
    })
}

fn normalize_path(path: Option<String>) -> Option<String> {
    path.map(|value| value.trim().replace('\\', "/"))
        .filter(|value| !value.is_empty())
}

fn path_filter_edge(engine: &QueryEngine, edge_id: &str, path: Option<&str>) -> bool {
    let Some(expected) = path else {
        return true;
    };
    let Some(edge) = engine
        .graph
        .relation_edges
        .iter()
        .find(|edge| edge.id.as_str() == edge_id)
    else {
        return false;
    };
    for address in [&edge.subject, &edge.object] {
        if engine
            .node_for_address(address)
            .and_then(|node| node.path.as_ref())
            .is_some_and(|p| p == expected)
        {
            return true;
        }
    }
    false
}

fn claim_edge_id(claim: &RelationClaim) -> Option<String> {
    match &claim.subject {
        SubjectId::RelationEdge { edge_id } => Some(edge_id.to_string()),
        _ => None,
    }
}

fn node_kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Repo => "repo",
        NodeKind::File => "file",
        NodeKind::ModuleStub => "module_stub",
        NodeKind::Callable => "callable",
        NodeKind::Declaration => "declaration",
        NodeKind::CallStub => "call_stub",
        NodeKind::ControlSite => "control_site",
        NodeKind::Resource => "resource",
    }
}

fn relation_kind_name(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Calls => "calls",
        RelationKind::Imports => "imports",
        RelationKind::Implements => "implements",
        RelationKind::DependsOn => "depends_on",
        RelationKind::Selects => "selects",
        RelationKind::RoutesTo => "routes_to",
        RelationKind::UsesConfiguration => "uses_configuration",
        RelationKind::ConnectsTo => "connects_to",
        RelationKind::ObservedAs => "observed_as",
        RelationKind::HasCanonicalForm => "has_canonical_form",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository, TopologyGraph, TopologyNode,
    };
    use entmoot_contracts::query::{FanDirection, FindQuery, StatusRequest};
    use entmoot_contracts::{
        relation_edge_with_id, topology_node_address, ProducerRef, RelationClaim, RelationClaimId,
        RelationClaimState, RelationClaimValue, RelationKind, ResolutionState, Scope, SubjectId,
    };

    fn empty_store(path: &std::path::Path) {
        let mut repo = SqliteKnowledgeRepository::open(path).unwrap();
        repo.replace_all(&KnowledgeStore::default()).unwrap();
    }

    #[test]
    fn status_empty_publication_reports_denominators_and_unavailable_facets() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("empty.sqlite3");
        empty_store(&store);
        let graph = TopologyGraph::new("status/empty");
        let engine = QueryEngine::new(graph, store);
        let report = engine.status(StatusRequest::default()).unwrap();
        assert_eq!(report.schema_version, "entmoot.query.status.v1");
        assert_eq!(report.health.graph.value, Some(1));
        assert_eq!(report.health.store.value, Some(1));
        assert!(report.health.source.unavailable_reason.is_some());
        assert_eq!(report.findings.value, Some(0));
        assert!(report.suggested_selectors.is_empty());
        assert!(report
            .limitations
            .iter()
            .any(|l| l.starts_with("changed_subject:")));
        let unknown = engine
            .find(FindQuery::UnresolvedBoundary {
                reason: None,
                path: None,
                limit: Some(5),
            })
            .unwrap();
        assert_eq!(
            unknown.status,
            entmoot_contracts::query::QueryStatus::Unknown
        );
    }

    #[test]
    fn status_and_facets_surface_conflict_fan_and_unresolved() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("structural.sqlite3");
        empty_store(&store);
        let mut graph = TopologyGraph::new("status/structural");
        graph.snapshot.revision = "rev-head".into();
        for id in ["hub", "a", "b", "c", "d", "e", "f", "g", "h"] {
            graph.upsert_node(TopologyNode::shell(
                id,
                NodeKind::Callable,
                id,
                Some("src/lib.rs".into()),
            ));
        }
        let scope = Scope {
            domain: "status/structural".into(),
            snapshot_revision: Some("rev-head".into()),
            snapshot_digest: None,
            occurrence_id: None,
            context_id: None,
        };
        for target in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            graph.relation_edges.push(relation_edge_with_id(
                "source",
                topology_node_address("hub"),
                RelationKind::Calls,
                topology_node_address(target),
                scope.clone(),
                None,
            ));
        }
        let conflicted = relation_edge_with_id(
            "source",
            topology_node_address("a"),
            RelationKind::DependsOn,
            topology_node_address("b"),
            scope.clone(),
            None,
        );
        let conflict_id = conflicted.id.clone();
        graph.relation_edges.push(conflicted);
        graph.relation_claims.push(RelationClaim {
            id: RelationClaimId::from_trusted(format!("blake3:{}", "1".repeat(64))),
            subject: SubjectId::RelationEdge {
                edge_id: conflict_id.clone(),
            },
            predicate: "resolution_state".into(),
            value: RelationClaimValue::ResolutionState {
                state: ResolutionState::Refuted,
            },
            producer: ProducerRef {
                id: "fixture".into(),
                name: "fixture".into(),
                version: Some("1".into()),
                config_fingerprint: None,
                environment_fingerprint: None,
                capabilities: vec![],
            },
            scope: scope,
            evidence: vec![],
            provenance: None,
            state: RelationClaimState::Active,
            supports: vec![],
            contradicts: vec![],
            derivation: None,
        });
        let engine = QueryEngine::new(graph, store);
        let report = engine.status(StatusRequest::default()).unwrap();
        assert!(report
            .suggested_selectors
            .iter()
            .any(|s| s.facet == "high_fan_in_or_out"));
        assert!(report
            .suggested_selectors
            .iter()
            .any(|s| s.facet == "conflicted_relation"));
        assert!(report
            .suggested_selectors
            .iter()
            .any(|s| s.facet == "external_boundary"));
        let fan = engine
            .find(FindQuery::HighFanInOrOut {
                direction: FanDirection::Out,
                min_degree: Some(8),
                path: None,
                limit: Some(5),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(fan["view"], "facet");
        assert_eq!(fan["items"][0]["id"], "hub");
        let conflict = engine
            .find(FindQuery::ConflictedRelation {
                path: None,
                limit: Some(5),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(conflict["items"][0]["id"], conflict_id.to_string());
    }

    #[test]
    fn changed_subject_requires_compare_graph_wiring() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("compare.sqlite3");
        empty_store(&store);
        let engine = QueryEngine::new(TopologyGraph::new("status/compare"), store);
        let rejected = engine.find(FindQuery::ChangedSubject {
            compare_revision: "rev-base".into(),
            compare_snapshot_digest: None,
            path: None,
            limit: Some(5),
        });
        assert!(
            matches!(rejected, Err(QueryError::InvalidInput(message)) if message.contains("compare_graph_not_configured"))
        );
        let no_revision = engine.find(FindQuery::ChangedSubject {
            compare_revision: String::new(),
            compare_snapshot_digest: None,
            path: None,
            limit: Some(5),
        });
        assert!(matches!(no_revision, Err(QueryError::InvalidInput(_))));
    }
}
