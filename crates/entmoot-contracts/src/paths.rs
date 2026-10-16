//! Repository-relative path normalization.

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("invalid scope path (absolute or traversal): {0}")]
pub struct InvalidRepoPath(pub String);

/// Normalize to repository-relative forward-slash form; reject absolute / `..`.
pub fn normalize_repo_path(path: &str) -> Result<String, InvalidRepoPath> {
    let trimmed = path.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return Err(InvalidRepoPath(path.into()));
    }
    if trimmed.starts_with('/') || trimmed.starts_with("//") {
        return Err(InvalidRepoPath(path.into()));
    }
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' {
        return Err(InvalidRepoPath(path.into()));
    }
    let mut parts = Vec::new();
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(InvalidRepoPath(path.into()));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(InvalidRepoPath(path.into()));
    }
    Ok(parts.join("/"))
}
