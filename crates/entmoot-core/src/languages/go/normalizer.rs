//! Go-specific AST normalizer.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Tree};

use crate::literals::{normalize_integer_literal_text, IntegerNormalizeOptions};
use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::visitor::GoNodeVisitor;

/// Go-specific AST normalizer.
#[derive(Default)]
pub struct GoNormalizer;

impl GoNormalizer {
    pub fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let policy = policy.cloned().unwrap_or_else(StructuralHashPolicy::v1);
        let mut visitor = GoNodeVisitor::new(Some(policy), source, Some(self));

        let root = tree.root_node();
        stacker::maybe_grow(32 * 1024, 1024 * 1024, || {
            self.serialize_node_recursive(&root, &mut visitor)
        })
        .map_err(|_| "Go serialization exceeded recursion depth")?;
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
        visitor: &mut GoNodeVisitor,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        visitor.base.set_max_depth(MAX_RECURSIVE_DEPTH);
        visitor.visit(node)
    }

    // ----------------------------------------------------------------------
    // Expression normalization hooks
    // ----------------------------------------------------------------------

    /// Extract fully qualified name from a selector expression with package alias resolution.
    pub fn extract_qualified_name_go(
        &self,
        node: &Node,
        source: &[u8],
        var_maps: &[HashMap<String, String>],
        arg_maps: &[HashMap<String, String>],
        package_aliases: &HashMap<String, String>,
    ) -> String {
        let mut parts = Vec::new();
        let mut is_first_identifier = true;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "field_identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        if is_first_identifier && !var_maps.is_empty() && child.kind() == "identifier" {
                            is_first_identifier = false;
                            let mut normalized = None;
                            for arg_map in arg_maps.iter().rev() {
                                if let Some(n) = arg_map.get(text) {
                                    normalized = Some(n.clone());
                                    break;
                                }
                            }
                            if normalized.is_none() {
                                for var_map in var_maps.iter().rev() {
                                    if let Some(n) = var_map.get(text) {
                                        normalized = Some(n.clone());
                                        break;
                                    }
                                }
                            }
                            if let Some(n) = normalized {
                                parts.push(n);
                            } else if let Some(package_path) = package_aliases.get(text) {
                                let last = package_path.rsplit('/').next().unwrap_or(package_path);
                                parts.push(last.to_string());
                            } else {
                                parts.push(text.to_string());
                            }
                        } else {
                            parts.push(text.to_string());
                            is_first_identifier = false;
                        }
                    }
                }
                "." => {}
                "selector_expression" => {
                    let nested = self.extract_qualified_name_go(&child, source, var_maps, arg_maps, package_aliases);
                    if !nested.is_empty() {
                        parts.push(nested);
                    }
                }
                _ => {}
            }
        }

        parts.join(".")
    }

    pub fn normalize_integer_literal_text(&self, literal_text: &str) -> String {
        normalize_integer_literal_text(
            literal_text,
            IntegerNormalizeOptions {
                leading_zero_octal: true,
            },
        )
    }
}

impl ASTNormalizer for GoNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        GoNormalizer::visit_tree(self, tree, source, policy)
    }
}

pub fn language() -> Language {
    crate::grammars::GO.into()
}

pub fn normalizer() -> Box<dyn ASTNormalizer> {
    Box::new(GoNormalizer)
}
