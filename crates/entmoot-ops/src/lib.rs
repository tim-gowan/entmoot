//! Explicit producer and collaboration execution over durable storage.
pub mod binding_operations;
pub mod canon;
pub mod collaboration;
pub mod collaboration_git;
pub mod integration_operations;
pub mod producer_operations;
pub mod revalidation_operations;
pub mod test_validation;
pub use entmoot_native::*;
pub use entmoot_query::*;

/// Compatibility name for the durable operation port; execution lives in this crate.
pub use entmoot_store::OperationRepository as LocalOperations;
