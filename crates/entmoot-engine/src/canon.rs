//! Explicit canon production. Queries only consume the retained claims.
use crate::{NodeKind, TopologyGraph};
use entmoot_contracts::*;
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonProduction {
    pub parsed_files: usize,
    pub reused_files: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CanonProduceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Core(#[from] entmoot_core::CoreError),
    #[error(transparent)]
    Contract(#[from] CanonError),
    #[error(transparent)]
    Repository(#[from] crate::KnowledgeRepositoryError),
    #[error("canon source identity changed: {0}")]
    Drift(String),
    #[error("canon requires UTF-8 source: {0}")]
    Encoding(String),
}

fn digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_trusted(format!("blake3:{}", blake3::hash(bytes).to_hex()))
}

fn claim(
    observation: CanonObservation,
    graph: &TopologyGraph,
    context: &ContentDigest,
    path: Option<&str>,
) -> Result<RelationClaim, CanonProduceError> {
    observation.validate()?;
    let value_digest = observation.digest()?;
    let claim = relation_claim_with_id(
        SubjectId::Address {
            address: Address::Opaque {
                system: "entmoot.canon".into(),
                value: path.unwrap_or("graph").into(),
            },
        },
        if path.is_some() {
            "canon.file"
        } else {
            "canon.graph"
        },
        RelationClaimValue::Canon {
            observation: Box::new(observation),
            digest: value_digest,
        },
        ProducerRef {
            id: "entmoot.canon".into(),
            name: "canon".into(),
            version: Some("v1".into()),
            config_fingerprint: Some(CANON_SCHEMA.into()),
            environment_fingerprint: Some(context.to_string()),
            capabilities: vec![],
        },
        Scope {
            domain: graph.snapshot.repository_id.clone(),
            snapshot_revision: Some(graph.snapshot.revision.clone()),
            snapshot_digest: graph.snapshot.snapshot_digest.clone(),
            occurrence_id: None,
            context_id: Some(context.to_string()),
        },
        vec![],
        None,
    );
    Ok(claim)
}

/// Durable counters are private operational state, never evidence on their own.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CanonCursor {
    pub completed: usize,
    patterns_left: usize,
    total_bytes: usize,
    limits: Vec<CanonLimit>,
    stopped: bool,
    production: CanonProduction,
}

impl CanonCursor {
    pub(crate) fn follows(&self, prior: &Self) -> bool {
        self.completed >= prior.completed
            && self.patterns_left <= prior.patterns_left
            && self.total_bytes >= prior.total_bytes
            && self.production.parsed_files >= prior.production.parsed_files
            && self.production.reused_files >= prior.production.reused_files
            && self
                .production
                .parsed_files
                .saturating_add(self.production.reused_files)
                <= self.completed
            && (!prior.stopped || self.stopped)
            && prior.limits.iter().all(|limit| self.limits.contains(limit))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CanonFileOutcome {
    pub input: entmoot_core::scan::ScanInput,
    claim: Option<RelationClaim>,
    edges: Vec<RelationEdge>,
}

impl CanonFileOutcome {
    pub(crate) fn valid_for(&self, request: &ProducerRequest) -> bool {
        if !matches!(request.task, ProducerTask::Canon { .. }) {
            return false;
        }
        let Some(claim) = &self.claim else {
            return self.edges.is_empty();
        };
        let RelationClaimValue::Canon {
            observation,
            digest,
        } = &claim.value
        else {
            return false;
        };
        let CanonObservation::File { file } = observation.as_ref() else {
            return false;
        };
        file.path == self.input.path
            && file.source_digest.as_str() == self.input.content_digest
            && observation.validate().is_ok()
            && observation.digest().is_ok_and(|value| &value == digest)
            && claim.producer.id == "entmoot.canon"
            && claim.scope.domain == request.source.snapshot.repository_id
            && claim.scope.snapshot_revision.as_ref() == Some(&request.source.snapshot.revision)
            && claim.scope.snapshot_digest == request.source.snapshot.snapshot_digest
    }
}

/// Immutable input indexes are reconstructed once per invocation from its pinned
/// view; completed folds and cumulative budgets are recovered from the repository.
pub(crate) struct CanonPlan<'a> {
    graph: &'a TopologyGraph,
    context: ContentDigest,
    source_manifest: ContentDigest,
    files: Vec<(String, Vec<(String, CodeContract)>)>,
    snapshot: BTreeMap<&'a str, &'a str>,
    nodes_by_address: BTreeMap<&'a str, &'a crate::TopologyNode>,
    facts_by_path: BTreeMap<&'a str, Vec<CanonExpressionFact>>,
    native_context: &'a [CanonNativeContext],
    previous: Option<&'a dyn crate::KnowledgeRepository>,
    hasher: entmoot_core::StructuralHasher,
    truncated_files: bool,
}

pub(crate) struct CanonProjection {
    claims: Vec<RelationClaim>,
    edges: Vec<RelationEdge>,
}

impl CanonProjection {
    pub(crate) fn apply(self, graph: &mut TopologyGraph) {
        // Other producers and immutable historical publications remain intact.
        graph
            .relation_claims
            .retain(|c| c.producer.id != "entmoot.canon");
        graph.relation_edges.retain(|e| e.schema_domain != "canon");
        graph.relation_claims.extend(self.claims);
        graph.relation_edges.extend(self.edges);
        graph.rebuild_indexes();
    }
}

impl<'a> CanonPlan<'a> {
    pub(crate) fn new(
        graph: &'a TopologyGraph,
        facts: &'a [CanonExpressionFact],
        native_context: &'a [CanonNativeContext],
        previous: Option<&'a dyn crate::KnowledgeRepository>,
    ) -> Result<Self, CanonProduceError> {
        let context = digest(&serde_json::to_vec(&(
            CANON_SCHEMA,
            &graph.snapshot,
            &graph.resolved_identities,
            facts,
            native_context,
        ))?);
        let mut files = BTreeMap::<String, Vec<_>>::new();
        for node in &graph.nodes {
            if node.kind == NodeKind::Callable {
                if let (Some(path), Some(contract)) = (&node.path, &node.code_contract) {
                    files
                        .entry(path.clone())
                        .or_default()
                        .push((node.id.clone(), contract.clone()));
                }
            }
        }
        let mut facts_by_path = BTreeMap::<&str, Vec<_>>::new();
        for fact in facts {
            facts_by_path
                .entry(&fact.site.path)
                .or_default()
                .push(fact.clone());
        }
        let previous = if let Some(repository) = previous {
            let manifests = repository.relation_claims_for_address(
                &Address::Opaque {
                    system: "entmoot.canon".into(),
                    value: "graph".into(),
                },
                "canon.graph",
                2,
            )?;
            match manifests.as_slice() {
                [RelationClaim {
                    value: RelationClaimValue::Canon { observation, .. },
                    ..
                }] if matches!(observation.as_ref(), CanonObservation::Graph { graph } if graph.implementation == crate::implementation_fingerprint()) => {
                    Some(repository)
                }
                _ => None,
            }
        } else {
            None
        };
        Ok(Self {
            context,
            source_manifest: digest(&serde_json::to_vec(&graph.snapshot_files)?),
            truncated_files: files.len() > 10_000,
            files: files.into_iter().take(10_000).collect(),
            snapshot: graph
                .snapshot_files
                .iter()
                .map(|f| (f.path.as_str(), f.content_digest.as_str()))
                .collect(),
            nodes_by_address: graph
                .nodes
                .iter()
                .filter_map(|n| n.address_id.as_deref().map(|id| (id, n)))
                .collect(),
            graph,
            facts_by_path,
            native_context,
            previous,
            hasher: entmoot_core::StructuralHasher::new(),
        })
    }

    pub(crate) fn inputs(&self) -> Result<Vec<entmoot_core::scan::ScanInput>, CanonProduceError> {
        self.files
            .iter()
            .map(|(path, _)| {
                Ok(entmoot_core::scan::ScanInput {
                    path: path.clone(),
                    content_digest: self
                        .snapshot
                        .get(path.as_str())
                        .ok_or_else(|| CanonProduceError::Drift(path.clone()))?
                        .to_string(),
                })
            })
            .collect()
    }

    pub(crate) fn cursor(&self) -> CanonCursor {
        CanonCursor {
            completed: 0,
            patterns_left: 100_000,
            total_bytes: 0,
            stopped: false,
            production: CanonProduction::default(),
            limits: if self.truncated_files {
                vec![CanonLimit::BudgetExceeded]
            } else {
                vec![]
            },
        }
    }

    pub(crate) fn batch(
        &self,
        root: &Path,
        cursor: &mut CanonCursor,
        limit: usize,
    ) -> Result<Vec<CanonFileOutcome>, CanonProduceError> {
        if cursor.completed > self.files.len() || cursor.patterns_left > 100_000 || limit == 0 {
            return Err(CanonError::Invalid("file checkpoint bounds").into());
        }
        let end = cursor.completed.saturating_add(limit).min(self.files.len());
        let mut outcomes = Vec::with_capacity(end - cursor.completed);
        while cursor.completed < end {
            outcomes.push(self.file(root, cursor)?);
            cursor.completed += 1;
        }
        Ok(outcomes)
    }

    fn file(
        &self,
        root: &Path,
        cursor: &mut CanonCursor,
    ) -> Result<CanonFileOutcome, CanonProduceError> {
        let (path, owners) = &self.files[cursor.completed];
        let graph = self.graph;
        let context = &self.context;
        let mut outcome = CanonFileOutcome {
            input: entmoot_core::scan::ScanInput {
                path: path.clone(),
                content_digest: self
                    .snapshot
                    .get(path.as_str())
                    .ok_or_else(|| CanonProduceError::Drift(path.clone()))?
                    .to_string(),
            },
            claim: None,
            edges: vec![],
        };
        if cursor.stopped {
            return Ok(outcome);
        }
        if cursor.patterns_left == 0 {
            cursor.limits.push(CanonLimit::BudgetExceeded);
            cursor.stopped = true;
            return Ok(outcome);
        }
        if std::fs::metadata(root.join(path))?.len() > 10_000_000 {
            cursor.limits.push(CanonLimit::BudgetExceeded);
            return Ok(outcome);
        }
        let bytes = std::fs::read(root.join(path))?;
        let source_digest = digest(&bytes);
        if self.snapshot.get(path.as_str()).copied() != Some(source_digest.as_str()) {
            return Err(CanonProduceError::Drift(path.clone()));
        }
        if bytes.len() > 10_000_000 {
            cursor.limits.push(CanonLimit::BudgetExceeded);
            return Ok(outcome);
        }
        let source =
            std::str::from_utf8(&bytes).map_err(|_| CanonProduceError::Encoding(path.clone()))?;
        let language = owners[0].1.language.clone();
        let contracts: Vec<_> = owners.iter().map(|(_, c)| c.clone()).collect();
        let by_start: BTreeMap<_, _> = owners
            .iter()
            .map(|(id, c)| (c.site.span.start_byte, id))
            .collect();
        let native: Vec<_> = self
            .facts_by_path
            .get(path.as_str())
            .cloned()
            .unwrap_or_default();
        let input_digest = digest(&serde_json::to_vec(&(
            &source_digest,
            &contracts,
            &native,
            cursor.patterns_left,
        ))?);
        let cached = if let Some(repository) = self.previous {
            repository
                .relation_claims_for_address(
                    &Address::Opaque {
                        system: "entmoot.canon".into(),
                        value: path.clone(),
                    },
                    "canon.file",
                    2,
                )?
                .into_iter()
                .find_map(|c| match c.value {
                    RelationClaimValue::Canon { observation, .. } => match *observation {
                        CanonObservation::File { file }
                            if file.path == *path
                                && file.input_digest == input_digest
                                && file.source_digest == source_digest
                                && file.language == language
                                && file.patterns.len() <= cursor.patterns_left =>
                        {
                            Some(file)
                        }
                        _ => None,
                    },
                    _ => None,
                })
        } else {
            None
        };
        let (mut patterns, mut forms, alpha_unavailable_callables, file_limits) =
            if let Some(file) = cached {
                cursor.production.reused_files += 1;
                (
                    file.patterns,
                    file.forms,
                    file.alpha_unavailable_callables,
                    file.limitations,
                )
            } else {
                cursor.production.parsed_files += 1;
                let analysis = self.hasher.canon_file(
                    source,
                    &language,
                    &contracts,
                    &native,
                    cursor.patterns_left,
                )?;
                let mut patterns = Vec::new();
                for pattern in analysis.clones.patterns {
                    if let Some(id) = by_start.get(&pattern.owner_start) {
                        patterns.push(CloneOccurrence {
                            node_id: (*id).clone(),
                            path: path.clone(),
                            span: pattern.span,
                            extent: pattern.extent,
                            token_count: pattern.token_count,
                            fingerprints: pattern.fingerprints,
                        });
                    }
                }
                (
                    patterns,
                    analysis.forms,
                    analysis.clones.alpha_unavailable_callables,
                    if analysis.clones.truncated {
                        vec![CanonLimit::BudgetExceeded]
                    } else {
                        vec![]
                    },
                )
            };
        // Rebind occurrence node IDs and provenance even when the pure fold is reusable.
        for pattern in &mut patterns {
            if pattern.extent == CloneExtent::Callable {
                if let Some(id) = by_start.get(&pattern.span.start_byte) {
                    pattern.node_id = (*id).clone();
                }
            }
        }
        cursor.patterns_left = cursor
            .patterns_left
            .checked_sub(patterns.len())
            .ok_or(CanonError::Invalid("file pattern budget"))?;
        let mut file_edges = Vec::new();
        let resolver_digest = digest(&serde_json::to_vec(&native)?);
        for form in &mut forms {
            form.inputs = None;
            if let Some(node) = self.nodes_by_address.get(form.owner.as_str()) {
                if let (Some(hash), Some(policy), Some(exact), Some(signature)) = (
                    &node.structural_hash,
                    graph.policy_ids.first(),
                    &node.exact_body_id,
                    &node.signature_hash,
                ) {
                    let mut inputs = BTreeMap::new();
                    for (name, value) in [
                        ("exact_body", ContentDigest::from_trusted(exact)),
                        ("occurrence", digest(form.owner.as_bytes())),
                        ("source_manifest", self.source_manifest.clone()),
                        ("build_context", context.clone()),
                        ("resolver_output", resolver_digest.clone()),
                        ("normalization_policy", digest(CANON_SCHEMA.as_bytes())),
                        (
                            "evaluation_policy",
                            digest(b"canon.bounded_scalars.4096.128.v1"),
                        ),
                        ("signature_hash", ContentDigest::from_trusted(signature)),
                    ] {
                        inputs.insert(name.into(), PropertyInputValue::Known { digest: value });
                    }
                    form.inputs = Some(PropertyInputManifest {
                        lookup: PropertyStoreKey {
                            policy_id: PolicyId::from_trusted(policy),
                            structural_hash: StructuralHash::from_trusted(hash),
                            property_schema_id: CANON_SCHEMA.into(),
                        },
                        snapshot: graph.snapshot.clone(),
                        inputs,
                    });
                }
                let edge = relation_edge_with_id(
                    "canon",
                    Address::AddressId {
                        address_id: form.owner.clone(),
                    },
                    RelationKind::HasCanonicalForm,
                    Address::CanonicalForm {
                        digest: form.fingerprint(),
                    },
                    Scope {
                        domain: graph.snapshot.repository_id.clone(),
                        snapshot_revision: Some(graph.snapshot.revision.clone()),
                        snapshot_digest: graph.snapshot.snapshot_digest.clone(),
                        occurrence_id: Some(form.owner.clone()),
                        context_id: Some(context.to_string()),
                    },
                    Some(form.owner.clone()),
                );
                file_edges.push(edge);
            }
        }
        let file = CanonFile {
            path: path.clone(),
            language,
            source_digest,
            input_digest,
            patterns,
            forms,
            alpha_unavailable_callables,
            limitations: file_limits,
        };
        let observation = CanonObservation::File { file };
        let encoded = serde_json::to_vec(&observation)?;
        cursor.total_bytes += encoded.len();
        if encoded.len() > 4_000_000 || cursor.total_bytes > 64_000_000 {
            cursor.limits.push(CanonLimit::BudgetExceeded);
            cursor.stopped = true;
            return Ok(outcome);
        }

        outcome.claim = Some(claim(observation, graph, context, Some(path))?);
        outcome.edges = file_edges;
        Ok(outcome)
    }

    pub(crate) fn finish(
        &self,
        cursor: &CanonCursor,
        outcomes: Vec<CanonFileOutcome>,
    ) -> Result<CanonProjection, CanonProduceError> {
        if cursor.completed != self.files.len() || outcomes.len() != self.files.len() {
            return Err(CanonError::Invalid("incomplete file checkpoints").into());
        }
        let mut claims = Vec::new();
        let mut edges = Vec::new();
        let mut file_ids = Vec::new();
        for ((path, _), outcome) in self.files.iter().zip(outcomes) {
            if &outcome.input.path != path
                || self.snapshot.get(path.as_str()).copied()
                    != Some(outcome.input.content_digest.as_str())
            {
                return Err(CanonError::Invalid("file checkpoint membership").into());
            }
            if let Some(file_claim) = outcome.claim {
                file_ids.push(CanonFileRef {
                    path: path.clone(),
                    claim_id: file_claim.id.clone(),
                });
                claims.push(file_claim);
                edges.extend(outcome.edges);
            }
        }
        let mut limits = cursor.limits.clone();
        limits.sort();
        limits.dedup();
        let observation = CanonObservation::Graph {
            graph: CanonGraph {
                schema: CANON_SCHEMA.into(),
                implementation: crate::implementation_fingerprint(),
                snapshot: self.graph.snapshot.clone(),
                files: file_ids,
                native_context: self.native_context.to_vec(),
                context: self.context.clone(),
                limitations: limits,
            },
        };
        claims.push(claim(observation, self.graph, &self.context, None)?);
        Ok(CanonProjection { claims, edges })
    }
}

/// Ordinary and restartable producers use the same file fold and final assembly.
pub fn produce(
    graph: &mut TopologyGraph,
    root: &Path,
    facts: &[CanonExpressionFact],
    native_context: &[CanonNativeContext],
    previous: Option<&dyn crate::KnowledgeRepository>,
) -> Result<CanonProduction, CanonProduceError> {
    let plan = CanonPlan::new(graph, facts, native_context, previous)?;
    let mut cursor = plan.cursor();
    let outcomes = plan.batch(root, &mut cursor, usize::MAX)?;
    let projection = plan.finish(&cursor, outcomes)?;
    projection.apply(graph);
    Ok(cursor.production)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeRepository;

    #[test]
    fn file_slices_preserve_global_limits_and_do_not_reparse_completed_files() {
        let root = tempfile::tempdir().unwrap();
        for path in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(root.path().join(path), "fn value() -> bool { true }").unwrap();
        }
        let graph =
            crate::build_topology_from_repo_with_options(root.path(), Default::default()).unwrap();
        for boundary in ["ordinary", "patterns", "bytes"] {
            let plan = CanonPlan::new(&graph, &[], &[], None).unwrap();
            let mut initial = plan.cursor();
            if boundary == "patterns" {
                initial.patterns_left = 1;
            }
            if boundary == "bytes" {
                initial.total_bytes = 64_000_000 - 1;
            }
            let mut full = initial.clone();
            let outcomes = plan.batch(root.path(), &mut full, usize::MAX).unwrap();
            let expected = plan.finish(&full, outcomes).unwrap();
            let mut cursor = initial;
            let mut resumed = Vec::new();
            for _ in 0..3 {
                resumed.extend(plan.batch(root.path(), &mut cursor, 1).unwrap());
                // Recovery uses only serialized counters, without resetting any budget.
                cursor = serde_json::from_slice(&serde_json::to_vec(&cursor).unwrap()).unwrap();
            }
            let actual = plan.finish(&cursor, resumed).unwrap();
            assert_eq!(actual.claims, expected.claims);
            assert_eq!(actual.edges, expected.edges);
            let RelationClaimValue::Canon { observation, .. } =
                &actual.claims.last().unwrap().value
            else {
                panic!("missing graph claim");
            };
            let CanonObservation::Graph { graph } = observation.as_ref() else {
                panic!("missing graph");
            };
            match boundary {
                "ordinary" => {
                    assert_eq!(graph.files.len(), 3);
                    assert!(graph.limitations.is_empty());
                }
                "patterns" => {
                    assert_eq!(graph.files.len(), 1);
                    assert_eq!(graph.limitations, [CanonLimit::BudgetExceeded]);
                }
                _ => {
                    assert!(graph.files.is_empty());
                    assert_eq!(graph.limitations, [CanonLimit::BudgetExceeded]);
                }
            }
        }
        let plan = CanonPlan::new(&graph, &[], &[], None).unwrap();
        let mut cursor = plan.cursor();
        let mut outcomes = plan.batch(root.path(), &mut cursor, 1).unwrap();
        assert!(plan.finish(&cursor, outcomes.clone()).is_err());
        std::fs::remove_file(root.path().join("a.rs")).unwrap();
        outcomes.extend(plan.batch(root.path(), &mut cursor, 2).unwrap());
        assert_eq!(plan.finish(&cursor, outcomes).unwrap().claims.len(), 4);
    }

    #[test]
    fn cache_cannot_spend_more_patterns_than_the_recorded_input_budget() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.rs"), "fn value() -> bool { true }").unwrap();
        let mut graph =
            crate::build_topology_from_repo_with_options(root.path(), Default::default()).unwrap();
        let plan = CanonPlan::new(&graph, &[], &[], None).unwrap();
        let mut cursor = plan.cursor();
        cursor.patterns_left = 1;
        let mut outcomes = plan.batch(root.path(), &mut cursor, 1).unwrap();
        let value = &outcomes[0].claim.as_ref().unwrap().value;
        let RelationClaimValue::Canon { observation, .. } = value else {
            panic!("not canon");
        };
        let CanonObservation::File { mut file } = *observation.clone() else {
            panic!("not a file");
        };
        assert_eq!(file.patterns.len(), 1);
        file.patterns.push(file.patterns[0].clone());
        // An attributable external claim can be internally valid yet unsuitable
        // for fold reuse under the budget committed by its input digest.
        outcomes[0].claim = Some(
            claim(
                CanonObservation::File { file },
                &graph,
                &plan.context,
                Some("a.rs"),
            )
            .unwrap(),
        );
        plan.finish(&cursor, outcomes).unwrap().apply(&mut graph);
        let storage = tempfile::tempdir().unwrap();
        let mut repository =
            crate::SqliteKnowledgeRepository::open(&storage.path().join("cache.db")).unwrap();
        repository
            .replace_all(&crate::KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        let plan = CanonPlan::new(&graph, &[], &[], Some(&repository)).unwrap();
        let mut cursor = plan.cursor();
        cursor.patterns_left = 1;
        plan.batch(root.path(), &mut cursor, 1).unwrap();
        assert_eq!(
            (
                cursor.production.parsed_files,
                cursor.production.reused_files
            ),
            (1, 0)
        );
        assert_eq!(cursor.patterns_left, 0);
    }

    #[test]
    fn persisted_folds_reuse_unchanged_files_and_invalidate_changed_inputs() {
        let root = tempfile::tempdir().unwrap();
        for name in ["a.rs", "b.rs"] {
            std::fs::write(root.path().join(name), "fn f() -> bool { true }").unwrap();
        }
        let inventory = || {
            crate::build_topology_from_repo_with_options(
                root.path(),
                crate::TopologyBuildOptions::default(),
            )
            .unwrap()
        };
        let mut first = inventory();
        assert_eq!(
            produce(&mut first, root.path(), &[], &[], None)
                .unwrap()
                .parsed_files,
            2
        );
        let storage = tempfile::tempdir().unwrap();
        let path = storage.path().join("knowledge.sqlite3");
        crate::SqliteKnowledgeRepository::open(&path)
            .unwrap()
            .replace_all(&crate::KnowledgeStore::extract_from_graph(&first))
            .unwrap();
        let repository = crate::SqliteKnowledgeRepository::open_read_only(&path).unwrap();
        let mut same = inventory();
        let reused = produce(&mut same, root.path(), &[], &[], Some(&repository)).unwrap();
        assert_eq!((reused.parsed_files, reused.reused_files), (0, 2));
        assert_eq!(same.relation_claims, first.relation_claims);
        std::fs::write(root.path().join("a.rs"), "fn f() -> bool { false }").unwrap();
        let mut changed = inventory();
        let reused = produce(&mut changed, root.path(), &[], &[], Some(&repository)).unwrap();
        assert_eq!((reused.parsed_files, reused.reused_files), (1, 1));
        let old = crate::QueryEngine::new(first, path)
            .clones(CloneRequest {
                min_tokens: 5,
                canonical: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(old.canon, CanonAvailability::Available);
        assert_eq!(old.groups.len(), 1);
    }
}
