//! Local lifecycle locks. Never remove a lock file: its inode is the lock identity.
//! Reads only open existing locks; collection requires exclusive quiescence.
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) type StorageLease = Arc<File>;

pub(crate) fn lease_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lease");
    name.into()
}

pub(crate) fn shared(path: &Path, create: bool) -> std::io::Result<StorageLease> {
    let path = lease_path(path);
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .open(&path)?;
    file.try_lock_shared().map_err(|error| {
        std::io::Error::other(format!(
            "storage maintenance busy at {}: {error}",
            path.display()
        ))
    })?;
    Ok(Arc::new(file))
}

pub(crate) fn exclusive(path: &Path) -> std::io::Result<StorageLease> {
    let path = lease_path(path);
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    file.try_lock().map_err(|error| {
        std::io::Error::other(format!(
            "storage busy at {}; close managed readers/writers and retry: {error}",
            path.display()
        ))
    })?;
    Ok(Arc::new(file))
}
