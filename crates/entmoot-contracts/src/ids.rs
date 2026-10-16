//! Typed identity wrappers enforcing digest wire rules.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::identity::DIGEST_PREFIX;

/// Prefixed structural or content digest (`blake3:<64-hex>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ContentDigest(String);

/// Prefixed structural hash (`blake3:<64-hex>`) — LogicId/LAH digest half under policy.
/// See ADR-0024 (renames historical `body_hash`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct StructuralHash(String);

/// Prefixed occurrence-scoped resolved-binding digest (`blake3:<64-hex>`).
/// Derived from Resolver artifacts; never rewrites [`StructuralHash`]. See ADR-0024.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ResolvedHash(String);

/// Prefixed digest of **observed source bytes** for a sealed unit (`blake3:<64-hex>`).
/// Distinct from [`StructuralHash`] (policy-normalized LogicId digest). See ADR-0002.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ExactBodyId(String);

/// Bare BLAKE3 hex policy fingerprint (no prefix).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PolicyId(String);

/// Bare BLAKE3 hex address id (no prefix).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AddressId(String);

/// Artifact instance id (bare hex from length-prefixed parts).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ArtifactId(String);

/// Attachment observation id (bare hex).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AttachmentId(String);

/// Finding observation id (bare hex).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct FindingId(String);

/// Native knowledge publication id (bare BLAKE3 hex from length-prefixed parts).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PublicationId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdParseError {
    Empty,
    MissingBlake3Prefix,
    UnexpectedPrefix,
    BadHexLength { expected: usize, actual: usize },
    NonHex,
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdParseError::Empty => write!(f, "empty identity value"),
            IdParseError::MissingBlake3Prefix => {
                write!(f, "digest missing `{DIGEST_PREFIX}` prefix")
            }
            IdParseError::UnexpectedPrefix => write!(f, "bare id must not include a digest prefix"),
            IdParseError::BadHexLength { expected, actual } => {
                write!(f, "expected {expected} hex chars, got {actual}")
            }
            IdParseError::NonHex => write!(f, "identity value contains non-hex characters"),
        }
    }
}

impl std::error::Error for IdParseError {}

fn validate_prefixed(value: &str) -> Result<(), IdParseError> {
    if value.is_empty() {
        return Err(IdParseError::Empty);
    }
    let Some(hex) = value.strip_prefix(DIGEST_PREFIX) else {
        return Err(IdParseError::MissingBlake3Prefix);
    };
    validate_bare_hex(hex, 64)
}

fn validate_bare_hex(value: &str, expected_len: usize) -> Result<(), IdParseError> {
    if value.is_empty() {
        return Err(IdParseError::Empty);
    }
    if value.starts_with(DIGEST_PREFIX) {
        return Err(IdParseError::UnexpectedPrefix);
    }
    if value.len() != expected_len {
        return Err(IdParseError::BadHexLength {
            expected: expected_len,
            actual: value.len(),
        });
    }
    if !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(IdParseError::NonHex);
    }
    Ok(())
}

macro_rules! impl_id {
    ($name:ident, $validate:expr) => {
        impl $name {
            /// Construct after validating wire rules (external / untrusted input).
            pub fn parse(value: impl Into<String>) -> Result<Self, IdParseError> {
                let value = value.into();
                ($validate)(&value)?;
                Ok(Self(value))
            }

            /// Construct from a trusted producer (hash output). Skips validation.
            pub fn from_trusted(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::from_trusted(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::from_trusted(value)
            }
        }
    };
}

impl_id!(ContentDigest, validate_prefixed);
impl_id!(StructuralHash, validate_prefixed);
impl_id!(ResolvedHash, validate_prefixed);
impl_id!(ExactBodyId, validate_prefixed);
impl_id!(PolicyId, |v: &str| validate_bare_hex(v, 64));
impl_id!(AddressId, |v: &str| validate_bare_hex(v, 64));
impl_id!(ArtifactId, |v: &str| validate_bare_hex(v, 64));
impl_id!(AttachmentId, |v: &str| validate_bare_hex(v, 64));
impl_id!(FindingId, |v: &str| validate_bare_hex(v, 64));
impl_id!(PublicationId, |v: &str| validate_bare_hex(v, 64));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_digest_requires_prefix() {
        assert!(ContentDigest::parse(
            "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_ok());
        assert!(ContentDigest::parse(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err());
    }

    #[test]
    fn policy_id_rejects_prefix() {
        assert!(PolicyId::parse(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_ok());
        assert!(PolicyId::parse(
            "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err());
    }

    #[test]
    fn structural_and_resolved_hashes_require_prefix() {
        let ok = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bare = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(StructuralHash::parse(ok).is_ok());
        assert!(StructuralHash::parse(bare).is_err());
        assert!(ResolvedHash::parse(ok).is_ok());
        assert!(ResolvedHash::parse(bare).is_err());
        assert!(ResolvedHash::parse(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err());
    }
}
