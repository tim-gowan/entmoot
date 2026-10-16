//! Shared context accumulated during a unified AST walk.

use entmoot_contracts::{DeclarationSyntax, FileMarkers, SiteSpan};

/// Visitor-local declaration before occurrence disambiguation. Never serialized.
#[derive(Debug, Clone)]
pub struct PendingDeclaration {
    pub syntax: Option<DeclarationSyntax>,
    pub name: String,
    pub kind: String,
    pub scope: Vec<String>,
    pub span: SiteSpan,
    pub callable_start_byte: Option<usize>,
}

/// Internal walk output. Portable declaration records are created only at sealing.
#[derive(Debug, Clone, Default)]
pub struct UnsealedMarkers {
    pub markers: FileMarkers,
    pub declarations: Vec<PendingDeclaration>,
}

/// Mutable analysis state threaded through [`crate::visitor::BaseVisitor`].
#[derive(Debug, Clone)]
pub struct FileAnalysisContext {
    pub markers: FileMarkers,
    pub declarations: Vec<PendingDeclaration>,
    /// When false, listeners skip marker emission (hash-only path).
    pub collect_markers: bool,
}

impl Default for FileAnalysisContext {
    fn default() -> Self {
        Self {
            markers: FileMarkers::new(),
            declarations: Vec::new(),
            collect_markers: true,
        }
    }
}

impl FileAnalysisContext {
    pub fn new(collect_markers: bool) -> Self {
        Self {
            markers: FileMarkers::new(),
            declarations: Vec::new(),
            collect_markers,
        }
    }

    pub fn take_unsealed_markers(&mut self) -> UnsealedMarkers {
        UnsealedMarkers {
            markers: std::mem::take(&mut self.markers),
            declarations: std::mem::take(&mut self.declarations),
        }
    }
}
