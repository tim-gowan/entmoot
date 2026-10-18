//! Production source/artifact fixtures with independent applicability expectations.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use entmoot_contracts::*;
use entmoot_engine::{
    collaboration, revalidation_operations as worker, KnowledgeRead, KnowledgeRepository,
    LocalOperations, SqliteKnowledgeRepository, TopologyGraph,
};
use std::{collections::BTreeSet, path::PathBuf};

struct Fixture {
    temp: tempfile::TempDir,
    store: PathBuf,
    base: TopologyGraph,
    head: TopologyGraph,
    request: RevalidationRequest,
}
impl Fixture {
    fn new() -> Self {
        Self::with_extra_policies(0)
    }
    fn with_extra_policies(extra: usize) -> Self {
        Self::with_inventory(extra, 0)
    }
    fn with_inventory(extra: usize, inventory_files: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let build = |name: &str, changed: bool| {
            let root = temp.path().join(name);
            std::fs::create_dir(&root).unwrap();
            std::fs::write(
                root.join("main.rs"),
                if changed {
                    "fn main() { let x = 1; stable(); }\n"
                } else {
                    "fn main() { stable(); }\n"
                },
            )
            .unwrap();
            std::fs::write(root.join("stable.rs"), "pub fn stable() {}\n").unwrap();
            for i in 0..inventory_files {
                std::fs::write(root.join(format!("bulk_{i}.rs")), format!("fn leaf_{i}() -> i32 {{ {i} }} fn caller_{i}() -> i32 {{ let value = leaf_{i}(); value }}\n")).unwrap();
            }
            entmoot_engine::build_topology_from_repo_with_options(
                &root,
                entmoot_engine::TopologyBuildOptions {
                    repository: Some("fixture/revalidation"),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let mut base = build("base", false);
        let mut head = build("head", true);
        let mut units = vec![];
        let policies = [
            ("changed", "main.rs"),
            ("stable", "stable.rs"),
            ("fresh", "main.rs"),
            ("unselected", "main.rs"),
        ]
        .into_iter()
        .map(|(name, path)| (name.to_owned(), path))
        .chain((0..extra).map(|i| {
            (
                format!("bulk-{i}"),
                if i % 2 == 0 { "main.rs" } else { "stable.rs" },
            )
        }))
        .collect::<Vec<_>>();
        for (name, path) in &policies {
            let (name, path) = (name.as_str(), *path);
            let attach = |graph: &mut TopologyGraph, config: Option<String>| {
                let bytes = serde_json::to_vec(&serde_json::json!({
                    "version":"2.1.0", "runs":[{"tool":{"driver":{"name":name,"version":"1"}},
                    "results":[{"ruleId":"RULE", "message":{"text":"fixture observation"},
                    "locations":[{"physicalLocation":{"artifactLocation":{"uri":path},"region":{"startLine":1,"startColumn":1}}}]}]}]
                })).unwrap();
                let mut manifest = ArtifactManifest::build(
                    ArtifactFormat::Sarif21,
                    ArtifactProducer {
                        name: name.into(),
                        version: Some("1".into()),
                        information_uri: None,
                        ruleset_id: Some("fixture".into()),
                        config_class: None,
                    },
                    graph.snapshot.clone(),
                    ScopeUniverse::from_selectors(
                        "fixture/revalidation",
                        vec!["rust".into()],
                        vec![path.into()],
                        vec![],
                        ScopeCompleteness::Partial,
                        Some(&[path.into()]),
                    ),
                    blake3_hex(&bytes),
                    config,
                )
                .unwrap();
                manifest.artifact_path = Some("input.sarif".into());
                let mp = temp.path().join("input.manifest.json");
                std::fs::write(temp.path().join("input.sarif"), bytes).unwrap();
                std::fs::write(&mp, serde_json::to_vec(&manifest).unwrap()).unwrap();
                entmoot_engine::ingest_artifact_manifest(graph, &mp, false).unwrap();
                manifest
            };
            let artifact = attach(&mut base, None);
            if name == "unselected" {
                continue;
            }
            let fresh = (name == "fresh")
                .then(|| attach(&mut head, Some(blake3_hex(b"pinned config two"))));
            let gates= ["qualification","environment","marginal_value"].map(|kind| serde_json::from_value(serde_json::json!({
                "kind":kind,"disposition":"accepted", "claimant":{"id":format!("fixture.{kind}"),"name":kind,"capabilities":["deriver"]},
                "method_id":"fixture.gate.v1", "assumptions":["pinned isolated fixture environment"],"limitations":["fixture only"],
                "evidence_refs":[{"artifact_id":artifact.artifact_id,"provider_pointer":"/runs/0"}]
            })).unwrap()).to_vec();
            units.push(RevalidationUnit {
                base_artifact_id: ArtifactId::parse(&artifact.artifact_id).unwrap(),
                head_artifact_id: fresh.map(|a| ArtifactId::parse(a.artifact_id).unwrap()),
                declaration: RevalidationDeclaration {
                    policy_producer: serde_json::from_value(
                        serde_json::json!({"id":"fixture.policy","name":"Fixture policy"}),
                    )
                    .unwrap(),
                    method_id: "fixture.complete-source.v1".into(),
                    input_scope: ScopeUniverse::from_selectors(
                        "fixture/revalidation",
                        vec![],
                        vec![path.into()],
                        vec![],
                        ScopeCompleteness::CompleteRelativeToUniverse,
                        Some(&[path.into()]),
                    ),
                    assumptions: vec!["pinned isolated producer and configuration".into()],
                    evidence_refs: vec![EvidenceRef {
                        artifact_id: artifact.artifact_id,
                        provider_pointer: "/runs/0".into(),
                        record_id: None,
                    }],
                },
                subject: serde_json::from_value(
                    serde_json::json!({"scheme":"opaque","system":"fixture","value":name}),
                )
                .unwrap(),
                gates,
                mode: AnalysisPassMode::Auto,
            });
        }
        let store = temp.path().join("shared.sqlite3");
        let b = collaboration::register_graph(&store, &base, "base", "agent", None, 0).unwrap();
        let h = collaboration::register_graph(&store, &head, "head", "agent", None, 0).unwrap();
        Self {
            temp,
            store,
            base,
            head,
            request: RevalidationRequest {
                base_view: b.view,
                head_view: h.view,
                units,
            },
        }
    }
    fn db(&self) -> SqliteKnowledgeRepository {
        SqliteKnowledgeRepository::open(&self.store).unwrap()
    }
    fn submit(&self, key: &str) -> OperationRecord {
        worker::submit(&mut self.db(), self.request.clone(), key, "agent").unwrap()
    }
    fn run(&self, id: &str, steps: usize, records: usize) -> OperationRecord {
        worker::run(&self.store, id, "worker", steps, records, None).unwrap()
    }
    fn resume(&self, id: &str) {
        self.db()
            .control_operation(id, OperationControl::Resume)
            .unwrap();
    }
    fn outputs(&self, id: &str) -> Vec<RevalidationUnitReceipt> {
        self.db()
            .operation_outputs(id, 0, 100)
            .unwrap()
            .outputs
            .into_iter()
            .map(|o| serde_json::from_value(o.value).unwrap())
            .collect()
    }
    fn serial(&self) -> (SqliteKnowledgeRepository, TransitionRevalidationReceipt) {
        let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let result = entmoot_engine::revalidate_transition(
            &mut db,
            &self.base,
            &self.head,
            &TransitionRevalidationRequest {
                base_snapshot: self.base.snapshot.clone(),
                head_snapshot: self.head.snapshot.clone(),
                units: self.request.units.clone(),
                max_records: 1_000_000,
            },
        )
        .unwrap();
        (db, result)
    }
    fn ledger(&self, item: &OperationRecord) -> SqliteKnowledgeRepository {
        SqliteKnowledgeRepository::open_view(
            &self.store,
            item.checkpoint["ledger_view"].as_str().unwrap(),
        )
        .unwrap()
    }
    fn count(&self, sql: &str, id: &str) -> i64 {
        rusqlite::Connection::open(&self.store)
            .unwrap()
            .query_row(sql, [id], |r| r.get(0))
            .unwrap()
    }
}

fn all_records(units: &[RevalidationUnitReceipt]) -> BTreeSet<KnowledgeRecordRef> {
    let mut records = BTreeSet::new();
    for unit in units {
        for p in std::iter::once(&unit.publication).chain(unit.reingested.as_ref()) {
            records.extend(p.retained_evidence.clone());
            records.extend(p.dependent_applicability.clone());
            records.insert(KnowledgeRecordRef::new(
                "input_roots",
                p.input_root.id.to_string(),
            ));
        }
        records.insert(KnowledgeRecordRef::new(
            "input_roots",
            unit.revalidation.current_input_root.id.to_string(),
        ));
    }
    records
}

#[test]
fn selected_artifact_evidence_does_not_acquire_structural_inventory() {
    let f = Fixture::new();
    let (ledger, receipt) = f.serial();
    assert!(!f.base.coverages.is_empty());
    assert!(f.base.nodes.iter().any(|n| n.call_site.is_some()));
    assert!(f.head.nodes.iter().any(|n| n.declaration.is_some()));
    for unit in &receipt.units {
        for publication in std::iter::once(&unit.publication).chain(unit.reingested.as_ref()) {
            assert!(
                publication.retained_evidence.iter().all(|record| !matches!(
                    record.collection.as_str(),
                    "coverages" | "call_sites" | "declaration_sites" | "structural_edges"
                )),
                "an artifact policy cannot own structural inventory: {:?}",
                publication.retained_evidence
            );
            assert!(publication
                .retained_evidence
                .iter()
                .any(|r| r.collection == "findings"));
        }
    }
    let store = ledger.load().unwrap();
    assert!(store.coverages.is_empty());
    assert!(store.call_sites.is_empty());
    assert!(store.declaration_sites.is_empty());
    assert!(store.structural_edges.is_empty());
}

#[test]
fn artifact_ledgers_preserve_traces_and_bom_details_without_other_producers() {
    let mut f = Fixture::new();
    let documents = [
        (
            ArtifactFormat::Sarif21,
            serde_json::json!({
                "version":"2.1.0", "runs":[{"tool":{"driver":{"name":"trace-provider"}},
                "results":[{"ruleId":"FLOW", "message":{"text":"attributed path"},
                    "locations":[{"physicalLocation":{"artifactLocation":{"uri":"main.rs"},"region":{"startLine":1}}}],
                    "codeFlows":[{"threadFlows":[{"locations":[
                        {"location":{"physicalLocation":{"artifactLocation":{"uri":"main.rs"},"region":{"startLine":1}}}},
                        {"location":{"physicalLocation":{"artifactLocation":{"uri":"stable.rs"},"region":{"startLine":1}}}}
                    ]}]}]}]}]
            }),
        ),
        (
            ArtifactFormat::CycloneDx,
            serde_json::json!({
                "bomFormat":"CycloneDX", "specVersion":"1.7",
                "components":[{"type":"application","bom-ref":"app","name":"app"},
                    {"type":"library","bom-ref":"lib","name":"lib","version":"2.0"}],
                "dependencies":[{"ref":"app","dependsOn":["lib","missing"]}],
                "vulnerabilities":[{"id":"CVE-2099-0001","affects":[{"ref":"lib"}],
                    "analysis":{"state":"not_affected","justification":"code_not_present","detail":"provider explanation"}}],
                "compositions":[{"aggregate":"incomplete_first_party_only","assemblies":["app"]}]
            }),
        ),
    ];
    let mut manifests = vec![];
    for (index, (format, document)) in documents.into_iter().enumerate() {
        let bytes = serde_json::to_vec(&document).unwrap();
        let mut manifest = ArtifactManifest::build(
            format,
            ArtifactProducer {
                name: format!("provider-{index}"),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            f.base.snapshot.clone(),
            ScopeUniverse::from_selectors(
                "provider",
                vec![],
                vec!["*.rs".into()],
                vec![],
                ScopeCompleteness::Partial,
                None,
            ),
            blake3_hex(&bytes),
            None,
        )
        .unwrap();
        manifest.artifact_path = Some(format!("provider-{index}.json"));
        std::fs::write(
            f.temp.path().join(manifest.artifact_path.as_ref().unwrap()),
            bytes,
        )
        .unwrap();
        let path = f
            .temp
            .path()
            .join(format!("provider-{index}.manifest.json"));
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        entmoot_engine::ingest_artifact_manifest(&mut f.base, &path, false).unwrap();
        manifests.push(manifest);
    }
    for manifest in &manifests {
        let mut declaration = f.request.units[0].declaration.clone();
        declaration.evidence_refs[0].artifact_id = manifest.artifact_id.clone();
        let mut ledger = SqliteKnowledgeRepository::open_in_memory().unwrap();
        entmoot_engine::publish_artifact_evidence(&mut ledger, &f.base, manifest, &declaration)
            .unwrap();
        let retained = ledger.load().unwrap();
        assert_eq!(retained.artifacts, vec![manifest.clone()]);
        assert!(retained.coverages.is_empty());
        if manifest.format == ArtifactFormat::Sarif21 {
            assert_eq!(retained.findings.len(), 1);
            assert_eq!(
                retained.findings[0].message.as_deref(),
                Some("attributed path")
            );
            assert_eq!(retained.traces.len(), 1);
            assert_eq!(retained.trace_steps.len(), 2);
            assert_eq!(
                retained.traces[0].finding_observation_id,
                retained.findings[0].observation_id
            );
            assert!(retained
                .trace_steps
                .iter()
                .all(|s| s.trace_id == retained.traces[0].trace_id));
            assert!(!retained.attachments.is_empty());
            assert!(!retained.assertions.is_empty());
            assert!(retained.components.is_empty());
        } else {
            assert!(
                retained.findings.is_empty()
                    && retained.traces.is_empty()
                    && retained.trace_steps.is_empty()
            );
            assert_eq!(retained.components.len(), 2);
            assert_eq!(retained.component_dependencies.len(), 2);
            assert_eq!(retained.component_vulnerabilities.len(), 1);
            let analysis = retained.component_vulnerabilities[0]
                .analysis
                .as_ref()
                .unwrap();
            assert_eq!(analysis.state, "not_affected");
            assert_eq!(analysis.detail.as_deref(), Some("provider explanation"));
            assert_eq!(
                retained.bom_compositions[0].aggregate,
                "incomplete_first_party_only"
            );
            assert_eq!(
                retained.diagnostics.len(),
                1,
                "missing BOM reference remains explicit"
            );
        }
    }
}

#[test]
fn invalid_graph_outside_selected_inputs_is_rejected_before_ledger_initialization() {
    let f = Fixture::new();
    for corrupt_base in [true, false] {
        let mut base = f.base.clone();
        let mut head = f.head.clone();
        let graph = if corrupt_base { &mut base } else { &mut head };
        // Corrupt an immutable graph record outside the selected policy input.
        graph.snapshot_files[1].content_digest = "not-a-digest".into();
        let mut ledger = SqliteKnowledgeRepository::open_in_memory().unwrap();
        assert!(entmoot_engine::revalidate_transition(
            &mut ledger,
            &base,
            &head,
            &TransitionRevalidationRequest {
                base_snapshot: base.snapshot.clone(),
                head_snapshot: head.snapshot.clone(),
                units: vec![f.request.units[0].clone()],
                max_records: 1_000_000
            }
        )
        .is_err());
        assert!(!ledger.is_initialized().unwrap());
    }
}

#[test]
fn selecting_one_artifact_does_not_hide_unselected_siblings_in_its_producer_series() {
    let mut f = Fixture::new();
    let original = f
        .base
        .artifacts
        .iter()
        .find(|a| a.producer.name == "stable")
        .unwrap()
        .clone();
    let bytes = serde_json::to_vec(&serde_json::json!({"version":"2.1.0","runs":[{
        "tool":{"driver":{"name":"stable","version":"1"}},
        "results":[{"ruleId":"OTHER","message":{"text":"independent retained observation"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"stable.rs"},"region":{"startLine":1,"startColumn":1}}}]}]
    }]})).unwrap();
    let mut sibling = ArtifactManifest::build(
        original.format,
        original.producer.clone(),
        f.base.snapshot.clone(),
        original.scope.clone(),
        blake3_hex(&bytes),
        None,
    )
    .unwrap();
    sibling.artifact_path = Some("sibling.sarif".into());
    assert_eq!(sibling.artifact_series_id, original.artifact_series_id);
    assert_ne!(sibling.artifact_id, original.artifact_id);
    std::fs::write(f.temp.path().join("sibling.sarif"), bytes).unwrap();
    let manifest = f.temp.path().join("sibling.manifest.json");
    std::fs::write(&manifest, serde_json::to_vec(&sibling).unwrap()).unwrap();
    entmoot_engine::ingest_artifact_manifest(&mut f.base, &manifest, false).unwrap();
    f.request.base_view =
        collaboration::register_graph(&f.store, &f.base, "base", "agent", None, 1)
            .unwrap()
            .view;
    let (ledger, receipt) = f.serial();
    assert_eq!(receipt.unselected_artifact_ids.len(), 2);
    assert!(receipt
        .unselected_artifact_ids
        .contains(&sibling.artifact_id));
    assert!(ledger
        .load()
        .unwrap()
        .artifacts
        .iter()
        .all(|a| a.artifact_id != sibling.artifact_id));
    let job = f.submit("exact-selection");
    let done = f.run(job.id.as_str(), 4, 1000);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(
        done.result.as_ref().unwrap()["unselected_artifact_ids"],
        serde_json::to_value(receipt.unselected_artifact_ids).unwrap()
    );
}

#[test]
fn unrelated_inventory_does_not_consume_an_artifact_revalidation_record_budget() {
    let small = Fixture::new();
    let large = Fixture::with_inventory(0, 64);
    let count = |receipt: &TransitionRevalidationReceipt| {
        receipt
            .units
            .iter()
            .flat_map(|u| std::iter::once(&u.publication).chain(u.reingested.as_ref()))
            .map(|p| p.retained_evidence.len() + p.dependent_applicability.len())
            .sum::<usize>()
    };
    let budget = count(&small.serial().1);
    assert_eq!(count(&large.serial().1), budget);
    let job = large.submit("bounded-artifacts");
    let done = large.run(job.id.as_str(), 4, budget);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.result.as_ref().unwrap()["semantic_records"], budget);
}

#[test]
#[ignore = "explicit revalidation projection time and record-count probe"]
fn revalidation_projection_cost_probe() {
    for files in [0, 256] {
        let f = Fixture::with_inventory(29, files);
        for repeat in 0..3 {
            let start = std::time::Instant::now();
            let (ledger, receipt) = f.serial();
            let elapsed = start.elapsed();
            let records: usize = receipt
                .units
                .iter()
                .flat_map(|u| std::iter::once(&u.publication).chain(u.reingested.as_ref()))
                .map(|p| p.retained_evidence.len() + p.dependent_applicability.len())
                .sum();
            println!(
                "{}",
                serde_json::json!({"inventory_files":files,"policies":receipt.units.len(),"repeat":repeat,"seconds":elapsed.as_secs_f64(),"semantic_records":records})
            );
            drop(ledger);
        }
    }
}

#[test]
fn sliced_reopen_and_serial_ledgers_match_with_changed_stable_fresh_and_unknown_producers() {
    let f = Fixture::new();
    let (serial, expected) = f.serial();
    assert_eq!(expected.unselected_artifact_ids.len(), 1);
    for unit in &expected.units {
        let artifact = f
            .base
            .artifacts
            .iter()
            .find(|a| a.artifact_id == unit.publication.artifact_id)
            .unwrap();
        assert!(!unit.publication.dependent_applicability.is_empty());
        match artifact.producer.name.as_str() {
            "stable" => {
                assert!(!unit.revalidation.inputs_changed);
                assert!(!unit.revalidation.retained_applicability.is_empty());
                assert_eq!(unit.decision.action, AnalysisAction::ReuseCandidate);
            }
            "changed" => {
                assert!(unit.revalidation.inputs_changed);
                assert!(!unit.revalidation.invalidated_applicability.is_empty());
                assert_eq!(unit.decision.action, AnalysisAction::RequestArtifact);
            }
            "fresh" => {
                assert!(unit.revalidation.inputs_changed);
                assert!(unit.reingested.is_some());
                assert_eq!(unit.decision.action, AnalysisAction::ReuseCandidate);
                assert_ne!(
                    unit.revalidation.current_input_root.id,
                    unit.decision.demand.input_root
                );
            }
            _ => panic!("unexpected selected producer"),
        }
        assert_eq!(unit.publication.snapshot, f.base.snapshot);
    }
    let submitted = f.submit("sliced");
    assert_eq!(f.submit("sliced"), submitted);
    let id = submitted.id.as_str();
    let mut saved = vec![];
    for i in 1..=3 {
        let item = f.run(id, 1, 1000);
        assert_eq!(item.state, OperationState::Paused);
        assert_eq!(item.checkpoint["completed_units"], i);
        assert_eq!(
            f.count(
                "SELECT COUNT(*) FROM local_operation_pins WHERE operation_id=?1",
                id
            ),
            3
        );
        let outputs = f.outputs(id);
        assert_eq!(&outputs[..saved.len()], saved);
        saved = outputs;
        assert!(
            worker::record_state(&f.store, id, KnowledgeRecordRef::new("findings", "missing"))
                .is_err()
        );
        assert!(f.ledger(&item).replace_all(&Default::default()).is_err());
        f.resume(id);
    }
    let completed = f.run(id, 1, 1000);
    assert_eq!(completed.state, OperationState::Completed);
    assert_eq!(f.outputs(id), expected.units);
    assert_eq!(f.run(id, 1, 1000), completed);
    let ledger = f.ledger(&completed);
    for record in all_records(&expected.units) {
        assert_eq!(
            ledger.record_state(&record).unwrap(),
            serial.record_state(&record).unwrap(),
            "{record:?}"
        );
        assert_eq!(
            ledger.owners_for_record(&record, 100).unwrap(),
            serial.owners_for_record(&record, 100).unwrap()
        );
        assert_eq!(
            ledger.dependency_inputs_for_record(&record, 100).unwrap(),
            serial.dependency_inputs_for_record(&record, 100).unwrap()
        );
        assert_eq!(
            worker::record_state(&f.store, id, record.clone())
                .unwrap()
                .visible,
            serial.record_state(&record).unwrap().map(|s| s.visible)
        );
    }
    assert_eq!(
        serde_json::to_value(ledger.load().unwrap()).unwrap(),
        serde_json::to_value(serial.load().unwrap()).unwrap()
    );
    assert_eq!(
        worker::record_state(
            &f.store,
            id,
            KnowledgeRecordRef::new("findings", "unreported")
        )
        .unwrap()
        .visible,
        None
    );
    assert!(f
        .ledger(&completed)
        .replace_all(&Default::default())
        .is_err());
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM local_operation_pins WHERE operation_id=?1",
            id
        ),
        0
    );
}

#[test]
fn allowances_pause_without_partial_units_and_increases_do_not_replay_prefix() {
    let f = Fixture::new();
    let item = f.submit("budget");
    let id = item.id.as_str();
    let zero = worker::run(&f.store, id, "worker", 10, 1000, Some(0)).unwrap();
    assert_eq!(zero.checkpoint_sequence, 0);
    assert!(f.outputs(id).is_empty());
    f.resume(id);
    let small = f.run(id, 10, 1);
    assert_eq!(small.checkpoint_sequence, 0);
    assert!(f.outputs(id).is_empty());
    f.resume(id);
    let first = f.run(id, 1, 1000);
    let budget = usize::try_from(first.checkpoint["semantic_records"].as_u64().unwrap()).unwrap();
    let prefix = f.outputs(id);
    f.resume(id);
    let below = f.run(id, 10, budget - 1);
    assert_eq!(below.checkpoint, first.checkpoint);
    f.resume(id);
    let exact = f.run(id, 10, budget);
    assert_eq!(exact.checkpoint, first.checkpoint);
    assert_eq!(f.outputs(id), prefix);
    f.resume(id);
    let done = f.run(id, 10, 1000);
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(&f.outputs(id)[..1], prefix);
    assert_eq!(f.outputs(id), f.serial().1.units);
}

#[test]
fn failed_output_insert_rolls_back_semantics_cursor_and_seal_then_resume_recovers() {
    let f = Fixture::new();
    let item = f.submit("fault");
    let id = item.id.as_str();
    let first = f.run(id, 1, 1000);
    let view = first.checkpoint["ledger_view"].as_str().unwrap();
    let before = serde_json::to_value(f.ledger(&first).load().unwrap()).unwrap();
    let cursor = f.ledger(&first).current_visibility_cursor().unwrap();
    let sql = rusqlite::Connection::open(&f.store).unwrap();
    sql.execute_batch("CREATE TRIGGER fixture_fail_output BEFORE INSERT ON local_operation_outputs BEGIN SELECT RAISE(ABORT,'injected output failure'); END;").unwrap();
    f.resume(id);
    assert!(worker::run(&f.store, id, "worker", 10, 1000, None).is_err());
    let failed = f.db().operation(id).unwrap().unwrap();
    assert_eq!(failed.checkpoint, first.checkpoint);
    assert_eq!(f.outputs(id).len(), 1);
    assert_eq!(
        f.ledger(&first).current_visibility_cursor().unwrap(),
        cursor
    );
    assert_eq!(
        serde_json::to_value(f.ledger(&first).load().unwrap()).unwrap(),
        before
    );
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM local_view_seals WHERE view_id=?1 AND published=0",
            view
        ),
        1
    );
    sql.execute_batch("DROP TRIGGER fixture_fail_output;")
        .unwrap();
    drop(sql);
    f.resume(id);
    assert_eq!(f.run(id, 10, 1000).state, OperationState::Completed);
    assert_eq!(f.outputs(id), f.serial().1.units);
}

#[test]
fn cancellation_cleans_only_its_private_ledger_and_keeps_receipts_and_other_jobs() {
    let f = Fixture::new();
    let a = f.submit("a");
    let b = f.submit("b");
    let prefix = f.run(a.id.as_str(), 1, 1000);
    let other = f.run(b.id.as_str(), 1, 1000);
    f.db()
        .control_operation(a.id.as_str(), OperationControl::Cancel)
        .unwrap();
    let view = prefix.checkpoint["ledger_view"].as_str().unwrap();
    for table in ["scoped_records", "scoped_metadata", "local_view_seals"] {
        assert_eq!(
            f.count(
                &format!("SELECT COUNT(*) FROM {table} WHERE view_id=?1"),
                view
            ),
            0
        );
    }
    assert_eq!(f.outputs(a.id.as_str()).len(), 1);
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM local_operation_pins WHERE operation_id=?1",
            a.id.as_str()
        ),
        0
    );
    assert!(!f.ledger(&other).load().unwrap().artifacts.is_empty());
    assert_eq!(
        f.db()
            .retained_graph(f.request.base_view.as_str())
            .unwrap()
            .snapshot,
        f.base.snapshot
    );
    f.resume(b.id.as_str());
    assert_eq!(
        f.run(b.id.as_str(), 10, 1000).state,
        OperationState::Completed
    );
    assert_eq!(f.outputs(b.id.as_str()), f.serial().1.units);
}

#[test]
fn frozen_inputs_survive_source_changes_and_workspace_retirement() {
    let f = Fixture::new();
    let expected = f.serial().1;
    let item = f.submit("frozen");
    f.run(item.id.as_str(), 1, 1000);
    std::fs::write(f.temp.path().join("head/main.rs"), "not valid Rust").unwrap();
    f.db().retire_workspace("base", 1).unwrap();
    f.db().retire_workspace("head", 1).unwrap();
    assert!(f
        .db()
        .workspaces("fixture/revalidation")
        .unwrap()
        .iter()
        .all(|workspace| !workspace.active));
    f.resume(item.id.as_str());
    assert_eq!(
        f.run(item.id.as_str(), 10, 1000).state,
        OperationState::Completed
    );
    assert_eq!(f.outputs(item.id.as_str()), expected.units);
}

#[test]
fn concurrent_jobs_produce_serial_equivalent_separate_ledgers() {
    let f = Fixture::new();
    let a = f.submit("parallel-a");
    let b = f.submit("parallel-b");
    let results = std::thread::scope(|scope| {
        let first = scope.spawn(|| f.run(a.id.as_str(), 10, 1000));
        let second = scope.spawn(|| f.run(b.id.as_str(), 10, 1000));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_eq!(results.0.state, OperationState::Completed);
    assert_eq!(results.1.state, OperationState::Completed);
    assert_ne!(
        results.0.checkpoint["ledger_view"],
        results.1.checkpoint["ledger_view"]
    );
    assert_eq!(f.outputs(a.id.as_str()), f.serial().1.units);
    assert_eq!(f.outputs(a.id.as_str()), f.outputs(b.id.as_str()));
    assert_eq!(
        serde_json::to_value(f.ledger(&results.0).load().unwrap()).unwrap(),
        serde_json::to_value(f.ledger(&results.1).load().unwrap()).unwrap()
    );
}

#[test]
fn invalid_policy_and_forged_envelopes_are_rejected_before_claim() {
    let f = Fixture::new();
    let mut request = f.request.clone();
    request.units[0].gates.pop();
    assert!(worker::submit(&mut f.db(), request, "invalid", "agent").is_err());
    for (key, repository, views) in [
        (
            "repo",
            "wrong",
            vec![f.request.base_view.clone(), f.request.head_view.clone()],
        ),
        (
            "pins",
            "fixture/revalidation",
            vec![f.request.base_view.clone()],
        ),
    ] {
        let mut views = views;
        views.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let spec = OperationSpec {
            repository_id: repository.into(),
            idempotency_key: key.into(),
            owner: "agent".into(),
            kind: OperationKind::Revalidation,
            views,
            prerequisites: vec![],
            input: serde_json::to_value(&f.request).unwrap(),
            implementation: entmoot_engine::implementation_fingerprint(),
        };
        let submitted = f.db().submit_operations(&[spec], 1000);
        if key == "repo" {
            assert!(submitted.is_err());
            continue;
        }
        let item = submitted.unwrap().pop().unwrap();
        assert!(worker::run(&f.store, item.id.as_str(), "worker", 10, 1000, None).is_err());
        assert_eq!(
            f.db().operation(item.id.as_str()).unwrap().unwrap().state,
            OperationState::Queued
        );
    }
}

#[test]
fn empty_selection_publishes_no_absence_claim_and_reports_all_producers_unknown() {
    let mut f = Fixture::new();
    f.request.units.clear();
    let item = f.submit("empty");
    let done = f.run(item.id.as_str(), 1, 1000);
    assert_eq!(done.state, OperationState::Completed);
    let result: RevalidationResult = serde_json::from_value(done.result.unwrap()).unwrap();
    assert_eq!(result.completed_units, 0);
    assert_eq!(result.semantic_records, 0);
    assert_eq!(result.unselected_artifact_ids.len(), 4);
    assert!(f.outputs(item.id.as_str()).is_empty());
    assert_eq!(
        worker::record_state(
            &f.store,
            item.id.as_str(),
            KnowledgeRecordRef::new("findings", "missing")
        )
        .unwrap()
        .visible,
        None
    );
}

#[test]
fn revalidation_crash_child() {
    let Ok(path) = std::env::var("ENTMOOT_TEST_REVALIDATION_STORE") else {
        return;
    };
    let id = std::env::var("ENTMOOT_TEST_REVALIDATION_ID").unwrap();
    // The parent kills this worker inside the injected SQLite publication stall.
    worker::run(
        std::path::Path::new(&path),
        &id,
        "crash-child",
        10,
        1000,
        None,
    )
    .unwrap();
    panic!("publication stall unexpectedly completed");
}

#[test]
fn killed_process_mid_publication_preserves_prefix_and_resumes_with_new_generation() {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let f = Fixture::new();
    let item = f.submit("kill");
    let id = item.id.as_str();
    let prefix = f.run(id, 1, 1000);
    let expected = f.outputs(id);
    let before = serde_json::to_value(f.ledger(&prefix).load().unwrap()).unwrap();
    f.resume(id);
    let sql = rusqlite::Connection::open(&f.store).unwrap();
    sql.busy_timeout(Duration::ZERO).unwrap();
    sql.execute_batch("CREATE TRIGGER fixture_stall BEFORE INSERT ON local_operation_outputs BEGIN SELECT (WITH RECURSIVE slow(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM slow WHERE n<1000000000) SELECT SUM(n) FROM slow); END;").unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "revalidation_crash_child", "--nocapture"])
        .env("ENTMOOT_TEST_REVALIDATION_STORE", &f.store)
        .env("ENTMOOT_TEST_REVALIDATION_ID", id)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut blocked_since = None;
    let mut stalled = false;
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        match sql.execute_batch("BEGIN IMMEDIATE; ROLLBACK;") {
            Ok(()) => blocked_since = None,
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::DatabaseBusy =>
            {
                let since = blocked_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_millis(250) {
                    stalled = true;
                    break;
                }
            }
            Err(error) => panic!("unexpected SQLite error {error}"),
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        stalled,
        "worker never held the injected publication transaction"
    );
    sql.execute_batch("DROP TRIGGER fixture_stall;").unwrap();
    drop(sql);
    let abandoned = f.db().operation(id).unwrap().unwrap();
    assert_eq!(abandoned.state, OperationState::Running);
    assert_eq!(abandoned.checkpoint, prefix.checkpoint);
    assert_eq!(f.outputs(id), expected);
    assert_eq!(
        serde_json::to_value(f.ledger(&prefix).load().unwrap()).unwrap(),
        before
    );
    assert!(f.ledger(&prefix).replace_all(&Default::default()).is_err());
    f.db()
        .control_operation(id, OperationControl::Pause)
        .unwrap();
    f.resume(id);
    let completed = f.run(id, 10, 1000);
    assert!(completed.generation > abandoned.generation);
    assert_eq!(completed.state, OperationState::Completed);
    assert_eq!(f.outputs(id), f.serial().1.units);
}

#[test]
#[ignore = "explicit local revalidation cost probe"]
fn revalidation_restart_cost_probe() {
    use std::time::Instant;
    let start = Instant::now();
    let f = Fixture::with_extra_policies(29);
    let setup = start.elapsed().as_millis();
    let initial_bytes = std::fs::metadata(&f.store).unwrap().len();
    let mut serial_ms = vec![];
    let mut durable_ms = vec![];
    let mut sliced_ms = vec![];
    let mut growth_bytes = vec![];
    let mut expected = None;
    for sample in 0..3 {
        // Alternate to avoid assigning every colder run to one mode. All modes
        // load retained inputs and publish into a new view of the same store.
        for mode in if sample % 2 == 0 {
            ["serial", "durable", "sliced"]
        } else {
            ["sliced", "durable", "serial"]
        } {
            let bytes_before = std::fs::metadata(&f.store).unwrap().len();
            let start = Instant::now();
            if mode == "serial" {
                let db = f.db();
                let base = db.retained_graph(f.request.base_view.as_str()).unwrap();
                let head = db.retained_graph(f.request.head_view.as_str()).unwrap();
                let view =
                    ContentDigest::from_trusted(blake3_hex(format!("serial-{sample}").as_bytes()));
                let mut ledger =
                    SqliteKnowledgeRepository::open_view(&f.store, view.as_str()).unwrap();
                let receipt = entmoot_engine::revalidate_transition(
                    &mut ledger,
                    &base,
                    &head,
                    &TransitionRevalidationRequest {
                        base_snapshot: base.snapshot.clone(),
                        head_snapshot: head.snapshot.clone(),
                        units: f.request.units.clone(),
                        max_records: 1_000_000,
                    },
                )
                .unwrap();
                drop(ledger);
                drop(db);
                serial_ms.push(start.elapsed().as_millis());
                if let Some(ref units) = expected {
                    assert_eq!(&receipt.units, units);
                } else {
                    expected = Some(receipt.units);
                }
            } else {
                let item = f.submit(&format!("{mode}-{sample}"));
                loop {
                    let result = f.run(
                        item.id.as_str(),
                        if mode == "sliced" { 1 } else { 1000 },
                        1_000_000,
                    );
                    if result.state == OperationState::Completed {
                        break;
                    }
                    f.resume(item.id.as_str());
                }
                let elapsed = start.elapsed().as_millis();
                if mode == "sliced" {
                    sliced_ms.push(elapsed);
                } else {
                    durable_ms.push(elapsed);
                }
                if let Some(ref units) = expected {
                    assert_eq!(&f.outputs(item.id.as_str()), units);
                }
            }
            growth_bytes.push(serde_json::json!({"mode":mode,"sample":sample,"bytes":std::fs::metadata(&f.store).unwrap().len()-bytes_before}));
        }
    }
    println!(
        "{}",
        serde_json::json!({"policies":32,"samples":3,"setup_ms":setup,"serial_ms":serial_ms,"durable_ms":durable_ms,"single_policy_slices_ms":sliced_ms,"growth_bytes":growth_bytes,"initial_store_bytes":initial_bytes,"final_store_bytes":std::fs::metadata(&f.store).unwrap().len()})
    );
}
