//! Local CLI/MCP composition and publication lifecycle.
pub mod workspace;
pub use entmoot_ops::*;
pub use workspace::{open_query_bundle, prepare_workspace, DeferredQueryBundle, OpenedQueryBundle};
