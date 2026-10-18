//! Durable knowledge, source observations and checked storage locations.
pub mod atomic_write;
pub mod knowledge_db;
pub mod storage_lease;
pub use atomic_write::AtomicGraphWrite;
pub mod adapter_profile;
pub mod analysis_plan;
pub mod analysis_work;
pub mod artifact_store;
pub mod checkpoints;
pub mod clock;
pub mod config_inventory;
pub mod inventory;
pub mod inventory_stitch;
pub mod location;
pub mod manifest_state;
pub mod revalidation;
pub mod scan_topology;
pub mod source_store;
mod sqlite_startup;
pub use analysis_plan::{plan_analysis, plan_revalidation_analysis, AnalysisPlanError};
pub use analysis_work::{AnalysisWorkError, AnalysisWorkQueue, SqliteAnalysisWorkQueue};
pub use artifact_store::{ArtifactStoreError, FilesystemArtifactObjectRepository};
pub use config_inventory::{
    apply_configuration_inventory, detect_configuration_syntax, index_configuration_source,
    index_repository_configurations, ConfigDiagnostic, ConfigInventory, ConfigInventoryError,
    ConfigSource, ConfigSyntax,
};
pub use entmoot_contracts::{
    ArtifactDiagnostic, ContentDigest, DiagnosticSeverity, ProducerSource,
};
pub use entmoot_topology::*;
pub use inventory::*;
pub use knowledge_db::{
    storage_fingerprint, BufferedPublicationReceipt, KnowledgeDependency, KnowledgeOwnershipStats,
    KnowledgeRead, KnowledgeRecordRef, KnowledgeRecordState, KnowledgeRepository,
    KnowledgeRepositoryError, KnowledgeStoreBatches, KnowledgeSyncStats, KnowledgeViewLocator,
    LocalVisibilityChange, LocalVisibilityPage, LocalVisibilityReason, OperationError,
    OperationRepository, OwnerBatch, OwnerReplacement, PublicationBuffer, PublicationOutcome,
    SqliteKnowledgeRepository,
};
pub use manifest_state::{ManifestUnverified, ManifestVerified};
pub use revalidation::{
    evaluate_artifact_revalidation, publish_artifact_evidence, publish_revalidation_ledger,
    revalidate_transition, ArtifactInputReceipt, ArtifactRevalidationReceipt, RevalidationError,
};
pub use source_store::{
    GixSourceStore, SourceFile, SourceObservation, SourceReadRequest, SourceStore, SourceStoreError,
};
pub fn graph_source(graph: &TopologyGraph) -> Result<ProducerSource, OperationError> {
    Ok(ProducerSource {
        snapshot: graph.snapshot.clone(),
        files_digest: ContentDigest::from_trusted(entmoot_contracts::blake3_hex(
            &serde_json::to_vec(&graph.snapshot_files)?,
        )),
    })
}

pub mod graph_io;
pub use graph_io::{load_graph_json, write_graph_json};

pub mod read;

pub use location::Locations;
