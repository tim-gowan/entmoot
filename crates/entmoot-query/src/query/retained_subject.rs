//! Retained subject-context inspect keeps a path-local projection, never a
//! repository-wide navigation graph.
use super::retained_calls::{
    exact_match, insert_read_node, retained_address_nodes, retained_exact_nodes,
};
use super::*;

impl QueryEngine {
    pub(super) fn retained_subject_inspect(
        &self,
        request: &InspectQuery,
    ) -> Result<Option<(Self, entmoot_contracts::query::QueryResult)>, QueryError> {
        let InspectQuery::Subject { id, limit } = request else {
            return Ok(None);
        };
        let subject = id.trim();
        if subject.is_empty() {
            return Err(QueryError::InvalidInput("subject is required".into()));
        }
        let repository = self.repository()?;
        let Some(candidates) = retained_exact_nodes(&repository, subject)? else {
            return Ok(None);
        };
        let mut graph = self.graph.as_ref().clone();
        if candidates.is_empty() {
            // Keep deferred unset so unknown subjects do not hydrate navigation.
            let mut projected = self.clone();
            projected.indexes = std::sync::Arc::new(QueryIndexes::new(&graph, 0));
            projected.graph = std::sync::Arc::new(graph);
            projected.deferred_navigation = None;
            let value = projected.get_subject_context(subject, *limit)?;
            let result = projected.finish_result(value)?;
            return Ok(Some((projected, result)));
        }
        for node in &candidates {
            insert_read_node(&mut graph, &repository, node.clone())?;
        }
        if let Some(path) = candidates
            .first()
            .and_then(|node| node.path.as_deref())
            .filter(|path| !path.is_empty())
        {
            let mut after = None;
            loop {
                let Some(page) = repository.inventory_nodes(Some(path), after, 256)? else {
                    return Ok(None);
                };
                for node in page.nodes {
                    insert_read_node(&mut graph, &repository, node)?;
                }
                after = page.next_ordinal;
                if after.is_none() {
                    break;
                }
            }
        }
        // Parent ids may live outside the subject's path page (for example a
        // synthetic root). Follow declared parents until the chain closes.
        let mut pending: Vec<String> = graph
            .nodes
            .iter()
            .filter_map(|node| node.parent.clone())
            .collect();
        let mut seen_parents = HashSet::new();
        while let Some(parent_id) = pending.pop() {
            if !seen_parents.insert(parent_id.clone()) || graph.node(&parent_id).is_some() {
                continue;
            }
            let Some(page) = repository.inventory_nodes_matching(
                None,
                std::slice::from_ref(&parent_id),
                None,
                4,
            )?
            else {
                return Ok(None);
            };
            for node in page.nodes {
                if exact_match(&node, &parent_id) || node.id == parent_id {
                    if let Some(next) = node.parent.clone() {
                        pending.push(next);
                    }
                    insert_read_node(&mut graph, &repository, node)?;
                }
            }
        }
        let node_ids: Vec<String> = graph.nodes.iter().map(|node| node.id.clone()).collect();
        let Some(edges) = repository.inventory_structural_edges_for_nodes(&node_ids)? else {
            return Ok(None);
        };
        graph.edges = edges.into();
        // Relation endpoints may sit on other paths; hydrate only the opposite
        // address subjects needed for the truncated relation_neighbors page.
        let primary = candidates
            .iter()
            .find(|node| exact_match(node, subject))
            .cloned()
            .or_else(|| candidates.first().cloned());
        if let Some(node) = primary.as_ref() {
            let limit = bounded_limit(*limit);
            let mut endpoint_keys = HashSet::new();
            for address in node_addresses(&graph, node) {
                for edge in repository.relation_edges_for_subject(&address)? {
                    let subject_key = address_identity_key(&edge.subject);
                    let node_keys: HashSet<_> = node_addresses(&graph, node)
                        .iter()
                        .map(address_identity_key)
                        .collect();
                    let opposite = if node_keys.contains(&subject_key) {
                        &edge.object
                    } else {
                        &edge.subject
                    };
                    if endpoint_keys.len() >= limit {
                        break;
                    }
                    if endpoint_keys.insert(address_identity_key(opposite)) {
                        for endpoint in retained_address_nodes(
                            &repository,
                            &graph.snapshot.repository_id,
                            opposite,
                        )? {
                            insert_read_node(&mut graph, &repository, endpoint)?;
                        }
                    }
                }
            }
        }
        graph.rebuild_indexes();
        let mut projected = self.clone();
        projected.indexes = std::sync::Arc::new(QueryIndexes::new(
            &graph,
            self.options.substring_postings_budget,
        ));
        projected.graph = std::sync::Arc::new(graph);
        projected.deferred_navigation = None;
        let mut value = projected.get_subject_context(subject, *limit)?;
        // Path-local projections do not hydrate attachment/finding inventory.
        // Empty findings here mean unselected, not absence.
        if let Some(gaps) = value.get_mut("gaps").and_then(|gaps| gaps.as_array_mut()) {
            if !gaps
                .iter()
                .any(|gap| gap.as_str() == Some("attachment_inventory_not_materialized"))
            {
                gaps.push(serde_json::json!("attachment_inventory_not_materialized"));
            }
        }
        let result = projected.finish_result(value)?;
        Ok(Some((projected, result)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{KnowledgeRepository, KnowledgeStore, SqliteKnowledgeRepository};

    #[test]
    fn retained_subject_context_matches_full_reads_without_navigation() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("a.py"),
            "def leaf():\n    return 1\ndef caller():\n    leaf()\n",
        )
        .unwrap();
        std::fs::write(
            source.path().join("b.py"),
            "def unrelated():\n    return 2\n",
        )
        .unwrap();
        let graph = crate::build_topology_from_repo_with_options(source.path(), Default::default())
            .unwrap();
        let caller = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Callable && node.label == "caller")
            .unwrap()
            .id
            .clone();
        let unrelated = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Callable && node.label == "unrelated")
            .unwrap()
            .id
            .clone();
        let path = data.path().join("knowledge.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&path).unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        drop(repository);
        let eager = QueryEngine::new(graph, &path);
        let retained = QueryEngine::from_retained(&path, Default::default()).unwrap();
        let expected = eager
            .inspect(InspectQuery::Subject {
                id: caller.clone(),
                limit: Some(20),
            })
            .unwrap()
            .into_value()
            .unwrap();
        let actual = retained
            .inspect(InspectQuery::Subject {
                id: caller.clone(),
                limit: Some(20),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert!(
            actual["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap.as_str() == Some("attachment_inventory_not_materialized")),
            "retained subject must not imply attachment absence"
        );
        for key in [
            "status",
            "subject",
            "relations",
            "relation_claims",
            "direct_findings",
            "inherited_file_findings",
        ] {
            assert_eq!(actual[key], expected[key], "{key}");
        }
        assert_eq!(
            actual["containment"]["descendant_counts"],
            expected["containment"]["descendant_counts"]
        );
        assert_eq!(
            actual["containment"]["file"],
            expected["containment"]["file"]
        );
        let sort_ids = |value: &serde_json::Value| -> Vec<String> {
            let mut ids: Vec<String> = value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    item.get("node_id")
                        .and_then(|id| id.as_str())
                        .map(str::to_owned)
                })
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(
            sort_ids(&actual["containment"]["contained_callables"]),
            sort_ids(&expected["containment"]["contained_callables"])
        );
        assert_eq!(
            sort_ids(&actual["neighbors"]),
            sort_ids(&expected["neighbors"])
        );
        assert_eq!(
            actual["structural_edges"]
                .as_array()
                .map(|edges| edges.len()),
            expected["structural_edges"]
                .as_array()
                .map(|edges| edges.len())
        );
        assert!(
            retained
                .deferred_navigation
                .as_ref()
                .unwrap()
                .get()
                .is_none(),
            "subject context must not materialize repository navigation"
        );
        let unknown = retained
            .inspect(InspectQuery::Subject {
                id: "missing-subject".into(),
                limit: Some(4),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(unknown["status"], "unknown");
        assert!(retained
            .deferred_navigation
            .as_ref()
            .unwrap()
            .get()
            .is_none());
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute(
            "UPDATE inventory_node_versions SET label='corrupted' WHERE node_id=?1",
            [unrelated],
        )
        .unwrap();
        drop(db);
        let reopened = QueryEngine::from_retained(&path, Default::default()).unwrap();
        let reopened_value = reopened
            .inspect(InspectQuery::Subject {
                id: caller,
                limit: Some(20),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(reopened_value["status"], expected["status"]);
        assert_eq!(reopened_value["subject"], expected["subject"]);
        assert_eq!(
            reopened_value["containment"]["descendant_counts"],
            expected["containment"]["descendant_counts"]
        );
        assert!(
            reopened.graph().is_err(),
            "full projection must still detect unrelated corruption"
        );
    }
}
