//! Public wire contracts for native Entmoot knowledge exchange.
//!
//! # Stability
//!
//! The manifest and visibility-page envelopes are public interchange contracts.
//! Segment payloads remain homogeneous and are identified by their own record
//! schema; this module does not introduce a universal untyped fact envelope.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    hash_parts, ContentDigest, IdParseError, PublicationId, ScopeCompleteness, ScopeUniverse,
    SourceSnapshot, SourceState,
};

pub const KNOWLEDGE_PUBLICATION_SCHEMA: &str = "entmoot.knowledge.publication.v1";
pub const KNOWLEDGE_VISIBILITY_PAGE_SCHEMA: &str = "entmoot.knowledge.visibility_page.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgePublicationMode {
    Full,
    Delta,
}

impl KnowledgePublicationMode {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Delta => "delta",
        }
    }
}

/// One homogeneous, independently digest-verified record segment.
///
/// `fact_family` is the logical record family (for example
/// `entmoot.relation_claim`), while `record_schema_ref` identifies the exact
/// typed record contract. Payload transport
/// is supplied by the exchange capability and is not encoded as a local path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeSegmentDescriptor {
    pub fact_family: String,
    pub record_schema_ref: String,
    pub content_digest: ContentDigest,
    pub record_count: u64,
}

/// Identity of the component assembling a publication.
///
/// This is deliberately separate from fact-level producer provenance: an
/// emitter packages records but does not become the source of their claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicationEmitter {
    pub emitter_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation: Option<String>,
}

/// Immutable native publication manifest for Entmoot-to-Entmoot interchange.
///
/// Provider-native artifacts do not use this envelope; they remain governed by
/// `ArtifactManifest` and `ArtifactAdapter`. Record provenance is preserved in
/// the referenced typed records and is not replaced by `emitter`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePublicationManifest {
    pub schema_version: String,
    pub publication_id: PublicationId,
    pub mode: KnowledgePublicationMode,
    pub snapshot: SourceSnapshot,
    pub scope: ScopeUniverse,
    pub emitter: PublicationEmitter,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_publication_id: Option<PublicationId>,
    /// Canonical order is `(fact_family, record_schema_ref)` with one segment per
    /// fact family. This keeps publication identity independent of arrival order.
    pub segments: Vec<KnowledgeSegmentDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeVisibility {
    Visible,
    Inactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeVisibilityReason {
    Published,
    OwnerRemoved,
    DependencyChanged,
    Superseded,
}

/// Visibility transition for an immutable record. Inactive is not deletion or
/// refutation; historical publications and their provenance remain addressable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeVisibilityEvent {
    pub publication_id: PublicationId,
    pub snapshot: SourceSnapshot,
    pub fact_family: String,
    pub record_id: String,
    pub visibility: KnowledgeVisibility,
    pub reason: KnowledgeVisibilityReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owner_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseding_publication_id: Option<PublicationId>,
}

/// Resumable egress page. Cursors are opaque transport state and never semantic
/// identity; stable record and publication IDs remain the portable references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeVisibilityPage {
    pub schema_version: String,
    pub repository_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<String>,
    pub next_cursor: String,
    pub events: Vec<KnowledgeVisibilityEvent>,
    pub truncated: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KnowledgeExchangeValidationError {
    #[error("unsupported knowledge exchange schema_version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid publication id: {0}")]
    InvalidPublicationId(IdParseError),
    #[error("publication_id does not match the manifest")]
    PublicationIdMismatch,
    #[error("full publication must not name base_publication_id")]
    FullHasBase,
    #[error("delta publication requires base_publication_id")]
    DeltaMissingBase,
    #[error("emitter_id must be non-empty")]
    EmptyEmitter,
    #[error("emitter optional field `{0}` must be non-empty when present")]
    EmptyEmitterField(&'static str),
    #[error("knowledge segments must be canonically sorted with unique fact families")]
    NonCanonicalSegments,
    #[error("knowledge segment field `{field}` must be non-empty")]
    EmptySegmentField { field: &'static str },
    #[error("invalid knowledge segment content digest: {0}")]
    InvalidSegmentDigest(IdParseError),
    #[error("repository_id must be non-empty")]
    EmptyRepositoryId,
    #[error("visibility event repository does not match visibility-page repository")]
    RepositoryMismatch,
    #[error("change-page cursor field `{0}` must be non-empty when present")]
    EmptyCursor(&'static str),
    #[error("visibility event field `{0}` must be non-empty")]
    EmptyEventField(&'static str),
    #[error("visibility event `{0}` must be sorted, unique, and non-empty")]
    NonCanonicalEventIds(&'static str),
    #[error("visibility event reason is inconsistent with its state or causes")]
    InvalidVisibilityTransition,
    #[error(transparent)]
    Snapshot(#[from] crate::SnapshotValidationError),
    #[error(transparent)]
    Scope(#[from] crate::ScopeValidationError),
}

impl KnowledgePublicationManifest {
    pub fn build(
        mode: KnowledgePublicationMode,
        snapshot: SourceSnapshot,
        scope: ScopeUniverse,
        emitter: PublicationEmitter,
        base_publication_id: Option<PublicationId>,
        mut segments: Vec<KnowledgeSegmentDescriptor>,
    ) -> Result<Self, KnowledgeExchangeValidationError> {
        segments.sort_by(|left, right| {
            (&left.fact_family, &left.record_schema_ref)
                .cmp(&(&right.fact_family, &right.record_schema_ref))
        });
        let mut manifest = Self {
            schema_version: KNOWLEDGE_PUBLICATION_SCHEMA.into(),
            publication_id: PublicationId::from_trusted(String::new()),
            mode,
            snapshot,
            scope,
            emitter,
            base_publication_id,
            segments,
        };
        manifest.publication_id = manifest.derive_publication_id();
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn derive_publication_id(&self) -> PublicationId {
        let mut parts = vec![
            "knowledge_publication".to_owned(),
            KNOWLEDGE_PUBLICATION_SCHEMA.to_owned(),
            self.mode.wire_name().to_owned(),
            self.emitter.emitter_id.clone(),
            self.emitter.version.clone().unwrap_or_default(),
            self.emitter.implementation.clone().unwrap_or_default(),
            self.snapshot.repository_id.clone(),
            self.snapshot.revision.clone(),
            source_state_name(self.snapshot.source_state).to_owned(),
            self.snapshot.snapshot_digest.clone().unwrap_or_default(),
            self.snapshot.estate_id.clone().unwrap_or_default(),
            self.scope.scope_universe_id.clone(),
            self.scope.selector_language_version.clone(),
            self.scope.profile.clone(),
            completeness_name(self.scope.completeness).to_owned(),
            self.scope
                .observed_document_set_digest
                .clone()
                .unwrap_or_default(),
            self.base_publication_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            self.scope.languages.len().to_string(),
        ];
        parts.extend(self.scope.languages.iter().cloned());
        parts.push(self.scope.included_paths.len().to_string());
        parts.extend(self.scope.included_paths.iter().cloned());
        parts.push(self.scope.excluded_paths.len().to_string());
        parts.extend(self.scope.excluded_paths.iter().cloned());
        parts.push(self.scope.producer_skips.len().to_string());
        parts.extend(self.scope.producer_skips.iter().cloned());
        parts.push(self.segments.len().to_string());
        for segment in &self.segments {
            parts.push(segment.fact_family.clone());
            parts.push(segment.record_schema_ref.clone());
            parts.push(segment.content_digest.to_string());
            parts.push(segment.record_count.to_string());
        }
        let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
        PublicationId::from_trusted(hash_parts(&refs))
    }

    pub fn validate(&self) -> Result<(), KnowledgeExchangeValidationError> {
        if self.schema_version != KNOWLEDGE_PUBLICATION_SCHEMA {
            return Err(KnowledgeExchangeValidationError::UnsupportedVersion(
                self.schema_version.clone(),
            ));
        }
        self.snapshot.validate()?;
        self.scope.validate()?;
        PublicationId::parse(self.publication_id.to_string())
            .map_err(KnowledgeExchangeValidationError::InvalidPublicationId)?;
        if let Some(base) = &self.base_publication_id {
            PublicationId::parse(base.to_string())
                .map_err(KnowledgeExchangeValidationError::InvalidPublicationId)?;
        }
        match (self.mode, self.base_publication_id.is_some()) {
            (KnowledgePublicationMode::Full, true) => {
                return Err(KnowledgeExchangeValidationError::FullHasBase)
            }
            (KnowledgePublicationMode::Delta, false) => {
                return Err(KnowledgeExchangeValidationError::DeltaMissingBase)
            }
            _ => {}
        }
        validate_emitter(&self.emitter)?;
        validate_segments(&self.segments)?;
        if self.publication_id != self.derive_publication_id() {
            return Err(KnowledgeExchangeValidationError::PublicationIdMismatch);
        }
        Ok(())
    }
}

impl KnowledgeVisibilityPage {
    pub fn validate(&self) -> Result<(), KnowledgeExchangeValidationError> {
        if self.schema_version != KNOWLEDGE_VISIBILITY_PAGE_SCHEMA {
            return Err(KnowledgeExchangeValidationError::UnsupportedVersion(
                self.schema_version.clone(),
            ));
        }
        if self.repository_id.trim().is_empty() {
            return Err(KnowledgeExchangeValidationError::EmptyRepositoryId);
        }
        if self
            .after_cursor
            .as_deref()
            .is_some_and(|cursor| cursor.trim().is_empty())
        {
            return Err(KnowledgeExchangeValidationError::EmptyCursor(
                "after_cursor",
            ));
        }
        if self.next_cursor.trim().is_empty() {
            return Err(KnowledgeExchangeValidationError::EmptyCursor("next_cursor"));
        }
        for event in &self.events {
            PublicationId::parse(event.publication_id.to_string())
                .map_err(KnowledgeExchangeValidationError::InvalidPublicationId)?;
            event.snapshot.validate()?;
            if event.snapshot.repository_id != self.repository_id {
                return Err(KnowledgeExchangeValidationError::RepositoryMismatch);
            }
            for (field, value) in [
                ("fact_family", event.fact_family.as_str()),
                ("record_id", event.record_id.as_str()),
            ] {
                if value.trim().is_empty() {
                    return Err(KnowledgeExchangeValidationError::EmptyEventField(field));
                }
            }
            validate_event_ids("owner_ids", &event.owner_ids)?;
            validate_event_ids("dependency_ids", &event.dependency_ids)?;
            if let Some(id) = &event.superseding_publication_id {
                PublicationId::parse(id.to_string())
                    .map_err(KnowledgeExchangeValidationError::InvalidPublicationId)?;
            }
            let valid = match (event.visibility, event.reason) {
                (KnowledgeVisibility::Visible, KnowledgeVisibilityReason::Published) => {
                    event.dependency_ids.is_empty() && event.superseding_publication_id.is_none()
                }
                (KnowledgeVisibility::Inactive, KnowledgeVisibilityReason::OwnerRemoved) => {
                    !event.owner_ids.is_empty()
                        && event.dependency_ids.is_empty()
                        && event.superseding_publication_id.is_none()
                }
                (KnowledgeVisibility::Inactive, KnowledgeVisibilityReason::DependencyChanged) => {
                    event.owner_ids.is_empty()
                        && !event.dependency_ids.is_empty()
                        && event.superseding_publication_id.is_none()
                }
                (KnowledgeVisibility::Inactive, KnowledgeVisibilityReason::Superseded) => {
                    event.owner_ids.is_empty()
                        && event.dependency_ids.is_empty()
                        && event.superseding_publication_id.is_some()
                }
                _ => false,
            };
            if !valid {
                return Err(KnowledgeExchangeValidationError::InvalidVisibilityTransition);
            }
        }
        Ok(())
    }
}

fn validate_emitter(emitter: &PublicationEmitter) -> Result<(), KnowledgeExchangeValidationError> {
    if emitter.emitter_id.trim().is_empty() {
        return Err(KnowledgeExchangeValidationError::EmptyEmitter);
    }
    for (field, value) in [
        ("version", emitter.version.as_deref()),
        ("implementation", emitter.implementation.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(KnowledgeExchangeValidationError::EmptyEmitterField(field));
        }
    }
    Ok(())
}

fn validate_segments(
    segments: &[KnowledgeSegmentDescriptor],
) -> Result<(), KnowledgeExchangeValidationError> {
    let mut prior: Option<&str> = None;
    for segment in segments {
        if segment.fact_family.trim().is_empty() {
            return Err(KnowledgeExchangeValidationError::EmptySegmentField {
                field: "fact_family",
            });
        }
        if segment.record_schema_ref.trim().is_empty() {
            return Err(KnowledgeExchangeValidationError::EmptySegmentField {
                field: "record_schema_ref",
            });
        }
        ContentDigest::parse(segment.content_digest.to_string())
            .map_err(KnowledgeExchangeValidationError::InvalidSegmentDigest)?;
        if prior.is_some_and(|value| value >= segment.fact_family.as_str()) {
            return Err(KnowledgeExchangeValidationError::NonCanonicalSegments);
        }
        prior = Some(&segment.fact_family);
    }
    Ok(())
}

fn validate_event_ids(
    field: &'static str,
    values: &[String],
) -> Result<(), KnowledgeExchangeValidationError> {
    if values.iter().any(|value| value.trim().is_empty()) || !is_strictly_sorted(values) {
        return Err(KnowledgeExchangeValidationError::NonCanonicalEventIds(
            field,
        ));
    }
    Ok(())
}

fn is_strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

const fn source_state_name(state: SourceState) -> &'static str {
    match state {
        SourceState::Committed => "committed",
        SourceState::WorkingTree => "working_tree",
    }
}

const fn completeness_name(completeness: ScopeCompleteness) -> &'static str {
    match completeness {
        ScopeCompleteness::CompleteRelativeToUniverse => "complete_relative_to_universe",
        ScopeCompleteness::Partial => "partial",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SourceSnapshot {
        SourceSnapshot {
            repository_id: "github.com/acme/app".into(),
            revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        }
    }

    fn scope() -> ScopeUniverse {
        ScopeUniverse::from_selectors(
            "full",
            vec!["rust".into()],
            Vec::new(),
            Vec::new(),
            ScopeCompleteness::Partial,
            None,
        )
    }

    fn emitter() -> PublicationEmitter {
        PublicationEmitter {
            emitter_id: "entmoot.local".into(),
            version: Some("1".into()),
            implementation: None,
        }
    }

    fn segment(fact_family: &str, byte: char) -> KnowledgeSegmentDescriptor {
        KnowledgeSegmentDescriptor {
            fact_family: fact_family.into(),
            record_schema_ref: format!("{fact_family}.v1"),
            content_digest: ContentDigest::from_trusted(format!(
                "blake3:{}",
                byte.to_string().repeat(64)
            )),
            record_count: 1,
        }
    }

    #[test]
    fn publication_build_is_order_independent() {
        let left = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            vec![
                segment("entmoot.relation_claim", 'b'),
                segment("entmoot.assertion", 'a'),
            ],
        )
        .expect("valid publication");
        let right = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            vec![
                segment("entmoot.assertion", 'a'),
                segment("entmoot.relation_claim", 'b'),
            ],
        )
        .expect("valid publication");
        assert_eq!(left.publication_id, right.publication_id);
        assert_eq!(left.segments, right.segments);
    }

    #[test]
    fn delta_requires_a_base_publication() {
        let error = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Delta,
            snapshot(),
            scope(),
            emitter(),
            None,
            Vec::new(),
        )
        .expect_err("base must be present");
        assert_eq!(error, KnowledgeExchangeValidationError::DeltaMissingBase);
    }

    #[test]
    fn changed_segment_changes_publication_identity() {
        let left = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            vec![segment("entmoot.relation_claim", 'a')],
        )
        .expect("valid publication");
        let right = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            vec![segment("entmoot.relation_claim", 'b')],
        )
        .expect("valid publication");
        assert_ne!(left.publication_id, right.publication_id);
    }

    #[test]
    fn publication_rejects_identity_tampering() {
        let mut publication = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            vec![segment("entmoot.relation_claim", 'a')],
        )
        .expect("valid publication");
        publication.segments[0].record_count = 2;
        assert_eq!(
            publication.validate(),
            Err(KnowledgeExchangeValidationError::PublicationIdMismatch)
        );
    }

    #[test]
    fn visibility_page_rejects_cross_repository_events() {
        let publication = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            Vec::new(),
        )
        .expect("valid publication");
        let page = KnowledgeVisibilityPage {
            schema_version: KNOWLEDGE_VISIBILITY_PAGE_SCHEMA.into(),
            repository_id: "github.com/other/app".into(),
            after_cursor: None,
            next_cursor: "cursor:1".into(),
            events: vec![KnowledgeVisibilityEvent {
                publication_id: publication.publication_id,
                snapshot: snapshot(),
                fact_family: "entmoot.relation_claim".into(),
                record_id: "claim:1".into(),
                visibility: KnowledgeVisibility::Inactive,
                reason: KnowledgeVisibilityReason::DependencyChanged,
                owner_ids: Vec::new(),
                dependency_ids: vec!["dependency:1".into()],
                superseding_publication_id: None,
            }],
            truncated: false,
        };
        assert_eq!(
            page.validate(),
            Err(KnowledgeExchangeValidationError::RepositoryMismatch)
        );
    }

    #[test]
    fn inactive_owner_event_requires_an_owner_cause() {
        let publication = KnowledgePublicationManifest::build(
            KnowledgePublicationMode::Full,
            snapshot(),
            scope(),
            emitter(),
            None,
            Vec::new(),
        )
        .expect("valid publication");
        let page = KnowledgeVisibilityPage {
            schema_version: KNOWLEDGE_VISIBILITY_PAGE_SCHEMA.into(),
            repository_id: snapshot().repository_id,
            after_cursor: None,
            next_cursor: "cursor:1".into(),
            events: vec![KnowledgeVisibilityEvent {
                publication_id: publication.publication_id,
                snapshot: snapshot(),
                fact_family: "entmoot.relation_claim".into(),
                record_id: "relation-claim:1".into(),
                visibility: KnowledgeVisibility::Inactive,
                reason: KnowledgeVisibilityReason::OwnerRemoved,
                owner_ids: Vec::new(),
                dependency_ids: Vec::new(),
                superseding_publication_id: None,
            }],
            truncated: false,
        };
        assert_eq!(
            page.validate(),
            Err(KnowledgeExchangeValidationError::InvalidVisibilityTransition)
        );
    }
}
