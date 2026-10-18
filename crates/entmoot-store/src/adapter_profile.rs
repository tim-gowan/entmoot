//! Stored adapter readiness; reading never performs discovery.
use crate::location::storage_directory;
use entmoot_contracts::{
    AdapterProfile, ADAPTER_PROFILE_FILE, ADAPTER_PROFILE_SCHEMA, OBSOLETE_NATIVE_PROFILE_FILE,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
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

pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let storage = storage_directory(ancestor).ok()?;
        if storage.join(ADAPTER_PROFILE_FILE).is_file() || storage.join("current.json").is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}
