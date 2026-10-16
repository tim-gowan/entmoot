//! C# AST normalizer.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Tree};

use crate::literals::{normalize_integer_literal_text, IntegerNormalizeOptions};
use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::visitor::CsharpNodeVisitor;

/// C#-specific AST normalizer.
#[derive(Default)]
pub struct CsharpNormalizer;

impl CsharpNormalizer {
    pub fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let policy = policy.cloned().unwrap_or_else(StructuralHashPolicy::v1);
        let using_aliases = HashMap::new();

        let root = tree.root_node();
        let mut visitor = CsharpNodeVisitor::new(Some(policy), source, Some(self), using_aliases);

        stacker::maybe_grow(32 * 1024, 1024 * 1024, || {
            self.serialize_node_recursive(&root, &mut visitor)
        })
        .map_err(|_| "C# serialization exceeded recursion depth")?;
        let (callables, file_imports, markers) = visitor.base.take_granular_outputs();

        Ok(VisitorOutput {
            callables,
            imports: file_imports,
            markers,
        })
    }

    pub fn resolve_using_alias(&self, identifier: &str, aliases: &HashMap<String, String>) -> String {
        if identifier.is_empty() || identifier.contains('.') {
            return identifier.to_string();
        }
        aliases
            .get(identifier)
            .cloned()
            .unwrap_or_else(|| identifier.to_string())
    }

    pub fn normalize_modifiers(&self, modifier_nodes: &[Node], source: &[u8]) -> Vec<String> {
        let mut names: Vec<String> = modifier_nodes
            .iter()
            .filter_map(|node| node.utf8_text(source).ok().map(str::to_string))
            .collect();
        names.sort();
        names
    }

    pub fn should_wrap_child(&self, node: &Node, parent_type: &str) -> bool {
        let control_flow = ["if_statement", "while_statement", "for_statement", "foreach_statement"];
        control_flow.contains(&parent_type) && node.kind() != "block"
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
        visitor: &mut CsharpNodeVisitor,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        visitor.base.set_max_depth(MAX_RECURSIVE_DEPTH);
        visitor.visit(node)
    }
}

impl ASTNormalizer for CsharpNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        CsharpNormalizer::visit_tree(self, tree, source, policy)
    }
}

pub fn language() -> Language {
    crate::grammars::CSHARP.into()
}

pub fn normalizer() -> Box<dyn ASTNormalizer> {
    Box::new(CsharpNormalizer)
}
