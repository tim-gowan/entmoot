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
    pub(crate) fn inventory_frontier(
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

    pub(crate) fn checkpoint_inventory_frontier(
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

    pub(crate) fn inventory_syntax_graph(
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
        for part in self.inventory_parts(id, Phase::Syntax)? {
            let Part::Syntax(part) = part else {
                return Err(invalid("unexpected inventory part kind"));
            };
            part.apply(&mut graph);
        }
        Ok(graph)
    }

    pub(crate) fn apply_inventory_calls(
        &self,
        id: &str,
        graph: &mut crate::TopologyGraph,
    ) -> Result<(), OperationError> {
        for part in self.inventory_parts(id, Phase::Calls)? {
            let Part::Calls(part) = part else {
                return Err(invalid("unexpected inventory part kind"));
            };
            part.apply(graph);
        }
        Ok(())
    }

    pub(crate) fn completed_inventory_cache(
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
        let mut cache = syntax_cache::SyntaxCache {
            context: syntax_cache::SyntaxCache::context(repository, policy)?,
            ..Default::default()
        };
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
            cache
                .parts
                .insert(path.clone(), syntax_cache::FileParts { syntax, calls });
            cache.records.insert(path, (digest, record));
        }
        if count != total {
            return Err(invalid("inventory cache is missing files"));
        }
        Ok(cache)
    }

    fn inventory_parts(&self, id: &str, phase: Phase) -> Result<Vec<Part>, OperationError> {
        let tx = self.connection.unchecked_transaction()?;
        let (parsed, total, syntax, calls) = counts(&tx, id)?;
        if parsed != total || syntax != total || (phase == Phase::Calls && calls != total) {
            return Err(invalid("inventory phase is incomplete"));
        }
        // Match ordinary resolution's path/occurrence ordering, independent of
        // platform manifest order and how frontiers were sliced.
        let mut statement = tx.prepare("SELECT p.payload FROM local_inventory_parts p JOIN local_file_outcomes f ON f.operation_id=p.operation_id AND f.ordinal=p.ordinal WHERE p.operation_id=?1 AND p.phase=?2 ORDER BY f.path")?;
        let rows = statement.query_map(params![id, phase.key()], |r| r.get::<_, i64>(0))?;
        let mut parts = Vec::new();
        for row in rows {
            parts.push(hydrate_record(&tx, row?)?);
        }
        if parts.len() != total {
            return Err(invalid("inventory parts are missing files"));
        }
        Ok(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory_stitch::{CallPart, SyntaxPart};
    use entmoot_contracts::{ProducerPublication, ProducerRequest, ProducerTask};

    fn setup(root: &Path, store: &Path) -> (SqliteKnowledgeRepository, OperationRecord) {
        std::fs::write(
            root.join("a.py"),
            "from .b import leaf\ndef caller():\n    return leaf()\n",
        )
        .unwrap();
        std::fs::write(root.join("b.py"), "def leaf():\n    return 1\n").unwrap();
        let mut db = SqliteKnowledgeRepository::open(store).unwrap();
        let request = ProducerRequest {
            source: crate::producer_operations::observe_source(root, "frontier/recovery").unwrap(),
            publication: ProducerPublication {
                workspace_id: "topic".into(),
                publisher: "agent".into(),
                branch: None,
                expected_generation: 0,
            },
            task: ProducerTask::Inventory {
                policy: Default::default(),
            },
        };
        let job = crate::producer_operations::submit(&mut db, request, "index", "agent").unwrap();
        let job = db.claim_operation(job.id.as_str(), "child", 0, 1).unwrap();
        let manifest = entmoot_core::scan::source_manifest(root).unwrap();
        db.begin_inventory(job.id.as_str(), job.generation, 0, &manifest)
            .unwrap();
        let outcomes = entmoot_core::scan::ScanWorkers::new()
            .unwrap()
            .scan(
                root,
                "frontier/recovery",
                &Default::default(),
                &manifest.inputs,
            )
            .unwrap();
        db.checkpoint_inventory_batch(job.id.as_str(), job.generation, 0, &outcomes, 0)
            .unwrap();
        (db, job)
    }

    fn syntax(
        db: &mut SqliteKnowledgeRepository,
        job: &OperationRecord,
        root: &Path,
        count: usize,
    ) {
        let frontier = db
            .inventory_frontier(job.id.as_str(), count)
            .unwrap()
            .unwrap();
        let parts = frontier
            .outcomes
            .iter()
            .map(|o| Part::Syntax(SyntaxPart::build(root, o).unwrap()))
            .collect::<Vec<_>>();
        db.checkpoint_inventory_frontier(job.id.as_str(), job.generation, 0, &frontier, &parts, 0)
            .unwrap();
    }

    #[test]
    fn frontier_crash_child() {
        let Some(root) = std::env::var_os("ENTMOOT_TEST_STITCH_CRASH") else {
            return;
        };
        let root = Path::new(&root);
        let store = root.parent().unwrap().join("shared.db");
        let (mut db, job) = setup(root, &store);
        if std::env::var("ENTMOOT_TEST_STITCH_PHASE").unwrap() == "syntax" {
            syntax(&mut db, &job, root, 1);
        } else {
            syntax(&mut db, &job, root, 2);
            let request: ProducerRequest = serde_json::from_value(job.spec.input.clone()).unwrap();
            let graph = db
                .inventory_syntax_graph(job.id.as_str(), root, &request.source.snapshot)
                .unwrap();
            let index = crate::resolve_edges::StructuralCallIndex::new(&graph);
            let frontier = db.inventory_frontier(job.id.as_str(), 1).unwrap().unwrap();
            let parts = frontier
                .outcomes
                .iter()
                .map(|o| Part::Calls(CallPart::build(&index, o)))
                .collect::<Vec<_>>();
            db.checkpoint_inventory_frontier(
                job.id.as_str(),
                job.generation,
                0,
                &frontier,
                &parts,
                0,
            )
            .unwrap();
        }
        std::fs::write(root.parent().unwrap().join("operation"), job.id.as_str()).unwrap();
        std::process::exit(0);
    }

    #[test]
    fn graph_frontiers_survive_process_exit_collection_and_new_worker_generation() {
        for phase in ["syntax", "calls"] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("source");
            std::fs::create_dir(&root).unwrap();
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "knowledge_db::operations::stitch::tests::frontier_crash_child"
                ])
                .env("ENTMOOT_TEST_STITCH_CRASH", &root)
                .env("ENTMOOT_TEST_STITCH_PHASE", phase)
                .status()
                .unwrap()
                .success());
            let store = tmp.path().join("shared.db");
            let id = std::fs::read_to_string(tmp.path().join("operation")).unwrap();
            let before = SqliteKnowledgeRepository::open_operations_read_only(&store)
                .unwrap()
                .operation(&id)
                .unwrap()
                .unwrap();
            assert_eq!(before.state, OperationState::Running);
            crate::knowledge_db::retention::Maintenance::open(&store)
                .unwrap()
                .collect(&std::collections::BTreeSet::new())
                .unwrap();
            let done = crate::producer_operations::run_bounded(
                &store,
                &root,
                &id,
                "replacement",
                10,
                1,
                None,
            )
            .unwrap();
            assert_eq!(done.state, OperationState::Completed);
            assert!(done.generation > before.generation);
            let graph = SqliteKnowledgeRepository::open_operations_read_only(&store)
                .unwrap()
                .retained_graph(done.result.unwrap()["view"].as_str().unwrap())
                .unwrap();
            assert!(graph.edges.iter().any(|e| {
                e.kind == crate::EdgeKind::Calls
                    && e.resolved
                    && graph
                        .node(&e.from)
                        .is_some_and(|n| n.path.as_deref() == Some("a.py"))
                    && graph
                        .node(&e.to)
                        .is_some_and(|n| n.path.as_deref() == Some("b.py"))
            }));
        }
    }

    #[test]
    fn incomplete_stitch_cannot_stage_and_stale_or_replayed_frontiers_cannot_advance() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let store = tmp.path().join("shared.db");
        let (mut db, old) = setup(&root, &store);
        let graph = crate::build_topology_from_repo_with_options(
            &root,
            crate::TopologyBuildOptions {
                repository: Some("frontier/recovery"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(db
            .stage_producer_graph(old.id.as_str(), old.generation, 0, &graph, None)
            .unwrap_err()
            .to_string()
            .contains("incomplete graph frontiers"));
        let frontier = db.inventory_frontier(old.id.as_str(), 1).unwrap().unwrap();
        let parts = frontier
            .outcomes
            .iter()
            .map(|o| Part::Syntax(SyntaxPart::build(&root, o).unwrap()))
            .collect::<Vec<_>>();
        let new = db.claim_operation(old.id.as_str(), "new", 1, 100).unwrap();
        assert!(db
            .checkpoint_inventory_frontier(old.id.as_str(), old.generation, 1, &frontier, &parts, 0)
            .is_err());
        db.checkpoint_inventory_frontier(old.id.as_str(), new.generation, 1, &frontier, &parts, 0)
            .unwrap();
        assert!(db
            .checkpoint_inventory_frontier(old.id.as_str(), new.generation, 1, &frontier, &parts, 0)
            .is_err());
        let request: ProducerRequest = serde_json::from_value(old.spec.input.clone()).unwrap();
        assert!(db
            .inventory_syntax_graph(old.id.as_str(), &root, &request.source.snapshot)
            .is_err());
        db.control_operation(old.id.as_str(), entmoot_contracts::OperationControl::Cancel)
            .unwrap();
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM local_inventory_parts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
