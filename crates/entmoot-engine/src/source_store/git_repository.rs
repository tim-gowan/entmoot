//! Effectful Git acquisition is separate from read-only SourceStore. Git owns
//! transfer, authentication, object deduplication and retention refs; no clone,
//! checkout, merge, or reset is performed by this interface.
use super::SourceStoreError;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct GitFetchRequest<'a> {
    pub remote: &'a str,
    pub revision: &'a str,
    pub timeout: std::time::Duration,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GitFetchReceipt {
    pub revision: String,
    pub retained_ref: String,
}

/// Operator-owned acquisition port. Query operations must never invoke it.
pub trait SourceRepository {
    fn fetch_revision(
        &self,
        request: GitFetchRequest<'_>,
    ) -> Result<GitFetchReceipt, SourceStoreError>;
}

pub struct GitSourceRepository {
    root: PathBuf,
}
impl GitSourceRepository {
    pub fn open(root: &Path) -> Result<Self, SourceStoreError> {
        let root = std::fs::canonicalize(root).map_err(|source| SourceStoreError::Io {
            path: root.display().to_string(),
            source,
        })?;
        gix::discover(&root).map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?;
        Ok(Self { root })
    }
}

impl SourceRepository for GitSourceRepository {
    fn fetch_revision(
        &self,
        request: GitFetchRequest<'_>,
    ) -> Result<GitFetchReceipt, SourceStoreError> {
        let invalid = |message: &str| SourceStoreError::SnapshotUnavailable(message.into());
        if !entmoot_contracts::is_immutable_revision(request.revision)
            || request.remote.is_empty()
            || request.remote.starts_with('-')
            || request.remote.chars().any(char::is_control)
            || request.timeout.is_zero()
            || request.timeout > std::time::Duration::from_secs(3600)
        {
            return Err(invalid("fetch requires an immutable Git revision, explicit remote, and timeout in (0,3600] seconds"));
        }
        let retained_ref = format!("refs/entmoot/retained/{}", request.revision);
        let refspec = format!("{}:{retained_ref}", request.revision);
        let stderr = tempfile::tempfile().map_err(|source| SourceStoreError::Io {
            path: "Git diagnostic buffer".into(),
            source,
        })?;
        let mut child = Command::new("git")
            .current_dir(&self.root)
            .args([
                "-c",
                "maintenance.auto=false",
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--no-auto-maintenance",
                "--",
                request.remote,
                &refspec,
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr.try_clone().map_err(|source| SourceStoreError::Io {
                path: "Git diagnostic buffer".into(),
                source,
            })?)
            .spawn()
            .map_err(|source| SourceStoreError::Io {
                path: self.root.display().to_string(),
                source,
            })?;
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|source| SourceStoreError::Io {
                path: "Git fetch".into(),
                source,
            })? {
                break status;
            }
            if started.elapsed() >= request.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(invalid("Git fetch exceeded its operator time budget"));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        if !status.success() {
            // Keep transport diagnostics outside portable source identities.
            // Do not include URLs or credentials from Git's stderr in receipts.
            return Err(invalid(
                "Git could not fetch the requested immutable revision from the explicit remote",
            ));
        }
        let repo = gix::discover(&self.root)
            .map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?;
        let id = repo
            .rev_parse_single(retained_ref.as_str())
            .map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?;
        if id.to_hex() != request.revision.to_ascii_lowercase() {
            return Err(invalid(
                "fetched ref does not match the requested immutable revision",
            ));
        }
        id.object()
            .map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?
            .peel_to_commit()
            .map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?;
        Ok(GitFetchReceipt {
            revision: request.revision.to_ascii_lowercase(),
            retained_ref,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_remote_fetch_reuses_repository_and_leaves_checkout_unchanged() {
        let upstream = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let git = |root: &Path, args: &[&str]| {
            let result = Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            String::from_utf8(result.stdout).unwrap().trim().to_owned()
        };
        git(upstream.path(), &["init", "-q"]);
        git(target.path(), &["init", "-q"]);
        std::fs::write(upstream.path().join("a.py"), "VALUE=1\n").unwrap();
        git(upstream.path(), &["add", "a.py"]);
        git(
            upstream.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        let revision = git(upstream.path(), &["rev-parse", "HEAD"]);
        std::fs::write(target.path().join("local.txt"), "keep").unwrap();
        let repository = GitSourceRepository::open(target.path()).unwrap();
        let remote = upstream.path().to_str().unwrap();
        let request = GitFetchRequest {
            remote,
            revision: &revision,
            timeout: std::time::Duration::from_secs(10),
        };
        repository.fetch_revision(request.clone()).unwrap();
        repository.fetch_revision(request).unwrap();
        assert!(!target.path().join("a.py").exists());
        assert_eq!(
            std::fs::read_to_string(target.path().join("local.txt")).unwrap(),
            "keep"
        );
        let source = super::super::GixSourceStore::open(target.path()).unwrap();
        let snapshot = entmoot_contracts::SourceSnapshot {
            repository_id: "fixture".into(),
            revision,
            source_state: entmoot_contracts::SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        use super::super::SourceStore;
        let digest = entmoot_contracts::blake3_hex(b"VALUE=1\n");
        assert_eq!(
            source
                .read_path(super::super::SourceReadRequest {
                    snapshot: &snapshot,
                    path: "a.py",
                    expected_content_digest: &digest
                })
                .unwrap()
                .bytes,
            b"VALUE=1\n"
        );
    }
}
