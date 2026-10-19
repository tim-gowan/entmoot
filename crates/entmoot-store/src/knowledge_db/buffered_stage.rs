//! Private publication preparation; no durable state is exposed before validation.
use super::*;
use std::borrow::Cow;

pub(super) struct StagedBatches {
    connection: Connection,
    _directory: tempfile::TempDir,
}

impl StagedBatches {
    pub(super) fn new(repository: &Connection) -> Result<Self, KnowledgeRepositoryError> {
        let parent = repository
            .path()
            .filter(|path| !path.is_empty())
            .and_then(|path| Path::new(path).parent())
            .filter(|path| !path.as_os_str().is_empty());
        let directory = match parent {
            Some(parent) => tempfile::Builder::new()
                .prefix("entmoot-publication-")
                .tempdir_in(parent)?,
            None => tempfile::Builder::new()
                .prefix("entmoot-publication-")
                .tempdir()?,
        };
        let connection = Connection::open(directory.path().join("stage.sqlite3"))?;
        // This is disposable IR. RAII removes it on success/error; existing
        // producer checkpoints, not this database, own restart and worker fences.
        connection.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-2048;
            CREATE TABLE batches(ordinal INTEGER PRIMARY KEY, body BLOB NOT NULL, digest BLOB NOT NULL);
            CREATE TABLE context(collection TEXT NOT NULL, record_id TEXT NOT NULL, body BLOB NOT NULL,
                digest BLOB NOT NULL, PRIMARY KEY(collection,record_id)) WITHOUT ROWID;
            BEGIN;")?;
        Ok(Self {
            connection,
            _directory: directory,
        })
    }

    pub(super) fn push(
        &self,
        store: &KnowledgeStore,
        rows: &BTreeMap<KnowledgeRecordRef, StoredRow>,
    ) -> Result<(), KnowledgeRepositoryError> {
        let bytes = serde_json::to_vec(store)?;
        self.connection.execute(
            "INSERT INTO batches(body,digest) VALUES(?1,?2)",
            params![bytes, blake3::hash(&bytes).as_bytes().as_slice()],
        )?;
        for row in rows.values() {
            if matches!(
                row.collection,
                "artifacts" | "locations" | "components" | "propositions" | "relation_edges"
            ) {
                self.connection.execute(
                    "INSERT OR IGNORE INTO context VALUES(?1,?2,?3,?4)",
                    params![
                        row.collection,
                        row.record_id,
                        row.payload,
                        blake3::hash(&row.payload).as_bytes().as_slice()
                    ],
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<(), KnowledgeRepositoryError> {
        self.connection.execute_batch("COMMIT;")?;
        Ok(())
    }

    pub(super) fn visit(
        &self,
        mut visit: impl FnMut(KnowledgeStore) -> Result<(), KnowledgeRepositoryError>,
    ) -> Result<(), KnowledgeRepositoryError> {
        let mut statement = self
            .connection
            .prepare("SELECT body,digest FROM batches ORDER BY ordinal")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            let digest: Vec<u8> = row.get(1)?;
            verify(&bytes, &digest)?;
            visit(serde_json::from_slice(&bytes)?)?;
        }
        Ok(())
    }

    fn fetch<T: DeserializeOwned>(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<T>, KnowledgeRepositoryError> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .connection
            .prepare_cached("SELECT body,digest FROM context WHERE collection=?1 AND record_id=?2")?
            .query_row(params![collection, id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        row.map(|(bytes, digest)| {
            verify(&bytes, &digest)?;
            Ok(serde_json::from_slice(&bytes)?)
        })
        .transpose()
    }
}

fn verify(bytes: &[u8], digest: &[u8]) -> Result<(), KnowledgeRepositoryError> {
    if blake3::hash(bytes).as_bytes().as_slice() != digest {
        return Err(invalid_record(
            "publication_stage",
            "payload commitment mismatch",
        ));
    }
    Ok(())
}

impl StoreRecordContext for StagedBatches {
    fn artifact(
        &self,
        id: &str,
    ) -> Result<Option<Cow<'_, ArtifactManifest>>, KnowledgeRepositoryError> {
        Ok(self.fetch("artifacts", id)?.map(Cow::Owned))
    }
    fn location(
        &self,
        id: &str,
    ) -> Result<Option<Cow<'_, LocationRecord>>, KnowledgeRepositoryError> {
        Ok(self.fetch("locations", id)?.map(Cow::Owned))
    }
    fn component_exists(&self, id: &str) -> Result<bool, KnowledgeRepositoryError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM context WHERE collection='components' AND record_id=?1)",
            [id],
            |row| row.get(0),
        )?)
    }
    fn proposition(
        &self,
        id: &str,
    ) -> Result<Option<Cow<'_, Proposition>>, KnowledgeRepositoryError> {
        Ok(self.fetch("propositions", id)?.map(Cow::Owned))
    }
    fn relation_edge(
        &self,
        id: &str,
    ) -> Result<Option<Cow<'_, RelationEdge>>, KnowledgeRepositoryError> {
        Ok(self.fetch("relation_edges", id)?.map(Cow::Owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_checks_commitments_and_removes_private_files() {
        let repository = Connection::open_in_memory().unwrap();
        let staged = StagedBatches::new(&repository).unwrap();
        let directory = staged._directory.path().to_owned();
        staged
            .push(&KnowledgeStore::default(), &BTreeMap::new())
            .unwrap();
        staged.finish().unwrap();
        let mut count = 0;
        staged
            .visit(|_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 1);
        staged
            .connection
            .execute("UPDATE batches SET body=?1", [b"{}".as_slice()])
            .unwrap();
        let error = staged
            .visit(|_| panic!("corrupt IR must not reach validation"))
            .unwrap_err();
        assert!(matches!(
            error,
            KnowledgeRepositoryError::InvalidRecord {
                collection: "publication_stage",
                ..
            }
        ));
        drop(staged);
        assert!(!directory.exists());
    }
}
