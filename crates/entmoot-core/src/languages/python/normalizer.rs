//! Python-specific AST normalizer.

use tree_sitter::{Language, Node, Tree};

use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::visitor::PythonNodeVisitor;

/// Python-specific AST normalizer.
#[derive(Default)]
pub struct PythonNormalizer;

impl PythonNormalizer {
    pub fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let policy = policy.cloned().unwrap_or_else(StructuralHashPolicy::v1);
        let mut visitor = PythonNodeVisitor::new(Some(policy), source, Some(self));

        let root = tree.root_node();
        stacker::maybe_grow(32 * 1024, 1024 * 1024, || {
            self.serialize_node_recursive(&root, &mut visitor)
        })
        .map_err(|_| "Python serialization exceeded recursion depth")?;
        let (callables, file_imports, markers) = visitor.base.take_granular_outputs();

        Ok(VisitorOutput {
            callables,
            imports: file_imports,
            markers,
        })
    }

    fn serialize_node_recursive(
        &self,
        node: &Node,
        visitor: &mut PythonNodeVisitor,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        visitor.base.set_max_depth(MAX_RECURSIVE_DEPTH);
        visitor.visit(node)
    }

    pub fn extract_qualified_name_python_normalized(
        &self,
        node: &Node,
        source: &[u8],
        var_maps: &[std::collections::HashMap<String, String>],
        arg_maps: &[std::collections::HashMap<String, String>],
    ) -> String {
        self.extract_qualified_name_part(node, source, var_maps, arg_maps, true)
    }

    fn extract_qualified_name_part(
        &self,
        node: &Node,
        source: &[u8],
        var_maps: &[std::collections::HashMap<String, String>],
        arg_maps: &[std::collections::HashMap<String, String>],
        normalize_identifier: bool,
    ) -> String {
        if node.kind() == "identifier" {
            let Ok(text) = node.utf8_text(source) else {
                return String::new();
            };
            if text == "self" || !normalize_identifier {
                return text.to_string();
            }
            return arg_maps
                .iter()
                .rev()
                .find_map(|map| map.get(text))
                .or_else(|| var_maps.iter().rev().find_map(|map| map.get(text)))
                .cloned()
                .unwrap_or_else(|| text.to_string());
        }
        if node.kind() == "call" {
            return node
                .child_by_field_name("function")
                .map(|function| {
                    self.extract_qualified_name_part(&function, source, var_maps, arg_maps, normalize_identifier)
                })
                .unwrap_or_default();
        }
        if node.kind() == "subscript" {
            return node
                .child_by_field_name("value")
                .map(|value| self.extract_qualified_name_part(&value, source, var_maps, arg_maps, normalize_identifier))
                .unwrap_or_default();
        }

        let mut parts = Vec::new();
        let mut first_part = true;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "attribute" | "call" | "subscript" => {
                    let nested = self.extract_qualified_name_part(
                        &child,
                        source,
                        var_maps,
                        arg_maps,
                        normalize_identifier && first_part,
                    );
                    if !nested.is_empty() {
                        parts.push(nested);
                        first_part = false;
                    }
                }
                "." => {}
                _ => {}
            }
        }

        parts.join(".")
    }
}

impl ASTNormalizer for PythonNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        PythonNormalizer::visit_tree(self, tree, source, policy)
    }
}

pub fn language() -> Language {
    crate::grammars::PYTHON.into()
}

pub fn normalizer() -> Box<dyn ASTNormalizer> {
    Box::new(PythonNormalizer)
}
