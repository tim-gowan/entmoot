//! RAII helper for atomic graph JSON replacement.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use crate::graph::TopologyGraph;

/// Resolve the existing parent, retaining Windows' extended-length path prefix.
/// `tempfile` publishes through Win32 directly rather than Rust's path conversion.
pub(crate) fn publication_path(path: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "publication requires a filename",
        )
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    Ok(std::fs::canonicalize(parent)?.join(name))
}

/// Writes graph JSON to a temp sibling and persists only after successful validate+write.
pub struct AtomicGraphWrite {
    path: PathBuf,
    temp: NamedTempFile,
}

impl AtomicGraphWrite {
    pub fn begin(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let path = publication_path(path)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let temp = NamedTempFile::new_in(parent)?;
        Ok(Self { path, temp })
    }

    pub fn write_graph(&mut self, graph: &TopologyGraph) -> Result<(), Box<dyn std::error::Error>> {
        graph.validate()?;
        self.write_json_compact(graph)?;
        Ok(())
    }

    pub fn write_json<T: serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buffered = BufWriter::new(&mut self.temp);
        serde_json::to_writer_pretty(&mut buffered, value)?;
        buffered.write_all(b"\n")?;
        buffered.flush()?;
        drop(buffered);
        self.temp.as_file_mut().sync_all()?;
        Ok(())
    }

    fn write_json_compact<T: serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buffered = BufWriter::new(&mut self.temp);
        serde_json::to_writer(&mut buffered, value)?;
        buffered.write_all(b"\n")?;
        buffered.flush()?;
        drop(buffered);
        self.temp.as_file_mut().sync_all()?;
        Ok(())
    }

    pub fn commit(self) -> Result<(), Box<dyn std::error::Error>> {
        self.temp.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }

    /// Publish a new receipt without ever replacing an existing file or symlink.
    pub fn commit_new(self) -> Result<(), Box<dyn std::error::Error>> {
        self.temp
            .persist_noclobber(&self.path)
            .map_err(|error| error.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_destinations_preserve_atomic_replace_and_no_clobber() {
        let directory = tempfile::tempdir().unwrap();
        let mut parent = directory.path().to_path_buf();
        while parent.as_os_str().len() < 280 {
            parent.push("nested-publication");
        }
        let path = parent.join("receipt.json");
        let mut first = AtomicGraphWrite::begin(&path).unwrap();
        first.write_json(&"first").unwrap();
        first.commit_new().unwrap();
        let mut conflict = AtomicGraphWrite::begin(&path).unwrap();
        conflict.write_json(&"conflict").unwrap();
        let error = conflict.commit_new().unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "\"first\"\n");
        let mut replacement = AtomicGraphWrite::begin(&path).unwrap();
        replacement.write_json(&"replacement").unwrap();
        replacement.commit().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "\"replacement\"\n");
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);
    }
}
