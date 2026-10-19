//! Bounded, disposable search sessions. Tokens never authorize knowledge access.
use super::walk_cache::RetainedSize;
use super::*;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime};

static NEXT_ENGINE: AtomicU64 = AtomicU64::new(1);

pub(super) struct Searches {
    prefix: String,
    entries: Mutex<BTreeMap<u64, Arc<Entry>>>,
    next: AtomicU64,
    bytes: Arc<AtomicUsize>,
    count: Arc<AtomicUsize>,
    options: QueryEngineOptions,
}

struct Entry {
    state: Mutex<Session>,
    bytes: AtomicUsize,
    aggregate: Arc<AtomicUsize>,
    count: Arc<AtomicUsize>,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.aggregate
            .fetch_sub(self.bytes.load(Ordering::Relaxed), Ordering::AcqRel);
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Entry {
    fn reserve(&self, needed: usize, limit: usize) -> bool {
        let old = self.bytes.load(Ordering::Relaxed);
        if needed < old {
            self.aggregate.fetch_sub(old - needed, Ordering::AcqRel);
        } else if self
            .aggregate
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |total| {
                let total = total.saturating_add(needed - old);
                (total <= limit).then_some(total)
            })
            .is_err()
        {
            return false;
        }
        self.bytes.store(needed, Ordering::Release);
        true
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Session {
    semantic: String,
    visibility: entmoot_contracts::ContentDigest,
    search: Search,
    slices: Vec<(BoundedWalkBudget, Arc<QueryResult>, usize)>,
    #[serde(skip, default = "Instant::now")]
    last_used: Instant,
    done: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum Search {
    Neighborhood(Box<neighborhood::NeighborhoodSearch>),
    Path {
        search: Box<walk::PathSearch>,
        paths: Option<walk_cache::WalkDiscovery>,
        offset: usize,
    },
}

impl Search {
    fn retained_bytes(&self) -> usize {
        match self {
            Self::Neighborhood(search) => search.retained_bytes(),
            Self::Path { search, paths, .. } => {
                search.retained_bytes() + paths.as_ref().map_or(0, RetainedSize::retained_bytes)
            }
        }
    }
}

fn unavailable(reason: &str) -> QueryError {
    QueryError::Unavailable(format!("search continuation unavailable: {reason}; retry an in-use token, or explicitly start a new resumable search with narrower limits"))
}

impl Searches {
    pub fn new(options: QueryEngineOptions) -> Self {
        let nonce = format!(
            "{:?}:{}:{}",
            SystemTime::now(),
            std::process::id(),
            NEXT_ENGINE.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            prefix: format!("et1.{}", blake3::hash(nonce.as_bytes()).to_hex()),
            entries: Default::default(),
            next: AtomicU64::new(1),
            bytes: Arc::new(AtomicUsize::new(0)),
            count: Arc::new(AtomicUsize::new(0)),
            options,
        }
    }
    fn token(&self, id: u64, slice: usize) -> String {
        format!("{}.{id:x}.{slice:x}", self.prefix)
    }
    fn parse(&self, token: &str) -> Result<(u64, usize), QueryError> {
        let suffix = token
            .strip_prefix(&format!("{}.", self.prefix))
            .ok_or_else(|| {
                unavailable("expired, restarted, or different publication generation")
            })?;
        let (id, slice) = suffix
            .split_once('.')
            .ok_or_else(|| unavailable("invalid token"))?;
        Ok((
            u64::from_str_radix(id, 16).map_err(|_| unavailable("invalid token"))?,
            usize::from_str_radix(slice, 16).map_err(|_| unavailable("invalid token"))?,
        ))
    }
    fn lookup(&self, id: u64) -> Result<Arc<Entry>, QueryError> {
        self.entries
            .lock()
            .map_err(|_| unavailable("search registry failed"))?
            .get(&id)
            .cloned()
            .ok_or_else(|| unavailable("expired or evicted token"))
    }
    fn insert(&self, state: Session) -> Result<(u64, Arc<Entry>), QueryError> {
        let retained = state
            .search
            .retained_bytes()
            .saturating_add(
                state
                    .slices
                    .iter()
                    .map(|(_, _, bytes)| *bytes)
                    .sum::<usize>(),
            )
            .saturating_add(state.semantic.capacity())
            .saturating_add(1024);
        if retained > self.options.continuation_search_bytes {
            return Err(unavailable(
                "initial selection exceeds per-search retained memory",
            ));
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| unavailable("search registry failed"))?;
        let idle = Duration::from_secs(self.options.continuation_idle_seconds);
        entries.retain(|_, entry| {
            Arc::strong_count(entry) > 1
                || entry
                    .state
                    .try_lock()
                    .is_ok_and(|s| s.last_used.elapsed() < idle)
        });
        if self.options.continuation_capacity == 0 {
            return Err(unavailable("resumable searches are disabled"));
        }
        if entries.len() >= self.options.continuation_capacity {
            let oldest = entries
                .iter()
                .filter(|(_, e)| Arc::strong_count(e) == 1)
                .filter_map(|(id, e)| e.state.try_lock().ok().map(|s| (*id, s.last_used)))
                .min_by_key(|(_, used)| *used)
                .map(|(id, _)| id);
            if let Some(id) = oldest {
                entries.remove(&id);
            } else {
                return Err(unavailable("all retained searches are in use"));
            }
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < self.options.continuation_capacity).then_some(count + 1)
            })
            .map_err(|_| unavailable("retained search capacity across generations is exhausted"))?;
        let entry = Arc::new(Entry {
            state: Mutex::new(state),
            bytes: AtomicUsize::new(0),
            aggregate: self.bytes.clone(),
            count: self.count.clone(),
        });
        if !entry.reserve(retained, self.options.continuation_bytes) {
            return Err(unavailable(
                "initial selection exceeds aggregate retained memory",
            ));
        }
        entries.insert(id, entry.clone());
        Ok((id, entry))
    }
    fn remove(&self, id: u64) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&id);
        }
    }
}

fn semantic_key(request: &TraverseQuery) -> Result<String, QueryError> {
    let mut request = request.clone();
    request.set_continuation(None);
    let budget = match &mut request {
        TraverseQuery::Path { budget, .. } | TraverseQuery::Neighborhood { budget, .. } => budget,
    };
    budget.max_expansions = 1;
    budget.max_examined_edges = 1;
    budget.max_results = 1;
    Ok(blake3::hash(&serde_json::to_vec(&request)?)
        .to_hex()
        .to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DurableSession {
    prefix: String,
    id: u64,
    session: Session,
}

fn session_key(token: &str) -> Result<&str, QueryError> {
    token
        .rsplit_once('.')
        .map(|(key, _)| key)
        .ok_or_else(|| unavailable("malformed token"))
}

fn slice_token(result: &QueryResult) -> Option<&str> {
    use entmoot_contracts::query::{QueryPayload, TraversePayload};
    match &result.payload {
        QueryPayload::Traverse(TraversePayload::Path(path)) => {
            path.search_receipt.slice_token.as_deref()
        }
        QueryPayload::Traverse(TraversePayload::Neighborhood(page)) => {
            page.search_receipt.slice_token.as_deref()
        }
        _ => None,
    }
}

impl QueryEngine {
    /// Enable repository-backed checkpoints for explicitly resumable queries.
    /// This changes only operational metadata, never knowledge publication.
    pub fn with_durable_continuations(mut self) -> Self {
        self.options.durable_continuations = true;
        self
    }
    /// Configure explicit retention for standalone durable searches. Zero disables
    /// retention; frontends should require a positive allowance when enabled.
    pub fn with_durable_continuation_lifetime(mut self, idle_seconds: u64) -> Self {
        self.options.durable_continuations = true;
        self.options.continuation_idle_seconds = idle_seconds;
        self.continuations = Arc::new(Searches::new(self.options));
        self
    }
    fn durable_traverse(&self, request: TraverseQuery) -> Result<QueryResult, QueryError> {
        let map_error = |e: crate::OperationError| unavailable(&e.to_string());
        let generation = self.publication_generation_digest(&self.repository()?)?;
        let now = crate::collaboration::now_ms().map_err(map_error)?;
        let mut repository = SqliteKnowledgeRepository::open(&self.store_path)?;
        let mut engine = self.clone();
        let mut previous_sequence = 0;
        if let Some(token) = request.continuation() {
            let (sequence, body) = repository
                .query_checkpoint(session_key(token)?, generation.as_str(), now)
                .map_err(map_error)?
                .ok_or_else(|| unavailable("durable checkpoint missing or expired"))?;
            previous_sequence = sequence;
            let state: DurableSession = serde_json::from_slice(&body)?;
            if state.session.slices.len() != sequence {
                return Err(unavailable("checkpoint sequence mismatch"));
            }
            let mut registry = Searches::new(self.options);
            registry.prefix = state.prefix;
            registry.bytes = self.continuations.bytes.clone();
            registry.count = self.continuations.count.clone();
            let (allocated, entry) = registry.insert(state.session)?;
            {
                let mut entries = registry
                    .entries
                    .lock()
                    .map_err(|_| unavailable("registry failed"))?;
                entries.remove(&allocated);
                entries.insert(state.id, entry);
            }
            registry
                .next
                .store(state.id.saturating_add(1), Ordering::Relaxed);
            registry.parse(token)?;
            engine.continuations = Arc::new(registry);
        }
        let result = engine.resume_traverse_memory(request)?;
        let Some(token) = slice_token(&result) else {
            return Ok(result);
        };
        let (id, _) = engine.continuations.parse(token)?;
        let entry = engine.continuations.lookup(id)?;
        // Durable state is restored from the repository on every call. Keep its
        // temporary memory charged until this call ends, including failed saves.
        engine.continuations.remove(id);
        let session = entry
            .state
            .lock()
            .map_err(|_| unavailable("registry failed"))?;
        // Replay has no advancement and must not race a writer by rewriting state.
        if session.slices.len() == previous_sequence {
            return Ok(result);
        }
        #[derive(serde::Serialize)]
        struct BorrowedSession<'a> {
            prefix: &'a str,
            id: u64,
            session: &'a Session,
        }
        let body = serde_json::to_vec(&BorrowedSession {
            prefix: &engine.continuations.prefix,
            id,
            session: &session,
        })?;
        if body.len() > self.options.continuation_search_bytes {
            return Err(unavailable(
                "durable checkpoint exceeds per-search byte allowance",
            ));
        }
        let expires =
            now.saturating_add(self.options.continuation_idle_seconds.saturating_mul(1000));
        repository
            .save_query_checkpoint(
                session_key(token)?,
                generation.as_str(),
                previous_sequence,
                session.slices.len(),
                expires,
                &body,
                self.options.continuation_capacity,
                self.options.continuation_bytes,
            )
            .map_err(map_error)?;
        Ok(result)
    }
}

impl QueryEngine {
    /// Retained disposable cache/search bytes. Search accounting is shared across
    /// retained generations; overlays and discovery memoization belong to this one.
    pub fn accelerator_retained_bytes(&self) -> usize {
        self.shortcuts.retained_bytes()
            + self.walk_cache.as_ref().map_or(0, |c| c.retained_bytes())
            + self.continuations.bytes.load(Ordering::Acquire)
    }

    pub(crate) fn share_continuation_limits(&mut self, previous: &QueryEngine) {
        let mut registry = Searches::new(self.options);
        registry.bytes = previous.continuations.bytes.clone();
        registry.count = previous.continuations.count.clone();
        self.continuations = Arc::new(registry);
    }

    pub(crate) fn expire_continuations(&self) {
        let idle = Duration::from_secs(self.options.continuation_idle_seconds);
        if let Ok(mut entries) = self.continuations.entries.lock() {
            entries.retain(|_, entry| {
                Arc::strong_count(entry) > 1
                    || entry
                        .state
                        .try_lock()
                        .is_ok_and(|s| s.last_used.elapsed() < idle)
            });
        }
    }
    /// Identify the generation owning an opaque continuation, without advancing it.
    pub fn owns_continuation(&self, token: &str) -> bool {
        if self.options.durable_continuations {
            return (|| {
                let repository = self.repository().ok()?;
                let generation = self.publication_generation_digest(&repository).ok()?;
                repository
                    .query_checkpoint(
                        session_key(token).ok()?,
                        generation.as_str(),
                        crate::collaboration::now_ms().ok()?,
                    )
                    .ok()?
            })()
            .is_some();
        }
        self.continuations.parse(token).is_ok()
    }

    pub(super) fn resume_traverse(
        &self,
        request: TraverseQuery,
    ) -> Result<QueryResult, QueryError> {
        if self.options.durable_continuations {
            return self.durable_traverse(request);
        }
        self.resume_traverse_memory(request)
    }

    fn resume_traverse_memory(&self, request: TraverseQuery) -> Result<QueryResult, QueryError> {
        let started = Instant::now();
        let budget = validate_walk_budget(request.budget())?;
        let semantic = semantic_key(&request)?;
        let repository = self.repository()?;
        let visibility = repository.read_state_digest()?;
        let registry = &self.continuations;
        let (id, slice, entry) = if let Some(token) = request.continuation() {
            let (id, slice) = registry.parse(token)?;
            (id, slice, registry.lookup(id)?)
        } else {
            let search = match &request {
                TraverseQuery::Path {
                    start,
                    target,
                    predicates,
                    evidence_filter,
                    prune,
                    ..
                } => {
                    let roots = self.walk_endpoint_keys(start);
                    let targets = self.walk_endpoint_keys(target);
                    if roots.is_empty() || targets.is_empty() {
                        return self.finish_result(self.bounded_walk_filtered(
                            start,
                            target,
                            predicates,
                            budget,
                            *evidence_filter,
                            prune,
                        )?);
                    }
                    if roots
                        .len()
                        .saturating_add(targets.len())
                        .saturating_mul(1024)
                        > self.options.continuation_search_bytes
                    {
                        return Err(unavailable("endpoint selection exceeds retained memory"));
                    }
                    Search::Path {
                        search: Box::new(walk::PathSearch::with_admission(
                            self,
                            &roots,
                            &targets,
                            parse_walk_predicates(predicates)?,
                            start == target,
                            admission::EdgeAdmission::new(self, prune, *evidence_filter)?,
                        )),
                        paths: None,
                        offset: 0,
                    }
                }
                TraverseQuery::Neighborhood { .. } => Search::Neighborhood(Box::new(
                    neighborhood::NeighborhoodSearch::new(self, &request)?,
                )),
            };
            let (id, entry) = registry.insert(Session {
                semantic: semantic.clone(),
                visibility: visibility.clone(),
                search,
                slices: vec![],
                last_used: Instant::now(),
                done: false,
            })?;
            (id, 0, entry)
        };
        // One caller advances a session. A concurrent caller receives an explicit
        // retryable unavailable outcome, never a second advancement.
        let mut session = entry
            .state
            .try_lock()
            .map_err(|_| unavailable("token is in use"))?;
        if session.last_used.elapsed()
            >= Duration::from_secs(self.options.continuation_idle_seconds)
        {
            registry.remove(id);
            return Err(unavailable("idle expiry"));
        }
        if session.semantic != semantic || session.visibility != visibility {
            return Err(unavailable("request or evidence visibility changed"));
        }
        session.last_used = Instant::now();
        if let Some((prior_budget, result, _)) = session.slices.get(slice) {
            if *prior_budget != budget {
                return Err(unavailable("retry must keep its execution-slice budget"));
            }
            return Ok(result.as_ref().clone());
        }
        if slice != session.slices.len() || session.done {
            return Err(unavailable("token does not identify the next slice"));
        }
        let limit = self.options.continuation_search_bytes.min(
            self.options
                .continuation_bytes
                .saturating_sub(registry.bytes.load(Ordering::Acquire))
                .saturating_add(entry.bytes.load(Ordering::Acquire)),
        );
        if !entry.reserve(limit, self.options.continuation_bytes) {
            return Err(unavailable("aggregate retained-memory allowance is busy"));
        }
        let replay_bytes: usize = session
            .slices
            .iter()
            .map(|(_, _, bytes)| bytes)
            .sum::<usize>()
            + session.semantic.capacity()
            + 1024;
        let advanced = (|| {
            let (mut value, done, state_bytes) = match (&mut session.search, &request) {
                (
                    Search::Path {
                        search,
                        paths,
                        offset,
                    },
                    TraverseQuery::Path { start, target, .. },
                ) => {
                    let before = search.counters.clone();
                    if !search.advance_with_limit(
                        self,
                        budget,
                        limit.saturating_sub(replay_bytes),
                    )? {
                        return Err(unavailable("per-search memory allowance exhausted"));
                    }
                    if search.done && paths.is_none() {
                        *paths = Some(
                            search
                                .discovery_with_limit(
                                    self,
                                    BoundedWalkBudget {
                                        max_results: usize::MAX,
                                        ..budget
                                    },
                                    limit
                                        .saturating_sub(replay_bytes)
                                        .saturating_sub(search.retained_bytes()),
                                )
                                .ok_or_else(|| {
                                    unavailable("witness reconstruction exceeds retained memory")
                                })?,
                        );
                    }
                    let end = paths.as_ref().map_or(0, |p| {
                        offset
                            .saturating_add(budget.max_results)
                            .min(p.candidates.len())
                    });
                    let discovery = walk_cache::WalkDiscovery {
                        counters: search.counters.clone(),
                        candidates: paths
                            .as_ref()
                            .map_or_else(Vec::new, |p| p.candidates[*offset..end].to_vec()),
                        total_paths: paths.as_ref().map_or(0, |p| p.total_paths),
                        incompatible_meets: paths.as_ref().map_or(0, |p| p.incompatible_meets),
                    };
                    let mut value =
                        self.walk_result((start, target), budget, &discovery, &repository, false)?;
                    value["bounds"]["offset"] = json!(*offset);
                    *offset = end;
                    let done =
                        search.done && paths.as_ref().is_none_or(|p| end == p.candidates.len());
                    let truncated = !done || search.counters.fanout_truncated;
                    value["bounds"]["truncated"] = truncated.into();
                    value["status"] = if truncated {
                        "truncated"
                    } else if discovery.candidates.is_empty() {
                        "unknown"
                    } else {
                        "known"
                    }
                    .into();
                    let receipt = &mut value["search_receipt"];
                    receipt["pruned_edges"] =
                        json!(search.counters.pruned_edges - before.pruned_edges);
                    receipt["evidence_filtered_edges"] = json!(
                        search.counters.evidence_filtered_edges - before.evidence_filtered_edges
                    );
                    receipt["admission_claims_examined"] = json!(
                        search.counters.admission_claims_examined
                            - before.admission_claims_examined
                    );
                    receipt["expanded_nodes"] =
                        json!(search.counters.expanded_nodes - before.expanded_nodes);
                    receipt["examined_edges"] =
                        json!(search.counters.examined_edges - before.examined_edges);
                    receipt["examined_shortcuts"] =
                        json!(search.counters.examined_shortcuts - before.examined_shortcuts);
                    receipt["index_examined_edges"] = json!(if slice == 0 {
                        search.counters.index_examined_edges
                    } else {
                        0
                    });
                    receipt["index_micros"] = json!(if slice == 0 {
                        search.counters.index_micros
                    } else {
                        0
                    });
                    receipt["cumulative_expanded_nodes"] = json!(search.counters.expanded_nodes);
                    receipt["cumulative_examined_edges"] = json!(search.counters.examined_edges);
                    receipt["cumulative_examined_shortcuts"] =
                        json!(search.counters.examined_shortcuts);
                    (
                        value,
                        done,
                        search.retained_bytes()
                            + paths.as_ref().map_or(0, RetainedSize::retained_bytes),
                    )
                }
                (Search::Neighborhood(search), TraverseQuery::Neighborhood { .. }) => {
                    let value = search.advance(
                        self,
                        &repository,
                        budget,
                        limit.saturating_sub(replay_bytes),
                    )?;
                    (value, search.done, search.retained_bytes())
                }
                _ => return Err(unavailable("incompatible search kind")),
            };
            if !done {
                value["bounds"]["truncated"] = true.into();
                value["status"] = "truncated".into();
            }
            value["search_receipt"]["slice_token"] = registry.token(id, slice).into();
            value["search_receipt"]["continuation"] = if done {
                Value::Null
            } else {
                registry.token(id, slice + 1).into()
            };
            value["search_receipt"]["elapsed_micros"] =
                json!(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
            let result = self.finish_result(value)?;
            if self.repository()?.read_state_digest()? != visibility {
                return Err(unavailable("evidence changed during the slice"));
            }
            // DTO capacity plus serde/map overhead; payload retention is separate
            // from the frontier and charged before admitting another slice.
            let result_bytes = super::output::byte_len(&result)?
                .saturating_mul(8)
                .saturating_add(1024);
            let retained = state_bytes
                .saturating_add(replay_bytes)
                .saturating_add(result_bytes);
            if retained > limit {
                return Err(unavailable(
                    "result replay exceeds per-search retained-memory allowance",
                ));
            }
            session.done = done;
            session
                .slices
                .push((budget, Arc::new(result.clone()), result_bytes));
            entry.reserve(retained, self.options.continuation_bytes);
            Ok(result)
        })();
        if advanced.is_err() {
            registry.remove(id);
        }
        advanced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_frontier_survives_a_fresh_engine_every_slice() {
        let options = QueryEngineOptions {
            durable_continuations: true,
            contract_paths: false,
            ..Default::default()
        };
        let (_dir, mut engine, mut request) = fixture(options);
        let first = engine.traverse(request.clone()).unwrap();
        let token = receipt(&first).slice_token.clone().unwrap();
        let mut replay = request.clone();
        replay.set_continuation(Some(token));
        let reloaded = QueryEngine::with_options(
            engine.graph.as_ref().clone(),
            engine.store_path.clone(),
            options,
        );
        let again = reloaded.traverse(replay).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(again).unwrap()
        );
        request.set_continuation(receipt(&first).continuation.clone());
        let mut completed = false;
        for _ in 0..40 {
            engine = QueryEngine::with_options(
                engine.graph.as_ref().clone(),
                engine.store_path.clone(),
                options,
            );
            assert!(engine.owns_continuation(request.continuation().unwrap()));
            let result = engine.traverse(request.clone()).unwrap();
            request.set_continuation(receipt(&result).continuation.clone());
            if request.continuation().is_none() {
                assert!(serde_json::to_string(&result).unwrap().contains("known"));
                completed = true;
                break;
            }
        }
        assert!(
            completed,
            "bounded slices must make progress after restarts"
        );
    }

    #[test]
    fn durable_concurrent_retries_and_corruption_never_publish_an_extra_slice() {
        let options = QueryEngineOptions {
            durable_continuations: true,
            contract_paths: false,
            ..Default::default()
        };
        let (_dir, engine, request) = fixture(options);
        let first = engine.traverse(request.clone()).unwrap();
        assert_eq!(engine.continuations.bytes.load(Ordering::Acquire), 0);
        let mut next = request;
        next.set_continuation(receipt(&first).continuation.clone());
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let run = || {
                let fresh = QueryEngine::with_options(
                    engine.graph.as_ref().clone(),
                    &engine.store_path,
                    options,
                );
                barrier.wait();
                fresh.traverse(next.clone())
            };
            let a = scope.spawn(run);
            let b = scope.spawn(run);
            vec![a.join().unwrap(), b.join().unwrap()]
        });
        let replay = engine.traverse(next.clone()).unwrap();
        assert_eq!(receipt(&replay).cumulative_expanded_nodes, Some(2));
        assert!(results.iter().any(Result::is_ok));
        for result in results {
            match result {
                Ok(value) => assert_eq!(value, replay),
                Err(QueryError::Unavailable(_)) => {}
                other => panic!("{other:?}"),
            }
        }
        let db = rusqlite::Connection::open(&engine.store_path).unwrap();
        db.execute("UPDATE local_query_checkpoints SET body=x'7b7d'", [])
            .unwrap();
        assert!(!engine.owns_continuation(next.continuation().unwrap()));
        assert!(engine.traverse(next).is_err());
    }

    #[test]
    fn durable_neighborhood_restarts_match_uninterrupted_results() {
        let options = QueryEngineOptions {
            durable_continuations: true,
            contract_paths: true,
            ..Default::default()
        };
        let (_dir, engine, path) = fixture(options);
        let TraverseQuery::Path { start, .. } = path else {
            panic!("fixture path")
        };
        let request: TraverseQuery = serde_json::from_value(json!({"kind":"neighborhood","start":start,"predicates":["calls"],"direction":"callees","evidence_filter":"any","resumable":true,"budget":{"max_hops":5,"max_expansions":1,"max_examined_edges":1,"max_edges_per_node":0,"max_results":1}})).unwrap();
        let mut next = request.clone();
        let mut sliced = std::collections::BTreeSet::new();
        let mut done = false;
        for _ in 0..40 {
            let fresh = QueryEngine::with_options(
                engine.graph.as_ref().clone(),
                &engine.store_path,
                options,
            );
            let page = fresh.traverse(next.clone()).unwrap();
            let entmoot_contracts::query::QueryPayload::Traverse(
                entmoot_contracts::query::TraversePayload::Neighborhood(page),
            ) = page.payload
            else {
                panic!("neighborhood")
            };
            for item in page.items {
                sliced.insert(serde_json::to_string(&item.subject).unwrap());
            }
            next.set_continuation(page.search_receipt.continuation);
            if next.continuation().is_none() {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(
            sliced.len(),
            3,
            "d has exactly a, b, c as represented transitive callees"
        );
        let mut full = serde_json::to_value(request).unwrap();
        full["resumable"] = json!(false);
        full["budget"]["max_expansions"] = json!(100);
        full["budget"]["max_examined_edges"] = json!(100);
        full["budget"]["max_results"] = json!(20);
        let whole = engine
            .traverse(serde_json::from_value(full).unwrap())
            .unwrap();
        let entmoot_contracts::query::QueryPayload::Traverse(
            entmoot_contracts::query::TraversePayload::Neighborhood(page),
        ) = whole.payload
        else {
            panic!("neighborhood")
        };
        assert_eq!(
            sliced,
            page.items
                .iter()
                .map(|item| serde_json::to_string(&item.subject).unwrap())
                .collect()
        );
    }

    fn fixture(options: QueryEngineOptions) -> (tempfile::TempDir, QueryEngine, TraverseQuery) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            "fn a(){} fn b(){a();} fn c(){b();} fn d(){c();}",
        )
        .unwrap();
        let graph =
            crate::build_topology_from_repo_with_options(dir.path(), Default::default()).unwrap();
        let node = |name| {
            graph
                .nodes
                .iter()
                .find(|n| n.kind == NodeKind::Callable && n.label == name)
                .unwrap()
                .id
                .clone()
        };
        let request=serde_json::from_value(json!({"kind":"path","start":node("d"),"target":node("a"),"predicates":["calls"],"resumable":true,"budget":{"max_hops":5,"max_expansions":1,"max_examined_edges":1,"max_edges_per_node":0,"max_results":1}})).unwrap();
        let store = dir.path().join("knowledge.sqlite3");
        SqliteKnowledgeRepository::open(&store)
            .unwrap()
            .replace_all(&crate::KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        (
            dir,
            QueryEngine::with_options(graph, store, options),
            request,
        )
    }
    fn receipt(result: &QueryResult) -> &entmoot_contracts::query::TraverseSearchReceipt {
        let entmoot_contracts::query::QueryPayload::Traverse(
            entmoot_contracts::query::TraversePayload::Path(path),
        ) = &result.payload
        else {
            panic!("path fixture");
        };
        &path.search_receipt
    }

    #[test]
    fn continuation_fences_request_evidence_restart_expiry_and_capacity() {
        let options = QueryEngineOptions {
            contract_paths: false,
            continuation_capacity: 1,
            ..Default::default()
        };
        let (_dir, engine, request) = fixture(options);
        let first = engine.traverse(request.clone()).unwrap();
        let mut next = request.clone();
        next.set_continuation(receipt(&first).continuation.clone());
        assert!(next.continuation().is_some());
        let mut changed = next.clone();
        if let TraverseQuery::Path { budget, .. } = &mut changed {
            budget.max_hops = 4;
        }
        assert!(matches!(
            engine.traverse(changed),
            Err(QueryError::Unavailable(_))
        ));
        let restarted =
            QueryEngine::with_options(engine.graph.as_ref().clone(), &engine.store_path, options);
        assert!(matches!(
            restarted.traverse(next.clone()),
            Err(QueryError::Unavailable(_))
        ));
        let (id, _) = engine
            .continuations
            .parse(next.continuation().unwrap())
            .unwrap();
        engine
            .continuations
            .lookup(id)
            .unwrap()
            .state
            .lock()
            .unwrap()
            .last_used = Instant::now() - Duration::from_secs(301);
        assert!(matches!(
            engine.traverse(next),
            Err(QueryError::Unavailable(_))
        ));
        let first = engine.traverse(request.clone()).unwrap();
        let mut next = request.clone();
        next.set_continuation(receipt(&first).continuation.clone());
        engine.traverse(request.clone()).unwrap();
        assert!(matches!(
            engine.traverse(next),
            Err(QueryError::Unavailable(_))
        ));
        let first = engine.traverse(request.clone()).unwrap();
        let mut next = request;
        next.set_continuation(receipt(&first).continuation.clone());
        SqliteKnowledgeRepository::open(&engine.store_path)
            .unwrap()
            .replace_all(&crate::KnowledgeStore::default())
            .unwrap();
        assert!(matches!(
            engine.traverse(next),
            Err(QueryError::Unavailable(_))
        ));
    }

    #[test]
    fn concurrent_retries_can_advance_only_once() {
        let (_dir, engine, request) = fixture(QueryEngineOptions {
            contract_paths: false,
            ..Default::default()
        });
        let first = engine.traverse(request.clone()).unwrap();
        let mut next = request;
        next.set_continuation(receipt(&first).continuation.clone());
        let barrier = std::sync::Barrier::new(3);
        let results = std::thread::scope(|scope| {
            let left = scope.spawn(|| {
                barrier.wait();
                engine.traverse(next.clone())
            });
            let right = scope.spawn(|| {
                barrier.wait();
                engine.traverse(next.clone())
            });
            barrier.wait();
            vec![left.join().unwrap(), right.join().unwrap()]
        });
        let replay = engine.traverse(next).unwrap();
        let mut successful = 0;
        for result in results {
            match result {
                Ok(result) => {
                    assert_eq!(result, replay);
                    successful += 1;
                }
                Err(QueryError::Unavailable(_)) => {}
                Err(error) => panic!("unexpected {error}"),
            }
        }
        assert!(successful >= 1);
        assert_eq!(receipt(&replay).cumulative_expanded_nodes, Some(2));
    }

    #[test]
    fn retained_memory_is_bounded_across_generations() {
        let (_dir, first, request) = fixture(QueryEngineOptions {
            contract_paths: false,
            continuation_capacity: 1,
            ..Default::default()
        });
        first.traverse(request.clone()).unwrap();
        let mut second = QueryEngine::with_options(
            first.graph.as_ref().clone(),
            &first.store_path,
            first.options,
        );
        second.share_continuation_limits(&first);
        assert!(matches!(
            second.traverse(request),
            Err(QueryError::Unavailable(_))
        ));
        assert!(
            first.continuations.bytes.load(Ordering::Acquire) <= first.options.continuation_bytes
        );
    }
}
