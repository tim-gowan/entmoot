//! Reference-engine owner publication for conditional revision revalidation.
//!
//! Original provider evidence remains attributable to its original snapshot.
//! Only its derived applicability depends on an input root. This is a local
//! ledger, not a head graph containing silently promoted historical findings.

use crate::{
    KnowledgeDependency, KnowledgeRecordRef, KnowledgeRepository, KnowledgeRepositoryError,
    KnowledgeStore, OwnerReplacement, TopologyGraph,
};
use entmoot_contracts::{
    AnalysisInputRoot, ArtifactManifest, ContentDigest, RevalidationDeclaration,
};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RevalidationError {
    #[error("invalid revalidation declaration: {0}")]
    Invalid(String),
    #[error("budget_exceeded: {required} records exceed limit {limit}")]
    BudgetExceeded { required: usize, limit: usize },
    #[error(transparent)]
    Repository(#[from] KnowledgeRepositoryError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub use entmoot_contracts::{ArtifactInputReceipt, ArtifactRevalidationReceipt};

impl RevalidationError {
    /// Stable operation category shared by CLI and MCP renderers.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::BudgetExceeded { .. } => "budget_exceeded",
            Self::Repository(_) | Self::Json(_) | Self::Io(_) => "storage_error",
        }
    }
}

/// Publish an isolated replayable ledger only after the full transition succeeds.
/// A caller cannot overwrite an existing ledger or partially publish a failed one.
pub fn publish_revalidation_ledger(
    base: &TopologyGraph,
    head: &TopologyGraph,
    request: &entmoot_contracts::TransitionRevalidationRequest,
    ledger: &std::path::Path,
) -> Result<entmoot_contracts::TransitionRevalidationReceipt, RevalidationError> {
    if ledger.exists() {
        return Err(RevalidationError::Invalid(
            "ledger already exists; use a new path for an independently replayable transition"
                .into(),
        ));
    }
    let parent = ledger
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    // Keep the private database, WAL and lifecycle sidecar under one RAII root;
    // publishing the database must not leak a fresh orphan lock per ledger.
    let stage = tempfile::tempdir_in(parent)?;
    let temp = tempfile::NamedTempFile::new_in(stage.path())?.into_temp_path();
    let receipt = {
        let mut repository = crate::SqliteKnowledgeRepository::open(&temp)?;
        revalidate_transition(&mut repository, base, head, request)?
    };
    // Closing the sole connection checkpoints WAL. Verify serialization before
    // atomically publishing; transport rendering cannot leave a partial ledger.
    serde_json::to_vec(&receipt)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&temp)?
        .sync_all()?;
    // Publish the local lifecycle sidecar before the database becomes visible.
    // Read-only opens must never create it implicitly.
    let _lease = crate::storage_lease::shared(ledger, true)?;
    temp.persist_noclobber(ledger)
        .map_err(|error| error.error)?;
    Ok(receipt)
}

fn input_root(
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
) -> Result<(AnalysisInputRoot, Vec<String>), RevalidationError> {
    graph
        .validate()
        .map_err(|e| RevalidationError::Invalid(e.to_string()))?;
    input_root_validated(graph, manifest, declaration)
}

/// Only for an immutable graph already validated by the enclosing operation.
fn input_root_validated(
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
) -> Result<(AnalysisInputRoot, Vec<String>), RevalidationError> {
    let invalid = |error: &dyn std::fmt::Display| RevalidationError::Invalid(error.to_string());
    manifest.validate_structure().map_err(|e| invalid(&e))?;
    declaration
        .input_scope
        .validate()
        .map_err(|e| invalid(&e))?;
    declaration
        .policy_producer
        .validate()
        .map_err(|e| invalid(&e))?;
    if graph.snapshot.repository_id != manifest.snapshot.repository_id
        || declaration.method_id.trim().is_empty()
        || declaration.assumptions.is_empty()
        || declaration.assumptions.iter().any(|a| a.trim().is_empty())
        || declaration.evidence_refs.is_empty()
        || declaration
            .evidence_refs
            .iter()
            .any(|e| e.artifact_id != manifest.artifact_id || e.provider_pointer.trim().is_empty())
        || !declaration.input_scope.languages.is_empty()
        || !declaration.input_scope.producer_skips.is_empty()
        || declaration.input_scope.completeness
            != entmoot_contracts::ScopeCompleteness::CompleteRelativeToUniverse
    {
        return Err(RevalidationError::Invalid(
            "repository, method, assumptions, evidence, or path-only input universe is invalid"
                .into(),
        ));
    }
    let mut files = Vec::new();
    for file in &graph.snapshot_files {
        if declaration
            .input_scope
            .path_in_scope(&file.path)
            .map_err(|e| invalid(&e))?
        {
            files.push((file.path.clone(), file.content_digest.clone()));
        }
    }
    files.sort();
    let paths = files
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    // The complete input denominator is independently checked at publication.
    // At a later revision the same selectors intentionally capture new files.
    if graph.snapshot == manifest.snapshot
        && (declaration.input_scope.completeness
            != entmoot_contracts::ScopeCompleteness::CompleteRelativeToUniverse
            || declaration
                .input_scope
                .observed_document_set_digest
                .as_deref()
                != Some(entmoot_contracts::observed_document_set_digest(&paths).as_str()))
    {
        return Err(RevalidationError::Invalid(
            "input universe denominator is incomplete or mismatched".into(),
        ));
    }
    // Producer/config identity is fixed by this conditional policy. A tool or
    // configuration change requires a new publication, not assumed equality.
    let content = serde_json::to_vec(&(
        &declaration.policy_producer,
        &graph.policy_ids,
        &manifest.producer,
        &manifest.config_digest,
        &declaration.method_id,
        &declaration.input_scope.scope_universe_id,
        &declaration.assumptions,
        files,
    ))?;
    let root = AnalysisInputRoot::new(
        graph.snapshot.repository_id.clone(),
        manifest.artifact_series_id.clone(),
        ContentDigest::from_trusted(entmoot_contracts::blake3_hex(&content)),
        declaration.method_id.clone(),
    );
    root.validate()
        .map_err(|error| RevalidationError::Invalid(error.to_string()))?;
    Ok((root, paths))
}

/// Minimal publication port shared by ordinary ledgers and fenced transactions.
pub(crate) trait RevalidationWriter {
    fn replace(
        &mut self,
        publications: &[OwnerReplacement<'_>],
    ) -> Result<(), KnowledgeRepositoryError>;
    fn visible(&self, record: &KnowledgeRecordRef) -> Result<bool, KnowledgeRepositoryError>;
}
impl<T: KnowledgeRepository + ?Sized> RevalidationWriter for T {
    fn replace(
        &mut self,
        publications: &[OwnerReplacement<'_>],
    ) -> Result<(), KnowledgeRepositoryError> {
        self.replace_owners(publications).map(|_| ())
    }
    fn visible(&self, record: &KnowledgeRecordRef) -> Result<bool, KnowledgeRepositoryError> {
        Ok(self
            .record_state(record)?
            .is_some_and(|state| state.visible))
    }
}

struct PreparedEvidence {
    roots: KnowledgeStore,
    evidence: KnowledgeStore,
    applicability: KnowledgeStore,
    dependencies: Vec<KnowledgeDependency>,
    receipt: ArtifactInputReceipt,
    series: String,
}
impl PreparedEvidence {
    fn publish(
        &self,
        repository: &mut (impl RevalidationWriter + ?Sized),
    ) -> Result<ArtifactInputReceipt, RevalidationError> {
        repository.replace(&[
            OwnerReplacement {
                owner_id: &format!("inputs:{}", self.series),
                store: &self.roots,
                dependencies: &[],
            },
            OwnerReplacement {
                owner_id: &format!("evidence:{}", self.receipt.artifact_id),
                store: &self.evidence,
                dependencies: &[],
            },
            OwnerReplacement {
                owner_id: &format!("applicability:{}", self.series),
                store: &self.applicability,
                dependencies: &self.dependencies,
            },
        ])?;
        Ok(self.receipt.clone())
    }
    fn record_count(&self) -> usize {
        self.receipt
            .retained_evidence
            .len()
            .saturating_add(self.receipt.dependent_applicability.len())
    }
}

/// Publish immutable evidence and input-dependent applicability atomically.
pub fn publish_artifact_evidence(
    repository: &mut dyn KnowledgeRepository,
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
) -> Result<ArtifactInputReceipt, RevalidationError> {
    prepare_artifact_evidence(graph, manifest, declaration)?.publish(repository)
}

fn prepare_artifact_evidence(
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
) -> Result<PreparedEvidence, RevalidationError> {
    let (root, paths) = input_root(graph, manifest, declaration)?;
    prepare_artifact_evidence_validated(graph, manifest, declaration, root, paths)
}

fn prepare_artifact_evidence_validated(
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
    root: AnalysisInputRoot,
    paths: Vec<String>,
) -> Result<PreparedEvidence, RevalidationError> {
    if graph.snapshot != manifest.snapshot || !graph.artifacts.iter().any(|a| a == manifest) {
        return Err(RevalidationError::Invalid(
            "artifact is not attached to this exact snapshot".into(),
        ));
    }
    let mut evidence = KnowledgeStore::extract_artifact_evidence(graph, &manifest.artifact_id);
    let applicability = KnowledgeStore {
        attachments: std::mem::take(&mut evidence.attachments),
        assertions: std::mem::take(&mut evidence.assertions),
        resolutions: std::mem::take(&mut evidence.resolutions),
        relation_claims: std::mem::take(&mut evidence.relation_claims),
        unresolved_relations: std::mem::take(&mut evidence.unresolved_relations),
        resolver_coverages: std::mem::take(&mut evidence.resolver_coverages),
        resolved_identities: std::mem::take(&mut evidence.resolved_identities),
        ..Default::default()
    };
    let retained_evidence = crate::knowledge_db::record_references(&evidence)?;
    let dependent_applicability = crate::knowledge_db::record_references(&applicability)?;
    let root_ref = KnowledgeRecordRef::new("input_roots", root.id.as_str());
    let dependencies = dependent_applicability
        .iter()
        .map(|record| KnowledgeDependency::new(record.clone(), vec![root_ref.clone()]))
        .collect::<Vec<_>>();
    let roots = KnowledgeStore {
        input_roots: vec![root.clone()],
        ..Default::default()
    };
    let receipt = ArtifactInputReceipt {
        snapshot: graph.snapshot.clone(),
        artifact_id: manifest.artifact_id.clone(),
        declaration: declaration.clone(),
        input_root: root,
        input_paths: paths,
        retained_evidence,
        dependent_applicability,
    };
    Ok(PreparedEvidence {
        roots,
        evidence,
        applicability,
        dependencies,
        receipt,
        series: manifest.artifact_series_id.clone(),
    })
}

/// Advance a published artifact policy and return the exact records affected by
/// the owner/dependency frontier. The historical artifact remains immutable.
pub fn evaluate_artifact_revalidation(
    repository: &mut dyn KnowledgeRepository,
    head: &TopologyGraph,
    artifact: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
    prior: &ArtifactInputReceipt,
) -> Result<ArtifactRevalidationReceipt, RevalidationError> {
    if prior.artifact_id != artifact.artifact_id
        || prior.declaration != *declaration
        || prior.input_root.owner_key != artifact.artifact_series_id
    {
        return Err(RevalidationError::Invalid(
            "prior receipt does not describe this artifact and declaration".into(),
        ));
    }
    let (current, input_paths) = input_root(head, artifact, declaration)?;
    evaluate_prepared(
        repository,
        &head.snapshot,
        artifact,
        declaration,
        prior,
        current,
        input_paths,
    )
}

fn evaluate_prepared(
    repository: &mut (impl RevalidationWriter + ?Sized),
    head_snapshot: &entmoot_contracts::SourceSnapshot,
    artifact: &ArtifactManifest,
    declaration: &RevalidationDeclaration,
    prior: &ArtifactInputReceipt,
    current: AnalysisInputRoot,
    input_paths: Vec<String>,
) -> Result<ArtifactRevalidationReceipt, RevalidationError> {
    let roots = KnowledgeStore {
        input_roots: vec![current.clone()],
        ..Default::default()
    };
    repository.replace(&[OwnerReplacement {
        owner_id: &format!("inputs:{}", artifact.artifact_series_id),
        store: &roots,
        dependencies: &[],
    }])?;
    let mut invalidated = Vec::new();
    let mut retained = Vec::new();
    for record in &prior.dependent_applicability {
        if repository.visible(record)? {
            retained.push(record.clone());
        } else {
            invalidated.push(record.clone());
        }
    }
    let mut retained_evidence = Vec::new();
    for record in &prior.retained_evidence {
        if repository.visible(record)? {
            retained_evidence.push(record.clone());
        } else {
            return Err(RevalidationError::Invalid(
                "advancing inputs removed immutable provider evidence".into(),
            ));
        }
    }
    Ok(ArtifactRevalidationReceipt {
        snapshot: head_snapshot.clone(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_series_id: artifact.artifact_series_id.clone(),
        declaration: declaration.clone(),
        inputs_changed: prior.input_root.id != current.id,
        prior_input_root: prior.input_root.clone(),
        current_input_root: current,
        input_paths,
        invalidated_applicability: invalidated,
        retained_applicability: retained,
        retained_provider_evidence: retained_evidence,
        limitations: vec![
            "retained applicability is conditional on the declared policy and assumptions".into(),
            "only fresh artifact ingestion establishes an observation at the new snapshot".into(),
        ],
    })
}

pub(crate) struct RevalidationPlan<'a> {
    base: &'a TopologyGraph,
    head: &'a TopologyGraph,
    units: Vec<PolicyPlan<'a>>,
    unselected_artifact_ids: Vec<String>,
    context: ContentDigest,
}
impl<'a> RevalidationPlan<'a> {
    pub(crate) fn new(
        base: &'a TopologyGraph,
        head: &'a TopologyGraph,
        request: &'a entmoot_contracts::TransitionRevalidationRequest,
    ) -> Result<Self, RevalidationError> {
        use entmoot_contracts::AnalysisDemand;
        let invalid = |message: &str| RevalidationError::Invalid(message.into());
        base.validate().map_err(|e| invalid(&e.to_string()))?;
        head.validate().map_err(|e| invalid(&e.to_string()))?;
        if request.base_snapshot != base.snapshot
            || request.head_snapshot != head.snapshot
            || base.snapshot.repository_id != head.snapshot.repository_id
        {
            return Err(invalid(
                "request snapshots must match the exact same-repository publications",
            ));
        }
        if request.max_records == 0 || request.max_records > 1_000_000 {
            return Err(invalid("max_records must be in 1..=1000000"));
        }
        let mut selected = BTreeSet::new();
        let mut prepared = Vec::new();
        // Every policy, artifact, snapshot, subject and scheduling gate is validated
        // before the first write. Native evidence is never selected by a model.
        for unit in &request.units {
            let artifact = base
                .artifacts
                .iter()
                .find(|a| a.artifact_id == unit.base_artifact_id.as_str())
                .ok_or_else(|| invalid("base artifact is not attached"))?;
            if !selected.insert(artifact.artifact_series_id.clone()) {
                return Err(invalid(
                    "multiple policies for one producer series are ambiguous",
                ));
            }
            let (prior, prior_paths) = input_root_validated(base, artifact, &unit.declaration)?;
            let (current, current_paths) = input_root_validated(head, artifact, &unit.declaration)?;
            let fresh = unit
                .head_artifact_id
                .as_ref()
                .map(|id| {
                    let manifest = head
                        .artifacts
                        .iter()
                        .find(|a| a.artifact_id == id.as_str())
                        .ok_or_else(|| invalid("head artifact is not attached"))?;
                    if manifest.artifact_series_id != artifact.artifact_series_id {
                        return Err(invalid(
                            "head artifact belongs to a different producer series",
                        ));
                    }
                    let mut declaration = unit.declaration.clone();
                    let paths = head
                        .snapshot_files
                        .iter()
                        .filter_map(|f| {
                            declaration
                                .input_scope
                                .path_in_scope(&f.path)
                                .ok()
                                .filter(|yes| *yes)
                                .map(|_| f.path.clone())
                        })
                        .collect::<Vec<_>>();
                    declaration.input_scope.observed_document_set_digest =
                        Some(entmoot_contracts::observed_document_set_digest(&paths));
                    for evidence in &mut declaration.evidence_refs {
                        evidence.artifact_id = manifest.artifact_id.clone();
                    }
                    let (root, paths) = input_root_validated(head, manifest, &declaration)?;
                    Ok::<_, RevalidationError>((manifest, declaration, root, paths))
                })
                .transpose()?;
            let has_fresh = fresh.is_some();
            let decision_root = fresh
                .as_ref()
                .map(|(_, _, r, _)| &r.id)
                .unwrap_or(&current.id);
            let decision = crate::plan_analysis(
                AnalysisDemand {
                    policy_producer: unit.declaration.policy_producer.clone(),
                    snapshot: head.snapshot.clone(),
                    subject: unit.subject.clone(),
                    method_id: unit.declaration.method_id.clone(),
                    input_root: decision_root.clone(),
                    // For already produced head evidence the most recent production
                    // used the current inputs; the transition roots remain in the receipt.
                    prior_input_root: Some(if has_fresh {
                        decision_root.clone()
                    } else {
                        prior.id.clone()
                    }),
                    current_artifact: if has_fresh {
                        unit.head_artifact_id.clone()
                    } else if prior.id == current.id {
                        Some(unit.base_artifact_id.clone())
                    } else {
                        None
                    },
                    gates: unit.gates.clone(),
                    relevant_unknown: false,
                    assumptions: unit.declaration.assumptions.clone(),
                    evidence_refs: fresh
                        .as_ref()
                        .map(|(_, d, _, _)| d.evidence_refs.clone())
                        .unwrap_or_else(|| unit.declaration.evidence_refs.clone()),
                },
                unit.mode,
            )
            .map_err(|e| invalid(&e.to_string()))?;
            prepared.push(PolicyPlan {
                unit,
                artifact,
                fresh,
                decision,
                prior,
                prior_paths,
                current,
                current_paths,
            });
        }
        prepared.sort_by(|a, b| {
            a.artifact
                .artifact_series_id
                .cmp(&b.artifact.artifact_series_id)
        });
        let selected_artifacts: BTreeSet<_> = request
            .units
            .iter()
            .map(|u| u.base_artifact_id.as_str())
            .collect();
        let mut unselected_artifact_ids = base
            .artifacts
            .iter()
            .filter(|a| !selected_artifacts.contains(a.artifact_id.as_str()))
            .map(|a| a.artifact_id.clone())
            .collect::<Vec<_>>();
        unselected_artifact_ids.sort();
        Ok(Self {
            base,
            head,
            units: prepared,
            unselected_artifact_ids,
            context: transition_context(request)?,
        })
    }
    pub(crate) fn len(&self) -> usize {
        self.units.len()
    }
    pub(crate) fn prepare(
        &self,
        index: usize,
    ) -> Result<PreparedRevalidation<'_>, RevalidationError> {
        let policy = self.units.get(index).ok_or_else(|| {
            RevalidationError::Invalid("revalidation ordinal out of range".into())
        })?;
        let publication = prepare_artifact_evidence_validated(
            self.base,
            policy.artifact,
            &policy.unit.declaration,
            policy.prior.clone(),
            policy.prior_paths.clone(),
        )?;
        let fresh = policy
            .fresh
            .as_ref()
            .map(|(artifact, declaration, root, paths)| {
                prepare_artifact_evidence_validated(
                    self.head,
                    artifact,
                    declaration,
                    root.clone(),
                    paths.clone(),
                )
            })
            .transpose()?;
        Ok(PreparedRevalidation {
            ordinal: index,
            context: self.context.clone(),
            policy,
            head_snapshot: &self.head.snapshot,
            publication,
            fresh,
            paths: policy.current_paths.clone(),
        })
    }
    pub(crate) fn receipt(
        &self,
        units: Vec<entmoot_contracts::RevalidationUnitReceipt>,
    ) -> entmoot_contracts::TransitionRevalidationReceipt {
        entmoot_contracts::TransitionRevalidationReceipt {
            schema_version: "entmoot.transition_revalidation.v1".into(),
            base_snapshot: self.base.snapshot.clone(),
            head_snapshot: self.head.snapshot.clone(),
            units,
            unselected_artifact_ids: self.unselected_artifact_ids.clone(),
            limitations: vec![
                "conditional on supplied complete input policies; unselected producers remain unknown".into(),
                "historical evidence retains its original snapshot; only fresh attached artifacts establish head observations".into(),
                "scheduling decisions do not execute producers or establish graph truth".into(),
            ],
        }
    }
}

struct PolicyPlan<'a> {
    unit: &'a entmoot_contracts::RevalidationUnit,
    artifact: &'a ArtifactManifest,
    fresh: Option<(
        &'a ArtifactManifest,
        RevalidationDeclaration,
        AnalysisInputRoot,
        Vec<String>,
    )>,
    decision: entmoot_contracts::AnalysisDecision,
    prior: AnalysisInputRoot,
    prior_paths: Vec<String>,
    current: AnalysisInputRoot,
    current_paths: Vec<String>,
}
pub(crate) struct PreparedRevalidation<'a> {
    pub(crate) ordinal: usize,
    pub(crate) context: ContentDigest,
    policy: &'a PolicyPlan<'a>,
    head_snapshot: &'a entmoot_contracts::SourceSnapshot,
    publication: PreparedEvidence,
    fresh: Option<PreparedEvidence>,
    paths: Vec<String>,
}

pub(crate) fn transition_context(
    request: &entmoot_contracts::TransitionRevalidationRequest,
) -> Result<ContentDigest, RevalidationError> {
    Ok(ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
        &serde_json::to_vec(request)?,
    )))
}
impl PreparedRevalidation<'_> {
    pub(crate) fn record_count(&self) -> usize {
        self.publication.record_count().saturating_add(
            self.fresh
                .as_ref()
                .map_or(0, PreparedEvidence::record_count),
        )
    }
    pub(crate) fn apply(
        &self,
        repository: &mut (impl RevalidationWriter + ?Sized),
    ) -> Result<entmoot_contracts::RevalidationUnitReceipt, RevalidationError> {
        let publication = self.publication.publish(repository)?;
        let revalidation = evaluate_prepared(
            repository,
            self.head_snapshot,
            self.policy.artifact,
            &self.policy.unit.declaration,
            &publication,
            self.policy.current.clone(),
            self.paths.clone(),
        )?;
        let reingested = self
            .fresh
            .as_ref()
            .map(|fresh| fresh.publish(repository))
            .transpose()?;
        let mut decision = self.policy.decision.clone();
        decision
            .limitations
            .extend(revalidation.limitations.clone());
        decision.limitations.sort();
        decision.limitations.dedup();
        Ok(entmoot_contracts::RevalidationUnitReceipt {
            publication,
            revalidation,
            reingested,
            decision,
        })
    }
}

/// Validate and apply a transition to an empty, isolated ledger. The composition
/// root publishes only on success and discards the ledger on error. This never
/// invokes a producer or changes graphs.
pub fn revalidate_transition(
    repository: &mut dyn KnowledgeRepository,
    base: &TopologyGraph,
    head: &TopologyGraph,
    request: &entmoot_contracts::TransitionRevalidationRequest,
) -> Result<entmoot_contracts::TransitionRevalidationReceipt, RevalidationError> {
    let plan = RevalidationPlan::new(base, head, request)?;
    if repository.is_initialized()? {
        return Err(RevalidationError::Invalid(
            "transition requires a new isolated ledger".into(),
        ));
    }
    repository.replace_all(&KnowledgeStore::default())?;
    let mut count = 0usize;
    let mut units = Vec::new();
    for index in 0..plan.len() {
        let prepared = plan.prepare(index)?;
        count = count.saturating_add(prepared.record_count());
        if count > request.max_records {
            return Err(RevalidationError::BudgetExceeded {
                required: count,
                limit: request.max_records,
            });
        }
        units.push(prepared.apply(repository)?);
    }
    Ok(plan.receipt(units))
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{
        ArtifactFormat, ArtifactProducer, EvidenceRef, ScopeCompleteness, ScopeUniverse,
    };

    #[test]
    fn new_files_in_declared_frontier_change_root_but_unrelated_files_do_not() {
        let mut base = TopologyGraph::new(std::env::current_dir().unwrap().to_string_lossy());
        let add_node = |graph: &mut TopologyGraph, path: &str| {
            let mut node = crate::TopologyNode::shell(
                crate::graph::file_node_id(path),
                crate::NodeKind::File,
                path,
                None,
            );
            node.path = Some(path.into());
            graph.upsert_node(node);
        };
        add_node(&mut base, "src/a.ts");
        base.snapshot_files.push(crate::SnapshotFileDigest {
            path: "src/a.ts".into(),
            content_digest: entmoot_contracts::blake3_hex(b"a"),
        });
        let paths = vec!["src/a.ts".into()];
        let scope = ScopeUniverse::from_selectors(
            "inputs",
            vec![],
            vec!["src/**".into()],
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&paths),
        );
        let artifact = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "ts".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            base.snapshot.clone(),
            scope.clone(),
            entmoot_contracts::blake3_hex(b"artifact"),
            None,
        )
        .unwrap();
        let policy = RevalidationDeclaration {
            policy_producer: entmoot_contracts::ProducerRef {
                id: "fixture.policy".into(),
                name: "Fixture input declaration".into(),
                version: Some("1".into()),
                config_fingerprint: None,
                environment_fingerprint: None,
                capabilities: vec![entmoot_contracts::ProducerCapabilityKind::Deriver],
            },
            method_id: "fixture.inputs.v1".into(),
            input_scope: scope,
            assumptions: vec!["same tool and build context".into()],
            evidence_refs: vec![EvidenceRef {
                artifact_id: artifact.artifact_id.clone(),
                provider_pointer: "/".into(),
                record_id: None,
            }],
        };
        let first = input_root(&base, &artifact, &policy).unwrap().0;
        let mut alternate = policy.clone();
        alternate.policy_producer.id = "another.policy".into();
        assert_ne!(first, input_root(&base, &artifact, &alternate).unwrap().0);
        let mut head = base.clone();
        head.snapshot.revision = "b".repeat(40);
        add_node(&mut head, "docs/readme.md");
        head.snapshot_files.push(crate::SnapshotFileDigest {
            path: "docs/readme.md".into(),
            content_digest: entmoot_contracts::blake3_hex(b"doc"),
        });
        assert_eq!(first, input_root(&head, &artifact, &policy).unwrap().0);
        let mut partial = policy.clone();
        partial.input_scope.completeness = ScopeCompleteness::Partial;
        assert!(input_root(&head, &artifact, &partial).is_err());
        add_node(&mut head, "src/new.ts");
        head.snapshot_files.push(crate::SnapshotFileDigest {
            path: "src/new.ts".into(),
            content_digest: entmoot_contracts::blake3_hex(b"new"),
        });
        assert_ne!(first, input_root(&head, &artifact, &policy).unwrap().0);
        base.artifacts.push(artifact.clone());
        let mut repository = crate::SqliteKnowledgeRepository::open_in_memory().unwrap();
        let published =
            publish_artifact_evidence(&mut repository, &base, &artifact, &policy).unwrap();
        let impact =
            evaluate_artifact_revalidation(&mut repository, &head, &artifact, &policy, &published)
                .unwrap();
        assert!(impact.inputs_changed);
        assert_eq!(impact.prior_input_root, published.input_root);
        assert_eq!(
            impact.retained_provider_evidence,
            published.retained_evidence
        );
        assert!(!impact.limitations.is_empty());
        let mut bad = policy;
        bad.assumptions.clear();
        assert!(input_root(&base, &artifact, &bad).is_err());
    }
    #[test]
    fn qualified_go_frontier_detects_new_modules_embeds_and_policy_drift() {
        // Authored frontier selectors: modules, embedded resources and assembly.
        let includes = vec![
            "**/*.go".into(),
            "**/go.mod".into(),
            "**/go.sum".into(),
            "**/go.work".into(),
            "**/go.work.sum".into(),
            "src/checkout/**".into(),
            "src/product-catalog/**".into(),
        ];
        let add = |graph: &mut TopologyGraph, path: &str| {
            let mut node = crate::TopologyNode::shell(
                crate::graph::file_node_id(path),
                crate::NodeKind::File,
                path,
                None,
            );
            node.path = Some(path.into());
            graph.upsert_node(node);
            graph.snapshot_files.push(crate::SnapshotFileDigest {
                path: path.into(),
                content_digest: entmoot_contracts::blake3_hex(path.as_bytes()),
            });
            graph.snapshot_files.sort_by(|a, b| a.path.cmp(&b.path));
        };
        let mut base = TopologyGraph::new(std::env::current_dir().unwrap().to_string_lossy());
        add(&mut base, "src/checkout/main.go");
        let scope = ScopeUniverse::from_selectors(
            "go-frontier",
            vec![],
            includes,
            vec![],
            ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&["src/checkout/main.go".into()]),
        );
        let artifact = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: "native-go".into(),
                version: Some("fixture".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            base.snapshot.clone(),
            scope.clone(),
            entmoot_contracts::blake3_hex(b"go"),
            None,
        )
        .unwrap();
        let declaration = RevalidationDeclaration {
            policy_producer: entmoot_contracts::ProducerRef {
                id: "fixture.go.policy".into(),
                name: "Go policy fixture".into(),
                version: Some("1".into()),
                config_fingerprint: None,
                environment_fingerprint: None,
                capabilities: vec![entmoot_contracts::ProducerCapabilityKind::Deriver],
            },
            method_id: "entmoot.fixture.go.module_frontier.v1".into(),
            input_scope: scope,
            assumptions: vec!["fixed external compiler and dependency inputs".into()],
            evidence_refs: vec![EvidenceRef {
                artifact_id: artifact.artifact_id.clone(),
                provider_pointer: "/".into(),
                record_id: None,
            }],
        };
        let root = input_root(&base, &artifact, &declaration).unwrap().0;
        for path in ["src/payment/logger.js", "compose.yaml"] {
            let mut head = base.clone();
            head.snapshot.revision = "b".repeat(40);
            add(&mut head, path);
            assert_eq!(
                root,
                input_root(&head, &artifact, &declaration).unwrap().0,
                "{path}"
            );
        }
        for path in [
            "new/service/main.go",
            "new/service/go.mod",
            "new/service/go.sum",
            "go.work",
            "go.work.sum",
            "src/checkout/embedded/new.json",
            "src/product-catalog/new.s",
        ] {
            let mut head = base.clone();
            head.snapshot.revision = "b".repeat(40);
            add(&mut head, path);
            assert_ne!(
                root,
                input_root(&head, &artifact, &declaration).unwrap().0,
                "{path}"
            );
        }
        let mut head = base;
        head.snapshot.revision = "b".repeat(40);
        head.policy_ids.push("b".repeat(64));
        head.policy_ids.sort();
        assert_ne!(root, input_root(&head, &artifact, &declaration).unwrap().0);
    }
}
