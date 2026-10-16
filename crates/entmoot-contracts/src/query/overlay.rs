//! Inventory-only cross-publication symbol overlay (no `package_api` required).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SourceSnapshot;

/// Schema id for inventory overlay reports.
pub const INVENTORY_OVERLAY_SCHEMA: &str = "entmoot.inventory_overlay.v1";

/// Request to compare callable/symbol presence across two publications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryOverlayRequest {
    /// Base (older / provider) publication digest or generation id.
    pub base_publication: String,
    /// Head (newer / consumer view) publication digest or generation id.
    pub head_publication: String,
    /// Optional name substrings; empty means all callables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_filters: Vec<String>,
    /// Optional repository-relative path prefixes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_prefixes: Vec<String>,
}

/// One inventory presence delta (structural identity only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryOverlayDelta {
    /// Symbol / callable name as inventoried.
    pub name: String,
    /// Repository-relative path when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Language family when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Kind of presence change.
    pub change: OverlayChangeKind,
    /// Optional `logic_address` from the side that still has the symbol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logic_address: Option<String>,
}

/// Presence change between base and head inventories.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OverlayChangeKind {
    /// Present in head, absent in base.
    Introduced,
    /// Present in base, absent in head.
    Removed,
    /// Present on both sides (scope/kind/signature presence, preserving multiplicity).
    Retained,
}

/// Inventory overlay report — not Bound/Fits and not `SemVer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryOverlayReport {
    /// Wire schema discriminator ([`INVENTORY_OVERLAY_SCHEMA`]).
    pub schema: String,
    /// Base publication source identity.
    pub base_snapshot: SourceSnapshot,
    /// Head publication source identity.
    pub head_snapshot: SourceSnapshot,
    /// Presence deltas after name/path filters.
    pub deltas: Vec<InventoryOverlayDelta>,
    /// Count of symbols present only in head.
    pub introduced: usize,
    /// Count of symbols present only in base.
    pub removed: usize,
    /// Count of symbols present on both sides.
    pub retained: usize,
    /// Explicit limitations (missing evidence ≠ absence).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}
