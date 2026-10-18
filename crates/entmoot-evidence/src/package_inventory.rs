//! Validate provider package boundaries against source inventory.
use crate::{NodeKind, TopologyGraph};
use entmoot_contracts::query::QueryServiceError;
use entmoot_contracts::{address_identity_key, topology_node_address, Address, ArtifactManifest};
use std::collections::BTreeMap;
fn invalid(e: impl std::fmt::Display) -> QueryServiceError {
    QueryServiceError::InvalidRequest(e.to_string())
}
pub fn validate_package_inventory(
    graph: &TopologyGraph,
    manifest: &ArtifactManifest,
    api: Option<&entmoot_contracts::package_api::PackageApiEvidence>,
) -> Result<(), QueryServiceError> {
    let Some(api) = api else {
        return Ok(());
    };
    let mut nodes: BTreeMap<String, Vec<&crate::TopologyNode>> = BTreeMap::new();
    for node in &graph.nodes {
        for address in std::iter::once(topology_node_address(&node.id))
            .chain(node.address_id.as_ref().map(|id| Address::AddressId {
                address_id: id.clone(),
            }))
            .chain(node.logic_address.as_ref().map(|id| Address::LogicAddress {
                logic_address: id.clone(),
            }))
        {
            nodes
                .entry(address_identity_key(&address))
                .or_default()
                .push(node);
        }
    }
    for surface in &api.surfaces {
        for export in &surface.exports {
            let Some([node]) = nodes
                .get(&address_identity_key(&export.declaration))
                .map(Vec::as_slice)
            else {
                return Err(invalid(
                    "package export does not uniquely identify an inventoried declaration",
                ));
            };
            if !matches!(node.kind, NodeKind::Callable | NodeKind::Declaration)
                || !node
                    .path
                    .as_deref()
                    .is_some_and(|path| manifest.path_in_scope(path).unwrap_or(false))
            {
                return Err(invalid(
                    "package export is not a declaration in the artifact scope",
                ));
            }
        }
    }
    Ok(())
}
