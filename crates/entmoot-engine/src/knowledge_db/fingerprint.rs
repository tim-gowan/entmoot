//! Storage compatibility is checked before persistent mutation or hydration.
use super::*;
use entmoot_contracts::{ContentDigest, StorageFingerprint, StorageSeries};

pub(super) const SCHEMA: &str = "CREATE TABLE storage_fingerprint (singleton INTEGER PRIMARY KEY CHECK(singleton=1), body TEXT NOT NULL);";

/// Current reader/writer receipt, computed without opening or creating a store.
pub fn current() -> StorageFingerprint {
    let mut hash = blake3::Hasher::new();
    hash.update(b"entmoot.storage_layout.v1\0");
    let operational_schema = operations::schema();
    for sql in [
        SCHEMA,
        layout::SCHEMA,
        inventory::SCHEMA,
        syntax_cache::SCHEMA,
        operational_schema.as_str(),
    ] {
        let normalized = sql.replace("\r\n", "\n");
        hash.update(&(normalized.len() as u64).to_le_bytes());
        hash.update(normalized.as_bytes());
    }
    StorageFingerprint {
        series: StorageSeries::V1,
        implementation: crate::implementation_fingerprint(),
        layout: ContentDigest::from_trusted(format!("blake3:{}", hash.finalize().to_hex())),
    }
}

pub(super) fn initialize(connection: &Connection) -> Result<(), KnowledgeRepositoryError> {
    connection.execute_batch(SCHEMA)?;
    connection.execute(
        "INSERT INTO storage_fingerprint VALUES (1, ?1)",
        [serde_json::to_string(&current())?],
    )?;
    Ok(())
}

pub(super) fn verify(connection: &Connection) -> Result<(), KnowledgeRepositoryError> {
    let expected = current();
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='storage_fingerprint')",
        [], |row| row.get(0),
    )?;
    let body: Option<String> = if exists {
        connection
            .query_row(
                "SELECT body FROM storage_fingerprint WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?
    } else {
        None
    };
    let actual = body
        .as_deref()
        .map(serde_json::from_str::<StorageFingerprint>)
        .transpose()
        .map_err(|error| invalid_record("storage_fingerprint", error))?;
    let Some(actual) = actual else {
        return Err(KnowledgeRepositoryError::IncompatibleFingerprint {
            component: "envelope",
            actual: None,
            expected: serde_json::to_string(&expected)?,
        });
    };
    for (component, actual, expected) in [
        (
            "contracts",
            actual.implementation.contracts.as_str(),
            expected.implementation.contracts.as_str(),
        ),
        (
            "derivation",
            actual.implementation.derivation.as_str(),
            expected.implementation.derivation.as_str(),
        ),
        ("layout", actual.layout.as_str(), expected.layout.as_str()),
    ] {
        if actual != expected {
            return Err(KnowledgeRepositoryError::IncompatibleFingerprint {
                component,
                actual: Some(actual.into()),
                expected: expected.into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fingerprint_dimensions_refuse_reads_writes_and_maintenance_without_reset() {
        let directory = tempfile::tempdir().unwrap();
        for component in ["contracts", "derivation", "layout", "series", "missing"] {
            let path = directory.path().join(format!("{component}.sqlite3"));
            let mut repository = SqliteKnowledgeRepository::open(&path).unwrap();
            repository.replace_all(&KnowledgeStore::default()).unwrap();
            let mut actual = current();
            let changed = ContentDigest::from_trusted(format!("blake3:{}", "a".repeat(64)));
            match component {
                "contracts" => actual.implementation.contracts = changed,
                "derivation" => actual.implementation.derivation = changed,
                "layout" => actual.layout = changed,
                "series" => (),
                "missing" => (),
                _ => unreachable!(),
            }
            if component == "missing" {
                repository
                    .connection
                    .execute_batch("DROP TABLE storage_fingerprint")
                    .unwrap();
            } else {
                let mut value = serde_json::to_value(&actual).unwrap();
                if component == "series" {
                    value["series"] = "entmoot.storage.obsolete".into();
                }
                repository
                    .connection
                    .execute(
                        "UPDATE storage_fingerprint SET body=?1",
                        [serde_json::to_string(&value).unwrap()],
                    )
                    .unwrap();
            }
            drop(repository);
            let before = std::fs::read(&path).unwrap();
            for error in [
                SqliteKnowledgeRepository::open_read_only(&path)
                    .err()
                    .unwrap(),
                SqliteKnowledgeRepository::open(&path).err().unwrap(),
                retention::Maintenance::open(&path).err().unwrap(),
            ] {
                if component == "series" {
                    assert!(matches!(
                        error,
                        KnowledgeRepositoryError::InvalidRecord {
                            collection: "storage_fingerprint",
                            ..
                        }
                    ));
                } else {
                    assert!(
                        matches!(error, KnowledgeRepositoryError::IncompatibleFingerprint { component: actual, .. } if actual == if component == "missing" { "envelope" } else { component })
                    );
                }
                assert_eq!(std::fs::read(&path).unwrap(), before);
            }
        }
    }

    #[test]
    fn compatible_store_reopens_and_missing_row_does_not_initialize_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite3");
        let mut repository = SqliteKnowledgeRepository::open(&path).unwrap();
        repository.replace_all(&KnowledgeStore::default()).unwrap();
        let value = serde_json::to_value(current()).unwrap();
        assert_eq!(
            serde_json::from_value::<StorageFingerprint>(value).unwrap(),
            current()
        );
        drop(repository);
        drop(SqliteKnowledgeRepository::open_read_only(&path).unwrap());
        let repository = SqliteKnowledgeRepository::open(&path).unwrap();
        repository
            .connection
            .execute_batch("DELETE FROM storage_fingerprint")
            .unwrap();
        drop(repository);
        assert!(matches!(
            SqliteKnowledgeRepository::open(&path),
            Err(KnowledgeRepositoryError::IncompatibleFingerprint {
                component: "envelope",
                actual: None,
                ..
            })
        ));
    }
}
