//! Compatibility identity is independent of a human-readable format series.
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ContentDigest;

/// Required at materialized Entmoot envelope boundaries. These describe the
/// reader/writer implementation, never a provider artifact's original format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImplementationFingerprint {
    pub contracts: ContentDigest,
    pub derivation: ContentDigest,
}

/// Local storage series; incompatible implementations still require new fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum StorageSeries {
    #[serde(rename = "entmoot.storage.v1")]
    V1,
}

/// Reader/writer compatibility receipt. SQL layout construction stays in the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageFingerprint {
    pub series: StorageSeries,
    pub implementation: ImplementationFingerprint,
    pub layout: ContentDigest,
}

/// Digest the generated, golden-checked wire catalogs. Normalize checkout line
/// endings so the same Rust schemas have the same identity on every platform.
pub fn contract_fingerprint() -> ContentDigest {
    static FINGERPRINT: OnceLock<ContentDigest> = OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            let mut digest = blake3::Hasher::new();
            digest.update(b"entmoot.contract_catalog.v1\0");
            for source in [
                include_str!("../generated/entmoot_contracts.schema.json"),
                include_str!("../generated/entmoot_query_contracts.schema.json"),
            ] {
                let normalized = source.replace("\r\n", "\n");
                digest.update(&(normalized.len() as u64).to_le_bytes());
                digest.update(normalized.as_bytes());
            }
            ContentDigest::from_trusted(format!("blake3:{}", digest.finalize().to_hex()))
        })
        .clone()
}
