//! Shared bounded WAL startup for independently opened local repositories.
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
