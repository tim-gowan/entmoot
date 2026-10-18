//! Roslyn (`Microsoft.CodeAnalysis`) C#/.NET producer profile.

use std::path::Path;
use std::time::Duration;

use super::sidecar::collect_via_sidecar;
use super::ProducerCapabilityProfile;
use entmoot_contracts::NativeRawObservation;

pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "csharp",
    producer_id: "roslyn_semantic_model",
    semantic_surface: "semantic_model_symbol_info_find_references_ioperation",
    supports_types: true,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "dotnet_process",
    limitations: &[
        "open_virtual_and_interface_dispatch_stay_open",
        "msbuild_workspace_failures_are_coverage_signals",
        "documentation_ids_are_provider_spellings_not_durable_ids",
        "cfg_not_emitted_non_goal",
        "find_references_and_ioperation_not_on_wire",
    ],
};

const ENV_VAR: &str = "ENTMOOT_ROSLYN_ADAPTER";

const CANDIDATES: &[&str] = &[
    "roslyn/bin/Release/net9.0/Entmoot.RoslynSolver.dll",
    "roslyn/bin/Release/net9.0/Entmoot.RoslynSolver.exe",
    "roslyn/bin/Debug/net9.0/Entmoot.RoslynSolver.dll",
    "roslyn/bin/Debug/net9.0/Entmoot.RoslynSolver.exe",
];

/// Collect C# observations via the Roslyn sidecar.
pub fn collect_roslyn_observation(
    source_root: &Path,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    let missing = super::super::family_registry::missing_adapter_message("roslyn", ENV_VAR);
    let mut raw = collect_via_sidecar(source_root, ENV_VAR, CANDIDATES, &missing, wall, canon);
    if !raw.invalid_environment
        && (!raw.diagnostics.is_empty()
            || raw
                .build_context
                .get("project_system")
                .and_then(serde_json::Value::as_str)
                == Some("adhoc"))
    {
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
    fn profile_uses_dotnet_isolation() {
        assert_eq!(PROFILE.isolation, "dotnet_process");
        const { assert!(!PROFILE.supports_cfg) };
    }
}
