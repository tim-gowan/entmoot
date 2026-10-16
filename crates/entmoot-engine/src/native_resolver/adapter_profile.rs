//! Adapter profile discovery, load/store and resolution in the workspace volume.
//!
//! Init refreshes the profile from host discovery. Bind/produce resolve adapters as:
//! process/nonempty config override (exclusive) → profile path → catalog relatives.
//! Entmoot never downloads toolchains.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use entmoot_contracts::{
    AdapterFamilyAvailability, AdapterIntegrity, AdapterProfile, AdapterProfileFamily,
    AdapterProfileSelection, ADAPTER_PROFILE_FILE, ADAPTER_PROFILE_SCHEMA,
    OBSOLETE_NATIVE_PROFILE_FILE,
};

use crate::workspace::storage_directory;

use super::family_registry::{
    adapter_env_configured, adapters_root_hint, resolve_adapter_binary, resolve_adapter_binary_for,
    sidecar_family, ADAPTERS_ROOT_ENV, SIDECAR_FAMILIES,
};
use super::producers::{clang, go, java, oxc, python, roslyn, typescript};

thread_local! {
    static ACTIVE_WORKSPACE_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Run `f` with adapter profile resolution scoped to `root`.
pub fn with_workspace_root<R>(root: &Path, f: impl FnOnce() -> R) -> R {
    ACTIVE_WORKSPACE_ROOT.with(|slot| {
        *slot.borrow_mut() = Some(root.to_path_buf());
    });
    let result = f();
    ACTIVE_WORKSPACE_ROOT.with(|slot| {
        *slot.borrow_mut() = None;
    });
    result
}

fn active_workspace_root() -> Option<PathBuf> {
    ACTIVE_WORKSPACE_ROOT.with(|slot| slot.borrow().clone())
}

/// Path to the adapter profile file in the workspace's selected storage volume.
pub fn profile_path(workspace_root: &Path) -> Result<PathBuf, String> {
    Ok(storage_directory(workspace_root)
        .map_err(|error| error.to_string())?
        .join(ADAPTER_PROFILE_FILE))
}

/// Load an adapter profile when the file exists and parses.
///
/// Refuses the obsolete pre-release `native-profile.json` in that volume (no alias).
pub fn load_profile(workspace_root: &Path) -> Result<Option<AdapterProfile>, String> {
    let dir = storage_directory(workspace_root).map_err(|error| error.to_string())?;
    let path = dir.join(ADAPTER_PROFILE_FILE);
    let obsolete = dir.join(OBSOLETE_NATIVE_PROFILE_FILE);
    if obsolete.is_file() && !path.is_file() {
        return Err(format!(
            "obsolete adapter profile layout {}; remove it and re-run entmoot init (expected {ADAPTER_PROFILE_FILE})",
            obsolete.display()
        ));
    }
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let profile: AdapterProfile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    if profile.schema_version != ADAPTER_PROFILE_SCHEMA {
        return Err(format!(
            "unsupported adapter profile schema_version {:?} (expected {ADAPTER_PROFILE_SCHEMA})",
            profile.schema_version
        ));
    }
    Ok(Some(profile))
}

/// Persist an adapter profile in the workspace's selected storage volume.
pub fn store_profile(workspace_root: &Path, profile: &AdapterProfile) -> Result<(), String> {
    let dir = storage_directory(workspace_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let path = dir.join(ADAPTER_PROFILE_FILE);
    let body = serde_json::to_vec_pretty(profile)
        .map_err(|error| format!("serialize adapter profile: {error}"))?;
    fs::write(&path, body).map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(())
}

/// Discover host adapters and write an honest profile (available vs missing).
///
/// Does not download tools or build adapters. `inventory_families` marks which
/// families are hard gaps when missing (`None` ⇒ all catalog sidecars + programs
/// informational except in-process producer features).
pub fn discover_and_store(
    workspace_root: &Path,
    inventory_families: Option<&[String]>,
) -> Result<AdapterProfile, String> {
    let profile = discover(workspace_root, inventory_families);
    store_profile(workspace_root, &profile)?;
    Ok(profile)
}

/// Discover without writing.
#[must_use]
pub fn discover(workspace_root: &Path, inventory_families: Option<&[String]>) -> AdapterProfile {
    let producer_features =
        cfg!(feature = "resolved-oxc") && cfg!(feature = "resolved-rust-analyzer");
    // Engine may be built with only one feature in tests; report either in-process capability.
    let producer_features = producer_features
        || cfg!(feature = "resolved-oxc")
        || cfg!(feature = "resolved-rust-analyzer");
    let mut profile = AdapterProfile::empty(producer_features);
    profile.adapters_root = Some(
        crate::workspace::resolve_entmoot_override(ADAPTERS_ROOT_ENV, Some(workspace_root))
            .unwrap_or_else(|| {
                super::family_registry::default_adapters_root()
                    .display()
                    .to_string()
            }),
    );

    let required = |family: &str| -> bool {
        inventory_families.is_some_and(|rows| rows.iter().any(|row| row == family))
    };

    // In-process families.
    for (family, available, pin) in [
        (
            "rust",
            cfg!(feature = "resolved-rust-analyzer"),
            Some(super::RA_AP_PIN),
        ),
        ("oxc", cfg!(feature = "resolved-oxc"), Some(oxc::OXC_PIN)),
    ] {
        let hard = required(family);
        profile.families.push(AdapterProfileFamily {
            family: family.into(),
            availability: if available {
                AdapterFamilyAvailability::Available
            } else if hard {
                AdapterFamilyAvailability::Missing
            } else {
                AdapterFamilyAvailability::Informational
            },
            selection: AdapterProfileSelection::ProducerFeature,
            path: None,
            pin: pin.map(str::to_owned),
            observed_version: None,
            digest: None,
            integrity: if available {
                AdapterIntegrity::Skipped
            } else {
                AdapterIntegrity::Missing
            },
            detail: (!available)
                .then(|| "rebuild entmoot with --features producer for in-process native".into()),
            hard: hard && !available,
        });
    }

    // Sidecar + program families from catalog / env / existing files.
    let program_families = [
        (
            "python",
            python::ENV_VAR,
            python::CANDIDATES,
            true,
            Some(python::PIN),
        ),
        (
            "typescript",
            typescript::ENV_VAR,
            typescript::CANDIDATES,
            true,
            Some(typescript::PIN),
        ),
    ];

    for family in SIDECAR_FAMILIES {
        push_sidecar_row(
            &mut profile,
            family.id,
            family.env,
            family.relative_artifacts,
            family.repo_relative,
            None,
            required(family.id),
            workspace_root,
        );
    }
    for (id, env, candidates, repo_relative, pin) in program_families {
        // TypeScript Program is opt-in: only hard when env claimed or inventory
        // selected typescript explicitly; inventory "oxc" does not require it.
        let hard = if id == "typescript" {
            adapter_env_configured(env) || required("typescript")
        } else {
            required(id)
        };
        push_sidecar_row(
            &mut profile,
            id,
            env,
            candidates,
            repo_relative,
            pin,
            hard,
            workspace_root,
        );
    }

    // Prefer go/roslyn pin labels from receipts when known.
    if let Some(row) = profile.families.iter_mut().find(|row| row.family == "go") {
        row.pin = Some(go::ADAPTER_PIN.into());
    }
    if let Some(row) = profile
        .families
        .iter_mut()
        .find(|row| row.family == "roslyn")
    {
        row.pin
            .get_or_insert_with(|| "catalog roslyn artifact".into());
    }
    if let Some(row) = profile.families.iter_mut().find(|row| row.family == "java") {
        row.pin.get_or_insert_with(|| "catalog java jar".into());
    }
    if let Some(row) = profile
        .families
        .iter_mut()
        .find(|row| row.family == "clang")
    {
        row.pin.get_or_insert_with(|| "libclang 18.1.1".into());
    }

    let _ = (
        roslyn::configured_binary(),
        java::configured_binary(),
        clang::configured_binary(),
    );
    profile
}

#[allow(clippy::too_many_arguments)]
fn push_sidecar_row(
    profile: &mut AdapterProfile,
    id: &str,
    env: &str,
    relatives: &[&str],
    repo_relative: bool,
    pin: Option<&str>,
    hard: bool,
    workspace_root: &Path,
) {
    let selection = if std::env::var_os(env).is_some() {
        AdapterProfileSelection::ProcessEnv
    } else {
        AdapterProfileSelection::Config
    };
    let configured = crate::workspace::resolve_entmoot_override(env, Some(workspace_root));
    let env_set = configured.is_some();
    let resolved = if let Some(value) = configured {
        Some(PathBuf::from(value))
    } else if let Some(path) = profile_recorded_path(workspace_root, id) {
        Some(path)
    } else {
        // Prefer workspace-local adapters root over compile-time repo root.
        let workspace_candidates =
            workspace_adapter_candidates(workspace_root, relatives, repo_relative);
        workspace_candidates
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| resolve_adapter_binary(env, relatives, repo_relative))
    };

    let (availability, selection, integrity, detail, hard_flag) = match &resolved {
        Some(path) if path.is_file() => (
            AdapterFamilyAvailability::Available,
            if env_set {
                selection
            } else {
                AdapterProfileSelection::Catalog
            },
            AdapterIntegrity::Unchecked,
            None,
            false,
        ),
        Some(path) if env_set => (
            AdapterFamilyAvailability::Invalid,
            selection,
            AdapterIntegrity::Missing,
            Some(format!(
                "invalid_environment: {env} points at missing path {}",
                path.display()
            )),
            hard,
        ),
        _ if env_set => (
            AdapterFamilyAvailability::Invalid,
            selection,
            AdapterIntegrity::Missing,
            Some(format!("invalid_environment: {env} set but path missing")),
            hard,
        ),
        _ => (
            if hard {
                AdapterFamilyAvailability::Missing
            } else {
                AdapterFamilyAvailability::Informational
            },
            AdapterProfileSelection::Unavailable,
            AdapterIntegrity::Missing,
            Some(format!(
                "adapter not found under {} (build with scripts/build_resolver_adapters.py)",
                adapters_root_hint()
            )),
            hard,
        ),
    };

    profile.families.push(AdapterProfileFamily {
        family: id.into(),
        availability,
        selection,
        path: resolved.map(|path| path.display().to_string()),
        pin: pin.map(str::to_owned),
        observed_version: None,
        digest: None,
        integrity,
        detail,
        hard: hard_flag,
    });
}

fn workspace_adapter_candidates(
    workspace_root: &Path,
    relatives: &[&str],
    repo_relative: bool,
) -> Vec<PathBuf> {
    super::family_registry::adapter_candidate_paths_for(
        "", // no exclusive env — checkout, PATH, then home
        relatives,
        repo_relative,
        Some(workspace_root),
    )
}

fn profile_recorded_path(workspace_root: &Path, family: &str) -> Option<PathBuf> {
    let profile = load_profile(workspace_root).ok().flatten()?;
    let row = profile.families.iter().find(|row| row.family == family)?;
    let path = row.path.as_ref()?;
    let path = PathBuf::from(path);
    path.is_file().then_some(path)
}

/// Resolve an adapter using process/config env → profile → checkout → PATH → home.
///
/// A present process override or nonempty config slot selects only that path.
/// Missing exclusive paths yield `None` so `available()` stays false; callers
/// use `adapter_env_configured` for explicit invalid-environment messaging.
pub fn resolve_with_profile(
    env_var: &str,
    relative_artifacts: &[&str],
    repo_relative: bool,
) -> Option<PathBuf> {
    let source = active_workspace_root().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| find_workspace_root(&cwd))
    });
    if let Some(value) = crate::workspace::resolve_entmoot_override(env_var, source.as_deref()) {
        return crate::workspace::canonicalize_override_path(&value)
            .ok()
            .filter(|path| path.is_file());
    }
    let family_id = SIDECAR_FAMILIES
        .iter()
        .find(|family| family.env == env_var)
        .map(|family| family.id)
        .or_else(|| {
            if env_var == python::ENV_VAR {
                Some("python")
            } else if env_var == typescript::ENV_VAR {
                Some("typescript")
            } else {
                None
            }
        });
    if let Some(family_id) = family_id {
        if let Some(root) = source.as_ref() {
            if let Some(path) = profile_recorded_path(root, family_id) {
                return Some(path);
            }
            if let Some(path) =
                workspace_adapter_candidates(root, relative_artifacts, repo_relative)
                    .into_iter()
                    .find(|path| path.is_file())
            {
                return Some(path);
            }
        }
    }
    resolve_adapter_binary_for(
        env_var,
        relative_artifacts,
        repo_relative,
        source.as_deref(),
    )
}

/// Walk source ancestors for an adapter profile or current publication in their selected volume.
#[must_use]
pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let storage = storage_directory(ancestor).ok()?;
        if storage.join(ADAPTER_PROFILE_FILE).is_file() || storage.join("current.json").is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

/// Inventory language → bind family ids present in a topology graph language set.
#[must_use]
pub fn families_for_languages(languages: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for language in languages {
        let family = match language.as_str() {
            "javascript" | "typescript" | "tsx" | "jsx" => {
                if typescript::explicitly_selected() {
                    "typescript"
                } else {
                    "oxc"
                }
            }
            "c_sharp" | "csharp" => "roslyn",
            "java" => "java",
            "c" | "cpp" => "clang",
            other => other,
        };
        if !out.iter().any(|row| row == family) {
            out.push(family.to_owned());
        }
    }
    out
}

/// Map family env var for tests / diagnostics.
#[must_use]
pub fn family_env(family: &str) -> Option<&'static str> {
    sidecar_family(family).map(|row| row.env).or(match family {
        "python" => Some(python::ENV_VAR),
        "typescript" => Some(typescript::ENV_VAR),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Environment changes are confined to one exact test in a fresh process.
    fn run_in_child(test: &str) -> bool {
        if std::env::var("ENTMOOT_PROFILE_TEST").as_deref() == Ok(test) {
            return false;
        }
        let fixture = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("native_resolver::adapter_profile::tests::{test}"),
            ])
            .env("ENTMOOT_PROFILE_TEST", test)
            .env("ENTMOOT_DATA_DIR", fixture.path().join("data"))
            .env(ADAPTERS_ROOT_ENV, fixture.path().join("adapters"))
            .env("PATH", "")
            .env_remove("ENTMOOT_GO_ADAPTER")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

    #[test]
    fn discover_marks_missing_sidecar_without_download() {
        if run_in_child("discover_marks_missing_sidecar_without_download") {
            return;
        }
        std::env::remove_var("ENTMOOT_GO_ADAPTER");
        let root = tempfile::tempdir().unwrap();
        let empty_adapters = root.path().join("adapters");
        fs::create_dir_all(&empty_adapters).unwrap();
        std::env::set_var(ADAPTERS_ROOT_ENV, &empty_adapters);
        let profile = discover(root.path(), Some(&["go".into()]));
        assert_eq!(profile.schema_version, ADAPTER_PROFILE_SCHEMA);
        let go = profile
            .families
            .iter()
            .find(|row| row.family == "go")
            .expect("go row");
        assert!(go.hard);
        assert_ne!(go.availability, AdapterFamilyAvailability::Available);
        assert!(profile
            .limitations
            .iter()
            .any(|row| row.contains("does_not_download")));
    }

    #[test]
    fn store_and_load_round_trip() {
        if run_in_child("store_and_load_round_trip") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut profile = AdapterProfile::empty(false);
        profile.families.push(AdapterProfileFamily {
            family: "go".into(),
            availability: AdapterFamilyAvailability::Available,
            selection: AdapterProfileSelection::Catalog,
            path: Some(root.path().join("fake-go").display().to_string()),
            pin: None,
            observed_version: None,
            digest: None,
            integrity: AdapterIntegrity::Unchecked,
            detail: None,
            hard: false,
        });
        store_profile(root.path(), &profile).unwrap();
        let loaded = load_profile(root.path()).unwrap().unwrap();
        assert_eq!(loaded.families.len(), 1);
    }

    #[test]
    fn refuses_obsolete_native_profile_filename() {
        if run_in_child("refuses_obsolete_native_profile_filename") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let dir = storage_directory(root.path()).unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(OBSOLETE_NATIVE_PROFILE_FILE), b"{}").unwrap();
        let err = load_profile(root.path()).unwrap_err();
        assert!(
            err.contains("obsolete"),
            "expected obsolete layout error: {err}"
        );
    }

    #[test]
    fn resolve_prefers_process_env_exclusively() {
        if run_in_child("resolve_prefers_process_env_exclusively") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing-adapter");
        std::env::set_var("ENTMOOT_GO_ADAPTER", &missing);
        let resolved =
            resolve_with_profile("ENTMOOT_GO_ADAPTER", &["go/entmoot-go-resolver"], false);
        assert_eq!(
            resolved, None,
            "exclusive missing path must not fall through"
        );
        assert!(adapter_env_configured("ENTMOOT_GO_ADAPTER"));
    }

    #[test]
    fn resolve_uses_profile_path_when_env_unset() {
        if run_in_child("resolve_uses_profile_path_when_env_unset") {
            return;
        }
        std::env::remove_var("ENTMOOT_GO_ADAPTER");
        let root = tempfile::tempdir().unwrap();
        let adapter = root.path().join("entmoot-go-resolver");
        fs::File::create(&adapter).unwrap().write_all(b"x").unwrap();
        let mut profile = AdapterProfile::empty(false);
        profile.families.push(AdapterProfileFamily {
            family: "go".into(),
            availability: AdapterFamilyAvailability::Available,
            selection: AdapterProfileSelection::Profile,
            path: Some(adapter.display().to_string()),
            pin: None,
            observed_version: None,
            digest: None,
            integrity: AdapterIntegrity::Unchecked,
            detail: None,
            hard: false,
        });
        store_profile(root.path(), &profile).unwrap();
        let resolved = with_workspace_root(root.path(), || {
            resolve_with_profile("ENTMOOT_GO_ADAPTER", &["go/nope"], false)
        });
        assert_eq!(resolved, Some(adapter));
    }
}
