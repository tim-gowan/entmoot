//! Compatibility facade for one release. New consumers use the owning crates.
//! Production implementation lives in topology, store, evidence, query, native, ops and workspace.
pub use entmoot_workspace::*;

pub use entmoot_workspace::resolve_edges::apply_resolved_call_edges;

/// Legacy extraction paths; source-reading construction is owned by store.
pub mod extractor {
    pub use entmoot_workspace::extractor::*;
    pub use entmoot_workspace::scan_topology::build_topology_from_scan;
}

/// Legacy storage paths; operation execution is owned by ops.
pub mod knowledge_db {
    pub use entmoot_workspace::knowledge_db::*;
    pub use entmoot_workspace::LocalOperations;
}
