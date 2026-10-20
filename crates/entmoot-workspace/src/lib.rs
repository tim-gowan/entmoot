//! Local CLI/MCP composition and publication lifecycle.
pub mod workspace;
pub use entmoot_ops::*;
pub use entmoot_store::{
    bind_unbound_edges, contract_boundaries, contract_callee_set, contract_caller_set,
    contract_components, contract_inventory, contract_neighborhood, contract_path, contract_relay,
    contract_status, find_contracts, inspect_contract, ContractBoundaries, ContractCallSet,
    ContractComponents, ContractInspection, ContractInventory, ContractMatch, ContractNeighborhood,
    ContractPath, ContractRelay, ContractSnapshotError, ContractStatus, UnboundEdge,
    FACT_GRAPH_PUBLICATION, FACT_NATIVE_DISPATCH, FACT_NON_CALLABLE, FACT_OTHER_EVIDENCE,
    FACT_PROVIDER_ROLES, FACT_SOURCE_TEXT,
};
pub use workspace::{open_query_bundle, prepare_workspace, DeferredQueryBundle, OpenedQueryBundle};
