//! Disposable bounded candidate postings; every candidate is verified against source fields.
//!
//! SQLite FTS5 was evaluated against repository declaration text before replacement:
//! its build and warm-query overhead did not justify replacing this bounded projection.
//! This is candidate pruning only; std::str::contains owns substring semantics.
#[derive(Debug, Default)]
pub(super) struct SubstringIndex {
    postings: Vec<Vec<u32>>,
    indexed: usize,
    len: usize,
}
impl SubstringIndex {
    pub(super) fn unindexed(len: usize) -> Self {
        Self {
            postings: vec![Vec::new(); 4096],
            indexed: 0,
            len,
        }
    }
    /// Fixed bucket count bounds dictionary overhead. Hash collisions only add
    /// candidates. The unindexed suffix is always scanned if postings run out.
    pub(super) fn new(texts: impl Iterator<Item = String>, budget: usize) -> Self {
        let mut index = Self {
            postings: vec![Vec::new(); 4096],
            indexed: 0,
            len: 0,
        };
        let mut used = 0;
        let mut exhausted = false;
        for text in texts {
            let node = index.len;
            index.len += 1;
            if exhausted {
                continue;
            }
            let Ok(node) = u32::try_from(node) else {
                exhausted = true;
                continue;
            };
            let mut buckets: Vec<_> = grams(&text).collect();
            buckets.sort_unstable();
            buckets.dedup();
            if buckets.len() > budget.saturating_sub(used) {
                exhausted = true;
                continue;
            }
            used += buckets.len();
            for bucket in buckets {
                index.postings[bucket].push(node);
            }
            index.indexed += 1;
        }
        for posting in &mut index.postings {
            posting.shrink_to_fit();
        }
        index
    }
    pub(super) fn candidates(&self, query: &str) -> impl Iterator<Item = usize> + '_ {
        let rarest = grams(query)
            .map(|g| self.postings[g].as_slice())
            .min_by_key(|p| p.len());
        let tail = if rarest.is_some() { self.indexed } else { 0 };
        rarest
            .unwrap_or_default()
            .iter()
            .map(|node| *node as usize)
            .chain(tail..self.len)
    }
}

fn grams(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.as_bytes().windows(3).map(|w| {
        (u32::from_be_bytes([0, w[0], w[1], w[2]]).wrapping_mul(0x9e37_79b1) >> 20) as usize
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postings_reduce_work_without_changing_substring_results() {
        let texts: Vec<_> = (0..1000)
            .map(|n| format!("file{n}::ordinary"))
            .chain([
                "src/rare.rs needle_callable".into(),
                "ångström needle_callable".into(),
                "préfixe😀🚀🎯界面".into(),
                "a\"b [*?]".into(),
                "embedded\0needle_after_nul".into(),
            ])
            .collect();
        let index = SubstringIndex::new(texts.iter().cloned(), 1_000_000);
        let candidates: Vec<_> = index.candidates("needle").collect();
        assert!(candidates.len() < texts.len() / 10);
        for query in [
            "needle",
            "ordinary",
            "src/",
            "ång",
            "öm",
            "e",
            "",
            "missing",
            "rs needle",
            "a\"b",
            "[*?]",
            "😀🚀🎯",
            "界面",
            "e\0n",
            "after_nul",
        ] {
            let actual: Vec<_> = index
                .candidates(query)
                .filter(|i| texts[*i].contains(query))
                .collect();
            let expected: Vec<_> = texts
                .iter()
                .enumerate()
                .filter_map(|(i, t)| t.contains(query).then_some(i))
                .collect();
            assert_eq!(actual, expected, "{query}");
        }
    }

    #[test]
    fn exhausted_index_budget_falls_back_without_losing_matches() {
        let texts = ["first needle", "second needle", "third needle"];
        for budget in [0, 3, 20, 1000] {
            let index = SubstringIndex::new(texts.iter().map(|s| s.to_string()), budget);
            assert_eq!(
                index
                    .candidates("needle")
                    .filter(|i| texts[*i].contains("needle"))
                    .collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
        }
    }
}
