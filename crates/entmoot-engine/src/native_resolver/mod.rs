//! Native resolver lifecycle for explicit CLI init/bind and standalone producers; reads never execute it.
//!
//! Shared pipeline owns request validation, budgets, input drift, exact-span joining,
//! receipts, and artifact publication. Language backends supply raw call observations only.

pub mod adapter_profile;
pub mod budget;
mod canon;
pub(crate) mod environment;
pub mod family_registry;
pub mod join;
mod lock_instances;
pub mod manifest;
mod package_api;
mod package_api_seek;
pub mod pipeline;
pub mod producers;
pub mod publish;

#[cfg(feature = "resolved-rust-analyzer")]
pub mod rust_analyzer;

pub use adapter_profile::{
    discover, discover_and_store, families_for_languages, find_workspace_root, load_profile,
    profile_path, store_profile, with_workspace_root,
};
pub use budget::{ProducerBudget, ProducerBudgetError};
pub use entmoot_contracts::ExecutionPolicy;
pub use entmoot_contracts::NativeReceipt;
pub use entmoot_contracts::{
    NativeCallFact, NativeCallTarget, NativeRawObservation, NativeReferenceFact,
};
pub use family_registry::{
    adapters_root_hint, default_adapters_root, missing_adapter_message, resolve_adapter_binary,
    resolve_adapter_binary_for, resolve_sidecar_family, ADAPTERS_ROOT_ENV, DEFAULT_ADAPTERS_SUBDIR,
    HOME_ADAPTERS_SUBDIR, SIDECAR_FAMILIES,
};
pub use join::{
    language_matches, normalize_observations, project_calls_onto_inventory,
    METHOD_ID as JOIN_METHOD_ID,
};
pub use manifest::prepare_resolver_manifest;
pub use pipeline::{
    produce_clang_resolver, produce_go_resolver, produce_java_resolver, produce_oxc_resolver,
    produce_python_resolver, produce_roslyn_resolver, produce_rust_resolver,
    produce_typescript_resolver, NativeProduceRequest, NativeProduceResult, NativeProducerError,
};
pub use producers::{
    clang, go, java, oxc, python, roslyn,
    sidecar::{resolve_sidecar_binary, run_sidecar, SidecarRequest, SidecarResponse},
    typescript, ProducerCapabilityProfile,
};
pub use publish::{
    publish_dogfood, DogfoodPublishRequest, DogfoodPublishResult, PublicationError,
    PublicationManifest, PUBLICATION_SCHEMA,
};

/// Aligned `ra_ap_*` pin reported in receipts and freshness keys.
pub const RA_AP_PIN: &str = "0.0.358";

pub(crate) mod parallel;
