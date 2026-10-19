#[cfg(test)]
mod tests {
    use entmoot_contracts::{SourceSnapshot, SourceState};
    use entmoot_core::{build_file_index, StructuralHasher};

    use entmoot_contracts::query::*;
    use entmoot_contracts::*;
    use entmoot_query::extractor::MarkerTopologyExtractor;
    use entmoot_query::graph::{file_node_id, SnapshotFileDigest};
    use entmoot_query::*;

    fn graph(source: &str, revision: char) -> TopologyGraph {
        let repository_id = "example/repo";
        let result = StructuralHasher::new()
            .compute_granular_hash_with_analysis_at(
                source,
                "python",
                Some("app.py"),
                Some(repository_id),
                None,
            )
            .unwrap();
        let snapshot = SourceSnapshot {
            repository_id: repository_id.into(),
            revision: revision.to_string().repeat(40),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let checkout = if cfg!(windows) {
            "C:/checkout"
        } else {
            "/checkout"
        };
        let mut extractor = MarkerTopologyExtractor::with_snapshot(checkout, snapshot);
        extractor.on_repo_begin(checkout);
        extractor.on_file("app.py", &build_file_index(&result), Some(source));
        extractor.on_file_end(&file_node_id("app.py"));
        let mut graph = extractor.finish();
        graph.snapshot_files = vec![SnapshotFileDigest {
            path: "app.py".into(),
            content_digest: format!("blake3:{}", revision.to_string().repeat(64)),
        }]
        .into();
        graph
    }

    #[test]
    fn change_inventory_keeps_body_boundary_and_unknown_evidence_without_cold_details() {
        let base = graph("def entmoot_delta(value):\n    return value\n", 'a');
        let head = graph(
            "def entmoot_delta(value, extra=0):\n    return value + 1\n",
            'b',
        );
        let changes = compare_revision_changes(&base, &head).unwrap();
        assert_eq!(changes.files.len(), 1);
        assert_eq!(changes.files[0].path, "app.py");
        assert_eq!(changes.callables.len(), 1);
        assert_eq!(
            changes.callables[0].kind,
            CallableImpactKind::BodyAndBoundaryChanged
        );
        assert!(changes
            .gaps
            .iter()
            .any(|gap| gap == "resolved_identity_evidence_missing_on_one_or_both_snapshots"));
        let complete = compare_revision_impact(&base, &head).unwrap();
        assert_eq!(changes.files, complete.files);
        assert_eq!(changes.callables, complete.callables);
        assert_eq!(changes.gaps, complete.gaps);
        assert_eq!(
            changes.signal_limitations,
            complete.breaking_change_signals.limitations
        );
    }

    #[test]
    fn body_and_boundary_changes_are_distinct_and_deterministic() {
        let base = graph("def work(value):\n    return value\n", 'a');
        let body = graph("def work(value):\n    return value + 1\n", 'b');
        let boundary = graph("def work(value, extra=0):\n    return value\n", 'c');

        let body_impact = compare_revision_impact(&base, &body).unwrap();
        assert!(body_impact.breaking_change_signals.signals.is_empty());
        assert_eq!(
            body_impact
                .breaking_change_signals
                .non_signals
                .body_only_changes,
            1
        );
        assert_eq!(body_impact.files.len(), 1);
        assert_eq!(body_impact.callables.len(), 1);
        assert_eq!(
            body_impact.callables[0].kind,
            CallableImpactKind::BodyChanged
        );

        let boundary_impact = compare_revision_impact(&base, &boundary).unwrap();
        assert_eq!(boundary_impact.breaking_change_signals.signals.len(), 1);
        assert_eq!(
            boundary_impact.breaking_change_signals.signals[0].callable,
            boundary_impact.callables[0]
        );
        assert_eq!(boundary_impact.callables.len(), 1);
        assert_eq!(
            boundary_impact.callables[0].kind,
            CallableImpactKind::BodyAndBoundaryChanged
        );
    }

    #[test]
    fn cross_repository_comparison_is_rejected_without_correspondence() {
        let base = graph("def work():\n    pass\n", 'a');
        let mut head = graph("def work():\n    pass\n", 'b');
        head.snapshot.repository_id = "other/repo".into();
        assert!(matches!(
            compare_revision_impact(&base, &head),
            Err(ImpactError::RepositoryMismatch { .. })
        ));
    }

    #[test]
    fn same_purl_subject_joins_across_materialization_roots() {
        use entmoot_contracts::SourceSubject;
        let mut base = graph("def work():\n    return 1\n", 'a');
        let mut head = graph("def work():\n    return 2\n", 'b');
        head.snapshot.repository_id = "other/extract".into();
        let subject = SourceSubject::Package {
            coordinate_purl: "pkg:pypi/demo".into(),
        };
        base.snapshot.subject = Some(subject.clone());
        head.snapshot.subject = Some(subject);
        let impact = compare_revision_impact(&base, &head).unwrap();
        assert_eq!(impact.callables.len(), 1);
        assert_eq!(impact.callables[0].kind, CallableImpactKind::BodyChanged);
        assert!(impact.correspondence_receipt.is_some());
    }

    #[test]
    fn explicit_alias_policy_joins_fork_subjects() {
        use entmoot_contracts::{
            CorrespondenceMode, CorrespondencePolicy, SourceSubject, SubjectAlias,
        };
        let mut base = graph("def work():\n    return 1\n", 'a');
        let mut head = graph("def work():\n    return 2\n", 'b');
        head.snapshot.repository_id = "fork/extract".into();
        base.snapshot.subject = Some(SourceSubject::Repository {
            id: "github.com/upstream/demo".into(),
            path: None,
        });
        head.snapshot.subject = Some(SourceSubject::Repository {
            id: "github.com/fork/demo".into(),
            path: None,
        });
        let policy = CorrespondencePolicy {
            mode: CorrespondenceMode::ExplicitAlias,
            subject: None,
            aliases: vec![SubjectAlias {
                from: SourceSubject::Repository {
                    id: "github.com/fork/demo".into(),
                    path: None,
                },
                to: SourceSubject::Repository {
                    id: "github.com/upstream/demo".into(),
                    path: None,
                },
            }],
        };
        let impact =
            compare_revision_impact_with_correspondence(&base, &head, Some(&policy)).unwrap();
        assert_eq!(impact.callables.len(), 1);
        assert_eq!(
            impact.correspondence_receipt.as_ref().unwrap().mode,
            CorrespondenceMode::ExplicitAlias
        );
    }

    #[test]
    fn relative_occurrence_policy_requires_subject_or_override() {
        use entmoot_contracts::{CorrespondenceMode, CorrespondencePolicy, SourceSubject};
        let base = graph("def work():\n    pass\n", 'a');
        let mut head = graph("def work():\n    pass\n", 'b');
        head.snapshot.repository_id = "other/repo".into();
        let bare = CorrespondencePolicy {
            mode: CorrespondenceMode::RelativeOccurrence,
            subject: None,
            aliases: Vec::new(),
        };
        assert!(matches!(
            compare_revision_impact_with_correspondence(&base, &head, Some(&bare)),
            Err(ImpactError::CorrespondenceInvalid { .. })
        ));
        let with_override = CorrespondencePolicy {
            mode: CorrespondenceMode::RelativeOccurrence,
            subject: Some(SourceSubject::Package {
                coordinate_purl: "pkg:pypi/demo".into(),
            }),
            aliases: Vec::new(),
        };
        assert!(
            compare_revision_impact_with_correspondence(&base, &head, Some(&with_override)).is_ok()
        );
    }

    fn fixture_binding(
        source: entmoot_contracts::ResolverEvidenceRef,
        completeness: ResolvedIdentityCompleteness,
    ) -> entmoot_contracts::ResolvedBindingEvidence {
        use entmoot_contracts::ResolvedBindingEvidence;
        match completeness {
            ResolvedIdentityCompleteness::Complete => ResolvedBindingEvidence::Complete {
                source,
                reference_count: 1,
            },
            ResolvedIdentityCompleteness::Partial => ResolvedBindingEvidence::Partial {
                source,
                reference_count: 1,
                unresolved_count: 1,
                ambiguous_count: 0,
                reason_codes: vec!["fixture_unresolved_reference".into()],
            },
            ResolvedIdentityCompleteness::Unavailable => ResolvedBindingEvidence::Unavailable {
                reason_codes: vec!["no_artifact".into()],
            },
        }
    }

    fn bind(graph: &mut TopologyGraph, digest: char, completeness: ResolvedIdentityCompleteness) {
        use entmoot_contracts::{ArtifactFormat, ArtifactProducer, ScopeUniverse};
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "fixture-resolver".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            ScopeUniverse::from_selectors(
                "fixture",
                vec![],
                vec!["**".into()],
                vec![],
                ScopeCompleteness::Partial,
                None,
            ),
            entmoot_contracts::blake3_hex(digest.to_string().as_bytes()),
            None,
        )
        .unwrap();
        let address_id = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Callable)
            .unwrap()
            .address_id
            .clone()
            .unwrap();
        graph.resolved_identities.push(ResolvedIdentityRecord {
            address_id,
            resolved_hash: format!("blake3:{}", digest.to_string().repeat(64)),
            resolution_policy_id: "a".repeat(64),
            binding: fixture_binding(
                entmoot_contracts::ResolverEvidenceRef {
                    producer: entmoot_contracts::ProducerIdentity {
                        producer_id: "fixture-resolver".into(),
                        version: Some("1".into()),
                        tool_name: None,
                    },
                    artifact: entmoot_contracts::ArtifactRef::from_manifest(&manifest).unwrap(),
                },
                completeness,
            ),
        });
        graph.artifacts.push(manifest);
    }

    #[test]
    fn binding_only_change_preserves_structural_identity_and_attribution() {
        let mut base = graph("def work():\n    return 1\n", 'a');
        let mut head = graph("def work():\n    return 1\n", 'b');
        head.snapshot_files = base.snapshot_files.clone();
        bind(&mut base, 'a', ResolvedIdentityCompleteness::Complete);
        bind(&mut head, 'b', ResolvedIdentityCompleteness::Complete);
        let before = serde_json::to_value(&base).unwrap();
        let impact = compare_revision_impact(&base, &head).unwrap();
        assert!(impact.files.is_empty());
        assert!(impact.callables.is_empty());
        assert!(impact.breaking_change_signals.signals.is_empty());
        assert_eq!(
            impact.breaking_change_signals.non_signals.binding_changes,
            1
        );
        assert_eq!(impact.bindings.len(), 1);
        assert_eq!(impact.bindings[0].kind, BindingImpactKind::Changed);
        assert_eq!(impact.bindings[0].base, base.resolved_identities);
        assert_eq!(impact.bindings[0].head, head.resolved_identities);
        assert_eq!(serde_json::to_value(&base).unwrap(), before);

        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("bindings.sqlite3");
        use entmoot_query::KnowledgeRepository;
        let changed = |graph, path| {
            // Each case publishes its evidence; changing an in-memory graph
            // alone cannot override the knowledge source of record.
            entmoot_query::SqliteKnowledgeRepository::open(&store)
                .unwrap()
                .replace_all(&entmoot_query::KnowledgeStore::extract_from_graph(&graph))
                .unwrap();
            entmoot_query::QueryEngine::new(graph, store.clone())
                .with_compare_graph(base.clone())
                .find(entmoot_contracts::query::FindQuery::ChangedSubject {
                    compare_revision: base.snapshot.revision.clone(),
                    compare_snapshot_digest: None,
                    path,
                    limit: Some(5),
                })
                .unwrap()
                .into_value()
                .unwrap()
        };
        assert_eq!(
            changed(head.clone(), None)["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(changed(head.clone(), Some("other.py".into()))["items"]
            .as_array()
            .unwrap()
            .is_empty());
        head.resolved_identities[0].binding = fixture_binding(
            head.resolved_identities[0]
                .binding
                .source()
                .unwrap()
                .clone(),
            ResolvedIdentityCompleteness::Partial,
        );
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].reasons,
            vec![BindingImpactReason::IncompleteEvidence]
        );
        assert!(changed(head, None)["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn partial_equal_missing_and_version_drift_bindings_never_authorize_reuse() {
        let mut base = graph("def work():\n    return 1\n", 'a');
        let mut head = graph("def work():\n    return 1\n", 'b');
        bind(&mut base, 'a', ResolvedIdentityCompleteness::Complete);
        bind(&mut head, 'a', ResolvedIdentityCompleteness::Complete);
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].kind,
            BindingImpactKind::Unchanged
        );
        head.resolved_identities[0].binding = fixture_binding(
            head.resolved_identities[0]
                .binding
                .source()
                .unwrap()
                .clone(),
            ResolvedIdentityCompleteness::Partial,
        );
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].kind,
            BindingImpactKind::NotComparable
        );
        let mut source = head.resolved_identities[0]
            .binding
            .source()
            .unwrap()
            .clone();
        source.producer.version = Some("2".into());
        head.resolved_identities[0].binding =
            fixture_binding(source, ResolvedIdentityCompleteness::Complete);
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].reasons,
            vec![BindingImpactReason::ProducerSeriesOrPolicyNotComparable]
        );
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].kind,
            BindingImpactKind::NotComparable
        );
        head.resolved_identities.clear();
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].reasons,
            vec![BindingImpactReason::MissingEvidence]
        );
        assert_eq!(
            compare_revision_impact(&base, &head).unwrap().bindings[0].kind,
            BindingImpactKind::NotComparable
        );
    }

    #[test]
    fn unresolved_attempts_partition_without_becoming_negative_evidence() {
        let base = graph("def work(callback):\n    return callback()\n", 'a');
        let head = graph("def work(callback):\n    return callback()\n", 'b');
        assert!(!base.unresolved_relations.is_empty());
        let impact = compare_revision_impact(&base, &head).unwrap();
        assert_eq!(
            impact
                .evidence
                .iter()
                .filter(|r| r.record_kind == "unresolved_relation")
                .count(),
            base.unresolved_relations.len()
        );
        assert!(impact
            .evidence
            .iter()
            .filter(|r| r.record_kind == "unresolved_relation")
            .all(|r| r.disposition == EvidenceImpactKind::Stale));
        assert_eq!(
            impact
                .introduced_evidence
                .iter()
                .filter(|r| r.record_kind == "unresolved_relation")
                .count(),
            head.unresolved_relations.len()
        );
        let identical = compare_revision_impact(&base, &base).unwrap();
        assert!(identical.introduced_evidence.is_empty());
        assert!(identical.breaking_change_signals.signals.is_empty());
        assert!(impact.breaking_change_signals.signals.is_empty());
        assert!(
            impact
                .breaking_change_signals
                .non_signals
                .stale_evidence_records
                > 0
        );
        assert!(identical
            .evidence
            .iter()
            .all(|r| r.disposition == EvidenceImpactKind::Retained));
    }

    #[test]
    fn resolver_projection_is_stable_across_storage_row_order() {
        let mut source = graph("def leaf():\n    return 1\ndef work(callback):\n    leaf()\n    callback()\n    missing()\n", 'a');
        bind(&mut source, 'a', ResolvedIdentityCompleteness::Complete);
        bind(&mut source, 'b', ResolvedIdentityCompleteness::Complete);
        let mut knowledge = entmoot_query::KnowledgeStore::extract_from_graph(&source);
        let first = knowledge.apply_to_inventory(entmoot_query::inventory_skeleton(&source));
        knowledge.resolved_identities.reverse();
        knowledge.artifacts.reverse();
        knowledge.relation_edges.reverse();
        knowledge.relation_claims.reverse();
        knowledge.unresolved_relations.reverse();
        let replay = knowledge.apply_to_inventory(entmoot_query::inventory_skeleton(&source));
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&replay).unwrap()
        );
    }

    #[test]
    fn repeated_series_has_one_rescan_and_ambiguous_bindings_stay_open() {
        let mut base = graph("def work():\n    return 1\n", 'a');
        let mut head = graph("def work():\n    return 2\n", 'b');
        bind(&mut base, 'a', ResolvedIdentityCompleteness::Complete);
        bind(&mut base, 'b', ResolvedIdentityCompleteness::Complete);
        bind(&mut head, 'c', ResolvedIdentityCompleteness::Complete);
        let impact = compare_revision_impact(&base, &head).unwrap();
        assert_eq!(impact.rescans.len(), 1);
        assert!(!impact.rescans[0].satisfied_by_head_artifact);
        assert_eq!(impact.bindings[0].kind, BindingImpactKind::NotComparable);
        assert_eq!(impact.bindings[0].base.len(), 2);
    }
    #[test]
    fn boundary_only_signal_and_missing_inventory_fail_closed() {
        let base = graph("def work(value):\n    return value\n", 'a');
        let mut head = base.clone();
        head.snapshot.revision = "b".repeat(40);
        let node = head
            .nodes
            .iter_mut()
            .find(|n| n.kind == NodeKind::Callable)
            .unwrap();
        // Contract-level boundary-only control: body and occurrence stay exact.
        node.signature_hash = Some(entmoot_contracts::blake3_hex(b"changed boundary"));
        let receipt = compare_revision_impact(&base, &head).unwrap();
        assert_eq!(
            receipt.callables[0].kind,
            CallableImpactKind::BoundaryChanged
        );
        assert_eq!(
            receipt.breaking_change_signals.signals[0].kind,
            BreakingChangeSignalKind::BoundaryChanged
        );
        let mut missing = head.clone();
        missing.diagnostics.push(
            entmoot_query::ArtifactDiagnostic::error("source.hash_failed", "fixture parse failure")
                .with_pointer("app.py"),
        );
        assert!(compare_revision_impact(&base, &missing)
            .unwrap()
            .breaking_change_signals
            .signals
            .is_empty());
        missing = head.clone();
        missing.policy_ids.clear();
        assert!(compare_revision_impact(&base, &missing)
            .unwrap()
            .breaking_change_signals
            .signals
            .is_empty());
    }
}
