//! Local lifecycle locks. Never remove a lock file: its inode is the lock identity.
//! Reads only open existing locks; collection requires exclusive quiescence.
//!
//! Abrupt process exit can leave the OS holding a flock until FDs close. Match the
//! initialization gate: retry `WouldBlock` briefly so a survivor can reclaim the
//! store after a crashed peer without waiting on a live long-held lease forever.
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) type StorageLease = Arc<File>;

/// Budget for reclaiming a lease after an abrupt peer exit (FD close / unlock).
const LIFECYCLE_LOCK_WAIT: Duration = Duration::from_secs(5);

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
    wait_lock(&file, &path, true)?;
    Ok(Arc::new(file))
}

pub(crate) fn exclusive(path: &Path) -> std::io::Result<StorageLease> {
    let path = lease_path(path);
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    wait_lock(&file, &path, false)?;
    Ok(Arc::new(file))
}

fn wait_lock(file: &File, path: &Path, shared: bool) -> std::io::Result<()> {
    let started = Instant::now();
    loop {
        let attempt = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match attempt {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < LIFECYCLE_LOCK_WAIT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                let kind = if shared {
                    "storage maintenance busy"
                } else {
                    "storage busy; close managed readers/writers and retry"
                };
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("{kind} at {}", path.display()),
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// Serialize only connection initialization, independently of lifetime leases.
/// The persistent sidecar shares the administrative `.lease` convention so
/// retention never hashes bytes locked by another process on Windows.
pub(crate) fn initialization(path: &Path) -> std::io::Result<File> {
    let path = initialization_path(path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    lock_initialization(file, &path)
}

/// Participate in managed startup without creating or changing sidecars. Legacy
/// stores without this lock still reach the normal schema/fingerprint preflight.
pub(crate) fn initialization_read_only(path: &Path) -> std::io::Result<Option<File>> {
    let path = initialization_path(path);
    match OpenOptions::new().read(true).open(&path) {
        Ok(file) => lock_initialization(file, &path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn initialization_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".startup.lease");
    PathBuf::from(name)
}

fn lock_initialization(file: File, path: &Path) -> std::io::Result<File> {
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
        let blocked_reader = initialization_read_only(&path);
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "child did not acquire the initialization lock");
        assert!(matches!(blocked, Err(error) if error.kind() == std::io::ErrorKind::TimedOut));
        assert!(
            matches!(blocked_reader, Err(error) if error.kind() == std::io::ErrorKind::TimedOut)
        );
        let recovered = initialization(&path).unwrap();
        drop(recovered);
        assert!(initialization_read_only(&path).unwrap().is_some());
        assert!(!path.exists(), "locking alone must not create the database");
        let absent = directory.path().join("absent.sqlite3");
        assert!(initialization_read_only(&absent).unwrap().is_none());
        assert!(
            !initialization_path(&absent).exists(),
            "read startup never creates a lock"
        );
    }

    #[test]
    fn lifecycle_lease_recovers_after_holder_exits_without_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("knowledge.sqlite3");
        // Create the sidecar so exclusive open does not fail on a missing file.
        drop(shared(&path, true).unwrap());
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage_lease::tests::lifecycle_lease_holder",
                "--ignored",
                "--nocapture",
            ])
            .env("ENTMOOT_TEST_LIFECYCLE_PATH", &path)
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
                Err(_) => break false,
            }
            if line.contains("lifecycle lock held") {
                break true;
            }
        };
        assert!(ready, "child did not acquire the lifecycle lease");
        // Child exits without Drop; parent must reclaim within the wait budget.
        let status = child.wait().unwrap();
        assert!(status.success());
        let recovered = exclusive(&path).unwrap();
        drop(recovered);
        assert!(shared(&path, false).is_ok());
    }

    #[test]
    #[ignore = "subprocess fixture invoked by the lifecycle recovery control"]
    fn lifecycle_lease_holder() {
        let path = std::env::var_os("ENTMOOT_TEST_LIFECYCLE_PATH").unwrap();
        let _guard = exclusive(Path::new(&path)).unwrap();
        println!("lifecycle lock held");
        // Abrupt exit: skip Drop so the parent exercises post-crash reclaim.
        let _ = std::io::stdout().flush();
        std::process::exit(0);
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
