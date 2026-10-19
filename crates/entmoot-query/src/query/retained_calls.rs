//! Indexed call reads keep selected nodes and evidence, never a repository graph.
use super::*;
use entmoot_contracts::{RelationEdge, UnresolvedRelation};
use std::collections::BTreeSet;

impl QueryEngine {
    pub(super) fn retained_call_inspect(
        &self,
        request: &InspectQuery,
    ) -> Result<Option<(Self, entmoot_contracts::query::QueryResult)>, QueryError> {
        let (subject, direction, after, limit) = match request {
            InspectQuery::Callers { id, after, limit } => {
                (id, Some(CallDirection::Callers), after.as_ref(), *limit)
            }
            InspectQuery::Callees { id, after, limit } => {
                (id, Some(CallDirection::Callees), after.as_ref(), *limit)
            }
            InspectQuery::Unresolved { id, limit, .. } => (id, None, None, *limit),
            _ => return Ok(None),
        };
        let limit = limit.unwrap_or(50);
        if !(1..=1000).contains(&limit) {
            return Err(QueryError::InvalidInput(
                if direction.is_some() {
                    "call-neighborhood limit must be in 1..=1000"
                } else {
                    "unresolved-attempt limit must be in 1..=1000"
                }
                .into(),
            ));
        }
        if let Some(after) = after {
            RelationEdgeId::parse(&after.relation_edge_id)
                .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
        }
        let repository = self.repository()?;
        let Some(nodes) = retained_exact_nodes(&repository, subject.trim())? else {
            return Ok(None);
        };
        let mut graph = self.graph.as_ref().clone();
        for node in nodes {
            insert_read_node(&mut graph, &repository, node)?;
        }
        let addresses = if graph.nodes.len() == 1 {
            node_addresses(&graph, &graph.nodes[0])
        } else {
            Vec::new()
        };
        let page = if let Some(direction) = direction {
            let Some(page) = repository.retained_call_edges(
                &addresses,
                direction,
                after.map(|cursor| cursor.relation_edge_id.as_str()),
                limit,
            )?
            else {
                return Ok(None);
            };
            let mut endpoints = HashSet::new();
            for edge in &page.edges {
                for address in [&edge.subject, &edge.object] {
                    if endpoints.insert(address_identity_key(address)) {
                        for node in retained_address_nodes(
                            &repository,
                            &graph.snapshot.repository_id,
                            address,
                        )? {
                            insert_read_node(&mut graph, &repository, node)?;
                        }
                    }
                }
            }
            graph.relation_edges = page.edges.clone();
            Some(page)
        } else {
            None
        };
        graph.rebuild_indexes();
        let mut projected = self.clone();
        projected.indexes = std::sync::Arc::new(QueryIndexes::new(&graph, 0));
        projected.graph = std::sync::Arc::new(graph);
        projected.deferred_navigation = None;
        let value = if let Some(direction) = direction {
            projected.call_neighborhood_page(subject, direction, after, Some(limit), page)?
        } else if let InspectQuery::Unresolved { after, .. } = request {
            projected.unresolved_page_from_repository(
                &repository,
                subject,
                after.as_ref(),
                Some(limit),
            )?
        } else {
            return Ok(None);
        };
        let attempts: Vec<UnresolvedRelation> = serde_json::from_value(
            value[if direction.is_some() {
                "unresolved_relations"
            } else {
                "items"
            }]
            .clone(),
        )?;
        let mut artifacts = BTreeSet::new();
        for attempt in attempts {
            artifacts.extend(
                attempt
                    .evidence
                    .into_iter()
                    .map(|evidence| evidence.artifact_id),
            );
            if let Some(provenance) = attempt.provenance {
                artifacts.extend(provenance.artifacts);
            }
        }
        let edges: Vec<&RelationEdge> = projected.graph.relation_edges.iter().collect();
        let Some(context) = repository.retained_relation_context(
            &edges,
            &artifacts.into_iter().collect::<Vec<_>>(),
            101,
        )?
        else {
            return Ok(None);
        };
        let graph = std::sync::Arc::make_mut(&mut projected.graph);
        graph.artifacts = context.artifacts;
        graph.resolver_coverages = context.resolver_coverages;
        graph.relation_claims = context
            .edge_claims
            .into_iter()
            .flatten()
            .chain(context.related_claims)
            .collect();
        let result = projected.finish_result(value)?;
        Ok(Some((projected, result)))
    }
}

fn insert_read_node(
    graph: &mut TopologyGraph,
    repository: &impl KnowledgeRead,
    node: TopologyNode,
) -> Result<(), QueryError> {
    if graph.node(&node.id).is_some() {
        return Ok(());
    }
    if node.kind == NodeKind::Resource {
        if let Some(resource) = repository.inventory_resource(&node.id)? {
            graph.resources.push(resource);
        }
    }
    graph.upsert_node(node);
    Ok(())
}

fn exact_match(node: &TopologyNode, subject: &str) -> bool {
    node.id == subject
        || node.display_ref == subject
        || node.address_id.as_deref() == Some(subject)
        || node.logic_address.as_deref() == Some(subject)
        || node.structural_hash.as_deref().is_some_and(|hash| {
            structural_hash_exact_selectors(hash)
                .iter()
                .any(|value| value == subject)
        })
}

fn retained_exact_nodes(
    repository: &impl KnowledgeRead,
    subject: &str,
) -> Result<Option<Vec<TopologyNode>>, QueryError> {
    let mut selectors = vec![subject.to_owned()];
    selectors.extend(structural_hash_exact_selectors(subject));
    selectors.sort();
    selectors.dedup();
    let mut after = None;
    let mut selected = Vec::new();
    loop {
        let Some(page) = repository.inventory_nodes_matching(None, &selectors, after, 16)? else {
            return Ok(None);
        };
        for node in page.nodes {
            if exact_match(&node, subject) {
                selected.push(node);
                if selected.len() == 2 {
                    return Ok(Some(selected));
                }
            }
        }
        after = page.next_ordinal;
        if after.is_none() {
            return Ok(Some(selected));
        }
    }
}

fn retained_address_nodes(
    repository: &impl KnowledgeRead,
    repository_id: &str,
    address: &Address,
) -> Result<Vec<TopologyNode>, QueryError> {
    let (exact, path) = match address {
        Address::Opaque { value, .. }
            if *address == entmoot_contracts::topology_node_address(value) =>
        {
            (vec![value.clone()], None)
        }
        Address::AddressId { address_id } => (vec![address_id.clone()], None),
        Address::LogicAddress { logic_address } => (vec![logic_address.clone()], None),
        Address::RepoPath {
            repository_id: Some(id),
            path,
            ..
        } if id == repository_id => (Vec::new(), Some(path.as_str())),
        Address::Resource { .. } => {
            let id = entmoot_contracts::resource_id(address)
                .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
            let mut nodes = Vec::new();
            for resource in repository.resources_by_logical_id(&id)? {
                if let Some(page) =
                    repository.inventory_nodes_matching(None, &[resource.occurrence_id], None, 2)?
                {
                    nodes.extend(page.nodes);
                    if nodes.len() >= 2 {
                        nodes.truncate(2);
                        break;
                    }
                }
            }
            return Ok(nodes);
        }
        _ => return Ok(Vec::new()),
    };
    let mut selected = Vec::new();
    let mut after = None;
    let key = address_identity_key(address);
    loop {
        let Some(page) = repository.inventory_nodes_matching(path, &exact, after, 16)? else {
            return Err(QueryError::Unavailable(
                "retained endpoint inventory is unavailable".into(),
            ));
        };
        for node in page.nodes {
            if node_addresses_with_resource(&node, repository_id, None)
                .iter()
                .any(|candidate| address_identity_key(candidate) == key)
            {
                selected.push(node);
                if selected.len() == 2 {
                    return Ok(selected);
                }
            }
        }
        after = page.next_ordinal;
        if after.is_none() {
            return Ok(selected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};
    use entmoot_contracts::query::QueryRequest;

    #[test]
    fn retained_call_pages_match_full_reads_without_navigation_or_unrelated_nodes() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("a.py"),
            "def leaf():\n    return 1\ndef caller():\n    leaf()\n    missing()\n",
        )
        .unwrap();
        std::fs::write(
            source.path().join("b.py"),
            "def unrelated():\n    return 2\n",
        )
        .unwrap();
        let graph = crate::build_topology_from_repo_with_options(source.path(), Default::default())
            .unwrap();
        let id = |name: &str| {
            graph
                .nodes
                .iter()
                .find(|node| node.kind == NodeKind::Callable && node.label == name)
                .unwrap()
                .id
                .clone()
        };
        let caller = id("caller");
        let leaf = id("leaf");
        let unrelated = id("unrelated");
        let path = data.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        drop(repository);
        let eager = QueryEngine::new(graph, &path);
        let retained = QueryEngine::from_retained(&path, Default::default()).unwrap();
        let first = retained
            .inspect(InspectQuery::Callees {
                id: caller.clone(),
                after: None,
                limit: Some(1),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(
            first["bounds"]["total_matches"], 2,
            "aggregate and callsite are distinct occurrences"
        );
        assert_eq!(
            first["unresolved_bounds"]["total_matches"], 1,
            "missing binding remains explicit"
        );
        let cursor: entmoot_contracts::query::CallPageCursor =
            serde_json::from_value(first["next_cursor"].clone()).unwrap();
        let next_request = InspectQuery::Callees {
            id: caller.clone(),
            after: Some(cursor.clone()),
            limit: Some(1),
        };
        let second = retained
            .inspect(next_request.clone())
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(
            second,
            eager.inspect(next_request).unwrap().into_value().unwrap()
        );
        assert!(second["next_cursor"].is_null());
        assert_ne!(
            first["items"][0]["relation"]["id"],
            second["items"][0]["relation"]["id"]
        );
        assert!(
            matches!(
                retained.inspect(InspectQuery::Callers {
                    id: leaf.clone(),
                    after: Some(cursor),
                    limit: Some(1)
                }),
                Err(QueryError::InvalidInput(_))
            ),
            "cursor must bind subject and direction"
        );
        let requests = [
            ("callees", caller.as_str()),
            ("callers", leaf.as_str()),
            ("unresolved", caller.as_str()),
            ("callers", "missing-subject"),
        ]
        .map(|(kind, id)| {
            serde_json::from_value::<QueryRequest>(
                json!({"operation":"inspect","request":{"kind":kind,"id":id,"limit":1}}),
            )
            .unwrap()
        });
        let expected: Vec<_> = requests
            .iter()
            .map(|request| eager.execute(request.clone()).unwrap())
            .collect();
        for (request, expected) in requests.iter().zip(&expected) {
            assert_eq!(
                serde_json::to_value(retained.execute(request.clone()).unwrap()).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        assert!(
            retained
                .deferred_navigation
                .as_ref()
                .unwrap()
                .get()
                .is_none(),
            "call reads must not materialize repository navigation"
        );
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute(
            "UPDATE inventory_node_versions SET label='corrupted' WHERE node_id=?1",
            [unrelated],
        )
        .unwrap();
        drop(db);
        let reopened = QueryEngine::from_retained(&path, Default::default()).unwrap();
        for (request, expected) in requests.iter().zip(&expected) {
            assert_eq!(
                serde_json::to_value(reopened.execute(request.clone()).unwrap()).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        assert!(
            reopened.graph().is_err(),
            "full projection must still detect unrelated corruption"
        );
    }
}
