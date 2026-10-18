//! Language-specific Resolver adapters for resolved-identity bind.
//!
//! Each module declares a capability profile and emits [`entmoot_contracts::NativeRawObservation`]
//! bags that join through exact spans. Adapters never rewrite `structural_hash`.

pub mod clang;
pub mod go;
pub mod java;
pub mod oxc;
#[cfg(feature = "resolved-oxc")]
pub mod oxc_package_api;
pub mod python;
pub mod roslyn;
pub mod sidecar;
pub mod typescript;

use serde::Serialize;

/// Declared resolved capability surface for one ecosystem producer (ADR-0012 / ADR-0024).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerCapabilityProfile {
    pub language: &'static str,
    pub producer_id: &'static str,
    pub semantic_surface: &'static str,
    pub supports_types: bool,
    pub supports_dispatch: bool,
    pub supports_cfg: bool,
    pub isolation: &'static str,
    pub limitations: &'static [&'static str],
}
