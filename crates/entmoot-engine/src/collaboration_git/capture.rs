//! Explicit raw working-tree receipts. No staging, filters, hooks or source refs.
use super::*;
use crate::{LocalOperations, SqliteKnowledgeRepository};
use entmoot_contracts::{ContentDigest, GitTreeCapture, GitTreeFile, WorkspacePublication};
use std::{
    collections::BTreeMap,
    io::{Seek, SeekFrom, Write},
};

#[derive(Debug)]
pub struct RetainedGitCapture {
    pub receipt: GitTreeCapture,
    pub blobs: BTreeMap<String, Vec<u8>>,
}

fn invalid(message: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(message.to_string())
}

fn required(root: &Path, args: &[&str], scratch: &Path) -> Result<Vec<u8>, OperationError> {
    let (status, bytes) = run_git(root, args, scratch, None)?;
    if status != 0 {
        return Err(OperationError::Unavailable(format!(
            "Git {} unavailable",
            args[0]
        )));
    }
    Ok(bytes)
}

fn text(root: &Path, args: &[&str], scratch: &Path) -> Result<String, OperationError> {
    Ok(String::from_utf8(required(root, args, scratch)?)
        .map_err(invalid)?
        .trim()
        .into())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\0', '\\', ':'])
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.eq_ignore_ascii_case(".git")
        })
}

fn oid(value: &str, format: &str) -> bool {
    value.len() == if format == "sha1" { 40 } else { 64 }
        && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl RetainedGitCapture {
    pub(crate) fn validate(&self) -> Result<(), OperationError> {
        let r = &self.receipt;
        r.source.snapshot.validate().map_err(invalid)?;
        if !matches!(r.object_format.as_str(), "sha1" | "sha256")
            || !oid(&r.parent_commit, &r.object_format)
            || r.files.len() > 100_000
        {
            return Err(invalid("invalid Git capture header"));
        }
        let mut previous: Option<&str> = None;
        let mut total = 0u64;
        for f in &r.files {
            if !valid_path(&f.path)
                || previous.is_some_and(|p| p >= f.path.as_str())
                || !matches!(f.mode.as_str(), "100644" | "100755" | "120000")
            {
                return Err(invalid("invalid Git capture member"));
            }
            previous = Some(&f.path);
            let bytes = self
                .blobs
                .get(f.content_digest.as_str())
                .ok_or_else(|| invalid("missing Git bytes"))?;
            total = total.saturating_add(bytes.len() as u64);
            if total > 1_073_741_824
                || entmoot_contracts::blake3_hex(bytes) != f.content_digest.as_str()
            {
                return Err(invalid("invalid Git capture bytes"));
            }
        }
        Ok(())
    }
}

#[derive(PartialEq, Eq)]
struct State {
    parent: String,
    format: String,
    index: Vec<u8>,
    flags: Vec<u8>,
    untracked: Vec<u8>,
    symlinks: bool,
    filemode: bool,
}

fn config_bool(
    root: &Path,
    name: &str,
    default: bool,
    scratch: &Path,
) -> Result<bool, OperationError> {
    let (status, value) = git(root, &["config", "--type=bool", "--get", name], scratch)?;
    match (status, value.trim()) {
        (0, "true") => Ok(true),
        (0, "false") => Ok(false),
        (1, _) => Ok(default),
        _ => Err(OperationError::Unavailable(format!(
            "invalid Git {name} setting"
        ))),
    }
}

fn state(root: &Path, scratch: &Path) -> Result<State, OperationError> {
    if !text(root, &["rev-parse", "--show-prefix"], scratch)?.is_empty()
        || text(root, &["rev-parse", "--is-bare-repository"], scratch)? != "false"
    {
        return Err(invalid("capture requires the Git worktree root"));
    }
    Ok(State {
        symlinks: config_bool(root, "core.symlinks", true, scratch)?,
        filemode: config_bool(root, "core.filemode", true, scratch)?,
        parent: text(root, &["rev-parse", "--verify", "HEAD^{commit}"], scratch)?,
        format: text(root, &["rev-parse", "--show-object-format"], scratch)?,
        index: required(root, &["ls-files", "--stage", "-z"], scratch)?,
        flags: required(root, &["ls-files", "-v", "-z"], scratch)?,
        untracked: required(
            root,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "--exclude=.tmp/entmoot-conflicts/",
                "-z",
            ],
            scratch,
        )?,
    })
}

type CapturedFiles = (Vec<GitTreeFile>, BTreeMap<String, Vec<u8>>);

fn files(
    root: &Path,
    state: &State,
    max_files: usize,
    max_bytes: u64,
) -> Result<CapturedFiles, OperationError> {
    if state
        .flags
        .split(|b| *b == 0)
        .any(|v| matches!(v.first(), Some(b'S' | b's')))
    {
        return Err(OperationError::Unavailable(
            "sparse/skip-worktree entries require a complete worktree".into(),
        ));
    }
    let mut paths = BTreeMap::new();
    for row in state.index.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let row = std::str::from_utf8(row).map_err(invalid)?;
        let (header, path) = row
            .split_once('\t')
            .ok_or_else(|| invalid("invalid Git index row"))?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 || fields[2] != "0" {
            return Err(OperationError::Unavailable(
                "unmerged Git index has no single stage".into(),
            ));
        }
        if fields[0] == "160000" {
            return Err(OperationError::Unavailable(
                "submodule working trees are outside same-repository capture".into(),
            ));
        }
        paths.insert(path.to_owned(), fields[0].to_owned());
    }
    for path in state.untracked.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        paths.insert(
            std::str::from_utf8(path).map_err(invalid)?.to_owned(),
            "100644".into(),
        );
    }
    if paths.len() > max_files {
        return Err(OperationError::Budget(
            "Git capture file budget exceeded".into(),
        ));
    }
    let mut files = Vec::new();
    let mut blobs = BTreeMap::new();
    let mut remaining = max_bytes;
    for (path, mut mode) in paths {
        if !valid_path(&path) {
            return Err(OperationError::Unavailable("unsupported Git path".into()));
        }
        // Do not follow a directory symlink while reading a tracked descendant.
        let mut ancestor = root.to_path_buf();
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                break;
            }
            ancestor.push(part);
            match std::fs::symlink_metadata(&ancestor) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(OperationError::Unavailable(
                        "symlink ancestor in Git capture".into(),
                    ))
                }
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(invalid(e)),
            }
        }
        let full = root.join(&path);
        let metadata = match std::fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(invalid(e)),
        };
        if metadata.is_dir() {
            continue;
        } // A tracked file replaced by a directory is deleted.
        let bytes = if metadata.file_type().is_symlink() {
            mode = "120000".into();
            std::fs::read_link(&full)
                .map_err(invalid)?
                .to_str()
                .ok_or_else(|| invalid("non-UTF8 symlink target"))?
                .as_bytes()
                .to_vec()
        } else if metadata.is_file() {
            if mode == "120000" && state.symlinks {
                // With native links enabled, a regular file replaced the link.
                mode = "100644".into();
            }
            #[cfg(unix)]
            if mode != "120000" && state.filemode {
                use std::os::unix::fs::PermissionsExt;
                mode = if metadata.permissions().mode() & 0o111 != 0 {
                    "100755"
                } else {
                    "100644"
                }
                .into();
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&full)
                .map_err(invalid)?
                .take(remaining + 1)
                .read_to_end(&mut bytes)
                .map_err(invalid)?;
            bytes
        } else {
            return Err(OperationError::Unavailable(
                "unsupported Git file type".into(),
            ));
        };
        remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| OperationError::Budget("Git capture byte budget exceeded".into()))?;
        let digest = entmoot_contracts::blake3_hex(&bytes);
        files.push(GitTreeFile {
            path,
            mode,
            content_digest: ContentDigest::from_trusted(digest.clone()),
        });
        blobs.insert(digest, bytes);
    }
    Ok((files, blobs))
}

/// Freeze raw files only after source, index, HEAD, path-set and byte rechecks.
/// Publication and retained bytes commit atomically under a workspace CAS.
#[allow(clippy::too_many_arguments)]
pub fn capture_workspace(
    store: &Path,
    root: &Path,
    workspace: &str,
    expected: u64,
    publisher: &str,
    max_files: usize,
    max_bytes: u64,
) -> Result<WorkspacePublication, OperationError> {
    if !(1..=100_000).contains(&max_files) || !(1..=1_073_741_824).contains(&max_bytes) {
        return Err(invalid(
            "Git capture requires 1..100000 files and 1..1073741824 bytes",
        ));
    }
    let root = std::fs::canonicalize(root).map_err(invalid)?;
    let scratch = root.join(".tmp/entmoot-conflicts");
    std::fs::create_dir_all(&scratch).map_err(invalid)?;
    let mut db = SqliteKnowledgeRepository::open(store)?;
    let publication = db.workspace_publication(workspace)?;
    if !publication.active
        || (publication.generation != expected
            && publication.generation != expected.saturating_add(1))
    {
        return Err(OperationError::Conflict(
            "workspace generation changed or retired".into(),
        ));
    }
    let source =
        crate::producer_operations::graph_source(&db.retained_graph(publication.view.as_str())?)?;
    let verify = || -> Result<(), OperationError> {
        if crate::producer_operations::observe_source(&root, &source.snapshot.repository_id)?
            != source
        {
            return Err(OperationError::Conflict(
                "Git capture source differs from registered view".into(),
            ));
        }
        Ok(())
    };
    verify()?;
    let before = state(&root, &scratch)?;
    if before.parent != source.snapshot.revision {
        return Err(OperationError::Conflict(
            "Git parent differs from structural source observation".into(),
        ));
    }
    let (manifest, blobs) = files(&root, &before, max_files, max_bytes)?;
    let after = state(&root, &scratch)?;
    if before != after
        || manifest != files(&root, &after, max_files, max_bytes)?.0
        || after != state(&root, &scratch)?
    {
        return Err(OperationError::Conflict(
            "Git capture inputs changed during observation".into(),
        ));
    }
    verify()?;
    let captured = RetainedGitCapture { receipt: GitTreeCapture {
        view: publication.view.clone(), source, parent_commit: before.parent, object_format: before.format,
        publisher: publisher.into(), files: manifest,
        limitations: vec!["raw working-tree bytes; index-only changes and clean/EOL/encoding filters are not applied".into(), "ignored untracked files and reserved .tmp/entmoot-conflicts scratch are excluded; tracked ignored files are retained".into(), "toolchain consistency is supplied by the host (for example Hermit); capture is observational, not a filesystem snapshot lock".into()],
    }, blobs };
    captured.validate()?;
    db.publish_git_capture(publication, &captured, expected)
}

/// Materialize a deterministic synthetic commit in the private object database.
pub(super) fn materialize(
    private: &Path,
    capture: &RetainedGitCapture,
    name: &str,
) -> Result<String, OperationError> {
    capture.validate()?;
    let mut input = tempfile::tempfile_in(private).map_err(invalid)?;
    writeln!(input, "commit refs/heads/{name}\ncommitter Entmoot <entmoot@invalid> 1 +0000\ndata 0\nfrom {}\ndeleteall", capture.receipt.parent_commit).map_err(invalid)?;
    for file in &capture.receipt.files {
        let mut quoted = String::from("\"");
        for b in file.path.bytes() {
            quoted.push_str(&format!("\\{b:03o}"));
        }
        quoted.push('"');
        let bytes = capture
            .blobs
            .get(file.content_digest.as_str())
            .ok_or_else(|| invalid("missing captured bytes"))?;
        writeln!(
            input,
            "M {} inline {quoted}\ndata {}",
            file.mode,
            bytes.len()
        )
        .map_err(invalid)?;
        input.write_all(bytes).map_err(invalid)?;
        writeln!(input).map_err(invalid)?;
    }
    writeln!(input, "\ndone").map_err(invalid)?;
    input.seek(SeekFrom::Start(0)).map_err(invalid)?;
    let (status, _) = run_git(
        private,
        &["fast-import", "--quiet", "--done"],
        private,
        Some(input),
    )?;
    if status != 0 {
        return Err(OperationError::Unavailable(
            "captured Git tree could not be materialized".into(),
        ));
    }
    text(
        private,
        &["rev-parse", &format!("refs/heads/{name}")],
        private,
    )
}
