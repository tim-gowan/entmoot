//! Query acceleration and continuation budgets.
/// Rebuildable local accelerators. Zero postings and disabled memoization are
/// the reference execution path; neither setting changes query meaning.
#[derive(Debug, Clone, Copy)]
pub struct QueryEngineOptions {
    /// Maximum postings admitted to the bounded disposable substring candidate index.
    /// Overflow or index unavailability falls back to scanning the unindexed suffix.
    pub substring_postings_budget: usize,
    /// Bounded graph-only discovery reuse (256 entries, 8 MiB retained payload).
    pub cache_walk_discovery: bool,
    /// Enable generation-local directed chain contraction for path discovery.
    pub contract_paths: bool,
    /// Maximum total retained shortcut bytes and selector overlays.
    pub shortcut_cache_bytes: usize,
    /// Maximum retained selector-specific shortcut overlays.
    pub shortcut_cache_entries: usize,
    /// Original edges allowed in one complete overlay construction.
    pub max_index_edges: usize,
    /// Maximum retained searches and aggregate/per-search state bytes.
    pub continuation_capacity: usize,
    /// Explicitly persist resumable search state in the knowledge repository.
    pub durable_continuations: bool,
    /// Total byte budget across retained search states.
    pub continuation_bytes: usize,
    /// Maximum bytes retained by one search.
    pub continuation_search_bytes: usize,
    /// Idle expiry; restart and generation retirement also expire tokens.
    pub continuation_idle_seconds: u64,
}

impl Default for QueryEngineOptions {
    fn default() -> Self {
        Self {
            substring_postings_budget: 8 * 1024 * 1024,
            cache_walk_discovery: true,
            contract_paths: true,
            shortcut_cache_bytes: 64 * 1024 * 1024,
            shortcut_cache_entries: 16,
            max_index_edges: 1_000_000,
            continuation_capacity: 64,
            durable_continuations: false,
            continuation_bytes: 64 * 1024 * 1024,
            continuation_search_bytes: 16 * 1024 * 1024,
            continuation_idle_seconds: 300,
        }
    }
}
