//! Local frontend storage selection. Portable queries never contain database paths.
//!
//! Durable state defaults under `~/.entmoot` (or `ENTMOOT_DATA_DIR`). This module
//! selects storage independently of source trees. Per-workspace `config.json`
//! points at a volume and supplies `ENTMOOT_*` slots when process env is empty/unset.
use super::{WorkspaceError, WorkspacePublication};
use entmoot_contracts::{
    EntmootConfig, ENTMOOT_CONFIG_FILE, ENTMOOT_CONFIG_SCHEMA, ENTMOOT_DEFAULT_STORE,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static DATA_DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Select a process-wide data directory before starting a CLI or MCP service.
/// A directory contains isolated workspace namespaces; it is not a SQLite filename.
/// Remote locations are deliberately unsupported until a transport is implemented.
pub fn configure_data_directory(directory: Option<&Path>) -> Result<(), WorkspaceError> {
    let directory = directory.map(validate_local_data_path).transpose()?;
    DATA_DIRECTORY
        .set(directory)
        .map_err(|_| super::failure("data directory already configured for this process"))
}

/// Host home directory used when resolving the default `~/.entmoot` root.
#[must_use]
pub fn host_home_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Default operator data root: `<home>/.entmoot`.
#[must_use]
pub fn default_entmoot_home() -> PathBuf {
    host_home_directory()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".entmoot")
}

/// Process-wide or env-selected data root; defaults to [`default_entmoot_home`].
#[must_use]
pub fn data_root() -> PathBuf {
    let configured = DATA_DIRECTORY
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::var_os("ENTMOOT_DATA_DIR").map(PathBuf::from));
    match configured {
        Some(path) => path,
        None => default_entmoot_home(),
    }
}

fn validate_local_data_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if path.to_string_lossy().contains("://") {
        return Err(super::failure(
            "remote storage is not implemented; choose a local data directory",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(absolute)
}

/// Stable namespace directory for a source root under a data root (no volume resolve).
#[must_use]
pub fn workspace_namespace(root: &Path, data_directory: &Path) -> PathBuf {
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let key =
        entmoot_contracts::blake3_hex(canonical.as_os_str().as_encoded_bytes()).replace(':', "-");
    data_directory.join("workspaces").join(key)
}

/// Load `config.json` when present; otherwise return an empty local template.
pub fn load_config_file(path: &Path) -> Result<EntmootConfig, WorkspaceError> {
    match fs::read(path) {
        Ok(bytes) => {
            let config: EntmootConfig = serde_json::from_slice(&bytes)
                .map_err(|error| super::failure(format!("invalid {}: {error}", path.display())))?;
            if config.schema != ENTMOOT_CONFIG_SCHEMA {
                return Err(super::failure(format!(
                    "unsupported config schema {:?} (expected {ENTMOOT_CONFIG_SCHEMA})",
                    config.schema
                )));
            }
            if config.is_remote() {
                return Err(super::failure(
                    "remote storage is not implemented; set mode to local or choose a local volume",
                ));
            }
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(EntmootConfig::empty_local())
        }
        Err(error) => Err(error.into()),
    }
}

/// Ensure the workspace namespace has a `config.json`, creating empty slots if missing.
pub fn ensure_workspace_config(namespace: &Path) -> Result<EntmootConfig, WorkspaceError> {
    let path = namespace.join(ENTMOOT_CONFIG_FILE);
    if path.is_file() {
        return load_config_file(&path);
    }
    fs::create_dir_all(namespace)?;
    let config = EntmootConfig::empty_local();
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| super::failure(format!("serialize workspace config: {error}")))?;
    fs::write(&path, bytes)?;
    Ok(config)
}

/// Ensure global `~/.entmoot/config.json` exists (or under the active data root).
pub fn ensure_global_config(data_directory: &Path) -> Result<EntmootConfig, WorkspaceError> {
    let path = data_directory.join(ENTMOOT_CONFIG_FILE);
    if path.is_file() {
        return load_config_file(&path);
    }
    fs::create_dir_all(data_directory)?;
    let config = EntmootConfig::empty_local();
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| super::failure(format!("serialize global config: {error}")))?;
    fs::write(&path, bytes)?;
    Ok(config)
}

fn resolve_volume(namespace: &Path, config: &EntmootConfig) -> Result<PathBuf, WorkspaceError> {
    if config.is_remote() {
        return Err(super::failure(
            "remote storage is not implemented; set mode to local or choose a local volume",
        ));
    }
    match config
        .volume
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(volume) => {
            let path = PathBuf::from(volume);
            let absolute = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()?.join(path)
            };
            Ok(absolute)
        }
        None => Ok(namespace.join(ENTMOOT_DEFAULT_STORE)),
    }
}

/// Compute the local storage volume without creating it when possible.
///
/// Configuration is namespaced under the data root; an explicit volume can be elsewhere.
/// Uses the best-effort fallback behavior of [`storage_directory_in`].
pub fn storage_directory(root: &Path) -> PathBuf {
    storage_directory_in(root, Some(&data_root()))
}

/// Best-effort volume lookup used by embedding applications and frontend discovery.
///
/// `data_directory` of `None` uses [`default_entmoot_home`]. When the workspace
/// config is valid, its `volume` is honored; otherwise the default `store/` sibling.
/// Reads configuration from disk without creating it. Invalid config or volume
/// errors currently fall back silently; this is not configuration validation.
/// Mutation callers must validate configuration before relying on this path.
pub fn storage_directory_in(root: &Path, data_directory: Option<&Path>) -> PathBuf {
    let data = data_directory
        .map(Path::to_owned)
        .unwrap_or_else(default_entmoot_home);
    let namespace = workspace_namespace(root, &data);
    let config_path = namespace.join(ENTMOOT_CONFIG_FILE);
    let config = load_config_file(&config_path).unwrap_or_else(|_| EntmootConfig::empty_local());
    resolve_volume(&namespace, &config).unwrap_or_else(|_| namespace.join(ENTMOOT_DEFAULT_STORE))
}

/// Resolve a single `ENTMOOT_*` value: process env → workspace config → global config.
/// Empty values are skipped; configuration read errors are currently ignored.
#[must_use]
pub fn resolve_entmoot_override(key: &str, source_root: Option<&Path>) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        if !value.is_empty() {
            return Some(value);
        }
    }
    let data = data_root();
    if let Some(root) = source_root {
        let namespace = workspace_namespace(root, &data);
        if let Ok(config) = load_config_file(&namespace.join(ENTMOOT_CONFIG_FILE)) {
            if let Some(value) = config.env_slot(key) {
                return Some(value.to_owned());
            }
        }
    }
    if let Ok(config) = load_config_file(&data.join(ENTMOOT_CONFIG_FILE)) {
        if let Some(value) = config.env_slot(key) {
            return Some(value.to_owned());
        }
    }
    None
}

/// True when a nonempty override is configured (process or config slot).
#[must_use]
pub fn entmoot_override_configured(key: &str, source_root: Option<&Path>) -> bool {
    resolve_entmoot_override(key, source_root).is_some()
}

/// Canonicalize an override path: relative values resolve against process CWD.
pub fn canonicalize_override_path(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot resolve relative override against cwd: {error}"))?
            .join(path)
    };
    fs::canonicalize(&absolute).map_err(|error| {
        format!(
            "missing build or dependency: override path does not exist: {} ({error})",
            absolute.display()
        )
    })
}

/// Prepare a local workspace once at frontend startup, then return its immutable view.
/// Only missing inventory is bootstrapped. Existing evidence is never silently
/// refreshed, replaced, migrated or re-bound by a query.
pub fn prepare_workspace(root: &Path) -> Result<WorkspacePublication, WorkspaceError> {
    let root = fs::canonicalize(root)?;
    let data = data_root();
    ensure_global_config(&data)?;
    let namespace = workspace_namespace(&root, &data);
    let config = ensure_workspace_config(&namespace)?;
    let storage = resolve_volume(&namespace, &config)?;
    fs::create_dir_all(&storage)?;
    let pointer = storage.join("current.json");
    match fs::symlink_metadata(&pointer) {
        Ok(_) => {
            let publication = super::read_publication(&pointer)?;
            if fs::canonicalize(&publication.current.source_root)? != root {
                return Err(super::failure(
                    "workspace publication belongs to another source root",
                ));
            }
            Ok(publication)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            super::initialize_workspace(
                &root,
                None,
                &entmoot_core::StructuralHashPolicy::default(),
                &super::BindOptions {
                    structural_only: true,
                    ..Default::default()
                },
            )?;
            super::read_publication(&pointer)
        }
        Err(error) => Err(error.into()),
    }
}

/// Use the nearest existing publication, otherwise bootstrap the discovered source root.
pub fn prepare_discovered_workspace(start: &Path) -> Result<WorkspacePublication, WorkspaceError> {
    let start = fs::canonicalize(start)?;
    for ancestor in start.ancestors() {
        let pointer = storage_directory(ancestor).join("current.json");
        match fs::symlink_metadata(&pointer) {
            Ok(_) => return prepare_workspace(ancestor),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    prepare_workspace(&super::workspace_root(&start)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_are_stable_and_distinct_without_creating_storage() {
        let base = tempfile::tempdir().unwrap();
        let a = base.path().join("a");
        let b = base.path().join("b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let data = base.path().join("data");
        assert_ne!(
            storage_directory_in(&a, Some(&data)),
            storage_directory_in(&b, Some(&data))
        );
        assert_eq!(
            storage_directory_in(&a, Some(&data)),
            storage_directory_in(&a.join("."), Some(&data))
        );
        // Never source-local `.entmoot`.
        assert_eq!(
            storage_directory_in(&a, Some(&data)),
            workspace_namespace(&a, &data).join(ENTMOOT_DEFAULT_STORE)
        );
        assert!(!a.join(".entmoot").exists());
        assert!(!data.exists());
    }

    #[test]
    fn bootstrap_writes_home_namespace_not_source_tree() {
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("app.py"), "def leaf(): pass\n").unwrap();
        // configure_data_directory is process-global; use ENTMOOT_DATA_DIR for this test.
        std::env::set_var("ENTMOOT_DATA_DIR", data.path());
        let first = prepare_workspace(source.path()).unwrap();
        let second = prepare_workspace(source.path()).unwrap();
        assert_eq!(first.current.graph, second.current.graph);
        assert!(!source.path().join(".entmoot").exists());
        let storage = storage_directory_in(source.path(), Some(data.path()));
        assert!(storage.join("current.json").is_file());
        assert!(workspace_namespace(source.path(), data.path())
            .join(ENTMOOT_CONFIG_FILE)
            .is_file());
        let summary: super::super::WorkspaceSummary =
            serde_json::from_slice(&fs::read(&first.summary).unwrap()).unwrap();
        assert!(summary.structural_only);
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("app.py"), "def other(): pass\n").unwrap();
        // Damaged pointer under the correct namespace must not be silently replaced.
        let pointer = storage.join("current.json");
        fs::write(&pointer, "invalid").unwrap();
        assert!(prepare_workspace(source.path()).is_err());
        assert_eq!(fs::read(&pointer).unwrap(), b"invalid");
        std::env::remove_var("ENTMOOT_DATA_DIR");
    }

    #[test]
    fn config_volume_override_is_honored() {
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path()).ok();
        let namespace = workspace_namespace(source.path(), data.path());
        fs::create_dir_all(&namespace).unwrap();
        let volume = data.path().join("custom-volume");
        let mut config = EntmootConfig::empty_local();
        config.volume = Some(volume.display().to_string());
        fs::write(
            namespace.join(ENTMOOT_CONFIG_FILE),
            serde_json::to_vec_pretty(&config).unwrap(),
        )
        .unwrap();
        assert_eq!(
            storage_directory_in(source.path(), Some(data.path())),
            volume
        );
    }
}
