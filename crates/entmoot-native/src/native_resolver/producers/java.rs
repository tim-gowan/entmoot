//! Java producer: shipped javac Trees profile only.

use std::path::Path;
use std::time::Duration;

use super::sidecar::collect_via_sidecar;
use super::ProducerCapabilityProfile;
use entmoot_contracts::NativeRawObservation;

/// Default Java materialization: JDK compiler Trees with explicit inputs.
pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "java",
    producer_id: "javac_trees",
    semantic_surface: "javac_trees_elements_explicit_sources_jars_release",
    supports_types: true,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "jvm_process",
    limitations: &[
        "selected_compiler_materialization_only",
        "compile_time_targets_only",
        "open_virtual_and_interface_dispatch_stay_open",
        "generated_sources_and_classpath_must_be_supplied_externally",
    ],
};

/// Env var for the exclusive Java adapter jar path.
pub const ENV_VAR: &str = "ENTMOOT_JAVA_ADAPTER";

const CANDIDATES: &[&str] = &[
    "java/entmoot-java-resolver.jar",
    "java/target/entmoot-java-resolver.jar",
    "java/build/libs/entmoot-java-resolver.jar",
];

/// Collect Java observations via the JVM sidecar (javac Trees).
pub fn collect_java_observation(
    source_root: &Path,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    let missing = super::super::family_registry::missing_adapter_message("java", ENV_VAR);
    let mut raw = collect_via_sidecar(source_root, ENV_VAR, CANDIDATES, &missing, wall, canon);
    // Preserve the native classpath mode (explicit jars, no-classpath, or the
    // selected javac materialization). A prepared compiler program still does
    // not establish complete project-build or runtime-dispatch coverage.
    raw.build_context.insert("partial".into(), true.into());
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
    fn profile_keeps_open_interface_dispatch() {
        const { assert!(PROFILE.supports_dispatch) };
        assert!(PROFILE
            .limitations
            .contains(&"open_virtual_and_interface_dispatch_stay_open"));
    }
}
