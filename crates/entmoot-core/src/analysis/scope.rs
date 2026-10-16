//! Callable scope preflight using language-specific [`ScopeAnalyzer`].

use std::collections::HashSet;

use tree_sitter::Node;

use crate::scope_analyzer::{analyze_scope_for_node, ScopeAnalyzer};

/// Scan a callable subtree for local definitions before classifying references.
pub fn preflight_callable(analyzer: &dyn ScopeAnalyzer, node: &Node, source: &[u8]) -> HashSet<String> {
    let (locals, globals) = analyze_scope_for_node(analyzer, node, source);
    let globals: HashSet<String> = globals.into_iter().collect();
    locals.into_iter().filter(|name| !globals.contains(name)).collect()
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    use super::*;
    use crate::languages::python::scope_analyzer::PythonScopeAnalyzer;

    #[test]
    fn preflight_matches_analyze_function_scope_for_sample() {
        let source = b"def f(x):\n    y = 1\n    return x + y\n";
        let mut parser = Parser::new();
        parser.set_language(&crate::grammars::PYTHON.into()).unwrap();
        let tree = parser.parse(source.as_slice(), None).unwrap();
        let root = tree.root_node();
        let func = root.named_child(0).unwrap();

        let analyzer = PythonScopeAnalyzer;
        let preflight = preflight_callable(&analyzer, &func, source.as_slice());

        let (locals, globals) = analyze_scope_for_node(&analyzer, &func, source.as_slice());
        let globals: HashSet<String> = globals.into_iter().collect();
        let expected: HashSet<String> = locals.into_iter().filter(|name| !globals.contains(name)).collect();
        assert_eq!(preflight, expected);
        assert!(preflight.contains("y"));
        assert!(preflight.contains("x"));
    }
}
