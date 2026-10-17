//! Restart, fencing, atomic admission and multi-client controls.
use entmoot_contracts::{OperationControl, OperationKind, OperationSpec, OperationState};
use entmoot_engine::{
    KnowledgeRepository, KnowledgeStore, LocalOperations, OperationError, SqliteKnowledgeRepository,
};
use serde_json::json;

fn spec(key: &str) -> OperationSpec {
    OperationSpec {
        repository_id: "repo".into(),
        idempotency_key: key.into(),
        owner: "agent".into(),
        kind: OperationKind::Conflicts,
        views: vec![],
        prerequisites: vec![],
        input: json!({"request":1}),
        implementation: entmoot_engine::implementation_fingerprint(),
    }
}

#[test]
fn scheduler_reads_keep_the_callers_snapshot_and_repositories_have_separate_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scheduler.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    db.replace_all(&KnowledgeStore::default()).unwrap();
    let pinned = SqliteKnowledgeRepository::open_read_snapshot(&path).unwrap();
    let before = pinned.scheduler_status("repo", 10).unwrap();
    db.configure_scheduler(
        "repo",
        0,
        entmoot_contracts::SchedulerPolicy {
            max_running: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut other = spec("other");
    other.repository_id = "other/repository".into();
    let jobs = db
        .submit_operations(&[spec("first"), spec("waiting"), other], 3)
        .unwrap();
    db.claim_operation(jobs[0].id.as_str(), "first", 1, 1000)
        .unwrap();
    assert!(matches!(
        db.claim_operation(jobs[1].id.as_str(), "waiting", 2, 1000),
        Err(OperationError::Capacity)
    ));
    db.claim_operation(jobs[2].id.as_str(), "other", 3, 1000)
        .unwrap();
    assert_eq!(pinned.scheduler_status("repo", 10).unwrap(), before);
    assert!(pinned
        .scheduler_status("other/repository", 10)
        .unwrap()
        .active
        .is_empty());
    let fresh = SqliteKnowledgeRepository::open_read_snapshot(&path).unwrap();
    let status = fresh.scheduler_status("repo", 10).unwrap();
    assert_eq!(status.configuration.generation, 1);
    assert_eq!(status.active.len(), 1);
    assert_eq!(status.waiting_total, 1);
    assert_eq!(
        fresh
            .scheduler_status("other/repository", 10)
            .unwrap()
            .active[0]
            .id,
        jobs[2].id
    );
}

#[test]
fn weighted_fifo_prevents_small_newcomers_from_starving_a_large_job() {
    use entmoot_contracts::SchedulerPolicy;
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let policy = SchedulerPolicy {
        max_running: 3,
        work_units: 3,
        costs: [(OperationKind::Binding, 3)].into(),
        wait_lease_ms: 1000,
        ..Default::default()
    };
    db.configure_scheduler("repo", 0, policy).unwrap();
    let mut heavy = spec("heavy");
    heavy.kind = OperationKind::Binding;
    let jobs = db
        .submit_operations(&[spec("active"), heavy, spec("newcomer")], 4)
        .unwrap();
    db.claim_operation(jobs[0].id.as_str(), "a", 10, 2000)
        .unwrap();
    for (index, now) in [(1, 20), (2, 30)] {
        assert!(matches!(
            db.claim_operation(jobs[index].id.as_str(), "wait", now, 2000),
            Err(OperationError::Capacity)
        ));
        assert_eq!(
            db.operation(jobs[index].id.as_str()).unwrap().unwrap(),
            jobs[index]
        );
    }
    let status = db.scheduler_status("repo", 40).unwrap();
    assert_eq!(status.used_work_units, 1);
    assert_eq!(status.next, Some(jobs[1].id.clone()));
    assert!(!status.next_can_run);
    assert_eq!(
        status
            .waiting
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>(),
        vec![jobs[1].id.clone(), jobs[2].id.clone()]
    );
    db.control_operation(jobs[0].id.as_str(), OperationControl::Cancel)
        .unwrap();
    assert!(db.scheduler_status("repo", 50).unwrap().next_can_run);
    assert!(matches!(
        db.claim_operation(jobs[2].id.as_str(), "newer", 50, 2000),
        Err(OperationError::Capacity)
    ));
    db.claim_operation(jobs[1].id.as_str(), "heavy", 60, 2000)
        .unwrap();
    assert_eq!(db.scheduler_status("repo", 61).unwrap().used_work_units, 3);
    db.control_operation(jobs[1].id.as_str(), OperationControl::Cancel)
        .unwrap();
    db.claim_operation(jobs[2].id.as_str(), "newer", 70, 2000)
        .unwrap();
    assert_eq!(db.scheduler_status("repo", 71).unwrap().waiting_total, 0);
}

#[test]
fn abandoned_waiters_expire_and_controls_release_their_turn_without_losing_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    let policy = entmoot_contracts::SchedulerPolicy {
        max_running: 1,
        wait_lease_ms: 1000,
        max_waiting: 2,
        ..Default::default()
    };
    db.configure_scheduler("repo", 0, policy).unwrap();
    let jobs = db
        .submit_operations(
            &[
                spec("active"),
                spec("abandoned"),
                spec("retry"),
                spec("overflow"),
            ],
            4,
        )
        .unwrap();
    db.claim_operation(jobs[0].id.as_str(), "a", 0, 3000)
        .unwrap();
    for (index, now) in [(1, 10), (2, 20), (3, 30)] {
        assert!(matches!(
            db.claim_operation(jobs[index].id.as_str(), "waiting", now, 1000),
            Err(OperationError::Capacity)
        ));
    }
    let state = db.scheduler_status("repo", 100).unwrap();
    assert_eq!(state.waiting_total, 2);
    assert!(!state.waiting.iter().any(|r| r.id == jobs[3].id));
    drop(db);
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    assert_eq!(db.scheduler_status("repo", 100).unwrap(), state);
    // Refresh the second ticket; the first one must retain its original expiry.
    assert!(matches!(
        db.claim_operation(jobs[2].id.as_str(), "replacement", 900, 1000),
        Err(OperationError::Capacity)
    ));
    db.control_operation(jobs[0].id.as_str(), OperationControl::Pause)
        .unwrap();
    let read_only = SqliteKnowledgeRepository::open_operations_read_only(&path).unwrap();
    let state = read_only.scheduler_status("repo", 1010).unwrap();
    assert_eq!(state.next, Some(jobs[2].id.clone()));
    assert_eq!(state.waiting_total, 1);
    // A status read filters expiry but must not mutate/prune the queue.
    assert_eq!(
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM local_operation_waiters", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let worker = db
        .claim_operation(jobs[2].id.as_str(), "replacement", 1010, 1000)
        .unwrap();
    db.checkpoint_operation(
        jobs[2].id.as_str(),
        worker.generation,
        1,
        1011,
        json!({"saved":42}),
        None,
    )
    .unwrap();
    db.yield_operation(jobs[2].id.as_str(), worker.generation, 1012, "slice")
        .unwrap();
    db.control_operation(jobs[2].id.as_str(), OperationControl::Resume)
        .unwrap();
    db.claim_operation(jobs[3].id.as_str(), "next", 1020, 1000)
        .unwrap();
    assert!(matches!(
        db.claim_operation(jobs[2].id.as_str(), "replacement", 1021, 1000),
        Err(OperationError::Capacity)
    ));
    db.control_operation(jobs[2].id.as_str(), OperationControl::Pause)
        .unwrap();
    assert_eq!(db.scheduler_status("repo", 1022).unwrap().waiting_total, 0);
    assert_eq!(
        db.operation(jobs[2].id.as_str())
            .unwrap()
            .unwrap()
            .checkpoint,
        json!({"saved":42})
    );
}

#[test]
fn scheduler_configuration_is_cas_scoped_and_does_not_revoke_active_workers() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let jobs = db
        .submit_operations(&[spec("a"), spec("b"), spec("c")], 4)
        .unwrap();
    let a = db
        .claim_operation(jobs[0].id.as_str(), "a", 1, 1000)
        .unwrap();
    db.claim_operation(jobs[1].id.as_str(), "b", 1, 1000)
        .unwrap();
    let policy = entmoot_contracts::SchedulerPolicy {
        max_running: 1,
        max_binding: 0,
        ..Default::default()
    };
    let config = db.configure_scheduler("repo", 0, policy.clone()).unwrap();
    assert_eq!(config.generation, 1);
    assert!(matches!(
        db.configure_scheduler("repo", 0, Default::default()),
        Err(OperationError::Conflict(_))
    ));
    assert_eq!(
        db.scheduler_status("repo", 2).unwrap().configuration,
        config
    );
    assert_eq!(
        db.scheduler_status("other", 2)
            .unwrap()
            .configuration
            .generation,
        0
    );
    assert_eq!(db.operation(jobs[0].id.as_str()).unwrap().unwrap(), a);
    db.renew_operation(jobs[0].id.as_str(), a.generation, 3, 1000)
        .unwrap();
    assert!(matches!(
        db.claim_operation(jobs[2].id.as_str(), "c", 4, 1000),
        Err(OperationError::Capacity)
    ));
    db.control_operation(jobs[0].id.as_str(), OperationControl::Cancel)
        .unwrap();
    assert!(matches!(
        db.claim_operation(jobs[2].id.as_str(), "c", 5, 1000),
        Err(OperationError::Capacity)
    ));
    db.control_operation(jobs[1].id.as_str(), OperationControl::Cancel)
        .unwrap();
    db.claim_operation(jobs[2].id.as_str(), "c", 6, 1000)
        .unwrap();
    for bad in [
        entmoot_contracts::SchedulerPolicy {
            max_running: 0,
            ..policy.clone()
        },
        entmoot_contracts::SchedulerPolicy {
            costs: [(OperationKind::Conflicts, 0)].into(),
            ..policy
        },
    ] {
        assert!(db.configure_scheduler("repo", 1, bad).is_err());
        assert_eq!(
            db.scheduler_status("repo", 7).unwrap().configuration,
            config
        );
    }
}

#[test]
fn disabled_native_waiters_and_unrequested_or_blocked_jobs_do_not_block_other_work() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    db.configure_scheduler(
        "repo",
        0,
        entmoot_contracts::SchedulerPolicy {
            max_binding: 0,
            ..Default::default()
        },
    )
    .unwrap();
    let mut native = spec("native");
    native.kind = OperationKind::Binding;
    let parent = db
        .submit_operations(&[spec("unrequested")], 8)
        .unwrap()
        .remove(0);
    let mut blocked = spec("blocked");
    blocked.prerequisites.push(parent.id);
    let jobs = db
        .submit_operations(&[native, blocked, spec("ready")], 8)
        .unwrap();
    assert!(matches!(
        db.claim_operation(jobs[0].id.as_str(), "native", 10, 1000),
        Err(OperationError::Capacity)
    ));
    assert!(matches!(
        db.claim_operation(jobs[1].id.as_str(), "blocked", 11, 1000),
        Err(OperationError::Conflict(_))
    ));
    let ready = db
        .claim_operation(jobs[2].id.as_str(), "ready", 12, 1000)
        .unwrap();
    let status = db.scheduler_status("repo", 13).unwrap();
    assert_eq!(status.active[0].id, ready.id);
    assert_eq!(status.waiting_total, 1);
    assert!(status.next.is_none());
    db.configure_scheduler("repo", 1, Default::default())
        .unwrap();
    assert_eq!(
        db.scheduler_status("repo", 14).unwrap().next,
        Some(jobs[0].id.clone())
    );
    db.claim_operation(jobs[0].id.as_str(), "native", 15, 1000)
        .unwrap();
}

#[test]
fn invalid_clock_never_advances_ownership_or_creates_waiting_interest() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let job = db.submit_operations(&[spec("clock")], 1).unwrap().remove(0);
    for (now, lease) in [
        (i64::MAX as u64, 1),
        (i64::MAX as u64 - 10, 1),
        (u64::MAX - 1, 1),
        (0, 0),
    ] {
        assert!(db
            .claim_operation(job.id.as_str(), "worker", now, lease)
            .is_err());
        assert_eq!(db.operation(job.id.as_str()).unwrap().unwrap(), job);
        assert_eq!(db.scheduler_status("repo", 0).unwrap().waiting_total, 0);
    }
}

#[test]
fn concurrent_clients_respect_weighted_limits_and_drain_in_persisted_fifo_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    db.configure_scheduler(
        "repo",
        0,
        entmoot_contracts::SchedulerPolicy {
            max_running: 4,
            work_units: 6,
            costs: [(OperationKind::Binding, 3)].into(),
            ..Default::default()
        },
    )
    .unwrap();
    let specs: Vec<_> = (0..24)
        .map(|n| {
            let mut item = spec(&format!("job-{n}"));
            if n % 3 == 0 {
                item.kind = OperationKind::Binding;
            }
            item
        })
        .collect();
    let jobs = db.submit_operations(&specs, 24).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(jobs.len()));
    let handles: Vec<_> = jobs
        .iter()
        .map(|job| {
            let path = path.clone();
            let id = job.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
                barrier.wait();
                db.claim_operation(id.as_str(), "client", 10, 10_000)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let state = db.scheduler_status("repo", 11).unwrap();
    assert!(!state.active.is_empty());
    assert!(state.active.len() <= 4);
    assert!(state.used_work_units <= 6);
    assert!(
        state
            .active
            .iter()
            .filter(|r| r.kind == OperationKind::Binding)
            .count()
            <= 1
    );
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        state.active.len()
    );
    let unexpected: Vec<_> = results
        .iter()
        .filter(|r| {
            !matches!(
                r,
                Ok(_)
                    | Err(OperationError::Capacity)
                    // Concurrent Immediate writers can surface SQLITE_BUSY under load.
                    | Err(OperationError::Sqlite(_))
            )
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "claim losers must be capacity or transient sqlite busy; got {unexpected:?}"
    );
    assert_eq!(state.active.len() + state.waiting_total, 24);
    for active in state.active {
        db.control_operation(active.id.as_str(), OperationControl::Cancel)
            .unwrap();
    }
    let order: Vec<_> = state.waiting.into_iter().map(|r| r.id).collect();
    drop(db);
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    for (position, id) in order.iter().enumerate() {
        assert_eq!(
            db.scheduler_status("repo", 20).unwrap().next.as_ref(),
            Some(id)
        );
        if let Some(newer) = order.last().filter(|last| *last != id) {
            assert!(matches!(
                db.claim_operation(newer.as_str(), "bypass", 20, 1000),
                Err(OperationError::Capacity)
            ));
        }
        db.claim_operation(id.as_str(), "ordered", 20, 1000)
            .unwrap();
        db.control_operation(id.as_str(), OperationControl::Cancel)
            .unwrap();
        assert_eq!(
            db.scheduler_status("repo", 20).unwrap().waiting_total,
            order.len() - position - 1
        );
    }
}

#[test]
fn waiting_pages_are_bounded_and_configuration_and_admission_failures_are_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    let jobs = db
        .submit_operations(
            &(0..106)
                .map(|i| spec(&format!("job-{i}")))
                .collect::<Vec<_>>(),
            106,
        )
        .unwrap();
    db.claim_operation(jobs[0].id.as_str(), "a", 1, 5000)
        .unwrap();
    db.claim_operation(jobs[1].id.as_str(), "b", 1, 5000)
        .unwrap();
    for job in &jobs[2..] {
        assert!(matches!(
            db.claim_operation(job.id.as_str(), "wait", 2, 5000),
            Err(OperationError::Capacity)
        ));
    }
    let before = db.scheduler_status("repo", 3).unwrap();
    assert_eq!(before.waiting_total, 104);
    assert_eq!(before.waiting.len(), 100);
    assert!(before.waiting_truncated);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER deny_policy BEFORE INSERT ON local_scheduler_profiles BEGIN SELECT RAISE(ABORT,'policy fault'); END;").unwrap();
    assert!(db
        .configure_scheduler("repo", 0, Default::default())
        .is_err());
    assert_eq!(db.scheduler_status("repo", 3).unwrap(), before);
    conn.execute_batch("DROP TRIGGER deny_policy;").unwrap();
    db.control_operation(jobs[0].id.as_str(), OperationControl::Cancel)
        .unwrap();
    let before = db.scheduler_status("repo", 4).unwrap();
    conn.execute_batch("CREATE TRIGGER deny_claim BEFORE UPDATE ON local_operations WHEN NEW.state='\"running\"' BEGIN SELECT RAISE(ABORT,'claim fault'); END;").unwrap();
    assert!(db
        .claim_operation(jobs[2].id.as_str(), "first", 4, 5000)
        .is_err());
    assert_eq!(db.scheduler_status("repo", 4).unwrap(), before);
    assert_eq!(db.operation(jobs[2].id.as_str()).unwrap().unwrap(), jobs[2]);
    conn.execute_batch("DROP TRIGGER deny_claim;").unwrap();
    db.claim_operation(jobs[2].id.as_str(), "first", 4, 5000)
        .unwrap();
    assert_eq!(
        db.scheduler_status("repo", 4).unwrap().next,
        Some(jobs[3].id.clone())
    );
}

#[test]
fn heartbeat_preserves_progress_and_never_resurrects_revoked_ownership() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let job = db
        .submit_operations(&[spec("heartbeat")], 1)
        .unwrap()
        .remove(0);
    let running = db
        .claim_operation(job.id.as_str(), "first", 100, 100)
        .unwrap();
    let progress = db
        .checkpoint_operation(
            job.id.as_str(),
            running.generation,
            1,
            110,
            json!({"saved":42}),
            None,
        )
        .unwrap();
    let renewed = db
        .renew_operation(job.id.as_str(), running.generation, 120, 200)
        .unwrap();
    assert_eq!(renewed.lease_until_ms, Some(320));
    assert_eq!(renewed.checkpoint, progress.checkpoint);
    assert_eq!(renewed.checkpoint_sequence, 1);
    assert_eq!(
        db.renew_operation(job.id.as_str(), running.generation, 121, 1)
            .unwrap(),
        renewed
    );
    for (now, lease) in [(122, 0), (122, 3_600_001), (u64::MAX, 1), (320, 1)] {
        assert!(db
            .renew_operation(job.id.as_str(), running.generation, now, lease)
            .is_err());
        assert_eq!(db.operation(job.id.as_str()).unwrap().unwrap(), renewed);
    }
    db.control_operation(job.id.as_str(), OperationControl::Pause)
        .unwrap();
    assert!(db
        .renew_operation(job.id.as_str(), running.generation, 130, 500)
        .is_err());
    db.control_operation(job.id.as_str(), OperationControl::Resume)
        .unwrap();
    let second = db
        .claim_operation(job.id.as_str(), "second", 131, 100)
        .unwrap();
    assert!(db
        .renew_operation(job.id.as_str(), running.generation, 132, 500)
        .is_err());
    assert_eq!(db.operation(job.id.as_str()).unwrap().unwrap(), second);
    db.control_operation(job.id.as_str(), OperationControl::Cancel)
        .unwrap();
    assert!(db
        .renew_operation(job.id.as_str(), second.generation, 132, 500)
        .is_err());
}

#[test]
fn generic_checkpoints_cannot_complete_producers_without_atomic_evidence_publication() {
    for kind in [
        OperationKind::Inventory,
        OperationKind::Canon,
        OperationKind::Binding,
        OperationKind::Revalidation,
    ] {
        let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let mut request = spec("protected");
        request.kind = kind;
        let job = db.submit_operations(&[request], 1).unwrap().remove(0);
        let running = db
            .claim_operation(job.id.as_str(), "worker", 100, 100)
            .unwrap();
        assert!(db
            .checkpoint_operation(
                job.id.as_str(),
                running.generation,
                1,
                101,
                json!({"view":"forged"}),
                Some(json!({"complete":true}))
            )
            .is_err());
        assert_eq!(db.operation(job.id.as_str()).unwrap().unwrap(), running);
    }
}

#[test]
fn restart_preserves_progress_and_recovery_fences_the_previous_worker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    db.replace_all(&KnowledgeStore::default()).unwrap();
    let job = db.submit_operations(&[spec("a")], 2).unwrap().remove(0);
    let first = db
        .claim_operation(job.id.as_str(), "worker-a", 100, 10)
        .unwrap();
    db.checkpoint_operation(
        job.id.as_str(),
        first.generation,
        1,
        105,
        json!({"frontier":[2,3]}),
        None,
    )
    .unwrap();
    drop(db);
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    assert_eq!(
        db.operation(job.id.as_str()).unwrap().unwrap().checkpoint,
        json!({"frontier":[2,3]})
    );
    assert!(db
        .claim_operation(job.id.as_str(), "worker-b", 109, 20)
        .is_err());
    let recovered = db
        .claim_operation(job.id.as_str(), "worker-b", 110, 20)
        .unwrap();
    assert!(recovered.generation > first.generation);
    assert!(db
        .checkpoint_operation(job.id.as_str(), first.generation, 2, 111, json!({}), None)
        .is_err());
    let done = db
        .checkpoint_operation(
            job.id.as_str(),
            recovered.generation,
            2,
            112,
            json!({"frontier":[]}),
            Some(json!({"answer":42})),
        )
        .unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(
        db.checkpoint_operation(
            job.id.as_str(),
            recovered.generation,
            2,
            999,
            json!({"frontier":[]}),
            Some(json!({"answer":42}))
        )
        .unwrap(),
        done
    );
    assert!(db
        .control_operation(job.id.as_str(), OperationControl::Cancel)
        .is_err());
}

#[test]
fn admission_is_atomic_idempotent_and_rejects_request_drift() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    assert!(matches!(
        db.submit_operations(&[spec("a"), spec("b")], 1),
        Err(OperationError::Capacity)
    ));
    let row = db.submit_operations(&[spec("b")], 1).unwrap().remove(0);
    assert_eq!(db.submit_operations(&[spec("b")], 1).unwrap(), vec![row]);
    let mut changed = spec("b");
    changed.input = json!({"request":2});
    assert!(matches!(
        db.submit_operations(&[changed], 1),
        Err(OperationError::Conflict(_))
    ));
}

#[test]
fn pause_cancel_dependencies_and_invalid_sequences_do_not_lose_progress() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let parent = db
        .submit_operations(&[spec("parent")], 4)
        .unwrap()
        .remove(0);
    let mut child = spec("child");
    child.prerequisites.push(parent.id.clone());
    let child = db.submit_operations(&[child], 4).unwrap().remove(0);
    assert!(db
        .claim_operation(child.id.as_str(), "worker", 1, 100)
        .is_err());
    let running = db
        .claim_operation(parent.id.as_str(), "worker", 1, 100)
        .unwrap();
    assert!(db
        .checkpoint_operation(parent.id.as_str(), running.generation, 2, 2, json!(1), None)
        .is_err());
    db.control_operation(parent.id.as_str(), OperationControl::Pause)
        .unwrap();
    assert!(db
        .checkpoint_operation(parent.id.as_str(), running.generation, 1, 3, json!(1), None)
        .is_err());
    assert!(db
        .claim_operation(parent.id.as_str(), "worker", 102, 100)
        .is_err());
    db.control_operation(parent.id.as_str(), OperationControl::Resume)
        .unwrap();
    let resumed = db
        .claim_operation(parent.id.as_str(), "worker", 103, 100)
        .unwrap();
    db.checkpoint_operation(
        parent.id.as_str(),
        resumed.generation,
        1,
        104,
        json!(1),
        Some(json!(true)),
    )
    .unwrap();
    db.claim_operation(child.id.as_str(), "worker", 105, 100)
        .unwrap();
    db.control_operation(child.id.as_str(), OperationControl::Cancel)
        .unwrap();
    assert!(db
        .claim_operation(child.id.as_str(), "worker", 999, 100)
        .is_err());
}

#[test]
fn simultaneous_claims_have_exactly_one_winner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    let job = db.submit_operations(&[spec("a")], 1).unwrap().remove(0);
    drop(db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let path = path.clone();
            let barrier = barrier.clone();
            let id = job.id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
                db.claim_operation(id.as_str(), &format!("worker-{n}"), 100, 1000)
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(outcomes
        .iter()
        .filter_map(|r| r.as_ref().err())
        .all(|e| matches!(e, OperationError::Conflict(_))));
}

#[test]
fn invalid_inputs_and_fingerprints_never_enter_the_queue() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    for field in 0..3 {
        let mut value = spec("a");
        match field {
            0 => value.repository_id.clear(),
            1 => value.idempotency_key.clear(),
            _ => value.owner.clear(),
        }
        assert!(db.submit_operations(&[value], 10).is_err());
    }
    let mut value = spec("a");
    value.implementation.contracts =
        entmoot_contracts::ContentDigest::from_trusted(entmoot_contracts::blake3_hex(b"other"));
    assert!(db.submit_operations(&[value], 10).is_err());
}

#[test]
fn global_admission_is_bounded_and_expired_workers_release_capacity() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let jobs = db
        .submit_operations(&[spec("a"), spec("b"), spec("c")], 3)
        .unwrap();
    db.claim_operation(jobs[0].id.as_str(), "worker", 10, 100)
        .unwrap();
    db.claim_operation(jobs[1].id.as_str(), "worker", 10, 100)
        .unwrap();
    assert!(matches!(
        db.claim_operation(jobs[2].id.as_str(), "worker", 20, 100),
        Err(OperationError::Capacity)
    ));
    db.claim_operation(jobs[2].id.as_str(), "worker", 111, 100)
        .unwrap();
}

#[test]
fn operation_reads_do_not_create_a_missing_store() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing.db");
    assert!(SqliteKnowledgeRepository::open_operations_read_only(&path).is_err());
    assert!(!path.exists());
}

#[test]
fn concurrent_first_open_does_not_mistake_another_initializer_for_foreign_storage() {
    let temp = tempfile::tempdir().unwrap();
    // Include immediate closes while other clients are still entering WAL mode.
    // Independent fresh stores exercise the startup race, not just steady-state opens.
    for round in 0..4 {
        let path = temp.path().join(format!("new-{round}.db"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    SqliteKnowledgeRepository::open(&path).map(|_| ())
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            results.iter().all(Result::is_ok),
            "round {round}: {results:?}"
        );
        let mut repository = SqliteKnowledgeRepository::open(&path).unwrap();
        let submitted = repository
            .submit_operations(&[spec("after-startup")], 1)
            .unwrap();
        drop(repository);
        let reader = SqliteKnowledgeRepository::open_operations_read_only(&path).unwrap();
        assert_eq!(
            reader.operation(submitted[0].id.as_str()).unwrap().as_ref(),
            Some(&submitted[0])
        );
    }
}

#[test]
fn stale_worker_cannot_yield_a_resumed_attempt() {
    let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
    let job = db.submit_operations(&[spec("yield")], 1).unwrap().remove(0);
    let old = db.claim_operation(job.id.as_str(), "old", 1, 10).unwrap();
    let new = db.claim_operation(job.id.as_str(), "new", 12, 10).unwrap();
    assert!(db
        .yield_operation(job.id.as_str(), old.generation, 13, "exhausted")
        .is_err());
    assert_eq!(db.operation(job.id.as_str()).unwrap().unwrap(), new);
    let paused = db
        .yield_operation(job.id.as_str(), new.generation, 13, "slice exhausted")
        .unwrap();
    assert_eq!(paused.state, OperationState::Paused);
    assert_eq!(paused.reason.as_deref(), Some("slice exhausted"));
}

#[test]
fn output_pages_survive_restart_and_exceed_single_record_capacity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("outputs.db");
    let mut db = SqliteKnowledgeRepository::open(&path).unwrap();
    let job = db
        .submit_operations(&[spec("outputs")], 1)
        .unwrap()
        .remove(0);
    let running = db
        .claim_operation(job.id.as_str(), "worker", 1, 100)
        .unwrap();
    let output = json!("x".repeat(3 * 1024 * 1024));
    for sequence in 1..=6 {
        let result = (sequence == 6).then(|| json!({"count":6}));
        let saved = db
            .checkpoint_operation_output(
                job.id.as_str(),
                running.generation,
                sequence,
                2,
                json!(sequence),
                result.clone(),
                output.clone(),
            )
            .unwrap();
        assert_eq!(
            db.checkpoint_operation_output(
                job.id.as_str(),
                running.generation,
                sequence,
                3,
                json!(sequence),
                result.clone(),
                output.clone()
            )
            .unwrap(),
            saved
        );
        assert!(db
            .checkpoint_operation_output(
                job.id.as_str(),
                running.generation,
                sequence,
                3,
                json!(sequence),
                result,
                json!("different")
            )
            .is_err());
    }
    drop(db);
    let db = SqliteKnowledgeRepository::open_operations_read_only(&path).unwrap();
    let first = db.operation_outputs(job.id.as_str(), 0, 100).unwrap();
    assert_eq!(first.outputs.len(), 5); // 16 MiB page ceiling, 18 MiB complete output.
    assert_eq!(first.next_sequence, 5);
    assert!(!first.complete);
    let last = db
        .operation_outputs(job.id.as_str(), first.next_sequence, 1)
        .unwrap();
    assert_eq!(last.outputs.len(), 1);
    assert_eq!(last.outputs[0].sequence, 6);
    assert_eq!(last.outputs[0].value, output);
    assert!(last.complete);
    assert!(db.operation_outputs(job.id.as_str(), 0, 0).is_err());
}
