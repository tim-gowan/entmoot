//! Context tracking for callable extraction during granular hashing.

use std::collections::HashSet;

use crate::logic_address::{AddressMeta, LogicAddress, LogicKind};

/// Context for tracking a single callable during visitor traversal.
#[derive(Debug, Clone)]
pub struct CallableContext {
    pub name: String,
    pub node_type: String,
    pub kind: LogicKind,
    /// Declared container names (outer → inner) at definition site.
    pub scope: Vec<String>,
    /// Start/end byte range in source (tree-sitter nodes are ephemeral).
    pub start_byte: usize,
    pub end_byte: usize,
    pub imports: HashSet<String>,
    pub signature: String,
    /// Structural hash of body + imports. Independent of logic address.
    pub structural_hash: String,
    /// Structural hash of signature IR only.
    pub signature_hash: String,
    /// Logic address envelope (`logic|…`). See `docs/architecture/ARCHITECTURE.md`.
    pub logic_address: String,
    /// BLAKE3 hex of `logic_address` (address id).
    pub address_id: String,
}

impl CallableContext {
    pub fn new(name: impl Into<String>, node_type: impl Into<String>, start_byte: usize, end_byte: usize) -> Self {
        Self::with_scope(name, node_type, Vec::new(), false, start_byte, end_byte)
    }

    pub fn with_scope(
        name: impl Into<String>,
        node_type: impl Into<String>,
        scope: Vec<String>,
        inside_type: bool,
        start_byte: usize,
        end_byte: usize,
    ) -> Self {
        let name = name.into();
        let node_type = node_type.into();
        let kind = LogicKind::from_node_type(&node_type, inside_type);
        Self {
            name,
            node_type,
            kind,
            scope,
            start_byte,
            end_byte,
            imports: HashSet::new(),
            signature: String::new(),
            structural_hash: String::new(),
            signature_hash: String::new(),
            logic_address: String::new(),
            address_id: String::new(),
        }
    }

    /// Assign the logic address from meta + captured scope/symbol/kind.
    ///
    /// Prefer span-free address; caller may re-assign with span for disambiguation.
    pub fn seal_address(&mut self, meta: &AddressMeta, with_span: bool) {
        let start = if with_span { Some(self.start_byte) } else { None };
        let end = if with_span { Some(self.end_byte) } else { None };
        let addr = LogicAddress::new(meta, self.scope.clone(), self.name.clone(), self.kind, start, end);
        self.logic_address = addr.canonical();
        self.address_id = addr.address_id();
    }
}
