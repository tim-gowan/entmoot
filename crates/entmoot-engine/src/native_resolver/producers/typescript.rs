//! Explicit TypeScript Program tier. Oxc remains the in-process lexical tier.
use super::{
    sidecar::{collect_via_sidecar, resolve_sidecar_binary},
    ProducerCapabilityProfile,
};
use entmoot_contracts::NativeRawObservation;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const ENV_VAR: &str = "ENTMOOT_TYPESCRIPT_ADAPTER";
pub const CANDIDATES: &[&str] = &[
    "typescript-program/entmoot-typescript.cjs",
    ".tmp/adapter-build/typescript-program/entmoot-typescript.cjs",
];

/// Explicit selection keeps the checker tier distinct from Oxc's default tier.
pub fn explicitly_selected() -> bool {
    std::env::var_os(ENV_VAR).is_some()
}

pub fn configured_binary() -> Option<PathBuf> {
    resolve_sidecar_binary(ENV_VAR, CANDIDATES)
}

pub const PIN: &str = "typescript.program/1";
pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "javascript",
    producer_id: "typescript_program",
    semantic_surface: "compiler_declarations_and_module_exports",
    supports_types: true,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "explicit_program_batch_process",
    limitations: &[
        "selected_tsconfig_and_observed_filesystem_queries_only",
        "closed_dispatch_unique_function_only",
        "declaration_files_do_not_prove_runtime_implementation",
        "typecheck_success_not_compatibility_or_runtime_dispatch",
        "computed_runtime_exports_remain_open",
    ],
};

/// Run only an explicitly installed compiler adapter; never install during bind.
pub fn collect_typescript_observation(
    root: &Path,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    collect_via_sidecar(
        root,
        ENV_VAR,
        CANDIDATES,
        "invalid_environment: build scripts/build_typescript_program.py and set ENTMOOT_TYPESCRIPT_ADAPTER; a tsconfig.json is required",
        wall,
        canon,
    )
}
