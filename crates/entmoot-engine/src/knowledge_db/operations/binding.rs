//! Native graph prefix, report and cursor form one fenced publication.
use super::*;
use crate::binding_operations::Progress;
use entmoot_contracts::{BindingOutcome, ProducerRequest, ProducerTask};

pub(super) fn require_complete(
    item: &OperationRecord,
    request: &ProducerRequest,
) -> Result<(), OperationError> {
    if let ProducerTask::Binding { view, plan } = &request.task {
        let progress = Progress::read(item, view, plan.families.len())?;
        if progress.completed_families != plan.families.len()
            || item.checkpoint["view"].as_str() != Some(progress.view.as_str())
        {
            return Err(OperationError::Conflict(
                "binding family prefix is incomplete".into(),
            ));
        }
    }
    Ok(())
}

impl SqliteKnowledgeRepository {
    /// The owning worker may hydrate its sealed prefix before public discovery.
    pub(crate) fn binding_prefix(
        &self,
        id: &str,
        view: &str,
    ) -> Result<crate::TopologyGraph, OperationError> {
        if !self.connection.query_row("SELECT EXISTS(SELECT 1 FROM local_operation_pins p JOIN local_view_seals s ON s.view_id=p.view_id WHERE p.operation_id=?1 AND p.view_id=?2)",params![id,view],|r|r.get::<_,bool>(0))? {
            return Err(OperationError::Conflict("binding prefix is not sealed and pinned by this operation".into()));
        }
        let path: String = self.connection.query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )?;
        let repository = Self::open_view_read_only(Path::new(&path), view)?;
        repository.connection.execute_batch("BEGIN DEFERRED")?;
        repository
            .load_inventory(false)?
            .ok_or_else(|| OperationError::Unavailable("binding prefix no longer retained".into()))
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn checkpoint_binding(
        &mut self,
        id: &str,
        generation: u64,
        now: u64,
        sequence: u64,
        graph: &crate::TopologyGraph,
        source: Option<&dyn crate::SourceStore>,
        outcome: &BindingOutcome,
    ) -> Result<OperationRecord, OperationError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = live_producer(&tx, id, generation, now)?;
        let request: ProducerRequest = serde_json::from_value(item.spec.input.clone())?;
        let ProducerTask::Binding { view, plan } = &request.task else {
            return Err(OperationError::Invalid("not a binding operation".into()));
        };
        let mut progress = Progress::read(&item, view, plan.families.len())?;
        if item.spec.kind != entmoot_contracts::OperationKind::Binding
            || item.checkpoint_sequence != sequence
            || plan.families.get(progress.completed_families) != Some(&outcome.family)
            || outcome.environment != plan.environment
            || crate::producer_operations::graph_source(graph)? != request.source
        {
            return Err(OperationError::Conflict(
                "binding checkpoint differs from frozen request or live cursor".into(),
            ));
        }
        if item.checkpoint_sequence >= 1000 {
            return Err(OperationError::Capacity);
        }
        outcome
            .receipt
            .validate(&crate::implementation_fingerprint())
            .map_err(|e| OperationError::Invalid(e.into()))?;
        let expected = matches!(
            outcome.receipt.semantic_status,
            entmoot_contracts::NativeSemanticStatus::Ok
                | entmoot_contracts::NativeSemanticStatus::Partial
        );
        let artifact = graph
            .artifacts
            .iter()
            .find(|a| {
                a.artifact_id == outcome.attachment.artifact_id
                    && a.content_digest == outcome.attachment.artifact_digest
                    && a.snapshot == request.source.snapshot
            })
            .ok_or_else(|| {
                OperationError::Invalid(
                    "binding receipt does not describe attached evidence".into(),
                )
            })?;
        crate::native_resolver::pipeline::validate_binding_evidence(
            outcome.family,
            &outcome.receipt,
            artifact,
        )
        .map_err(|e| OperationError::Invalid(e.into()))?;
        if outcome.completed != expected {
            return Err(OperationError::Invalid(
                "binding receipt does not describe attached evidence".into(),
            ));
        }
        let selected = stage_graph(&tx, id, graph, source)?;
        progress.view =
            ContentDigest::parse(&selected).map_err(|e| OperationError::Invalid(e.to_string()))?;
        if outcome.completed {
            progress.completed_families += 1;
        }
        item.checkpoint_sequence = next(item.checkpoint_sequence)?;
        item.checkpoint = serde_json::json!({"view":selected,"binding":progress});
        tx.execute(
            "INSERT INTO local_operation_outputs VALUES(?1,?2,?3)",
            params![
                id,
                i64::try_from(item.checkpoint_sequence)
                    .map_err(|e| OperationError::Invalid(e.to_string()))?,
                encode(outcome)?
            ],
        )?;
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }
}
