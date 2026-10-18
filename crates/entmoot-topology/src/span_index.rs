//! Span and line-based attachment index for topology nodes and external evidence.

use std::collections::HashMap;

use entmoot_contracts::{CallSiteMarker, DefMarker, FileMarkers, OccurrenceRecord};

use crate::graph::{callable_node_id, file_node_id};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefinitionIdentityKey {
    scope: String,
    symbol: String,
    kind: String,
    start_byte: Option<usize>,
}

impl DefinitionIdentityKey {
    pub fn from_marker(definition: &DefMarker, include_span: bool) -> Self {
        Self {
            scope: definition.scope.join("/"),
            symbol: definition.name.clone(),
            kind: definition.kind.clone(),
            start_byte: include_span.then_some(definition.start_byte),
        }
    }
}

pub fn definition_key_from_address(address: &str) -> Option<DefinitionIdentityKey> {
    let parts: Vec<&str> = address.split('|').collect();
    if parts.len() != 9 || parts[0] != "logic" || parts[5] == "-" {
        return None;
    }
    Some(DefinitionIdentityKey {
        scope: if parts[4] == "-" {
            String::new()
        } else {
            parts[4].into()
        },
        symbol: parts[5].into(),
        kind: parts[6].into(),
        start_byte: if parts[7] == "-" {
            None
        } else {
            Some(parts[7].parse().ok()?)
        },
    })
}

#[derive(Debug, Clone)]
pub struct CallableMatch {
    pub node_id: String,
    pub address_id: Option<String>,
    pub logic_address: Option<String>,
    pub structural_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CallSiteMatch {
    pub call: CallSiteMarker,
    pub enclosing: Option<CallableMatch>,
}

#[derive(Debug, Default)]
struct FileSpanData {
    defs: Vec<DefMarker>,
    def_ranges: RangeIndex,
    call_sites: Vec<CallSiteMarker>,
    call_ranges: RangeIndex,
    call_midpoints: Vec<(usize, usize)>,
    occurrences: HashMap<DefinitionIdentityKey, OccurrenceRecord>,
    line_starts: Vec<usize>,
}

/// Immutable augmented interval index over indices into an owner-managed vector.
///
/// Entries are sorted by start and backed by a max-end segment tree. Containment
/// is O(log n); overlap enumeration is output-sensitive and prunes subtrees whose
/// maximum end cannot intersect the query.
#[derive(Debug, Default)]
pub struct RangeIndex {
    entries: Vec<RangeEntry>,
    leaf_count: usize,
    max_ends: Vec<usize>,
}

#[derive(Debug)]
struct RangeEntry {
    start: usize,
    end: usize,
    value_index: usize,
}

#[derive(Clone, Copy)]
struct OverlapQuery {
    entry_limit: usize,
    minimum_end: usize,
    maximum_matches: usize,
}

impl RangeIndex {
    pub fn from_ranges(ranges: impl IntoIterator<Item = (usize, usize, usize)>) -> Self {
        let mut entries: Vec<_> = ranges
            .into_iter()
            .map(|(start, end, value_index)| RangeEntry {
                start,
                end,
                value_index,
            })
            .collect();
        entries.sort_by_key(|entry| (entry.start, std::cmp::Reverse(entry.end), entry.value_index));
        if entries.is_empty() {
            return Self::default();
        }

        let leaf_count = entries.len().next_power_of_two();
        let mut max_ends = vec![0; leaf_count * 2];
        for (index, entry) in entries.iter().enumerate() {
            max_ends[leaf_count + index] = entry.end;
        }
        for node in (1..leaf_count).rev() {
            max_ends[node] = max_ends[node * 2].max(max_ends[node * 2 + 1]);
        }
        Self {
            entries,
            leaf_count,
            max_ends,
        }
    }

    pub fn enclosing(&self, start: usize, end: usize) -> Option<usize> {
        self.enclosing_with_visits(start, end).0
    }

    fn enclosing_with_visits(&self, start: usize, end: usize) -> (Option<usize>, usize) {
        if self.entries.is_empty() {
            return (None, 0);
        }
        let limit = self.entries.partition_point(|entry| entry.start <= start);
        let mut visits = 0;
        let found = self.find_rightmost(1, 0, self.leaf_count, limit, end, &mut visits);
        (found.map(|index| self.entries[index].value_index), visits)
    }

    fn find_rightmost(
        &self,
        node: usize,
        left: usize,
        right: usize,
        limit: usize,
        minimum_end: usize,
        visits: &mut usize,
    ) -> Option<usize> {
        *visits += 1;
        if left >= limit || self.max_ends[node] < minimum_end {
            return None;
        }
        if right - left == 1 {
            return (left < self.entries.len()).then_some(left);
        }
        let middle = left + (right - left) / 2;
        self.find_rightmost(node * 2 + 1, middle, right, limit, minimum_end, visits)
            .or_else(|| self.find_rightmost(node * 2, left, middle, limit, minimum_end, visits))
    }

    pub fn overlapping(&self, start: usize, end: usize) -> Vec<usize> {
        self.overlapping_up_to(start, end, usize::MAX)
    }

    fn overlapping_up_to(&self, start: usize, end: usize, maximum: usize) -> Vec<usize> {
        if self.entries.is_empty() || end <= start {
            return Vec::new();
        }
        let limit = self.entries.partition_point(|entry| entry.start < end);
        let mut matches = Vec::new();
        self.collect_overlapping(
            1,
            0,
            self.leaf_count,
            OverlapQuery {
                entry_limit: limit,
                minimum_end: start,
                maximum_matches: maximum,
            },
            &mut matches,
        );
        matches
    }

    fn collect_overlapping(
        &self,
        node: usize,
        left: usize,
        right: usize,
        query: OverlapQuery,
        matches: &mut Vec<usize>,
    ) {
        if matches.len() >= query.maximum_matches
            || left >= query.entry_limit
            || self.max_ends[node] <= query.minimum_end
        {
            return;
        }
        if right - left == 1 {
            if let Some(entry) = self.entries.get(left) {
                matches.push(entry.value_index);
            }
            return;
        }
        let middle = left + (right - left) / 2;
        self.collect_overlapping(node * 2, left, middle, query, matches);
        self.collect_overlapping(node * 2 + 1, middle, right, query, matches);
    }
}

/// Index for resolving byte spans and source lines to topology callable nodes.
#[derive(Debug, Default)]
pub struct SpanIndex {
    files: HashMap<String, FileSpanData>,
}

impl SpanIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest_file(
        &mut self,
        path: &str,
        markers: Option<&FileMarkers>,
        occurrences: &[OccurrenceRecord],
        source: Option<&str>,
    ) {
        let mut data = FileSpanData::default();

        if let Some(markers) = markers {
            data.defs = markers.definitions.clone();
            data.defs.sort_by_key(|d| d.start_byte);
            data.def_ranges =
                RangeIndex::from_ranges(data.defs.iter().enumerate().map(|(index, definition)| {
                    (definition.start_byte, definition.end_byte, index)
                }));
            data.call_sites = markers.call_sites.clone();
            data.call_sites.sort_by_key(|c| c.start_byte);
            data.call_ranges = RangeIndex::from_ranges(
                data.call_sites
                    .iter()
                    .enumerate()
                    .map(|(index, call)| (call.start_byte, call.end_byte, index)),
            );
            data.call_midpoints = data
                .call_sites
                .iter()
                .enumerate()
                .map(|(index, call)| (call.start_byte.saturating_add(call.end_byte) / 2, index))
                .collect();
            data.call_midpoints.sort_unstable();
        }

        for occ in occurrences {
            if let Some(key) = definition_key_from_address(&occ.logic_address) {
                data.occurrences.insert(key, occ.clone());
            }
        }

        if let Some(source) = source {
            data.line_starts = line_starts(source);
        }

        self.files.insert(normalize_path(path), data);
    }

    pub fn resolve_byte(&self, file: &str, byte: usize) -> Option<CallableMatch> {
        let data = self.files.get(&normalize_path(file))?;
        let def = data
            .def_ranges
            .enclosing(byte, byte)
            .and_then(|index| data.defs.get(index))?;
        self.match_from_def(file, def, data)
    }

    /// Smallest callable containing the complete half-open byte range.
    pub fn resolve_range(&self, file: &str, start: usize, end: usize) -> Option<CallableMatch> {
        let data = self.files.get(&normalize_path(file))?;
        let def = data
            .def_ranges
            .enclosing(start, end)
            .and_then(|index| data.defs.get(index))?;
        self.match_from_def(file, def, data)
    }

    pub fn resolve_line(
        &self,
        file: &str,
        line: usize,
        column: Option<usize>,
    ) -> Option<CallableMatch> {
        let byte = self.line_column_byte(file, line, column)?;
        self.resolve_byte(file, byte)
    }

    /// Nearest call site at `(file, line, column)` (column preferred when present).
    pub fn resolve_call_at(
        &self,
        file: &str,
        line: usize,
        column: Option<usize>,
    ) -> Option<CallSiteMatch> {
        let data = self.files.get(&normalize_path(file))?;
        if data.call_sites.is_empty() {
            return None;
        }
        let target = self.line_column_byte(file, line, column)?;
        let call_index = data.call_ranges.enclosing(target, target).or_else(|| {
            let insertion = data
                .call_midpoints
                .partition_point(|(midpoint, _)| *midpoint < target);
            [insertion.checked_sub(1), Some(insertion)]
                .into_iter()
                .flatten()
                .filter_map(|index| data.call_midpoints.get(index))
                .min_by_key(|(midpoint, call_index)| (midpoint.abs_diff(target), *call_index))
                .map(|(_, call_index)| *call_index)
        })?;
        let call = data.call_sites.get(call_index)?.clone();
        let enclosing = self.resolve_byte(file, call.start_byte);
        Some(CallSiteMatch { call, enclosing })
    }

    /// Call site whose byte span contains `(file, line, column)`.
    ///
    /// Unlike [`Self::resolve_call_at`], this never guesses based on proximity.
    /// It is the appropriate join for precise provider reference occurrences.
    pub fn resolve_call_containing(
        &self,
        file: &str,
        line: usize,
        column: usize,
    ) -> Option<CallSiteMatch> {
        let data = self.files.get(&normalize_path(file))?;
        let target = self.line_column_byte(file, line, Some(column))?;
        let call = data
            .call_ranges
            .enclosing(target, target)
            .and_then(|index| data.call_sites.get(index))?
            .clone();
        let enclosing = self.resolve_byte(file, call.start_byte);
        Some(CallSiteMatch { call, enclosing })
    }

    /// Innermost call containing the query, or the only partially overlapping call.
    ///
    /// A broad location overlapping multiple calls is deliberately not attached
    /// to one by ingestion order or proximity.
    pub fn resolve_call_overlapping(
        &self,
        file: &str,
        start: usize,
        end: usize,
    ) -> Option<CallSiteMatch> {
        let data = self.files.get(&normalize_path(file))?;
        let call_index = if let Some(index) = data.call_ranges.enclosing(start, end) {
            index
        } else {
            let overlaps = data.call_ranges.overlapping_up_to(start, end, 2);
            match overlaps.as_slice() {
                [only] => *only,
                [] | [_, _, ..] => return None,
            }
        };
        let call = data.call_sites.get(call_index)?.clone();
        let enclosing = self.resolve_byte(file, call.start_byte);
        Some(CallSiteMatch { call, enclosing })
    }

    #[cfg(test)]
    fn resolve_byte_with_visits(&self, file: &str, byte: usize) -> (Option<CallableMatch>, usize) {
        let Some(data) = self.files.get(&normalize_path(file)) else {
            return (None, 0);
        };
        let (index, visits) = data.def_ranges.enclosing_with_visits(byte, byte);
        let matched = index
            .and_then(|index| data.defs.get(index))
            .and_then(|definition| self.match_from_def(file, definition, data));
        (matched, visits)
    }

    fn line_column_byte(&self, file: &str, line: usize, column: Option<usize>) -> Option<usize> {
        let data = self.files.get(&normalize_path(file))?;
        if data.line_starts.is_empty() || line == 0 {
            return None;
        }
        let idx = line.saturating_sub(1);
        let line_start = *data.line_starts.get(idx)?;
        let next_line = data
            .line_starts
            .get(idx + 1)
            .copied()
            .unwrap_or(line_start.saturating_add(10_000));
        let col = column.unwrap_or(1).saturating_sub(1);
        let byte = line_start.saturating_add(col);
        Some(byte.min(next_line.saturating_sub(1)).max(line_start))
    }

    fn match_from_def(
        &self,
        file: &str,
        def: &DefMarker,
        data: &FileSpanData,
    ) -> Option<CallableMatch> {
        let node_id = callable_node_id(file, &def.name, def.start_byte);
        let occ = data
            .occurrences
            .get(&DefinitionIdentityKey::from_marker(def, true))
            .or_else(|| {
                data.occurrences
                    .get(&DefinitionIdentityKey::from_marker(def, false))
            });
        Some(CallableMatch {
            node_id,
            address_id: occ.map(|o| o.address_id.clone()),
            logic_address: occ.map(|o| o.logic_address.clone()),
            structural_hash: occ.map(|o| o.structural_hash.clone()),
        })
    }

    pub fn resolve_to_file(&self, file: &str) -> String {
        file_node_id(&normalize_path(file))
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::{CallSiteMarker, DefMarker, FileMarkers};

    #[test]
    fn resolves_line_to_callable() {
        let source = "def outer():\n    pass\n\ndef inner():\n    x = 1\n";
        let mut index = SpanIndex::new();
        let markers = FileMarkers {
            definitions: vec![
                DefMarker {
                    name: "outer".into(),
                    kind: "function".into(),
                    scope: vec![],
                    start_byte: 0,
                    end_byte: 20,
                },
                DefMarker {
                    name: "inner".into(),
                    kind: "function".into(),
                    scope: vec![],
                    start_byte: 22,
                    end_byte: 45,
                },
            ],
            ..FileMarkers::new()
        };
        index.ingest_file("a.py", Some(&markers), &[], Some(source));
        let m = index.resolve_line("a.py", 5, None).expect("match");
        assert_eq!(m.node_id, callable_node_id("a.py", "inner", 22));
    }

    #[test]
    fn resolve_call_at_prefers_containing_site() {
        let source = "def f():\n    foo()\n    obj.bar()\n";
        // Approximate spans: "foo()" and "obj.bar()"
        let foo_start = source.find("foo()").unwrap();
        let foo_end = foo_start + 5;
        let bar_start = source.find("obj.bar()").unwrap();
        let bar_end = bar_start + 9;
        let markers = FileMarkers {
            definitions: vec![DefMarker {
                name: "f".into(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 0,
                end_byte: source.len(),
            }],
            call_sites: vec![
                CallSiteMarker {
                    ports: None,
                    callee: "foo".into(),
                    start_byte: foo_start,
                    end_byte: foo_end,
                    owner: None,
                },
                CallSiteMarker {
                    ports: None,
                    callee: "obj.bar".into(),
                    start_byte: bar_start,
                    end_byte: bar_end,
                    owner: None,
                },
            ],
            ..FileMarkers::new()
        };
        let mut index = SpanIndex::new();
        index.ingest_file("a.py", Some(&markers), &[], Some(source));
        let line3 = 3usize;
        let col = source
            .lines()
            .nth(2)
            .and_then(|l| l.find('b'))
            .map(|i| i + 1);
        let hit = index.resolve_call_at("a.py", line3, col).expect("call");
        assert_eq!(hit.call.callee, "obj.bar");
        assert!(hit.enclosing.unwrap().node_id.contains("f"));

        let nearest = index
            .resolve_call_at("a.py", 1, Some(1))
            .expect("nearest call");
        assert_eq!(nearest.call.callee, "foo");
    }

    #[test]
    fn resolve_call_containing_does_not_join_a_nearby_call() {
        let source = "fn f() {\n    let value = 1;\n    use_it(value);\n}\n";
        let call_start = source.find("use_it").unwrap();
        let markers = FileMarkers {
            definitions: vec![DefMarker {
                name: "f".into(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 0,
                end_byte: source.len(),
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "use_it".into(),
                start_byte: call_start,
                end_byte: call_start + "use_it(value)".len(),
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let mut index = SpanIndex::new();
        index.ingest_file("lib.rs", Some(&markers), &[], Some(source));

        assert!(index.resolve_call_containing("lib.rs", 2, 9).is_none());
        assert!(index.resolve_call_containing("lib.rs", 3, 5).is_some());
    }

    #[test]
    fn enclosing_def_picks_innermost() {
        let defs = [
            DefMarker {
                name: "outer".into(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 0,
                end_byte: 100,
            },
            DefMarker {
                name: "inner".into(),
                kind: "function".into(),
                scope: vec!["outer".into()],
                start_byte: 10,
                end_byte: 50,
            },
        ];
        let index = RangeIndex::from_ranges(
            defs.iter()
                .enumerate()
                .map(|(index, definition)| (definition.start_byte, definition.end_byte, index)),
        );
        let definition = &defs[index.enclosing(25, 25).unwrap()];
        assert_eq!(definition.name, "inner");
    }

    #[test]
    fn overlap_index_uses_half_open_boundaries_and_bounded_results() {
        let index = RangeIndex::from_ranges([(0, 10, 0), (10, 20, 1), (12, 18, 2)]);

        assert_eq!(index.overlapping(10, 12), vec![1]);
        assert_eq!(index.overlapping_up_to(11, 19, 2).len(), 2);
    }

    #[test]
    fn broad_region_does_not_select_one_of_multiple_calls() {
        let markers = FileMarkers {
            call_sites: vec![
                CallSiteMarker {
                    ports: None,
                    callee: "first".into(),
                    start_byte: 10,
                    end_byte: 15,
                    owner: None,
                },
                CallSiteMarker {
                    ports: None,
                    callee: "second".into(),
                    start_byte: 20,
                    end_byte: 25,
                    owner: None,
                },
            ],
            ..FileMarkers::new()
        };
        let mut index = SpanIndex::new();
        index.ingest_file("a.py", Some(&markers), &[], None);

        assert!(index.resolve_call_overlapping("a.py", 5, 30).is_none());
    }

    #[test]
    fn point_resolution_visits_logarithmic_index_nodes() {
        let definitions = (0..16_384)
            .map(|index| DefMarker {
                name: format!("f{index}"),
                kind: "function".into(),
                scope: vec![],
                start_byte: index * 4,
                end_byte: index * 4 + 2,
            })
            .collect();
        let markers = FileMarkers {
            definitions,
            ..FileMarkers::new()
        };
        let mut index = SpanIndex::new();
        index.ingest_file("many.py", Some(&markers), &[], None);

        let (matched, visited) = index.resolve_byte_with_visits("many.py", 32_000);
        assert!(matched.is_some());
        assert!(visited <= 32, "visited {visited} nodes for 16K spans");
    }
}
