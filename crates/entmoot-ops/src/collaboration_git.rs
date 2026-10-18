//! Isolated Git observations. Never runs hooks, filters or configured merge drivers.
use crate::OperationError;
use entmoot_contracts::{GitMergeObservation, SourceSnapshot, SourceState};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
mod capture;
mod integration;
pub use capture::{capture_workspace, RetainedGitCapture};
pub use integration::GitIntegration;

fn git(root: &Path, args: &[&str], scratch: &Path) -> Result<(i32, String), OperationError> {
    let (status, output) = run_git(root, args, scratch, None)?;
    Ok((status, String::from_utf8_lossy(&output).into_owned()))
}

fn run_git(
    root: &Path,
    args: &[&str],
    scratch: &Path,
    input: Option<std::fs::File>,
) -> Result<(i32, Vec<u8>), OperationError> {
    run_git_bounded(root, args, scratch, input, 32 * 1024 * 1024)
}

fn run_git_bounded(
    root: &Path,
    args: &[&str],
    scratch: &Path,
    input: Option<std::fs::File>,
    output_limit: u64,
) -> Result<(i32, Vec<u8>), OperationError> {
    let mut stdout =
        tempfile::tempfile_in(scratch).map_err(crate::KnowledgeRepositoryError::from)?;
    let stderr = tempfile::tempfile_in(scratch).map_err(crate::KnowledgeRepositoryError::from)?;
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(["-c", "core.fsmonitor=false"])
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(input.map_or_else(Stdio::null, Stdio::from))
        .stdout(
            stdout
                .try_clone()
                .map_err(crate::KnowledgeRepositoryError::from)?,
        )
        .stderr(stderr);
    let result = crate::process::run(command, Duration::from_secs(30))
        .map_err(crate::KnowledgeRepositoryError::from)?;
    let status = result
        .status
        .map_err(|e| OperationError::Invalid(e.to_string()))?;
    if let Some(error) = result.cleanup_error {
        return Err(OperationError::Invalid(error.to_string()));
    }
    use std::io::{Seek, SeekFrom};
    stdout
        .seek(SeekFrom::Start(0))
        .map_err(crate::KnowledgeRepositoryError::from)?;
    let mut output = Vec::new();
    stdout
        .take(output_limit.saturating_add(1))
        .read_to_end(&mut output)
        .map_err(crate::KnowledgeRepositoryError::from)?;
    if output.len() as u64 > output_limit {
        return Err(OperationError::Budget(
            "Git output exceeded observation budget".into(),
        ));
    }
    Ok((status.code().unwrap_or(-1), output))
}

/// Both commits are read from the explicitly supplied local object database;
/// result objects go into a private bare repository with empty Git configuration.
pub fn observe_merge(
    root: &Path,
    scratch: &Path,
    left: &SourceSnapshot,
    right: &SourceSnapshot,
) -> Result<GitMergeObservation, OperationError> {
    observe_captured_merge(root, scratch, left, right, None, None)
}

/// Captures are immutable input trees; the source root supplies parent history only.
pub fn observe_captured_merge(
    root: &Path,
    scratch: &Path,
    left: &SourceSnapshot,
    right: &SourceSnapshot,
    left_capture: Option<&RetainedGitCapture>,
    right_capture: Option<&RetainedGitCapture>,
) -> Result<GitMergeObservation, OperationError> {
    for snapshot in [left, right] {
        snapshot
            .validate()
            .map_err(|e| OperationError::Invalid(e.to_string()))?;
    }
    if left.repository_id != right.repository_id {
        return Err(OperationError::Invalid(
            "cross-repository merge observation".into(),
        ));
    }
    for (snapshot, capture) in [(left, left_capture), (right, right_capture)] {
        if let Some(capture) = capture {
            capture.validate()?;
            if capture.receipt.source.snapshot != *snapshot {
                return Err(OperationError::Invalid("capture snapshot mismatch".into()));
            }
        }
    }
    if (left.source_state != SourceState::Committed && left_capture.is_none())
        || (right.source_state != SourceState::Committed && right_capture.is_none())
    {
        return Ok(GitMergeObservation { merge_bases: vec![], status: "unavailable".into(), details: "working-tree observations require a separately captured Git tree; structural comparison remains available".into() });
    }
    let left_parent =
        left_capture.map_or(left.revision.as_str(), |c| c.receipt.parent_commit.as_str());
    let right_parent = right_capture.map_or(right.revision.as_str(), |c| {
        c.receipt.parent_commit.as_str()
    });
    if left_parent.len() != right_parent.len() {
        return Err(OperationError::Unavailable(
            "Git object formats differ".into(),
        ));
    }
    let private = private_repository(root, scratch, left_parent.len() == 64)?;
    let left_revision = match left_capture {
        Some(c) => capture::materialize(private.path(), c, "left")?,
        None => left.revision.clone(),
    };
    let right_revision = match right_capture {
        Some(c) => capture::materialize(private.path(), c, "right")?,
        None => right.revision.clone(),
    };
    let mut observation = merge_commits(private.path(), &left_revision, &right_revision)?;
    for capture in [left_capture, right_capture].into_iter().flatten() {
        for limitation in &capture.receipt.limitations {
            observation.details.push('\n');
            observation.details.push_str(limitation);
        }
    }
    Ok(observation)
}

fn private_repository(
    root: &Path,
    scratch: &Path,
    sha256: bool,
) -> Result<tempfile::TempDir, OperationError> {
    std::fs::create_dir_all(scratch).map_err(crate::KnowledgeRepositoryError::from)?;
    let private = tempfile::tempdir_in(scratch).map_err(crate::KnowledgeRepositoryError::from)?;
    let (code, objects) = git(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ],
        private.path(),
    )?;
    if code != 0 {
        return Err(OperationError::Invalid(
            "source Git object database unavailable".into(),
        ));
    }
    let objects = objects.trim();
    if objects.is_empty() || objects.contains(['\r', '\n']) {
        return Err(OperationError::Invalid(
            "unsupported object database path".into(),
        ));
    }
    let object_format = if sha256 {
        "--object-format=sha256"
    } else {
        "--object-format=sha1"
    };
    let (code, _) = git(
        private.path(),
        &["init", "--bare", "--template=", object_format, "."],
        private.path(),
    )?;
    if code != 0 {
        return Err(OperationError::Invalid(
            "cannot initialize isolated merge repository".into(),
        ));
    }
    std::fs::write(
        private.path().join("objects/info/alternates"),
        format!("{}\n", objects.replace('\\', "/")),
    )
    .map_err(crate::KnowledgeRepositoryError::from)?;
    Ok(private)
}

fn merge_commits(
    private: &Path,
    left_revision: &str,
    right_revision: &str,
) -> Result<GitMergeObservation, OperationError> {
    let (code, bases) = git(
        private,
        &["merge-base", "--all", left_revision, right_revision],
        private,
    )?;
    let merge_bases: Vec<_> = bases
        .lines()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if code != 0 || merge_bases.len() != 1 {
        return Ok(GitMergeObservation { merge_bases, status: "unavailable".into(), details: "missing history, unrelated histories or multiple merge bases; no unique baseline qualified".into() });
    }
    let (code, output) = git(
        private,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            left_revision,
            right_revision,
        ],
        private,
    )?;
    Ok(GitMergeObservation {
        merge_bases,
        status: match code {
            0 => "clean",
            1 => "conflict",
            _ => "unavailable",
        }
        .into(),
        details: if (0..=1).contains(&code) {
            format!("{output}\nGit built-in merge only; custom merge drivers are not qualified; clean does not imply semantic compatibility")
        } else {
            "Git could not establish a merge result".into()
        },
    })
}
