//! Shared local SQLite path handling and bounded WAL startup.

/// SQLite's Win32 calls need an extended-length spelling for deep data roots.
/// Resolve existing files or their parent without creating anything on reads.
pub(crate) fn file_path(path: &std::path::Path) -> std::borrow::Cow<'_, std::path::Path> {
    #[cfg(windows)]
    {
        let resolved = std::fs::canonicalize(path).or_else(|error| {
            let Some(name) = path.file_name() else {
                return Err(error);
            };
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            std::fs::canonicalize(parent).map(|parent| parent.join(name))
        });
        if let Ok(resolved) = resolved {
            return std::borrow::Cow::Owned(resolved);
        }
        // Preserve SQLite's normal open error when the filesystem cannot resolve
        // the path; never redirect to another root or initialize a missing parent.
    }
    std::borrow::Cow::Borrowed(path)
}

pub(crate) fn enable_wal(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    // Journal-mode transitions can return BUSY without honoring busy_timeout
    // while another first opener briefly owns the schema reservation.
    let started = std::time::Instant::now();
    loop {
        match connection.execute_batch("PRAGMA journal_mode=WAL;") {
            Ok(()) => return Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if matches!(
                    error.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) && started.elapsed() < std::time::Duration::from_secs(5) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}
