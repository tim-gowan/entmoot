//! Private graph-frontier checkpoints share the parsed-file lifetime and fence.
use super::*;
use crate::inventory_stitch::{Frontier, Part, Phase};
use entmoot_core::scan::ScanOutcome;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE local_inventory_parts (
    operation_id TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('syntax','calls')),
    ordinal INTEGER NOT NULL, payload INTEGER NOT NULL REFERENCES shared_payloads,
    PRIMARY KEY(operation_id,phase,ordinal),
    FOREIGN KEY(operation_id,ordinal) REFERENCES local_file_outcomes(operation_id,ordinal) ON DELETE CASCADE
) WITHOUT ROWID;
"#;

fn invalid(error: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(error.to_string())
}

// Counts are stored with the file progress, so advancing a frontier does not
// count or rewrite all previous parts on every step.
fn counts(
    connection: &Connection,
    id: &str,
) -> Result<(usize, usize, usize, usize), OperationError> {
    let (parsed,total,syntax,calls): (i64,i64,i64,i64) = connection.query_row(
        "SELECT completed,total,syntax_completed,calls_completed FROM local_file_progress WHERE operation_id=?1", [id],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    Ok((
        usize::try_from(parsed).map_err(invalid)?,
        usize::try_from(total).map_err(invalid)?,
        usize::try_from(syntax).map_err(invalid)?,
        usize::try_from(calls).map_err(invalid)?,
    ))
}

pub(super) fn require_complete_if_present(
    tx: &Transaction<'_>,
    item: &OperationRecord,
) -> Result<(), OperationError> {
    if item.spec.kind == entmoot_contracts::OperationKind::Inventory && tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_file_progress WHERE operation_id=?1 AND (syntax_completed<>total OR calls_completed<>total))", [item.id.as_str()], |r|r.get::<_,bool>(0))? {
        return Err(OperationError::Conflict("producer has incomplete graph frontiers".into()));
    }
    Ok(())
}

impl SqliteKnowledgeRepository {
    pub fn inventory_frontier(
        &self,
        id: &str,
        limit: usize,
    ) -> Result<Option<Frontier>, OperationError> {
        if !(1..=4096).contains(&limit) {
            return Err(invalid("inventory frontier requires 1..4096 files"));
        }
        let tx = self.connection.unchecked_transaction()?;
        let (parsed, total, syntax, calls) = counts(&tx, id)?;
        if parsed != total {
            return Err(invalid("inventory parsing is incomplete"));
        }
        let (phase, start) = if syntax < total {
            (Phase::Syntax, syntax)
        } else if calls < total {
            (Phase::Calls, calls)
        } else {
            return Ok(None);
        };
        let mut statement = tx.prepare("SELECT path,digest,payload FROM local_file_outcomes WHERE operation_id=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3")?;
        let rows = statement.query_map(
            params![
                id,
                i64::try_from(start).map_err(invalid)?,
                i64::try_from(limit).map_err(invalid)?
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?;
        let mut outcomes = Vec::new();
        for row in rows {
            let (path, digest, payload) = row?;
            let outcome: ScanOutcome = hydrate_record(&tx, payload)?;
            if outcome.input.path != path || outcome.input.content_digest != digest {
                return Err(invalid("inventory frontier membership mismatch"));
            }
            outcomes.push(outcome);
        }
        if outcomes.len() != limit.min(total - start) {
            return Err(invalid("inventory frontier is missing files"));
        }
        Ok(Some(Frontier {
            phase,
            start,
            outcomes,
        }))
    }

    pub fn checkpoint_inventory_frontier(
        &mut self,
        id: &str,
        generation: u64,
        now_ms: u64,
        frontier: &Frontier,
        parts: &[Part],
        reused: usize,
    ) -> Result<OperationRecord, OperationError> {
        if parts.is_empty()
            || parts.len() > 4096
            || parts.len() != frontier.outcomes.len()
            || reused > parts.len()
        {
            return Err(invalid("inventory part count differs from frontier"));
        }
        let encoded = parts
            .iter()
            .map(serde_json::to_vec)
            .collect::<Result<Vec<_>, _>>()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut item = live_producer(&tx, id, generation, now_ms)?;
        let (parsed, total, mut syntax, mut calls) = counts(&tx, id)?;
        let expected = match frontier.phase {
            Phase::Syntax => syntax,
            Phase::Calls => calls,
        };
        if item.spec.kind != entmoot_contracts::OperationKind::Inventory
            || parsed != total
            || expected != frontier.start
            || (frontier.phase == Phase::Calls && syntax != total)
            || parts.len() > total.saturating_sub(expected)
        {
            return Err(OperationError::Conflict(
                "inventory frontier changed or phase is incomplete".into(),
            ));
        }
        for (offset, ((outcome, part), bytes)) in frontier
            .outcomes
            .iter()
            .zip(parts)
            .zip(&encoded)
            .enumerate()
        {
            let ordinal = expected + offset;
            let (path, digest): (String, String) = tx.query_row(
                "SELECT path,digest FROM local_file_outcomes WHERE operation_id=?1 AND ordinal=?2",
                params![id, i64::try_from(ordinal).map_err(invalid)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if part.phase() != frontier.phase
                || outcome.input.path != path
                || outcome.input.content_digest != digest
            {
                return Err(invalid("inventory part does not match frontier input"));
            }
            let payload = intern_payload(&tx, bytes)?;
            tx.execute(
                "INSERT INTO local_inventory_parts VALUES(?1,?2,?3,?4)",
                params![
                    id,
                    frontier.phase.key(),
                    i64::try_from(ordinal).map_err(invalid)?,
                    payload
                ],
            )?;
        }
        match frontier.phase {
            Phase::Syntax => syntax += parts.len(),
            Phase::Calls => calls += parts.len(),
        }
        tx.execute("UPDATE local_file_progress SET syntax_completed=?2,calls_completed=?3 WHERE operation_id=?1",params![id,i64::try_from(syntax).map_err(invalid)?,i64::try_from(calls).map_err(invalid)?])?;
        item.checkpoint_sequence = next(item.checkpoint_sequence)?;
        let detail = &mut item.checkpoint["inventory"];
        detail["syntax_files"] = syntax.into();
        detail["stitched_files"] = calls.into();
        let key = match frontier.phase {
            Phase::Syntax => "reused_extraction_files",
            Phase::Calls => "reused_stitching_files",
        };
        detail[key] = detail[key]
            .as_u64()
            .unwrap_or(0)
            .checked_add(reused as u64)
            .ok_or_else(|| invalid("reuse counter overflow"))?
            .into();
        save(&tx, &item)?;
        tx.commit()?;
        Ok(item)
    }

    pub fn inventory_syntax_graph(
        &self,
        id: &str,
        root: &Path,
        snapshot: &entmoot_contracts::SourceSnapshot,
    ) -> Result<crate::TopologyGraph, OperationError> {
        let mut extractor = crate::extractor::MarkerTopologyExtractor::with_snapshot(
            root.to_string_lossy(),
            snapshot.clone(),
        );
        extractor.on_repo_begin(&root.to_string_lossy());
        let (mut graph, _) = extractor.finish_syntax();
        self.for_each_inventory_part(id, Phase::Syntax, |part| {
            let Part::Syntax(part) = part else {
                return Err(invalid("unexpected inventory part kind"));
            };
            part.apply(&mut graph);
            Ok(())
        })?;
        Ok(graph)
    }

    pub fn apply_inventory_calls(
        &self,
        id: &str,
        graph: &mut crate::TopologyGraph,
    ) -> Result<(), OperationError> {
        self.for_each_inventory_part(id, Phase::Calls, |part| {
            let Part::Calls(part) = part else {
                return Err(invalid("unexpected inventory part kind"));
            };
            part.apply(graph);
            Ok(())
        })
    }

    pub fn completed_inventory_cache(
        &self,
        id: &str,
        repository: &str,
        policy: &entmoot_contracts::StructuralHashPolicy,
    ) -> Result<syntax_cache::SyntaxCache, OperationError> {
        let tx = self.connection.unchecked_transaction()?;
        let (parsed, total, syntax, calls) = counts(&tx, id)?;
        if parsed != total || syntax != total || calls != total {
            return Err(invalid("inventory cache requires complete frontiers"));
        }
        let mut cache = syntax_cache::SyntaxCache::new(syntax_cache::SyntaxCache::context(
            repository, policy,
        )?)?;
        let mut statement = tx.prepare("SELECT f.path,f.digest,f.payload,s.payload,c.payload FROM local_file_outcomes f JOIN local_inventory_parts s ON s.operation_id=f.operation_id AND s.ordinal=f.ordinal AND s.phase='syntax' JOIN local_inventory_parts c ON c.operation_id=f.operation_id AND c.ordinal=f.ordinal AND c.phase='calls' WHERE f.operation_id=?1")?;
        let mut rows = statement.query([id])?;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            count += 1;
            let path: String = row.get(0)?;
            let digest: String = row.get(1)?;
            let outcome: ScanOutcome = hydrate_record(&tx, row.get(2)?)?;
            if path != outcome.input.path || digest != outcome.input.content_digest {
                return Err(invalid("cache source membership mismatch"));
            }
            let Some(record) = outcome.record else {
                continue;
            };
            let Part::Syntax(syntax) = hydrate_record(&tx, row.get(3)?)? else {
                return Err(invalid("unexpected syntax cache part"));
            };
            let Part::Calls(calls) = hydrate_record(&tx, row.get(4)?)? else {
                return Err(invalid("unexpected call cache part"));
            };
            cache.insert_record(path.clone(), digest, &record)?;
            cache.insert_parts(&path, &syntax_cache::FileParts { syntax, calls })?;
        }
        if count != total {
            return Err(invalid("inventory cache is missing files"));
        }
        Ok(cache)
    }

    fn for_each_inventory_part(
        &self,
        id: &str,
        phase: Phase,
        mut visit: impl FnMut(Part) -> Result<(), OperationError>,
    ) -> Result<(), OperationError> {
        let tx = self.connection.unchecked_transaction()?;
        let (parsed, total, syntax, calls) = counts(&tx, id)?;
        if parsed != total || syntax != total || (phase == Phase::Calls && calls != total) {
            return Err(invalid("inventory phase is incomplete"));
        }
        // Match ordinary resolution's path/occurrence ordering, independent of
        // platform manifest order and how frontiers were sliced.
        let mut statement = tx.prepare("SELECT p.payload FROM local_inventory_parts p JOIN local_file_outcomes f ON f.operation_id=p.operation_id AND f.ordinal=p.ordinal WHERE p.operation_id=?1 AND p.phase=?2 ORDER BY f.path")?;
        let rows = statement.query_map(params![id, phase.key()], |r| r.get::<_, i64>(0))?;
        let mut count = 0;
        for row in rows {
            visit(hydrate_record(&tx, row?)?)?;
            count += 1;
        }
        if count != total {
            return Err(invalid("inventory parts are missing files"));
        }
        Ok(())
    }
}
