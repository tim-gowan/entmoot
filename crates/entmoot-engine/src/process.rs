//! Bounded process-tree lifetime shared by native adapters and explicit validation.
use std::{
    process::{Command, ExitStatus},
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProcessError {
    #[error("process ownership revoked")]
    Cancelled,
    #[error("process wall-clock budget exhausted")]
    TimedOut,
    #[error("process wait failed: {0}")]
    Wait(std::io::Error),
}

pub(crate) struct ProcessResult {
    pub status: Result<ExitStatus, ProcessError>,
    pub cleanup_error: Option<std::io::Error>,
}

pub(crate) fn run(command: Command, wall: Duration) -> std::io::Result<ProcessResult> {
    run_inner(
        command,
        wall,
        std::env::var_os("ENTMOOT_OWNED_PROCESS_GROUP").is_some(),
        || true,
    )
}

/// The controller runs outside any process wait and can revoke execution. Callers
/// may renew a durable lease here; they must not hold a transaction between polls.
pub(crate) fn run_controlled(
    command: Command,
    wall: Duration,
    keep_running: impl FnMut() -> bool,
) -> std::io::Result<ProcessResult> {
    run_inner(command, wall, false, keep_running)
}

fn run_inner(
    command: Command,
    wall: Duration,
    _inherit_group: bool,
    mut keep_running: impl FnMut() -> bool,
) -> std::io::Result<ProcessResult> {
    let start = Instant::now();
    let mut command = process_wrap::std::CommandWrap::from(command);
    #[cfg(unix)]
    if !_inherit_group {
        command.wrap(process_wrap::std::ProcessGroup::leader());
    }
    #[cfg(windows)]
    {
        command.wrap(process_wrap::std::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::std::JobObject);
    }
    let admitted = keep_running();
    if wall.is_zero() || !admitted {
        return Ok(ProcessResult {
            status: Err(if admitted {
                ProcessError::TimedOut
            } else {
                ProcessError::Cancelled
            }),
            cleanup_error: None,
        });
    }
    let mut child = command.spawn()?;
    let mut checked = Instant::now();
    let status = loop {
        if checked.elapsed() >= Duration::from_millis(100) {
            if !keep_running() {
                break Err(ProcessError::Cancelled);
            }
            checked = Instant::now();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() >= wall => break Err(ProcessError::TimedOut),
            Ok(None) => std::thread::sleep(
                Duration::from_millis(10).min(wall.saturating_sub(start.elapsed())),
            ),
            Err(error) => break Err(ProcessError::Wait(error)),
        }
    };
    let cleanup_error = child.start_kill().err().filter(|error| {
        // A reaped Unix group has no remaining members to kill. This is the
        // normal successful-exit case, not a cleanup failure.
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ESRCH) {
            return false;
        }
        let _ = error;
        true
    });
    if status.is_err() {
        let cleanup = Instant::now();
        while cleanup.elapsed() < Duration::from_millis(250) {
            if !matches!(child.try_wait(), Ok(None)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(ProcessResult {
        status,
        cleanup_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper(marker: &std::path::Path, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process::tests::child",
                "--ignored",
                "--nocapture",
            ])
            .env("ENTMOOT_PROCESS_MARKER", marker)
            .env("ENTMOOT_PROCESS_MODE", mode)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    #[test]
    #[ignore = "subprocess fixture only"]
    fn child() {
        let marker = std::path::PathBuf::from(std::env::var_os("ENTMOOT_PROCESS_MARKER").unwrap());
        match std::env::var("ENTMOOT_PROCESS_MODE").unwrap().as_str() {
            "tree" => {
                let mut child = helper(&marker, "leaf").spawn().unwrap();
                child.wait().unwrap();
            }
            "nested" => {
                let result = run(helper(&marker, "leaf"), Duration::from_secs(10)).unwrap();
                assert!(result.status.unwrap().success());
            }
            "leaf" => {
                std::fs::write(marker.with_extension("ready"), b"ready").unwrap();
                std::thread::sleep(Duration::from_millis(1500));
                std::fs::write(marker, b"escaped").unwrap();
            }
            "exit" => std::fs::write(marker, b"normal").unwrap(),
            _ => panic!("unknown child mode"),
        }
    }

    #[test]
    fn revoked_process_tree_cannot_finish_after_controller_returns() {
        for mode in ["tree", "nested"] {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("cancelled");
            let mut command = helper(&marker, mode);
            command.env("ENTMOOT_OWNED_PROCESS_GROUP", "1");
            let result = run_controlled(command, Duration::from_secs(10), || {
                !marker.with_extension("ready").is_file()
            })
            .unwrap();
            assert!(matches!(result.status, Err(ProcessError::Cancelled)));
            assert!(result.cleanup_error.is_none(), "{:?}", result.cleanup_error);
            std::thread::sleep(Duration::from_millis(1800));
            assert!(!marker.exists());
        }
    }

    #[test]
    fn timeout_cleans_tree_and_denied_admission_never_spawns() {
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        let result =
            run_controlled(helper(&denied, "exit"), Duration::from_secs(5), || false).unwrap();
        assert!(matches!(result.status, Err(ProcessError::Cancelled)));
        assert!(!denied.exists());
        let marker = dir.path().join("timeout");
        let result = run(helper(&marker, "tree"), Duration::from_millis(1000)).unwrap();
        assert!(matches!(result.status, Err(ProcessError::TimedOut)));
        assert!(result.cleanup_error.is_none());
        std::thread::sleep(Duration::from_millis(1800));
        assert!(!marker.exists());
    }

    #[test]
    fn successful_child_is_reaped_without_false_cleanup_failure() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("success");
        let result = run(helper(&marker, "exit"), Duration::from_secs(10)).unwrap();
        assert!(result.status.unwrap().success());
        assert!(result.cleanup_error.is_none(), "{:?}", result.cleanup_error);
        assert_eq!(std::fs::read(marker).unwrap(), b"normal");
    }
}
