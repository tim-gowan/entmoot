//! Graph-only discovery memoization. No claims, absence proofs, or graph writes.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::WalkCounters;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct WalkDiscovery {
    pub counters: WalkCounters,
    pub candidates: Vec<(usize, String, Vec<usize>)>,
    pub total_paths: usize,
    pub incompatible_meets: usize,
}

/// Owned by one immutable QueryEngine generation; clones share it. A new graph
/// gets an empty cache, including after edge additions (negative frontiers).
pub(super) type WalkCache = BoundedCache<WalkDiscovery>;

pub(super) trait RetainedSize {
    fn retained_bytes(&self) -> usize;
}

pub(super) struct BoundedCache<T> {
    entries: Mutex<VecDeque<(String, Arc<T>, usize)>>,
    max_entries: usize,
    max_bytes: usize,
}

impl<T> Default for BoundedCache<T> {
    fn default() -> Self {
        Self::new(256, 8 * 1024 * 1024)
    }
}

impl<T> BoundedCache<T> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            max_entries,
            max_bytes,
        }
    }

    pub fn retained_bytes(&self) -> usize {
        self.entries
            .lock()
            .map_or(0, |entries| entries.iter().map(|entry| entry.2).sum())
    }

    pub fn get(&self, key: &str) -> Option<Arc<T>> {
        self.entries
            .lock()
            .ok()?
            .iter()
            .find(|entry| entry.0 == key)
            .map(|entry| entry.1.clone())
    }
}

impl<T: RetainedSize> BoundedCache<T> {
    pub fn insert(&self, key: String, value: Arc<T>) -> bool {
        // Conservative retained-capacity accounting. FIFO avoids write locking
        // or copying paths on a hit. Computation and hydration are outside lock.
        let weight = key
            .capacity()
            .saturating_add(value.retained_bytes())
            .saturating_add(128);
        if self.max_entries == 0 || weight > self.max_bytes {
            return false;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return false;
        };
        if entries.iter().any(|entry| entry.0 == key) {
            return true;
        }
        let mut bytes: usize = entries.iter().map(|entry| entry.2).sum();
        while entries.len() >= self.max_entries || bytes.saturating_add(weight) > self.max_bytes {
            let Some(position) = entries
                .iter()
                .position(|entry| Arc::strong_count(&entry.1) == 1)
            else {
                return false;
            };
            let Some((_, _, removed)) = entries.remove(position) else {
                return false;
            };
            bytes = bytes.saturating_sub(removed);
        }
        entries.push_back((key, value, weight));
        true
    }
}

impl RetainedSize for WalkDiscovery {
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.candidates.capacity() * std::mem::size_of::<(usize, String, Vec<usize>)>()
            + self
                .candidates
                .iter()
                .map(|(_, meet, edges)| {
                    meet.capacity() + edges.capacity() * std::mem::size_of::<usize>()
                })
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty() -> Arc<WalkDiscovery> {
        Arc::new(WalkDiscovery {
            counters: WalkCounters::default(),
            candidates: vec![],
            total_paths: 0,
            incompatible_meets: 0,
        })
    }
    #[test]
    fn bounded_cache_evicts_and_refuses_oversized_keys() {
        let cache = WalkCache::default();
        for n in 0..257 {
            cache.insert(n.to_string(), empty());
        }
        assert!(cache.get("0").is_none());
        assert!(cache.get("256").is_some());
        let key = "x".repeat(8 * 1024 * 1024);
        cache.insert(key.clone(), empty());
        assert!(cache.get(&key).is_none());
        assert_eq!(cache.entries.lock().unwrap().len(), 256);
    }
}
