//! Always-on human summary for init/bind (stderr). Not filtered by RUST_LOG.
use entmoot_contracts::package_api::PackageApiEvidence;
use entmoot_engine::workspace::WorkspaceSummary;
use std::fmt::Write as _;
use std::path::Path;

/// Format a multi-line operator summary for stderr. Display-only; does not alter the receipt.
pub(crate) fn format_workspace_summary(command: &str, summary: &WorkspaceSummary) -> String {
    let incomplete = is_incomplete(summary);
    let partial = summary.stages.iter().any(|s| {
        s.native_receipt
            .as_ref()
            .is_some_and(|r| r.semantic_status.as_str() == "partial")
    });
    let banner = if incomplete {
        "INCOMPLETE"
    } else if partial {
        "PARTIAL"
    } else {
        "OK"
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "entmoot {command} complete [{banner}] outcome={} wall={:.1}s",
        summary.publication_outcome, summary.wall_seconds
    );
    let root = display_path(&summary.publication.source_root);
    let _ = writeln!(out, "  source_root={root}");
    let _ = writeln!(
        out,
        "  repo={} revision={}",
        summary.snapshot.repository_id, summary.snapshot.revision
    );
    let mut package_refs = 0usize;
    for stage in &summary.stages {
        let status = stage
            .native_receipt
            .as_ref()
            .map(|r| r.semantic_status.as_str())
            .unwrap_or("—");
        let wall = stage
            .native_receipt
            .as_ref()
            .map(|r| format!("{:.1}s", r.wall_seconds))
            .unwrap_or_else(|| "—".into());
        let (unresolved, resolved) = stage
            .attachment
            .as_ref()
            .map(|a| (a.unresolved, a.resolved_identities))
            .unwrap_or((0, 0));
        let _ = writeln!(
            out,
            "  stage family={} stage={} semantic={status} wall={wall} unresolved={unresolved} resolved_identities={resolved}",
            stage.family, stage.stage
        );
        if let Some(receipt) = &stage.native_receipt {
            let _ = writeln!(
                out,
                "    calls observed={} inventory={}",
                receipt.observed_calls, receipt.inventory_calls
            );
            for (gap, count) in &receipt.call_gaps {
                let _ = writeln!(out, "    call_gap={gap} count={count}");
            }
        }
        if let Some(limit) = first_actionable_limitation(stage) {
            let _ = writeln!(out, "    limitation={limit}");
        }
        if let Some(counts) = package_api_counts(stage.manifest.as_deref()) {
            package_refs += counts.refs;
            let mut line = format!(
                "    package_api surfaces={} refs={}",
                counts.surfaces, counts.refs
            );
            if counts.unassessed > 0 {
                let _ = write!(line, " unassessed={}", counts.unassessed);
            }
            let _ = writeln!(out, "{line}");
        }
    }
    if package_refs > 0 {
        let _ = writeln!(
            out,
            "  next: entmoot assess compatibility --provider <other-checkout>"
        );
    } else {
        let _ = writeln!(out, "  next: entmoot status");
    }
    if incomplete {
        let _ = writeln!(
            out,
            "  tip: fix adapter profile gaps (adapters / --features producer), then entmoot bind; use bind --structural-only to waive"
        );
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct PackageApiCounts {
    surfaces: usize,
    refs: usize,
    unassessed: usize,
}

/// Display-only: load `package_api` counts from the stage resolver artifact when present.
fn package_api_counts(manifest: Option<&Path>) -> Option<PackageApiCounts> {
    let manifest = manifest?;
    let artifact_path = manifest
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".manifest.json"))
        .map(|stem| manifest.with_file_name(stem))
        .unwrap_or_else(|| manifest.with_file_name("resolver.json"));
    let bytes = std::fs::read(&artifact_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let api: PackageApiEvidence = serde_json::from_value(value.get("package_api")?.clone()).ok()?;
    let unassessed = api
        .references
        .iter()
        .filter(|reference| reference.boundary_policy == "unassessed")
        .count();
    Some(PackageApiCounts {
        surfaces: api.surfaces.len(),
        refs: api.references.len(),
        unassessed,
    })
}

pub(crate) fn write_workspace_summary(command: &str, summary: &WorkspaceSummary) {
    eprint!("{}", format_workspace_summary(command, summary));
}

fn is_incomplete(summary: &WorkspaceSummary) -> bool {
    for stage in &summary.stages {
        if stage.stage == "failed" {
            return true;
        }
        if summary.structural_only && stage.stage == "skipped" {
            continue;
        }
        if !summary.structural_only && stage.stage == "skipped" {
            return true;
        }
        if stage.native_receipt.as_ref().is_some_and(|r| {
            matches!(
                r.semantic_status.as_str(),
                "failed" | "invalid_environment" | "budget_exceeded"
            )
        }) {
            return true;
        }
    }
    false
}

fn first_actionable_limitation(stage: &entmoot_engine::workspace::BindStage) -> Option<&str> {
    stage
        .native_receipt
        .as_ref()
        .and_then(|r| r.limitations.first())
        .map(String::as_str)
        .or_else(|| {
            stage.limitations.iter().map(String::as_str).find(|line| {
                !line.starts_with("fresh_production_required:")
                    && !line.starts_with("structural_only_requested")
            })
        })
}

fn display_path(path: &Path) -> String {
    let raw = path.display().to_string();
    raw.strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix("//?/"))
        .unwrap_or(&raw)
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::ExecutionPolicy;
    use entmoot_contracts::NativeReceipt;
    use entmoot_contracts::{SourceSnapshot, SourceState};

    use entmoot_engine::workspace::{BindStage, PublicationLocation, WorkspaceSummary};
    use entmoot_engine::CoverageReport;
    use std::path::PathBuf;

    fn base_summary(stages: Vec<BindStage>, structural_only: bool) -> WorkspaceSummary {
        WorkspaceSummary {
            indexed_files: 0,
            reused_syntax_files: 0,
            reused_extraction_files: 0,
            reused_stitching_files: 0,
            schema_version: "entmoot.workspace_summary.v1".into(),
            publication: PublicationLocation {
                graph: PathBuf::from(r"\\?\G:\repo\.entmoot\current\generation-x\graph.json"),
                store: PathBuf::from(
                    r"\\?\G:\repo\.entmoot\current\generation-x\graph.json.knowledge.sqlite3",
                ),
                source_root: PathBuf::from(r"\\?\G:\repo"),
            },
            snapshot: SourceSnapshot {
                repository_id: "local/repo".into(),
                revision: "abc".into(),
                source_state: SourceState::WorkingTree,
                snapshot_digest: Some("blake3:00".into()),
                estate_id: None,
                subject: None,
                subject_aliases: Vec::new(),
            },
            policy_ids: vec![],
            execution_policy: ExecutionPolicy::Safe,
            full: false,
            require_native: false,
            plan_only: false,
            structural_only,
            stages,
            coverage: CoverageReport {
                findings_total: 0,
                traces_total: 0,
                trace_steps_total: 0,
                locations_total: 0,
                attachments_total: 0,
                assertions_total: 0,
                attachment_density: Default::default(),
                findings_by_producer: Default::default(),
                providers: vec![],
                provider_roles_present: false,
                no_provider_native_roles: true,
                partial_scope_artifacts: vec![],
            },
            wall_seconds: 12.5,
            budget_seconds: 300,
            limitations: vec![],
            adapter_profile: None,
            publication_outcome: "replaced".into(),
            publication_digest: "blake3:11".into(),
        }
    }

    fn failed_receipt() -> NativeReceipt {
        NativeReceipt {
            tool_environment: Vec::new(),
            wall_seconds: 39.0,
            artifact_bytes: 1,
            raw_bytes: 1,
            inventory_calls: 0,
            observed_calls: 0,
            call_gaps: Default::default(),
            input_files: 1,
            schema_version: entmoot_contracts::NativeReceiptSeries::V1,
            implementation: entmoot_engine::implementation_fingerprint(),
            producer: entmoot_contracts::NativeProducerContext {
                adapter_pin: "test".into(),
                method_id: "fixture.v1".into(),
                language: "python".into(),
                execution_policy: ExecutionPolicy::Safe,
            },
            semantic_status: entmoot_contracts::NativeSemanticStatus::Failed,
            limitations: vec!["invalid_environment: missing materialization".into()],
        }
    }

    #[test]
    fn partial_dynamic_calls_are_visible_without_an_environment_failure() {
        let mut receipt = failed_receipt();
        receipt.semantic_status = entmoot_contracts::NativeSemanticStatus::Partial;
        receipt.observed_calls = 2;
        receipt.inventory_calls = 2;
        receipt.call_gaps.insert("indirect_call".into(), 1);
        receipt.limitations.clear();
        let text = format_workspace_summary("bind", &base_summary(vec![BindStage {
            family: "python".into(), languages: vec!["python".into()], stage: "produced".into(),
            input_key: "k".into(), reuse_policy: "unqualified_external_closure".into(),
            adapter_path: None, producer_digest: "d".into(), manifest: None,
            native_receipt: Some(receipt), attachment: None, work: None,
            events: vec![], limitations: vec!["fresh_production_required: external/build input closure is not qualified for automatic reuse".into()],
        }], false));
        assert!(text.contains("[PARTIAL]"));
        assert!(text.contains("semantic=partial"));
        assert!(text.contains("calls observed=2 inventory=2"));
        assert!(text.contains("call_gap=indirect_call count=1"));
        assert!(!text.contains("fix environment"));
        assert!(!text.contains("fresh_production_required"));
    }

    #[test]
    fn strips_unc_prefix_and_reports_ok_for_structural_skips() {
        let text = format_workspace_summary(
            "init",
            &base_summary(
                vec![BindStage {
                    family: "python".into(),
                    languages: vec!["python".into()],
                    stage: "skipped".into(),
                    input_key: "k".into(),
                    reuse_policy: "n/a".into(),
                    adapter_path: None,
                    producer_digest: "d".into(),
                    manifest: None,
                    native_receipt: None,
                    attachment: None,
                    work: None,
                    events: vec!["planned".into()],
                    limitations: vec!["structural_only_requested; run entmoot bind".into()],
                }],
                true,
            ),
        );
        assert!(text.contains("[OK]"));
        assert!(text.contains("source_root=G:/repo"));
        assert!(!text.contains(r"\\?\"));
        assert!(!text.contains("pointer="));
        assert!(!text.contains("graph="));
        assert!(text.contains("entmoot init complete"));
    }

    #[test]
    fn incomplete_when_native_semantic_failed() {
        let text = format_workspace_summary(
            "bind",
            &base_summary(
                vec![BindStage {
                    family: "python".into(),
                    languages: vec!["python".into()],
                    stage: "produced".into(),
                    input_key: "k".into(),
                    reuse_policy: "n/a".into(),
                    adapter_path: None,
                    producer_digest: "d".into(),
                    manifest: None,
                    native_receipt: Some(failed_receipt()),
                    attachment: None,
                    work: None,
                    events: vec!["planned".into(), "produced".into()],
                    limitations: vec![],
                }],
                false,
            ),
        );
        assert!(text.contains("[INCOMPLETE]"));
        assert!(text.contains("limitation=invalid_environment: missing materialization"));
        assert!(text.contains("bind --structural-only"));
        assert!(text.contains("next: entmoot status"));
    }

    #[test]
    fn package_api_refs_tip_assess_compatibility() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = serde_json::json!({
            "package_api": {
                "surfaces": [{"package": {"ecosystem": "javascript", "name": "consumer"},
                    "export_universes": [], "completeness": "partial", "exports": [], "limitations": []}],
                "references": [{
                    "observation_id": "obs-1",
                    "target_index": 0,
                    "target": {"package": {"ecosystem": "javascript", "name": "accepts"},
                        "symbol": {"scheme": "javascript.compiler.symbol", "spelling": "accepts#default"}},
                    "boundary_policy": "unassessed"
                }],
                "references_complete": false,
                "limitations": []
            }
        });
        let artifact_path = dir.path().join("resolver.json");
        std::fs::write(
            &artifact_path,
            serde_json::to_vec_pretty(&artifact).unwrap(),
        )
        .unwrap();
        let manifest = dir.path().join("resolver.json.manifest.json");
        std::fs::write(&manifest, b"{}").unwrap();
        let text = format_workspace_summary(
            "init",
            &base_summary(
                vec![BindStage {
                    family: "oxc".into(),
                    languages: vec!["javascript".into()],
                    stage: "produced".into(),
                    input_key: "k".into(),
                    reuse_policy: "n/a".into(),
                    adapter_path: None,
                    producer_digest: "d".into(),
                    manifest: Some(manifest),
                    native_receipt: None,
                    attachment: None,
                    work: None,
                    events: vec!["planned".into(), "produced".into()],
                    limitations: vec![],
                }],
                false,
            ),
        );
        assert!(text.contains("package_api surfaces=1 refs=1 unassessed=1"));
        assert!(text.contains("next: entmoot assess compatibility --provider <other-checkout>"));
        assert!(!text.contains("next: entmoot status"));
    }
}
