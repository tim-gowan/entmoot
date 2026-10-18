//! Disposable, latest-only syntax IR; never a source of binding/claim truth.
use super::*;
use crate::inventory_stitch::{CallPart, SyntaxPart};
use entmoot_contracts::{FileIndexRecord, StructuralHashPolicy};
use entmoot_core::scan::ScanInput;

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS syntax_cache_header(context TEXT PRIMARY KEY, digest TEXT NOT NULL); CREATE TABLE IF NOT EXISTS file_syntax_cache(path TEXT PRIMARY KEY, digest TEXT NOT NULL, payload INTEGER NOT NULL REFERENCES shared_payloads, parts INTEGER REFERENCES shared_payloads);";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FileParts {
    pub syntax: SyntaxPart,
    pub calls: CallPart,
}

/// Structural IR and derived fragments staged on disk before publication.
/// Only file commitments remain resident between jobs. Temporary bytes are
/// removed on success or failure; durable producer checkpoints own restart.
pub struct SyntaxCache {
    pub context: String,
    files: BTreeMap<String, CachedFile>,
    directory: tempfile::TempDir,
    next_payload: u64,
    pub reused: usize,
    pub reused_extraction: usize,
    pub reused_stitching: usize,
    /// Complete qualified reuse permits skipping this disposable cache refresh.
    pub unchanged: bool,
}
struct CachedFile {
    digest: String,
    record: Payload,
    parts: Option<Payload>,
    syntax: Option<Payload>,
}
struct Payload {
    path: std::path::PathBuf,
    digest: String,
}
impl Payload {
    fn read(&self) -> Result<Vec<u8>, KnowledgeRepositoryError> {
        let bytes = std::fs::read(&self.path)?;
        if entmoot_contracts::blake3_hex(&bytes) != self.digest {
            return Err(invalid_record(
                "syntax_cache",
                "staged payload commitment mismatch",
            ));
        }
        Ok(bytes)
    }
    fn hydrate<T: serde::de::DeserializeOwned>(&self) -> Result<T, KnowledgeRepositoryError> {
        Ok(serde_json::from_slice(&self.read()?)?)
    }
}
impl SyntaxCache {
    pub fn new(context: String) -> Result<Self, KnowledgeRepositoryError> {
        Self::new_in(context, None)
    }
    fn new_in(
        context: String,
        parent: Option<&std::path::Path>,
    ) -> Result<Self, KnowledgeRepositoryError> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("entmoot-syntax-ir-");
        let directory = match parent {
            Some(parent) => builder.tempdir_in(parent)?,
            None => builder.tempdir()?,
        };
        Ok(Self {
            context,
            files: BTreeMap::new(),
            directory,
            next_payload: 0,
            reused: 0,
            reused_extraction: 0,
            reused_stitching: 0,
            unchanged: false,
        })
    }
    pub fn context(
        repository: &str,
        policy: &StructuralHashPolicy,
    ) -> Result<String, serde_json::Error> {
        Ok(entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
            "entmoot.file_syntax_cache.v1",
            repository,
            entmoot_core::scan::STRUCTURAL_SCAN_POLICY_ID,
            policy.fingerprint(),
        ))?))
    }
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
    pub fn source_digest(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(|file| file.digest.as_str())
    }
    pub fn record(&self, path: &str) -> Result<Option<FileIndexRecord>, KnowledgeRepositoryError> {
        self.files
            .get(path)
            .map(|file| file.record.hydrate())
            .transpose()
    }
    /// Hydrate only the admitted scan batch.
    pub fn records_for_inputs(
        &self,
        inputs: &[ScanInput],
    ) -> Result<HashMap<String, (String, FileIndexRecord)>, KnowledgeRepositoryError> {
        inputs
            .iter()
            .filter_map(|input| {
                self.files
                    .get(&input.path)
                    .filter(|file| file.digest == input.content_digest)
                    .map(|file| {
                        file.record
                            .hydrate()
                            .map(|record| (input.path.clone(), (file.digest.clone(), record)))
                    })
            })
            .collect()
    }
    pub fn qualified_parts(
        &self,
        path: &str,
        digest: &str,
    ) -> Result<Option<FileParts>, KnowledgeRepositoryError> {
        self.files
            .get(path)
            .filter(|file| file.digest == digest)
            .and_then(|file| file.parts.as_ref())
            .map(Payload::hydrate)
            .transpose()
    }
    pub fn syntax(&self, path: &str) -> Result<SyntaxPart, KnowledgeRepositoryError> {
        let file = self
            .files
            .get(path)
            .ok_or_else(|| invalid_record("syntax_cache", "syntax lacks source IR"))?;
        if let Some(payload) = &file.syntax {
            return payload.hydrate();
        }
        file.parts
            .as_ref()
            .ok_or_else(|| invalid_record("syntax_cache", "file lacks syntax fragment"))?
            .hydrate::<FileParts>()
            .map(|parts| parts.syntax)
    }
    fn payload(&mut self, bytes: &[u8]) -> Result<Payload, KnowledgeRepositoryError> {
        let path = self.directory.path().join(self.next_payload.to_string());
        self.next_payload = self
            .next_payload
            .checked_add(1)
            .ok_or_else(|| invalid_record("syntax_cache", "staged payload counter exhausted"))?;
        use std::io::Write;
        let mut output = std::fs::File::create_new(&path)?;
        output.write_all(bytes)?;
        Ok(Payload {
            path,
            digest: entmoot_contracts::blake3_hex(bytes),
        })
    }
    pub fn insert_record(
        &mut self,
        path: String,
        digest: String,
        record: &FileIndexRecord,
    ) -> Result<(), KnowledgeRepositoryError> {
        if !record.resolved_identities.is_empty() {
            return Err(invalid_record(
                "syntax_cache",
                "binding evidence in syntax cache",
            ));
        }
        let payload = self.payload(&encode(record)?)?;
        self.files.insert(
            path,
            CachedFile {
                digest,
                record: payload,
                parts: None,
                syntax: None,
            },
        );
        Ok(())
    }
    pub fn insert_parts(
        &mut self,
        path: &str,
        parts: &FileParts,
    ) -> Result<(), KnowledgeRepositoryError> {
        if !self.files.contains_key(path) {
            return Err(invalid_record(
                "syntax_cache",
                "graph parts lack their source record",
            ));
        }
        let payload = self.payload(&encode(parts)?)?;
        if let Some(file) = self.files.get_mut(path) {
            file.parts = Some(payload);
        }
        Ok(())
    }
    pub fn insert_syntax(
        &mut self,
        path: &str,
        syntax: &SyntaxPart,
    ) -> Result<(), KnowledgeRepositoryError> {
        if !self.files.contains_key(path) {
            return Err(invalid_record("syntax_cache", "syntax lacks source IR"));
        }
        let payload = self.payload(&encode(syntax)?)?;
        if let Some(file) = self.files.get_mut(path) {
            file.syntax = Some(payload);
        }
        Ok(())
    }
    fn membership(&self) -> Membership {
        self.files
            .iter()
            .map(|(path, file)| {
                (
                    path.clone(),
                    (
                        file.digest.clone(),
                        file.record.digest.clone(),
                        file.parts.as_ref().map(|part| part.digest.clone()),
                    ),
                )
            })
            .collect()
    }
}
// Shared payload storage canonicalizes object ordering. Commit those exact bytes.
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&serde_json::to_value(value)?)
}
type Membership = BTreeMap<String, (String, String, Option<String>)>;
fn membership_digest(context: &str, rows: &Membership) -> Result<String, serde_json::Error> {
    Ok(entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
        "entmoot.file_syntax_membership.v1",
        context,
        rows,
    ))?))
}
impl SqliteKnowledgeRepository {
    pub fn load_syntax_cache(
        &self,
        context: &str,
    ) -> Result<SyntaxCache, KnowledgeRepositoryError> {
        let _snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let parent = self
            .connection
            .path()
            .filter(|path| !path.is_empty())
            .and_then(|path| std::path::Path::new(path).parent());
        let mut cache = SyntaxCache::new_in(context.into(), parent)?;
        let digest: Option<String> = self
            .connection
            .query_row(
                "SELECT digest FROM syntax_cache_header WHERE context=?1",
                [context],
                |r| r.get(0),
            )
            .optional()?;
        let Some(digest) = digest else {
            return Ok(cache);
        };
        let mut stmt = self
            .connection
            .prepare("SELECT path,digest,payload,parts FROM file_syntax_cache")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let bytes = hydrate_payload(&self.connection, row.get(2)?)?;
            let record: FileIndexRecord = serde_json::from_slice(&bytes)?;
            if !record.resolved_identities.is_empty() {
                return Err(invalid_record(
                    "syntax_cache",
                    "binding evidence in syntax cache",
                ));
            }
            let record = cache.payload(&bytes)?;
            let parts = if let Some(id) = row.get::<_, Option<i64>>(3)? {
                let bytes = hydrate_payload(&self.connection, id)?;
                let _: FileParts = serde_json::from_slice(&bytes)?;
                Some(cache.payload(&bytes)?)
            } else {
                None
            };
            cache.files.insert(
                row.get(0)?,
                CachedFile {
                    digest: row.get(1)?,
                    record,
                    parts,
                    syntax: None,
                },
            );
        }
        if membership_digest(context, &cache.membership())? != digest {
            return Err(invalid_record(
                "syntax_cache",
                "syntax membership commitment mismatch",
            ));
        }
        Ok(cache)
    }
    pub fn store_syntax_cache(
        &mut self,
        cache: &SyntaxCache,
    ) -> Result<(), KnowledgeRepositoryError> {
        if cache.unchanged {
            return Ok(());
        }
        let prepared = PreparedCache::prepare(cache)?;
        let current: Option<String> = self
            .connection
            .query_row(
                "SELECT digest FROM syntax_cache_header WHERE context=?1",
                [&cache.context],
                |r| r.get(0),
            )
            .optional()?;
        if current.as_deref() == Some(&prepared.digest) {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        prepared.store(&transaction)?;
        transaction.commit()?;
        Ok(())
    }
}
pub(super) struct PreparedCache<'a> {
    cache: &'a SyntaxCache,
    digest: String,
}
impl<'a> PreparedCache<'a> {
    pub(super) fn prepare(cache: &'a SyntaxCache) -> Result<Self, KnowledgeRepositoryError> {
        Ok(Self {
            cache,
            digest: membership_digest(&cache.context, &cache.membership())?,
        })
    }
    pub(super) fn store(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<(), KnowledgeRepositoryError> {
        let prior: Option<String> = transaction
            .query_row(
                "SELECT digest FROM syntax_cache_header WHERE context=?1",
                [&self.cache.context],
                |r| r.get(0),
            )
            .optional()?;
        if prior.as_deref() == Some(&self.digest) {
            return Ok(());
        }
        let stale_paths = {
            let mut statement = transaction.prepare("SELECT path FROM file_syntax_cache")?;
            let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for path in stale_paths {
            if !self.cache.files.contains_key(&path) {
                transaction.execute("DELETE FROM file_syntax_cache WHERE path=?1", [path])?;
            }
        }
        transaction.execute("DELETE FROM syntax_cache_header", [])?;
        transaction.execute(
            "INSERT INTO syntax_cache_header VALUES(?1,?2)",
            params![self.cache.context, self.digest],
        )?;
        for (path, file) in &self.cache.files {
            let payload = intern_payload(transaction, &file.record.read()?)?;
            let parts = file
                .parts
                .as_ref()
                .map(|part| {
                    part.read()
                        .and_then(|bytes| intern_payload(transaction, &bytes))
                })
                .transpose()?;
            transaction.execute(
                "INSERT INTO file_syntax_cache VALUES(?1,?2,?3,?4) ON CONFLICT(path) DO UPDATE SET digest=excluded.digest,payload=excluded.payload,parts=excluded.parts WHERE digest<>excluded.digest OR payload<>excluded.payload OR parts IS NOT excluded.parts",
                params![path, file.digest, payload, parts],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_ir_detects_corruption_and_rolls_back_cache_replacement() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("a.py"), "def first():\n    return 1\n").unwrap();
        let (_, mut cache) = crate::build_topology_with_syntax_cache(
            source.path(),
            crate::TopologyBuildOptions {
                repository: Some("ir-test"),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let scratch = cache.directory.path().to_owned();
        let mut db =
            SqliteKnowledgeRepository::open(&data.path().join("knowledge.sqlite3")).unwrap();
        db.store_syntax_cache(&cache).unwrap();
        let original = cache.record("a.py").unwrap().unwrap();
        let digest = cache.source_digest("a.py").unwrap().to_owned();
        cache
            .insert_record("b.py".into(), digest, &original)
            .unwrap();
        let record_file = &cache.files["b.py"].record.path;
        std::fs::write(record_file, b"{}").unwrap();
        let error = db.store_syntax_cache(&cache).unwrap_err();
        assert!(matches!(
            error,
            KnowledgeRepositoryError::InvalidRecord {
                collection: "syntax_cache",
                ..
            }
        ));
        let loaded = db.load_syntax_cache(&cache.context).unwrap();
        assert_eq!(loaded.paths().collect::<Vec<_>>(), ["a.py"]);
        assert_eq!(loaded.record("a.py").unwrap(), Some(original));
        assert!(loaded
            .qualified_parts("a.py", "wrong-source-digest")
            .unwrap()
            .is_none());
        drop(cache);
        assert!(
            !scratch.exists(),
            "temporary IR must be removed after failure"
        );
    }
}
