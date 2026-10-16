//! Independent, read-only continuation for original unresolved attempt details.
use entmoot_contracts::query::UnresolvedPageCursor;
use entmoot_contracts::{blake3_hex, ContentDigest};
use serde_json::{json, Value};

use super::{node_addresses, QueryEngine, QueryError, ResultBounds, QUERY_RESULT_SCHEMA};
use crate::KnowledgeRepository;

impl QueryEngine {
    pub(super) fn unresolved_page(
        &self,
        subject: &str,
        after: Option<&UnresolvedPageCursor>,
        requested_limit: Option<usize>,
    ) -> Result<Value, QueryError> {
        let limit = requested_limit.unwrap_or(50);
        if !(1..=1_000).contains(&limit) {
            return Err(QueryError::InvalidInput(
                "unresolved-attempt limit must be in 1..=1000".into(),
            ));
        }
        let repository = self.repository()?;
        let visibility_cursor = repository.current_visibility_cursor()?;
        // Facets return attempt identities. Permit that exact durable record as
        // well as paging all attempts for a source subject.
        let exact_attempt =
            if entmoot_contracts::UnresolvedRelationId::parse(subject.trim()).is_ok() {
                repository.unresolved_by_id(subject.trim())?
            } else {
                None
            };
        if let Some(attempt) = exact_attempt {
            if after.is_some() {
                return Err(QueryError::InvalidInput(
                    "an exact unresolved attempt does not accept a page cursor".into(),
                ));
            }
            if repository.current_visibility_cursor()? != visibility_cursor {
                return Err(QueryError::InvalidInput(
                    "unresolved-attempt evidence changed while reading; retry".into(),
                ));
            }
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA, "operation": "inspect",
                "view": "unresolved", "snapshot": self.graph.snapshot,
                "subject": subject, "visibility_cursor": visibility_cursor,
                "status": "known", "items": [attempt], "next_cursor": null,
                "bounds": ResultBounds::complete(1),
                "gaps": ["repository_call_graph_completeness_not_established", "dependency_call_graph_completeness_not_established", "native_library_call_graph_completeness_not_established"],
            }));
        }
        let candidates = self.exact_subject_nodes(subject.trim());
        if candidates.len() != 1 {
            if after.is_some() {
                return Err(QueryError::InvalidInput(
                    "unresolved-attempt cursor requires the original unique subject".into(),
                ));
            }
            return Ok(json!({
                "schema_version": QUERY_RESULT_SCHEMA, "operation": "inspect",
                "view": "unresolved", "snapshot": self.graph.snapshot,
                "subject": subject, "visibility_cursor": visibility_cursor,
                "status": if candidates.is_empty() { "unknown" } else { "conflict" },
                "items": [], "bounds": ResultBounds::complete(0),
                "gaps": if candidates.is_empty() { vec!["subject_not_resolved"] }
                        else { vec!["subject_is_ambiguous"] },
            }));
        }
        let node = candidates[0];
        let mut attempts = Vec::new();
        for address in node_addresses(&self.graph, node) {
            attempts.extend(repository.unresolved_for_source(&address)?);
        }
        attempts.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        attempts.dedup_by(|a, b| a.id == b.id);
        // Visibility changes invalidate even identical collections. Comparing the
        // complete bytes also catches replacement through legacy publication.
        if repository.current_visibility_cursor()? != visibility_cursor {
            return Err(QueryError::InvalidInput(
                "unresolved-attempt evidence changed while reading; restart paging".into(),
            ));
        }
        let collection_digest =
            ContentDigest::from_trusted(blake3_hex(&serde_json::to_vec(&attempts)?));
        let offset = if let Some(cursor) = after {
            if cursor.subject_id != node.id
                || cursor.snapshot != self.graph.snapshot
                || cursor.visibility_cursor != visibility_cursor
                || cursor.collection_digest != collection_digest
            {
                return Err(QueryError::InvalidInput(
                    "unresolved-attempt cursor does not match subject, snapshot or evidence; restart paging".into(),
                ));
            }
            attempts
                .binary_search_by(|attempt| {
                    attempt
                        .id
                        .as_str()
                        .cmp(cursor.unresolved_relation_id.as_str())
                })
                .map_err(|_| {
                    QueryError::InvalidInput(
                        "unresolved-attempt cursor key is absent; restart paging".into(),
                    )
                })?
                + 1
        } else {
            0
        };
        let total = attempts.len();
        let page = attempts.iter().skip(offset).take(limit).collect::<Vec<_>>();
        let truncated = offset.saturating_add(page.len()) < total;
        let next_cursor = if truncated {
            page.last().map(|attempt| UnresolvedPageCursor {
                unresolved_relation_id: attempt.id.clone(),
                subject_id: node.id.clone(),
                snapshot: self.graph.snapshot.clone(),
                visibility_cursor,
                collection_digest,
            })
        } else {
            None
        };
        Ok(json!({
            "schema_version": QUERY_RESULT_SCHEMA, "operation": "inspect",
            "view": "unresolved", "snapshot": self.graph.snapshot,
            "subject": subject, "visibility_cursor": visibility_cursor,
            "status": if truncated { "truncated" } else if page.is_empty() { "unknown" } else { "known" },
            "items": page, "next_cursor": next_cursor,
            "bounds": {"offset": offset, "limit": limit, "total_matches": total, "truncated": truncated},
            "gaps": ["repository_call_graph_completeness_not_established", "dependency_call_graph_completeness_not_established", "native_library_call_graph_completeness_not_established"],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::InspectQuery;
    use crate::{KnowledgeStore, NodeKind, SqliteKnowledgeRepository, TopologyGraph, TopologyNode};
    use entmoot_contracts::{
        topology_node_address, unresolved_relation_with_id, ProducerCapabilityKind, ProducerRef,
        RelationKind, Scope, TargetConstraint, UnresolvedReason,
    };

    #[test]
    fn pages_every_attempt_preserving_records_and_rejects_changed_context() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("attempts.sqlite3");
        let mut graph = TopologyGraph::new("paging");
        for name in ["caller", "empty"] {
            graph.upsert_node(TopologyNode::shell(name, NodeKind::Callable, name, None));
        }
        for i in 0..134 {
            graph.unresolved_relations.push(unresolved_relation_with_id(
                topology_node_address("caller"),
                RelationKind::Calls,
                TargetConstraint::Syntactic {
                    callee: format!("target_{i}"),
                },
                UnresolvedReason::DispatchNotEvaluated,
                ProducerRef {
                    id: "fixture".into(),
                    name: "fixture".into(),
                    version: Some("1".into()),
                    config_fingerprint: None,
                    environment_fingerprint: None,
                    capabilities: vec![ProducerCapabilityKind::Resolver],
                },
                Scope {
                    domain: graph.snapshot.repository_id.clone(),
                    snapshot_revision: Some(graph.snapshot.revision.clone()),
                    snapshot_digest: graph.snapshot.snapshot_digest.clone(),
                    occurrence_id: None,
                    context_id: None,
                },
                Some(format!("site-{i}")),
                vec![],
                None,
                Some(ProducerCapabilityKind::Resolver),
            ));
        }
        let mut repository = SqliteKnowledgeRepository::open(&store).unwrap();
        repository
            .replace_owner("fixture", &KnowledgeStore::extract_from_graph(&graph), &[])
            .unwrap();
        let before = std::fs::read(&store).unwrap();
        let engine = QueryEngine::new(graph.clone(), store.clone());
        let query = |id: &str, cursor: Option<UnresolvedPageCursor>, limit| {
            engine.inspect(InspectQuery::Unresolved {
                id: id.into(),
                after: cursor,
                limit: Some(limit),
            })
        };
        let first = query("caller", None, 50).unwrap().into_value().unwrap();
        let status = engine.status(Default::default()).unwrap();
        assert_eq!(status.unresolved_reasons[0].name, "dispatch_not_evaluated");
        let facet = engine
            .find(entmoot_contracts::query::FindQuery::UnresolvedBoundary {
                reason: Some(status.unresolved_reasons[0].name.clone()),
                path: None,
                limit: Some(10),
            })
            .unwrap()
            .into_value()
            .unwrap();
        assert_eq!(facet["items"].as_array().unwrap().len(), 10);
        let attempt_id = facet["items"][0]["id"].as_str().unwrap();
        let exact = query(attempt_id, None, 10).unwrap().into_value().unwrap();
        assert_eq!(exact["items"][0]["id"], attempt_id);
        assert_eq!(exact["items"].as_array().unwrap().len(), 1);
        assert_eq!(exact["items"][0]["reason"], "dispatch_not_evaluated");
        assert_eq!(first["status"], "truncated");
        assert_eq!(first["bounds"]["total_matches"], 134);
        assert_eq!(first["receipt"]["unresolved"]["count"], 50);
        let cursor: UnresolvedPageCursor =
            serde_json::from_value(first["next_cursor"].clone()).unwrap();
        assert!(query(attempt_id, Some(cursor.clone()), 10).is_err());
        let mut all = first["items"].as_array().unwrap().clone();
        let mut next = Some(cursor.clone());
        while let Some(after) = next {
            let page = query("caller", Some(after), 50)
                .unwrap()
                .into_value()
                .unwrap();
            assert_eq!(
                page["gaps"][0],
                "repository_call_graph_completeness_not_established"
            );
            all.extend(page["items"].as_array().unwrap().iter().cloned());
            next = page
                .get("next_cursor")
                .and_then(|c| serde_json::from_value(c.clone()).ok());
        }
        let mut expected = graph.unresolved_relations.clone();
        expected.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        assert_eq!(serde_json::to_value(expected).unwrap(), json!(all));
        for id in ["empty", "missing"] {
            let page = query(id, None, 10).unwrap().into_value().unwrap();
            assert_eq!(page["status"], "unknown");
            assert!(page["items"].as_array().unwrap().is_empty());
            assert!(query(id, Some(cursor.clone()), 50).is_err());
        }
        for limit in [0, 1001] {
            assert!(query("caller", None, limit).is_err());
        }
        for mutate in 0..6 {
            let mut bad = cursor.clone();
            match mutate {
                0 => bad.subject_id = "empty".into(),
                1 => bad.snapshot.repository_id = "different-repo".into(),
                2 => bad.snapshot.revision = "a".repeat(40),
                3 => bad.visibility_cursor += 1,
                4 => bad.collection_digest = ContentDigest::from_trusted(blake3_hex(b"different")),
                _ => {
                    bad.unresolved_relation_id =
                        entmoot_contracts::UnresolvedRelationId::parse(blake3_hex(b"absent"))
                            .unwrap()
                }
            }
            assert!(query("caller", Some(bad), 50).is_err());
        }
        assert_eq!(
            before,
            std::fs::read(&store).unwrap(),
            "reads must not publish"
        );
        // A legacy replace can change evidence bytes without changing identity.
        graph.unresolved_relations[0].remaining_capability = None;
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        assert!(query("caller", Some(cursor), 50).is_err());
    }
}
