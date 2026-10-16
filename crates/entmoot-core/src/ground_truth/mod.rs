//! Diagnostic whole-file fixture validation; visitors use revision_pairs tests.

mod dataset;
mod policy;
mod validator;

pub use dataset::{GroundTruthCase, GroundTruthDataset};
pub use policy::{config_name_from_policy, policy_from_config_name};
pub use validator::GroundTruthValidator;

/// Resolve dataset path from env or sibling default; returns None if missing.
pub fn resolve_dataset_path() -> Option<std::path::PathBuf> {
    let path = GroundTruthDataset::default_path();
    if path.exists() {
        Some(path)
    } else {
        None
    }
}
