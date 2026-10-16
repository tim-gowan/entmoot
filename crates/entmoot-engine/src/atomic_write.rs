//! RAII helper for atomic graph JSON replacement.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use crate::graph::TopologyGraph;

/// Writes graph JSON to a temp sibling and persists only after successful validate+write.
pub struct AtomicGraphWrite {
    path: PathBuf,
    temp: NamedTempFile,
}

impl AtomicGraphWrite {
    pub fn begin(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let temp = NamedTempFile::new_in(parent)?;
        Ok(Self {
            path: path.to_path_buf(),
            temp,
        })
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
