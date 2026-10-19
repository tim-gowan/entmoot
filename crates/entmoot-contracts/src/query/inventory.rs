//! Paginated inventory subject listing (read-only review projection).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::query::{QueryGapCode, ResultBounds, SchemaJson, SubjectCandidate};
use crate::SourceSnapshot;

/// Wire schema id for [`InventoryReport`].
pub const INVENTORY_REPORT_SCHEMA: &str = "entmoot.inventory.v1";

/// Default page size for inventory listing.
pub const DEFAULT_INVENTORY_LIMIT: usize = 50;

/// Maximum page size for inventory listing.
pub const MAX_INVENTORY_LIMIT: usize = 500;

/// Read-only request to page inventory subjects in one publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryRequest {
    /// Shared inventory predicates, intersected with kinds and legacy path prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<super::SubjectSelector>,
    /// Legacy kind allowlist, intersected with selector. Empty uses selector kinds
    /// when supplied, otherwise callable only.
    #[serde(default = "default_inventory_kinds")]
    pub kinds: Vec<String>,
    /// Optional repository-relative POSIX path prefix filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Zero-based offset into the filtered subject set.
    #[serde(default)]
    pub offset: usize,
    /// Maximum subjects returned in this page.
    #[serde(default = "default_inventory_limit")]
    pub limit: usize,
}

fn default_inventory_kinds() -> Vec<String> {
    Vec::new()
}

fn default_inventory_limit() -> usize {
    DEFAULT_INVENTORY_LIMIT
}

impl Default for InventoryRequest {
    fn default() -> Self {
        Self {
            selector: None,
            kinds: default_inventory_kinds(),
            path: None,
            offset: 0,
            limit: DEFAULT_INVENTORY_LIMIT,
        }
    }
}

/// Paginated inventory subjects for one publication.
///
/// Empty pages are not a negative claim about repository contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryReport {
    /// Counts over the selected inventory universe before paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<super::SelectionReceipt>,
    /// Version of this report envelope.
    pub schema_version: String,
    /// Exact inventory snapshot.
    pub snapshot: SchemaJson<SourceSnapshot>,
    /// Applied request (after clamp).
    pub request: InventoryRequest,
    /// Ordered subject cards for this page.
    pub items: Vec<SubjectCandidate>,
    /// Pagination receipt.
    pub bounds: ResultBounds,
    /// Explicit gaps / interpretation limits.
    pub gaps: Vec<QueryGapCode>,
    /// Human-readable limitations for consumers.
    pub limitations: Vec<String>,
}

impl InventoryReport {
    /// Construct a report with the current schema version.
    #[must_use]
    pub fn new(snapshot: SourceSnapshot, request: InventoryRequest) -> Self {
        Self {
            selection: None,
            schema_version: INVENTORY_REPORT_SCHEMA.into(),
            snapshot: SchemaJson::new(snapshot),
            request,
            items: Vec::new(),
            bounds: ResultBounds::complete(0),
            gaps: Vec::new(),
            limitations: vec![
                "Inventory pages list materialized subjects only; empty is not absence.".into(),
            ],
        }
    }
}
