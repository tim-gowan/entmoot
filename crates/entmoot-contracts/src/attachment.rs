//! Code-plane attachment projection (multi-dimension).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::foundational::LocationRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentDisposition {
    Attached,
    Excluded,
    Unresolved,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentTargetKind {
    CallSite,
    Callable,
    File,
    Module,
    Resource,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentFreshness {
    Current,
    StaleSnapshot,
    /// Current inventory ExactBodyId differs from the attachment's expected body (INV-16).
    StaleBody,
    DegradedOverride,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentReason {
    ExactPortSpan,
    ExactRegionOverlap,
    EnclosingCallableSpan,
    EnclosingResourceSpan,
    ExactFile,
    FileFallback,
    ProviderSymbolResolve,
    ScopeExcludeMatch,
    PathMissing,
    RegionInvalid,
    ArtifactIdentityMismatch,
    CoordinateEncodingUnknown,
    CoordinateEndUnknown,
    SourceDigestUnavailable,
    SourceDigestMismatch,
    RevisionMismatch,
    LanguageUnsupported,
    SymbolSpanDisagreement,
    AmbiguousCandidates,
    ExactBodyMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentTarget {
    /// Syntax port within a callsite target; never a parameter or transfer edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<CallPort>,
    pub kind: AttachmentTargetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CallPort {
    Callee,
    Receiver,
    Argument { position: usize },
    Result,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationAttachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_span: Option<crate::VerifiedSourceSpan>,
    pub attachment_id: String,
    pub artifact_id: String,
    pub location_id: String,
    pub disposition: AttachmentDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_target: Option<AttachmentTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_targets: Vec<AttachmentTarget>,
    pub freshness: AttachmentFreshness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<AttachmentReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// Directional binding to the ExactBodyId observed/expected at attach time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_ref: Option<LocationRef>,
}

impl LocationAttachment {
    /// Validate the projection's relationship to its retained provider location.
    /// The repository additionally checks the digest against its source snapshot.
    pub fn validate_location(&self, location: &crate::LocationRecord) -> Result<(), String> {
        if self.location_id != location.location_id || self.artifact_id != location.artifact_id {
            return Err("attachment and location have different evidence identities".into());
        }
        let attached = self.disposition == AttachmentDisposition::Attached;
        if attached != self.selected_target.is_some() {
            return Err("attachment disposition disagrees with selected target".into());
        }
        if (attached && !location.extent.is_file()) != self.verified_span.is_some() {
            return Err(
                "attached regions require a verified span; other outcomes cannot carry one".into(),
            );
        }
        if location.extent.is_file()
            && self
                .selected_target
                .as_ref()
                .is_some_and(|target| target.kind != AttachmentTargetKind::File)
        {
            return Err("file-grain coordinates cannot establish a descendant attachment".into());
        }
        if let Some(verified) = &self.verified_span {
            crate::ContentDigest::parse(verified.source_digest.as_str())
                .map_err(|error| error.to_string())?;
            if verified.span.start_byte > verified.span.end_byte {
                return Err("verified source span is reversed".into());
            }
            if let Some(crate::SourceRegion::Bytes { span }) = location.extent.region() {
                if span != &verified.span {
                    return Err("verified span disagrees with asserted byte coordinates".into());
                }
            }
        }
        Ok(())
    }
}
