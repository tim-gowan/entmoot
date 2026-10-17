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

/// Serialize only connection initialization, independently of lifetime leases.
/// The persistent sidecar shares the administrative `.lease` convention so
/// retention never hashes bytes locked by another process on Windows.
pub(crate) fn initialization(path: &Path) -> std::io::Result<File> {
    let mut name = path.as_os_str().to_owned();
    name.push(".startup.lease");
    let path = PathBuf::from(name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    let started = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock)
                if started.elapsed() < std::time::Duration::from_secs(5) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("database initialization busy at {}", path.display()),
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    #[test]
    fn initialization_lock_bounds_contention_and_recovers_after_owner_exit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("knowledge.sqlite3");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage_lease::tests::initialization_lock_holder",
                "--ignored",
                "--nocapture",
            ])
            .env("ENTMOOT_TEST_INITIALIZATION_PATH", &path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        let ready = loop {
            line.clear();
            match output.read_line(&mut line) {
                Ok(0) => break false,
                Ok(_) => {}
                Err(error) => {
                    eprintln!("initialization fixture output failed: {error}");
                    break false;
                }
            }
            if line.contains("initialization lock held") {
                break true;
            }
        };
        let blocked = initialization(&path);
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "child did not acquire the initialization lock");
        assert!(matches!(blocked, Err(error) if error.kind() == std::io::ErrorKind::TimedOut));
        let recovered = initialization(&path).unwrap();
        drop(recovered);
        assert!(!path.exists(), "locking alone must not create the database");
    }

    #[test]
    #[ignore = "subprocess fixture invoked by the initialization recovery control"]
    fn initialization_lock_holder() {
        let path = std::env::var_os("ENTMOOT_TEST_INITIALIZATION_PATH").unwrap();
        let _guard = initialization(Path::new(&path)).unwrap();
        println!("initialization lock held");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
}
