//! Scheme-aware addresses (foundational). Not a universal erased ID.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::ids::{AddressId, ExactBodyId, IdParseError, PolicyId, StructuralHash};
use crate::{is_immutable_revision, normalize_repo_path, InvalidRepoPath};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressValidationError {
    #[error("invalid {scheme} address identity: {source}")]
    InvalidId {
        scheme: &'static str,
        source: IdParseError,
    },
    #[error("{0} address field must be non-empty")]
    EmptyField(&'static str),
    #[error("invalid repository path: {0}")]
    InvalidPath(InvalidRepoPath),
    #[error("invalid package-url: {0}")]
    InvalidPurl(String),
    #[error("package-url is not in canonical form: expected {0}")]
    NonCanonicalPurl(String),
    #[error("repository path is not in canonical POSIX form: {0}")]
    NonCanonicalPath(String),
    #[error("repo_path end_byte precedes start_byte")]
    InvertedSpan,
    #[error("git_oid must be a full immutable hexadecimal object id")]
    InvalidGitOid,
}

/// Scheme-specific address. Verification and canonicalization stay scheme-explicit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scheme", rename_all = "snake_case")]
pub enum Address {
    CanonicalForm {
        digest: crate::ContentDigest,
    },
    /// LogicId / LAH: policy + structural hash (ADR-0024).
    LogicId {
        policy_id: PolicyId,
        structural_hash: StructuralHash,
    },
    /// Exact observed unit bytes.
    ExactBody {
        exact_body_id: ExactBodyId,
    },
    /// Occurrence / logic-address envelope string.
    LogicAddress {
        logic_address: String,
    },
    /// Compact occurrence key (bare hex of logic_address).
    AddressId {
        address_id: String,
    },
    /// Repository-relative path + optional region (navigation without body binding).
    RepoPath {
        repository_id: Option<String>,
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_byte: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_byte: Option<u32>,
    },
    /// Git object id (hex); never a LogicId.
    GitOid {
        oid: String,
    },
    /// Package-url.
    Purl {
        purl: String,
    },
    /// Producer-independent logical identity for a configuration or runtime resource.
    ///
    /// Free-string `resource_type` is intentional (for example Kubernetes Gateway
    /// API types or Envoy listeners). Gateways and proxies use ordinary resource
    /// addresses when an artifact states them—no `AuthzGateway` address scheme.
    Resource {
        resource_system: String,
        resource_namespace: String,
        resource_type: String,
        logical_name: String,
    },
    /// Opaque producer-native address.
    Opaque {
        system: String,
        value: String,
    },
}

impl Address {
    /// Stable wire scheme name used by producer capability negotiation.
    pub fn scheme(&self) -> &'static str {
        match self {
            Self::CanonicalForm { .. } => "canonical_form",
            Self::LogicId { .. } => "logic_id",
            Self::ExactBody { .. } => "exact_body",
            Self::LogicAddress { .. } => "logic_address",
            Self::AddressId { .. } => "address_id",
            Self::RepoPath { .. } => "repo_path",
            Self::GitOid { .. } => "git_oid",
            Self::Purl { .. } => "purl",
            Self::Resource { .. } => "resource",
            Self::Opaque { .. } => "opaque",
        }
    }

    /// Validate an address received across a trust boundary.
    ///
    /// Constructors used by trusted hash producers remain intentionally cheap;
    /// deserialized values must pass this check before entering durable records.
    pub fn validate(&self) -> Result<(), AddressValidationError> {
        match self {
            Self::CanonicalForm { digest } => {
                crate::ContentDigest::parse(digest.as_str()).map_err(|source| {
                    AddressValidationError::InvalidId {
                        scheme: "canonical_form",
                        source,
                    }
                })?;
            }
            Self::LogicId {
                policy_id,
                structural_hash,
            } => {
                PolicyId::parse(policy_id.as_str()).map_err(|source| {
                    AddressValidationError::InvalidId {
                        scheme: "logic_id.policy_id",
                        source,
                    }
                })?;
                StructuralHash::parse(structural_hash.as_str()).map_err(|source| {
                    AddressValidationError::InvalidId {
                        scheme: "logic_id.structural_hash",
                        source,
                    }
                })?;
            }
            Self::ExactBody { exact_body_id } => {
                ExactBodyId::parse(exact_body_id.as_str()).map_err(|source| {
                    AddressValidationError::InvalidId {
                        scheme: "exact_body",
                        source,
                    }
                })?;
            }
            Self::LogicAddress { logic_address } => {
                require_nonempty(logic_address, "logic_address")?;
            }
            Self::AddressId { address_id } => {
                AddressId::parse(address_id).map_err(|source| {
                    AddressValidationError::InvalidId {
                        scheme: "address_id",
                        source,
                    }
                })?;
            }
            Self::RepoPath {
                repository_id,
                path,
                start_byte,
                end_byte,
            } => {
                if let Some(repository_id) = repository_id {
                    require_nonempty(repository_id, "repo_path.repository_id")?;
                }
                let normalized =
                    normalize_repo_path(path).map_err(AddressValidationError::InvalidPath)?;
                if normalized != *path {
                    return Err(AddressValidationError::NonCanonicalPath(path.clone()));
                }
                if matches!((start_byte, end_byte), (Some(start), Some(end)) if end < start) {
                    return Err(AddressValidationError::InvertedSpan);
                }
            }
            Self::GitOid { oid } => {
                if !is_immutable_revision(oid) {
                    return Err(AddressValidationError::InvalidGitOid);
                }
            }
            Self::Purl { purl } => {
                require_nonempty(purl, "purl")?;
                let parsed = packageurl::PackageUrl::from_str(purl)
                    .map_err(|error| AddressValidationError::InvalidPurl(error.to_string()))?;
                let canonical = parsed.to_string();
                if canonical != *purl {
                    return Err(AddressValidationError::NonCanonicalPurl(canonical));
                }
            }
            Self::Resource {
                resource_system,
                resource_namespace,
                resource_type,
                logical_name,
            } => {
                require_canonical_component(resource_system, "resource.resource_system")?;
                require_canonical_component(resource_namespace, "resource.resource_namespace")?;
                require_canonical_component(resource_type, "resource.resource_type")?;
                require_canonical_component(logical_name, "resource.logical_name")?;
            }
            Self::Opaque { system, value } => {
                require_nonempty(system, "opaque.system")?;
                require_nonempty(value, "opaque.value")?;
            }
        }
        Ok(())
    }
}

/// Parse and canonicalize a Package URL before it enters an identity-bearing record.
pub fn purl_address(value: &str) -> Result<Address, AddressValidationError> {
    let parsed = packageurl::PackageUrl::from_str(value)
        .map_err(|error| AddressValidationError::InvalidPurl(error.to_string()))?;
    let address = Address::Purl {
        purl: parsed.to_string(),
    };
    address.validate()?;
    Ok(address)
}

fn require_canonical_component(
    value: &str,
    field: &'static str,
) -> Result<(), AddressValidationError> {
    require_nonempty(value, field)?;
    if value.trim() != value || value.contains('\0') {
        return Err(AddressValidationError::EmptyField(field));
    }
    Ok(())
}

/// Construct and validate a producer-independent resource address.
pub fn resource_address(
    resource_system: impl Into<String>,
    resource_namespace: impl Into<String>,
    resource_type: impl Into<String>,
    logical_name: impl Into<String>,
) -> Result<Address, AddressValidationError> {
    let address = Address::Resource {
        resource_system: resource_system.into(),
        resource_namespace: resource_namespace.into(),
        resource_type: resource_type.into(),
        logical_name: logical_name.into(),
    };
    address.validate()?;
    Ok(address)
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), AddressValidationError> {
    if value.trim().is_empty() {
        Err(AddressValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

/// Thing a claim speaks about (address-bearing subject).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Subject {
    pub address: Address,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_paths_and_invalid_spans() {
        let noncanonical = Address::RepoPath {
            repository_id: Some("repo".into()),
            path: "src\\lib.rs".into(),
            start_byte: None,
            end_byte: None,
        };
        assert!(noncanonical.validate().is_err());

        let inverted = Address::RepoPath {
            repository_id: Some("repo".into()),
            path: "src/lib.rs".into(),
            start_byte: Some(8),
            end_byte: Some(4),
        };
        assert!(inverted.validate().is_err());
    }

    #[test]
    fn validates_typed_and_provider_addresses() {
        let valid = Address::AddressId {
            address_id: "a".repeat(64),
        };
        assert!(valid.validate().is_ok());

        let malformed = Address::AddressId {
            address_id: "not-an-id".into(),
        };
        assert!(malformed.validate().is_err());
        assert!(Address::Opaque {
            system: " ".into(),
            value: "value".into(),
        }
        .validate()
        .is_err());
    }

    #[test]
    fn package_urls_are_parsed_and_canonicalized() {
        let address = purl_address("pkg:npm/%40angular/animation@12.3.1").expect("valid PURL");
        assert!(address.validate().is_ok());
        assert!(purl_address("animation@12.3.1").is_err());
    }
}
