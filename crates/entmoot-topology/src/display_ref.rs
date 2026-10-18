//! Human-readable location references: FQN preferred, then line / hash / file fallbacks.

use std::collections::HashMap;

use crate::graph::{NodeKind, TopologyGraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRefKind {
    Fqn,
    Line,
    Hash,
    File,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParsedLogicAddress {
    pub path: String,
    pub scope: Vec<String>,
    pub symbol: String,
    pub kind: String,
    pub start_byte: Option<usize>,
}

/// Parse canonical `logic|repo|lang|path|scope|symbol|kind|start|end`.
pub fn parse_logic_address(logic_address: &str) -> Option<ParsedLogicAddress> {
    let parts: Vec<&str> = logic_address.split('|').collect();
    if parts.len() != 9 || parts[0] != "logic" {
        return None;
    }
    let path = parts[3].to_string();
    let scope = if parts[4] == "-" {
        Vec::new()
    } else {
        parts[4].split('/').map(str::to_string).collect()
    };
    let symbol = if parts[5] == "-" {
        String::new()
    } else {
        parts[5].to_string()
    };
    let kind = parts[6].to_string();
    let start_byte = if parts[7] == "-" {
        None
    } else {
        parts[7].parse().ok()
    };
    Some(ParsedLogicAddress {
        path,
        scope,
        symbol,
        kind,
        start_byte,
    })
}

/// Build FQN from file path, scope chain, and symbol name.
pub fn build_fqn(file_path: &str, scope: &[String], symbol: &str) -> Option<String> {
    if symbol.is_empty() {
        return None;
    }
    if scope.is_empty() {
        Some(format!("{file_path}::{symbol}"))
    } else {
        Some(format!("{file_path}::{}.{symbol}", scope.join(".")))
    }
}

/// Build FQN from a logic address string when possible.
pub fn fqn_from_logic_address(logic_address: &str) -> Option<String> {
    let parsed = parse_logic_address(logic_address)?;
    build_fqn(&parsed.path, &parsed.scope, &parsed.symbol)
}

pub fn byte_to_line(source: &str, byte: usize) -> usize {
    if source.is_empty() {
        return 1;
    }
    let clamped = byte.min(source.len().saturating_sub(1));
    source
        .as_bytes()
        .iter()
        .take(clamped + 1)
        .filter(|&&byte| byte == b'\n')
        .count()
        + 1
}

pub struct DisplayRefInput<'a> {
    pub kind: NodeKind,
    pub file_path: Option<&'a str>,
    pub scope: &'a [String],
    pub symbol: Option<&'a str>,
    pub logic_address: Option<&'a str>,
    pub address_id: Option<&'a str>,
    pub structural_hash: Option<&'a str>,
    pub file_hash: Option<&'a str>,
    pub start_byte: Option<usize>,
    pub source: Option<&'a str>,
    pub label: &'a str,
}

/// Resolve display reference with fallback chain: FQN → line → hash → file.
pub fn resolve_display_ref(
    input: DisplayRefInput<'_>,
) -> (String, Option<String>, DisplayRefKind, Option<usize>) {
    let start_line = line_for(&input);
    resolve_display_ref_with_line(input, start_line)
}

fn resolve_display_ref_with_line(
    input: DisplayRefInput<'_>,
    start_line: Option<usize>,
) -> (String, Option<String>, DisplayRefKind, Option<usize>) {
    let file_path: String = input
        .file_path
        .map(str::to_string)
        .or_else(|| {
            input
                .logic_address
                .and_then(parse_logic_address)
                .map(|p| p.path)
        })
        .unwrap_or_else(|| "-".to_string());

    let scope: &[String] = if !input.scope.is_empty() {
        input.scope
    } else {
        &[]
    };

    let symbol = if matches!(input.kind, NodeKind::Callable | NodeKind::Declaration) {
        input.symbol.or(Some(input.label))
    } else {
        None
    };

    if let Some(sym) = symbol {
        if let Some(fqn) = input
            .logic_address
            .and_then(fqn_from_logic_address)
            .or_else(|| build_fqn(&file_path, scope, sym))
        {
            return (fqn.clone(), Some(fqn), DisplayRefKind::Fqn, start_line);
        }
    }

    if let Some(line) = start_line {
        let reference = format!("{file_path}:{line}");
        return (reference, None, DisplayRefKind::Line, Some(line));
    }

    if let Some(id) = input.address_id.filter(|s| !s.is_empty()) {
        let short: String = id.chars().take(12).collect();
        let reference = format!("{file_path}#{short}");
        return (reference, None, DisplayRefKind::Hash, None);
    }

    if let Some(hash) = input.structural_hash.or(input.file_hash) {
        let short = hash.strip_prefix("blake3:").unwrap_or(hash);
        let short: String = short.chars().take(12).collect();
        let reference = format!("{file_path}#{short}");
        return (reference, None, DisplayRefKind::Hash, None);
    }

    let reference = file_path;
    (reference, None, DisplayRefKind::File, None)
}

fn line_for(input: &DisplayRefInput<'_>) -> Option<usize> {
    match (input.start_byte, input.source) {
        (Some(b), Some(s)) => Some(byte_to_line(s, b)),
        _ => None,
    }
}

struct SourceLineIndex {
    line_breaks: Vec<usize>,
    source_len: usize,
}

impl SourceLineIndex {
    fn new(source: &str) -> Self {
        let line_breaks = source
            .bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index))
            .collect();
        Self {
            line_breaks,
            source_len: source.len(),
        }
    }

    fn line(&self, byte: usize) -> usize {
        if self.source_len == 0 {
            return 1;
        }
        let clamped = byte.min(self.source_len.saturating_sub(1));
        self.line_breaks
            .partition_point(|line_break| *line_break <= clamped)
            + 1
    }
}

pub fn file_path_from_node(node: &crate::graph::TopologyNode) -> Option<&str> {
    node.path.as_deref()
}

/// Populate `fqn`, `display_ref`, `start_line`, and identity evidence on nodes
/// that are not listed in `reused_paths`.
pub fn apply_display_refs_preserving(
    graph: &mut TopologyGraph,
    file_sources: &HashMap<String, String>,
    reused_paths: &std::collections::HashSet<String>,
) {
    let line_indexes: HashMap<&str, SourceLineIndex> = file_sources
        .iter()
        .map(|(path, source)| (path.as_str(), SourceLineIndex::new(source)))
        .collect();
    for node in &mut graph.nodes {
        if node
            .path
            .as_ref()
            .is_some_and(|path| reused_paths.contains(path))
        {
            continue;
        }
        let file_path = file_path_from_node(node).map(str::to_string);
        let source = file_path
            .as_deref()
            .and_then(|p| file_sources.get(p).map(String::as_str));
        let symbol = if matches!(node.kind, NodeKind::Callable | NodeKind::Declaration) {
            Some(node.label.as_str())
        } else {
            None
        };

        let input = DisplayRefInput {
            kind: node.kind,
            file_path: file_path.as_deref(),
            scope: &node.scope,
            symbol,
            logic_address: node.logic_address.as_deref(),
            address_id: node.address_id.as_deref(),
            structural_hash: node.structural_hash.as_deref(),
            file_hash: node.file_hash.as_deref(),
            start_byte: node.start_byte,
            source,
            label: &node.label,
        };
        let indexed_line = file_path
            .as_deref()
            .and_then(|path| line_indexes.get(path))
            .zip(node.start_byte)
            .map(|(index, byte)| index.line(byte));
        let (display_ref, fqn, kind, start_line) =
            resolve_display_ref_with_line(input, indexed_line);

        node.display_ref = display_ref.clone();
        node.fqn = fqn;
        node.display_ref_kind = Some(kind);
        node.start_line = start_line;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fqn_from_logic_address_with_scope() {
        let la = "logic|repo|python|src/app.py|AuthService|verify|method|-|-";
        assert_eq!(
            fqn_from_logic_address(la).as_deref(),
            Some("src/app.py::AuthService.verify")
        );
    }

    #[test]
    fn fallback_to_line_without_symbol() {
        let source = "def a():\n    pass\n";
        let (reference, _, kind, line) = resolve_display_ref(DisplayRefInput {
            kind: NodeKind::CallStub,
            file_path: Some("f.py"),
            scope: &[],
            symbol: None,
            logic_address: None,
            address_id: None,
            structural_hash: None,
            file_hash: None,
            start_byte: Some(10),
            source: Some(source),
            label: "print",
        });
        assert_eq!(kind, DisplayRefKind::Line);
        assert_eq!(reference, "f.py:2");
        assert_eq!(line, Some(2));
    }

    #[test]
    fn byte_to_line_accepts_offsets_inside_a_multibyte_scalar() {
        assert_eq!(byte_to_line("a😀\nnext", 2), 1);
        assert_eq!(byte_to_line("a😀\nnext", 6), 2);
    }

    #[test]
    fn indexed_lines_match_byte_scans_at_boundaries_and_out_of_range() {
        let source = "first\n😀 second\nthird";
        let index = SourceLineIndex::new(source);
        for byte in [0, 5, 6, 7, 16, source.len(), usize::MAX] {
            assert_eq!(index.line(byte), byte_to_line(source, byte));
        }
        assert_eq!(SourceLineIndex::new("").line(99), 1);
    }

    #[test]
    fn callable_gets_fqn_from_path_and_symbol() {
        let (reference, fqn, kind, _) = resolve_display_ref(DisplayRefInput {
            kind: NodeKind::Callable,
            file_path: Some("f.py"),
            scope: &[],
            symbol: Some("fn"),
            logic_address: None,
            address_id: Some("abcdef1234567890"),
            structural_hash: None,
            file_hash: None,
            start_byte: None,
            source: None,
            label: "fn",
        });
        assert_eq!(kind, DisplayRefKind::Fqn);
        assert_eq!(reference, "f.py::fn");
        assert_eq!(fqn.as_deref(), Some("f.py::fn"));
    }

    #[test]
    fn fallback_to_hash() {
        let (reference, _, kind, _) = resolve_display_ref(DisplayRefInput {
            kind: NodeKind::CallStub,
            file_path: Some("f.py"),
            scope: &[],
            symbol: None,
            logic_address: None,
            address_id: Some("abcdef1234567890"),
            structural_hash: None,
            file_hash: None,
            start_byte: None,
            source: None,
            label: "print",
        });
        assert_eq!(kind, DisplayRefKind::Hash);
        assert!(reference.starts_with("f.py#"));
    }
}
