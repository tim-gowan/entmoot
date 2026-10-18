//! Digest-addressed storage for immutable native artifact bytes.

use std::io::Write;
use std::path::{Path, PathBuf};

use entmoot_contracts::{ArtifactManifest, ContentDigest, ManifestValidationError};
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactStoreError {
    #[error("artifact store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact manifest JSON error: {0}")]
    ManifestJson(#[from] serde_json::Error),
    #[error("artifact manifest validation error: {0}")]
    Manifest(#[from] ManifestValidationError),
    #[error("artifact manifest requires artifact_path")]
    MissingArtifactPath,
    #[error("artifact_path must be relative to its manifest without parent traversal")]
    UnsafeArtifactPath,
    #[error("existing object at {path} does not match digest {digest}")]
    CorruptExistingObject { path: PathBuf, digest: String },
}

#[derive(Debug, Clone)]
pub struct FilesystemArtifactObjectRepository {
    root: PathBuf,
}

impl FilesystemArtifactObjectRepository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn archive_manifest(&self, manifest_path: &Path) -> Result<PathBuf, ArtifactStoreError> {
        let manifest: ArtifactManifest = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
        manifest.validate_structure()?;
        let artifact_path = artifact_path(manifest_path, &manifest)?;
        let bytes = std::fs::read(artifact_path)?;
        manifest.verify_content_digest(&bytes)?;
        let digest = ContentDigest::parse(manifest.content_digest)
            .map_err(ManifestValidationError::InvalidContentDigest)?;
        self.put(&digest, &bytes)
    }

    fn object_path(&self, digest: &ContentDigest) -> PathBuf {
        let hex = digest
            .as_str()
            .strip_prefix("blake3:")
            .unwrap_or(digest.as_str());
        self.root.join("blake3").join(&hex[..2]).join(&hex[2..])
    }

    fn verify_existing(
        &self,
        path: &Path,
        digest: &ContentDigest,
    ) -> Result<(), ArtifactStoreError> {
        let actual = entmoot_contracts::blake3_hex(&std::fs::read(path)?);
        if actual != digest.as_str() {
            return Err(ArtifactStoreError::CorruptExistingObject {
                path: path.to_path_buf(),
                digest: digest.as_str().to_string(),
            });
        }
        Ok(())
    }
}

impl FilesystemArtifactObjectRepository {
    pub fn put(&self, digest: &ContentDigest, bytes: &[u8]) -> Result<PathBuf, ArtifactStoreError> {
        ContentDigest::parse(digest.as_str().to_string())
            .map_err(ManifestValidationError::InvalidContentDigest)?;
        let actual = entmoot_contracts::blake3_hex(bytes);
        if actual != digest.as_str() {
            return Err(ArtifactStoreError::Manifest(
                ManifestValidationError::DigestMismatch {
                    expected: digest.as_str().to_string(),
                    actual,
                },
            ));
        }
        std::fs::create_dir_all(&self.root)?;
        let _lease = crate::storage_lease::shared(&self.root.join("objects"), true)?;
        let path = self.object_path(digest);
        if path.exists() {
            self.verify_existing(&path, digest)?;
            return Ok(path);
        }
        let destination = crate::atomic_write::publication_path(&path)?;
        let parent = destination.parent().unwrap_or(&self.root);
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary.write_all(bytes)?;
        temporary.as_file_mut().sync_all()?;
        match temporary.persist_noclobber(&destination) {
            Ok(_) => Ok(path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.verify_existing(&path, digest)?;
                Ok(path)
            }
            Err(error) => Err(ArtifactStoreError::Io(error.error)),
        }
    }

    pub fn get(&self, digest: &ContentDigest) -> Result<Vec<u8>, ArtifactStoreError> {
        self.get_bounded(digest, u64::MAX)
    }

    /// Read and validate one immutable object within a caller's byte budget.
    pub fn get_bounded(
        &self,
        digest: &ContentDigest,
        max_bytes: u64,
    ) -> Result<Vec<u8>, ArtifactStoreError> {
        use std::io::Read;
        ContentDigest::parse(digest.as_str().to_string())
            .map_err(ManifestValidationError::InvalidContentDigest)?;
        let _lease = crate::storage_lease::shared(&self.root.join("objects"), false)?;
        let path = self.object_path(digest);
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(std::io::Error::other("artifact read byte budget exhausted").into());
        }
        if entmoot_contracts::blake3_hex(&bytes) != digest.as_str() {
            return Err(ArtifactStoreError::CorruptExistingObject {
                path,
                digest: digest.to_string(),
            });
        }
        Ok(bytes)
    }
}

fn artifact_path(
    manifest_path: &Path,
    manifest: &ArtifactManifest,
) -> Result<PathBuf, ArtifactStoreError> {
    let relative = manifest
        .artifact_path
        .as_deref()
        .ok_or(ArtifactStoreError::MissingArtifactPath)?;
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ArtifactStoreError::UnsafeArtifactPath);
    }
    Ok(manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_put_is_atomic_idempotent_and_verified_on_read() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemArtifactObjectRepository::new(directory.path());
        let bytes = b"immutable artifact";
        let digest = ContentDigest::parse(entmoot_contracts::blake3_hex(bytes)).unwrap();
        let first = repository.put(&digest, bytes).unwrap();
        let second = repository.put(&digest, bytes).unwrap();
        assert_eq!(first, second);
        assert_eq!(repository.get(&digest).unwrap(), bytes);

        std::fs::write(&first, b"corrupt").unwrap();
        assert!(matches!(
            repository.get(&digest),
            Err(ArtifactStoreError::CorruptExistingObject { .. })
        ));
    }

    #[test]
    fn deep_data_roots_preserve_object_publication_and_reload() {
        let directory = tempfile::tempdir().unwrap();
        let mut root = directory.path().to_path_buf();
        // The object name adds a full digest beneath a user-selected data root.
        // Exercise paths beyond Win32 MAX_PATH without oversized components.
        while root.as_os_str().len() < 280 {
            root.push("nested-workspace-data");
        }
        let repository = FilesystemArtifactObjectRepository::new(&root);
        let bytes = b"native detail in a deeply nested data root";
        let digest = ContentDigest::parse(entmoot_contracts::blake3_hex(bytes)).unwrap();
        let first = repository.put(&digest, bytes).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), bytes);
        let reopened = FilesystemArtifactObjectRepository::new(&root);
        assert_eq!(reopened.put(&digest, bytes).unwrap(), first);
        assert_eq!(reopened.get(&digest).unwrap(), bytes);
        std::fs::write(&first, b"corrupt").unwrap();
        assert!(matches!(
            reopened.put(&digest, bytes),
            Err(ArtifactStoreError::CorruptExistingObject { .. })
        ));
    }

    #[test]
    fn digest_mismatch_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemArtifactObjectRepository::new(directory.path());
        let digest = ContentDigest::parse(entmoot_contracts::blake3_hex(b"expected")).unwrap();
        assert!(repository.put(&digest, b"different").is_err());
        assert!(!repository.object_path(&digest).exists());
    }

    #[test]
    fn malformed_typed_digest_is_an_error_not_a_path_slice_panic() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemArtifactObjectRepository::new(directory.path());
        let malformed = ContentDigest::from_trusted("b");
        assert!(repository.get(&malformed).is_err());
    }
}
