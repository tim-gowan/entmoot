//! Indexed evidence reads for bounded query projections.
use super::*;
use crate::read::{RelationReadContext, RelationReadPage, UnresolvedReadPage};
use entmoot_contracts::{query::CallDirection, ContentDigest, RelationKind};

const PAGE_BYTES: usize = 8 * 1024 * 1024;

fn keys(addresses: &[Address], limit: usize) -> Result<Vec<String>, KnowledgeRepositoryError> {
    if addresses.len() > 8 || !(1..=1024).contains(&limit) {
        return Err(KnowledgeRepositoryError::InvalidPageSize);
    }
    addresses
        .iter()
        .map(|address| {
            validate_query_address(address)?;
            Ok(address_identity_key(address))
        })
        .collect()
}

fn count(value: i64) -> Result<usize, KnowledgeRepositoryError> {
    usize::try_from(value)
        .map_err(|error| KnowledgeRepositoryError::BackendCount(error.to_string()))
}

impl SqliteKnowledgeRepository {
    pub(super) fn read_call_edges(
        &self,
        addresses: &[Address],
        direction: CallDirection,
        after: Option<&str>,
        limit: usize,
    ) -> Result<RelationReadPage, KnowledgeRepositoryError> {
        let keys = keys(addresses, limit)?;
        if let Some(after) = after {
            RelationEdgeId::parse(after)
                .map_err(|e| KnowledgeRepositoryError::InvalidQueryRelationEdgeId(e.to_string()))?;
        }
        let encoded_keys = serde_json::to_string(&keys)?;
        let (view, column) = match direction {
            CallDirection::Callees => ("knowledge_records_by_subject", "subject_id"),
            CallDirection::Callers => ("knowledge_records_by_object", "object_id"),
        };
        let selected = format!("FROM {view} WHERE collection='relation_edges' AND {column} IN (SELECT value FROM json_each(?1)) AND predicate='calls' AND visible=1");
        let (total, offset): (i64, i64) = self
            .connection
            .prepare_cached(&format!(
                "SELECT COUNT(*),COALESCE(SUM(record_id<=?2),0) {selected}"
            ))?
            .query_row(params![encoded_keys, after], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        let mut statement = self.connection.prepare_cached(&format!("SELECT record_id,payload {selected} AND (?2 IS NULL OR record_id>?2) ORDER BY record_id LIMIT ?3"))?;
        let mut rows = statement.query(params![
            encoded_keys,
            after,
            i64::try_from(limit).map_err(|_| KnowledgeRepositoryError::InvalidPageSize)?
        ])?;
        let mut edges = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let edge: RelationEdge = hydrate_record(&self.connection, row.get(1)?)?;
            edge.validate()
                .map_err(|e| invalid_record("relation_edges", e))?;
            let address = if direction == CallDirection::Callees {
                &edge.subject
            } else {
                &edge.object
            };
            if edge.id.as_str() != row.get::<_, String>(0)?
                || edge.predicate != RelationKind::Calls
                || !keys.contains(&address_identity_key(address))
            {
                return Err(invalid_record(
                    "relation_edges",
                    "indexed relation disagrees with its payload",
                ));
            }
            let size = serde_json::to_vec(&edge)?.len();
            if size > PAGE_BYTES {
                return Err(KnowledgeRepositoryError::BufferLimit(
                    "relation exceeds 8 MiB",
                ));
            }
            if bytes + size > PAGE_BYTES {
                break;
            }
            bytes += size;
            edges.push(edge);
        }
        Ok(RelationReadPage {
            edges,
            total: count(total)?,
            offset: count(offset)?,
        })
    }

    pub(super) fn read_unresolved_page(
        &self,
        addresses: &[Address],
        after: Option<&str>,
        limit: usize,
    ) -> Result<UnresolvedReadPage, KnowledgeRepositoryError> {
        let keys = keys(addresses, limit)?;
        let mut statement = self.connection.prepare_cached("SELECT record_id,payload FROM knowledge_records_by_subject WHERE collection='unresolved_relations' AND subject_id IN (SELECT value FROM json_each(?1)) AND visible=1 ORDER BY record_id")?;
        let mut rows = statement.query([serde_json::to_string(&keys)?])?;
        let mut hash = blake3::Hasher::new();
        hash.update(b"[");
        let mut page = UnresolvedReadPage {
            items: Vec::new(),
            offset: 0,
            total: 0,
            collection_digest: ContentDigest::from_trusted(entmoot_contracts::blake3_hex(b"[]")),
            after_present: after.is_none(),
        };
        let mut bytes = 0;
        let mut full = false;
        while let Some(row) = rows.next()? {
            let attempt: UnresolvedRelation = hydrate_record(&self.connection, row.get(1)?)?;
            attempt
                .validate()
                .map_err(|e| invalid_record("unresolved_relations", e))?;
            if attempt.id.as_str() != row.get::<_, String>(0)?
                || !keys.contains(&address_identity_key(&attempt.source))
            {
                return Err(invalid_record(
                    "unresolved_relations",
                    "indexed attempt disagrees with its payload",
                ));
            }
            if page.total > 0 {
                hash.update(b",");
            }
            serde_json::to_writer(&mut hash, &attempt)?;
            page.total += 1;
            if after == Some(attempt.id.as_str()) {
                page.after_present = true;
            }
            if after.is_some_and(|after| attempt.id.as_str() <= after) {
                page.offset += 1;
                continue;
            }
            if page.items.len() == limit || full {
                continue;
            }
            let size = serde_json::to_vec(&attempt)?.len();
            if size > PAGE_BYTES {
                return Err(KnowledgeRepositoryError::BufferLimit(
                    "unresolved attempt exceeds 8 MiB",
                ));
            }
            if bytes + size > PAGE_BYTES {
                full = true;
                continue;
            }
            bytes += size;
            page.items.push(attempt);
        }
        hash.update(b"]");
        page.collection_digest =
            ContentDigest::from_trusted(format!("blake3:{}", hash.finalize().to_hex()));
        Ok(page)
    }

    pub(super) fn read_relation_context(
        &self,
        edges: &[&RelationEdge],
        artifact_ids: &[String],
        coverage_limit: usize,
    ) -> Result<RelationReadContext, KnowledgeRepositoryError> {
        if edges.len() > 1024 || !(1..=1024).contains(&coverage_limit) {
            return Err(KnowledgeRepositoryError::InvalidPageSize);
        }
        let edge_claims = self.relation_claims_for_edges(edges)?;
        let originals: BTreeSet<_> = edge_claims
            .iter()
            .flatten()
            .map(|claim| claim.id.as_str())
            .collect();
        let mut related = BTreeMap::new();
        for target in edge_claims
            .iter()
            .flatten()
            .flat_map(|claim| &claim.contradicts)
        {
            if !originals.contains(target.as_str()) && !related.contains_key(target.as_str()) {
                if let Some(claim) = self.relation_claim_by_id(target.as_str())? {
                    related.insert(claim.id.to_string(), claim);
                }
            }
        }
        // A contradiction can point into the requested page from another edge.
        // The payload-keyed reverse index is shared across views; visibility is
        // always resolved against this pinned view before accepting evidence.
        let mut statement = self.connection.prepare_cached("SELECT DISTINCT r.payload FROM shared_claim_links l INDEXED BY claim_links_by_target JOIN shared_payloads p ON p.payload_id=l.payload JOIN scoped_records r ON r.view_id=(SELECT id FROM temp.selected_knowledge_view) AND r.collection='relation_claims' AND r.record_id=json_extract(CAST(p.body AS TEXT),'$.body.id') AND r.payload=p.payload_id AND r.visible=1 WHERE l.target IN (SELECT value FROM json_each(?1))")?;
        let rows = statement.query_map([serde_json::to_string(&originals)?], |row| {
            row.get::<_, i64>(0)
        })?;
        for row in rows {
            let claim: RelationClaim = hydrate_record(&self.connection, row?)?;
            claim
                .validate()
                .map_err(|e| invalid_record("relation_claims", e))?;
            related.insert(claim.id.to_string(), claim);
        }
        let mut artifacts: BTreeSet<String> = artifact_ids.iter().cloned().collect();
        for claim in edge_claims.iter().flatten() {
            artifacts.extend(
                claim
                    .evidence
                    .iter()
                    .map(|evidence| evidence.artifact_id.clone()),
            );
            if let Some(provenance) = &claim.provenance {
                artifacts.extend(provenance.artifacts.iter().cloned());
            }
        }
        let mut statement = self.connection.prepare_cached("SELECT r.payload FROM scoped_records r INDEXED BY knowledge_by_artifact JOIN shared_payloads p ON p.payload_id=r.payload WHERE r.view_id=(SELECT id FROM temp.selected_knowledge_view) AND r.collection='resolver_coverages' AND r.artifact_id IN (SELECT value FROM json_each(?1)) AND r.visible=1 ORDER BY json_extract(CAST(p.body AS TEXT),'$.body.coverage_id') LIMIT ?2")?;
        let rows = statement.query_map(
            params![
                serde_json::to_string(&artifacts)?,
                i64::try_from(coverage_limit)
                    .map_err(|_| KnowledgeRepositoryError::InvalidPageSize)?
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let resolver_coverages = rows
            .map(|row| hydrate_record(&self.connection, row?))
            .collect::<Result<Vec<_>, _>>()?;
        let artifacts = artifacts
            .into_iter()
            .map(|id| self.artifact_by_id(&id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(RelationReadContext {
            edge_claims,
            related_claims: related.into_values().collect(),
            artifacts,
            resolver_coverages,
        })
    }
}
