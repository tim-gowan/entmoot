//! Pure inventory model, identities, evidence projections and revision comparison.
pub mod display_ref;
pub mod extractor;
pub mod graph;
pub mod impact;
pub mod implementation;
pub mod knowledge;
pub mod location_join;
pub mod overlay;
pub mod relation_claims;
pub mod resolve_edges;
pub mod resolved_identity;
pub mod span_index;
pub use display_ref::{build_fqn, fqn_from_logic_address, DisplayRefKind};
pub use entmoot_contracts::artifact_data::{ArtifactDiagnostic, ArtifactError, NormalizedArtifact};
pub use extractor::{build_topology_from_records, MarkerTopologyExtractor};
pub use graph::{
    EdgeKind, GraphSummary, NodeKind, SnapshotFileDigest, StructuralEdge, TopologyGraph,
    TopologyNode, TopologyValidationError, TOPOLOGY_SCHEMA_VERSION,
};
pub use impact::{
    compare_revision_impact, compare_revision_impact_with_correspondence, ImpactError,
};
pub use implementation::implementation_fingerprint;
pub use knowledge::{inventory_skeleton, KnowledgeStore};
pub use location_join::{resolve_location, AttachmentIndexes};
pub use overlay::{compare_inventory_overlay, OverlayError};
pub use relation_claims::{
    record_structural_resolved_call, record_structural_unresolved_invoke, upsert_relation_claim,
    upsert_relation_edge,
};
pub use resolved_identity::{
    derive_resolved_identities_from_graph, derive_resolved_identity,
    inventoried_calls_for_callable, InventoriedReference, RESOLUTION_POLICY_ID,
};
pub use span_index::{CallableMatch, SpanIndex};
