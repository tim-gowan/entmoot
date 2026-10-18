//! Disposable coverage projections (Wave C Segment 7). Never a universal risk score.

use std::collections::BTreeMap;

use entmoot_contracts::{AttachmentDisposition, AttachmentTargetKind, ScopeCompleteness};

use crate::assertions::PREDICATE_PROVIDER_ROLE;
use crate::graph::TopologyGraph;

/// Per-provider finding / attachment densities for reviewer UX.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderCoverage {
    pub producer_name: String,
    pub artifact_id: String,
    pub format: String,
    pub findings: usize,
    pub traces: usize,
    pub trace_steps: usize,
    pub attachments: usize,
    pub partial_scope: bool,
}

/// Attachment outcome histogram (selected_target kind or disposition).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentDensity {
    pub file: usize,
    pub callable: usize,
    pub call_site: usize,
    pub resource: usize,
    pub excluded: usize,
    pub unresolved: usize,
    pub invalid: usize,
    pub other: usize,
}

/// Compact coverage projection derived from normalized graph collections.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CoverageReport {
    pub findings_total: usize,
    pub traces_total: usize,
    pub trace_steps_total: usize,
    pub locations_total: usize,
    pub attachments_total: usize,
    pub assertions_total: usize,
    pub attachment_density: AttachmentDensity,
    pub findings_by_producer: BTreeMap<String, usize>,
    pub providers: Vec<ProviderCoverage>,
    /// True when any `entmoot.provider_role` assertion with an object value exists.
    pub provider_roles_present: bool,
    /// Explicit UX flag when no provider-native roles are available.
    pub no_provider_native_roles: bool,
    /// Producers whose artifact scope is explicitly partial (resolved resolver or otherwise).
    pub partial_scope_artifacts: Vec<String>,
}

/// Build a coverage report. Totals must reconcile with graph collection lengths.
pub fn build_coverage_report(graph: &TopologyGraph) -> CoverageReport {
    let mut attachment_density = AttachmentDensity::default();
    for att in &graph.attachments {
        match att.disposition {
            AttachmentDisposition::Excluded => attachment_density.excluded += 1,
            AttachmentDisposition::Unresolved => attachment_density.unresolved += 1,
            AttachmentDisposition::Invalid => attachment_density.invalid += 1,
            AttachmentDisposition::Attached => match att.selected_target.as_ref().map(|t| t.kind) {
                Some(AttachmentTargetKind::File) => attachment_density.file += 1,
                Some(AttachmentTargetKind::Callable) => attachment_density.callable += 1,
                Some(AttachmentTargetKind::CallSite) => attachment_density.call_site += 1,
                Some(AttachmentTargetKind::Resource) => attachment_density.resource += 1,
                _ => attachment_density.other += 1,
            },
        }
    }

    let mut findings_by_producer = BTreeMap::new();
    let mut providers = Vec::new();
    let mut partial_scope_artifacts = Vec::new();

    for artifact in &graph.artifacts {
        let findings = graph
            .findings
            .iter()
            .filter(|f| f.artifact_id == artifact.artifact_id)
            .count();
        *findings_by_producer
            .entry(artifact.producer.name.clone())
            .or_insert(0) += findings;

        let traces = graph
            .traces
            .iter()
            .filter(|t| {
                graph.findings.iter().any(|f| {
                    f.observation_id == t.finding_observation_id
                        && f.artifact_id == artifact.artifact_id
                })
            })
            .count();
        let trace_steps = graph
            .trace_steps
            .iter()
            .filter(|s| {
                graph.traces.iter().any(|t| {
                    t.trace_id == s.trace_id
                        && graph.findings.iter().any(|f| {
                            f.observation_id == t.finding_observation_id
                                && f.artifact_id == artifact.artifact_id
                        })
                })
            })
            .count();
        let attachments = graph
            .attachments
            .iter()
            .filter(|a| a.artifact_id == artifact.artifact_id)
            .count();

        let partial_scope = artifact.scope.completeness == ScopeCompleteness::Partial;
        if partial_scope {
            partial_scope_artifacts.push(format!(
                "{} ({})",
                artifact.producer.name, artifact.scope.profile
            ));
        }

        providers.push(ProviderCoverage {
            producer_name: artifact.producer.name.clone(),
            artifact_id: artifact.artifact_id.clone(),
            format: artifact.format.as_str().to_string(),
            findings,
            traces,
            trace_steps,
            attachments,
            partial_scope,
        });
    }

    let provider_roles_present = graph
        .assertions
        .iter()
        .any(|a| a.predicate == PREDICATE_PROVIDER_ROLE && a.object.value.is_some());

    CoverageReport {
        findings_total: graph.findings.len(),
        traces_total: graph.traces.len(),
        trace_steps_total: graph.trace_steps.len(),
        locations_total: graph.locations.len(),
        attachments_total: graph.attachments.len(),
        assertions_total: graph.assertions.len(),
        attachment_density,
        findings_by_producer,
        providers,
        provider_roles_present,
        no_provider_native_roles: !provider_roles_present,
        partial_scope_artifacts,
    }
}

impl CoverageReport {
    /// Attachment density counts must sum to attachments_total.
    pub fn attachment_density_reconciles(&self) -> bool {
        let d = &self.attachment_density;
        d.file
            + d.callable
            + d.call_site
            + d.resource
            + d.excluded
            + d.unresolved
            + d.invalid
            + d.other
            == self.attachments_total
    }

    /// Per-provider finding counts must sum to findings_total.
    pub fn findings_by_producer_reconcile(&self) -> bool {
        self.findings_by_producer.values().sum::<usize>() == self.findings_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        ArtifactFormat, ArtifactManifest, ArtifactProducer, AttachmentDisposition,
        AttachmentFreshness, AttachmentReason, AttachmentTarget, AttachmentTargetKind,
        LocationAttachment, ScopeCompleteness, ScopeUniverse,
    };

    #[test]
    fn empty_graph_reports_no_provider_roles() {
        let graph = TopologyGraph::new("C:/repo");
        let report = build_coverage_report(&graph);
        assert!(report.no_provider_native_roles);
        assert!(!report.provider_roles_present);
        assert!(report.attachment_density_reconciles());
        assert!(report.findings_by_producer_reconcile());
    }

    #[test]
    fn attachment_density_separates_file_and_callable() {
        let mut graph = TopologyGraph::new("C:/repo");
        graph.attachments.push(LocationAttachment {
            verified_span: None,
            attachment_id: "a1".into(),
            artifact_id: "art".into(),
            location_id: "l1".into(),
            disposition: AttachmentDisposition::Attached,
            selected_target: Some(AttachmentTarget {
                port: None,
                kind: AttachmentTargetKind::File,
                node_id: Some("f".into()),
                address_id: None,
            }),
            candidate_targets: vec![],
            freshness: AttachmentFreshness::Current,
            reason_codes: vec![AttachmentReason::FileFallback],
            conflicts: vec![],
            location_ref: None,
        });
        graph.attachments.push(LocationAttachment {
            verified_span: None,
            attachment_id: "a2".into(),
            artifact_id: "art".into(),
            location_id: "l2".into(),
            disposition: AttachmentDisposition::Attached,
            selected_target: Some(AttachmentTarget {
                port: None,
                kind: AttachmentTargetKind::Callable,
                node_id: Some("c".into()),
                address_id: None,
            }),
            candidate_targets: vec![],
            freshness: AttachmentFreshness::Current,
            reason_codes: vec![AttachmentReason::ExactRegionOverlap],
            conflicts: vec![],
            location_ref: None,
        });
        let report = build_coverage_report(&graph);
        assert_eq!(report.attachment_density.file, 1);
        assert_eq!(report.attachment_density.callable, 1);
        assert!(report.attachment_density_reconciles());
    }

    fn test_manifest(
        format: ArtifactFormat,
        completeness: ScopeCompleteness,
        included: Vec<String>,
        snapshot: entmoot_contracts::SourceSnapshot,
    ) -> ArtifactManifest {
        let observed = included.clone();
        let scope = ScopeUniverse::from_selectors(
            "review",
            vec!["typescript".into()],
            included,
            vec![],
            completeness,
            if completeness == ScopeCompleteness::CompleteRelativeToUniverse {
                Some(observed.as_slice())
            } else {
                None
            },
        );
        ArtifactManifest::build(
            format,
            ArtifactProducer {
                name: "fixture-producer".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            snapshot,
            scope,
            "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn complete_relative_includes_are_not_partial_scope() {
        let mut graph = TopologyGraph::new("C:/repo");
        graph.artifacts.push(test_manifest(
            ArtifactFormat::Resolver,
            ScopeCompleteness::CompleteRelativeToUniverse,
            vec!["routes/**".into(), "lib/**".into()],
            graph.snapshot.clone(),
        ));
        let report = build_coverage_report(&graph);
        assert_eq!(report.providers.len(), 1);
        assert!(!report.providers[0].partial_scope);
        assert!(report.partial_scope_artifacts.is_empty());
    }

    #[test]
    fn partial_completeness_flags_partial_scope() {
        let mut graph = TopologyGraph::new("C:/repo");
        graph.artifacts.push(test_manifest(
            ArtifactFormat::Resolver,
            ScopeCompleteness::Partial,
            vec!["routes/**".into()],
            graph.snapshot.clone(),
        ));
        let report = build_coverage_report(&graph);
        assert!(report.providers[0].partial_scope);
        assert_eq!(report.partial_scope_artifacts.len(), 1);
    }
}
