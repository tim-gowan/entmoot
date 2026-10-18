//! Private Git object execution shared by durable ordered integration workers.
use super::*;
use std::io::{Seek, SeekFrom, Write};

pub struct GitIntegration {
    private: tempfile::TempDir,
    pub version: String,
    oid_len: usize,
}

impl GitIntegration {
    pub fn open(root: &Path, scratch: &Path, oid_len: usize) -> Result<Self, OperationError> {
        if !matches!(oid_len, 40 | 64) {
            return Err(OperationError::Invalid("invalid Git object format".into()));
        }
        let private = private_repository(root, scratch, oid_len == 64)?;
        let (code, version) = git(private.path(), &["--version"], private.path())?;
        if code != 0 {
            return Err(OperationError::Unavailable(
                "Git version unavailable".into(),
            ));
        }
        Ok(Self {
            private,
            version: version.trim().into(),
            oid_len,
        })
    }

    fn oid(&self, value: &str) -> Result<(), OperationError> {
        if value.len() != self.oid_len || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(OperationError::Invalid(
                "invalid or incompatible Git object ID".into(),
            ));
        }
        Ok(())
    }

    fn input(&self, bytes: &[u8]) -> Result<std::fs::File, OperationError> {
        let mut input = tempfile::tempfile_in(self.private.path())
            .map_err(crate::KnowledgeRepositoryError::from)?;
        input
            .write_all(bytes)
            .map_err(crate::KnowledgeRepositoryError::from)?;
        input
            .seek(SeekFrom::Start(0))
            .map_err(crate::KnowledgeRepositoryError::from)?;
        Ok(input)
    }

    fn required(&self, args: &[&str], input: &[u8], limit: u64) -> Result<Vec<u8>, OperationError> {
        let (code, bytes) = run_git_bounded(
            self.private.path(),
            args,
            self.private.path(),
            Some(self.input(input)?),
            limit,
        )?;
        if code != 0 {
            return Err(OperationError::Unavailable(format!(
                "Git {} could not establish an integration result",
                args[0]
            )));
        }
        Ok(bytes)
    }

    pub fn restore(&self, pack: &[u8]) -> Result<(), OperationError> {
        self.required(&["index-pack", "--stdin"], pack, 1024 * 1024)?;
        Ok(())
    }

    pub fn check_commit(&self, head: &str) -> Result<(), OperationError> {
        self.oid(head)?;
        self.required(
            &["cat-file", "-e", &format!("{head}^{{commit}}")],
            &[],
            1024,
        )?;
        Ok(())
    }

    pub fn materialize(
        &self,
        snapshot: &SourceSnapshot,
        captured: Option<&RetainedGitCapture>,
        name: &str,
    ) -> Result<String, OperationError> {
        snapshot
            .validate()
            .map_err(|e| OperationError::Invalid(e.to_string()))?;
        let head = if let Some(captured) = captured {
            if captured.receipt.source.snapshot != *snapshot {
                return Err(OperationError::Invalid("capture source mismatch".into()));
            }
            self.oid(&captured.receipt.parent_commit)?;
            capture::materialize(self.private.path(), captured, name)?
        } else if snapshot.source_state == SourceState::Committed {
            snapshot.revision.clone()
        } else {
            return Err(OperationError::Unavailable(
                "working-tree integration requires an explicit retained Git capture".into(),
            ));
        };
        self.check_commit(&head)?;
        Ok(head)
    }

    pub fn merge(
        &self,
        current: &str,
        member: &str,
    ) -> Result<(GitMergeObservation, Option<String>), OperationError> {
        self.oid(current)?;
        self.oid(member)?;
        let observation = merge_commits(self.private.path(), current, member)?;
        if observation.status != "clean" {
            return Ok((observation, None));
        }
        let tree =
            observation.details.lines().next().ok_or_else(|| {
                OperationError::Unavailable("Git omitted its integrated tree".into())
            })?;
        self.oid(tree)?;
        // Fixed attribution/time and explicit parents make restart IDs deterministic.
        let extra_parent = if current == member {
            String::new()
        } else {
            format!("parent {member}\n")
        };
        let body=format!("tree {tree}\nparent {current}\n{extra_parent}author Entmoot <entmoot@invalid> 1 +0000\ncommitter Entmoot <entmoot@invalid> 1 +0000\n\nEntmoot ordered integration\n");
        let bytes = self.required(
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            body.as_bytes(),
            1024,
        )?;
        let commit = String::from_utf8(bytes)
            .map_err(|e| OperationError::Invalid(e.to_string()))?
            .trim()
            .to_owned();
        self.check_commit(&commit)?;
        Ok((observation, Some(commit)))
    }

    /// Non-thin packs contain only newly reachable objects; originals remain in
    /// the explicit source repository and earlier private packs remain pinned.
    pub fn delta(
        &self,
        head: &str,
        exclude: &[String],
        limit: u64,
    ) -> Result<Vec<u8>, OperationError> {
        self.oid(head)?;
        let mut revisions = format!("{head}\n");
        for excluded in exclude {
            self.oid(excluded)?;
            revisions.push_str(&format!("^{excluded}\n"));
        }
        self.required(
            &[
                "pack-objects",
                "--stdout",
                "--revs",
                "--threads=1",
                "--compression=6",
            ],
            revisions.as_bytes(),
            limit,
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn committed_binary_delete_and_unicode_changes_survive_private_pack_restoration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let command = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        command(&["init", "-q"]);
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            command(&["config", key, value]);
        }
        std::fs::write(root.join("binary.bin"), [0, 1, 2, 0, 255]).unwrap();
        std::fs::write(root.join("delete.txt"), "remove this\n").unwrap();
        std::fs::write(root.join("tail.py"), "tail = 0\r\n").unwrap();
        command(&["add", "."]);
        command(&["commit", "-qm", "base"]);
        let base = command(&["rev-parse", "HEAD"]);
        std::fs::write(root.join("binary.bin"), [0, 9, 8, 0, 254]).unwrap();
        std::fs::remove_file(root.join("delete.txt")).unwrap();
        std::fs::write(root.join("雪.txt"), "new UTF-8 path\r\n").unwrap();
        command(&["add", "."]);
        command(&["commit", "-qm", "binary deletion and addition"]);
        let first = command(&["rev-parse", "HEAD"]);
        command(&["checkout", "-qb", "second", &base]);
        std::fs::write(root.join("tail.py"), "tail = 2\r\n").unwrap();
        command(&["add", "."]);
        command(&["commit", "-qm", "independent tail"]);
        let second = command(&["rev-parse", "HEAD"]);
        let snapshot = |revision: &str| SourceSnapshot {
            repository_id: "fixture/committed".into(),
            revision: revision.into(),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let scratch = temp.path().join("private");
        let git = GitIntegration::open(&root, &scratch, 40).unwrap();
        let initial = git.materialize(&snapshot(&base), None, "initial").unwrap();
        let member = git.materialize(&snapshot(&first), None, "first").unwrap();
        let (report, prefix) = git.merge(&initial, &member).unwrap();
        assert_eq!(report.status, "clean");
        let prefix = prefix.unwrap();
        let pack = git.delta(&prefix, &[base, first], 1024 * 1024).unwrap();
        drop(git);
        let git = GitIntegration::open(&root, &scratch, 40).unwrap();
        git.restore(&pack).unwrap();
        let member = git.materialize(&snapshot(&second), None, "second").unwrap();
        let (report, _) = git.merge(&prefix, &member).unwrap();
        assert_eq!(report.status, "clean");
        // Independently stage the expected union on the second branch.
        std::fs::write(root.join("binary.bin"), [0, 9, 8, 0, 254]).unwrap();
        std::fs::remove_file(root.join("delete.txt")).unwrap();
        std::fs::write(root.join("雪.txt"), "new UTF-8 path\r\n").unwrap();
        command(&["add", "."]);
        assert_eq!(
            report.details.lines().next().unwrap(),
            command(&["write-tree"])
        );
    }
}
