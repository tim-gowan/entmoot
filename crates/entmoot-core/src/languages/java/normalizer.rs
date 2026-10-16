//! Java AST normalizer.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Tree};

use crate::literals::{normalize_integer_literal_text, IntegerNormalizeOptions};
use crate::normalizer::{ASTNormalizer, VisitorOutput, VisitorResult, MAX_RECURSIVE_DEPTH};
use crate::serialization::TokenStream;
use crate::visitor::{NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::visitor::JavaNodeVisitor;

/// Well-known `java.lang` types for bare-identifier qualification.
pub static JAVA_LANG_TYPES: &[&str] = &[
    "Appendable",
    "AutoCloseable",
    "Boolean",
    "Byte",
    "CharSequence",
    "Character",
    "Class",
    "ClassLoader",
    "ClassValue",
    "Cloneable",
    "Comparable",
    "Compiler",
    "Double",
    "Enum",
    "Error",
    "Exception",
    "Float",
    "InheritableThreadLocal",
    "Integer",
    "Iterable",
    "Long",
    "Math",
    "Number",
    "Object",
    "Package",
    "Process",
    "ProcessBuilder",
    "Readable",
    "Runtime",
    "RuntimePermission",
    "RuntimeException",
    "SecurityManager",
    "Short",
    "StackTraceElement",
    "StrictMath",
    "String",
    "StringBuffer",
    "StringBuilder",
    "System",
    "Thread",
    "ThreadGroup",
    "ThreadLocal",
    "Throwable",
    "Void",
];

/// Java-specific AST normalizer.
#[derive(Default)]
pub struct JavaNormalizer;

impl JavaNormalizer {
    /// Qualify bare java.lang identifiers for canonicalization.
    pub fn qualify_java_lang_identifier(&self, identifier: &str, regular_imports: &HashMap<String, String>) -> String {
        if identifier.is_empty() || identifier.contains('.') {
            return identifier.to_string();
        }

        if let Some(qualified) = regular_imports.get(identifier) {
            return qualified.clone();
        }

        if JAVA_LANG_TYPES.contains(&identifier) {
            return format!("java.lang.{identifier}");
        }

        identifier.to_string()
    }

    pub fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        let policy = policy.cloned().unwrap_or_else(StructuralHashPolicy::v1);
        let static_imports = HashMap::new();
        let regular_imports = HashMap::new();

        let root = tree.root_node();
        let mut visitor = JavaNodeVisitor::new(Some(policy), source, Some(self), static_imports, regular_imports);

        stacker::maybe_grow(32 * 1024, 1024 * 1024, || {
            self.serialize_node_recursive(&root, &mut visitor)
        })
        .map_err(|_| "Java serialization exceeded recursion depth")?;
        let (callables, file_imports, markers) = visitor.base.take_granular_outputs();

        Ok(VisitorOutput {
            callables,
            imports: file_imports,
            markers,
        })
    }

    pub fn normalize_modifiers(&self, modifier_nodes: &[Node], source: &[u8]) -> Vec<String> {
        let mut names: Vec<String> = modifier_nodes
            .iter()
            .filter_map(|node| node.utf8_text(source).ok().map(str::to_string))
            .collect();
        names.sort();
        names
    }

    // ----------------------------------------------------------------------
    // Expression normalization hooks
    // ----------------------------------------------------------------------

    pub fn should_wrap_child(&self, node: &Node, parent_type: &str) -> bool {
        let control_flow = [
            "if_statement",
            "while_statement",
            "for_statement",
            "enhanced_for_statement",
        ];
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
        visitor: &mut JavaNodeVisitor,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        visitor.base.set_max_depth(MAX_RECURSIVE_DEPTH);
        visitor.visit(node)
    }
}

impl ASTNormalizer for JavaNormalizer {
    fn visit_tree(&self, tree: &Tree, source: &[u8], policy: Option<&StructuralHashPolicy>) -> VisitorResult {
        JavaNormalizer::visit_tree(self, tree, source, policy)
    }
}

pub fn language() -> Language {
    crate::grammars::JAVA.into()
}

pub fn normalizer() -> Box<dyn ASTNormalizer> {
    Box::new(JavaNormalizer)
}
