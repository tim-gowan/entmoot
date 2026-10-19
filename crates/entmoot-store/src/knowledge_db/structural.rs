//! File-sized structural publication directly into an unpublished SQLite view.
//! Only the callable lookup index crosses the file barrier; graph exports are
//! streamed from retained rows in the same canonical order as an eager graph.
use super::inventory::{hydrate_node, NODE_COLUMNS, NODE_FROM};
use super::*;
use crate::{NodeKind, SnapshotFileDigest, TopologyGraph, TopologyNode};
use serde::de::{MapAccess, Visitor};
use serde_json::value::RawValue;
use std::io::Write;

pub(crate) struct StructuralWriter<'a, 'db> {
    transaction: &'a Transaction<'db>,
    metadata: &'a TopologyGraph,
    files: BTreeMap<&'a str, &'a SnapshotFileDigest>,
    ordinal: usize,
}

impl StructuralWriter<'_, '_> {
    /// File projections are validated with their exact retained endpoint/parent
    /// context. Global identity collisions abort the view, never overwrite.
    pub(crate) fn append(&mut self, graph: &TopologyGraph) -> Result<(), KnowledgeRepositoryError> {
        let mut checked = graph.clone();
        let mut needed = BTreeSet::new();
        for edge in &checked.edges {
            needed.insert(edge.from.clone());
            needed.insert(edge.to.clone());
        }
        needed.extend(checked.nodes.iter().filter_map(|node| node.parent.clone()));
        while let Some(id) = needed.pop_first() {
            if checked.node(&id).is_some() {
                continue;
            }
            let node = self.node(&id)?.ok_or_else(|| {
                invalid_record("inventory", format!("missing structural endpoint {id}"))
            })?;
            if let Some(parent) = &node.parent {
                needed.insert(parent.clone());
            }
            checked.upsert_node(node);
        }
        let paths: BTreeSet<_> = checked
            .nodes
            .iter()
            .filter_map(|node| node.path.as_deref())
            .collect();
        checked.snapshot_files = paths
            .into_iter()
            .filter_map(|path| self.files.get(path).copied().cloned())
            .collect::<Vec<_>>()
            .into();
        checked.rebuild_indexes();
        checked
            .validate()
            .map_err(|e| invalid_record("inventory", e))?;
        for node in &graph.nodes {
            if let Some(prior) = self.node(&node.id)? {
                if serde_json::to_vec(&prior)? != serde_json::to_vec(node)? {
                    return Err(invalid_record(
                        "inventory",
                        "conflicting structural node identity",
                    ));
                }
                continue;
            }
            super::inventory::store_node(
                self.transaction,
                &self.metadata.policy_ids,
                self.ordinal,
                node,
            )?;
            self.ordinal += 1;
        }
        let context = StoreValidationContext::from_graph(graph);
        for batch in KnowledgeStoreBatches::from_graph(graph, PublicationBuffer::default())? {
            let batch = batch?;
            validate_store_records_in_context(&batch.store, &context)?;
            insert_immutable_chunk(self.transaction, &batch.store)?;
        }
        Ok(())
    }

    pub(crate) fn append_syntax(
        &mut self,
        unit: i64,
        graph: &TopologyGraph,
    ) -> Result<(), KnowledgeRepositoryError> {
        self.append(graph)?;
        super::structural_files::seal(self.transaction, unit, graph)
    }

    pub(crate) fn reuse_syntax(
        &mut self,
        unit: i64,
        header: &TopologyGraph,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        let Some(projection) = super::structural_files::load(self.transaction, unit, header)?
        else {
            return Ok(None);
        };
        // Copy integer memberships and existing edge references in SQLite, not
        // through a Rust lookup/insert round trip for each unchanged node.
        self.transaction.prepare_cached("INSERT OR IGNORE INTO inventory_nodes SELECT (SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)),?2+ROW_NUMBER() OVER(ORDER BY ordinal)-1,node_key FROM structural_file_nodes WHERE unit_id=?1")?
            .execute(params![unit,i64::try_from(self.ordinal).map_err(|e|invalid_record("inventory",e))?])?;
        self.ordinal = self
            .ordinal
            .checked_add(projection.graph.nodes.len())
            .ok_or_else(|| invalid_record("inventory", "node ordinal overflow"))?;
        self.transaction.prepare_cached("INSERT INTO scoped_records(view_id,collection,record_id,payload) SELECT (SELECT id FROM temp.selected_knowledge_view),'structural_edges',record_id,payload FROM structural_file_edges WHERE unit_id=?1 ON CONFLICT(view_id,collection,record_id) DO UPDATE SET payload=CASE WHEN scoped_records.payload=excluded.payload THEN excluded.payload ELSE NULL END")?
            .execute([unit])?;
        // These envelopes include the current snapshot. Reusing the unchanged
        // syntax does not authorize copying an older revision's record IDs.
        let mut syntax = projection.graph.clone();
        syntax.edges.clear();
        for batch in KnowledgeStoreBatches::from_graph(&syntax, PublicationBuffer::default())? {
            insert_immutable_chunk(self.transaction, &batch?.store)?;
        }
        Ok(Some(projection.graph))
    }

    fn node(&self, id: &str) -> Result<Option<TopologyNode>, KnowledgeRepositoryError> {
        let sql = node_lookup_sql();
        let mut statement = self.transaction.prepare_cached(&sql)?;
        let mut rows = statement.query([id])?;
        rows.next()?
            .map(|row| hydrate_node(self.transaction, row))
            .transpose()
    }
}

fn node_lookup_sql() -> String {
    // Select versions by identity before joining revision membership. A predicate
    // on v.node_id alone lets SQLite scan every member for every appended node.
    let from = NODE_FROM.replace(
        "inventory_nodes n",
        "inventory_nodes n INDEXED BY inventory_members_by_version",
    );
    format!("SELECT {NODE_COLUMNS} FROM {from} WHERE n.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) AND n.node_key IN (SELECT node_key FROM inventory_node_versions WHERE node_id=?1)")
}

// An indexed temporary permutation avoids SQLite rescanning a materialized CTE
// once per node. It stores only two integers and is dropped before publication.
const PREPARE_NODE_ORDER: &str = "CREATE TEMP TABLE structural_positions(ordinal INTEGER PRIMARY KEY, position INTEGER NOT NULL) WITHOUT ROWID; INSERT INTO structural_positions SELECT ordinal,ROW_NUMBER() OVER(ORDER BY node_id) FROM inventory_nodes JOIN inventory_node_versions USING(node_key) WHERE view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view));";
const APPLY_NODE_ORDER: &str = "UPDATE inventory_nodes SET ordinal=-(SELECT position FROM structural_positions WHERE structural_positions.ordinal=inventory_nodes.ordinal) WHERE view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view))";
const FINISH_NODE_ORDER: &str = "UPDATE inventory_nodes SET ordinal=-ordinal-1 WHERE view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)); DROP TABLE structural_positions;";

fn insert_immutable_chunk(
    transaction: &Transaction<'_>,
    store: &KnowledgeStore,
) -> Result<(), KnowledgeRepositoryError> {
    let mut statement = transaction.prepare_cached("INSERT INTO scoped_records(view_id,collection,record_id,artifact_id,subject_id,object_id,proposition_id,predicate,location_id,payload) VALUES((SELECT id FROM temp.selected_knowledge_view),?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(view_id,collection,record_id) DO UPDATE SET payload=CASE WHEN scoped_records.payload=excluded.payload THEN excluded.payload ELSE NULL END")?;
    for_each_store_row(store, |row| {
        statement.execute(params![
            row.collection,
            row.record_id,
            row.artifact_id,
            row.subject_id,
            row.object_id,
            row.proposition_id,
            row.predicate,
            row.location_id,
            intern_row_payload(transaction, &row)?
        ])?;
        Ok(())
    })?;
    Ok(())
}

impl SqliteKnowledgeRepository {
    /// Build an immutable structural view without retaining repository nodes or
    /// relation bags in process memory. The source fence and pointer publication
    /// remain the façade's responsibility. Failure rolls back the complete view.
    pub(crate) fn seal_structural_parts(
        &mut self,
        metadata: &TopologyGraph,
        cache: &mut super::syntax_cache::SyntaxCache,
        produce: impl FnOnce(
            &mut StructuralWriter<'_, '_>,
            &mut super::syntax_cache::SyntaxCache,
        ) -> Result<(), KnowledgeRepositoryError>,
    ) -> Result<(String, PublicationOutcome), KnowledgeRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selected: String =
            transaction.query_row("SELECT id FROM temp.selected_knowledge_view", [], |row| {
                row.get(0)
            })?;
        // The deterministic derivation key is known before inserting any rows.
        // A separate canonical graph commitment verifies exports. This avoids
        // rewriting every record's primary key after graph hashing finishes.
        let view = entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
            "entmoot.structural_view.v1",
            metadata,
            cache.file_commitment()?,
        ))?);
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM inventory_headers WHERE view_id=?1)",
            [&view],
            |row| row.get(0),
        )?;
        if exists {
            transaction.rollback()?;
            let historical = self.load_syntax_cache_for_view(&cache.context, &view)?;
            cache.reuse_historical_selection(&historical)?;
            return Ok((view.clone(), PublicationOutcome::Unchanged { digest: view }));
        }
        transaction.execute("UPDATE temp.selected_knowledge_view SET id=?1", [&view])?;
        let mut header = TopologyGraph::new(metadata.root.clone());
        header.snapshot = metadata.snapshot.clone();
        header.policy_ids = metadata.policy_ids.clone();
        let payload = intern_payload(&transaction, &serde_json::to_vec(&header)?)?;
        transaction.execute(
            "INSERT INTO inventory_headers VALUES(NULL,?1,?2,'',1,'',?3)",
            params![
                view,
                payload,
                super::inventory::files_digest(&metadata.snapshot, &metadata.snapshot_files)?
            ],
        )?;
        let mut writer = StructuralWriter {
            transaction: &transaction,
            metadata,
            files: metadata
                .snapshot_files
                .iter()
                .map(|file| (file.path.as_str(), file))
                .collect(),
            ordinal: 0,
        };
        // Repository identity is needed for each file's parent check.
        let mut root = header;
        for node in metadata
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Repo)
        {
            root.upsert_node(node.clone());
        }
        writer.append(&root)?;
        produce(&mut writer, cache)?;
        // Finalization contributes skipped/unsupported files and configuration.
        // Parsed file rows already own their richer file/hash observation.
        let mut final_part = metadata.clone();
        final_part.nodes.retain(|node| node.kind != NodeKind::Repo);
        let mut omitted = BTreeSet::new();
        for node in final_part
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::File && node.file_hash.is_none())
        {
            if writer.node(&node.id)?.is_some() {
                omitted.insert(node.id.clone());
            }
        }
        final_part.nodes.retain(|node| !omitted.contains(&node.id));
        final_part.rebuild_indexes();
        writer.append(&final_part)?;
        drop(writer);
        let conflicting: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM inventory_nodes JOIN inventory_node_versions USING(node_key) WHERE view_key=(SELECT view_key FROM inventory_headers WHERE view_id=?1) GROUP BY node_id HAVING COUNT(*)>1)", [&view], |row|row.get(0))?;
        if conflicting {
            return Err(invalid_record(
                "inventory",
                "conflicting structural node identity",
            ));
        }
        let duplicate: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM inventory_nodes JOIN inventory_node_versions USING(node_key) WHERE view_key=(SELECT view_key FROM inventory_headers WHERE view_id=?1) AND kind='declaration' AND address_id IS NOT NULL GROUP BY address_id HAVING COUNT(*)>1)", [&view], |row| row.get(0))?;
        if duplicate {
            return Err(invalid_record(
                "inventory",
                "ambiguous declaration observation address",
            ));
        }
        // Reorder integer membership positions without a second payload copy.
        let order = crate::measurement::PhaseTimer::start("publication_node_order");
        transaction.execute_batch(PREPARE_NODE_ORDER)?;
        transaction.execute(APPLY_NODE_ORDER, [])?;
        transaction.execute_batch(FINISH_NODE_ORDER)?;
        cache.phases.push(order.finish());
        let files = crate::measurement::PhaseTimer::start("publication_source_membership");
        for (ordinal, file) in metadata.snapshot_files.iter().enumerate() {
            let path = super::inventory::intern_path(&transaction, &file.path)?;
            let (source, _) =
                super::inventory::intern_source_digest(&transaction, &file.content_digest)?;
            transaction.execute("INSERT INTO inventory_files VALUES((SELECT view_key FROM inventory_headers WHERE view_id=?1),?2,?3,?4)",params![view,i64::try_from(ordinal).map_err(|e|invalid_record("inventory",e))?,path,source])?;
        }
        cache.phases.push(files.finish());
        let commitments = crate::measurement::PhaseTimer::start("publication_commitments");
        let mut digest = blake3::Hasher::new();
        let inventory_digest = stream_graph(&transaction, metadata, &mut digest)?;
        digest.update(b"\n");
        let export = format!("blake3:{}", digest.finalize().to_hex());
        cache.phases.push(commitments.finish());
        transaction.execute(
            "UPDATE inventory_headers SET graph_digest=?1,inventory_digest=?2 WHERE view_id=?3",
            params![export, inventory_digest, view],
        )?;
        // The immutable graph commitment covers the complete canonical projection,
        // including syntax-derived records; mutable stores retain their row digest.
        set_store_digest(&transaction, &export)?;
        let selection = crate::measurement::PhaseTimer::start("publication_file_selection");
        super::syntax_cache::PreparedCache::prepare(cache)?.store(&transaction)?;
        cache.phases.push(selection.finish());
        transaction.execute("UPDATE temp.selected_knowledge_view SET id=?1", [selected])?;
        let commit = crate::measurement::PhaseTimer::start("publication_commit");
        transaction.commit()?;
        cache.phases.push(commit.finish());
        Ok((view.clone(), PublicationOutcome::Replaced { digest: view }))
    }
}

fn stream_nodes(
    connection: &Connection,
    output: &mut impl Write,
) -> Result<(), KnowledgeRepositoryError> {
    let mut statement=connection.prepare_cached(&format!("SELECT {NODE_COLUMNS} FROM {NODE_FROM} WHERE n.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) ORDER BY n.ordinal"))?;
    let mut rows = statement.query([])?;
    output.write_all(b"[")?;
    let mut first = true;
    while let Some(row) = rows.next()? {
        if !first {
            output.write_all(b",")?;
        }
        first = false;
        serde_json::to_writer(&mut *output, &hydrate_node(connection, row)?)?;
    }
    output.write_all(b"]")?;
    Ok(())
}

fn stream_records<T: serde::de::DeserializeOwned + Serialize>(
    connection: &Connection,
    collection: &str,
    order: &str,
    output: &mut impl Write,
) -> Result<(), KnowledgeRepositoryError> {
    let mut statement=connection.prepare_cached(&format!("SELECT r.payload FROM scoped_records r JOIN shared_payloads p ON p.payload_id=r.payload WHERE r.view_id=(SELECT id FROM temp.selected_knowledge_view) AND r.collection=?1 ORDER BY {order}"))?;
    let mut rows = statement.query([collection])?;
    output.write_all(b"[")?;
    let mut first = true;
    while let Some(row) = rows.next()? {
        if !first {
            output.write_all(b",")?;
        }
        first = false;
        let record: T = hydrate_record(connection, row.get(0)?)?;
        serde_json::to_writer(&mut *output, &record)?;
    }
    output.write_all(b"]")?;
    Ok(())
}

// Preserve the Rust graph serializer's field order and raw metadata encoding.
// Only the five large structural collections are supplied by retained rows.
struct Envelope(Vec<(String, Box<RawValue>)>);
impl<'de> serde::Deserialize<'de> for Envelope {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Envelope;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("graph object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(Envelope(fields))
            }
        }
        deserializer.deserialize_map(Fields)
    }
}
// Feed the same verified node bytes to both commitments. The inventory digest
// must not require a second hydration/serialization of every unchanged node.
struct NodeCommitments<'a, W> {
    output: &'a mut W,
    inventory: &'a mut blake3::Hasher,
}
impl<W: Write> Write for NodeCommitments<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.output.write_all(bytes)?;
        self.inventory.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

fn stream_graph(
    connection: &Connection,
    metadata: &TopologyGraph,
    output: &mut impl Write,
) -> Result<String, KnowledgeRepositoryError> {
    let mut inventory = blake3::Hasher::new();
    inventory.write_all(b"[")?;
    serde_json::to_writer(&mut inventory, "entmoot.inventory.membership.v1")?;
    inventory.write_all(b",")?;
    serde_json::to_writer(&mut inventory, &metadata.snapshot)?;
    inventory.write_all(b",")?;
    serde_json::to_writer(&mut inventory, &metadata.policy_ids)?;
    inventory.write_all(b",")?;
    let mut header = metadata.clone();
    header.nodes = Vec::new().into();
    header.edges = Vec::new().into();
    header.relation_edges.clear();
    header.relation_claims.clear();
    header.unresolved_relations.clear();
    let Envelope(fields) = serde_json::from_slice(&serde_json::to_vec(&header)?)?;
    output.write_all(b"{")?;
    let mut first = true;
    for (key, value) in fields {
        if key == "diagnostics" {
            for collection in ["relation_edges", "relation_claims", "unresolved_relations"] {
                let exists: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM knowledge_records WHERE collection=?1)",
                    [collection],
                    |row| row.get(0),
                )?;
                if !exists {
                    continue;
                }
                if !first {
                    output.write_all(b",")?;
                }
                first = false;
                serde_json::to_writer(&mut *output, collection)?;
                output.write_all(b":")?;
                match collection {
                    "relation_edges" => stream_records::<RelationEdge>(
                        connection,
                        collection,
                        "r.record_id",
                        output,
                    )?,
                    "relation_claims" => stream_records::<RelationClaim>(
                        connection,
                        collection,
                        "r.record_id",
                        output,
                    )?,
                    _ => stream_records::<UnresolvedRelation>(
                        connection,
                        collection,
                        "r.record_id",
                        output,
                    )?,
                }
            }
        }
        if !first {
            output.write_all(b",")?;
        }
        first = false;
        serde_json::to_writer(&mut *output, &key)?;
        output.write_all(b":")?;
        match key.as_str() {
            "nodes"=>stream_nodes(connection,&mut NodeCommitments { output, inventory: &mut inventory })?,
            "edges"=>stream_records::<crate::graph::StructuralEdge>(connection,"structural_edges","json_extract(CAST(p.body AS TEXT),'$.body.from'),json_extract(CAST(p.body AS TEXT),'$.body.to'),CASE json_extract(CAST(p.body AS TEXT),'$.body.kind') WHEN 'contains' THEN 0 WHEN 'imports' THEN 1 WHEN 'calls' THEN 2 WHEN 'has_control_site' THEN 3 ELSE 4 END",output)?,
            _=>output.write_all(value.get().as_bytes())?,
        }
    }
    output.write_all(b"}")?;
    inventory.write_all(b",")?;
    serde_json::to_writer(&mut inventory, &metadata.snapshot_files)?;
    inventory.write_all(b"]")?;
    Ok(format!("blake3:{}", inventory.finalize().to_hex()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_membership_lookup_and_ordering_avoid_repeated_full_scans() {
        // Use the real layout and lookup but omit payload hydration: this control
        // measures database selection work independently of JSON and wall time.
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE shared_payloads(payload_id INTEGER PRIMARY KEY); INSERT INTO shared_payloads VALUES(1);").unwrap();
        connection
            .execute_batch(super::super::inventory::SCHEMA)
            .unwrap();
        connection.execute_batch("CREATE TEMP TABLE selected_knowledge_view(id TEXT); INSERT INTO selected_knowledge_view VALUES('selected'); INSERT INTO inventory_headers VALUES(1,'selected',1,'',1,'',''),(2,'other',1,'',1,'','');
            WITH RECURSIVE ids(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM ids WHERE x<10000)
            INSERT INTO inventory_node_versions(node_key,node_id,payload,node_digest,kind,label,display_ref) SELECT x,'node-'||x,1,CAST(x AS BLOB),'callable','','' FROM ids;
            INSERT INTO inventory_nodes SELECT 1,node_key,node_key FROM inventory_node_versions;
            INSERT INTO inventory_node_versions(node_key,node_id,payload,node_digest,kind,label,display_ref) VALUES(10001,'node-10000',1,X'AAAA','callable','','');
            INSERT INTO inventory_nodes VALUES(2,0,10001);").unwrap();
        for (identity, expected) in [("node-10000", Some(10000_i64)), ("missing", None)] {
            let mut statement = connection.prepare(&node_lookup_sql()).unwrap();
            let ordinals = statement
                .query_map([identity], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(ordinals, expected.into_iter().collect::<Vec<_>>());
            let steps = statement.get_status(rusqlite::StatementStatus::VmStep);
            assert!(
                steps < 500,
                "point lookup scanned unrelated members: {steps} VM steps"
            );
        }
        connection.execute_batch(PREPARE_NODE_ORDER).unwrap();
        let mut reorder = connection.prepare(APPLY_NODE_ORDER).unwrap();
        assert_eq!(reorder.execute([]).unwrap(), 10000);
        let steps = reorder.get_status(rusqlite::StatementStatus::VmStep);
        assert!(
            steps < 2_000_000,
            "ordering rescanned the permutation: {steps} VM steps"
        );
        drop(reorder);
        connection.execute_batch(FINISH_NODE_ORDER).unwrap();
        let mut query = connection.prepare("SELECT node_id FROM inventory_nodes JOIN inventory_node_versions USING(node_key) WHERE view_key=1 ORDER BY ordinal").unwrap();
        let actual = query
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut expected = (1..=10000)
            .map(|id| format!("node-{id}"))
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn interrupted_structural_stream_rolls_back_view_and_keeps_sealed_file_units() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let database = data.path().join("knowledge.sqlite3");
        std::fs::write(source.path().join("a.py"), "def f():\n    return 1\n").unwrap();
        let (graph, mut cache) = crate::build_topology_with_retained_facts(
            source.path(),
            Default::default(),
            None,
            &database,
        )
        .unwrap();
        let mut repository = SqliteKnowledgeRepository::open(&database).unwrap();
        let result = repository.seal_structural_parts(&graph, &mut cache, |writer, _| {
            writer.append(&graph)?;
            Err(invalid_record(
                "fixture",
                "interrupted after file projection",
            ))
        });
        assert!(matches!(
            result,
            Err(KnowledgeRepositoryError::InvalidRecord {
                collection: "fixture",
                ..
            })
        ));
        assert!(!repository.is_initialized().unwrap());
        assert_eq!(
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM inventory_headers", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM scoped_records", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM file_fact_units", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let repeat = crate::retain_structural_inventory(
            source.path(),
            Default::default(),
            None,
            &database,
            false,
            None,
            &[],
        )
        .unwrap();
        assert!(
            matches!(repeat, crate::StructuralRefresh::Retained { cache, .. } if cache.reused == 1)
        );
    }
}
