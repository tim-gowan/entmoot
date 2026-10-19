//! Retained Git bytes and validation; no Git process execution.
use crate::OperationError;
use entmoot_contracts::GitTreeCapture;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn invalid(e: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(e.to_string())
}
#[derive(Debug)]
pub struct RetainedGitCapture {
    pub receipt: GitTreeCapture,
    pub blobs: CapturedBlobs,
}

/// Disposable verified blob staging. Receipts retain membership; only one file
/// is hydrated at a time during publication, validation and Git materialization.
#[derive(Debug)]
pub struct CapturedBlobs {
    paths: BTreeMap<String, PathBuf>,
    directory: tempfile::TempDir,
}

impl CapturedBlobs {
    pub fn temporary() -> Result<Self, OperationError> {
        let directory = tempfile::Builder::new()
            .prefix("entmoot-git-input-")
            .tempdir()
            .map_err(invalid)?;
        Ok(Self {
            paths: BTreeMap::new(),
            directory,
        })
    }
    pub fn new_in(parent: &Path) -> Result<Self, OperationError> {
        let directory = tempfile::Builder::new()
            .prefix("entmoot-git-input-")
            .tempdir_in(parent)
            .map_err(invalid)?;
        Ok(Self {
            paths: BTreeMap::new(),
            directory,
        })
    }
    pub fn insert(&mut self, digest: &str, bytes: &[u8]) -> Result<(), OperationError> {
        if entmoot_contracts::blake3_hex(bytes) != digest {
            return Err(invalid("Git blob commitment mismatch"));
        }
        if !self.paths.contains_key(digest) {
            let path = self.directory.path().join(self.paths.len().to_string());
            std::fs::write(&path, bytes).map_err(invalid)?;
            self.paths.insert(digest.to_owned(), path);
        }
        Ok(())
    }
    pub fn read(&self, digest: &str) -> Result<Vec<u8>, OperationError> {
        let path = self
            .paths
            .get(digest)
            .ok_or_else(|| invalid("missing Git bytes"))?;
        let bytes = std::fs::read(path).map_err(invalid)?;
        if entmoot_contracts::blake3_hex(&bytes) != digest {
            return Err(invalid("Git blob commitment mismatch"));
        }
        Ok(bytes)
    }
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\0', '\\', ':'])
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.eq_ignore_ascii_case(".git")
        })
}

fn oid(value: &str, format: &str) -> bool {
    value.len() == if format == "sha1" { 40 } else { 64 }
        && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl RetainedGitCapture {
    pub fn validate(&self) -> Result<(), OperationError> {
        let r = &self.receipt;
        r.source.snapshot.validate().map_err(invalid)?;
        if !matches!(r.object_format.as_str(), "sha1" | "sha256")
            || !oid(&r.parent_commit, &r.object_format)
            || r.files.len() > 100_000
        {
            return Err(invalid("invalid Git capture header"));
        }
        let mut previous: Option<&str> = None;
        let mut total = 0u64;
        for f in &r.files {
            if !valid_path(&f.path)
                || previous.is_some_and(|p| p >= f.path.as_str())
                || !matches!(f.mode.as_str(), "100644" | "100755" | "120000")
            {
                return Err(invalid("invalid Git capture member"));
            }
            previous = Some(&f.path);
            let bytes = self.blobs.read(f.content_digest.as_str())?;
            total = total.saturating_add(bytes.len() as u64);
            if total > 1_073_741_824
                || entmoot_contracts::blake3_hex(&bytes) != f.content_digest.as_str()
            {
                return Err(invalid("invalid Git capture bytes"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_git_blobs_verify_members_and_cleanup_without_retaining_corpus_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let mut blobs = CapturedBlobs::new_in(directory.path()).unwrap();
        let bytes = b"binary\0\xff\r\n";
        let digest = entmoot_contracts::blake3_hex(bytes);
        blobs.insert(&digest, bytes).unwrap();
        blobs.insert(&digest, bytes).unwrap();
        assert_eq!(blobs.paths.len(), 1);
        assert_eq!(blobs.read(&digest).unwrap(), bytes);
        assert!(blobs.insert(&digest, b"other").is_err());
        std::fs::write(blobs.paths.get(&digest).unwrap(), b"corrupted").unwrap();
        assert!(blobs.read(&digest).is_err());
        let path = blobs.directory.path().to_owned();
        drop(blobs);
        assert!(!path.exists());
    }
}
