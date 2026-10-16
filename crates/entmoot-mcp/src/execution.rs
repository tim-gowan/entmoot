//! Bounded blocking execution; permits outlive a cancelled request.
use entmoot_contracts::query::QueryServiceError;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct ReadExecutor {
    active: Arc<Semaphore>,
    admitted: Arc<Semaphore>,
}

impl ReadExecutor {
    pub(crate) fn new(workers: usize, pending: usize) -> Self {
        Self {
            active: Arc::new(Semaphore::new(workers)),
            admitted: Arc::new(Semaphore::new(workers.saturating_add(pending))),
        }
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, QueryServiceError> {
        let admitted = self.admitted.clone().try_acquire_owned().map_err(|_| {
            QueryServiceError::Unavailable("execution capacity exhausted; retry later".into())
        })?;
        let active = self
            .active
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| QueryServiceError::Unavailable("executor closed".into()))?;
        tokio::task::spawn_blocking(move || {
            // Cancelling the async waiter cannot release running-work capacity.
            let _permits = (admitted, active);
            work()
        })
        .await
        .map_err(|error| QueryServiceError::Execution(format!("worker failed: {error}")))
    }
}

/// Control admission cannot queue behind the work it needs to revoke. The control
/// lane is separately bounded and does not consume interactive read capacity.
#[derive(Clone)]
pub(crate) struct OperationExecutor {
    work: ReadExecutor,
    control: ReadExecutor,
}

impl OperationExecutor {
    pub(crate) fn new(workers: usize, pending: usize) -> Self {
        Self {
            work: ReadExecutor::new(workers, pending),
            control: ReadExecutor::new(1, pending),
        }
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        request: entmoot_contracts::CollaborationWrite,
        work: impl FnOnce(entmoot_contracts::CollaborationWrite) -> T + Send + 'static,
    ) -> Result<T, QueryServiceError> {
        let executor = if matches!(
            request,
            entmoot_contracts::CollaborationWrite::Control { .. }
                | entmoot_contracts::CollaborationWrite::ConfigureScheduler { .. }
        ) {
            &self.control
        } else {
            &self.work
        };
        executor.run(move || work(request)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn controls_run_while_every_producer_slot_is_occupied() {
        use entmoot_contracts::{CollaborationWrite, ContentDigest, OperationControl};
        let executor = OperationExecutor::new(2, 0);
        let id = ContentDigest::from_trusted(entmoot_contracts::blake3_hex(b"operation"));
        let request = || CollaborationWrite::RunProducer {
            id: id.clone(),
            worker: "agent".into(),
            steps: 2,
            files_per_step: 128,
            wall_time_ms: None,
        };
        let mut releases = Vec::new();
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let (started, running) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            releases.push(release);
            let executor = executor.clone();
            let request = request();
            tasks.push(tokio::spawn(async move {
                executor
                    .run(request, move |_| {
                        started.send(()).unwrap();
                        wait.recv().unwrap();
                    })
                    .await
            }));
            running.await.unwrap();
        }
        let overload = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            executor.run(request(), |_| ()),
        )
        .await;
        let mut controls = Vec::new();
        for control in [
            OperationControl::Pause,
            OperationControl::Cancel,
            OperationControl::Resume,
        ] {
            controls.push(
                tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    executor.run(
                        CollaborationWrite::Control {
                            id: id.clone(),
                            control,
                        },
                        move |_| control,
                    ),
                )
                .await,
            );
        }
        // Release workers before asserting so a broken admission path cannot hang shutdown.
        let configured = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            executor.run(
                CollaborationWrite::ConfigureScheduler {
                    repository_id: "repo".into(),
                    expected_generation: 0,
                    policy: Default::default(),
                },
                |_| "configured",
            ),
        )
        .await;
        for release in releases {
            release.send(()).unwrap();
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert!(
            overload.unwrap().is_err(),
            "producer admission remains bounded"
        );
        assert_eq!(configured.unwrap().unwrap(), "configured");
        for (observed, expected) in controls.into_iter().zip([
            OperationControl::Pause,
            OperationControl::Cancel,
            OperationControl::Resume,
        ]) {
            assert_eq!(observed.unwrap().unwrap(), expected);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn work_does_not_run_on_the_request_executor() {
        let executor = ReadExecutor::new(1, 1);
        let request_thread = std::thread::current().id();
        let worker = executor.run(|| std::thread::current().id()).await.unwrap();
        assert_ne!(request_thread, worker);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn overload_and_cancellation_keep_running_work_bounded() {
        let executor = ReadExecutor::new(1, 1);
        let (started, running) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let first_executor = executor.clone();
        let first = tokio::spawn(async move {
            first_executor
                .run(move || {
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                })
                .await
        });
        running.await.unwrap();
        let queued_executor = executor.clone();
        let queued = tokio::spawn(async move { queued_executor.run(|| 2).await });
        // Drive the queued request until it is waiting for an active permit.
        tokio::task::yield_now().await;
        assert_eq!(executor.admitted.available_permits(), 0);
        assert!(matches!(
            executor.run(|| 3).await,
            Err(QueryServiceError::Unavailable(_))
        ));
        first.abort();
        let _ = first.await;
        assert_eq!(executor.active.available_permits(), 0);
        queued.abort();
        let _ = queued.await;
        assert_eq!(executor.admitted.available_permits(), 1);
        release.send(()).unwrap();
        assert_eq!(executor.run(|| 4).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn panic_releases_capacity() {
        let executor = ReadExecutor::new(1, 0);
        assert!(executor.run(|| panic!("fixture panic")).await.is_err());
        assert_eq!(executor.run(|| 1).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn thousand_requests_share_a_bounded_worker_set() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let executor = ReadExecutor::new(4, 1000);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut requests = Vec::new();
        for n in 0..1000 {
            let executor = executor.clone();
            let active = active.clone();
            let peak = peak.clone();
            requests.push(tokio::spawn(async move {
                executor
                    .run(move || {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        std::thread::yield_now();
                        active.fetch_sub(1, Ordering::SeqCst);
                        n
                    })
                    .await
                    .unwrap()
            }));
        }
        for (n, request) in requests.into_iter().enumerate() {
            assert_eq!(request.await.unwrap(), n);
        }
        assert!(peak.load(Ordering::SeqCst) <= 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
