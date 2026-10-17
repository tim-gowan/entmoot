//! Family catalog and discovery: exclusive env/config override, checkout, PATH, home.

use crate::workspace::{
    canonicalize_override_path, default_entmoot_home, resolve_entmoot_override,
};
use std::path::{Path, PathBuf};

/// Shared build-output root for Go/Roslyn/Java/Clang resolver adapters.
pub const ADAPTERS_ROOT_ENV: &str = "ENTMOOT_ADAPTERS_ROOT";

/// Home-relative adapters directory under `~/.entmoot/adapters`.
pub const HOME_ADAPTERS_SUBDIR: &str = "adapters";

/// One cataloged resolver-adapter family (mirrors `tools/resolver-adapters/catalog.toml`).
#[derive(Debug, Clone, Copy)]
pub struct AdapterFamily {
    pub id: &'static str,
    pub env: &'static str,
    /// Paths relative to adapters root (or checkout when probing source).
    pub relative_artifacts: &'static [&'static str],
    /// When true, also probe checkout-relative paths after overrides.
    pub repo_relative: bool,
}

/// Sidecar families installed under `~/.entmoot/adapters/` (or `ENTMOOT_ADAPTERS_ROOT`).
pub const SIDECAR_FAMILIES: &[AdapterFamily] = &[
    AdapterFamily {
        id: "go",
        env: "ENTMOOT_GO_ADAPTER",
        relative_artifacts: &["go/entmoot-go-resolver.exe", "go/entmoot-go-resolver"],
        repo_relative: false,
    },
    AdapterFamily {
        id: "roslyn",
        env: "ENTMOOT_ROSLYN_ADAPTER",
        relative_artifacts: &[
            "roslyn/bin/Release/net9.0/Entmoot.RoslynSolver.dll",
            "roslyn/bin/Release/net9.0/Entmoot.RoslynSolver.exe",
            "roslyn/bin/Debug/net9.0/Entmoot.RoslynSolver.dll",
            "roslyn/bin/Debug/net9.0/Entmoot.RoslynSolver.exe",
        ],
        repo_relative: false,
    },
    AdapterFamily {
        id: "java",
        env: "ENTMOOT_JAVA_ADAPTER",
        relative_artifacts: &[
            "java/entmoot-java-resolver.jar",
            "java/target/entmoot-java-resolver.jar",
            "java/build/libs/entmoot-java-resolver.jar",
        ],
        repo_relative: false,
    },
    AdapterFamily {
        id: "clang",
        env: "ENTMOOT_CLANG_ADAPTER",
        relative_artifacts: &[
            "clang/entmoot-clang-resolver.exe",
            "clang/entmoot-clang-resolver",
            "clang/entmoot-clang-resolver.py",
        ],
        repo_relative: false,
    },
];

/// Program adapter relatives under home adapters (and checkout probe).
pub const PROGRAM_PYRIGHT_RELATIVES: &[&str] =
    &["pyright-program/entmoot-pyright.cjs", "entmoot-pyright.cjs"];
pub const PROGRAM_TYPESCRIPT_RELATIVES: &[&str] = &[
    "typescript-program/entmoot-typescript.cjs",
    "entmoot-typescript.cjs",
];

fn runtime_repository_root() -> PathBuf {
    // Development checkout detection for maintainer/CI builds under the repo.
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            cwd.ancestors()
                .find(|root| root.join("tools/resolver-adapters/catalog.toml").is_file())
                .map(Path::to_owned)
        })
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_owned))
                .unwrap_or_else(|| PathBuf::from("."))
        })
}

/// Default adapters root: `~/.entmoot/adapters` (override with `ENTMOOT_ADAPTERS_ROOT`).
#[must_use]
pub fn default_adapters_root() -> PathBuf {
    default_entmoot_home().join(HOME_ADAPTERS_SUBDIR)
}

fn adapters_root(source_root: Option<&Path>) -> PathBuf {
    if let Some(value) = resolve_entmoot_override(ADAPTERS_ROOT_ENV, source_root) {
        return PathBuf::from(value);
    }
    default_adapters_root()
}

/// Maintainer/CI checkout-relative build tree (not the operator default).
fn checkout_adapter_build_root(checkout: &Path) -> PathBuf {
    checkout.join(".tmp/adapter-build/resolver-adapters")
}

fn checkout_program_roots(checkout: &Path) -> [PathBuf; 2] {
    [
        checkout.join(".tmp/adapter-build"),
        checkout.join(".tmp/adapter-build/resolver-adapters"),
    ]
}

/// Candidate paths for an adapter env var + relative artifact list.
///
/// Order: exclusive override (process/config) → analyzed checkout conventional
/// paths → PATH basenames → home/`ENTMOOT_ADAPTERS_ROOT` adapters.
/// Process overrides are exclusive even when empty or invalid; empty config slots are unset.
pub fn adapter_candidate_paths(
    env_var: &str,
    relative_artifacts: &[&str],
    repo_relative: bool,
) -> Vec<PathBuf> {
    adapter_candidate_paths_for(env_var, relative_artifacts, repo_relative, None)
}

/// Like [`adapter_candidate_paths`] with an optional analyzed source root.
pub fn adapter_candidate_paths_for(
    env_var: &str,
    relative_artifacts: &[&str],
    repo_relative: bool,
    source_root: Option<&Path>,
) -> Vec<PathBuf> {
    if let Some(value) = resolve_entmoot_override(env_var, source_root) {
        return vec![PathBuf::from(value)];
    }
    let mut out = Vec::new();
    // Analyzed checkout / maintainer tree first (conventional build outputs).
    let checkout = source_root
        .map(Path::to_owned)
        .unwrap_or_else(runtime_repository_root);
    if repo_relative {
        for relative in relative_artifacts {
            out.push(checkout.join(relative));
        }
        for root in checkout_program_roots(&checkout) {
            for relative in relative_artifacts {
                let name = Path::new(relative)
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(relative));
                out.push(root.join(relative));
                out.push(root.join(&name));
            }
        }
    } else {
        let build = checkout_adapter_build_root(&checkout);
        for relative in relative_artifacts {
            out.push(build.join(relative));
        }
    }
    // Installed CLI names take precedence over the home fallback.
    for relative in relative_artifacts {
        if let Some(name) = Path::new(relative).file_name() {
            if let Some(path) = path_lookup(name) {
                out.push(path);
            }
        }
    }
    let root = adapters_root(source_root);
    for relative in relative_artifacts {
        out.push(root.join(relative));
    }
    out
}

fn path_lookup(file_name: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").or_else(|| std::env::var_os("Path"))?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(file_name))
        .find(|candidate| candidate.is_file())
}

/// First existing adapter binary for the given env + relatives.
pub fn resolve_adapter_binary(
    env_var: &str,
    relative_artifacts: &[&str],
    repo_relative: bool,
) -> Option<PathBuf> {
    resolve_adapter_binary_for(env_var, relative_artifacts, repo_relative, None)
}

/// Resolve with optional source root for checkout-first and config slots.
pub fn resolve_adapter_binary_for(
    env_var: &str,
    relative_artifacts: &[&str],
    repo_relative: bool,
    source_root: Option<&Path>,
) -> Option<PathBuf> {
    let exclusive = resolve_entmoot_override(env_var, source_root);
    if let Some(raw) = exclusive {
        return canonicalize_override_path(&raw)
            .ok()
            .filter(|path| path.is_file());
    }
    adapter_candidate_paths_for(env_var, relative_artifacts, repo_relative, source_root)
        .into_iter()
        .find(|path| path.is_file())
}

/// Lookup a sidecar family by id (`go`, `roslyn`, `java`, `clang`).
pub fn sidecar_family(id: &str) -> Option<&'static AdapterFamily> {
    SIDECAR_FAMILIES.iter().find(|family| family.id == id)
}

/// Resolve using cataloged sidecar family metadata.
pub fn resolve_sidecar_family(id: &str) -> Option<PathBuf> {
    let family = sidecar_family(id)?;
    resolve_adapter_binary(family.env, family.relative_artifacts, family.repo_relative)
}

/// True when an exclusive process/config override exists, even for an invalid path.
pub fn adapter_env_configured(env_var: &str) -> bool {
    resolve_entmoot_override(env_var, None).is_some()
}

/// Display helper for missing-adapter messages.
pub fn adapters_root_hint() -> String {
    if let Some(value) = resolve_entmoot_override(ADAPTERS_ROOT_ENV, None) {
        value
    } else {
        format!(
            "{} (or set {ADAPTERS_ROOT_ENV}; missing build or dependency)",
            default_adapters_root().display()
        )
    }
}

/// Missing-adapter message emphasizing build/dependency first.
pub fn missing_adapter_message(family: &str, env_var: &str) -> String {
    format!(
        "missing build or dependency: {family} adapter not installed; build with scripts/build_resolver_adapters.py (or Program build scripts), install under {}, or set {env_var}",
        default_adapters_root().display()
    )
}

/// Repo root used for program-adapter defaults (tests / messages).
pub fn repository_root() -> PathBuf {
    runtime_repository_root()
}

/// Join a repo-relative path (POSIX segments).
pub fn repo_join(relative: impl AsRef<Path>) -> PathBuf {
    runtime_repository_root().join(relative)
}

/// Deprecated alias kept for call sites expecting the old constant name in messages.
pub const DEFAULT_ADAPTERS_SUBDIR: &str = HOME_ADAPTERS_SUBDIR;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};

    #[test]
    fn discovery_orders_defaults_and_keeps_process_overrides_exclusive() {
        const TEST: &str = "native_resolver::family_registry::tests::discovery_orders_defaults_and_keeps_process_overrides_exclusive";
        const ENV: &str = "ENTMOOT_GO_ADAPTER";
        const RELATIVES: &[&str] = &["go/entmoot-go-resolver"];
        if let Ok(expected) = std::env::var("ENTMOOT_DISCOVERY_EXPECTED") {
            let source = std::env::current_dir().unwrap();
            let expected = (!expected.is_empty()).then(|| PathBuf::from(expected));
            assert_eq!(
                resolve_adapter_binary_for(ENV, RELATIVES, false, Some(&source)),
                expected
            );
            assert_eq!(
                super::super::adapter_profile::with_workspace_root(&source, || {
                    super::super::adapter_profile::resolve_with_profile(ENV, RELATIVES, false)
                }),
                expected
            );
            let profile = super::super::adapter_profile::discover(&source, Some(&["go".into()]));
            let go = profile
                .families
                .iter()
                .find(|row| row.family == "go")
                .unwrap();
            assert_eq!(
                go.availability == entmoot_contracts::AdapterFamilyAvailability::Available,
                expected.is_some(),
                "{go:?}"
            );
            if let Some(expected) = expected {
                assert_eq!(
                    fs::canonicalize(go.path.as_ref().unwrap()).unwrap(),
                    fs::canonicalize(expected).unwrap()
                );
            }
            if std::env::var_os(ENV).is_some() {
                assert_eq!(
                    go.selection,
                    entmoot_contracts::AdapterProfileSelection::ProcessEnv
                );
            } else if crate::workspace::entmoot_override_configured(ENV, Some(&source)) {
                assert_eq!(
                    go.selection,
                    entmoot_contracts::AdapterProfileSelection::Config
                );
            }
            return;
        }
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source");
        let data = fixture.path().join("data");
        let bin = fixture.path().join("bin");
        let home = fixture.path().join("adapters");
        let checkout = source.join(".tmp/adapter-build/resolver-adapters/go/entmoot-go-resolver");
        let on_path = bin.join("entmoot-go-resolver");
        let installed = home.join("go/entmoot-go-resolver");
        let explicit = fixture.path().join("explicit-adapter");
        for path in [&checkout, &on_path, &installed, &explicit] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "authored discovery fixture; never executed").unwrap();
        }
        let check = |selection: Option<&Path>, expected: Option<&Path>| {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", TEST])
                .current_dir(&source)
                .env("ENTMOOT_DATA_DIR", &data)
                .env(ADAPTERS_ROOT_ENV, &home)
                .env("PATH", &bin)
                .env(
                    "ENTMOOT_DISCOVERY_EXPECTED",
                    expected.unwrap_or(Path::new("")),
                )
                .env_remove(ENV);
            if let Some(value) = selection {
                child.env(ENV, value);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "selection={selection:?}, expected={expected:?}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        check(None, Some(&checkout));
        fs::remove_file(&checkout).unwrap();
        check(None, Some(&on_path));
        fs::remove_file(&on_path).unwrap();
        check(None, Some(&installed));
        let namespace = crate::workspace::workspace_namespace(&source, &data);
        fs::create_dir_all(&namespace).unwrap();
        fs::write(
            namespace.join("config.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema":"entmoot.config.v1", "mode":"local", "env":{(ENV): explicit}
            }))
            .unwrap(),
        )
        .unwrap();
        let explicit = fs::canonicalize(&explicit).unwrap();
        check(None, Some(&explicit));
        let installed = fs::canonicalize(&installed).unwrap();
        check(Some(&installed), Some(&installed));
        check(Some(&fixture.path().join("missing")), None);
        check(Some(Path::new("")), None);
    }
}
