//! Explicit preparation composes source inventory with storage selection.
use std::{fs, path::Path};

pub use entmoot_store::location::*;
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
        let pointer = storage_directory(ancestor)?.join("current.json");
        match fs::symlink_metadata(&pointer) {
            Ok(_) => return prepare_workspace(ancestor),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    prepare_workspace(&super::workspace_root(&start)?)
}
