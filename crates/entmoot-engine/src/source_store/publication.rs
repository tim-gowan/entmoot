use super::*;
use std::path::{Path, PathBuf};

/// Digest-verified retained source. Each read opens a short read transaction;
/// the adapter is shareable without a connection mutex and never fills caches.
pub struct PublicationSourceStore {
    store: PathBuf,
    fallback: Option<std::sync::Arc<dyn SourceStore>>,
}

impl PublicationSourceStore {
    pub fn new(store: &Path) -> Self {
        Self {
            store: store.to_owned(),
            fallback: None,
        }
    }
    pub fn with_fallback(mut self, fallback: std::sync::Arc<dyn SourceStore>) -> Self {
        self.fallback = Some(fallback);
        self
    }
}

impl SourceStore for PublicationSourceStore {
    fn observe(&self) -> Result<SourceObservation, SourceStoreError> {
        Err(SourceStoreError::SnapshotUnavailable(
            "publication stores require an explicit snapshot".into(),
        ))
    }

    fn read_path(&self, request: SourceReadRequest<'_>) -> Result<SourceFile, SourceStoreError> {
        let repository = crate::SqliteKnowledgeRepository::open_read_snapshot(&self.store)
            .map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?;
        if let Some(file) = repository
            .retained_source(request)
            .map_err(|e| SourceStoreError::SnapshotUnavailable(e.to_string()))?
        {
            return Ok(file);
        }
        self.fallback
            .as_ref()
            .ok_or_else(|| {
                SourceStoreError::SnapshotUnavailable("source digest retained without bytes".into())
            })?
            .read_path(request)
    }
}
