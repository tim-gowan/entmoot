//! Repository scan helpers for topology / index emission.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;
use thiserror::Error;
use walkdir::WalkDir;

use entmoot_contracts::FileIndexRecord;

use crate::hasher::StructuralHasher;
use crate::index::build_file_index;
use entmoot_contracts::StructuralHashPolicy;

/// Inventory scan policy: supported sources, property-bound callables and bounded initializer syntax.
/// The series remains v1; policy and implementation fingerprints fence derivations.
pub const STRUCTURAL_SCAN_POLICY_ID: &str = "entmoot.scan.structural.v1";

/// Outcome of scanning one file.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub record: FileIndexRecord,
}

/// Content digest captured during the same repository walk as topology parsing.
#[derive(Debug, Clone)]
pub struct SnapshotFile {
    pub path: PathBuf,
    pub content_digest: String,
}

/// Aggregate scan result.
#[derive(Debug, Clone)]
pub struct RepoScanResult {
    pub reused_files: usize,
    pub excluded_directories: Vec<PathBuf>,
    /// Files excluded before reading by root-relative .gitignore rules.
    pub ignored_files: Vec<PathBuf>,
    pub files: Vec<ScannedFile>,
    pub skipped: Vec<SkippedFile>,
    pub snapshot_files: Vec<SnapshotFile>,
}

/// Source membership and explicit gaps after parser IR has been consumed.
/// This cannot be mistaken for a complete set of hydrated parser records.
#[derive(Debug, Clone)]
pub struct ScanMetadata {
    pub excluded_directories: Vec<PathBuf>,
    pub ignored_files: Vec<PathBuf>,
    pub skipped: Vec<SkippedFile>,
    pub snapshot_files: Vec<SnapshotFile>,
}

impl From<&RepoScanResult> for ScanMetadata {
    fn from(scan: &RepoScanResult) -> Self {
        Self {
            excluded_directories: scan.excluded_directories.clone(),
            ignored_files: scan.ignored_files.clone(),
            skipped: scan.skipped.clone(),
            snapshot_files: scan.snapshot_files.clone(),
        }
    }
}

impl ScanMetadata {
    pub fn from_manifest(root: &Path, manifest: &ScanManifest) -> Self {
        Self {
            excluded_directories: manifest.exclusions.directories.iter().map(|p| root.join(p)).collect(),
            ignored_files: manifest.exclusions.files.iter().map(|p| root.join(p)).collect(),
            skipped: Vec::new(),
            snapshot_files: manifest
                .inputs
                .iter()
                .map(|input| SnapshotFile {
                    path: root.join(&input.path),
                    content_digest: input.content_digest.clone(),
                })
                .collect(),
        }
    }
}

/// A file that was not hashed during repo scan.
#[derive(Debug, Clone)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SkipReason {
    UnknownExtension,
    ReadError(String),
    HashError(String),
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("repository root does not exist: {0}")]
    RootNotFound(String),
    #[error("failed to read repository root: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to create repository scan workers: {0}")]
    WorkerPool(String),
    #[error("source input changed: {0}")]
    InputChanged(String),
    #[error("invalid source path: {0}")]
    InvalidPath(String),
}

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    ".entmoot",
    ".venv",
    "venv",
    "node_modules",
    "target",
    "__pycache__",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".endorlabs",
];

/// Shared source-directory boundary for inventory and native project discovery.
pub fn should_skip_dir(name: &str) -> bool {
    SKIP_DIR_NAMES.contains(&name) || (name.starts_with('.') && name != ".github")
}

/// Exclude conventional .NET build outputs only beside a project. A bare `bin`
/// or `obj` name is not enough: Rust and other authored trees use those names.
/// Observed exclusions are retained in the scan coverage scope.
fn should_skip_source_dir(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if should_skip_dir(&name) {
        return true;
    }
    if !name.eq_ignore_ascii_case("bin") && !name.eq_ignore_ascii_case("obj") {
        return false;
    }
    path.parent()
        .and_then(|parent| std::fs::read_dir(parent).ok())
        .is_some_and(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_file())
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("csproj"))
            })
        })
}

/// Versioned scan selectors; observed hidden-directory exclusions are added by
/// the scan receipt because `.github` is an explicit exception to that rule.
pub fn scan_scope_exclusions() -> Vec<String> {
    SKIP_DIR_NAMES
        .iter()
        .map(|name| format!("**/{name}/**"))
        .chain(std::iter::once("**/.git".into()))
        .collect()
}

/// One directory walk for inventory, cached scans and source revalidation.
/// Compile each local ignore file once and retain only the active ancestor
/// matchers. Prune excluded directories before reading their contents.
fn walk_source_files(
    root: &Path,
    mut visit: impl FnMut(&Path) -> Result<(), ScanError>,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), ScanError> {
    let mut walker = WalkDir::new(root).follow_links(false).into_iter();
    let mut rules: Vec<(usize, Gitignore)> = Vec::new();
    let mut excluded_directories = Vec::new();
    let mut ignored_files = Vec::new();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(std::io::Error::other)?;
        while rules.last().is_some_and(|(depth, _)| *depth >= entry.depth()) {
            rules.pop();
        }
        let is_dir = entry.file_type().is_dir();
        let policy_file = entry.file_name() == ".gitignore";
        let ignored = !policy_file
            && rules
                .iter()
                .rev()
                .map(|(_, matcher)| matcher.matched(entry.path(), is_dir))
                .find(|matched| !matched.is_none())
                .is_some_and(|matched| matched.is_ignore());
        if is_dir {
            if entry.depth() > 0 && (should_skip_source_dir(entry.path()) || ignored) {
                excluded_directories.push(entry.path().to_path_buf());
                walker.skip_current_dir();
                continue;
            }
            let policy = entry.path().join(".gitignore");
            match std::fs::symlink_metadata(&policy) {
                Ok(metadata) if metadata.is_file() => {
                    let mut builder = GitignoreBuilder::new(entry.path());
                    if let Some(error) = builder.add(&policy) {
                        return Err(std::io::Error::other(error).into());
                    }
                    rules.push((entry.depth(), builder.build().map_err(std::io::Error::other)?));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        } else if entry.file_type().is_file() && entry.file_name() != ".git" {
            if ignored {
                ignored_files.push(entry.path().to_path_buf());
            } else {
                visit(entry.path())?;
            }
        }
    }
    excluded_directories.sort();
    ignored_files.sort();
    Ok((excluded_directories, ignored_files))
}

/// Re-observe the complete scan universe without reparsing structural inventory.
/// Added and removed paths participate; unreadable directories fail closed.
pub fn snapshot_file_digests(root: &Path) -> Result<Vec<SnapshotFile>, ScanError> {
    Ok(source_manifest(root)?
        .inputs
        .into_iter()
        .map(|input| SnapshotFile {
            path: root.join(input.path),
            content_digest: input.content_digest,
        })
        .collect())
}

/// Portable paths and exact bytes selected by the shared source walk. Parsing is
/// independent of this manifest; a partial batch is never a complete inventory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanInput {
    pub path: String,
    pub content_digest: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ScanExclusions {
    pub directories: Vec<String>,
    pub files: Vec<String>,
}

pub struct ScanManifest {
    pub inputs: Vec<ScanInput>,
    pub exclusions: ScanExclusions,
}

/// Complete outcome for a single selected file, including unsupported syntax.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanOutcome {
    pub input: ScanInput,
    pub record: Option<FileIndexRecord>,
    pub skipped: Option<SkipReason>,
}

pub fn source_manifest(root: &Path) -> Result<ScanManifest, ScanError> {
    if !root.is_dir() {
        return Err(ScanError::RootNotFound(root.display().to_string()));
    }
    let mut inputs = Vec::new();
    let relative = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let (directories, files) = walk_source_files(root, |path| {
        inputs.push(ScanInput {
            path: relative(path),
            content_digest: entmoot_contracts::blake3_hex(&std::fs::read(path)?),
        });
        Ok(())
    })?;
    // Match the shared scan's platform path order, including non-ASCII names.
    inputs.sort_by(|a, b| Path::new(&a.path).cmp(Path::new(&b.path)));
    Ok(ScanManifest {
        inputs,
        exclusions: ScanExclusions {
            directories: directories.iter().map(|p| relative(p)).collect(),
            files: files.iter().map(|p| relative(p)).collect(),
        },
    })
}

/// Reuse at most four workers across bounded batches. A batch reads only its selected files;
/// parsing and diagnostics use the same owner as ordinary and cached scans.
pub struct ScanWorkers {
    pool: rayon::ThreadPool,
}

impl ScanWorkers {
    pub fn new() -> Result<Self, ScanError> {
        Ok(Self {
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(
                    std::thread::available_parallelism()
                        .map(usize::from)
                        .unwrap_or(1)
                        .min(4),
                )
                .stack_size(SCAN_WORKER_STACK_BYTES)
                .build()
                .map_err(|e| ScanError::WorkerPool(e.to_string()))?,
        })
    }

    pub fn scan(
        &self,
        root: &Path,
        repository: &str,
        policy: &StructuralHashPolicy,
        inputs: &[ScanInput],
    ) -> Result<Vec<ScanOutcome>, ScanError> {
        self.scan_cached(root, repository, policy, inputs, &HashMap::new())
            .map(|(outcomes, _)| outcomes)
    }

    /// Exact-byte cache reuse under a caller-qualified repository/implementation
    /// context. Every selected file is still read and checked against its input.
    pub fn scan_cached(
        &self,
        root: &Path,
        repository: &str,
        policy: &StructuralHashPolicy,
        inputs: &[ScanInput],
        records: &HashMap<String, (String, FileIndexRecord)>,
    ) -> Result<(Vec<ScanOutcome>, usize), ScanError> {
        let policy_id = policy.fingerprint();
        let outcomes = self.pool.install(|| {
            inputs
                .par_iter()
                .map(|input| {
                    if input.path.is_empty()
                        || Path::new(&input.path)
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                    {
                        return Err(ScanError::InvalidPath(input.path.clone()));
                    }
                    let path = root.join(&input.path);
                    let bytes = std::fs::read(&path)?;
                    if entmoot_contracts::blake3_hex(&bytes) != input.content_digest {
                        return Err(ScanError::InputChanged(input.path.clone()));
                    }
                    let extension = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{e}"))
                        .unwrap_or_default();
                    let language = THREAD_HASHER.with(|h| h.language_for_extension(&extension));
                    let work = file_work(path, input.path.clone(), input.content_digest.clone(), bytes, language);
                    let work = reuse_file_work(work, &input.path, &input.content_digest, &policy_id, Some(records));
                    let reused = matches!(work, FileWork::Cached { .. });
                    let (_, file, skipped) = process_file(work, repository, Some(policy));
                    Ok((
                        ScanOutcome {
                            input: input.clone(),
                            record: file.map(|f| f.record),
                            skipped: skipped.map(|f| f.reason),
                        },
                        reused,
                    ))
                })
                .collect::<Result<Vec<_>, ScanError>>()
        })?;
        let reused = outcomes.iter().filter(|(_, reused)| *reused).count();
        Ok((outcomes.into_iter().map(|(outcome, _)| outcome).collect(), reused))
    }
}

/// Reassemble only after the caller verifies every manifest member has an outcome.
pub fn completed_scan(root: &Path, exclusions: ScanExclusions, outcomes: Vec<ScanOutcome>) -> RepoScanResult {
    let mut scan = RepoScanResult {
        reused_files: 0,
        excluded_directories: exclusions.directories.iter().map(|p| root.join(p)).collect(),
        ignored_files: exclusions.files.iter().map(|p| root.join(p)).collect(),
        files: vec![],
        skipped: vec![],
        snapshot_files: vec![],
    };
    for outcome in outcomes {
        let path = root.join(&outcome.input.path);
        scan.snapshot_files.push(SnapshotFile {
            path: path.clone(),
            content_digest: outcome.input.content_digest,
        });
        if let Some(record) = outcome.record {
            scan.files.push(ScannedFile {
                path: path.clone(),
                record,
            });
        }
        if let Some(reason) = outcome.skipped {
            scan.skipped.push(SkippedFile { path, reason });
        }
    }
    scan.files.sort_by(|a, b| a.path.cmp(&b.path));
    scan.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    scan.snapshot_files.sort_by(|a, b| a.path.cmp(&b.path));
    scan
}

thread_local! {
    static THREAD_HASHER: StructuralHasher = StructuralHasher::new();
}

/// Language visitors recurse while assembling canonical token streams. Keep the
/// worker stack explicit so behavior does not depend on Rayon's platform default;
/// [`crate::normalizer::MAX_RECURSIVE_DEPTH`] remains the fail-closed syntax limit.
const SCAN_WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

enum FileWork {
    Cached {
        path: PathBuf,
        content_digest: String,
        record: Box<FileIndexRecord>,
    },
    SnapshotOnly {
        path: PathBuf,
        content_digest: String,
        skipped: SkippedFile,
    },
    /// Digest matches a prior snapshot; skip structural parse/hash.
    Reuse { path: PathBuf, content_digest: String },
    Hashable {
        path: PathBuf,
        rel: String,
        source: String,
        content_digest: String,
        language: String,
    },
}

/// Walk `root`, hash supported source files with analysis markers, emit index records.
///
/// File hashing is parallelized with thread-local hashers; outputs are sorted by path.
pub fn scan_repo_with_analysis(
    hasher: &StructuralHasher,
    root: &Path,
    policy: Option<&StructuralHashPolicy>,
    repository: Option<&str>,
) -> Result<RepoScanResult, ScanError> {
    scan_repo_with_analysis_digest_gated(hasher, root, policy, repository, None)
}

/// Like [`scan_repo_with_analysis`], but skip structural parse/hash when
/// `prior_digests` maps the repository-relative POSIX path to the same content digest.
///
/// Every path still appears in `snapshot_files`. Unchanged hashable paths omit
/// `files` entries so callers can splice structural inventory from a prior graph.
pub fn scan_repo_with_analysis_digest_gated(
    hasher: &StructuralHasher,
    root: &Path,
    policy: Option<&StructuralHashPolicy>,
    repository: Option<&str>,
    prior_digests: Option<&HashMap<String, String>>,
) -> Result<RepoScanResult, ScanError> {
    scan_repo_inner(hasher, root, policy, repository, prior_digests, None)
}

/// Reuse complete syntax records; callers must qualify repository/path, parser
/// scan policy and normalization policy before supplying this disposable cache.
/// Bindings must be recomputed over the resulting complete record set.
pub fn scan_repo_with_analysis_cached(
    hasher: &StructuralHasher,
    root: &Path,
    policy: Option<&StructuralHashPolicy>,
    repository: Option<&str>,
    records: &HashMap<String, (String, FileIndexRecord)>,
) -> Result<RepoScanResult, ScanError> {
    scan_repo_inner(hasher, root, policy, repository, None, Some(records))
}

fn reuse_file_work(
    work: FileWork,
    relative: &str,
    digest: &str,
    policy: &entmoot_contracts::PolicyId,
    records: Option<&HashMap<String, (String, FileIndexRecord)>>,
) -> FileWork {
    if let FileWork::Hashable { path, .. } = &work {
        if let Some((prior, record)) = records.and_then(|records| records.get(relative)) {
            if prior == digest && &record.policy_id == policy && record.resolved_identities.is_empty() {
                return FileWork::Cached {
                    path: path.clone(),
                    content_digest: digest.into(),
                    record: Box::new(record.clone()),
                };
            }
        }
    }
    work
}

fn scan_repo_inner(
    hasher: &StructuralHasher,
    root: &Path,
    policy: Option<&StructuralHashPolicy>,
    repository: Option<&str>,
    prior_digests: Option<&HashMap<String, String>>,
    records: Option<&HashMap<String, (String, FileIndexRecord)>>,
) -> Result<RepoScanResult, ScanError> {
    if !root.exists() {
        return Err(ScanError::RootNotFound(root.display().to_string()));
    }

    let repo_name = repository
        .map(str::to_string)
        .unwrap_or_else(|| root.file_name().and_then(|n| n.to_str()).unwrap_or("repo").to_string());

    let policy_id = policy.cloned().unwrap_or_default().fingerprint();
    let mut work = Vec::new();
    let (excluded_directories, ignored_files) = walk_source_files(root, |path| {
        let path = path.to_path_buf();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path)?;
        let content_digest = entmoot_contracts::blake3_hex(&bytes);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();

        let item = file_work(
            path.clone(),
            rel.clone(),
            content_digest.clone(),
            bytes,
            hasher.language_for_extension(&ext),
        );
        if !matches!(item, FileWork::Hashable { .. }) {
            work.push(item);
            return Ok(());
        }

        let item = reuse_file_work(item, &rel, &content_digest, &policy_id, records);
        if matches!(item, FileWork::Cached { .. }) {
            work.push(item);
            return Ok(());
        }

        if prior_digests
            .and_then(|prior| prior.get(rel.as_str()))
            .is_some_and(|prior| prior == &content_digest)
        {
            work.push(FileWork::Reuse { path, content_digest });
            return Ok(());
        }
        work.push(item);
        Ok(())
    })?;

    let reused_files = work
        .iter()
        .filter(|w| matches!(w, FileWork::Cached { .. } | FileWork::Reuse { .. }))
        .count();
    let policy = policy.cloned();
    let needs_workers = work.iter().any(|w| matches!(w, FileWork::Hashable { .. }));
    let outcomes: Vec<_> = if needs_workers {
        let workers = ScanWorkers::new()?;
        workers.pool.install(|| {
            work.into_par_iter()
                .map(|item| process_file(item, &repo_name, policy.as_ref()))
                .collect()
        })
    } else {
        work.into_iter()
            .map(|item| process_file(item, &repo_name, policy.as_ref()))
            .collect()
    };

    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut snapshot_files = Vec::new();
    for (snap, file, skip) in outcomes {
        snapshot_files.push(snap);
        if let Some(f) = file {
            files.push(f);
        }
        if let Some(s) = skip {
            skipped.push(s);
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    skipped.sort_by(|a, b| a.path.cmp(&b.path));
    snapshot_files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(RepoScanResult {
        reused_files,
        excluded_directories,
        ignored_files,
        files,
        skipped,
        snapshot_files,
    })
}

/// One boundary for extension classification, source decoding and skip reasons.
fn file_work(path: PathBuf, rel: String, content_digest: String, bytes: Vec<u8>, language: Option<String>) -> FileWork {
    let Some(language) = language else {
        return FileWork::SnapshotOnly {
            path: path.clone(),
            content_digest,
            skipped: SkippedFile {
                path,
                reason: SkipReason::UnknownExtension,
            },
        };
    };
    match String::from_utf8(bytes) {
        Ok(source) => FileWork::Hashable {
            path,
            rel,
            source,
            content_digest,
            language,
        },
        Err(error) => FileWork::SnapshotOnly {
            path: path.clone(),
            content_digest,
            skipped: SkippedFile {
                path,
                reason: SkipReason::ReadError(error.to_string()),
            },
        },
    }
}

fn process_file(
    item: FileWork,
    repo_name: &str,
    policy: Option<&StructuralHashPolicy>,
) -> (SnapshotFile, Option<ScannedFile>, Option<SkippedFile>) {
    match item {
        FileWork::Cached {
            path,
            content_digest,
            record,
        } => (
            SnapshotFile {
                path: path.clone(),
                content_digest,
            },
            Some(ScannedFile { path, record: *record }),
            None,
        ),
        FileWork::SnapshotOnly {
            path,
            content_digest,
            skipped,
        } => (SnapshotFile { path, content_digest }, None, Some(skipped)),
        FileWork::Reuse { path, content_digest } => (SnapshotFile { path, content_digest }, None, None),
        FileWork::Hashable {
            path,
            rel,
            source,
            content_digest,
            language,
        } => {
            let hash_result = THREAD_HASHER.with(|local| {
                local.compute_granular_hash_with_analysis_at(&source, &language, Some(&rel), Some(repo_name), policy)
            });
            match hash_result {
                Ok(result) => (
                    SnapshotFile {
                        path: path.clone(),
                        content_digest,
                    },
                    Some(ScannedFile {
                        path,
                        record: build_file_index(&result),
                    }),
                    None,
                ),
                Err(e) => (
                    SnapshotFile {
                        path: path.clone(),
                        content_digest,
                    },
                    None,
                    Some(SkippedFile {
                        path,
                        reason: SkipReason::HashError(e.to_string()),
                    }),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn cached_batches_still_fence_source_policy_path_and_native_evidence() {
        let dir = tempdir().unwrap();
        let source = "def leaf(): return 1\n";
        fs::write(dir.path().join("leaf.py"), source).unwrap();
        let workers = ScanWorkers::new().unwrap();
        let inputs = source_manifest(dir.path()).unwrap().inputs;
        let cold = workers
            .scan(dir.path(), "fixture", &Default::default(), &inputs)
            .unwrap();
        let record = cold[0].record.clone().unwrap();
        let mut records = HashMap::from([("leaf.py".into(), (inputs[0].content_digest.clone(), record.clone()))]);
        let (warm, reused) = workers
            .scan_cached(dir.path(), "fixture", &Default::default(), &inputs, &records)
            .unwrap();
        assert_eq!(reused, 1);
        assert_eq!(
            serde_json::to_value(warm).unwrap(),
            serde_json::to_value(&cold).unwrap()
        );
        records.get_mut("leaf.py").unwrap().1.policy_id = entmoot_contracts::PolicyId::from_trusted("different-policy");
        assert_eq!(
            workers
                .scan_cached(dir.path(), "fixture", &Default::default(), &inputs, &records)
                .unwrap()
                .1,
            0
        );
        let cached = &mut records.get_mut("leaf.py").unwrap().1;
        *cached = record;
        cached
            .resolved_identities
            .push(entmoot_contracts::ResolvedIdentityRecord {
                address_id: "native".into(),
                resolved_hash: "native".into(),
                resolution_policy_id: "native".into(),
                binding: entmoot_contracts::ResolvedBindingEvidence::Unavailable {
                    reason_codes: vec!["missing".into()],
                },
            });
        let (clean, reused) = workers
            .scan_cached(dir.path(), "fixture", &Default::default(), &inputs, &records)
            .unwrap();
        assert_eq!(reused, 0);
        assert!(clean[0].record.as_ref().unwrap().resolved_identities.is_empty());
        records.get_mut("leaf.py").unwrap().1.resolved_identities.clear();
        fs::write(dir.path().join("leaf.py"), source.replace('1', "2")).unwrap();
        assert!(matches!(
            workers.scan_cached(dir.path(), "fixture", &Default::default(), &inputs, &records),
            Err(ScanError::InputChanged(_))
        ));
        fs::write(dir.path().join("leaf.py"), source).unwrap();
        fs::rename(dir.path().join("leaf.py"), dir.path().join("moved.py")).unwrap();
        let moved = source_manifest(dir.path()).unwrap().inputs;
        assert_eq!(
            workers
                .scan_cached(dir.path(), "fixture", &Default::default(), &moved, &records)
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn file_batches_preserve_empty_scans_and_reject_changed_or_escaping_inputs() {
        let dir = tempdir().unwrap();
        let workers = ScanWorkers::new().unwrap();
        let manifest = source_manifest(dir.path()).unwrap();
        assert!(manifest.inputs.is_empty());
        let outcomes = workers
            .scan(dir.path(), "fixture", &Default::default(), &manifest.inputs)
            .unwrap();
        let scan = completed_scan(dir.path(), manifest.exclusions, outcomes);
        assert!(scan.files.is_empty());
        assert!(scan.skipped.is_empty());
        assert!(scan.snapshot_files.is_empty());
        fs::write(dir.path().join("source.py"), "def before(): return 1\n").unwrap();
        let manifest = source_manifest(dir.path()).unwrap();
        fs::write(dir.path().join("source.py"), "def after(): return 2\n").unwrap();
        assert!(
            matches!(workers.scan(dir.path(), "fixture", &Default::default(), &manifest.inputs), Err(ScanError::InputChanged(path)) if path == "source.py")
        );
        for path in [
            String::new(),
            "../source.py".into(),
            dir.path().join("source.py").to_string_lossy().into_owned(),
        ] {
            let input = ScanInput {
                path,
                content_digest: manifest.inputs[0].content_digest.clone(),
            };
            assert!(matches!(
                workers.scan(dir.path(), "fixture", &Default::default(), &[input]),
                Err(ScanError::InvalidPath(_))
            ));
        }
        assert!(matches!(
            source_manifest(&dir.path().join("missing")),
            Err(ScanError::RootNotFound(_))
        ));
        assert!(matches!(
            source_manifest(&dir.path().join("source.py")),
            Err(ScanError::RootNotFound(_))
        ));
    }

    #[test]
    fn file_batches_match_shared_scope_and_unicode_path_order() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::create_dir(dir.path().join("ignored")).unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored/\n*.txt\n").unwrap();
        fs::write(dir.path().join("ignored/hidden.py"), "def ignored(): pass\n").unwrap();
        fs::write(dir.path().join("unselected.txt"), "ignored").unwrap();
        for name in ["nested/é.py", "α.py", "a.py", "𐐀.py"] {
            fs::write(dir.path().join(name), "def selected(): return 1\n").unwrap();
        }
        let full = scan_repo_with_analysis(&StructuralHasher::new(), dir.path(), None, Some("fixture")).unwrap();
        let manifest = source_manifest(dir.path()).unwrap();
        let workers = ScanWorkers::new().unwrap();
        let mut outcomes = Vec::new();
        for batch in manifest.inputs.chunks(2) {
            outcomes.extend(workers.scan(dir.path(), "fixture", &Default::default(), batch).unwrap());
        }
        let resumed = completed_scan(dir.path(), manifest.exclusions, outcomes);
        assert_eq!(resumed.excluded_directories, full.excluded_directories);
        assert_eq!(resumed.ignored_files, full.ignored_files);
        assert_eq!(
            resumed
                .snapshot_files
                .iter()
                .map(|f| (&f.path, &f.content_digest))
                .collect::<Vec<_>>(),
            full.snapshot_files
                .iter()
                .map(|f| (&f.path, &f.content_digest))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            resumed.skipped.iter().map(|f| (&f.path, &f.reason)).collect::<Vec<_>>(),
            full.skipped.iter().map(|f| (&f.path, &f.reason)).collect::<Vec<_>>()
        );
        assert_eq!(resumed.files.len(), full.files.len());
        for (resumed, full) in resumed.files.iter().zip(&full.files) {
            // Windows accepts both separators; paths compare by components.
            assert_eq!(resumed.path, full.path);
            assert_eq!(
                serde_json::to_value(&resumed.record).unwrap(),
                serde_json::to_value(&full.record).unwrap()
            );
        }
    }

    #[test]
    fn gitignore_scope_is_shared_by_full_cached_and_snapshot_scans() {
        let parent = tempdir().unwrap();
        fs::write(parent.path().join(".gitignore"), ".tmp/\n").unwrap();
        let root = parent.path().join(".tmp/checkout");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir(root.join("ignored")).unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n*.py\n!keep.py\n").unwrap();
        fs::write(root.join("nested/.gitignore"), "!override.py\n").unwrap();
        for name in [
            "keep.py",
            "drop.py",
            "nested/keep.py",
            "nested/drop.py",
            "nested/override.py",
        ] {
            fs::write(root.join(name), "def leaf(): return 1\n").unwrap();
        }
        fs::write(root.join("ignored/broken.py"), [0xff]).unwrap();
        let hasher = StructuralHasher::new();
        let scan = scan_repo_with_analysis(&hasher, &root, None, None).unwrap();
        let names = |files: &[ScannedFile]| {
            files
                .iter()
                .map(|file| {
                    file.path
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&scan.files), ["keep.py", "nested/keep.py", "nested/override.py"]);
        let digests = |files: &[SnapshotFile]| {
            files
                .iter()
                .map(|file| (file.path.clone(), file.content_digest.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            digests(&scan.snapshot_files),
            digests(&snapshot_file_digests(&root).unwrap())
        );
        assert!(scan
            .snapshot_files
            .iter()
            .any(|file| file.path == root.join(".gitignore")));
        fs::write(root.join("drop.py"), "def changed(): return 2\n").unwrap();
        assert_eq!(
            digests(&scan.snapshot_files),
            digests(&snapshot_file_digests(&root).unwrap())
        );
        let records = scan
            .files
            .iter()
            .map(|file| {
                let digest = scan
                    .snapshot_files
                    .iter()
                    .find(|source| source.path == file.path)
                    .unwrap();
                (
                    file.path
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    (digest.content_digest.clone(), file.record.clone()),
                )
            })
            .collect();
        let cached = scan_repo_with_analysis_cached(&hasher, &root, None, None, &records).unwrap();
        assert_eq!(names(&cached.files), names(&scan.files));
        assert_eq!(cached.reused_files, 3);
        fs::write(root.join(".gitignore"), "ignored/\n*.py\n!keep.py\n!drop.py\n").unwrap();
        let changed = scan_repo_with_analysis_cached(&hasher, &root, None, None, &records).unwrap();
        assert_eq!(
            names(&changed.files),
            [
                "drop.py",
                "keep.py",
                "nested/drop.py",
                "nested/keep.py",
                "nested/override.py"
            ]
        );
        assert_eq!(changed.reused_files, 3);
    }

    #[test]
    fn scans_python_fixture() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.py");
        fs::write(&path, "def foo():\n    return 1\n").unwrap();
        let hasher = StructuralHasher::new();
        let result = scan_repo_with_analysis(&hasher, dir.path(), None, None).unwrap();
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn linked_worktree_administration_is_outside_every_source_universe() {
        let dir = tempdir().unwrap();
        for (name, source) in [
            ("a.rs", "fn value() -> i32 { 1 }"),
            ("a.py", "def value():\n    return 1\n"),
            ("a.js", "function value() { return 1; }"),
            ("a.ts", "function value(): number { return 1; }"),
            ("a.tsx", "function value() { return <div/>; }"),
            ("a.go", "package fixture\nfunc value() int { return 1 }"),
            ("A.java", "class A { int value() { return 1; } }"),
            ("A.cs", "class A { int Value() { return 1; } }"),
            ("a.c", "int value(void) { return 1; }"),
        ] {
            fs::write(dir.path().join(name), source).unwrap();
        }
        fs::write(dir.path().join(".git"), "gitdir: /first/location\n").unwrap();
        let hasher = StructuralHasher::new();
        let first = scan_repo_with_analysis(&hasher, dir.path(), None, None).unwrap();
        assert_eq!(first.files.len(), 9);
        assert_eq!(first.snapshot_files.len(), 9);
        assert!(first.skipped.is_empty());
        fs::write(dir.path().join(".git"), "gitdir: /different/location\n").unwrap();
        let observed = snapshot_file_digests(dir.path()).unwrap();
        let expected: Vec<_> = first
            .snapshot_files
            .iter()
            .map(|file| (&file.path, &file.content_digest))
            .collect();
        let actual: Vec<_> = observed.iter().map(|file| (&file.path, &file.content_digest)).collect();
        assert_eq!(actual, expected);
        assert!(scan_scope_exclusions().contains(&"**/.git".into()));
    }

    #[test]
    fn generated_directory_is_structurally_indexed() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("generated")).unwrap();
        fs::write(dir.path().join("generated/schema.json"), "{}\n").unwrap();
        fs::write(dir.path().join("generated/api.py"), "def generated():\n    return 1\n").unwrap();
        let result = scan_repo_with_analysis(&StructuralHasher::new(), dir.path(), None, None).unwrap();
        assert_eq!(result.snapshot_files.len(), 2);
        assert_eq!(result.files.len(), 1);
        assert!(result.files[0].path.ends_with("api.py"));
    }

    #[test]
    fn digest_gate_skips_hash_for_unchanged_paths() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("keep.py"), "def keep():\n    return 1\n").unwrap();
        fs::write(dir.path().join("change.py"), "def change():\n    return 1\n").unwrap();
        let hasher = StructuralHasher::new();
        let first = scan_repo_with_analysis(&hasher, dir.path(), None, None).unwrap();
        assert_eq!(first.files.len(), 2);
        let prior: HashMap<String, String> = first
            .snapshot_files
            .iter()
            .map(|file| {
                (
                    file.path
                        .strip_prefix(dir.path())
                        .unwrap_or(&file.path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                    file.content_digest.clone(),
                )
            })
            .collect();
        fs::write(dir.path().join("change.py"), "def change():\n    return 2\n").unwrap();
        let second = scan_repo_with_analysis_digest_gated(&hasher, dir.path(), None, None, Some(&prior)).unwrap();
        assert_eq!(second.files.len(), 1);
        assert!(second.files.iter().any(|file| file.path.ends_with("change.py")));
        assert_eq!(second.snapshot_files.len(), 2);
    }
}
