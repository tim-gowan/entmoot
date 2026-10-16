//! Maintained graph and coverage projections.

pub mod coverage;
pub mod json;

pub use coverage::{build_coverage_report, AttachmentDensity, CoverageReport, ProviderCoverage};
pub use json::{load_graph_json, write_graph_json};
