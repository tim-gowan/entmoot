//! Disposable, latest-only syntax cache; never a source of binding/claim truth.
use super::*;
use crate::inventory_stitch::{CallPart, SyntaxPart};
use entmoot_contracts::{FileIndexRecord, StructuralHashPolicy};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS syntax_cache_header(context TEXT PRIMARY KEY, digest TEXT NOT NULL); CREATE TABLE IF NOT EXISTS file_syntax_cache(path TEXT PRIMARY KEY, digest TEXT NOT NULL, payload INTEGER NOT NULL REFERENCES shared_payloads, parts INTEGER REFERENCES shared_payloads);";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FileParts {
    pub syntax: SyntaxPart,
    pub calls: CallPart,
}

#[derive(Default)]
pub struct SyntaxCache {
    pub context: String,
    pub records: HashMap<String, (String, FileIndexRecord)>,
    pub parts: HashMap<String, FileParts>,
    pub reused: usize,
    pub reused_extraction: usize,
    pub reused_stitching: usize,
    /// A producer reused the complete cache it read. Skipping its refresh is
    /// safe even if another producer has since replaced this disposable cache.
    pub unchanged: bool,
}

impl SyntaxCache {
    pub fn context(
        repository: &str,
        policy: &StructuralHashPolicy,
    ) -> Result<String, serde_json::Error> {
        // Paths inside records are repository-relative. The host checkout root
        // is local wiring and must not prevent reuse by another worktree.
        Ok(entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
            "entmoot.file_syntax_cache.v1",
            repository,
            entmoot_core::scan::STRUCTURAL_SCAN_POLICY_ID,
            policy.fingerprint(),
        ))?))
    }

    pub fn qualified_parts(&self, path: &str, digest: &str) -> Option<&FileParts> {
        self.records
            .get(path)
            .filter(|(prior, _)| prior == digest)
            .and_then(|_| self.parts.get(path))
    }

    fn validate(&self) -> Result<(), KnowledgeRepositoryError> {
        if self
            .records
            .values()
            .any(|(_, record)| !record.resolved_identities.is_empty())
        {
            return Err(invalid_record(
                "syntax_cache",
                "binding evidence in syntax cache",
            ));
        }
        if self
            .parts
            .keys()
            .any(|path| !self.records.contains_key(path))
        {
            return Err(invalid_record(
                "syntax_cache",
                "graph parts lack their source record",
            ));
        }
        Ok(())
    }
}

// Commit verified payload bytes, not a second serialization of hydrated trees.
// The implementation fingerprint fences changes to this derivation within v1.
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
        // Ordinary readers already pin a transaction. Producer connections need
        // a temporary read snapshot while another job may replace the cache.
        let _snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let mut cache = SyntaxCache {
            context: context.into(),
            ..Default::default()
        };
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
        let mut membership = Membership::new();
        while let Some(row) = rows.next()? {
            let bytes = hydrate_payload(&self.connection, row.get(2)?)?;
            let record: FileIndexRecord = serde_json::from_slice(&bytes)?;
            if !record.resolved_identities.is_empty() {
                return Err(invalid_record(
                    "syntax_cache",
                    "binding evidence in syntax cache",
                ));
            }
            let path: String = row.get(0)?;
            let parts_digest = if let Some(payload) = row.get::<_, Option<i64>>(3)? {
                let bytes = hydrate_payload(&self.connection, payload)?;
                cache
                    .parts
                    .insert(path.clone(), serde_json::from_slice(&bytes)?);
                Some(entmoot_contracts::blake3_hex(&bytes))
            } else {
                None
            };
            let source: String = row.get(1)?;
            membership.insert(
                path.clone(),
                (
                    source.clone(),
                    entmoot_contracts::blake3_hex(&bytes),
                    parts_digest,
                ),
            );
            cache.records.insert(path, (source, record));
        }
        if membership_digest(context, &membership)? != digest {
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
        cache.validate()?;
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
    encoded: Vec<EncodedFile<'a>>,
}
struct EncodedFile<'a> {
    path: &'a str,
    digest: &'a str,
    record: Vec<u8>,
    parts: Option<Vec<u8>>,
}
impl<'a> PreparedCache<'a> {
    pub(super) fn prepare(cache: &'a SyntaxCache) -> Result<Self, KnowledgeRepositoryError> {
        cache.validate()?;
        // Shared payload storage canonicalizes JSON object ordering. Commit that
        // representation so hydration need not reconstruct typed serialization.
        fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
            serde_json::to_vec(&serde_json::to_value(value)?)
        }
        let encoded = cache
            .records
            .iter()
            .map(|(path, (digest, record))| {
                Ok(EncodedFile {
                    path,
                    digest,
                    record: encode(record)?,
                    parts: cache.parts.get(path).map(encode).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let membership = encoded
            .iter()
            .map(|file| {
                (
                    file.path.to_owned(),
                    (
                        file.digest.to_owned(),
                        entmoot_contracts::blake3_hex(&file.record),
                        file.parts
                            .as_ref()
                            .map(|bytes| entmoot_contracts::blake3_hex(bytes)),
                    ),
                )
            })
            .collect();
        let digest = membership_digest(&cache.context, &membership)?;
        Ok(Self {
            cache,
            digest,
            encoded,
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
        // Keep unchanged memberships in place. Content-addressed payloads also
        // share identical fragments between the cache and durable checkpoints.
        let stale_paths = {
            let mut statement = transaction.prepare("SELECT path FROM file_syntax_cache")?;
            let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for path in stale_paths {
            if !self.cache.records.contains_key(&path) {
                transaction.execute("DELETE FROM file_syntax_cache WHERE path=?1", [path])?;
            }
        }
        transaction.execute("DELETE FROM syntax_cache_header", [])?;
        transaction.execute(
            "INSERT INTO syntax_cache_header VALUES(?1,?2)",
            params![self.cache.context, self.digest],
        )?;
        for file in &self.encoded {
            let payload = intern_payload(transaction, &file.record)?;
            let parts = file
                .parts
                .as_ref()
                .map(|bytes| intern_payload(transaction, bytes))
                .transpose()?;
            transaction.execute(
                "INSERT INTO file_syntax_cache VALUES(?1,?2,?3,?4) ON CONFLICT(path) DO UPDATE SET digest=excluded.digest,payload=excluded.payload,parts=excluded.parts WHERE digest<>excluded.digest OR payload<>excluded.payload OR parts IS NOT excluded.parts",
                params![file.path, file.digest, payload, parts],
            )?;
        }
        Ok(())
    }
}
