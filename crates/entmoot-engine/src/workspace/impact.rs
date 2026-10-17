//! Local package composition over portable impact, applicability and work contracts.
//! This module's read/planning functions never invoke a producer.
use super::*;
use entmoot_contracts::{
    Address, AnalysisAction, AnalysisDecision, AnalysisDemand, AnalysisGate, AnalysisGateClaim,
    AnalysisGateKind, AnalysisPassMode, ArtifactFormat, ArtifactId, ArtifactManifest,
    ContentDigest, EvidenceRef, ProducerCapabilityKind, ProducerRef, RevalidationDeclaration,
    RevalidationUnit, ScopeCompleteness, ScopeUniverse, TransitionRevalidationReceipt,
    TransitionRevalidationRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindDecision {
    pub family: String,
    pub decision: AnalysisDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactPackage {
    pub schema_version: String,
    pub base: PublicationLocation,
    pub head: PublicationLocation,
    pub base_graph_digest: String,
    pub head_graph_digest: String,
    pub impact: entmoot_contracts::query::RevisionImpactManifest,
    pub revalidation: Option<TransitionRevalidationReceipt>,
    pub ledger: Option<PathBuf>,
    pub work_hints: Vec<BindDecision>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactBindReceipt {
    pub schema_version: String,
    pub status: String,
    pub decisions: Vec<BindDecision>,
    pub publication: Option<WorkspaceSummary>,
    pub limitations: Vec<String>,
}

pub(super) fn policy_producer() -> ProducerRef {
    ProducerRef {
        id: "entmoot.bind.operator_policy".into(),
        name: "Entmoot explicit native bind policy".into(),
        version: Some("1".into()),
        config_fingerprint: None,
        environment_fingerprint: None,
        capabilities: vec![ProducerCapabilityKind::Deriver],
    }
}

fn supported_family(artifact: &ArtifactManifest) -> Option<&'static str> {
    if artifact.format != ArtifactFormat::Resolver
        || artifact.producer.ruleset_id.as_deref() != Some("entmoot.native_resolver.v1")
    {
        return None;
    }
    match artifact.producer.name.as_str() {
        "native-javascript" if artifact.producer.version.as_deref() == Some(oxc::OXC_PIN) => {
            Some("oxc")
        }
        "native-rust" if artifact.producer.version.as_deref() == Some(RA_AP_PIN) => Some("rust"),
        "native-typescript" if artifact.producer.version.as_deref() == Some(typescript::PIN) => {
            Some("typescript")
        }
        "native-go" => Some("go"),
        "native-csharp" => Some("roslyn"),
        "native-java" => Some("java"),
        "native-python" => Some("python"),
        "native-cxx" => Some("clang"),
        _ => None,
    }
}

fn gate_claims(
    artifact: &ArtifactManifest,
    family: &str,
    relevant: bool,
) -> Vec<AnalysisGateClaim> {
    [AnalysisGateKind::Qualification, AnalysisGateKind::Environment, AnalysisGateKind::MarginalValue]
        .into_iter().map(|kind| AnalysisGateClaim {
            kind,
            disposition: if kind == AnalysisGateKind::Environment && !available(family) {
                AnalysisGate::Rejected
            } else if kind == AnalysisGateKind::MarginalValue && !relevant {
                AnalysisGate::Unknown
            } else { AnalysisGate::Accepted },
            claimant: policy_producer(), method_id: "entmoot.bind.fresh_native.v1".into(),
            assumptions: vec!["explicit operator requested fresh evidence from the shipped native adapter".into()],
            limitations: vec![
                "qualifies fresh artifact plumbing only; compiler/build environment validity is determined by the producer receipt".into(),
                "partial results and failed attempts remain evidence; no external cache reuse or closed dispatch".into(),
            ],
            evidence_refs: vec![EvidenceRef { record_id: None, artifact_id: artifact.artifact_id.clone(), provider_pointer: "/producer".into() }],
        }).collect()
}

/// Plan only fresh supported native work. Unknown third-party evidence remains explicit.
fn decisions(
    base: &TopologyGraph,
    head: &TopologyGraph,
    mode: AnalysisPassMode,
) -> Result<Vec<BindDecision>, WorkspaceError> {
    let mut families = std::collections::BTreeMap::new();
    for artifact in &base.artifacts {
        if let Some(family) = supported_family(artifact) {
            families.entry(family).or_insert(artifact);
        }
    }
    let mut output = Vec::new();
    for (family, artifact) in families {
        let fresh = head.artifacts.iter().find(|other| {
            supported_family(other) == Some(family) && other.snapshot == head.snapshot
        });
        let root = ContentDigest::from_trusted(blake3_hex(&serde_json::to_vec(&(
            "entmoot.bind.fresh_native.v1",
            family,
            &head.snapshot,
            &head.snapshot_files,
            &head.policy_ids,
        ))?));
        let refs = vec![EvidenceRef {
            record_id: None,
            artifact_id: artifact.artifact_id.clone(),
            provider_pointer: "/".into(),
        }];
        let demand = AnalysisDemand {
            policy_producer: policy_producer(), snapshot: head.snapshot.clone(),
            subject: Address::Opaque { system: "entmoot.bind.family".into(), value: family.into() },
            method_id: "entmoot.bind.fresh_native.v1".into(), input_root: root.clone(),
            // Same-snapshot attached evidence needs no repeated production. This is
            // a scheduling no-op, not authorization to retarget historical artifacts.
            prior_input_root: fresh.map(|_| root),
            current_artifact: fresh.map(|a| ArtifactId::parse(&a.artifact_id)).transpose().map_err(failure)?,
            gates: gate_claims(artifact, family, fresh.is_none()), relevant_unknown: fresh.is_none(),
            assumptions: vec!["fresh head artifact means this adapter already ran for this snapshot; unresolved evidence does not demand an identical rerun".into()],
            evidence_refs: refs,
        };
        output.push(BindDecision {
            family: family.into(),
            decision: crate::plan_analysis(demand, mode).map_err(failure)?,
        });
    }
    Ok(output)
}

/// Default conditional retention policy is deliberately limited to the in-process
/// Oxc lexical universe. Other native/external build closures remain unselected.
fn revalidation_request(
    base: &TopologyGraph,
    head: &TopologyGraph,
) -> Result<TransitionRevalidationRequest, WorkspaceError> {
    let mut units = Vec::new();
    let mut series = BTreeSet::new();
    let paths = base
        .snapshot_files
        .iter()
        .map(|f| f.path.clone())
        .collect::<Vec<_>>();
    for artifact in &base.artifacts {
        if supported_family(artifact) != Some("oxc") || !series.insert(&artifact.artifact_series_id)
        {
            continue;
        }
        let fresh = head
            .artifacts
            .iter()
            .find(|other| other.artifact_series_id == artifact.artifact_series_id);
        units.push(RevalidationUnit {
            base_artifact_id: ArtifactId::parse(&artifact.artifact_id).map_err(failure)?,
            head_artifact_id: fresh.map(|a| ArtifactId::parse(&a.artifact_id)).transpose().map_err(failure)?,
            declaration: RevalidationDeclaration {
                policy_producer: policy_producer(), method_id: "entmoot.oxc.lexical.source_universe.v1".into(),
                input_scope: ScopeUniverse::from_selectors("oxc-inventory", vec![], vec!["**".into()], vec![],
                    ScopeCompleteness::CompleteRelativeToUniverse, Some(&paths)),
                assumptions: vec![
                    "fixed Oxc and exact-span join versions; lexical resolution consumes the complete inventoried source universe".into(),
                    "retention preserves historical evidence; it does not create head bindings or authorize artifact reuse".into(),
                ],
                evidence_refs: vec![EvidenceRef { record_id: None, artifact_id: artifact.artifact_id.clone(), provider_pointer: "/".into() }],
            },
            subject: Address::Opaque { system: "entmoot.bind.family".into(), value: "oxc".into() },
            gates: gate_claims(artifact, "oxc", true), mode: AnalysisPassMode::Auto,
        });
    }
    Ok(TransitionRevalidationRequest {
        base_snapshot: base.snapshot.clone(),
        head_snapshot: head.snapshot.clone(),
        units,
        max_records: 1_000_000,
    })
}

pub fn location(graph_path: &Path) -> Result<PublicationLocation, WorkspaceError> {
    let graph_path = fs::canonicalize(graph_path)?;
    let graph = crate::load_graph_json(&graph_path).map_err(failure)?;
    Ok(PublicationLocation {
        store: PathBuf::from(format!("{}.knowledge.sqlite3", graph_path.display())),
        graph: graph_path,
        source_root: PathBuf::from(graph.root),
    })
}

/// Compose an impact package; a ledger is the only optional effect. Never binds.
pub fn create_package(
    base: PublicationLocation,
    head: PublicationLocation,
    ledger: Option<&Path>,
) -> Result<ImpactPackage, WorkspaceError> {
    let base_graph = crate::load_graph_json(&base.graph).map_err(failure)?;
    let head_graph = crate::load_graph_json(&head.graph).map_err(failure)?;
    let impact = crate::compare_revision_impact(&base_graph, &head_graph).map_err(failure)?;
    let work_hints = decisions(&base_graph, &head_graph, AnalysisPassMode::Auto)?;
    let revalidation = ledger
        .map(|path| {
            crate::publish_revalidation_ledger(
                &base_graph,
                &head_graph,
                &revalidation_request(&base_graph, &head_graph)?,
                path,
            )
            .map_err(failure)
        })
        .transpose()?;
    Ok(ImpactPackage {
        schema_version: "entmoot.local_impact_package.v1".into(),
        base_graph_digest: blake3_hex(&fs::read(&base.graph)?),
        head_graph_digest: blake3_hex(&fs::read(&head.graph)?),
        base, head, impact, revalidation, ledger: ledger.map(Path::to_path_buf), work_hints,
        limitations: vec![
            "work hints cover previously attached shipping native adapters only; missing/foreign evidence needs explicit bind or external production".into(),
            "default conditional revalidation is Oxc lexical only; undeclared external build closure remains unknown".into(),
            "impact and revalidation never execute producers; breaking-change signals are not SemVer or vulnerability verdicts".into(),
        ],
    })
}

/// Validate the package and the current snapshot before an explicit fenced refresh.
pub fn bind_from_package(
    path: &Path,
    options: &BindOptions,
) -> Result<ImpactBindReceipt, WorkspaceError> {
    let package: ImpactPackage = serde_json::from_slice(&fs::read(path)?)?;
    if package.schema_version != "entmoot.local_impact_package.v1"
        || blake3_hex(&fs::read(&package.base.graph)?) != package.base_graph_digest
        || blake3_hex(&fs::read(&package.head.graph)?) != package.head_graph_digest
    {
        return Err(failure("impact package publication digest mismatch"));
    }
    let base = crate::load_graph_json(&package.base.graph).map_err(failure)?;
    let original_head = crate::load_graph_json(&package.head.graph).map_err(failure)?;
    if crate::compare_revision_impact(&base, &original_head).map_err(failure)? != package.impact {
        return Err(failure("impact package receipt mismatch"));
    }
    let _guard = if options.plan_only {
        None
    } else {
        Some(WriteGuard::acquire(&package.head.source_root)?)
    };
    let previous = discover_publication(&package.head.source_root)?;
    let head = crate::load_graph_json(&previous.current.graph).map_err(failure)?;
    if head.snapshot != original_head.snapshot {
        return Err(failure(
            "current publication does not match impact head; generate a new impact package",
        ));
    }
    verify_sources(&head, &previous.current.source_root)?;
    // Recompute from attached evidence; never trust editable scheduling hints.
    let decisions = decisions(
        &base,
        &head,
        if options.full {
            AnalysisPassMode::Always
        } else {
            AnalysisPassMode::Auto
        },
    )?;
    let requested = decisions
        .iter()
        .any(|d| d.decision.action == AnalysisAction::RequestArtifact);
    let blocked = decisions
        .iter()
        .any(|d| d.decision.action == AnalysisAction::Blocked);
    if options.require_native && blocked {
        return Err(failure(
            "invalid_environment: impact work requires an unavailable native adapter",
        ));
    }
    let publication = if requested {
        Some(super::publish(
            previous.current.source_root.clone(),
            head,
            Some(previous),
            options,
            Some(&decisions),
            None,
        )?)
    } else {
        None
    };
    Ok(ImpactBindReceipt {
        schema_version: "entmoot.impact_bind.v1".into(),
        status: if options.plan_only {
            "planned"
        } else if requested {
            "published"
        } else if blocked {
            "blocked"
        } else {
            "no_work"
        }
        .into(),
        decisions,
        publication,
        limitations: package.limitations,
    })
}

#[cfg(all(test, feature = "resolved-oxc"))]
mod tests {
    use super::*;
    #[test]
    fn impact_then_fenced_bind_and_noop_are_separate_effects() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("app.js"),
            "function callee() { return 1; }\nfunction caller() { return callee(); }\n",
        )
        .unwrap();
        let policy = entmoot_core::ground_truth::policy_from_config_name("v1").unwrap();
        let base = initialize_workspace(
            root.path(),
            Some("fixture"),
            &policy,
            &BindOptions::default(),
        )
        .unwrap();
        fs::write(
            root.path().join("app.js"),
            "function callee() { return 2; }\nfunction caller() { return callee(); }\n",
        )
        .unwrap();
        let head = initialize_workspace(
            root.path(),
            Some("fixture"),
            &policy,
            &BindOptions {
                structural_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        let pointer = crate::workspace::storage_directory(root.path())
            .unwrap()
            .join("current.json");
        let before = fs::read(&pointer).unwrap();
        let ledger = crate::workspace::storage_directory(root.path())
            .unwrap()
            .join("applicability.sqlite3");
        let package = create_package(base.publication, head.publication, Some(&ledger)).unwrap();
        assert_eq!(fs::read(&pointer).unwrap(), before);
        assert!(ledger.is_file());
        assert_eq!(package.work_hints.len(), 1);
        assert_eq!(
            package.work_hints[0].decision.action,
            AnalysisAction::RequestArtifact
        );
        assert!(
            package.revalidation.as_ref().unwrap().units[0]
                .revalidation
                .inputs_changed
        );
        let path = crate::workspace::storage_directory(root.path())
            .unwrap()
            .join("impact.json");
        write_json(&path, &package).unwrap();
        let plan = bind_from_package(
            &path,
            &BindOptions {
                plan_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(plan.status, "planned");
        assert_eq!(fs::read(&pointer).unwrap(), before);
        let bound = bind_from_package(&path, &BindOptions::default()).unwrap();
        let publication = bound.publication.unwrap();
        let oxc = publication
            .stages
            .iter()
            .find(|s| s.family == "oxc")
            .unwrap();
        assert_eq!(
            oxc.events,
            [
                "planned",
                "fenced",
                "produced",
                "artifact_ready",
                "attached",
                "ingested"
            ]
        );
        assert_eq!(
            oxc.work.as_ref().unwrap().state,
            entmoot_contracts::AnalysisWorkState::Ingested
        );
        let after = fs::read(&pointer).unwrap();
        let noop = bind_from_package(&path, &BindOptions::default()).unwrap();
        assert_eq!(noop.status, "no_work");
        assert!(noop.publication.is_none());
        assert_eq!(fs::read(&pointer).unwrap(), after);
        // A forged comparison cannot schedule work.
        let mut tampered = package;
        tampered.impact.files.clear();
        write_json(&path, &tampered).unwrap();
        assert!(bind_from_package(&path, &BindOptions::default())
            .unwrap_err()
            .to_string()
            .contains("receipt mismatch"));
        assert_eq!(fs::read(&pointer).unwrap(), after);
    }
}
