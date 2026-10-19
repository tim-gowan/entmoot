//! Offline whole-view collection. No record TTL or invalidation semantics here.
use super::*;
use crate::storage_lease::StorageLease;

pub struct Maintenance {
    connection: Connection,
    _lease: StorageLease,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreCatalog {
    pub views: BTreeSet<String>,
    pub fence: String,
}

impl Maintenance {
    pub fn open(path: &Path) -> Result<Self, KnowledgeRepositoryError> {
        let lease = crate::storage_lease::exclusive(path)?;
        let connection = Connection::open_with_flags(
            crate::sqlite_startup::file_path(path),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(1))?;
        let actual = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if actual != STORAGE_SCHEMA_VERSION {
            return Err(KnowledgeRepositoryError::UnsupportedSchema {
                actual,
                supported: STORAGE_SCHEMA_VERSION,
            });
        }
        fingerprint::verify(&connection)?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;",
        )?;
        Ok(Self {
            connection,
            _lease: lease,
        })
    }

    pub fn catalog(&self) -> Result<StoreCatalog, KnowledgeRepositoryError> {
        let views = self.connection.prepare("SELECT view_id FROM scoped_metadata UNION SELECT view_id FROM inventory_headers ORDER BY view_id")?
            .query_map([], |r| r.get(0))?.collect::<Result<BTreeSet<String>, _>>()?;
        // Hash all physical rows, including inactive facts and ownership. This is
        // a maintenance fence, deliberately not a query identity or hot-path scan.
        let mut digest = blake3::Hasher::new();
        digest.update(b"entmoot.retention.store.v1");
        for table in TABLES {
            digest.update(table.as_bytes());
            let count = self
                .connection
                .prepare(&format!("SELECT * FROM {table}"))?
                .column_count();
            let order = (1..=count)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut stmt = self
                .connection
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))?;
            let columns = stmt.column_count();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                for column in 0..columns {
                    use rusqlite::types::ValueRef;
                    let value = row.get_ref(column)?;
                    let bytes = match value {
                        ValueRef::Null => vec![0],
                        ValueRef::Integer(v) => [vec![1], v.to_le_bytes().to_vec()].concat(),
                        ValueRef::Real(v) => [vec![2], v.to_bits().to_le_bytes().to_vec()].concat(),
                        ValueRef::Text(v) => [vec![3], v.to_vec()].concat(),
                        ValueRef::Blob(v) => [vec![4], v.to_vec()].concat(),
                    };
                    digest.update(&(bytes.len() as u64).to_le_bytes());
                    digest.update(&bytes);
                }
            }
        }
        Ok(StoreCatalog {
            views,
            fence: format!("blake3:{}", digest.finalize().to_hex()),
        })
    }

    pub fn artifact_digests(
        &self,
        views: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, KnowledgeRepositoryError> {
        let mut digests = BTreeSet::new();
        let mut stmt = self.connection.prepare(
            "SELECT payload FROM scoped_records WHERE view_id=?1 AND collection='artifacts'",
        )?;
        for view in views {
            for payload in stmt.query_map([view], |r| r.get::<_, i64>(0))? {
                let artifact: ArtifactManifest = hydrate_record(&self.connection, payload?)?;
                artifact
                    .validate_structure()
                    .map_err(|e| invalid_record("artifact", e))?;
                digests.insert(artifact.content_digest);
            }
        }
        Ok(digests)
    }

    pub fn collect(&mut self, remove: &BTreeSet<String>) -> Result<(), KnowledgeRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for view in remove {
            let now = crate::clock::now_ms().map_err(|e| invalid_record("retention", e))?;
            if transaction.query_row("SELECT EXISTS(SELECT 1 FROM local_query_checkpoints WHERE view_id=?1 AND expires_ms>?2)", params![view, i64::try_from(now).map_err(|e| invalid_record("retention", e))?], |r| r.get::<_, bool>(0))? {
                return Err(invalid_record("retention", "view pinned by resumable query"));
            }
            if transaction.query_row("SELECT EXISTS(SELECT 1 FROM local_workspaces WHERE view_id=?1 UNION ALL SELECT 1 FROM local_operation_pins WHERE view_id=?1)", [view], |r| r.get::<_, bool>(0))? {
                return Err(invalid_record("retention", "view pinned by workspace or active operation"));
            }
            transaction.execute("DELETE FROM local_view_seals WHERE view_id=?1", [view])?;
            for table in [
                "local_git_captures",
                "scoped_visibility_events",
                "scoped_record_dependency_sets",
                "scoped_record_dependencies",
                "scoped_record_owners",
                "scoped_records",
                "scoped_metadata",
                "inventory_headers",
                "file_fact_manifests",
            ] {
                transaction.execute(&format!("DELETE FROM {table} WHERE view_id=?1"), [view])?;
            }
        }
        // Mark every retained membership, including inactive records. Metadata
        // references are codec-owned; malformed envelopes abort the transaction.
        transaction.execute_batch("DELETE FROM file_fact_units WHERE unit_id NOT IN (SELECT unit_id FROM file_fact_membership);
            DELETE FROM inventory_node_versions WHERE node_key NOT IN (SELECT node_key FROM inventory_nodes UNION SELECT node_key FROM structural_file_nodes);
            CREATE TEMP TABLE retained_payloads(id INTEGER PRIMARY KEY);
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM scoped_records;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM inventory_node_versions;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM structural_file_edges;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM inventory_headers;
            INSERT OR IGNORE INTO retained_payloads SELECT parts FROM file_fact_membership WHERE parts IS NOT NULL;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM local_file_progress;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM local_git_captures;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM local_file_outcomes WHERE payload IS NOT NULL;
            INSERT OR IGNORE INTO retained_payloads SELECT payload FROM local_inventory_parts;
            DELETE FROM shared_payloads WHERE payload_id NOT IN (SELECT id FROM retained_payloads);
            CREATE TEMP TABLE retained_metadata(id INTEGER PRIMARY KEY);
            INSERT OR IGNORE INTO retained_metadata SELECT j.value FROM shared_payloads p, json_each(CAST(p.body AS TEXT),'$.metadata') j;
            INSERT OR IGNORE INTO retained_metadata SELECT metadata FROM file_fact_sites;
            DELETE FROM shared_metadata WHERE metadata_id NOT IN (SELECT id FROM retained_metadata);
            DELETE FROM shared_definitions WHERE definition_id NOT IN (SELECT definition_id FROM inventory_node_versions WHERE definition_id IS NOT NULL);
            DELETE FROM shared_source_files WHERE file_id NOT IN (SELECT file_id FROM inventory_files UNION SELECT file_id FROM local_git_files);
            DELETE FROM shared_paths WHERE path_id NOT IN (SELECT path_id FROM inventory_node_versions WHERE path_id IS NOT NULL UNION SELECT path_id FROM inventory_files);
            DROP TABLE retained_payloads; DROP TABLE retained_metadata;")?;
        if transaction
            .prepare("PRAGMA foreign_key_check")?
            .exists([])?
        {
            return Err(invalid_record(
                "retention",
                "foreign key violation; collection rolled back",
            ));
        }
        if transaction.prepare("SELECT 1 FROM shared_payloads p, json_each(CAST(p.body AS TEXT),'$.metadata') j LEFT JOIN shared_metadata m ON m.metadata_id=j.value WHERE m.metadata_id IS NULL LIMIT 1")?.exists([])? {
            return Err(invalid_record("retention", "missing interned metadata; collection rolled back"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn compact(&self) -> Result<(), KnowledgeRepositoryError> {
        // Requires spare space; never run implicitly at the storage-full boundary.
        self.connection.execute_batch("VACUUM")?;
        self.checkpoint()
    }

    pub fn checkpoint(&self) -> Result<(), KnowledgeRepositoryError> {
        let busy: i64 = self
            .connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(invalid_record(
                "retention",
                "checkpoint busy; physical reclamation incomplete",
            ));
        }
        Ok(())
    }
}

const TABLES: &[&str] = &[
    "local_integration_packs",
    "local_git_captures",
    "local_git_files",
    "local_file_progress",
    "local_file_outcomes",
    "local_inventory_parts",
    "local_workspaces",
    "local_operations",
    "local_scheduler_profiles",
    "local_operation_waiters",
    "local_operation_pins",
    "local_operation_outputs",
    "local_view_seals",
    "local_query_checkpoints",
    "storage_fingerprint",
    "shared_metadata",
    "shared_payloads",
    "shared_claim_links",
    "scoped_records",
    "scoped_metadata",
    "scoped_record_owners",
    "scoped_record_dependencies",
    "scoped_record_dependency_sets",
    "scoped_visibility_events",
    "shared_definitions",
    "shared_paths",
    "shared_source_files",
    "inventory_headers",
    "inventory_nodes",
    "inventory_node_versions",
    "inventory_files",
    "file_fact_units",
    "file_fact_callables",
    "file_fact_sites",
    "file_fact_manifests",
    "file_fact_membership",
    "structural_file_projections",
    "structural_file_nodes",
    "structural_file_edges",
];
