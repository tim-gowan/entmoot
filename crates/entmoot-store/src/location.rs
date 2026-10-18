//! Local frontend storage selection. Portable queries never contain database paths.
//!
//! Durable state defaults under `~/.entmoot` (or `ENTMOOT_DATA_DIR`). This module
//! selects storage independently of source trees. Per-workspace `config.json`
//! points at a volume and supplies `ENTMOOT_*` slots when process env is unset.

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
        .map_err(|_| failure("data directory already configured for this process"))
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
        return Err(failure(
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
                .map_err(|error| failure(format!("invalid {}: {error}", path.display())))?;
            if config.schema != ENTMOOT_CONFIG_SCHEMA {
                return Err(failure(format!(
                    "unsupported config schema {:?} (expected {ENTMOOT_CONFIG_SCHEMA})",
                    config.schema
                )));
            }
            if config.is_remote() {
                return Err(failure(
                    "remote storage is not implemented; set mode to local or choose a local volume",
                ));
            }
            if let Some(volume) = &config.volume {
                validate_local_data_path(Path::new(volume))?;
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
        .map_err(|error| failure(format!("serialize workspace config: {error}")))?;
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
        .map_err(|error| failure(format!("serialize global config: {error}")))?;
    fs::write(&path, bytes)?;
    Ok(config)
}

pub fn resolve_volume(namespace: &Path, config: &EntmootConfig) -> Result<PathBuf, WorkspaceError> {
    if config.is_remote() {
        return Err(failure(
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

/// Resolve the local storage volume without creating directories or config files.
///
/// Configuration is namespaced under the data root; an explicit volume can be elsewhere.
/// Invalid global/workspace configuration fails before any store is selected.
pub fn storage_directory(root: &Path) -> Result<PathBuf, WorkspaceError> {
    storage_directory_in(root, Some(&data_root()))
}

/// Checked volume lookup used by embedding applications and frontend discovery.
///
/// `data_directory` of `None` uses [`default_entmoot_home`]. When the workspace
/// config is absent, the default is its `store/` sibling. Malformed, unreadable or
/// remote configuration returns an error instead of redirecting reads or writes.
pub fn storage_directory_in(
    root: &Path,
    data_directory: Option<&Path>,
) -> Result<PathBuf, WorkspaceError> {
    Ok(Locations::resolve(root, data_directory)?.volume)
}

/// Validated local locations shared by readers, producers and orchestration.
/// Resolving locations never creates directories or writes configuration.
#[derive(Debug, Clone)]
pub struct Locations {
    namespace: PathBuf,
    volume: PathBuf,
}

impl Locations {
    /// Resolve explicit data-root selection; `None` selects the default home root.
    pub fn resolve(root: &Path, data_directory: Option<&Path>) -> Result<Self, WorkspaceError> {
        let data = data_directory
            .map(Path::to_owned)
            .unwrap_or_else(default_entmoot_home);
        let data = validate_local_data_path(&data)?;
        load_config_file(&data.join(ENTMOOT_CONFIG_FILE))?;
        let namespace = workspace_namespace(root, &data);
        let config = load_config_file(&namespace.join(ENTMOOT_CONFIG_FILE))?;
        let volume = resolve_volume(&namespace, &config)?;
        Ok(Self { namespace, volume })
    }

    pub fn namespace(&self) -> &Path {
        &self.namespace
    }
    pub fn volume(&self) -> &Path {
        &self.volume
    }
}

/// Resolve a single `ENTMOOT_*` value: process env → workspace config → global config.
/// A present process value is exclusive, including an invalid empty path. Empty
/// config slots are unset. Frontends validate config before invoking producers;
/// this optional discovery lookup ignores configuration read errors.
#[must_use]
pub fn resolve_entmoot_override(key: &str, source_root: Option<&Path>) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        return Some(value);
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

/// True when a process override or nonempty config slot is configured.
#[must_use]
pub fn entmoot_override_configured(key: &str, source_root: Option<&Path>) -> bool {
    resolve_entmoot_override(key, source_root).is_some()
}

/// Canonicalize an override path: relative values resolve against process CWD.
pub fn canonicalize_override_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("invalid_environment: override path is empty".into());
    }
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
            storage_directory_in(&a, Some(&data)).unwrap(),
            storage_directory_in(&b, Some(&data)).unwrap()
        );
        assert_eq!(
            storage_directory_in(&a, Some(&data)).unwrap(),
            storage_directory_in(&a.join("."), Some(&data)).unwrap()
        );
        // Never source-local `.entmoot`.
        assert_eq!(
            storage_directory_in(&a, Some(&data)).unwrap(),
            workspace_namespace(&a, &data).join(ENTMOOT_DEFAULT_STORE)
        );
        assert!(!a.join(".entmoot").exists());
        assert!(!data.exists());
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
            storage_directory_in(source.path(), Some(data.path())).unwrap(),
            volume
        );
    }
}

use crate::AtomicGraphWrite;
use entmoot_contracts::SourceSnapshot;
use serde::{Deserialize, Serialize};
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub fn failure(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::Invalid(error.to_string())
}

/// Machine-local paths; never a portable operation payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationLocation {
    pub graph: PathBuf,
    pub store: PathBuf,
    pub source_root: PathBuf,
}

/// Atomic local pointer. A bind of the same snapshot preserves the compare base.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePublication {
    pub schema_version: String,
    pub current: PublicationLocation,
    pub base: Option<PublicationLocation>,
    pub snapshot: SourceSnapshot,
    pub summary: PathBuf,
}

/// Resolve Git top-level, falling back to the current directory for non-Git input.
pub fn workspace_root(start: &Path) -> Result<PathBuf, WorkspaceError> {
    let start = fs::canonicalize(start)?;
    Ok(gix::discover(&start)
        .ok()
        .and_then(|repo| repo.workdir().map(Path::to_path_buf))
        .unwrap_or(start))
}

/// Read-only discovery; never creates directories or invokes producers.
pub fn discover_publication(start: &Path) -> Result<WorkspacePublication, WorkspaceError> {
    let start = fs::canonicalize(start)?;
    for ancestor in start.ancestors() {
        let pointer = storage_directory(ancestor)?.join("current.json");
        if pointer.is_file() {
            return read_publication(&pointer);
        }
    }
    Err(WorkspaceError::Invalid(
        "no current workspace publication; run entmoot init or pass --graph/--publication".into(),
    ))
}

pub fn read_publication(path: &Path) -> Result<WorkspacePublication, WorkspaceError> {
    let publication: WorkspacePublication = serde_json::from_slice(&fs::read(path)?)?;
    if publication.schema_version != "entmoot.workspace_publication.v1" {
        return Err(WorkspaceError::Invalid(
            "unsupported workspace publication schema".into(),
        ));
    }
    // Reading a pointer must not initialize an empty store.
    if !publication.current.graph.is_file() || !publication.current.store.is_file() {
        return Err(WorkspaceError::Invalid(
            "publication graph/store missing; run entmoot init".into(),
        ));
    }
    Ok(publication)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), WorkspaceError> {
    let mut write = AtomicGraphWrite::begin(path).map_err(failure)?;
    write.write_json(value).map_err(failure)?;
    write.commit().map_err(failure)
}

/// Adjacent knowledge store path used when callers omit `--store`.
pub fn default_store_path(graph_path: &Path) -> PathBuf {
    let locator = PathBuf::from(format!("{}.knowledge.view.json", graph_path.display()));
    if locator.is_file() {
        return locator;
    }
    let mut path = graph_path.as_os_str().to_os_string();
    path.push(".knowledge.sqlite3");
    PathBuf::from(path)
}

pub fn artifact_store_path(store: &Path) -> Result<PathBuf, WorkspaceError> {
    let database = crate::KnowledgeViewLocator::read(store)
        .map_err(failure)?
        .map_or_else(|| store.to_owned(), |locator| locator.database);
    let mut path = database.into_os_string();
    path.push(".objects");
    Ok(PathBuf::from(path))
}
