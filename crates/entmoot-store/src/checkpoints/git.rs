//! Retained Git bytes and validation; no Git process execution.
use crate::OperationError;
use entmoot_contracts::GitTreeCapture;
use std::collections::BTreeMap;
fn invalid(e: impl std::fmt::Display) -> OperationError {
    OperationError::Invalid(e.to_string())
}
#[derive(Debug)]
pub struct RetainedGitCapture {
    pub receipt: GitTreeCapture,
    pub blobs: BTreeMap<String, Vec<u8>>,
}

pub fn valid_path(path: &str) -> bool {
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
    pub fn validate(&self) -> Result<(), OperationError> {
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
