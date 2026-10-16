//! Clang LibTooling / libclang C/C++ producer — pinned process sidecar.

use std::path::Path;
use std::time::Duration;

use super::sidecar::collect_via_sidecar;
use super::ProducerCapabilityProfile;
use entmoot_contracts::NativeRawObservation;

pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "cxx",
    producer_id: "clang_libtooling",
    semantic_surface: "ast_usr_types_bases_overrides_macro_locations",
    supports_types: true,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "pinned_clang_process",
    limitations: &[
        "requires_compile_commands_json",
        "header_command_inference_is_heuristic",
        "one_build_configuration_per_artifact",
        "virtual_cxx_methods_stay_open",
        "cfg_not_emitted_non_goal",
        "usr_bases_overrides_not_on_wire",
        "clangd_indexes_are_navigation_only",
    ],
};

const ENV_VAR: &str = "ENTMOOT_CLANG_ADAPTER";

const CANDIDATES: &[&str] = &[
    "clang/entmoot-clang-resolver.exe",
    "clang/entmoot-clang-resolver",
    "clang/entmoot-clang-resolver.py",
];

/// Collect C/C++ observations via the Clang sidecar when installed.
pub fn collect_clang_observation(
    source_root: &Path,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    // The profile requires a compilation database. Do not spend the budget on
    // the wrapper's empty-argument fallback and present it as a build context.
    // These are the explicit discovery locations of the current sidecar protocol.
    let explicit = std::env::var_os("ENTMOOT_CLANG_COMPDB");
    if explicit.as_ref().is_some_and(|path| {
        !Path::new(path).is_file()
            || Path::new(path)
                .file_name()
                .is_none_or(|name| name != "compile_commands.json")
    }) {
        return NativeRawObservation { invalid_environment: true, diagnostics: vec!["invalid_environment: ENTMOOT_CLANG_COMPDB does not identify a compilation database".into()], ..Default::default() };
    }
    if explicit.is_none()
        && ![
            "compile_commands.json",
            "build/compile_commands.json",
            "out/compile_commands.json",
            "cmake-build-debug/compile_commands.json",
            "cmake-build-release/compile_commands.json",
        ]
        .iter()
        .any(|path| source_root.join(path).is_file())
    {
        return NativeRawObservation {
            invalid_environment: true,
            diagnostics: vec!["invalid_environment: Clang requires compile_commands.json at the source root or a supported build directory; no compiler was invoked".into()],
            ..Default::default()
        };
    }
    let missing = format!(
        "{}; compile_commands.json required",
        super::super::family_registry::missing_adapter_message("clang", ENV_VAR)
    );
    let mut raw = collect_via_sidecar(
        source_root,
        ENV_VAR,
        CANDIDATES,
        &missing,
        wall,
        canon,
    );
    if !raw.diagnostics.is_empty() && !raw.invalid_environment {
        raw.build_context.insert("partial".into(), true.into());
    }
    raw
}

/// Locate the configured adapter without starting a process.
pub fn configured_binary() -> Option<std::path::PathBuf> {
    super::sidecar::resolve_sidecar_binary(ENV_VAR, CANDIDATES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_compilation_database_is_loud_without_invoking_a_compiler() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("main.c"), "int f() { return 0; }").unwrap();
        let raw = collect_clang_observation(root.path(), Duration::ZERO, false);
        assert!(raw.invalid_environment);
        assert!(!raw.budget_exceeded);
        assert!(raw.sidecar_output.is_none());
        assert!(raw.calls.is_empty());
        assert!(raw.diagnostics[0].contains("compile_commands.json"));
    }

    #[test]
    fn profile_requires_compile_commands() {
        assert!(PROFILE
            .limitations
            .contains(&"requires_compile_commands_json"));
        assert_eq!(PROFILE.isolation, "pinned_clang_process");
    }
}
