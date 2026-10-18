//! Physical storage is private: immutable payloads and metadata are shared;
//! headers, owners, dependencies, and visibility belong to one selected view.
use super::*;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS shared_metadata (
    metadata_id INTEGER PRIMARY KEY, digest BLOB NOT NULL UNIQUE, body BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS shared_payloads (
    payload_id INTEGER PRIMARY KEY, digest BLOB NOT NULL UNIQUE, body BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS scoped_records (
    view_id TEXT NOT NULL, collection TEXT NOT NULL, record_id TEXT NOT NULL,
    artifact_id TEXT, subject_id TEXT, object_id TEXT, proposition_id TEXT,
    predicate TEXT, location_id TEXT, payload INTEGER NOT NULL REFERENCES shared_payloads,
    visible INTEGER NOT NULL DEFAULT 1 CHECK(visible IN(0,1)),
    ownership_managed INTEGER NOT NULL DEFAULT 0 CHECK(ownership_managed IN(0,1)),
    PRIMARY KEY(view_id,collection,record_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS knowledge_by_artifact ON scoped_records(view_id,artifact_id,collection);
CREATE INDEX IF NOT EXISTS knowledge_by_subject_predicate ON scoped_records(view_id,collection,subject_id,predicate);
CREATE INDEX IF NOT EXISTS knowledge_by_object_predicate ON scoped_records(view_id,collection,object_id,predicate);
CREATE INDEX IF NOT EXISTS knowledge_by_location ON scoped_records(view_id,collection,location_id);
CREATE INDEX IF NOT EXISTS knowledge_by_proposition ON scoped_records(view_id,collection,proposition_id);
CREATE TABLE IF NOT EXISTS scoped_metadata (
    view_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY(view_id,key)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS scoped_record_owners (
    view_id TEXT NOT NULL, owner_id TEXT NOT NULL, collection TEXT NOT NULL, record_id TEXT NOT NULL,
    PRIMARY KEY(view_id,owner_id,collection,record_id),
    FOREIGN KEY(view_id,collection,record_id) REFERENCES scoped_records ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS knowledge_owners_by_record ON scoped_record_owners(view_id,collection,record_id,owner_id);
CREATE TABLE IF NOT EXISTS scoped_record_dependencies (
    view_id TEXT NOT NULL, collection TEXT NOT NULL, record_id TEXT NOT NULL,
    input_collection TEXT NOT NULL, input_record_id TEXT NOT NULL,
    PRIMARY KEY(view_id,collection,record_id,input_collection,input_record_id),
    FOREIGN KEY(view_id,collection,record_id) REFERENCES scoped_records ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS knowledge_dependencies_by_input ON scoped_record_dependencies(view_id,input_collection,input_record_id,collection,record_id);
CREATE TABLE IF NOT EXISTS scoped_record_dependency_sets (
    view_id TEXT NOT NULL, collection TEXT NOT NULL, record_id TEXT NOT NULL, dependency_set_digest TEXT NOT NULL,
    PRIMARY KEY(view_id,collection,record_id),
    FOREIGN KEY(view_id,collection,record_id) REFERENCES scoped_records ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS scoped_visibility_events (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT, view_id TEXT NOT NULL,
    collection TEXT NOT NULL, record_id TEXT NOT NULL, visible INTEGER NOT NULL CHECK(visible IN(0,1)),
    reason TEXT NOT NULL, cause_ids BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS knowledge_events_by_view ON scoped_visibility_events(view_id,cursor);
PRAGMA user_version=1;
"#;

// TEMP objects belong exclusively to this connection and never change the
// shared database. Keeping a fixed view for the connection makes accidental
// unscoped reads impossible, including joins and recursive dependency queries.
pub(super) fn select_view(
    connection: &Connection,
    view: &str,
) -> Result<(), KnowledgeRepositoryError> {
    if view != "default" {
        entmoot_contracts::ContentDigest::parse(view.to_owned())
            .map_err(|e| invalid_record("view", e))?;
    }
    connection.execute_batch("CREATE TEMP TABLE selected_knowledge_view(id TEXT NOT NULL);")?;
    connection.execute("INSERT INTO selected_knowledge_view VALUES (?1)", [view])?;
    for (name, columns) in [
        ("records", "collection,record_id,artifact_id,subject_id,object_id,proposition_id,predicate,location_id,payload,visible,ownership_managed"),
        ("metadata", "key,value"),
        ("record_owners", "owner_id,collection,record_id"),
        ("record_dependencies", "collection,record_id,input_collection,input_record_id"),
        ("record_dependency_sets", "collection,record_id,dependency_set_digest"),
        ("visibility_events", "cursor,collection,record_id,visible,reason,cause_ids"),
    ] {
        connection.execute_batch(&format!("CREATE TEMP VIEW knowledge_{name} AS SELECT {columns} FROM main.scoped_{name} WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)"))?;
    }
    for (suffix, index) in [
        ("subject", "knowledge_by_subject_predicate"),
        ("object", "knowledge_by_object_predicate"),
        ("proposition", "knowledge_by_proposition"),
        ("location", "knowledge_by_location"),
    ] {
        connection.execute_batch(&format!("CREATE TEMP VIEW knowledge_records_by_{suffix} AS SELECT * FROM main.scoped_records INDEXED BY {index} WHERE view_id=(SELECT id FROM temp.selected_knowledge_view)"))?;
    }
    Ok(())
}

#[derive(Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    body: serde_json::Value,
    metadata: BTreeMap<String, i64>,
}

pub(super) fn intern_payload(
    connection: &Connection,
    bytes: &[u8],
) -> Result<i64, KnowledgeRepositoryError> {
    let mut body: serde_json::Value = serde_json::from_slice(bytes)?;
    let canonical = serde_json::to_vec(&body)?;
    let digest = blake3::hash(&canonical);
    if let Some(id) = connection
        .query_row(
            "SELECT payload_id FROM shared_payloads WHERE digest=?1",
            [digest.as_bytes().as_slice()],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let mut metadata = BTreeMap::new();
    if let Some(object) = body.as_object_mut() {
        // Only storage representation is shared. In particular a shared scope
        // object never turns an occurrence claim into a definition claim.
        let keys: Vec<_> = object
            .iter()
            .filter(|(key, value)| {
                matches!(
                    key.as_str(),
                    "snapshot"
                        | "scope"
                        | "producer"
                        | "subject"
                        | "object"
                        | "subject_address"
                        | "input_manifest"
                        | "build_context"
                        | "policy"
                        | "evidence_refs"
                        | "call_site"
                        | "declaration"
                        | "marker"
                ) && (value.is_object() || value.is_array())
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            let Some(value) = object.remove(&key) else {
                continue;
            };
            let bytes = serde_json::to_vec(&value)?;
            if bytes.len() < 64 {
                object.insert(key, value);
                continue;
            }
            let digest = blake3::hash(&bytes);
            connection.execute(
                "INSERT OR IGNORE INTO shared_metadata(digest,body) VALUES(?1,?2)",
                params![digest.as_bytes().as_slice(), bytes],
            )?;
            let id = connection.query_row(
                "SELECT metadata_id FROM shared_metadata WHERE digest=?1",
                [digest.as_bytes().as_slice()],
                |r| r.get(0),
            )?;
            metadata.insert(key, id);
        }
    }
    connection.execute(
        "INSERT INTO shared_payloads(digest,body) VALUES(?1,?2)",
        params![
            digest.as_bytes().as_slice(),
            serde_json::to_vec(&Payload { body, metadata })?
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

pub(super) fn hydrate_payload(
    connection: &Connection,
    id: i64,
) -> Result<Vec<u8>, KnowledgeRepositoryError> {
    Ok(hydrate_verified_payload(connection, id)?.1)
}

/// Decode the already reconstructed and digest-verified JSON tree directly.
/// Byte consumers still receive the same canonical representation; typed reads
/// avoid reparsing that representation without bypassing its commitment check.
pub(super) fn hydrate_record<T: DeserializeOwned>(
    connection: &Connection,
    id: i64,
) -> Result<T, KnowledgeRepositoryError> {
    let (value, _) = hydrate_verified_payload(connection, id)?;
    Ok(serde_json::from_value(value)?)
}

fn hydrate_verified_payload(
    connection: &Connection,
    id: i64,
) -> Result<(serde_json::Value, Vec<u8>), KnowledgeRepositoryError> {
    let (digest, bytes): (Vec<u8>, Vec<u8>) = connection
        .prepare_cached("SELECT digest,body FROM shared_payloads WHERE payload_id=?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut payload: Payload = serde_json::from_slice(&bytes)?;
    for (key, id) in payload.metadata {
        let bytes: Vec<u8> = connection
            .prepare_cached("SELECT body FROM shared_metadata WHERE metadata_id=?1")?
            .query_row([id], |r| r.get(0))?;
        let object = payload
            .body
            .as_object_mut()
            .ok_or_else(|| invalid_record("payload", "metadata on non-object"))?;
        if object
            .insert(key, serde_json::from_slice(&bytes)?)
            .is_some()
        {
            return Err(invalid_record("payload", "duplicate metadata field"));
        }
    }
    let bytes = serde_json::to_vec(&payload.body)?;
    if blake3::hash(&bytes).as_bytes().as_slice() != digest {
        return Err(invalid_record("payload", "content digest mismatch"));
    }
    Ok((payload.body, bytes))
}

pub(super) fn payload_matches(
    connection: &Connection,
    id: i64,
    expected: &[u8],
) -> Result<bool, KnowledgeRepositoryError> {
    let expected = serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(expected)?)?;
    Ok(hydrate_payload(connection, id)? == expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_payload_comparison_preserves_signed_zero() {
        let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let id = intern_payload(&repository.connection, br#"{"value":-0.0}"#).unwrap();
        assert!(payload_matches(&repository.connection, id, br#"{"value":-0.0}"#).unwrap());
        assert!(!payload_matches(&repository.connection, id, br#"{"value":0.0}"#).unwrap());
        let typed: BTreeMap<String, f64> = hydrate_record(&repository.connection, id).unwrap();
        assert_eq!(typed["value"].to_bits(), (-0.0_f64).to_bits());
    }

    #[derive(Debug, PartialEq, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Record {
        scope: serde_json::Value,
        marker: serde_json::Value,
        values: Vec<serde_json::Value>,
        upper: u64,
        lower: i64,
    }

    #[test]
    fn typed_hydration_preserves_shared_metadata_and_extensible_detail() {
        let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "scope": {"repository": "fixture", "snapshot": "a".repeat(64), "configuration": "b".repeat(64)},
            "marker": {"category": "indirect_call", "detail": {"future_provider_detail": ["日本語", "λ", null], "unknown": "preserve me"}},
            "values": [null, true, false, [], {}, "line\nquote\"slash\\", -0.0, 1.25],
            "upper": u64::MAX,
            "lower": i64::MIN,
        })).unwrap();
        let id = intern_payload(&repository.connection, &bytes).unwrap();
        let shared: i64 = repository
            .connection
            .query_row("SELECT COUNT(*) FROM shared_metadata", [], |r| r.get(0))
            .unwrap();
        assert_eq!(shared, 2, "fixture must exercise shared scope and marker");
        let typed: Record = hydrate_record(&repository.connection, id).unwrap();
        let old: Record =
            serde_json::from_slice(&hydrate_payload(&repository.connection, id).unwrap()).unwrap();
        assert_eq!(typed, old);
        assert_eq!(typed.upper, u64::MAX);
        assert_eq!(typed.lower, i64::MIN);
        assert_eq!(
            typed.marker["detail"]["future_provider_detail"][0],
            "日本語"
        );
        assert_eq!(
            typed.values[6].as_f64().unwrap().to_bits(),
            (-0.0_f64).to_bits()
        );
    }

    fn assert_rejected(connection: &Connection, id: i64, message: &str) {
        assert!(hydrate_payload(connection, id)
            .unwrap_err()
            .to_string()
            .contains(message));
        assert!(hydrate_record::<serde_json::Value>(connection, id)
            .unwrap_err()
            .to_string()
            .contains(message));
    }

    #[test]
    fn typed_and_byte_hydration_recheck_body_and_shared_metadata_commitments() {
        for metadata in [false, true] {
            let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
            let connection = &repository.connection;
            let bytes = serde_json::to_vec(&serde_json::json!({
                "value": "before", "scope": {"snapshot": "a".repeat(64)},
            }))
            .unwrap();
            let id = intern_payload(connection, &bytes).unwrap();
            let _: serde_json::Value = hydrate_record(connection, id).unwrap();
            let wrapper: Vec<u8> = connection
                .query_row(
                    "SELECT body FROM shared_payloads WHERE payload_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut payload: Payload = serde_json::from_slice(&wrapper).unwrap();
            if metadata {
                connection
                    .execute(
                        "UPDATE shared_metadata SET body=?1 WHERE metadata_id=?2",
                        params![
                            br#"{"snapshot":"tampered"}"#.as_slice(),
                            payload.metadata["scope"]
                        ],
                    )
                    .unwrap();
            } else {
                payload.body["value"] = "tampered".into();
                connection
                    .execute(
                        "UPDATE shared_payloads SET body=?1 WHERE payload_id=?2",
                        params![serde_json::to_vec(&payload).unwrap(), id],
                    )
                    .unwrap();
            }
            assert_rejected(connection, id, "content digest mismatch");
        }
    }

    #[test]
    fn typed_and_byte_hydration_reject_ambiguous_or_missing_metadata() {
        for kind in ["duplicate", "non-object", "missing"] {
            let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
            let connection = &repository.connection;
            let bytes =
                serde_json::to_vec(&serde_json::json!({"scope": {"snapshot": "a".repeat(64)}}))
                    .unwrap();
            let id = intern_payload(connection, &bytes).unwrap();
            let wrapper: Vec<u8> = connection
                .query_row(
                    "SELECT body FROM shared_payloads WHERE payload_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut payload: Payload = serde_json::from_slice(&wrapper).unwrap();
            let expected = match kind {
                "duplicate" => {
                    payload.body["scope"] = serde_json::json!({});
                    "duplicate metadata field"
                }
                "non-object" => {
                    payload.body = serde_json::json!([]);
                    "metadata on non-object"
                }
                _ => {
                    payload.metadata.insert("scope".into(), i64::MAX);
                    "Query returned no rows"
                }
            };
            connection
                .execute(
                    "UPDATE shared_payloads SET body=?1 WHERE payload_id=?2",
                    params![serde_json::to_vec(&payload).unwrap(), id],
                )
                .unwrap();
            assert_rejected(connection, id, expected);
        }
    }

    #[test]
    fn digest_valid_payload_still_requires_typed_contract() {
        let repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        for bytes in [
            br#"{"scope":null,"marker":null,"values":[],"upper":0,"lower":0,"future":true}"#
                .as_slice(),
            br#"{"scope":null,"marker":null,"values":[],"upper":-1,"lower":0}"#,
            br#"{"scope":null,"marker":null,"values":[],"upper":0}"#,
        ] {
            let id = intern_payload(&repository.connection, bytes).unwrap();
            assert!(hydrate_payload(&repository.connection, id).is_ok());
            assert!(hydrate_record::<Record>(&repository.connection, id).is_err());
        }
    }
}
