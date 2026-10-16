//! Local gitoxide-backed implementation of [`super::SourceStore`].
//!
//! Only this module imports gix. Source reads remain expressed as snapshot/path
//! requests so a remote Git implementation does not inherit checkout paths.

use std::path::{Path, PathBuf};

use super::{
    local_repository_id, normalize_source_path, SourceFile, SourceObservation, SourceReadRequest,
    SourceStore, SourceStoreError,
};

/// SourceStore backed by a configured local checkout.
#[derive(Debug, Clone)]
pub struct GixSourceStore {
    root: PathBuf,
}

impl GixSourceStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, SourceStoreError> {
        let root = std::fs::canonicalize(root.as_ref()).map_err(|source| SourceStoreError::Io {
            path: root.as_ref().display().to_string(),
            source,
        })?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn committed_file(
        &self,
        request: SourceReadRequest<'_>,
    ) -> Result<SourceFile, SourceStoreError> {
        let unavailable =
            |e: &dyn std::fmt::Display| SourceStoreError::SnapshotUnavailable(e.to_string());
        if !entmoot_contracts::is_immutable_revision(&request.snapshot.revision) {
            return Err(SourceStoreError::SnapshotUnavailable(
                "Git source requires an immutable revision".into(),
            ));
        }
        let path = normalize_source_path(request.path)?;
        let repo = gix::discover(&self.root).map_err(|e| unavailable(&e))?;
        let id = repo
            .rev_parse_single(request.snapshot.revision.as_str())
            .map_err(|e| unavailable(&e))?;
        let commit = id
            .object()
            .map_err(|e| unavailable(&e))?
            .peel_to_commit()
            .map_err(|e| unavailable(&e))?;
        let tree = commit.tree().map_err(|e| unavailable(&e))?;
        let tree_path = if let Some(workdir) = repo.workdir() {
            let workdir = std::fs::canonicalize(workdir).map_err(|e| unavailable(&e))?;
            self.root
                .strip_prefix(workdir)
                .map_err(|e| unavailable(&e))?
                .join(&path)
        } else {
            PathBuf::from(&path)
        };
        let entry = tree
            .lookup_entry_by_path(tree_path)
            .map_err(|e| unavailable(&e))?
            .ok_or_else(|| {
                SourceStoreError::SnapshotUnavailable(format!(
                    "Git path {path} is absent at the requested revision"
                ))
            })?;
        if !entry.mode().is_blob() {
            return Err(SourceStoreError::SnapshotUnavailable(
                "Git entry is not a regular file".into(),
            ));
        }
        let header = repo
            .find_header(entry.object_id())
            .map_err(|e| unavailable(&e))?;
        if header.size() > super::MAX_SOURCE_FILE_BYTES {
            return Err(SourceStoreError::SnapshotUnavailable(
                "source file exceeds 64 MiB read budget".into(),
            ));
        }
        let blob = entry
            .object()
            .map_err(|e| unavailable(&e))?
            .try_into_blob()
            .map_err(|e| unavailable(&e))?;
        verified(path, blob.data.to_vec(), request.expected_content_digest)
    }
}

impl SourceStore for GixSourceStore {
    fn observe(&self) -> Result<SourceObservation, SourceStoreError> {
        Ok(match gix::discover(&self.root) {
            Ok(repo) => {
                let workdir = repo.workdir().unwrap_or(&self.root);
                SourceObservation {
                    repository_id: local_repository_id(workdir),
                    revision: head_oid_hex(&repo),
                    // Observation describes the working tree. Explicit committed
                    // requests are served separately from immutable Git objects.
                    source_state: entmoot_contracts::SourceState::WorkingTree,
                }
            }
            Err(_) => SourceObservation {
                repository_id: local_repository_id(&self.root),
                revision: None,
                // No usable `.git`: require a content digest rather than claiming committed.
                source_state: entmoot_contracts::SourceState::WorkingTree,
            },
        })
    }

    fn read_path(&self, request: SourceReadRequest<'_>) -> Result<SourceFile, SourceStoreError> {
        if request.snapshot.source_state == entmoot_contracts::SourceState::Committed {
            return self.committed_file(request);
        }
        match self.working_file(request) {
            Ok(file) => Ok(file),
            Err(error @ SourceStoreError::ContentDigestMismatch { .. })
            | Err(error @ SourceStoreError::Io { .. }) => {
                self.committed_file(request).or(Err(error))
            }
            Err(error) => Err(error),
        }
    }

    fn read_immutable(
        &self,
        request: SourceReadRequest<'_>,
    ) -> Result<Option<SourceFile>, SourceStoreError> {
        // Unavailable Git objects are not evidence of absent source. The caller
        // retains verified working bytes instead.
        Ok(self.committed_file(request).ok())
    }
}

impl GixSourceStore {
    fn working_file(&self, request: SourceReadRequest<'_>) -> Result<SourceFile, SourceStoreError> {
        let path = normalize_source_path(request.path)?;
        let mut candidate = self.root.clone();
        for part in path.split('/') {
            candidate.push(part);
        }
        let resolved =
            std::fs::canonicalize(&candidate).map_err(|source| SourceStoreError::Io {
                path: path.clone(),
                source,
            })?;
        if !resolved.starts_with(&self.root) {
            return Err(SourceStoreError::PathEscape { path });
        }
        let file = std::fs::File::open(&resolved).map_err(|source| SourceStoreError::Io {
            path: path.clone(),
            source,
        })?;
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(super::MAX_SOURCE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| SourceStoreError::Io {
                path: path.clone(),
                source,
            })?;
        if bytes.len() as u64 > super::MAX_SOURCE_FILE_BYTES {
            return Err(SourceStoreError::SnapshotUnavailable(
                "source file exceeds 64 MiB read budget".into(),
            ));
        }
        verified(path, bytes, request.expected_content_digest)
    }
}

fn verified(path: String, bytes: Vec<u8>, expected: &str) -> Result<SourceFile, SourceStoreError> {
    let observed = entmoot_contracts::blake3_hex(&bytes);
    if observed != expected {
        return Err(SourceStoreError::ContentDigestMismatch {
            path,
            expected: expected.to_owned(),
            observed,
        });
    }
    Ok(SourceFile {
        path,
        content_digest: observed,
        bytes,
    })
}

fn head_oid_hex(repo: &gix::Repository) -> Option<String> {
    if let Ok(id) = repo.head_id() {
        let hex = id.to_hex().to_string();
        if entmoot_contracts::is_immutable_revision(&hex) {
            return Some(hex);
        }
    }
    let id = repo.rev_parse_single("HEAD").ok()?;
    let hex = id.to_hex().to_string();
    entmoot_contracts::is_immutable_revision(&hex).then_some(hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_store::local_repository_id;
    use entmoot_contracts::{SourceSnapshot, SourceState};

    #[test]
    fn local_repository_id_uses_basename_not_windows_path() {
        let id = local_repository_id(Path::new(r"\\?\G:\GitHub\juice-shop"));
        assert_eq!(id, "local/juice-shop");
        assert!(!id.contains('\\'));
        assert!(!id.contains(':'));
    }

    #[test]
    fn gix_source_store_observes_configured_root() {
        let store = GixSourceStore::open(".").unwrap();
        let facts = store.observe().unwrap();
        assert!(facts.repository_id.starts_with("local/"));
        assert!(!facts.repository_id.contains('\\'));
        assert_eq!(
            facts.source_state,
            entmoot_contracts::SourceState::WorkingTree,
            "checkout-backed observations require a digest"
        );
        if let Some(rev) = &facts.revision {
            assert!(entmoot_contracts::is_immutable_revision(rev), "rev={rev}");
        }
    }

    #[test]
    fn reads_only_relative_digest_matched_paths() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("main.rs"), b"fn main() {}\n").unwrap();
        let store = GixSourceStore::open(directory.path()).unwrap();
        let snapshot = SourceSnapshot {
            repository_id: "example/source".into(),
            revision: "a".repeat(40),
            source_state: SourceState::WorkingTree,
            snapshot_digest: Some(entmoot_contracts::blake3_hex(b"manifest")),
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let digest = entmoot_contracts::blake3_hex(b"fn main() {}\n");
        let file = store
            .read_path(SourceReadRequest {
                snapshot: &snapshot,
                path: "main.rs",
                expected_content_digest: &digest,
            })
            .unwrap();
        assert_eq!(file.bytes, b"fn main() {}\n");

        let escaped = store.read_path(SourceReadRequest {
            snapshot: &snapshot,
            path: "../main.rs",
            expected_content_digest: &digest,
        });
        assert!(matches!(escaped, Err(SourceStoreError::InvalidPath(_))));

        let mismatch = store.read_path(SourceReadRequest {
            snapshot: &snapshot,
            path: "main.rs",
            expected_content_digest: &entmoot_contracts::blake3_hex(b"different"),
        });
        assert!(matches!(
            mismatch,
            Err(SourceStoreError::ContentDigestMismatch { .. })
        ));
    }

    #[test]
    fn committed_reads_use_existing_objects_after_checkout_changes() {
        use std::process::Command;
        let directory = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(directory.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        std::fs::write(directory.path().join("source.py"), "VALUE = 1\n").unwrap();
        git(&["add", "source.py"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "one",
        ]);
        let revision = git(&["rev-parse", "HEAD"]);
        std::fs::write(directory.path().join("source.py"), "VALUE = 2\n").unwrap();
        let store = GixSourceStore::open(directory.path()).unwrap();
        let snapshot = SourceSnapshot {
            repository_id: "fixture".into(),
            revision,
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let digest = entmoot_contracts::blake3_hex(b"VALUE = 1\n");
        let read = store
            .read_path(SourceReadRequest {
                snapshot: &snapshot,
                path: "source.py",
                expected_content_digest: &digest,
            })
            .unwrap();
        assert_eq!(read.bytes, b"VALUE = 1\n");
        assert_eq!(
            std::fs::read(directory.path().join("source.py")).unwrap(),
            b"VALUE = 2\n"
        );
    }
}
