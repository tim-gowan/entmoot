//! Provider normalization and evidence attachment.
pub mod artifact;
pub mod assertions;
pub mod export;
pub mod ingest;
pub mod package_inventory;
pub mod resolver_accounting;
pub use artifact::{
    adapter_for_manifest, decode_text_bytes, ArtifactAdapter, ArtifactDiagnostic, ArtifactError,
    CycloneDxAdapter, DiagnosticSeverity, InspectedArtifact, NormalizedArtifact, OsvScannerAdapter,
    ResourceInventoryAdapter, SarifAdapter, SemanticClusterAdapter,
};
pub use assertions::derive_assertions_for_artifact;
pub use entmoot_store::*;
pub use export::{
    build_coverage_report, load_graph_json, write_graph_json, AttachmentDensity, CoverageReport,
    ProviderCoverage,
};
pub use ingest::{ingest_artifact_manifest, ArtifactIngestStats};
