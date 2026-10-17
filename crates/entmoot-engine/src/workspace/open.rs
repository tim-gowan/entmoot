//! Shared CLI/MCP publication open: sync bundle + deferred ready gate.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use entmoot_contracts::query::QueryServiceError;

use super::{open_query_publication_options, OpenedPublication, WorkspaceError};

/// Paths and knobs for opening a read-only query publication (+ optional compatibility).
#[derive(Debug, Clone)]
pub struct PublicationOpenSpec {
    pub query_options: crate::query::QueryEngineOptions,
    pub graph: PathBuf,
    pub store: PathBuf,
    pub source_root: Option<PathBuf>,
    pub compare_graph: Option<PathBuf>,
    pub require_source: bool,
    /// Explicit provider graphs/pointers/checkouts; empty discovers the workspace volume's `deps/`.
    pub providers: Vec<PathBuf>,
    /// Workspace root for provider discovery when `providers` is empty.
    /// Defaults to effective source_root, else process cwd, at open time.
    pub discovery_root: Option<PathBuf>,
}

/// Opened publication with compatibility evidence deferred until first use.
pub struct OpenedQueryBundle {
    pub engine: crate::QueryEngine,
    pub compare: Option<std::sync::Arc<crate::TopologyGraph>>,
    pub source_root: Option<PathBuf>,
}

impl PublicationOpenSpec {
    /// Minimal open (no compare, soft source, no explicit providers).
    #[must_use]
    pub fn graph_store(graph: impl Into<PathBuf>, store: impl Into<PathBuf>) -> Self {
        Self {
            query_options: crate::query::QueryEngineOptions::default(),
            graph: graph.into(),
            store: store.into(),
            source_root: None,
            compare_graph: None,
            require_source: false,
            providers: Vec::new(),
            discovery_root: None,
        }
    }
}

/// Open graph/store/compare/source; hydrate native compatibility evidence on first use.
///
/// Emits tracing progress on start/finish. Failures map to [`QueryServiceError::Unavailable`].
pub fn open_query_bundle(
    spec: &PublicationOpenSpec,
) -> Result<OpenedQueryBundle, QueryServiceError> {
    let started = Instant::now();
    tracing::info!(
        graph = %spec.graph.display(),
        store = %spec.store.display(),
        "open query bundle starting"
    );
    let opened = open_query_publication_options(
        &spec.graph,
        &spec.store,
        spec.source_root.as_deref(),
        spec.compare_graph.as_deref(),
        spec.require_source,
        spec.query_options,
    )
    .map_err(unavailable_open)?;
    let OpenedPublication {
        engine,
        publication_digest,
        compare,
        source_root,
    } = opened;
    let discovery = spec
        .discovery_root
        .clone()
        .or_else(|| source_root.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let graph_bytes =
        std::fs::read(&spec.graph).map_err(|error| unavailable_open(WorkspaceError::Io(error)))?;
    let objects = crate::FilesystemArtifactObjectRepository::new(
        super::artifact_store_path(&spec.store).map_err(unavailable_open)?,
    );
    let providers = spec.providers.clone();
    let engine = engine.with_compatibility_loader(move |_graph| {
        // Pin compatibility to the graph bytes observed at open so CLI assess
        // and MCP reads stay aligned even when inventory-backed navigation uses
        // a hydrated projection of the same publication.
        let mut graph: crate::TopologyGraph =
            serde_json::from_slice(&graph_bytes).map_err(|error| {
                QueryServiceError::Unavailable(format!(
                    "compatibility graph bytes are invalid: {error}"
                ))
            })?;
        graph.rebuild_indexes();
        let consumer = crate::compatibility::PackagePublication::from_graph(
            &graph,
            publication_digest.clone(),
            |manifest| {
                let digest =
                    entmoot_contracts::ContentDigest::parse(manifest.content_digest.clone())
                        .map_err(|e| QueryServiceError::Unavailable(e.to_string()))?;
                match objects.get(&digest) {
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(crate::ArtifactStoreError::Io(e))
                        if e.kind() == std::io::ErrorKind::NotFound =>
                    {
                        Ok(None)
                    }
                    Err(e) => Err(QueryServiceError::Unavailable(e.to_string())),
                }
            },
        )?;
        let providers = crate::compatibility::load_providers(&discovery, &providers)?;
        crate::compatibility::CompatibilityCatalog::new(consumer, providers)
    });
    let wall_seconds = started.elapsed().as_secs_f64();
    tracing::info!(
        graph = %spec.graph.display(),
        wall_seconds,
        "open query bundle ready"
    );
    Ok(OpenedQueryBundle {
        engine,
        compare,
        source_root,
    })
}

fn open_expected(
    spec: &PublicationOpenSpec,
    expected: Option<&entmoot_contracts::SourceSnapshot>,
) -> Result<OpenedQueryBundle, QueryServiceError> {
    let bundle = open_query_bundle(spec)?;
    if expected.is_some_and(|snapshot| snapshot != &bundle.engine.graph().snapshot) {
        return Err(QueryServiceError::Unavailable(
            "publication pointer snapshot differs from its graph".into(),
        ));
    }
    Ok(bundle)
}

fn unavailable_open(error: WorkspaceError) -> QueryServiceError {
    QueryServiceError::Unavailable(error.to_string())
}

enum DeferredState {
    Pending,
    Ready(Result<Arc<OpenedQueryBundle>, String>),
}

enum Refresh {
    Ready,
    Warming,
    Failed(String),
}

struct DeferredShared {
    previous: Mutex<std::collections::VecDeque<Arc<OpenedQueryBundle>>>,
    refresh: Mutex<Refresh>,
    state: Mutex<DeferredState>,
    ready: Condvar,
}

/// Background open so MCP stdio can answer `initialize` / `tools/list` first.
#[derive(Clone)]
pub struct DeferredQueryBundle {
    shared: Arc<DeferredShared>,
}

impl DeferredQueryBundle {
    /// Explicit backend startup with passive pointer monitoring. Neither initial
    /// open nor refresh schedules a producer. Readers retain their selected Arc.
    pub fn start_monitored(
        spec: PublicationOpenSpec,
        pointer: PathBuf,
        follow_compare: bool,
        generations: usize,
    ) -> Self {
        let initial = spec.clone();
        let initial_pointer = pointer.clone();
        let bundle = Self::start_with(move || {
            let publication =
                super::read_publication(&initial_pointer).map_err(unavailable_open)?;
            open_expected(&initial, Some(&publication.snapshot))
        });
        let weak = Arc::downgrade(&bundle.shared);
        let worker = std::thread::Builder::new()
            .name("entmoot-publication-monitor".into())
            .spawn(move || {
                let mut current = spec;
                loop {
                    let Some(shared) = weak.upgrade() else {
                        break;
                    };
                    let bundle = Self { shared };
                    let mut retry_delay = Duration::from_secs(1);
                    if bundle.try_get().is_some() {
                        match super::read_publication(&pointer) {
                            Ok(publication) => {
                                let compare =
                                    publication.base.as_ref().map(|base| base.graph.clone());
                                let changed = publication.current.graph != current.graph
                                    || publication.current.store != current.store
                                    || (follow_compare && compare != current.compare_graph);
                                let mismatched =
                                    bundle.try_get().and_then(Result::ok).is_some_and(|opened| {
                                        opened.engine.graph().snapshot != publication.snapshot
                                    });
                                if changed {
                                    let mut replacement = current.clone();
                                    replacement.graph = publication.current.graph;
                                    replacement.store = publication.current.store;
                                    if !replacement.require_source {
                                        replacement.source_root =
                                            Some(publication.current.source_root);
                                    }
                                    if follow_compare {
                                        replacement.compare_graph =
                                            publication.base.map(|base| base.graph);
                                    }
                                    if bundle
                                        .refresh(
                                            &replacement,
                                            Some(&publication.snapshot),
                                            generations,
                                        )
                                        .is_ok()
                                    {
                                        current = replacement;
                                    } else {
                                        retry_delay = Duration::from_secs(5);
                                    }
                                } else if let Ok(mut refresh) = bundle.shared.refresh.lock() {
                                    *refresh = if mismatched {
                                        Refresh::Failed(
                                            "publication pointer snapshot differs from its graph"
                                                .into(),
                                        )
                                    } else {
                                        Refresh::Ready
                                    };
                                }
                            }
                            Err(error) => {
                                retry_delay = Duration::from_secs(5);
                                if let Ok(mut refresh) = bundle.shared.refresh.lock() {
                                    *refresh = Refresh::Failed(error.to_string());
                                }
                            }
                        }
                        if let Some(Ok(current)) = bundle.try_get() {
                            current.engine.expire_continuations();
                        }
                        if let Ok(previous) = bundle.shared.previous.lock() {
                            for bundle in previous.iter() {
                                bundle.engine.expire_continuations();
                            }
                        }
                    }
                    // Do not keep the backend alive through its own monitor.
                    drop(bundle);
                    std::thread::park_timeout(retry_delay);
                }
            });
        if let Err(error) = worker {
            if let Ok(mut refresh) = bundle.shared.refresh.lock() {
                *refresh = Refresh::Failed(format!("cannot start publication monitor: {error}"));
            }
        }
        bundle
    }

    fn refresh(
        &self,
        spec: &PublicationOpenSpec,
        expected: Option<&entmoot_contracts::SourceSnapshot>,
        generations: usize,
    ) -> Result<(), QueryServiceError> {
        *self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Refresh::Warming;
        tracing::info!(graph = %spec.graph.display(), "warming replacement query generation");
        let result = open_expected(spec, expected);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut refresh = self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(mut replacement) => {
                let mut previous = self
                    .shared
                    .previous
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let DeferredState::Ready(Ok(current)) = &*state {
                    replacement
                        .engine
                        .share_continuation_limits(&current.engine);
                    previous.push_back(current.clone());
                }
                while previous.len() >= generations.clamp(1, 2) {
                    previous.pop_front();
                }
                *state = DeferredState::Ready(Ok(Arc::new(replacement)));
                *refresh = Refresh::Ready;
                self.shared.ready.notify_all();
                Ok(())
            }
            Err(error) => {
                tracing::warn!(%error,"query generation refresh failed");
                *refresh = Refresh::Failed(error.to_string());
                Err(error)
            }
        }
    }

    /// Resolve a search/page continuation against its original retained generation.
    pub fn wait_for_token(
        &self,
        token: Option<&str>,
        timeout: Duration,
    ) -> Result<Arc<OpenedQueryBundle>, QueryServiceError> {
        let Some(token) = token else {
            return self.wait(timeout);
        };
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let DeferredState::Ready(Ok(current)) = &*state {
            if current.engine.owns_continuation(token) {
                return Ok(current.clone());
            }
        }
        let previous = self
            .shared
            .previous
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        previous.iter().find(|bundle| bundle.engine.owns_continuation(token)).cloned().ok_or_else(|| QueryServiceError::Unavailable("search continuation generation expired or backend restarted; explicitly start a new resumable query".into()))
    }

    /// Start opening `spec` on a worker thread.
    #[must_use]
    pub fn start(spec: PublicationOpenSpec) -> Self {
        Self::start_with(move || open_query_bundle(&spec))
    }

    /// Start with a custom opener (tests / alternate loaders).
    #[must_use]
    pub fn start_with<F>(opener: F) -> Self
    where
        F: FnOnce() -> Result<OpenedQueryBundle, QueryServiceError> + Send + 'static,
    {
        let shared = Arc::new(DeferredShared {
            previous: Default::default(),
            refresh: Mutex::new(Refresh::Ready),
            state: Mutex::new(DeferredState::Pending),
            ready: Condvar::new(),
        });
        let thread_shared = Arc::clone(&shared);
        match std::thread::Builder::new()
            .name("entmoot-open-bundle".into())
            .spawn(move || {
                let result = opener().map(Arc::new).map_err(|error| error.to_string());
                let mut guard = thread_shared
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *guard = DeferredState::Ready(result);
                thread_shared.ready.notify_all();
            }) {
            Ok(_) => {}
            Err(error) => {
                let mut guard = shared
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *guard = DeferredState::Ready(Err(format!("failed to spawn open worker: {error}")));
                shared.ready.notify_all();
            }
        }
        Self { shared }
    }

    /// Block until open completes or `timeout` elapses.
    ///
    /// # Errors
    /// - `publication_warming:` when still pending after the timeout
    /// - `publication_open_failed:` when the worker returned an open error
    pub fn wait(&self, timeout: Duration) -> Result<Arc<OpenedQueryBundle>, QueryServiceError> {
        match &*self
            .shared
            .refresh
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Refresh::Warming => {
                return Err(QueryServiceError::Unavailable(
                    "publication_warming: replacement generation is being validated".into(),
                ))
            }
            Refresh::Failed(error) => {
                return Err(QueryServiceError::Unavailable(format!(
                    "publication_refresh_failed: {error}"
                )))
            }
            Refresh::Ready => {}
        }
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match &*guard {
                DeferredState::Ready(Ok(bundle)) => return Ok(Arc::clone(bundle)),
                DeferredState::Ready(Err(message)) => {
                    return Err(QueryServiceError::Unavailable(format!(
                        "publication_open_failed: {message}"
                    )));
                }
                DeferredState::Pending => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(QueryServiceError::Unavailable(format!(
                            "publication_warming: open still in progress after {}s",
                            timeout.as_secs()
                        )));
                    }
                    let (next, wait) = self
                        .shared
                        .ready
                        .wait_timeout(guard, deadline.saturating_duration_since(now))
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard = next;
                    if wait.timed_out() && matches!(*guard, DeferredState::Pending) {
                        return Err(QueryServiceError::Unavailable(format!(
                            "publication_warming: open still in progress after {}s",
                            timeout.as_secs()
                        )));
                    }
                }
            }
        }
    }

    /// Non-blocking peek for tests.
    #[must_use]
    pub fn try_get(&self) -> Option<Result<Arc<OpenedQueryBundle>, QueryServiceError>> {
        let guard = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*guard {
            DeferredState::Pending => None,
            DeferredState::Ready(Ok(bundle)) => Some(Ok(Arc::clone(bundle))),
            DeferredState::Ready(Err(message)) => Some(Err(QueryServiceError::Unavailable(
                format!("publication_open_failed: {message}"),
            ))),
        }
    }
}

/// Convenience: sync open via [`PublicationOpenSpec::graph_store`] plus optional knobs.
pub fn open_query_bundle_paths(
    graph: &Path,
    store: &Path,
    source_root: Option<&Path>,
    compare_graph: Option<&Path>,
    require_source: bool,
) -> Result<OpenedQueryBundle, QueryServiceError> {
    open_query_bundle(&PublicationOpenSpec {
        query_options: crate::query::QueryEngineOptions::default(),
        graph: graph.to_path_buf(),
        store: store.to_path_buf(),
        source_root: source_root.map(Path::to_path_buf),
        compare_graph: compare_graph.map(Path::to_path_buf),
        require_source,
        providers: Vec::new(),
        discovery_root: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build_topology_from_repo_with_options, write_graph_json, KnowledgeRepository,
        KnowledgeStore, SqliteKnowledgeRepository, TopologyBuildOptions,
    };
    use std::sync::Barrier;

    fn fixture_publication() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("main.rs"), "fn work() {}\n").unwrap();
        let graph = build_topology_from_repo_with_options(
            directory.path(),
            TopologyBuildOptions {
                policy: None,
                repository: Some("fixture/open"),
            },
        )
        .unwrap();
        let graph_path = directory.path().join("graph.json");
        let store_path = directory.path().join("graph.json.knowledge.sqlite3");
        write_graph_json(&graph, &graph_path).unwrap();
        let mut repository = SqliteKnowledgeRepository::open(&store_path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        (directory, graph_path, store_path)
    }

    #[test]
    fn refresh_pins_inflight_reads_and_retires_only_old_continuation_generations() {
        use entmoot_contracts::query::{QueryPayload, TraversePayload, TraverseQuery};
        let (_dir, graph, store) = fixture_publication();
        let spec = PublicationOpenSpec::graph_store(graph, store);
        let deferred = DeferredQueryBundle::start(spec.clone());
        let first = deferred.wait(Duration::from_secs(30)).unwrap();
        let node = first
            .engine
            .graph()
            .nodes
            .iter()
            .find(|n| n.kind == crate::NodeKind::Callable)
            .unwrap()
            .id
            .clone();
        let request: TraverseQuery = serde_json::from_value(
            serde_json::json!({"kind":"path","start":node,"target":node,"predicates":[],"resumable":true,"budget":entmoot_contracts::query::BoundedWalkBudget::default()}),
        )
        .unwrap();
        let result = first.engine.traverse(request.clone()).unwrap();
        let QueryPayload::Traverse(TraversePayload::Path(path)) = &result.payload else {
            panic!("path");
        };
        let token = path.search_receipt.slice_token.as_deref().unwrap();
        deferred
            .refresh(&spec, Some(&first.engine.graph().snapshot), 2)
            .unwrap();
        let second = deferred.wait(Duration::ZERO).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(
            &first,
            &deferred
                .wait_for_token(Some(token), Duration::ZERO)
                .unwrap()
        ));
        let mut replay = request;
        replay.set_continuation(Some(token.into()));
        assert_eq!(first.engine.traverse(replay).unwrap(), result);
        let mut wrong = first.engine.graph().snapshot.clone();
        wrong.revision = "wrong".into();
        assert!(deferred.refresh(&spec, Some(&wrong), 2).is_err());
        assert!(deferred
            .wait(Duration::ZERO)
            .err()
            .unwrap()
            .to_string()
            .contains("publication_refresh_failed"));
        assert!(deferred.wait_for_token(Some(token), Duration::ZERO).is_ok());
        deferred
            .refresh(&spec, Some(&first.engine.graph().snapshot), 2)
            .unwrap();
        assert!(deferred
            .wait_for_token(Some(token), Duration::ZERO)
            .is_err());
        assert_eq!(
            first.engine.graph().snapshot,
            second.engine.graph().snapshot
        );
    }

    #[test]
    fn monitor_adopts_a_new_pointer_without_mutating_inflight_bundle() {
        let (first_dir, first_graph, first_store) = fixture_publication();
        let (second_dir, second_graph, second_store) = fixture_publication();
        let pointer = first_dir.path().join("current.json");
        let publication =
            |graph: PathBuf, store: PathBuf, root: &Path| super::super::WorkspacePublication {
                schema_version: "entmoot.workspace_publication.v1".into(),
                snapshot: crate::load_graph_json(&graph).unwrap().snapshot,
                current: super::super::PublicationLocation {
                    graph,
                    store,
                    source_root: root.into(),
                },
                base: None,
                summary: root.join("summary.json"),
            };
        super::super::write_json(
            &pointer,
            &publication(first_graph.clone(), first_store.clone(), first_dir.path()),
        )
        .unwrap();
        let deferred = DeferredQueryBundle::start_monitored(
            PublicationOpenSpec::graph_store(first_graph, first_store),
            pointer.clone(),
            true,
            2,
        );
        let inflight = deferred.wait(Duration::from_secs(30)).unwrap();
        super::super::write_json(
            &pointer,
            &publication(second_graph, second_store, second_dir.path()),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(current) = deferred.wait(Duration::ZERO) {
                if !Arc::ptr_eq(&current, &inflight) {
                    assert_ne!(current.engine.graph().root, inflight.engine.graph().root);
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "monitor did not adopt the publication"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn deferred_wait_succeeds_for_fixture() {
        let (_dir, graph, store) = fixture_publication();
        let deferred = DeferredQueryBundle::start(PublicationOpenSpec::graph_store(graph, store));
        let bundle = deferred
            .wait(Duration::from_secs(60))
            .expect("open should succeed");
        assert!(!bundle.engine.graph().nodes.is_empty());
        assert!(matches!(deferred.try_get(), Some(Ok(_))));
    }

    #[test]
    fn deferred_timeout_reports_publication_warming() {
        let barrier = Arc::new(Barrier::new(2));
        let release = Arc::clone(&barrier);
        let deferred = DeferredQueryBundle::start_with(move || {
            release.wait();
            Err(QueryServiceError::Unavailable("should not reach".into()))
        });
        let error = deferred
            .wait(Duration::from_millis(50))
            .err()
            .expect("timeout expected");
        assert!(
            error.to_string().contains("publication_warming:"),
            "{error}"
        );
        barrier.wait();
    }

    #[test]
    fn deferred_open_failure_prefix() {
        let deferred = DeferredQueryBundle::start_with(|| {
            Err(QueryServiceError::Unavailable("missing graph".into()))
        });
        let error = deferred
            .wait(Duration::from_secs(5))
            .err()
            .expect("open failed");
        assert!(
            error.to_string().contains("publication_open_failed:"),
            "{error}"
        );
        assert!(error.to_string().contains("missing graph"), "{error}");
    }

    #[test]
    fn sync_open_bundle_matches_publication() {
        let (_dir, graph, store) = fixture_publication();
        let bundle = open_query_bundle_paths(&graph, &store, None, None, false).unwrap();
        assert!(!bundle.engine.graph().nodes.is_empty());
    }

    #[test]
    fn navigation_defers_provider_evidence_until_compatibility() {
        let (dir, graph, store) = fixture_publication();
        let provider = dir.path().join("corrupt-provider.json");
        std::fs::write(&provider, "invalid native publication").unwrap();
        let mut spec = PublicationOpenSpec::graph_store(graph, store);
        spec.providers.push(provider);
        let bundle = open_query_bundle(&spec).unwrap();
        bundle
            .engine
            .find(crate::FindQuery::Subject {
                query: Some("work".into()),
                path: None,
                line: None,
                limit: Some(10),
            })
            .unwrap();
        assert!(bundle.engine.compatibility_request().is_err());
    }

    #[test]
    fn deferred_consumer_stays_pinned_to_opened_publication() {
        let (_dir, graph, store) = fixture_publication();
        let digest = entmoot_contracts::blake3_hex(&std::fs::read(&graph).unwrap());
        let mut spec = PublicationOpenSpec::graph_store(&graph, &store);
        spec.discovery_root = graph.parent().map(Path::to_owned);
        let bundle = open_query_bundle(&spec).unwrap();
        std::fs::write(&graph, "replaced after open").unwrap();
        let request = bundle.engine.compatibility_request().unwrap();
        assert_eq!(request.consumer_publication, digest);
    }

    #[test]
    fn concurrent_compatibility_loads_once_and_preserves_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (_dir, graph, store) = fixture_publication();
        let engine = crate::workspace::open_query_publication(&graph, &store, None, None, false)
            .unwrap()
            .engine;
        let loads = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&loads);
        let engine = engine.with_compatibility_loader(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Err(QueryServiceError::InvalidRequest("corrupt evidence".into()))
        });
        assert_eq!(loads.load(Ordering::SeqCst), 0);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let engine = &engine;
                scope.spawn(move || {
                    assert_eq!(
                        engine.compatibility_request().unwrap_err(),
                        QueryServiceError::InvalidRequest("corrupt evidence".into()),
                    )
                });
            }
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }
}
