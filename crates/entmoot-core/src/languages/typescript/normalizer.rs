//! TypeScript/TypeScript AST normalizer.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Tree};

use crate::literals::{normalize_integer_literal_text, IntegerNormalizeOptions};
use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::visitor::TypeScriptNodeVisitor;

/// TypeScript/TypeScript-specific AST normalizer.
#[derive(Default)]
pub struct TypeScriptNormalizer;

impl TypeScriptNormalizer {
    pub fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let policy = policy.cloned().unwrap_or_else(StructuralHashPolicy::v1);
        let root = tree.root_node();
        let mut visitor = TypeScriptNodeVisitor::new(Some(policy), source, Some(self));

        stacker::maybe_grow(32 * 1024, 1024 * 1024, || {
            self.serialize_node_recursive(&root, &mut visitor)
        })
        .map_err(|_| "TypeScript serialization exceeded recursion depth")?;
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
        visitor: &mut TypeScriptNodeVisitor,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        visitor.base.set_max_depth(MAX_RECURSIVE_DEPTH);
        visitor.visit(node)
    }

    pub fn extract_qualified_name_ts_normalized(
        &self,
        node: &Node,
        source: &[u8],
        var_maps: &[HashMap<String, String>],
        arg_maps: &[HashMap<String, String>],
    ) -> String {
        self.extract_qualified_name_part(node, source, var_maps, arg_maps, true)
    }

    fn extract_qualified_name_part(
        &self,
        node: &Node,
        source: &[u8],
        var_maps: &[HashMap<String, String>],
        arg_maps: &[HashMap<String, String>],
        normalize_identifier: bool,
    ) -> String {
        if node.kind() == "identifier" {
            let Ok(text) = node.utf8_text(source) else {
                return String::new();
            };
            return if normalize_identifier {
                self.normalize_base_identifier_ts(text, var_maps, arg_maps)
            } else {
                text.to_string()
            };
        }
        if node.kind() == "call_expression" {
            return node
                .child_by_field_name("function")
                .map(|function| {
                    self.extract_qualified_name_part(&function, source, var_maps, arg_maps, normalize_identifier)
                })
                .unwrap_or_default();
        }
        if node.kind() == "subscript_expression" {
            return node
                .child_by_field_name("object")
                .map(|object| {
                    self.extract_qualified_name_part(&object, source, var_maps, arg_maps, normalize_identifier)
                })
                .unwrap_or_default();
        }

        let mut parts = Vec::new();
        let mut first_part = true;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        if normalize_identifier && first_part {
                            parts.push(self.normalize_base_identifier_ts(text, var_maps, arg_maps));
                        } else {
                            parts.push(text.to_string());
                        }
                        first_part = false;
                    }
                }
                "property_identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        parts.push(text.to_string());
                        first_part = false;
                    }
                }
                "." | "?." => {}
                "member_expression" | "call_expression" | "subscript_expression" => {
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
                _ => {}
            }
        }

        parts.join(".")
    }

    fn normalize_base_identifier_ts(
        &self,
        text: &str,
        var_maps: &[HashMap<String, String>],
        arg_maps: &[HashMap<String, String>],
    ) -> String {
        for arg_map in arg_maps.iter().rev() {
            if let Some(normalized) = arg_map.get(text) {
                return normalized.clone();
            }
        }
        for var_map in var_maps.iter().rev() {
            if let Some(normalized) = var_map.get(text) {
                return normalized.clone();
            }
        }
        text.to_string()
    }

    pub fn normalize_integer_literal_text(&self, literal_text: &str) -> String {
        normalize_integer_literal_text(
            literal_text,
            IntegerNormalizeOptions {
                leading_zero_octal: false,
            },
        )
    }
}

impl ASTNormalizer for TypeScriptNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        TypeScriptNormalizer::visit_tree(self, tree, source, policy)
    }
}

pub fn language() -> Language {
    crate::grammars::TYPESCRIPT.into()
}

pub fn language_tsx() -> Language {
    crate::grammars::TSX.into()
}

pub fn normalizer() -> Box<dyn ASTNormalizer> {
    Box::new(TypeScriptNormalizer)
}
