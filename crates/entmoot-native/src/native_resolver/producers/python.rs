//! Pyright Program batch declarations. Execution is explicit and process isolated.
use super::{
    sidecar::{collect_via_sidecar_with_env, resolve_sidecar_binary},
    ProducerCapabilityProfile,
};
use entmoot_contracts::NativeRawObservation;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const PIN: &str =
    entmoot_contracts::BindingEvidence::for_family(entmoot_contracts::BindingFamily::Python)
        .adapter_pin;
pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "python",
    producer_id: "pyright_program",
    semantic_surface: "exact_source_function_method_and_class_declarations",
    supports_types: false,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "pinned_program_batch_process",
    limitations: &[
        "stub_and_dynamic_bindings_remain_open",
        "closed_dispatch_unique_function_and_member_only",
        "selected_environment_recorded",
        "static_function_method_and_declared_constructor_headers_only",
        "static_string_all_closes_public_export_universe",
        "inherited_construction_decorators_and_import_only_sites_unassessed",
        "typecheck_success_not_compatibility",
    ],
};

pub const ENV_VAR: &str = "ENTMOOT_PYRIGHT_ADAPTER";
const ENV_ENVIRONMENT: &str = "ENTMOOT_PYTHON_ENVIRONMENT";

/// Relatives under checkout `.tmp/adapter-build/` and `~/.entmoot/adapters/`.
pub const CANDIDATES: &[&str] = &[
    "pyright-program/entmoot-pyright.cjs",
    ".tmp/adapter-build/pyright-program/entmoot-pyright.cjs",
];

/// Optional explicit interpreter/import-root configuration; otherwise probe the selected interpreter.
const ENVIRONMENT_CANDIDATES: &[&str] =
    &[".tmp/adapter-build/pyright-program/python_environment.json"];

const MISSING_MESSAGE: &str = "missing build or dependency: Python requires ENTMOOT_PYRIGHT_ADAPTER or pyright-program under ~/.entmoot/adapters (build with scripts/build_pyright_program.py). The installed interpreter is selected automatically unless an explicit Python environment is configured";

pub fn configured_binary() -> Option<PathBuf> {
    resolve_sidecar_binary(ENV_VAR, CANDIDATES)
}

pub fn configured_environment(root: &Path) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(ENV_ENVIRONMENT) {
        return Some(PathBuf::from(value));
    }
    ENVIRONMENT_CANDIDATES
        .iter()
        .map(|relative| root.join(relative))
        .find(|path| path.is_file())
}

/// Loud skip/unavailable text for workspace bind staging.
pub fn unavailable_message() -> &'static str {
    MISSING_MESSAGE
}

pub fn collect_python_observation(
    root: &Path,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    let environment = configured_environment(root);
    let child_env: Vec<_> = environment
        .map(|path| (ENV_ENVIRONMENT, path))
        .into_iter()
        .collect();
    collect_via_sidecar_with_env(
        root,
        ENV_VAR,
        CANDIDATES,
        MISSING_MESSAGE,
        wall,
        &child_env,
        canon,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn env_typo_does_not_fall_through_to_candidates() {
        let _guard = env_lock().lock().expect("env lock");
        let prior_adapter = std::env::var_os(ENV_VAR);
        let prior_env = std::env::var_os(ENV_ENVIRONMENT);
        std::env::set_var(ENV_VAR, r"G:\definitely-missing-entmoot-pyright.cjs");
        std::env::set_var(
            ENV_ENVIRONMENT,
            r"G:\definitely-missing-python-environment.json",
        );
        assert!(configured_binary().is_none());
        assert!(!configured_environment(Path::new(".")).unwrap().is_file());
        match prior_adapter {
            Some(value) => std::env::set_var(ENV_VAR, value),
            None => std::env::remove_var(ENV_VAR),
        }
        match prior_env {
            Some(value) => std::env::set_var(ENV_ENVIRONMENT, value),
            None => std::env::remove_var(ENV_ENVIRONMENT),
        }
    }

    #[test]
    fn default_candidates_resolve_when_files_exist() {
        let _guard = env_lock().lock().expect("env lock");
        let prior_adapter = std::env::var_os(ENV_VAR);
        let prior_env = std::env::var_os(ENV_ENVIRONMENT);
        std::env::remove_var(ENV_VAR);
        std::env::remove_var(ENV_ENVIRONMENT);
        let solver = PathBuf::from(CANDIDATES[0]);
        let environment = PathBuf::from(ENVIRONMENT_CANDIDATES[0]);
        if solver.is_file() {
            assert_eq!(configured_binary().as_deref(), Some(solver.as_path()));
        }
        if environment.is_file() {
            assert_eq!(
                configured_environment(Path::new(".")).as_deref(),
                Some(Path::new(".").join(&environment).as_path())
            );
        }
        match prior_adapter {
            Some(value) => std::env::set_var(ENV_VAR, value),
            None => std::env::remove_var(ENV_VAR),
        }
        match prior_env {
            Some(value) => std::env::set_var(ENV_ENVIRONMENT, value),
            None => std::env::remove_var(ENV_ENVIRONMENT),
        }
    }
}
