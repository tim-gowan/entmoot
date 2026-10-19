//! File-fact selection and disposable structural call summaries. Graph fragments
//! are projected from canonical syntax and retained source, never persisted here.
use super::*;
use crate::inventory_stitch::CallPart;
use entmoot_contracts::{FileFacts, FileIndexRecord, StructuralHashPolicy};
use entmoot_core::scan::ScanInput;
use std::rc::Rc;

/// Structural IR and derived fragments staged on disk before publication.
/// Only file commitments remain resident between jobs. Temporary bytes are
/// removed on success or failure; durable producer checkpoints own restart.
pub struct SyntaxCache {
    pub context: String,
    pub(crate) scan_commitment: Option<String>,
    files: BTreeMap<String, CachedFile>,
    directory: tempfile::TempDir,
    next_payload: u64,
    unit_store: Option<Rc<SqliteKnowledgeRepository>>,
    pub reused: usize,
    pub reused_extraction: usize,
    pub reused_stitching: usize,
    /// Complete qualified reuse permits skipping this disposable cache refresh.
    pub unchanged: bool,
    pub phases: Vec<crate::measurement::PhaseMeasurement>,
    pub build_wall_seconds: f64,
}
struct CachedFile {
    digest: String,
    record: Payload,
    parts: Option<Payload>,
    source: Option<Payload>,
}
#[derive(Clone)]
struct Payload {
    path: std::path::PathBuf,
    digest: String,
    retained: Option<(Rc<SqliteKnowledgeRepository>, RetainedKind)>,
}
#[derive(Clone)]
enum RetainedKind {
    Facts(i64),
    Calls(i64),
    Source,
}
impl Payload {
    fn read(&self) -> Result<Vec<u8>, KnowledgeRepositoryError> {
        let bytes = match &self.retained {
            None => std::fs::read(&self.path)?,
            Some((db, RetainedKind::Facts(id))) => {
                serde_json::to_vec(&super::file_facts::load(&db.connection, *id)?)?
            }
            Some((db, RetainedKind::Calls(id))) => hydrate_payload(&db.connection, *id)?,
            Some((db, RetainedKind::Source)) => db.connection.query_row(
                "SELECT bytes FROM shared_source_files WHERE digest=?1",
                [&self.digest],
                |r| r.get(0),
            )?,
        };
        if entmoot_contracts::blake3_hex(&bytes) != self.digest {
            return Err(invalid_record(
                "syntax_cache",
                "staged payload commitment mismatch",
            ));
        }
        Ok(bytes)
    }
    fn facts(&self) -> Result<FileFacts, KnowledgeRepositoryError> {
        if let Some((db, RetainedKind::Facts(id))) = &self.retained {
            let facts = super::file_facts::load(&db.connection, *id)?;
            if format!("blake3:{}", super::file_facts::content_digest(&facts)?) != self.digest {
                return Err(invalid_record(
                    "syntax_cache",
                    "retained file content commitment mismatch",
                ));
            }
            return Ok(facts);
        }
        self.hydrate()
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
            scan_commitment: None,
            files: BTreeMap::new(),
            directory,
            next_payload: 0,
            unit_store: None,
            reused: 0,
            reused_extraction: 0,
            reused_stitching: 0,
            unchanged: false,
            phases: Vec::new(),
            build_wall_seconds: 0.0,
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
    /// Retain completed file units before downstream work. Selection remains
    /// unpublished until store_syntax_cache commits the manifest. This enables
    /// retrying an interrupted parse without introducing a per-fact work queue.
    pub fn retain_units_in(&mut self, database: &Path) -> Result<(), KnowledgeRepositoryError> {
        self.unit_store = Some(Rc::new(SqliteKnowledgeRepository::open(database)?));
        Ok(())
    }
    /// Unsupported/skipped inputs still have exact historical source bytes.
    /// Their presence does not create parsed facts or close a coverage gap.
    pub(crate) fn retain_source(
        &self,
        digest: &str,
        bytes: &[u8],
    ) -> Result<(), KnowledgeRepositoryError> {
        if entmoot_contracts::blake3_hex(bytes) != digest {
            return Err(invalid_record(
                "source",
                "captured source commitment mismatch",
            ));
        }
        if let Some(store) = &self.unit_store {
            store.connection.prepare_cached("INSERT INTO shared_source_files(digest,bytes) VALUES(?1,?2) ON CONFLICT(digest) DO UPDATE SET bytes=excluded.bytes WHERE bytes IS NULL")?.execute(params![digest,bytes])?;
        }
        Ok(())
    }
    /// Portable commitment to admitted parser units, independent of SQLite IDs
    /// and disposable call-cache state. The derivation fingerprint and publication
    /// metadata separately bind the deterministic downstream interpretation.
    pub(crate) fn file_commitment(&self) -> Result<String, KnowledgeRepositoryError> {
        let files: Vec<_> = self
            .files
            .iter()
            .map(|(path, file)| (path, &file.digest, &file.record.digest))
            .collect();
        Ok(entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
            "entmoot.file_selection.v1",
            &self.context,
            &self.scan_commitment,
            files,
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

    /// Reuse only the exact sealed file/call selection for identical scan inputs.
    /// Full-source equality covers negative lookup dependencies too. Every reused
    /// payload is still verified; an absent summary is a rebuild, never no calls.
    pub(crate) fn reuse_sealed_scan(
        &mut self,
        prior: &Self,
        publication: &Path,
    ) -> Result<bool, KnowledgeRepositoryError> {
        if self.len() != prior.len() || self.reused != self.len() {
            return Ok(false);
        }
        let repository = SqliteKnowledgeRepository::open_read_snapshot(publication)?;
        let manifest: Option<(String, Option<String>)> = repository.connection.query_row(
            "SELECT digest,scan_commitment FROM file_fact_manifests WHERE view_id=(SELECT id FROM temp.selected_knowledge_view) AND context=?1",
            [&self.context], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        let Some((digest, Some(scan))) = manifest else {
            return Ok(false);
        };
        if self.scan_commitment.as_ref() != Some(&scan) {
            return Ok(false);
        }
        for (path, file) in &mut self.files {
            let Some(previous) = prior.files.get(path) else {
                return Ok(false);
            };
            if file.digest != previous.digest || file.record.digest != previous.record.digest {
                return Ok(false);
            }
            let Some(calls) = &previous.parts else {
                return Ok(false);
            };
            // Read through the original payload commitment before accepting reuse.
            let _: CallPart = calls.hydrate()?;
            file.parts = Some(calls.clone());
        }
        if membership_digest(
            &self.context,
            self.scan_commitment.as_deref(),
            &self.membership(),
        )? != digest
        {
            return Ok(false);
        }
        self.reused_stitching = self.len();
        self.unchanged = true;
        Ok(true)
    }
    /// Reuse a previously sealed complete selection, including its negative
    /// lookup dependencies. The caller has selected the exact derivation key.
    pub(crate) fn reuse_historical_selection(
        &mut self,
        prior: &Self,
    ) -> Result<(), KnowledgeRepositoryError> {
        if self.file_commitment()? != prior.file_commitment()? {
            return Err(invalid_record(
                "syntax_cache",
                "historical file selection disagrees with derivation key",
            ));
        }
        for (path, file) in &mut self.files {
            let calls = prior
                .files
                .get(path)
                .and_then(|file| file.parts.as_ref())
                .ok_or_else(|| {
                    invalid_record("syntax_cache", "historical selection lacks call summary")
                })?;
            let _: CallPart = calls.hydrate()?;
            file.parts = Some(calls.clone());
        }
        self.reused_stitching = self.len();
        Ok(())
    }

    pub fn record(&self, path: &str) -> Result<Option<FileIndexRecord>, KnowledgeRepositoryError> {
        Ok(self.facts(path)?.map(|facts| facts.to_index()))
    }
    pub fn facts(&self, path: &str) -> Result<Option<FileFacts>, KnowledgeRepositoryError> {
        self.files
            .get(path)
            .map(|file| file.record.facts())
            .transpose()
    }
    /// Captured source is used by extraction; only the publication fence reads
    /// the live tree again. Older checkpoint producers may not supply bytes.
    pub fn source(&self, path: &str) -> Result<Option<Vec<u8>>, KnowledgeRepositoryError> {
        self.files
            .get(path)
            .and_then(|file| file.source.as_ref())
            .map(Payload::read)
            .transpose()
    }
    pub fn facts_for_inputs(
        &self,
        inputs: &[ScanInput],
    ) -> Result<HashMap<String, (String, FileFacts)>, KnowledgeRepositoryError> {
        let mut found: HashMap<_, _> = inputs
            .iter()
            .filter_map(|input| {
                self.files
                    .get(&input.path)
                    .filter(|file| file.digest == input.content_digest)
                    .map(|file| {
                        file.record
                            .facts()
                            .map(|facts| (input.path.clone(), (file.digest.clone(), facts)))
                    })
            })
            .collect::<Result<_, _>>()?;
        if let Some(store) = &self.unit_store {
            let mut lookup = store.connection.prepare_cached("SELECT unit_id FROM file_fact_units WHERE context=?1 AND path=?2 AND source_digest=?3")?;
            let context = super::file_facts::binary_digest(&self.context)?;
            for input in inputs
                .iter()
                .filter(|input| !self.files.contains_key(&input.path))
            {
                let id: Option<i64> = lookup
                    .query_row(
                        params![
                            context,
                            input.path,
                            super::file_facts::binary_digest(&input.content_digest)?
                        ],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = id {
                    found.insert(
                        input.path.clone(),
                        (
                            input.content_digest.clone(),
                            super::file_facts::load(&store.connection, id)?,
                        ),
                    );
                }
            }
        }
        Ok(found)
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
                        file.record.facts().map(|facts| {
                            (input.path.clone(), (file.digest.clone(), facts.to_index()))
                        })
                    })
            })
            .collect()
    }
    pub fn qualified_calls(
        &self,
        path: &str,
        digest: &str,
    ) -> Result<Option<CallPart>, KnowledgeRepositoryError> {
        self.files
            .get(path)
            .filter(|file| file.digest == digest)
            .and_then(|file| file.parts.as_ref())
            .map(Payload::hydrate)
            .transpose()
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
            retained: None,
        })
    }
    pub fn insert_record(
        &mut self,
        path: String,
        digest: String,
        record: &FileIndexRecord,
    ) -> Result<(), KnowledgeRepositoryError> {
        let facts =
            FileFacts::from_index(record.clone()).map_err(|e| invalid_record("syntax_cache", e))?;
        self.insert_facts(path, digest, &facts, None)
    }
    pub fn insert_facts(
        &mut self,
        path: String,
        digest: String,
        facts: &FileFacts,
        source: Option<&[u8]>,
    ) -> Result<(), KnowledgeRepositoryError> {
        if let Some(store) = &self.unit_store {
            if let Some(bytes) = source {
                if entmoot_contracts::blake3_hex(bytes) != digest {
                    return Err(invalid_record(
                        "syntax_cache",
                        "captured source commitment mismatch",
                    ));
                }
            }
            let transaction = Transaction::new_unchecked(
                &store.connection,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let unit =
                super::file_facts::retain(&transaction, &self.context, &path, &digest, facts)?;
            if let Some(bytes) = source {
                transaction.prepare_cached("INSERT INTO shared_source_files(digest,bytes) VALUES(?1,?2) ON CONFLICT(digest) DO UPDATE SET bytes=excluded.bytes WHERE bytes IS NULL")?.execute(params![digest, bytes])?;
            }
            transaction.commit()?;
            let record = Payload {
                path: PathBuf::new(),
                digest: format!("blake3:{}", super::file_facts::content_digest(facts)?),
                retained: Some((Rc::clone(store), RetainedKind::Facts(unit))),
            };
            let source = source.map(|_| Payload {
                path: PathBuf::new(),
                digest: digest.clone(),
                retained: Some((Rc::clone(store), RetainedKind::Source)),
            });
            self.files.insert(
                path,
                CachedFile {
                    digest,
                    record,
                    parts: None,
                    source,
                },
            );
            return Ok(());
        }
        let source = source
            .map(|bytes| {
                if entmoot_contracts::blake3_hex(bytes) != digest {
                    return Err(invalid_record(
                        "syntax_cache",
                        "captured source commitment mismatch",
                    ));
                }
                self.payload(bytes)
            })
            .transpose()?;
        let payload = self.payload(&serde_json::to_vec(facts)?)?;
        self.files.insert(
            path,
            CachedFile {
                digest,
                record: payload,
                parts: None,
                source,
            },
        );
        Ok(())
    }
    pub fn insert_calls(
        &mut self,
        path: &str,
        calls: &CallPart,
    ) -> Result<(), KnowledgeRepositoryError> {
        if !self.files.contains_key(path) {
            return Err(invalid_record(
                "syntax_cache",
                "graph parts lack their source record",
            ));
        }
        let payload = self.payload(&encode(calls)?)?;
        if let Some(file) = self.files.get_mut(path) {
            file.parts = Some(payload);
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
fn membership_digest(
    context: &str,
    scan: Option<&str>,
    rows: &Membership,
) -> Result<String, serde_json::Error> {
    Ok(entmoot_contracts::blake3_hex(&serde_json::to_vec(&(
        "entmoot.file_syntax_membership.v1",
        context,
        scan,
        rows,
    ))?))
}
impl SqliteKnowledgeRepository {
    pub fn load_syntax_cache(
        &self,
        context: &str,
    ) -> Result<SyntaxCache, KnowledgeRepositoryError> {
        let view: String = self.connection.query_row(
            "SELECT id FROM temp.selected_knowledge_view",
            [],
            |row| row.get(0),
        )?;
        self.load_syntax_cache_for_view(context, &view)
    }

    /// Reuse hint for a new producer with no selected publication. Choose the
    /// newest retained view in the same extraction context; consumers still
    /// verify source commitments and positive/negative lookup dependencies.
    pub fn load_latest_syntax_cache(
        &self,
        context: &str,
    ) -> Result<SyntaxCache, KnowledgeRepositoryError> {
        let view: Option<String> = self.connection.query_row("SELECT m.view_id FROM file_fact_manifests m JOIN inventory_headers h USING(view_id) WHERE m.context=?1 ORDER BY h.view_key DESC LIMIT 1", [context], |row| row.get(0)).optional()?;
        match view {
            Some(view) => self.load_syntax_cache_for_view(context, &view),
            None => SyntaxCache::new(context.to_owned()),
        }
    }

    pub(super) fn load_syntax_cache_for_view(
        &self,
        context: &str,
        view: &str,
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
        // Immutable units can be loaded lazily under the same lifecycle lease.
        // In-memory stores have no reopenable backing and retain the small
        // staging fallback used by isolated tests.
        let backing = self
            .connection
            .path()
            .filter(|path| !path.is_empty())
            .map(|path| {
                let connection =
                    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                fingerprint::verify(&connection)?;
                Ok::<_, KnowledgeRepositoryError>(Rc::new(SqliteKnowledgeRepository {
                    connection,
                    lease: self.lease.clone(),
                }))
            })
            .transpose()?;
        let header: Option<(String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT digest,scan_commitment FROM file_fact_manifests WHERE context=?1 AND view_id=?2",
                [context, view],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((digest, scan)) = header else {
            return Ok(cache);
        };
        cache.scan_commitment = scan;
        let mut stmt = self
            .connection
            .prepare("SELECT c.path,'blake3:'||lower(hex(u.source_digest)),c.unit_id,c.parts,u.content_digest,p.digest,EXISTS(SELECT 1 FROM shared_source_files s WHERE s.digest='blake3:'||lower(hex(u.source_digest)) AND s.bytes IS NOT NULL) FROM file_fact_membership c JOIN file_fact_units u USING(unit_id) LEFT JOIN shared_payloads p ON p.payload_id=c.parts WHERE c.view_id=?1")?;
        let mut rows = stmt.query([view])?;
        while let Some(row) = rows.next()? {
            let record = match &backing {
                Some(db) => Payload {
                    path: PathBuf::new(),
                    digest: super::file_facts::digest_text(row.get(4)?, true)?,
                    retained: Some((Rc::clone(db), RetainedKind::Facts(row.get(2)?))),
                },
                None => cache.payload(&serde_json::to_vec(&super::file_facts::load(
                    &self.connection,
                    row.get(2)?,
                )?)?)?,
            };
            let parts = if let Some(id) = row.get::<_, Option<i64>>(3)? {
                Some(match &backing {
                    Some(db) => Payload {
                        path: PathBuf::new(),
                        digest: super::file_facts::digest_text(row.get(5)?, true)?,
                        retained: Some((Rc::clone(db), RetainedKind::Calls(id))),
                    },
                    None => cache.payload(&hydrate_payload(&self.connection, id)?)?,
                })
            } else {
                None
            };
            let source_digest: String = row.get(1)?;
            let source = if row.get::<_, bool>(6)? {
                Some(match &backing {
                    Some(db) => Payload {
                        path: PathBuf::new(),
                        digest: source_digest.clone(),
                        retained: Some((Rc::clone(db), RetainedKind::Source)),
                    },
                    None => {
                        let bytes: Vec<u8> = self.connection.query_row(
                            "SELECT bytes FROM shared_source_files WHERE digest=?1",
                            [&source_digest],
                            |r| r.get(0),
                        )?;
                        cache.payload(&bytes)?
                    }
                })
            } else {
                None
            };
            cache.files.insert(
                row.get(0)?,
                CachedFile {
                    digest: row.get(1)?,
                    record,
                    parts,
                    source,
                },
            );
        }
        if membership_digest(
            context,
            cache.scan_commitment.as_deref(),
            &cache.membership(),
        )? != digest
        {
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
        let prepared = PreparedCache::prepare(cache)?;
        let current: Option<String> = self
            .connection
            .query_row(
                "SELECT digest FROM file_fact_manifests WHERE context=?1 AND view_id=(SELECT id FROM temp.selected_knowledge_view)",
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
            digest: membership_digest(
                &cache.context,
                cache.scan_commitment.as_deref(),
                &cache.membership(),
            )?,
        })
    }
    pub(super) fn store(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<(), KnowledgeRepositoryError> {
        let prior: Option<String> = transaction
            .query_row(
                "SELECT digest FROM file_fact_manifests WHERE context=?1 AND view_id=(SELECT id FROM temp.selected_knowledge_view)",
                [&self.cache.context],
                |r| r.get(0),
            )
            .optional()?;
        if prior.as_deref() == Some(&self.digest) {
            return Ok(());
        }
        if prior.is_some() {
            return Err(invalid_record(
                "file_facts",
                "publication already selects a different file manifest",
            ));
        }
        transaction.execute("INSERT INTO file_fact_manifests VALUES((SELECT id FROM temp.selected_knowledge_view),?1,?2,?3)", params![self.cache.context, self.digest, self.cache.scan_commitment])?;
        for (path, file) in &self.cache.files {
            let facts: FileFacts = file.record.facts()?;
            let unit = super::file_facts::retain(
                transaction,
                &self.cache.context,
                path,
                &file.digest,
                &facts,
            )?;
            if let Some(source) = &file.source {
                let bytes = source.read()?;
                transaction.prepare_cached("INSERT INTO shared_source_files(digest,bytes) VALUES(?1,?2) ON CONFLICT(digest) DO UPDATE SET bytes=excluded.bytes WHERE bytes IS NULL")?.execute(params![file.digest, bytes])?;
            }
            let parts = file
                .parts
                .as_ref()
                .map(|part| {
                    part.read()
                        .and_then(|bytes| intern_payload(transaction, &bytes))
                })
                .transpose()?;
            transaction.execute(
                "INSERT INTO file_fact_membership VALUES((SELECT id FROM temp.selected_knowledge_view),?1,?2,?3)",
                params![path, unit, parts],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_file_units_survive_interruption_without_publishing_inventory() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let database = data.path().join("knowledge.sqlite3");
        std::fs::write(source.path().join("a.py"), "def stable(x):\n    return x\n").unwrap();
        let options = || crate::TopologyBuildOptions {
            repository: Some("restart-fixture"),
            ..Default::default()
        };
        let (first, cache) =
            crate::build_topology_with_retained_facts(source.path(), options(), None, &database)
                .unwrap();
        assert_eq!(cache.reused, 0);
        assert!(!SqliteKnowledgeRepository::open(&database)
            .unwrap()
            .is_initialized()
            .unwrap());
        drop(cache); // A crash before selecting a publication has the same durable boundary.
        let (restarted, cache) =
            crate::build_topology_with_retained_facts(source.path(), options(), None, &database)
                .unwrap();
        assert_eq!(cache.reused, 1);
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&restarted).unwrap()
        );
        drop(cache);
        std::fs::rename(source.path().join("a.py"), source.path().join("b.py")).unwrap();
        let (moved, cache) =
            crate::build_topology_with_retained_facts(source.path(), options(), None, &database)
                .unwrap();
        assert_eq!(cache.reused, 0, "moving a file changes occurrence context");
        let callable = |g: &TopologyGraph| {
            g.nodes
                .iter()
                .find(|n| n.kind == crate::NodeKind::Callable)
                .unwrap()
                .clone()
        };
        assert_eq!(
            callable(&first).structural_hash,
            callable(&moved).structural_hash
        );
        assert_ne!(callable(&first).address_id, callable(&moved).address_id);
        assert!(!SqliteKnowledgeRepository::open(&database)
            .unwrap()
            .is_initialized()
            .unwrap());
    }

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
        drop(db);
        let mut db = SqliteKnowledgeRepository::open_view(
            &data.path().join("knowledge.sqlite3"),
            &entmoot_contracts::blake3_hex(b"interrupted"),
        )
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
        assert!(
            db.load_syntax_cache(&cache.context).unwrap().is_empty(),
            "the interrupted revision must not inherit another revision's selection"
        );
        let original_view =
            SqliteKnowledgeRepository::open(&data.path().join("knowledge.sqlite3")).unwrap();
        let loaded = original_view.load_syntax_cache(&cache.context).unwrap();
        assert_eq!(loaded.paths().collect::<Vec<_>>(), ["a.py"]);
        assert_eq!(loaded.record("a.py").unwrap(), Some(original));
        assert!(loaded
            .qualified_calls("a.py", "wrong-source-digest")
            .unwrap()
            .is_none());
        drop(cache);
        assert!(
            !scratch.exists(),
            "temporary IR must be removed after failure"
        );
    }
}
