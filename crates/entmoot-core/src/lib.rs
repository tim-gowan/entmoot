//! Structural hashing — logic-addressable identifiers from normalized ASTs.
//!
//! Emits **structural hashes** (fingerprints of canonical IR) and **logic addresses**
//! (definition-site envelopes). Terminology: `docs/architecture/ARCHITECTURE.md`.

pub mod analysis;
mod call_ports;
pub mod callable_context;
pub mod canon;
pub mod clones;
mod code_contracts;
mod declaration_syntax;
pub mod error;
pub mod grammars;
pub mod granular;
pub mod ground_truth;
pub mod hash;
pub mod hasher;
pub mod index;
pub mod languages;
pub mod literals;
pub mod location_verify;
pub mod logic_address;
pub mod normalizer;
pub mod registry;
pub mod resolve;
pub mod scan;
pub mod scope_analyzer;
pub mod serialization;
pub mod sound_cst;
pub mod string_escape;
mod traversal;
pub mod visitor;

pub use error::{CoreError, CoreResult};

pub use analysis::{binding_marker_local, binding_marker_parameter, FileAnalysisContext};
pub use callable_context::CallableContext;
pub use entmoot_contracts::StructuralHashPolicy;
pub use entmoot_contracts::{
    BindingKind, BindingMarker, CallSiteMarker, ControlSite, ControlSiteKind, DeclarationMarker, DefMarker,
    DefinitionRecord, DefinitionSiteRef, FileIndexRecord, FileMarkers, ImportKind, ImportMarker, OccurrenceRecord,
    PolicyId, PropertyStoreKey,
};
pub use granular::{CallableHash, GranularHashResult};
pub use ground_truth::{resolve_dataset_path, GroundTruthDataset, GroundTruthValidator};
pub use hasher::StructuralHasher;
pub use index::build_file_index;
pub use location_verify::{exact_body_id_of_bytes, exact_body_id_of_span, logic_id_from_parts, verify_location_ref};
pub use logic_address::{AddressMeta, LogicAddress, LogicKind};
pub use normalizer::{ASTNormalizer, MAX_RECURSIVE_DEPTH};
pub use registry::{LanguageCapabilities, LanguageFamily, LanguagePlugin, LanguageRegistry};
pub use resolve::{build_occurrence_fqn, resolve_file_calls, resolve_repo_calls, ResolveConfidence, ResolvedCall};
pub use scan::{
    scan_repo_with_analysis, scan_repo_with_analysis_digest_gated, RepoScanResult, ScanError, ScannedFile, SkipReason,
    SkippedFile,
};
pub use scope_analyzer::ScopeAnalyzer;
pub use serialization::TokenStream;
pub use visitor::{BaseVisitor, NodeVisitor, RecursionDepthError, VisitorState};

/// Package-local source and release dependency commitment.
#[doc(hidden)]
pub const PACKAGE_FINGERPRINT: &str = env!("ENTMOOT_PACKAGE_FINGERPRINT");

pub mod provider_language;
