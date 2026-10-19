//! Verified file-local projection membership. Rows reference the ordinary node
//! versions and edge payloads. Structural call evidence is revision-independent
//! and copied by membership when CallSummary inputs still match.
use super::inventory::{hydrate_node, NODE_COLUMNS};
use super::*;
use crate::{StructuralEdge, TopologyGraph};

pub(super) struct VerifiedProjection {
    pub graph: TopologyGraph,
}

fn commitment(
    connection: &Connection,
    unit: i64,
    nodes: &[[u8; 32]],
    edges: &[(String, [u8; 32])],
) -> Result<[u8; 32], KnowledgeRepositoryError> {
    let (context, path, source, facts): (Vec<u8>, String, Vec<u8>, Vec<u8>) = connection
        .prepare_cached("SELECT context,path,source_digest,content_digest FROM file_fact_units WHERE unit_id=?1")?
        .query_row([unit], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    let mut hash = blake3::Hasher::new();
    serde_json::to_writer(
        &mut hash,
        &(
            "entmoot.structural_file_projection.v1",
            context,
            path,
            source,
            facts,
            nodes,
            edges,
        ),
    )?;
    Ok(*hash.finalize().as_bytes())
}

/// Called only after the ordinary publisher validated and retained this syntax.
/// The surrounding transaction exposes both projection and view, or neither.
pub(super) fn seal(
    connection: &Connection,
    unit: i64,
    graph: &TopologyGraph,
) -> Result<(), KnowledgeRepositoryError> {
    let mut nodes = Vec::with_capacity(graph.nodes.len());
    let mut edges = Vec::with_capacity(graph.edges.len());
    let mut node_refs = Vec::with_capacity(graph.nodes.len());
    let mut edge_refs = Vec::with_capacity(graph.edges.len());
    for node in &graph.nodes {
        let digest = *blake3::hash(&serde_json::to_vec(node)?).as_bytes();
        let key: i64 = connection
            .prepare_cached("SELECT node_key FROM inventory_node_versions WHERE node_digest=?1")?
            .query_row([digest], |row| row.get(0))?;
        nodes.push(digest);
        node_refs.push(key);
    }
    for edge in &graph.edges {
        let row = StoredRow::new("structural_edges", edge)?;
        let payload = intern_payload(connection, &row.payload)?;
        let digest = *blake3::hash(&serde_json::to_vec(&serde_json::to_value(edge)?)?).as_bytes();
        edges.push((row.record_id.clone(), digest));
        edge_refs.push((row.record_id, payload));
    }
    let digest = commitment(connection, unit, &nodes, &edges)?;
    connection
        .prepare_cached("INSERT INTO structural_file_projections VALUES(?1,?2)")?
        .execute(params![unit, digest])?;
    let mut statement =
        connection.prepare_cached("INSERT INTO structural_file_nodes VALUES(?1,?2,?3)")?;
    for (ordinal, key) in node_refs.iter().enumerate() {
        statement.execute(params![
            unit,
            i64::try_from(ordinal).map_err(|e| invalid_record("file_projection", e))?,
            key
        ])?;
    }
    let mut statement =
        connection.prepare_cached("INSERT INTO structural_file_edges VALUES(?1,?2,?3,?4)")?;
    for (ordinal, (id, payload)) in edge_refs.iter().enumerate() {
        statement.execute(params![
            unit,
            i64::try_from(ordinal).map_err(|e| invalid_record("file_projection", e))?,
            id,
            payload
        ])?;
    }
    Ok(())
}

/// Verify membership and each referenced content commitment before reuse.
/// Missing projections are ordinary cache misses; damaged sealed projections
/// fail visibly instead of silently dropping nodes, edges or unknown outcomes.
pub(super) fn load(
    connection: &Connection,
    unit: i64,
    header: &TopologyGraph,
) -> Result<Option<VerifiedProjection>, KnowledgeRepositoryError> {
    let expected: Option<Vec<u8>> = connection
        .prepare_cached("SELECT commitment FROM structural_file_projections WHERE unit_id=?1")?
        .query_row([unit], |row| row.get(0))
        .optional()?;
    let Some(expected) = expected else {
        return Ok(None);
    };
    let mut projection = VerifiedProjection {
        graph: header.clone(),
    };
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let columns = NODE_COLUMNS.replace("n.ordinal", "m.ordinal");
    let mut statement = connection.prepare_cached(&format!("SELECT {columns} FROM structural_file_nodes m JOIN inventory_node_versions v USING(node_key) LEFT JOIN shared_definitions d USING(definition_id) LEFT JOIN shared_paths p USING(path_id) WHERE m.unit_id=?1 ORDER BY m.ordinal"))?;
    let mut rows = statement.query([unit])?;
    while let Some(row) = rows.next()? {
        let node = hydrate_node(connection, row)?;
        let digest = *blake3::hash(&serde_json::to_vec(&node)?).as_bytes();
        if row.get::<_, Vec<u8>>(4)? != digest {
            return Err(invalid_record(
                "file_projection",
                "node version commitment mismatch",
            ));
        }
        nodes.push(digest);
        projection.graph.nodes.push(node);
    }
    let mut statement = connection.prepare_cached("SELECT m.record_id,m.payload FROM structural_file_edges m WHERE m.unit_id=?1 ORDER BY m.ordinal")?;
    let mut rows = statement.query([unit])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let payload = row.get(1)?;
        let bytes = hydrate_payload(connection, payload)?;
        let edge: StructuralEdge = serde_json::from_slice(&bytes)?;
        if StoredRow::new("structural_edges", &edge)?.record_id != id {
            return Err(invalid_record(
                "file_projection",
                "edge identity commitment mismatch",
            ));
        }
        edges.push((id, *blake3::hash(&bytes).as_bytes()));
        projection.graph.edges.push(edge);
    }
    if commitment(connection, unit, &nodes, &edges)?.as_slice() != expected {
        return Err(invalid_record(
            "file_projection",
            "file projection membership commitment mismatch",
        ));
    }
    projection.graph.rebuild_indexes();
    Ok(Some(projection))
}

const CALL_COLLECTIONS: &[&str] = &[
    "structural_edges",
    "relation_edges",
    "relation_claims",
    "unresolved_relations",
    "propositions",
];

/// Seal revision-independent call rows for a file unit after resolution.
pub(super) fn seal_calls(
    connection: &Connection,
    unit: i64,
    store: &KnowledgeStore,
) -> Result<(), KnowledgeRepositoryError> {
    connection
        .prepare_cached("DELETE FROM structural_file_calls WHERE unit_id=?1")?
        .execute([unit])?;
    let mut statement =
        connection.prepare_cached("INSERT INTO structural_file_calls VALUES(?1,?2,?3,?4,?5)")?;
    let mut ordinal = 0i64;
    for_each_store_row(store, |row| {
        if !CALL_COLLECTIONS.contains(&row.collection) {
            return Ok(());
        }
        let payload = intern_row_payload(connection, &row)?;
        statement.execute(params![
            unit,
            ordinal,
            row.collection,
            row.record_id,
            payload
        ])?;
        ordinal = ordinal
            .checked_add(1)
            .ok_or_else(|| invalid_record("file_calls", "ordinal overflow"))?;
        Ok(())
    })?;
    Ok(())
}

/// Copy sealed call membership into the selected view. Missing seals are misses.
pub(super) fn reuse_calls(
    connection: &Connection,
    unit: i64,
) -> Result<bool, KnowledgeRepositoryError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM structural_file_calls WHERE unit_id=?1",
        [unit],
        |row| row.get(0),
    )?;
    if count == 0 {
        return Ok(false);
    }
    connection
        .prepare_cached(
            "INSERT INTO scoped_records(view_id,collection,record_id,payload) \
         SELECT (SELECT id FROM temp.selected_knowledge_view),collection,record_id,payload \
         FROM structural_file_calls WHERE unit_id=?1 \
         ON CONFLICT(view_id,collection,record_id) DO UPDATE SET payload=CASE \
         WHEN scoped_records.payload=excluded.payload THEN excluded.payload ELSE NULL END",
        )?
        .execute([unit])?;
    Ok(true)
}
