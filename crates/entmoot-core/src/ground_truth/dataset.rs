//! Load explicit whole-file revision expectations for diagnostic qualification.

use std::path::{Path, PathBuf};

use entmoot_contracts::{ContentDigest, NormalizationCaseExpectation};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundTruthCase {
    pub id: String,
    pub language: String,
    pub before: String,
    pub after: String,
    pub expectation: NormalizationCaseExpectation,
}

#[derive(Debug, Clone, Deserialize)]
struct DatasetFile {
    cases: Vec<GroundTruthCase>,
}

#[derive(Debug, Clone)]
pub struct GroundTruthDataset {
    cases: Vec<GroundTruthCase>,
    digest: ContentDigest,
}

impl GroundTruthDataset {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let file: DatasetFile = serde_json::from_str(&content)?;
        Ok(Self {
            cases: file.cases,
            digest: ContentDigest::parse(entmoot_contracts::blake3_hex(content.as_bytes()))?,
        })
    }

    /// Explicit override or the focused repository fixture; no foreign fallback.
    pub fn default_path() -> PathBuf {
        if let Ok(path) = std::env::var("STRUCTURAL_HASHING_GROUND_TRUTH") {
            return PathBuf::from(path);
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/file_revision_pairs.json")
    }

    pub fn len(&self) -> usize {
        self.cases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    pub fn cases(&self) -> &[GroundTruthCase] {
        &self.cases
    }

    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}
