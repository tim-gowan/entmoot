//! Pull-driven ETL publication. SQLite owns staging; RAII owns rollback.
use super::*;

/// Already normalized records for one independently regenerable owner. Include
/// schema references (such as an attachment's location) somewhere in the
/// publication; immutable context records may repeat. Forward references and
/// generic dependency inputs are checked before publication.
pub struct OwnerBatch {
    pub store: KnowledgeStore,
    pub dependencies: Vec<KnowledgeDependency>,
}

/// Per-batch serialized payload and record limits, not a process-RSS guarantee.
#[derive(Debug, Clone, Copy)]
pub struct PublicationBuffer {
    pub max_records: usize,
    pub max_bytes: usize,
}

impl Default for PublicationBuffer {
    fn default() -> Self {
        Self {
            max_records: 1024,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Whether an owner publication mutated durable membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationOutcome {
    /// Desired owner set matches the last committed publication digest.
    Unchanged { digest: String },
    /// Owner membership and/or newly inserted immutable rows were committed.
    Replaced { digest: String },
}

impl Default for PublicationOutcome {
    fn default() -> Self {
        Self::Unchanged {
            digest: String::new(),
        }
    }
}

impl PublicationOutcome {
    pub fn digest(&self) -> &str {
        match self {
            Self::Unchanged { digest } | Self::Replaced { digest } => digest,
        }
    }

    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged { .. })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unchanged { .. } => "unchanged",
            Self::Replaced { .. } => "replaced",
        }
    }
}

/// Returned after the owner publication is applied or recognized as unchanged.
#[derive(Debug, Default)]
pub struct BufferedPublicationReceipt {
    pub batches: usize,
    pub peak_batch_records: usize,
    pub peak_batch_bytes: usize,
    pub publication: KnowledgeOwnershipStats,
    pub outcome: PublicationOutcome,
}

pub(super) fn publish(
    connection: &mut Connection,
    owner: &str,
    batches: &mut dyn Iterator<Item = Result<OwnerBatch, KnowledgeRepositoryError>>,
    buffer: PublicationBuffer,
) -> Result<BufferedPublicationReceipt, KnowledgeRepositoryError> {
    validate_owner(owner)?;
    if buffer.max_records == 0 || buffer.max_bytes == 0 {
        return Err(KnowledgeRepositoryError::BufferLimit(
            "positive limits required",
        ));
    }

    // Pull all batches first so publication identity can be compared without a
    // write transaction when nothing changed (ADR-0017 buffer limits still apply).
    let mut collected = Vec::new();
    let mut receipt = BufferedPublicationReceipt::default();
    let mut union_rows: BTreeMap<KnowledgeRecordRef, StoredRow> = BTreeMap::new();
    let mut union_deps: BTreeMap<KnowledgeRecordRef, BTreeSet<KnowledgeRecordRef>> =
        BTreeMap::new();
    for batch in batches {
        let batch = batch?;
        let mut bytes = 0usize;
        let mut count = 0usize;
        let mut rows: BTreeMap<KnowledgeRecordRef, StoredRow> = BTreeMap::new();
        for_each_store_row(&batch.store, |row| {
            count += 1;
            bytes = bytes.saturating_add(row.payload.len());
            if count > buffer.max_records || bytes > buffer.max_bytes {
                return Err(KnowledgeRepositoryError::BufferLimit(
                    "records or payload bytes",
                ));
            }
            let record = KnowledgeRecordRef::new(row.collection, row.record_id.clone());
            if let Some(existing) = rows.get(&record) {
                if existing.payload != row.payload {
                    return Err(collision(&record));
                }
            } else {
                rows.insert(record, row);
            }
            Ok(())
        })?;
        for dependency in &batch.dependencies {
            bytes = bytes.saturating_add(
                serde_json::to_vec(&(&dependency.dependent, &dependency.inputs))?.len(),
            );
            if bytes > buffer.max_bytes {
                return Err(KnowledgeRepositoryError::BufferLimit("dependency bytes"));
            }
        }
        let dependencies = validate_dependency_batch(&rows, &batch.dependencies)?;
        for (record, row) in &rows {
            if let Some(existing) = union_rows.get(record) {
                if existing.payload != row.payload {
                    return Err(collision(record));
                }
            } else {
                union_rows.insert(record.clone(), row.clone());
            }
        }
        for (record, inputs) in dependencies {
            match union_deps.entry(record.clone()) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(inputs);
                }
                std::collections::btree_map::Entry::Occupied(existing) => {
                    if existing.get() != &inputs {
                        return Err(collision(&record));
                    }
                }
            }
        }
        receipt.batches += 1;
        receipt.peak_batch_records = receipt.peak_batch_records.max(count);
        receipt.peak_batch_bytes = receipt.peak_batch_bytes.max(bytes);
        collected.push(batch);
    }

    // Batch boundaries have no semantic meaning. Validate references against
    // this publication before either the no-change shortcut or durable writes.
    let context = StoreValidationContext::new(collected.iter().map(|batch| &batch.store));
    for batch in &collected {
        validate_store_records_in_context(&batch.store, &context)?;
    }

    let digest = owner_publication_digest(owner, &union_rows, &union_deps);
    if owner_publication_digest_matches(connection, owner, &digest)? {
        receipt.outcome = PublicationOutcome::Unchanged { digest: digest };
        return Ok(receipt);
    }

    // File-backed temporary tables spill beyond SQLite's bounded page cache.
    connection.execute_batch("PRAGMA temp_store=FILE;")?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS staged_owner_records (
        collection TEXT NOT NULL, record_id TEXT NOT NULL, PRIMARY KEY(collection, record_id)
        ) WITHOUT ROWID; DELETE FROM staged_owner_records;",
    )?;
    prepare_visibility_queue(&transaction)?;
    for batch in &collected {
        let mut rows: BTreeMap<KnowledgeRecordRef, StoredRow> = BTreeMap::new();
        for_each_store_row(&batch.store, |row| {
            let record = KnowledgeRecordRef::new(row.collection, row.record_id.clone());
            rows.insert(record, row);
            Ok(())
        })?;
        let dependencies = validate_dependency_batch(&rows, &batch.dependencies)?;
        for (record, row) in &rows {
            let prior = transaction
                .query_row(
                    "SELECT payload FROM knowledge_records WHERE collection=?1 AND record_id=?2",
                    params![record.collection, record.record_id],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                if !payload_matches(&transaction, prior, &row.payload)? {
                    return Err(collision(record));
                }
                transaction.execute("UPDATE scoped_records SET ownership_managed=1 WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND collection=?1 AND record_id=?2", params![record.collection, record.record_id])?;
            } else {
                insert_owned_row(&transaction, row)?;
                receipt.publication.inserted_records += 1;
            }
            let empty = BTreeSet::new();
            let inputs = dependencies.get(record).unwrap_or(&empty);
            let dep_digest = dependency_set_digest(inputs);
            let prior = transaction.query_row("SELECT dependency_set_digest FROM knowledge_record_dependency_sets WHERE collection=?1 AND record_id=?2",
                params![record.collection, record.record_id], |r| r.get::<_, String>(0)).optional()?;
            if prior.is_some_and(|prior| prior != dep_digest) {
                return Err(collision(record));
            }
            transaction.execute(
                "INSERT OR IGNORE INTO scoped_record_dependency_sets VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1,?2,?3)",
                params![record.collection, record.record_id, dep_digest],
            )?;
            for input in inputs {
                transaction.execute(
                    "INSERT OR IGNORE INTO scoped_record_dependencies VALUES ((SELECT id FROM temp.selected_knowledge_view), ?1,?2,?3,?4)",
                    params![
                        record.collection,
                        record.record_id,
                        input.collection,
                        input.record_id
                    ],
                )?;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO staged_owner_records VALUES (?1,?2)",
                params![record.collection, record.record_id],
            )?;
        }
    }
    // Validate the complete universe after all batches, including forward references.
    let missing = transaction.query_row("SELECT d.input_collection, d.input_record_id
        FROM staged_owner_records s JOIN knowledge_record_dependencies d USING(collection,record_id)
        LEFT JOIN knowledge_records i ON i.collection=d.input_collection AND i.record_id=d.input_record_id
        WHERE i.record_id IS NULL LIMIT 1", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional()?;
    if let Some((collection, record_id)) = missing {
        return Err(KnowledgeRepositoryError::MissingDependency {
            collection,
            record_id,
        });
    }
    transaction.execute("INSERT OR IGNORE INTO visibility_pending SELECT o.collection, o.record_id FROM knowledge_record_owners o
        WHERE o.owner_id=?1 AND NOT EXISTS (SELECT 1 FROM staged_owner_records s WHERE s.collection=o.collection AND s.record_id=o.record_id)", [owner])?;
    transaction.execute("INSERT OR IGNORE INTO visibility_pending SELECT s.collection, s.record_id FROM staged_owner_records s
        WHERE NOT EXISTS (SELECT 1 FROM knowledge_record_owners o WHERE o.owner_id=?1 AND o.collection=s.collection AND o.record_id=s.record_id)", [owner])?;
    receipt.publication.removed_owners = transaction.execute("DELETE FROM scoped_record_owners
        WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND owner_id=?1 AND NOT EXISTS (SELECT 1 FROM staged_owner_records s WHERE s.collection=scoped_record_owners.collection AND s.record_id=scoped_record_owners.record_id)", [owner])?;
    receipt.publication.added_owners = transaction.execute(
        "INSERT OR IGNORE INTO scoped_record_owners
        SELECT (SELECT id FROM temp.selected_knowledge_view), ?1, collection, record_id FROM staged_owner_records",
        [owner],
    )?;
    let visibility = drain_visibility_queue(&transaction, Some(owner))?;
    receipt.publication.evaluated_records = visibility.evaluated_records;
    receipt.publication.became_visible = visibility.became_visible;
    receipt.publication.became_inactive = visibility.became_inactive;
    set_owner_publication_digest(&transaction, owner, &digest)?;
    set_store_digest(&transaction, OWNERSHIP_STORE_MARKER)?;
    transaction.execute_batch("DELETE FROM staged_owner_records;")?;
    transaction.commit()?;
    receipt.outcome = PublicationOutcome::Replaced { digest };
    Ok(receipt)
}

fn collision(record: &KnowledgeRecordRef) -> KnowledgeRepositoryError {
    KnowledgeRepositoryError::ImmutableRecordCollision {
        collection: record.collection.clone(),
        record_id: record.record_id.clone(),
    }
}
