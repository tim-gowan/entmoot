//! Rust AST normalizer.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Tree};

use crate::literals::{normalize_integer_literal_text, IntegerNormalizeOptions};
use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::visitor::RustNodeVisitor;

/// Rust-specific AST normalizer.
#[derive(Default)]
pub struct RustNormalizer;

impl RustNormalizer {
    pub fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let policy = policy.cloned().unwrap_or_else(StructuralHashPolicy::v1);
        let use_aliases = HashMap::new();
        let root = tree.root_node();
        let mut visitor = RustNodeVisitor::new(Some(policy), source, Some(self), use_aliases);

        stacker::maybe_grow(32 * 1024, 1024 * 1024, || {
            self.serialize_node_recursive(&root, &mut visitor)
        })
        .map_err(|_| "Rust serialization exceeded recursion depth")?;
        let (callables, file_imports, markers) = visitor.base.take_granular_outputs();

        Ok(VisitorOutput {
            callables,
            imports: file_imports,
            markers,
        })
    }

    /// Resolve a simple identifier against `use path as Alias` bindings.
    pub fn resolve_use_alias(&self, identifier: &str, aliases: &HashMap<String, String>) -> String {
        if identifier.is_empty() || identifier.contains("::") {
            return identifier.to_string();
        }
        aliases
            .get(identifier)
            .cloned()
            .unwrap_or_else(|| identifier.to_string())
    }

    pub fn normalize_integer_literal_text(&self, literal_text: &str) -> String {
        normalize_integer_literal_text(
            literal_text,
            IntegerNormalizeOptions {
                leading_zero_octal: true,
            },
        )
    }

    fn serialize_node_recursive(
        &self,
        node: &Node,
        visitor: &mut RustNodeVisitor,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        visitor.base.set_max_depth(MAX_RECURSIVE_DEPTH);
        visitor.visit(node)
    }
}

impl ASTNormalizer for RustNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        RustNormalizer::visit_tree(self, tree, source, policy)
    }
}

pub fn language() -> Language {
    crate::grammars::RUST.into()
}

pub fn normalizer() -> Box<dyn ASTNormalizer> {
    Box::new(RustNormalizer)
}
