//! Immutable file units. Structural rows have no per-fact ownership or
//! visibility machinery: selecting a sealed file selects all of its facts.
use super::*;
use entmoot_contracts::{CallableFact, FileFacts, FileMarkers, OccurrenceRef};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS file_fact_units (
    unit_id INTEGER PRIMARY KEY, context BLOB NOT NULL, path TEXT NOT NULL,
    source_digest BLOB NOT NULL, policy_id TEXT NOT NULL,
    file_hash BLOB NOT NULL, module_hash BLOB NOT NULL, markers_present INTEGER NOT NULL,
    seal BLOB NOT NULL, content_digest BLOB NOT NULL, UNIQUE(context,path,source_digest)
);
CREATE TABLE IF NOT EXISTS file_fact_callables (
    unit_id INTEGER NOT NULL REFERENCES file_fact_units ON DELETE CASCADE,
    ordinal INTEGER NOT NULL, address BLOB NOT NULL, logic_address TEXT NOT NULL,
    structural_hash BLOB NOT NULL, exact_body BLOB, signature_hash BLOB NOT NULL,
    signature TEXT NOT NULL, PRIMARY KEY(unit_id,ordinal), UNIQUE(unit_id,address)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS file_facts_by_address ON file_fact_callables(address,unit_id);
CREATE TABLE IF NOT EXISTS file_fact_sites (
    unit_id INTEGER NOT NULL REFERENCES file_fact_units ON DELETE CASCADE,
    kind INTEGER NOT NULL, ordinal INTEGER NOT NULL, metadata INTEGER NOT NULL REFERENCES shared_metadata,
    PRIMARY KEY(unit_id,kind,ordinal)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS file_fact_manifests (
    view_id TEXT PRIMARY KEY, context TEXT NOT NULL, digest TEXT NOT NULL, scan_commitment TEXT
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS file_fact_membership (
    view_id TEXT NOT NULL REFERENCES file_fact_manifests ON DELETE CASCADE,
    path TEXT NOT NULL, unit_id INTEGER NOT NULL REFERENCES file_fact_units,
    parts INTEGER REFERENCES shared_payloads, PRIMARY KEY(view_id,path)
) WITHOUT ROWID;
"#;

pub(super) fn binary_digest(value: &str) -> Result<[u8; 32], KnowledgeRepositoryError> {
    blake3::Hash::from_hex(value.strip_prefix("blake3:").unwrap_or(value))
        .map(|hash| *hash.as_bytes())
        .map_err(|error| invalid_record("file_facts", error))
}

pub(super) fn digest_text(value: Vec<u8>, prefix: bool) -> Result<String, rusqlite::Error> {
    let array: [u8; 32] = value.try_into().map_err(|bytes: Vec<u8>| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("expected 32 digest bytes, found {}", bytes.len()).into(),
        )
    })?;
    let hash = blake3::Hash::from_bytes(array);
    Ok(if prefix {
        format!("blake3:{hash}")
    } else {
        hash.to_hex().to_string()
    })
}

pub(super) fn content_digest(facts: &FileFacts) -> Result<blake3::Hash, KnowledgeRepositoryError> {
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut hasher, facts)?;
    Ok(hasher.finalize())
}

fn seal(
    context: &str,
    path: &str,
    source: &str,
    facts: &FileFacts,
) -> Result<[u8; 32], KnowledgeRepositoryError> {
    let mut hash = blake3::Hasher::new();
    serde_json::to_writer(
        &mut hash,
        &("entmoot.file_facts.v1", context, path, source, facts),
    )?;
    Ok(*hash.finalize().as_bytes())
}

/// Store one immutable unit inside the caller's publication transaction. Hashes
/// are binary and callable boundaries share one occurrence row. Marker families
/// are separate rows, so neither a repository graph nor a giant JSON document is
/// needed to retain or retrieve the file. A conflicting retry fails atomically.
pub(super) fn retain(
    connection: &Connection,
    context: &str,
    path: &str,
    source: &str,
    facts: &FileFacts,
) -> Result<i64, KnowledgeRepositoryError> {
    let context_key = binary_digest(context)?;
    let source_key = binary_digest(source)?;
    let commitment = seal(context, path, source, facts)?;
    let prior: Option<(i64, Vec<u8>)> = connection.query_row(
        "SELECT unit_id,seal FROM file_fact_units WHERE context=?1 AND path=?2 AND source_digest=?3",
        params![context_key, path, source_key], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional()?;
    if let Some((id, prior)) = prior {
        if prior != commitment {
            return Err(invalid_record(
                "file_facts",
                "conflicting facts for sealed file context",
            ));
        }
        // A reusable unit must still pass its payload commitment on hydration.
        return Ok(id);
    }
    let content_digest = content_digest(facts)?;
    connection.prepare_cached("INSERT INTO file_fact_units(context,path,source_digest,policy_id,file_hash,module_hash,markers_present,seal,content_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")?.execute(
        params![context_key, path, source_key, facts.policy_id.as_str(), binary_digest(&facts.file_hash)?, binary_digest(&facts.module_shell_hash)?, facts.markers.is_some(), commitment, content_digest.as_bytes()],
    )?;
    let id = connection.last_insert_rowid();
    let mut callable = connection
        .prepare_cached("INSERT INTO file_fact_callables VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
    for (ordinal, fact) in facts.callables.iter().enumerate() {
        if fact.syntactic_signature.len() > 8 * 1024 * 1024 {
            return Err(KnowledgeRepositoryError::BufferLimit(
                "callable signature exceeds 8 MiB",
            ));
        }
        fact.occurrence
            .validate()
            .map_err(|e| invalid_record("file_facts", e))?;
        let ordinal = i64::try_from(ordinal).map_err(|e| invalid_record("file_facts", e))?;
        callable.execute(params![
            id,
            ordinal,
            binary_digest(fact.occurrence.address_id.as_str())?,
            fact.occurrence.logic_address,
            binary_digest(&fact.structural_hash)?,
            fact.exact_body_id
                .as_deref()
                .map(binary_digest)
                .transpose()?,
            binary_digest(&fact.signature_hash)?,
            fact.syntactic_signature
        ])?;
    }
    if let Some(markers) = &facts.markers {
        let mut statement =
            connection.prepare_cached("INSERT INTO file_fact_sites VALUES(?1,?2,?3,?4)")?;
        retain_sites(connection, &mut statement, id, 0, &markers.definitions)?;
        retain_sites(connection, &mut statement, id, 1, &markers.imports)?;
        retain_sites(connection, &mut statement, id, 2, &markers.declarations)?;
        retain_sites(connection, &mut statement, id, 3, &markers.bindings)?;
        retain_sites(connection, &mut statement, id, 4, &markers.control_sites)?;
        retain_sites(connection, &mut statement, id, 5, &markers.call_sites)?;
        retain_sites(connection, &mut statement, id, 6, &markers.code_contracts)?;
    }
    Ok(id)
}

fn retain_sites<T: Serialize>(
    connection: &Connection,
    statement: &mut rusqlite::CachedStatement<'_>,
    unit: i64,
    kind: u8,
    values: &[T],
) -> Result<(), KnowledgeRepositoryError> {
    for (ordinal, value) in values.iter().enumerate() {
        let bytes = serde_json::to_vec(&serde_json::to_value(value)?)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(KnowledgeRepositoryError::BufferLimit(
                "syntax record exceeds 8 MiB",
            ));
        }
        let ordinal = i64::try_from(ordinal).map_err(|e| invalid_record("file_facts", e))?;
        statement.execute(params![
            unit,
            kind,
            ordinal,
            super::layout::intern_metadata_bytes(connection, &bytes)?
        ])?;
    }
    Ok(())
}

pub(super) fn load(
    connection: &Connection,
    id: i64,
) -> Result<FileFacts, KnowledgeRepositoryError> {
    let (context, path, source, mut facts, commitment) = connection.query_row(
        "SELECT context,path,source_digest,policy_id,file_hash,module_hash,markers_present,seal FROM file_fact_units WHERE unit_id=?1",
        [id], |row| Ok((digest_text(row.get(0)?, true)?, row.get::<_, String>(1)?,
            digest_text(row.get(2)?, true)?, FileFacts {
                policy_id: entmoot_contracts::PolicyId::from_trusted(row.get::<_, String>(3)?),
                file_hash: digest_text(row.get(4)?, true)?,
                module_shell_hash: digest_text(row.get(5)?, true)?,
                callables: Vec::new(),
                markers: row.get::<_, bool>(6)?.then(FileMarkers::default),
            }, row.get::<_, Vec<u8>>(7)?)),
    )?;
    let mut statement = connection.prepare_cached("SELECT address,logic_address,structural_hash,exact_body,signature_hash,signature FROM file_fact_callables WHERE unit_id=?1 ORDER BY ordinal")?;
    facts.callables = statement
        .query_map([id], |row| {
            Ok(CallableFact {
                occurrence: OccurrenceRef {
                    address_id: entmoot_contracts::AddressId::from_trusted(digest_text(
                        row.get(0)?,
                        false,
                    )?),
                    logic_address: row.get(1)?,
                },
                structural_hash: digest_text(row.get(2)?, true)?,
                exact_body_id: row
                    .get::<_, Option<Vec<u8>>>(3)?
                    .map(|v| digest_text(v, true))
                    .transpose()?,
                signature_hash: digest_text(row.get(4)?, true)?,
                syntactic_signature: row.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut statement = connection.prepare_cached(
        "SELECT kind,metadata FROM file_fact_sites WHERE unit_id=?1 ORDER BY kind,ordinal",
    )?;
    let mut rows = statement.query([id])?;
    while let Some(row) = rows.next()? {
        let markers = facts
            .markers
            .as_mut()
            .ok_or_else(|| invalid_record("file_facts", "sites without marker coverage"))?;
        let bytes = super::layout::hydrate_metadata_bytes(connection, row.get(1)?)?;
        match row.get::<_, u8>(0)? {
            0 => markers.definitions.push(serde_json::from_slice(&bytes)?),
            1 => markers.imports.push(serde_json::from_slice(&bytes)?),
            2 => markers.declarations.push(serde_json::from_slice(&bytes)?),
            3 => markers.bindings.push(serde_json::from_slice(&bytes)?),
            4 => markers.control_sites.push(serde_json::from_slice(&bytes)?),
            5 => markers.call_sites.push(serde_json::from_slice(&bytes)?),
            6 => markers.code_contracts.push(serde_json::from_slice(&bytes)?),
            _ => return Err(invalid_record("file_facts", "unknown syntax site family")),
        }
    }
    if seal(&context, &path, &source, &facts)?.as_slice() != commitment {
        return Err(invalid_record(
            "file_facts",
            "sealed file commitment mismatch",
        ));
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_units_reuse_without_conflating_changed_or_moved_occurrences() {
        let mut db = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let context = entmoot_contracts::blake3_hex(b"fixture-context");
        let source = "def leaf(x):\n    return x\n";
        let digest = entmoot_contracts::blake3_hex(source.as_bytes());
        let parse = |path| {
            entmoot_core::index::build_file_facts(
                entmoot_core::StructuralHasher::new()
                    .compute_granular_hash_with_analysis_at(
                        source,
                        "python",
                        Some(path),
                        Some("fixture"),
                        None,
                    )
                    .unwrap(),
            )
        };
        let first = parse("a.py");
        let transaction = db.connection.transaction().unwrap();
        let id = retain(&transaction, &context, "a.py", &digest, &first).unwrap();
        assert_eq!(
            retain(&transaction, &context, "a.py", &digest, &first).unwrap(),
            id
        );
        let moved = parse("b.py");
        let second = retain(&transaction, &context, "b.py", &digest, &moved).unwrap();
        assert_ne!(id, second);
        assert_eq!(
            first.callables[0].structural_hash,
            moved.callables[0].structural_hash
        );
        assert_ne!(first.callables[0].occurrence, moved.callables[0].occurrence);
        assert!(retain(&transaction, &context, "a.py", &digest, &moved).is_err());
        transaction.commit().unwrap();
        let contract = &first.markers.as_ref().unwrap().code_contracts[0];
        let node_payload = super::layout::intern_payload(
            &db.connection,
            &serde_json::to_vec(&serde_json::json!({"code_contract": contract})).unwrap(),
        )
        .unwrap();
        let scoped_payload = super::layout::intern_payload(
            &db.connection,
            &serde_json::to_vec(
                &serde_json::json!({"contract": contract, "scope": {"domain": "fixture"}}),
            )
            .unwrap(),
        )
        .unwrap();
        let fact_metadata: i64 = db
            .connection
            .query_row(
                "SELECT metadata FROM file_fact_sites WHERE unit_id=?1 AND kind=6 AND ordinal=0",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        for (payload, key) in [
            (node_payload, "code_contract"),
            (scoped_payload, "contract"),
        ] {
            let shared: i64 = db.connection.query_row("SELECT json_extract(CAST(body AS TEXT), '$.metadata.' || ?2) FROM shared_payloads WHERE payload_id=?1", params![payload,key], |row| row.get(0)).unwrap();
            assert_eq!(shared, fact_metadata, "syntax body must have one retained representation across IR, nodes and scoped records");
        }
        assert_eq!(load(&db.connection, id).unwrap(), first);
        assert_eq!(load(&db.connection, second).unwrap(), moved);
        db.connection
            .execute(
                "DELETE FROM file_fact_sites WHERE unit_id=?1 AND kind=0",
                [id],
            )
            .unwrap();
        assert!(matches!(
            load(&db.connection, id),
            Err(KnowledgeRepositoryError::InvalidRecord {
                collection: "file_facts",
                ..
            })
        ));
        assert_eq!(load(&db.connection, second).unwrap(), moved);
    }
}
