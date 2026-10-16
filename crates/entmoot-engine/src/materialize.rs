//! Bounded, confined source copies. Publication remains a separate operation.
use cap_std::{ambient_authority, fs::Dir};
use entmoot_contracts::SourceSubject;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum MaterializeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Subject(#[from] entmoot_contracts::SourceSubjectError),
    #[error("invalid dependency materialization: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy)]
pub struct CopyBudget {
    pub files: u64,
    pub bytes: u64,
    pub depth: usize,
}
impl Default for CopyBudget {
    fn default() -> Self {
        Self {
            files: 100_000,
            bytes: 1_073_741_824,
            depth: 128,
        }
    }
}

/// Copy into a new immutable directory below the selected dependencies directory.
/// Directory capabilities confine source traversal even if names change during a copy.
/// Symlinks, special files, overlapping trees and exhausted budgets are errors.
pub fn materialize(
    dependencies: &Path,
    subject: &SourceSubject,
    version: &str,
    source: &Path,
    metadata: &[u8],
    python_environment: Option<&Path>,
    mut budget: CopyBudget,
) -> Result<PathBuf, MaterializeError> {
    subject.validate()?;
    if !matches!(subject, SourceSubject::Package { .. }) {
        return Err(MaterializeError::Invalid(
            "a package subject is required".into(),
        ));
    }
    if version.is_empty()
        || version.len() > 256
        || version != version.trim()
        || version == "."
        || version == ".."
        || version
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return Err(MaterializeError::Invalid(
            "version must be a nonempty release, commit or alias label without path separators"
                .into(),
        ));
    }
    if budget.files == 0 || budget.bytes == 0 || budget.depth == 0 {
        return Err(MaterializeError::Invalid(
            "copy budgets must be positive".into(),
        ));
    }
    if metadata.len() > 1_048_576 {
        return Err(MaterializeError::Invalid("metadata exceeds 1 MiB".into()));
    }
    serde_json::from_slice::<serde_json::Value>(metadata).map_err(io::Error::other)?;
    let source = fs::canonicalize(source)?;
    // Check existing ancestors before creating anything inside the source tree.
    let mut existing = dependencies;
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| {
            MaterializeError::Invalid("dependencies path has no existing parent".into())
        })?;
    }
    if fs::canonicalize(existing)?.starts_with(&source) {
        return Err(MaterializeError::Invalid(
            "source and destination overlap".into(),
        ));
    }
    fs::create_dir_all(dependencies)?;
    let dependencies = fs::canonicalize(dependencies)?;
    if source.starts_with(&dependencies) || dependencies.starts_with(&source) {
        return Err(MaterializeError::Invalid(
            "source and destination overlap".into(),
        ));
    }
    let directory = Dir::open_ambient_dir(&dependencies, ambient_authority())?;
    let mut lock_options = cap_std::fs::OpenOptions::new();
    lock_options.read(true).write(true).create(true);
    let lock = directory
        .open_with("materialize.lock", &lock_options)?
        .into_std();
    lock.try_lock().map_err(|e| {
        MaterializeError::Invalid(format!("another materialization is active: {e}"))
    })?;
    // Opaque labels never become paths; the metadata preserves their exact meaning.
    let key = entmoot_contracts::blake3_hex(
        &serde_json::to_vec(&(subject, version)).map_err(io::Error::other)?,
    )
    .replace(':', "-");
    let destination = dependencies.join(&key);
    if directory.symlink_metadata(&key).is_ok() {
        return Err(MaterializeError::Invalid(
            "materialization already exists".into(),
        ));
    }
    let staging = tempfile::Builder::new()
        .prefix(".copy-")
        .tempdir_in(&dependencies)?;
    let input = Dir::open_ambient_dir(&source, ambient_authority())?;
    let output = Dir::open_ambient_dir(staging.path(), ambient_authority())?;
    copy_tree(&input, &output, &mut budget, 0)?;
    output.write("entmoot.subject.json", metadata)?;
    if let Some(environment) = python_environment {
        let mut file = fs::File::open(environment)?;
        let mut bytes = Vec::new();
        (&mut file).take(1_048_577).read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            return Err(MaterializeError::Invalid(
                "Python environment exceeds 1 MiB".into(),
            ));
        }
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)?;
        output.write("python_environment.json", bytes)?;
    }
    drop(output);
    let stage_name = staging
        .path()
        .file_name()
        .ok_or_else(|| io::Error::other("missing stage name"))?;
    directory.rename(stage_name, &directory, &key)?;
    Ok(destination)
}

fn copy_tree(
    input: &Dir,
    output: &Dir,
    budget: &mut CopyBudget,
    depth: usize,
) -> Result<(), MaterializeError> {
    if depth >= budget.depth {
        return Err(MaterializeError::Invalid("copy depth exhausted".into()));
    }
    for entry in input.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
            return Err(MaterializeError::Invalid(format!(
                "symlink or special file: {}",
                name.to_string_lossy()
            )));
        }
        budget.files = budget
            .files
            .checked_sub(1)
            .ok_or_else(|| MaterializeError::Invalid("copy entry budget exhausted".into()))?;
        if kind.is_dir() {
            output.create_dir(&name)?;
            copy_tree(
                &input.open_dir(&name)?,
                &output.open_dir(&name)?,
                budget,
                depth + 1,
            )?;
        } else {
            let mut source = input.open(&name)?;
            if !source.metadata()?.is_file() {
                return Err(MaterializeError::Invalid("source changed type".into()));
            }
            let mut target = output.create(&name)?;
            let copied = io::copy(
                &mut (&mut source).take(budget.bytes.saturating_add(1)),
                &mut target,
            )?;
            budget.bytes = budget
                .bytes
                .checked_sub(copied)
                .ok_or_else(|| MaterializeError::Invalid("copy byte budget exhausted".into()))?;
            // Preserve ordinary permissions but never set-id/sticky privilege bits.
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt;
                target.set_permissions(cap_std::fs::Permissions::from_mode(
                    source.metadata()?.permissions().mode() & 0o777,
                ))?;
            }
            target.sync_all()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confines_labels_and_rolls_back_failed_copies() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("app.py"), b"def leaf(): pass\n").unwrap();
        let deps = temp.path().join("deps");
        let subject = SourceSubject::package("pkg:pypi/example").unwrap();
        for version in [
            "../../escaped",
            "C:\\escape",
            "/escape",
            "..",
            "",
            "a/b",
            "a:b",
        ] {
            assert!(materialize(
                &deps,
                &subject,
                version,
                &source,
                b"{}",
                None,
                CopyBudget::default()
            )
            .is_err());
        }
        assert!(!deps.exists());
        let small = CopyBudget {
            bytes: 2,
            ..Default::default()
        };
        assert!(materialize(&deps, &subject, "1", &source, b"{}", None, small).is_err());
        assert_eq!(fs::read_dir(&deps).unwrap().count(), 1); // only the coordination lock
        assert!(materialize(
            &source.join("nested"),
            &subject,
            "1",
            &source,
            b"{}",
            None,
            CopyBudget::default()
        )
        .is_err());
        assert!(!source.join("nested").exists());
        let result = materialize(
            &deps,
            &subject,
            "1",
            &source,
            b"{}",
            None,
            CopyBudget::default(),
        )
        .unwrap();
        assert_eq!(
            fs::read(result.join("app.py")).unwrap(),
            b"def leaf(): pass\n"
        );
        assert!(result.starts_with(fs::canonicalize(&deps).unwrap()));
        assert!(materialize(
            &deps,
            &subject,
            "1",
            &source,
            b"{}",
            None,
            CopyBudget::default()
        )
        .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_source_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", source.join("escape")).unwrap();
        assert!(materialize(
            &temp.path().join("deps"),
            &SourceSubject::package("pkg:pypi/example").unwrap(),
            "1",
            &source,
            b"{}",
            None,
            CopyBudget::default()
        )
        .is_err());
    }
}
