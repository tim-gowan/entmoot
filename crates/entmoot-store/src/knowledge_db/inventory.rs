//! Revision membership, shared definitions, and source content. No binding,
//! claim, source span, or exact body is promoted to a shared definition.
use super::*;
use crate::{SourceReadRequest, SourceStore, TopologyGraph};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS shared_definitions (
    definition_id INTEGER PRIMARY KEY, policy_id TEXT NOT NULL, structural_hash TEXT NOT NULL,
    UNIQUE(policy_id,structural_hash)
);
CREATE TABLE IF NOT EXISTS shared_paths(path_id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS shared_source_files (
    file_id INTEGER PRIMARY KEY, digest TEXT NOT NULL UNIQUE, bytes BLOB
);
CREATE TABLE IF NOT EXISTS inventory_headers (
    view_key INTEGER PRIMARY KEY, view_id TEXT NOT NULL UNIQUE, payload INTEGER NOT NULL REFERENCES shared_payloads,
    graph_digest TEXT NOT NULL, source_checked INTEGER NOT NULL CHECK(source_checked IN(0,1)), inventory_digest TEXT NOT NULL, files_digest TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS inventory_node_versions (
    node_key INTEGER PRIMARY KEY, node_id TEXT NOT NULL,
    payload INTEGER NOT NULL REFERENCES shared_payloads, definition_id INTEGER REFERENCES shared_definitions,
    path_id INTEGER REFERENCES shared_paths, node_digest BLOB NOT NULL UNIQUE,
    kind TEXT NOT NULL, label TEXT NOT NULL, display_ref TEXT NOT NULL,
    fqn TEXT, address_id TEXT, logic_address TEXT, structural_hash TEXT
);
CREATE INDEX IF NOT EXISTS inventory_by_id ON inventory_node_versions(node_id,node_key);
CREATE INDEX IF NOT EXISTS inventory_by_definition ON inventory_node_versions(definition_id,node_key);
CREATE INDEX IF NOT EXISTS inventory_by_path ON inventory_node_versions(path_id,node_key);
CREATE INDEX IF NOT EXISTS inventory_by_display ON inventory_node_versions(display_ref,node_key);
CREATE INDEX IF NOT EXISTS inventory_by_address ON inventory_node_versions(address_id,node_key) WHERE address_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS inventory_by_logic ON inventory_node_versions(logic_address,node_key) WHERE logic_address IS NOT NULL;
CREATE INDEX IF NOT EXISTS inventory_by_unshared_hash ON inventory_node_versions(structural_hash,node_key) WHERE structural_hash IS NOT NULL;
CREATE TABLE IF NOT EXISTS inventory_nodes (
    view_key INTEGER NOT NULL REFERENCES inventory_headers ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, node_key INTEGER NOT NULL REFERENCES inventory_node_versions,
    PRIMARY KEY(view_key,ordinal)
) WITHOUT ROWID;
CREATE UNIQUE INDEX inventory_members_by_version ON inventory_nodes(view_key,node_key);
CREATE INDEX IF NOT EXISTS definitions_by_hash ON shared_definitions(structural_hash,definition_id);
CREATE TABLE IF NOT EXISTS inventory_files (
    view_key INTEGER NOT NULL REFERENCES inventory_headers ON DELETE CASCADE, ordinal INTEGER NOT NULL, path_id INTEGER NOT NULL REFERENCES shared_paths,
    file_id INTEGER NOT NULL REFERENCES shared_source_files,
    PRIMARY KEY(view_key,ordinal), UNIQUE(view_key,path_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS inventory_by_content ON inventory_files(file_id,view_key,path_id);
-- Projection membership references the existing immutable nodes/payloads. It
-- retains no second graph or syntax body, and is sealed with its parser unit.
CREATE TABLE IF NOT EXISTS structural_file_projections (
    unit_id INTEGER PRIMARY KEY REFERENCES file_fact_units ON DELETE CASCADE,
    commitment BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS structural_file_nodes (
    unit_id INTEGER NOT NULL REFERENCES structural_file_projections ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, node_key INTEGER NOT NULL REFERENCES inventory_node_versions,
    PRIMARY KEY(unit_id,ordinal)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS structural_file_edges (
    unit_id INTEGER NOT NULL REFERENCES structural_file_projections ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, record_id TEXT NOT NULL, payload INTEGER NOT NULL REFERENCES shared_payloads,
    PRIMARY KEY(unit_id,ordinal)
) WITHOUT ROWID;
"#;

/// Local storage locator. Portable query payloads never contain this layout.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeViewLocator {
    pub database: PathBuf,
    pub view: String,
}

impl KnowledgeViewLocator {
    pub fn read(path: &Path) -> Result<Option<Self>, KnowledgeRepositoryError> {
        if !path.to_string_lossy().ends_with(".view.json") {
            return Ok(None);
        }
        let mut value: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        entmoot_contracts::ContentDigest::parse(value.view.clone())
            .map_err(|e| invalid_record("view", e))?;
        if value.database.is_relative() {
            value.database = path.parent().unwrap_or(Path::new(".")).join(value.database);
        }
        Ok(Some(value))
    }
}

impl SqliteKnowledgeRepository {
    /// Seal an immutable revision with its inventory and optional file manifest.
    /// A publication selects this whole view; it does not need mutable per-record
    /// owner/dependency/visibility histories. The caller exposes its locator only
    /// after this transaction commits. Existing data is never reset on conflict.
    pub fn seal_inventory(
        &mut self,
        graph: &TopologyGraph,
        source: Option<&dyn SourceStore>,
        cache: Option<&super::syntax_cache::SyntaxCache>,
    ) -> Result<PublicationOutcome, KnowledgeRepositoryError> {
        let prepared = cache
            .map(super::syntax_cache::PreparedCache::prepare)
            .transpose()?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view))",
            [], |r| r.get(0),
        )?;
        import_graph_in_transaction(&transaction, graph, source)?;
        if let Some(prepared) = prepared {
            prepared.store(&transaction)?;
        }
        let digest: String =
            transaction.query_row("SELECT id FROM temp.selected_knowledge_view", [], |r| {
                r.get(0)
            })?;
        transaction.commit()?;
        Ok(if existed {
            PublicationOutcome::Unchanged { digest }
        } else {
            PublicationOutcome::Replaced { digest }
        })
    }

    /// Persist inventory independently of the graph export. The caller commits
    /// knowledge before this step and exposes the locator only after success.
    pub fn store_inventory(
        &mut self,
        graph: &TopologyGraph,
        source: Option<&dyn SourceStore>,
    ) -> Result<(), KnowledgeRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        store_inventory_in_transaction(&transaction, graph, source)?;
        transaction.commit()?;
        Ok(())
    }

    /// Hydrate a bounded inventory page. A node commitment also covers its
    /// shared path and definition, avoiding whole-graph validation for this read.
    pub fn inventory_nodes(
        &self,
        path: Option<&str>,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<crate::read::InventoryNodePage>, KnowledgeRepositoryError> {
        self.inventory_nodes_matching(path, &[], after, limit)
    }

    /// Exact identity alternatives are narrowed by retained indexes before any
    /// node detail is hydrated. Empty alternatives select the ordinary page.
    pub fn inventory_nodes_matching(
        &self,
        path: Option<&str>,
        exact: &[String],
        after: Option<i64>,
        limit: usize,
    ) -> Result<Option<crate::read::InventoryNodePage>, KnowledgeRepositoryError> {
        if !(1..=1024).contains(&limit) {
            return Err(invalid_record(
                "inventory",
                "node page limit must be in 1..=1024",
            ));
        }
        if self.inventory_export_digest()?.is_none() {
            return Ok(None);
        }
        if exact.len() > 8 {
            return Err(invalid_record(
                "inventory",
                "at most eight identity alternatives are allowed",
            ));
        }
        let where_path = if path.is_some() {
            " AND n.node_key IN (SELECT node_key FROM inventory_node_versions WHERE path_id=(SELECT path_id FROM shared_paths WHERE path=?3))"
        } else {
            ""
        };
        let where_identity = if exact.is_empty() {
            ""
        } else {
            r#" AND n.node_key IN (
            SELECT node_key FROM inventory_node_versions WHERE node_id IN (SELECT value FROM json_each(?4))
            UNION SELECT node_key FROM inventory_node_versions INDEXED BY inventory_by_display WHERE display_ref IN (SELECT value FROM json_each(?4))
            UNION SELECT node_key FROM inventory_node_versions INDEXED BY inventory_by_address WHERE address_id IN (SELECT value FROM json_each(?4))
            UNION SELECT node_key FROM inventory_node_versions INDEXED BY inventory_by_logic WHERE logic_address IN (SELECT value FROM json_each(?4))
            UNION SELECT node_key FROM inventory_node_versions INDEXED BY inventory_by_unshared_hash WHERE structural_hash IN (SELECT value FROM json_each(?4))
            UNION SELECT node_key FROM inventory_node_versions WHERE definition_id IN (SELECT definition_id FROM shared_definitions WHERE structural_hash IN (SELECT value FROM json_each(?4)))
        )"#
        };
        let from = if path.is_some() || !exact.is_empty() {
            NODE_FROM.replace(
                "inventory_nodes n",
                "inventory_nodes n INDEXED BY inventory_members_by_version",
            )
        } else {
            NODE_FROM.to_owned()
        };
        let sql = format!("SELECT {NODE_COLUMNS} FROM {from} WHERE n.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) AND n.ordinal>?1{where_path}{where_identity} ORDER BY n.ordinal LIMIT ?2");
        let mut statement = self.connection.prepare_cached(&sql)?;
        let sql_limit = i64::try_from(limit).map_err(|e| invalid_record("inventory", e))?;
        let after = after.unwrap_or(-1);
        let exact_json = serde_json::to_string(exact)?;
        let mut rows = if !exact.is_empty() {
            statement.query(params![after, sql_limit + 1, path, exact_json])?
        } else if let Some(path) = path {
            statement.query(params![after, sql_limit + 1, path])?
        } else {
            statement.query(params![after, sql_limit + 1])?
        };
        let mut nodes = Vec::new();
        let mut page_bytes = 0usize;
        let mut last = None;
        let mut more = false;
        while let Some(row) = rows.next()? {
            if nodes.len() == limit {
                more = true;
                break;
            }
            let node = hydrate_node(&self.connection, row)?;
            let expected: Vec<u8> = row.get(4)?;
            let encoded = serde_json::to_vec(&node)?;
            if blake3::hash(&encoded).as_bytes().as_slice() != expected {
                return Err(invalid_record(
                    "inventory",
                    "node membership commitment mismatch",
                ));
            }
            if encoded.len() > 8 * 1024 * 1024 {
                return Err(KnowledgeRepositoryError::BufferLimit(
                    "inventory node exceeds 8 MiB",
                ));
            }
            if page_bytes + encoded.len() > 8 * 1024 * 1024 {
                more = true;
                break;
            }
            page_bytes += encoded.len();
            last = Some(row.get(0)?);
            nodes.push(node);
        }
        Ok(Some(crate::read::InventoryNodePage {
            nodes,
            next_ordinal: if more { last } else { None },
        }))
    }

    /// Read only publication identity and the committed source manifest. Node and
    /// evidence payloads are deliberately untouched; this is not a graph view.
    pub fn inventory_metadata(
        &self,
    ) -> Result<Option<crate::read::InventoryMetadata>, KnowledgeRepositoryError> {
        let row: Option<(i64, String)> = self.connection.query_row(
            "SELECT payload,files_digest FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        let Some((payload, expected)) = row else {
            return Ok(None);
        };
        let header: TopologyGraph = hydrate_record(&self.connection, payload)?;
        let mut statement = self.connection.prepare_cached("SELECT p.path,f.digest FROM inventory_files m JOIN shared_paths p USING(path_id) JOIN shared_source_files f USING(file_id) WHERE m.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) ORDER BY ordinal")?;
        let files = statement
            .query_map([], |r| {
                Ok(crate::SnapshotFileDigest {
                    path: r.get(0)?,
                    content_digest: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if files_digest(&header.snapshot, &files)? != expected {
            return Err(invalid_record(
                "inventory",
                "source manifest commitment mismatch",
            ));
        }
        Ok(Some(crate::read::InventoryMetadata {
            root: header.root,
            snapshot: header.snapshot,
            policy_ids: header.policy_ids,
            snapshot_files: files,
        }))
    }

    /// Hydrate hot navigation records only. Provider symbol dumps, resolution
    /// payloads and ordered traces stay in the repository until requested.
    pub fn load_inventory(
        &self,
        query_only: bool,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        self.load_inventory_projection(if query_only {
            super::ReadProjection::Query
        } else {
            super::ReadProjection::Complete
        })
    }

    pub fn load_assessment_inventory(
        &self,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        self.load_inventory_projection(super::ReadProjection::Assessment)
    }

    fn load_inventory_projection(
        &self,
        projection: super::ReadProjection,
    ) -> Result<Option<TopologyGraph>, KnowledgeRepositoryError> {
        let payload: Option<i64> = self.connection.query_row("SELECT payload FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r| r.get(0)).optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let mut graph: TopologyGraph = hydrate_record(&self.connection, payload)?;
        // Preserve the exact membership encoding while releasing syntax that
        // will be reattached from visible source-of-record rows below.
        let mut commitment = blake3::Hasher::new();
        commitment.update(b"[");
        serde_json::to_writer(&mut commitment, "entmoot.inventory.membership.v1")?;
        commitment.update(b",");
        serde_json::to_writer(&mut commitment, &graph.snapshot)?;
        commitment.update(b",");
        serde_json::to_writer(&mut commitment, &graph.policy_ids)?;
        commitment.update(b",[");
        let mut statement = self.connection.prepare_cached(&format!("SELECT {NODE_COLUMNS} FROM {NODE_FROM} WHERE n.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) ORDER BY ordinal"))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let mut node = hydrate_node(&self.connection, row)?;
            if !graph.nodes.is_empty() {
                commitment.update(b",");
            }
            serde_json::to_writer(&mut commitment, &node)?;
            node.declaration = None;
            node.code_contract = None;
            graph.nodes.push(node);
        }
        let mut statement = self.connection.prepare_cached("SELECT p.path,f.digest FROM inventory_files m JOIN shared_paths p USING(path_id) JOIN shared_source_files f USING(file_id) WHERE m.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) ORDER BY ordinal")?;
        graph.snapshot_files = statement
            .query_map([], |r| {
                Ok(crate::SnapshotFileDigest {
                    path: r.get(0)?,
                    content_digest: r.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let expected: String = self.connection.query_row("SELECT inventory_digest FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r|r.get(0))?;
        commitment.update(b"],");
        serde_json::to_writer(&mut commitment, &graph.snapshot_files)?;
        commitment.update(b"]");
        if format!("blake3:{}", commitment.finalize().to_hex()) != expected {
            return Err(invalid_record(
                "inventory",
                "membership or shared definition commitment mismatch",
            ));
        }
        let knowledge = self.load_mode(projection)?;
        Ok(Some(knowledge.into_inventory(graph)))
    }

    pub fn inventory_export_digest(&self) -> Result<Option<String>, KnowledgeRepositoryError> {
        Ok(self.connection.query_row("SELECT graph_digest FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r|r.get(0)).optional()?)
    }

    pub fn verify_inventory_export(&self, path: &Path) -> Result<(), KnowledgeRepositoryError> {
        if let Some(locator) = KnowledgeViewLocator::read(path)? {
            let selected: String = self.connection.query_row(
                "SELECT id FROM temp.selected_knowledge_view",
                [],
                |r| r.get(0),
            )?;
            let same_database = self.connection.path().is_some_and(|database| {
                std::fs::canonicalize(database)
                    .ok()
                    .zip(std::fs::canonicalize(&locator.database).ok())
                    .is_some_and(|(left, right)| left == right)
            });
            if locator.view != selected
                || !same_database
                || self.inventory_export_digest()?.is_none()
            {
                return Err(invalid_record(
                    "inventory",
                    "locator does not match selected revision/build view",
                ));
            }
            return Ok(());
        }
        if let Some(expected) = self.inventory_export_digest()? {
            let mut file = std::fs::File::open(path)?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            if format!("blake3:{}", hasher.finalize().to_hex()) != expected {
                return Err(invalid_record(
                    "inventory",
                    "graph export does not match selected revision/build view",
                ));
            }
        }
        Ok(())
    }

    pub fn retained_source(
        &self,
        request: SourceReadRequest<'_>,
    ) -> Result<Option<crate::SourceFile>, KnowledgeRepositoryError> {
        let path = crate::source_store::normalize_source_path(request.path)
            .map_err(|e| invalid_record("source", e))?;
        let payload: i64 = self.connection.query_row("SELECT payload FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r| r.get(0))?;
        let header: TopologyGraph = hydrate_record(&self.connection, payload)?;
        if header.snapshot != *request.snapshot {
            return Err(invalid_record(
                "source",
                "snapshot does not match selected view",
            ));
        }
        let row: Option<(String,Option<Vec<u8>>)> = self.connection.query_row("SELECT f.digest,f.bytes FROM inventory_files m JOIN shared_paths p USING(path_id) JOIN shared_source_files f USING(file_id) WHERE m.view_key=(SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)) AND p.path=?1", [path.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((digest, bytes)) = row else {
            return Err(invalid_record("source", "path not in selected view"));
        };
        if digest != request.expected_content_digest {
            return Err(invalid_record(
                "source",
                "requested digest does not match membership",
            ));
        }
        let Some(bytes) = bytes else { return Ok(None) };
        if entmoot_contracts::blake3_hex(&bytes) != digest {
            return Err(invalid_record("source", "retained source digest mismatch"));
        }
        Ok(Some(crate::SourceFile {
            path,
            content_digest: digest,
            bytes,
        }))
    }
}

pub(super) const NODE_FROM: &str = "inventory_nodes n JOIN inventory_node_versions v USING(node_key) LEFT JOIN shared_definitions d USING(definition_id) LEFT JOIN shared_paths p USING(path_id)";

pub(super) const NODE_COLUMNS: &str = "n.ordinal,v.payload,COALESCE(d.structural_hash,v.structural_hash),p.path,v.node_digest,v.node_id,v.kind,v.label,v.display_ref,v.fqn,v.address_id,v.logic_address";

pub(super) fn hydrate_node(
    connection: &Connection,
    row: &rusqlite::Row<'_>,
) -> Result<crate::TopologyNode, KnowledgeRepositoryError> {
    let mut value: serde_json::Value = hydrate_record(connection, row.get(1)?)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid_record("inventory", "node detail must be an object"))?;
    for (column, name) in [
        (2, "structural_hash"),
        (3, "path"),
        (5, "id"),
        (6, "kind"),
        (7, "label"),
        (8, "display_ref"),
        (9, "fqn"),
        (10, "address_id"),
        (11, "logic_address"),
    ] {
        if let Some(value) = row.get::<_, Option<String>>(column)? {
            if object
                .insert(name.into(), serde_json::Value::String(value))
                .is_some()
            {
                return Err(invalid_record(
                    "inventory",
                    "duplicate node field in cold detail",
                ));
            }
        }
    }
    Ok(serde_json::from_value(value)?)
}

pub(super) fn intern_path(
    connection: &Connection,
    path: &str,
) -> Result<i64, KnowledgeRepositoryError> {
    if let Some(id) = connection
        .prepare_cached("SELECT path_id FROM shared_paths WHERE path=?1")?
        .query_row([path], |row| row.get(0))
        .optional()?
    {
        return Ok(id);
    }
    connection
        .prepare_cached("INSERT INTO shared_paths(path) VALUES(?1)")?
        .execute([path])?;
    Ok(connection.last_insert_rowid())
}

pub(super) fn files_digest(
    snapshot: &entmoot_contracts::SourceSnapshot,
    files: &[crate::SnapshotFileDigest],
) -> Result<String, KnowledgeRepositoryError> {
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(
        &mut hasher,
        &("entmoot.inventory.files.v1", snapshot, files),
    )?;
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn inventory_digest(graph: &TopologyGraph) -> Result<String, KnowledgeRepositoryError> {
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(
        &mut hasher,
        &(
            "entmoot.inventory.membership.v1",
            &graph.snapshot,
            &graph.policy_ids,
            &graph.nodes,
            &graph.snapshot_files,
        ),
    )?;
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn store_inventory_in_transaction(
    transaction: &Transaction<'_>,
    graph: &TopologyGraph,
    source: Option<&dyn SourceStore>,
) -> Result<(), KnowledgeRepositoryError> {
    let mut digest = blake3::Hasher::new();
    serde_json::to_writer(&mut digest, graph)?;
    digest.update(b"\n");
    let graph_digest = format!("blake3:{}", digest.finalize().to_hex());
    let prior: Option<(String,bool)> = transaction.query_row("SELECT graph_digest,source_checked FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)", [], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if prior
        .is_some_and(|(digest, checked)| digest == graph_digest && (source.is_none() || checked))
    {
        return Ok(());
    }
    graph
        .validate()
        .map_err(|e| invalid_record("inventory", e))?;
    let mut header = TopologyGraph::new(graph.root.clone());
    header.schema_version = graph.schema_version.clone();
    header.snapshot = graph.snapshot.clone();
    header.policy_ids = graph.policy_ids.clone();
    transaction.execute(
        "DELETE FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)",
        [],
    )?;
    let payload = intern_payload(transaction, &serde_json::to_vec(&header)?)?;
    transaction.execute("INSERT INTO inventory_headers VALUES(NULL,(SELECT id FROM temp.selected_knowledge_view),?1,?2,?3,?4,?5)", params![payload,graph_digest,source.is_some(),inventory_digest(graph)?,files_digest(&graph.snapshot, &graph.snapshot_files)?])?;
    for (ordinal, node) in graph.nodes.iter().enumerate() {
        store_node(transaction, &graph.policy_ids, ordinal, node)?;
    }
    for (ordinal, file) in graph.snapshot_files.iter().enumerate() {
        let path = intern_path(transaction, &file.path)?;
        let id = intern_source_file(transaction, &graph.snapshot, file, source)?;
        transaction.execute("INSERT INTO inventory_files VALUES((SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)),?1,?2,?3)", params![i64::try_from(ordinal).map_err(|e| invalid_record("inventory", e))?,path,id])?;
    }
    Ok(())
}

pub(super) fn store_node(
    transaction: &Transaction<'_>,
    policy_ids: &[String],
    ordinal: usize,
    node: &crate::TopologyNode,
) -> Result<(), KnowledgeRepositoryError> {
    let encoded = serde_json::to_vec(node)?;
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(KnowledgeRepositoryError::BufferLimit(
            "inventory node exceeds 8 MiB",
        ));
    }
    let mut value: serde_json::Value = serde_json::from_slice(&encoded)?;
    // Multiple policies without a node->policy witness cannot be
    // coalesced. Preserve the original occurrence instead of guessing.
    let definition = if let ([policy], Some(hash)) = (policy_ids, node.structural_hash.as_deref()) {
        transaction.execute(
            "INSERT OR IGNORE INTO shared_definitions(policy_id,structural_hash) VALUES(?1,?2)",
            params![policy, hash],
        )?;
        let id: i64 = transaction.query_row("SELECT definition_id FROM shared_definitions WHERE policy_id=?1 AND structural_hash=?2", params![policy,hash], |r| r.get(0))?;
        if let Some(object) = value.as_object_mut() {
            object.remove("structural_hash");
        }
        Some(id)
    } else {
        None
    };
    let path = node
        .path
        .as_deref()
        .map(|path| intern_path(transaction, path))
        .transpose()?;
    if let Some(object) = value.as_object_mut() {
        for key in [
            "id",
            "kind",
            "label",
            "display_ref",
            "path",
            "fqn",
            "address_id",
            "logic_address",
            "structural_hash",
        ] {
            object.remove(key);
        }
    }
    let payload = intern_payload(transaction, &serde_json::to_vec(&value)?)?;
    let node_digest = blake3::hash(&encoded);
    let unshared_hash = definition
        .is_none()
        .then_some(node.structural_hash.as_deref())
        .flatten();
    transaction.prepare_cached("INSERT OR IGNORE INTO inventory_node_versions VALUES(NULL,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")?.execute(params![node.id,payload,definition,path,node_digest.as_bytes().as_slice(),node.kind.as_str(),node.label,node.display_ref,node.fqn,node.address_id,node.logic_address,unshared_hash])?;
    let key: i64 = transaction
        .prepare_cached("SELECT node_key FROM inventory_node_versions WHERE node_digest=?1")?
        .query_row([node_digest.as_bytes().as_slice()], |row| row.get(0))?;
    // Sharing a version never bypasses its hot-field/payload commitment.
    let mut statement = transaction.prepare_cached(&format!("SELECT {} FROM inventory_node_versions v LEFT JOIN shared_definitions d USING(definition_id) LEFT JOIN shared_paths p USING(path_id) WHERE v.node_key=?1", NODE_COLUMNS.replace("n.ordinal", "0")))?;
    let mut rows = statement.query([key])?;
    let row = rows
        .next()?
        .ok_or_else(|| invalid_record("inventory", "node version disappeared"))?;
    if serde_json::to_vec(&hydrate_node(transaction, row)?)? != encoded {
        return Err(invalid_record(
            "inventory",
            "shared node version commitment mismatch",
        ));
    }
    transaction.prepare_cached("INSERT INTO inventory_nodes VALUES((SELECT view_key FROM inventory_headers WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)),?1,?2)")?.execute(params![i64::try_from(ordinal).map_err(|e|invalid_record("inventory",e))?,key])?;
    Ok(())
}

/// Shared byte identity for inventory and explicit Git replay inputs.
pub(super) fn intern_source_digest(
    transaction: &Transaction<'_>,
    digest: &str,
) -> Result<(i64, bool), KnowledgeRepositoryError> {
    entmoot_contracts::ContentDigest::parse(digest).map_err(|e| invalid_record("source", e))?;
    transaction.execute(
        "INSERT OR IGNORE INTO shared_source_files(digest) VALUES(?1)",
        [digest],
    )?;
    let (id, retained): (i64, bool) = transaction.query_row(
        "SELECT file_id,bytes IS NOT NULL FROM shared_source_files WHERE digest=?1",
        [digest],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((id, retained))
}

/// Callers verify bytes before entering mutation; never replace retained content.
pub(super) fn retain_source_bytes(
    transaction: &Transaction<'_>,
    id: i64,
    bytes: &[u8],
) -> Result<(), KnowledgeRepositoryError> {
    transaction.execute(
        "UPDATE shared_source_files SET bytes=?1 WHERE file_id=?2 AND bytes IS NULL",
        params![bytes, id],
    )?;
    Ok(())
}

pub(super) fn intern_source_file(
    transaction: &Transaction<'_>,
    snapshot: &entmoot_contracts::SourceSnapshot,
    file: &crate::SnapshotFileDigest,
    source: Option<&dyn SourceStore>,
) -> Result<i64, KnowledgeRepositoryError> {
    let (id, retained) = intern_source_digest(transaction, &file.content_digest)?;
    if !retained {
        if let Some(source) = source {
            let request = SourceReadRequest {
                snapshot,
                path: &file.path,
                expected_content_digest: &file.content_digest,
            };
            if source
                .read_immutable(request)
                .map_err(|e| invalid_record("source", e))?
                .is_none()
            {
                let content = source
                    .read_path(request)
                    .map_err(|e| invalid_record("source", e))?;
                retain_source_bytes(transaction, id, &content.bytes)?;
            }
        }
    }
    Ok(id)
}

/// Import one complete content-addressed result. Caller owns publication visibility.
pub(super) fn import_graph_in_transaction(
    transaction: &Transaction<'_>,
    graph: &TopologyGraph,
    source: Option<&dyn SourceStore>,
) -> Result<(), KnowledgeRepositoryError> {
    graph
        .validate()
        .map_err(|e| invalid_record("inventory", e))?;
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut hasher, graph)?;
    let view = format!("blake3:{}", hasher.finalize().to_hex());
    let selected: String =
        transaction.query_row("SELECT id FROM temp.selected_knowledge_view", [], |r| {
            r.get(0)
        })?;
    if selected != view {
        return Err(invalid_record(
            "inventory",
            "immutable view must equal graph digest",
        ));
    }
    let prior: Option<String> = transaction
        .query_row(
            "SELECT graph_digest FROM inventory_headers WHERE view_id=?1",
            [&selected],
            |r| r.get(0),
        )
        .optional()?;
    hasher.update(b"\n");
    let export = format!("blake3:{}", hasher.finalize().to_hex());
    if let Some(prior) = prior {
        if prior != export {
            return Err(invalid_record(
                "inventory",
                "immutable view already contains another graph",
            ));
        }
        // Adding digest-verified source bytes does not mutate a sealed view's
        // identities, records or visibility. Imports may have omitted these bytes.
        if source.is_some() {
            for file in &graph.snapshot_files {
                intern_source_file(transaction, &graph.snapshot, file, source)?;
            }
        }
        return Ok(());
    }
    // Explicit graph imports already own the complete immutable input. Retain
    // only borrowed lookup context and one bounded emission batch. Any later
    // validation/insertion failure rolls this entire private view back.
    let context = StoreValidationContext::from_graph(graph);
    let mut digest = store_digest_hasher();
    for batch in KnowledgeStoreBatches::from_graph(graph, PublicationBuffer::default())? {
        let batch = batch?;
        validate_store_records_in_context(&batch.store, &context)?;
        insert_store_chunk(transaction, &batch.store, &mut digest)?;
    }
    set_store_digest(
        transaction,
        &format!("blake3:{}", digest.finalize().to_hex()),
    )?;
    store_inventory_in_transaction(transaction, graph, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_identity_reads_preserve_ambiguity_and_avoid_unrelated_detail() {
        let directory = tempfile::tempdir().unwrap();
        let mut graph = TopologyGraph::new(directory.path().to_string_lossy());
        let hash = entmoot_contracts::blake3_hex(b"same-structure");
        // Multiple policies deliberately prevent shared-definition coalescing.
        graph.policy_ids = vec!["a".repeat(64), "b".repeat(64)];
        for (id, display, path) in [
            ("a", "module.work", "a.py"),
            ("b", "module.work", "b.py"),
            ("other", "other", "c.py"),
        ] {
            let mut node = crate::TopologyNode::shell(id, crate::NodeKind::Callable, id, None);
            node.display_ref = display.into();
            node.path = Some(path.into());
            node.structural_hash = Some(hash.clone());
            graph.upsert_node(node);
        }
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        repository
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        repository.store_inventory(&graph, None).unwrap();
        let page = repository
            .inventory_nodes_matching(None, &["module.work".into()], None, 1)
            .unwrap()
            .unwrap();
        assert_eq!(page.nodes[0].id, "a");
        let second = repository
            .inventory_nodes_matching(None, &["module.work".into()], page.next_ordinal, 1)
            .unwrap()
            .unwrap();
        assert_eq!(second.nodes[0].id, "b");
        assert!(second.next_ordinal.is_none());
        let by_hash = repository
            .inventory_nodes_matching(Some("b.py"), &[hash], None, 4)
            .unwrap()
            .unwrap();
        assert_eq!(by_hash.nodes.len(), 1);
        assert_eq!(by_hash.nodes[0].id, "b");
        let restored = repository.load_inventory(false).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&restored.nodes).unwrap(),
            serde_json::to_value(&graph.nodes).unwrap()
        );
        repository
            .connection
            .execute(
                "UPDATE inventory_node_versions SET node_digest=zeroblob(32) WHERE node_id='other'",
                [],
            )
            .unwrap();
        assert_eq!(
            repository
                .inventory_nodes_matching(None, &["a".into()], None, 4)
                .unwrap()
                .unwrap()
                .nodes[0]
                .id,
            "a"
        );
        assert!(repository.inventory_nodes(None, None, 4).is_err());
        repository
            .connection
            .execute(
                "UPDATE inventory_node_versions SET label='changed' WHERE node_id='a'",
                [],
            )
            .unwrap();
        assert!(
            repository
                .inventory_nodes_matching(None, &["a".into()], None, 4)
                .is_err(),
            "hot fields participate in the node commitment"
        );
    }

    #[test]
    fn sealed_inventory_uses_view_membership_and_rolls_back_incomplete_selection() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("a.py"), "def first():\n    return 1\n").unwrap();
        let (first, cache) =
            crate::build_topology_with_syntax_cache(source.path(), Default::default(), None)
                .unwrap();
        let database = data.path().join("knowledge.sqlite3");
        let view = first.content_digest().unwrap();
        let mut repository = SqliteKnowledgeRepository::open_view(&database, &view).unwrap();
        repository.connection.execute_batch("CREATE TRIGGER reject_manifest BEFORE INSERT ON file_fact_manifests BEGIN SELECT RAISE(ABORT, 'interrupted selection'); END;").unwrap();
        assert!(repository
            .seal_inventory(&first, None, Some(&cache))
            .is_err());
        assert!(!repository.is_initialized().unwrap());
        assert!(repository.inventory_metadata().unwrap().is_none());
        let records: i64 = repository
            .connection
            .query_row("SELECT COUNT(*) FROM scoped_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(records, 0);
        repository
            .connection
            .execute_batch("DROP TRIGGER reject_manifest")
            .unwrap();
        assert!(!repository
            .seal_inventory(&first, None, Some(&cache))
            .unwrap()
            .is_unchanged());
        assert!(repository
            .seal_inventory(&first, None, Some(&cache))
            .unwrap()
            .is_unchanged());
        for table in [
            "scoped_record_owners",
            "scoped_record_dependencies",
            "scoped_visibility_events",
        ] {
            let count: i64 = repository
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "immutable view does not need {table}");
        }
        std::fs::write(source.path().join("a.py"), "def renamed():\n    return 2\n").unwrap();
        let (head, cache) =
            crate::build_topology_with_syntax_cache(source.path(), Default::default(), None)
                .unwrap();
        assert!(repository
            .seal_inventory(&head, None, Some(&cache))
            .is_err());
        let mut other =
            SqliteKnowledgeRepository::open_view(&database, &head.content_digest().unwrap())
                .unwrap();
        other.seal_inventory(&head, None, Some(&cache)).unwrap();
        let old = repository.load_inventory(false).unwrap().unwrap();
        assert_eq!(old.snapshot, first.snapshot);
        assert!(old.nodes.iter().any(|node| node.label == "first"));
        assert!(!old.nodes.iter().any(|node| node.label == "renamed"));
        assert_eq!(
            other.inventory_metadata().unwrap().unwrap().snapshot,
            head.snapshot
        );
    }

    #[test]
    fn shared_inventory_preserves_all_family_occurrences_and_source_revisions() {
        let source = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        let fixtures = [
            ("a.rs", "fn work() -> i32 { 1 }\n", "// comment\n"),
            ("a.py", "def work():\n    return 1\n", "# comment\n"),
            ("a.js", "function work() { return 1; }\n", "// comment\n"),
            (
                "a.ts",
                "function work(): number { return 1; }\n",
                "// comment\n",
            ),
            (
                "a.tsx",
                "function Work() { return <div/>; }\n",
                "// comment\n",
            ),
            (
                "a.go",
                "package a\nfunc Work() int { return 1 }\n",
                "// comment\n",
            ),
            (
                "A.java",
                "class A { int work() { return 1; } }\n",
                "// comment\n",
            ),
            (
                "A.cs",
                "class A { int Work() { return 1; } }\n",
                "// comment\n",
            ),
            ("a.c", "int work(void) { return 1; }\n", "// comment\n"),
            ("a.cpp", "int work() { return 1; }\n", "// comment\n"),
        ];
        for (path, bytes, _) in fixtures {
            std::fs::write(source.path().join(path), bytes).unwrap();
        }
        let graph = crate::build_topology_from_repo_with_options(
            source.path(),
            crate::TopologyBuildOptions::default(),
        )
        .unwrap();
        let source_store = crate::GixSourceStore::open(source.path()).unwrap();
        let db = database.path().join("shared.sqlite3");
        let mut a =
            SqliteKnowledgeRepository::open_view(&db, &entmoot_contracts::blake3_hex(b"a/build-a"))
                .unwrap();
        a.replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        a.store_inventory(&graph, Some(&source_store)).unwrap();
        let changes = a.connection.total_changes();
        a.store_inventory(&graph, Some(&source_store)).unwrap();
        assert_eq!(
            changes,
            a.connection.total_changes(),
            "unchanged inventory must not rewrite shared rows"
        );
        let count: i64 = a
            .connection
            .query_row("SELECT COUNT(*) FROM shared_definitions", [], |r| r.get(0))
            .unwrap();
        assert!(count > 0);
        let roundtrip = a.load_inventory(false).unwrap().unwrap();
        let expected = serde_json::to_value(&graph).unwrap();
        let observed = serde_json::to_value(&roundtrip).unwrap();
        for (key, value) in expected.as_object().unwrap() {
            let normalize = |value: &serde_json::Value| {
                if let Some(array) = value.as_array() {
                    let mut rows: Vec<_> = array
                        .iter()
                        .map(|v| serde_json::to_string(v).unwrap())
                        .collect();
                    rows.sort();
                    serde_json::to_value(rows).unwrap()
                } else {
                    value.clone()
                }
            };
            assert_eq!(
                normalize(value),
                normalize(&observed[key]),
                "collection {key}"
            );
        }
        for (path, bytes, comment) in fixtures {
            std::fs::write(source.path().join(path), format!("{comment}{bytes}")).unwrap();
        }
        let newer = crate::build_topology_from_repo_with_options(
            source.path(),
            crate::TopologyBuildOptions::default(),
        )
        .unwrap();
        let mut b =
            SqliteKnowledgeRepository::open_view(&db, &entmoot_contracts::blake3_hex(b"b/build-b"))
                .unwrap();
        b.replace_all(&KnowledgeStore::extract_from_graph(&newer))
            .unwrap();
        b.store_inventory(&newer, Some(&source_store)).unwrap();
        let next_count: i64 = b
            .connection
            .query_row("SELECT COUNT(*) FROM shared_definitions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, next_count,
            "ordinary comments do not create definitions"
        );
        for file in &graph.snapshot_files {
            let request = SourceReadRequest {
                snapshot: &graph.snapshot,
                path: &file.path,
                expected_content_digest: &file.content_digest,
            };
            assert!(a.retained_source(request).unwrap().is_some());
            assert!(
                b.retained_source(request).is_err(),
                "cross-revision source must be refused"
            );
        }
        let old = a.load_inventory(true).unwrap().unwrap();
        assert_eq!(old.snapshot, graph.snapshot);
        assert_ne!(old.snapshot, newer.snapshot);
        assert_eq!(
            old.nodes
                .iter()
                .find(|n| n.kind == crate::NodeKind::Callable)
                .unwrap()
                .start_line,
            graph
                .nodes
                .iter()
                .find(|n| n.kind == crate::NodeKind::Callable)
                .unwrap()
                .start_line
        );
        a.connection.execute("UPDATE shared_definitions SET structural_hash=?1 WHERE definition_id=(SELECT MIN(definition_id) FROM shared_definitions)", [entmoot_contracts::blake3_hex(b"corrupt definition pointer")]).unwrap();
        assert!(a
            .load_inventory(true)
            .unwrap_err()
            .to_string()
            .contains("commitment mismatch"));
    }
}
