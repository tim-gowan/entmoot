//! Snapshot-verified, bounded source reads.

use entmoot_contracts::query::ResultBounds;
use serde_json::{json, Value};

use super::{QueryEngine, QueryError, QUERY_RESULT_SCHEMA};
use crate::{SourceFile, SourceReadRequest};

impl QueryEngine {
    /// Return a bounded, line-addressed source excerpt verified against the
    /// indexed snapshot file digest.
    pub(super) fn read_source_span(
        &self,
        path: &str,
        start_line: usize,
        end_line: Option<usize>,
        context_lines: Option<usize>,
        requested_max_bytes: Option<usize>,
    ) -> Result<Value, QueryError> {
        if start_line == 0 {
            return Err(QueryError::InvalidInput("start_line is one-based".into()));
        }
        let end_line = end_line.unwrap_or(start_line);
        if end_line < start_line {
            return Err(QueryError::InvalidInput(
                "end_line must be greater than or equal to start_line".into(),
            ));
        }
        let path = path.trim().replace('\\', "/");
        if path.is_empty() {
            return Err(QueryError::InvalidInput("path is required".into()));
        }
        let Some(source_file) = self.read_source_file(&path)? else {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "read",
                "view": "source",
                "snapshot": self.graph.snapshot,
                "status": "unknown",
                "path": path,
                "items": [],
                "gaps": [if self.source_store.is_some() { "path_not_in_snapshot" } else { "source_store_unavailable" }],
                "bounds": ResultBounds::complete(0),
            }));
        };
        let source = std::str::from_utf8(&source_file.bytes)
            .map_err(|_| QueryError::UnsupportedSourceEncoding(path.clone()))?;
        let mut all_lines: Vec<&str> = source.split_terminator('\n').collect();
        if all_lines.is_empty() {
            all_lines.push("");
        }
        let total_lines = all_lines.len();
        if start_line > total_lines {
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA,
                "operation": "read",
                "view": "source",
                "snapshot": self.graph.snapshot,
                "status": "unknown",
                "path": path,
                "content_digest": source_file.content_digest,
                "total_lines": total_lines,
                "items": [],
                "gaps": ["location_out_of_range"],
                "bounds": ResultBounds::complete(0),
            }));
        }
        let context = context_lines.unwrap_or(3).min(20);
        let requested_first = start_line.saturating_sub(context).max(1);
        let requested_last = end_line.saturating_add(context).min(total_lines);
        let maximum_last = requested_first.saturating_add(399).min(requested_last);
        let max_bytes = requested_max_bytes
            .unwrap_or(32 * 1024)
            .clamp(1_024, 64 * 1024);
        let mut used_bytes = 0usize;
        let mut lines = Vec::new();
        let mut byte_truncated = false;
        for number in requested_first..=maximum_last {
            let text = all_lines[number - 1]
                .strip_suffix('\r')
                .unwrap_or(all_lines[number - 1]);
            if used_bytes.saturating_add(text.len()).saturating_add(1) > max_bytes {
                byte_truncated = true;
                break;
            }
            used_bytes += text.len() + 1;
            lines.push(json!({"line": number, "text": text}));
        }
        let returned_range = lines
            .last()
            .and_then(|line| line["line"].as_u64())
            .map(|last| json!({"start_line": requested_first, "end_line": last}));
        let truncated = byte_truncated || maximum_last < requested_last;
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "operation": "read",
                "view": "source",
            "snapshot": self.graph.snapshot,
            "status": if truncated { "truncated" } else { "known" },
            "path": path,
            "content_digest": source_file.content_digest,
            "total_lines": total_lines,
            "requested_range": {"start_line": start_line, "end_line": end_line, "context_lines": context},
            "returned_range": returned_range,
            "items": lines,
            "gaps": [],
            "bounds": ResultBounds { offset: requested_first - 1, limit: maximum_last - requested_first + 1, total_matches: requested_last - requested_first + 1, truncated },
        }))
    }

    pub(super) fn read_source_file(&self, path: &str) -> Result<Option<SourceFile>, QueryError> {
        let Some(source_store) = &self.source_store else {
            return Ok(None);
        };
        let normalized = path.replace('\\', "/");
        let Some(expected_content_digest) = self.indexes.snapshot_digest_by_path.get(&normalized)
        else {
            return Ok(None);
        };
        Ok(Some(source_store.read_path(SourceReadRequest {
            snapshot: &self.graph.snapshot,
            path: &normalized,
            expected_content_digest,
        })?))
    }
}
