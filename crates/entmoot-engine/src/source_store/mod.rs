//! Entmoot-owned authoritative source access (`SourceStore` capability).
//!
//! Git objects, working trees, and retained publication content implement this
//! port. Remote acquisition uses the separate effectful SourceRepository port.
//! Query consumers address source
//! by Entmoot snapshot and repository-relative path, never by vendor objects.

mod git_repository;
mod gix_store;
pub use git_repository::{GitFetchReceipt, GitFetchRequest, GitSourceRepository, SourceRepository};
mod publication;
pub use publication::PublicationSourceStore;

use entmoot_contracts::{SourceSnapshot, SourceState};
use thiserror::Error;

pub use gix_store::GixSourceStore;

/// Observed source facts used while constructing a [`SourceSnapshot`].
///
/// Do **not** key `repository_id` on `remote.origin.url` or the physical scan
/// root (`\\?\…`); default is `local/<workdir-basename>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObservation {
    pub repository_id: String,
    pub revision: Option<String>,
    /// Provenance of the bytes this implementation serves. Checkout-backed
    /// stores emit `WorkingTree`; immutable object/tree stores may emit `Committed`.
    pub source_state: SourceState,
}

/// Snapshot-addressed source request. Implementations may use a checkout, a
/// remote Git object database, or another authoritative source representation.
#[derive(Debug, Clone, Copy)]
pub struct SourceReadRequest<'a> {
    pub snapshot: &'a SourceSnapshot,
    pub path: &'a str,
    pub expected_content_digest: &'a str,
}

/// Verified bytes returned by a [`SourceStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: String,
    pub content_digest: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum SourceStoreError {
    #[error("source path is invalid: {0}")]
    InvalidPath(String),
    #[error("source snapshot is unavailable: {0}")]
    SnapshotUnavailable(String),
    #[error("source path `{path}` is unavailable: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("source path `{path}` resolved outside the configured source store")]
    PathEscape { path: String },
    #[error(
        "source content digest mismatch for `{path}`: expected {expected}, observed {observed}"
    )]
    ContentDigestMismatch {
        path: String,
        expected: String,
        observed: String,
    },
}

/// Entmoot capability for authoritative, snapshot-addressed source access.
///
/// This interface is deliberately synchronous at the reference-engine boundary.
/// A remote implementation may use a local object cache or a blocking adapter;
/// transport scheduling remains outside the identity and query contracts.
pub trait SourceStore: Send + Sync {
    fn observe(&self) -> Result<SourceObservation, SourceStoreError>;

    /// Read one repository-relative path and verify its indexed content digest.
    fn read_path(&self, request: SourceReadRequest<'_>) -> Result<SourceFile, SourceStoreError>;

    /// Optional verified immutable copy already owned by the source system.
    /// Publication can retain its digest without copying those bytes again.
    fn read_immutable(
        &self,
        _request: SourceReadRequest<'_>,
    ) -> Result<Option<SourceFile>, SourceStoreError> {
        Ok(None)
    }
}

/// A source file is bounded independently of a query's returned excerpt budget.
pub const MAX_SOURCE_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn normalize_source_path(path: &str) -> Result<String, SourceStoreError> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err(SourceStoreError::InvalidPath(path.to_owned()));
    }
    Ok(normalized)
}

pub(crate) fn local_repository_id(toplevel_or_root: &std::path::Path) -> String {
    // Parse Windows drive/UNC spellings even on Unix hosts, where `\` is an
    // ordinary filename character and verbatim prefixes never split.
    let raw = toplevel_or_root.to_string_lossy();
    let stripped = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix("//?/"))
        .unwrap_or(&raw);
    let name = toplevel_or_root
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && !s.contains('\\') && !s.contains(':'))
        .or_else(|| stripped.rsplit(['/', '\\']).find(|part| !part.is_empty()))
        .unwrap_or("repo");
    format!("local/{name}")
}
