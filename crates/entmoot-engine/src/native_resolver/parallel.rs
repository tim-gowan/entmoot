//! Bounded compiler invocations. Nested Go module work shares the family pool.
use rayon::prelude::*;
use std::sync::LazyLock;

// Compilers have their own worker pools and large working sets. Keep the host
// cap conservative; failure to allocate workers falls back to sequential work.
static POOL: LazyLock<Option<rayon::ThreadPool>> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .stack_size(8 * 1024 * 1024)
        .thread_name(|index| format!("entmoot-producer-{index}"))
        .build()
        .ok()
});

/// Indexed collection preserves input order even when producers finish out of order.
pub(crate) fn ordered_map<T: Sync, R: Send>(
    items: &[T],
    run: impl Fn(&T) -> R + Sync + Send,
) -> Vec<R> {
    match &*POOL {
        Some(pool) => pool.install(|| items.par_iter().map(run).collect()),
        None => items.iter().map(run).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn nested_work_overlaps_with_one_shared_cap_and_stable_results() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let values = ordered_map(&[0, 1, 2], |family| {
            ordered_map(&[0, 1, 2], |module| {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                active.fetch_sub(1, Ordering::SeqCst);
                family * 10 + module
            })
        });
        assert_eq!(values, [vec![0, 1, 2], vec![10, 11, 12], vec![20, 21, 22]]);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }
}
