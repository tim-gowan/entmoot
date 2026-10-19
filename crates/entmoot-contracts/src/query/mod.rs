//! Transport-neutral contracts for deterministic queries and optional candidate retrieval.
//!
//! Candidate retrieval only narrows discovery. Durable predicates, authorization,
//! snapshot identity, and evidence resolution remain authoritative downstream.
//!
//! Public query operations are [`FindQuery`], [`InspectQuery`], [`TraverseQuery`],
//! and [`ReadQuery`]. Consumer responses use [`QueryResponse`] with explicit information tiers;
//! [`QueryResult`] retains the expanded operation evidence.

#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]
#![deny(clippy::doc_markdown)]
#![deny(clippy::missing_errors_doc)]
#![deny(clippy::missing_panics_doc)]
#![deny(clippy::missing_safety_doc)]

mod output;
pub use output::*;

mod boundaries;
mod call_components;
pub use boundaries::*;
mod common;
mod compatibility;
mod find;
mod impact;
mod inspect;
mod inventory;
mod overlay;
mod read;
mod request;
mod selectors;
pub use selectors::*;
mod result;
mod seeds;
mod service;
mod status;
mod traverse;

pub use crate::package_api::{CompatibilityLimitation, CompatibilityLimitationCode};
pub use call_components::{
    CallComponent, CallComponentAnchorKind, CallComponentsReport, CallComponentsRequest,
    CALL_COMPONENTS_REPORT_SCHEMA, DEFAULT_CALL_COMPONENTS_LIMIT,
    DEFAULT_CALL_COMPONENT_SAMPLE_LIMIT, MAX_CALL_COMPONENTS_LIMIT,
};
pub use common::{
    AttachmentDispositionCount, AttachmentEvidenceSummary, AttachmentGrainCount, EvidenceReceipt,
    LineRange, ProviderEvidenceReceipt, QueryDiagnostic, QueryEvidenceReceipt, QueryGapCode,
    QueryScope, QueryStatus, RelationEvidenceSummary, RelationNeighbor, RequestedSourceRange,
    ResultBounds, SchemaJson, SourceLine, SourceVerificationReceipt, SourceVerificationStatus,
    StructuralEdgeRef, SubjectCandidate, UnresolvedEvidenceSummary,
};
pub use compatibility::{
    BoundaryKind, CompatibilityBoundary, CompatibilityPublication, CompatibilityReason,
    CompatibilityRequest, CompatibilityResult, CompatibilitySite, CompatibilityVerdict,
    PACKAGE_COMPATIBILITY_SCHEMA,
};
pub use find::{
    EvidenceRoleFindResult, EvidenceRoleHit, FacetFindResult, FacetHit, FindPayload,
    FindingFindResult, FindingHit, LocationFindResult, SubjectFindResult,
};
pub use impact::{
    BindingImpact, BindingImpactKind, BindingImpactReason, BreakingChangeSignal,
    BreakingChangeSignalKind, BreakingChangeSignals, CallableImpact, CallableImpactKind,
    CodeContractImpact, CodeContractImpactKind, DeclarationImpact, DeclarationImpactKind,
    EvidenceImpact, EvidenceImpactKind, FileImpact, FileImpactKind, ImpactNonSignals,
    ImpactRelayItem, ImpactRelayRequest, ImpactRelayResult, IntroducedEvidence, RescanRequirement,
    RevisionImpactManifest, REVISION_IMPACT_SCHEMA,
};
pub use inspect::{
    AttachedFinding, CallDirection, CallNeighborhoodResult, CallPageCursor, CallRelationHit,
    EvidenceInspectBody, EvidenceInspectResult, FileContainmentContext, FileDescendantCounts,
    InheritedFileFinding, InspectPayload, PropositionInspectResult, RelationClaimsInspectResult,
    SubjectInspectResult, SubjectInspectTarget, TraceInspectResult, TraceStepHit,
    UnresolvedPageCursor, UnresolvedPageResult,
};
pub use inventory::{
    InventoryReport, InventoryRequest, DEFAULT_INVENTORY_LIMIT, INVENTORY_REPORT_SCHEMA,
    MAX_INVENTORY_LIMIT,
};
pub use overlay::{
    InventoryOverlayDelta, InventoryOverlayReport, InventoryOverlayRequest, OverlayChangeKind,
    INVENTORY_OVERLAY_SCHEMA,
};
pub use read::{ReadPayload, SourceReadResult};
pub use request::{
    BoundedWalkBudget, FindQuery, HopEvidenceFilter, InspectQuery, QueryRequest, ReadQuery,
    TraverseQuery,
};
pub use result::{
    QueryContractError, QueryPayload, QueryResult, QueryResultSchema, QUERY_RESULT_SCHEMA,
};
pub use seeds::{
    ProviderRoleSelector, RoleSelectionReceipt, SeedAnchor, SeededPathReport, SeededPathRequest,
    SeededRoute,
};
pub use service::{QueryService, QueryServiceError};
pub use status::{
    CountedStat, FanDirection, NamedCount, PublicationHealth, StatusReport, StatusRequest,
    SuggestedSelector, DEFAULT_STATUS_SAMPLE_LIMIT, DEFAULT_STATUS_SUGGESTION_LIMIT,
    STATUS_REPORT_SCHEMA,
};
pub use traverse::{
    NeighborhoodHit, PathClassification, TraverseHop, TraverseNeighborhoodResult, TraversePath,
    TraversePathResult, TraversePayload, TraverseSearchReceipt,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use thiserror::Error;

/// Stable identifier of the generated query-contract JSON Schema catalog.
pub const QUERY_CONTRACT_CATALOG_ID: &str =
    "https://entmoot.dev/schemas/contracts/entmoot-contracts.v1.json";

/// Deterministic scope applied before optional candidate ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateEligibility {
    /// Stable repository identity.
    pub repository_id: String,
    /// Exact source or knowledge snapshot identity.
    pub snapshot_id: String,
    /// Declared query-scope identity within the snapshot.
    pub scope_id: String,
    /// Optional repository-relative POSIX path restrictions.
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    /// Optional allowlist of subject-kind wire names.
    #[serde(default)]
    pub subject_kinds: Vec<String>,
}

/// Request to rank eligible subjects for subsequent authoritative hydration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateRequest {
    /// Consumer-supplied discovery text; never stored as graph truth.
    pub query_text: String,
    /// Deterministic eligibility boundary applied before ranking.
    pub eligibility: CandidateEligibility,
    /// Inclusive output bound in the range `1..=1000`.
    pub max_candidates: usize,
}

impl CandidateRequest {
    /// Validate the request before invoking a candidate backend.
    ///
    /// # Examples
    ///
    /// ```
    /// use entmoot_contracts::query::{CandidateEligibility, CandidateRequest};
    ///
    /// let request = CandidateRequest {
    ///     query_text: "authentication boundary".into(),
    ///     eligibility: CandidateEligibility {
    ///         repository_id: "repo:example".into(),
    ///         snapshot_id: "snapshot:1".into(),
    ///         scope_id: "scope:all".into(),
    ///         path_prefixes: vec!["src".into()],
    ///         subject_kinds: vec!["callable".into()],
    ///     },
    ///     max_candidates: 20,
    /// };
    ///
    /// assert!(request.validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CandidateRetrievalError::InvalidRequest`] when required scope
    /// identity is empty, the candidate bound is outside `1..=1000`, or a path
    /// prefix is not a repository-relative POSIX path.
    pub fn validate(&self) -> Result<(), CandidateRetrievalError> {
        if self.query_text.trim().is_empty() {
            return Err(CandidateRetrievalError::InvalidRequest(
                "query_text is required".into(),
            ));
        }
        if self.eligibility.repository_id.trim().is_empty()
            || self.eligibility.snapshot_id.trim().is_empty()
            || self.eligibility.scope_id.trim().is_empty()
        {
            return Err(CandidateRetrievalError::InvalidRequest(
                "repository_id, snapshot_id, and scope_id are required".into(),
            ));
        }
        if !(1..=1_000).contains(&self.max_candidates) {
            return Err(CandidateRetrievalError::InvalidRequest(
                "max_candidates must be in 1..=1000".into(),
            ));
        }
        if self.eligibility.path_prefixes.iter().any(|path| {
            path.starts_with('/')
                || path.contains('\\')
                || path.split('/').any(|segment| segment == "..")
        }) {
            return Err(CandidateRetrievalError::InvalidRequest(
                "path_prefixes must be repository-relative POSIX paths without traversal".into(),
            ));
        }
        Ok(())
    }
}

/// One ranked candidate selected for authoritative hydration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateHit {
    /// Stable subject identity already present in the eligible inventory.
    pub subject_id: String,
    /// One-based rank assigned by the backend after eligibility filtering.
    pub rank: usize,
    /// Backend-native ranking value. It is not epistemic confidence.
    pub score: f64,
    /// Backend-defined name describing the ranking score's semantics.
    pub score_kind: String,
}

/// Provenance and cardinality receipt for one candidate-retrieval operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetrievalReceipt {
    /// Stable implementation identifier for the ranking backend.
    pub backend_id: String,
    /// Backend algorithm or service version.
    pub backend_version: String,
    /// Exact disposable index used for ranking.
    pub index_id: String,
    /// Number of subjects remaining after deterministic eligibility filtering.
    pub eligible_subjects: usize,
    /// Number of candidates returned to the caller.
    pub returned_candidates: usize,
    /// Whether an explicit candidate bound omitted additional eligible results.
    pub truncated: bool,
}

/// Ranked candidates and the receipt needed to audit their selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateBatch {
    /// Candidates in backend rank order.
    pub candidates: Vec<CandidateHit>,
    /// Backend, index, and cardinality metadata for this batch.
    pub receipt: RetrievalReceipt,
}

/// Failure to validate or execute optional candidate retrieval.
#[derive(Debug, Error)]
pub enum CandidateRetrievalError {
    /// The request violates a deterministic input invariant.
    #[error("candidate request is invalid: {0}")]
    InvalidRequest(String),
    /// The selected backend or index cannot currently serve the request.
    #[error("candidate index is unavailable: {0}")]
    Unavailable(String),
    /// The backend failed while ranking an otherwise valid request.
    #[error("candidate backend failed: {0}")]
    Backend(String),
}

/// Replaceable discovery accelerator. Implementations must apply eligibility
/// before ranking and must not manufacture durable facts or evidence state.
pub trait CandidateRetriever: Send + Sync {
    /// Return ranked subjects after applying [`CandidateRequest::eligibility`].
    ///
    /// # Errors
    ///
    /// Returns [`CandidateRetrievalError`] for invalid requests, unavailable
    /// indexes, or backend execution failures.
    fn retrieve(
        &self,
        request: &CandidateRequest,
    ) -> Result<CandidateBatch, CandidateRetrievalError>;
}

/// Emit the public query and candidate-retrieval wire shapes as one generated
/// catalog. Semantic validation remains in the Rust constructors and tests.
///
/// # Errors
///
/// Returns a serialization error if a derived schema cannot be represented as
/// JSON. The generated catalog is golden-tested before publication.
pub fn emit_query_contract_schemas_json() -> Result<String, serde_json::Error> {
    let mut definitions = Map::new();
    let mut insert = |name: &str, schema: schemars::Schema| -> Result<(), serde_json::Error> {
        let mut schema = serde_json::to_value(schema)?;
        schema["$id"] = serde_json::json!(format!("{QUERY_CONTRACT_CATALOG_ID}?type={name}"));
        definitions.insert(name.into(), schema);
        Ok(())
    };
    insert(
        "BoundedWalkBudget",
        schemars::schema_for!(BoundedWalkBudget),
    )?;
    insert("CandidateBatch", schemars::schema_for!(CandidateBatch))?;
    insert(
        "CandidateEligibility",
        schemars::schema_for!(CandidateEligibility),
    )?;
    insert("CandidateHit", schemars::schema_for!(CandidateHit))?;
    insert("CandidateRequest", schemars::schema_for!(CandidateRequest))?;
    insert("EvidenceReceipt", schemars::schema_for!(EvidenceReceipt))?;
    insert(
        "CompatibilityRequest",
        schemars::schema_for!(CompatibilityRequest),
    )?;
    insert(
        "CompatibilityResult",
        schemars::schema_for!(CompatibilityResult),
    )?;
    insert("FindPayload", schemars::schema_for!(FindPayload))?;
    insert("FindQuery", schemars::schema_for!(FindQuery))?;
    insert("InspectPayload", schemars::schema_for!(InspectPayload))?;
    insert("InspectQuery", schemars::schema_for!(InspectQuery))?;
    insert(
        "RevisionImpactManifest",
        schemars::schema_for!(RevisionImpactManifest),
    )?;
    insert(
        "InventoryOverlayRequest",
        schemars::schema_for!(InventoryOverlayRequest),
    )?;
    insert(
        "InventoryOverlayReport",
        schemars::schema_for!(InventoryOverlayReport),
    )?;
    insert("QueryGapCode", schemars::schema_for!(QueryGapCode))?;
    insert(
        "QueryEvidenceReceipt",
        schemars::schema_for!(QueryEvidenceReceipt),
    )?;
    insert("QueryPayload", schemars::schema_for!(QueryPayload))?;
    insert("QueryResult", schemars::schema_for!(QueryResult))?;
    insert("QueryRequest", schemars::schema_for!(QueryRequest))?;
    insert("QueryResponse", schemars::schema_for!(QueryResponse))?;
    insert(
        "ImpactRelayRequest",
        schemars::schema_for!(ImpactRelayRequest),
    )?;
    insert(
        "ImpactRelayResult",
        schemars::schema_for!(ImpactRelayResult),
    )?;
    insert("OutputOptions", schemars::schema_for!(OutputOptions))?;
    insert(
        "SeededPathResponse",
        schemars::schema_for!(SeededPathResponse),
    )?;
    insert("QueryStatus", schemars::schema_for!(QueryStatus))?;
    insert("ReadPayload", schemars::schema_for!(ReadPayload))?;
    insert("ReadQuery", schemars::schema_for!(ReadQuery))?;
    insert("ResultBounds", schemars::schema_for!(ResultBounds))?;
    insert("RetrievalReceipt", schemars::schema_for!(RetrievalReceipt))?;
    insert("SubjectCandidate", schemars::schema_for!(SubjectCandidate))?;
    insert("TraversePayload", schemars::schema_for!(TraversePayload))?;
    insert("TraverseQuery", schemars::schema_for!(TraverseQuery))?;
    insert(
        "SeededPathRequest",
        schemars::schema_for!(SeededPathRequest),
    )?;
    insert("SeededPathReport", schemars::schema_for!(SeededPathReport))?;
    insert("StatusRequest", schemars::schema_for!(StatusRequest))?;
    insert("StatusReport", schemars::schema_for!(StatusReport))?;
    insert("InventoryRequest", schemars::schema_for!(InventoryRequest))?;
    insert("InventoryReport", schemars::schema_for!(InventoryReport))?;
    insert(
        "CallComponentsRequest",
        schemars::schema_for!(CallComponentsRequest),
    )?;
    insert(
        "CallComponentsReport",
        schemars::schema_for!(CallComponentsReport),
    )?;
    let document = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": QUERY_CONTRACT_CATALOG_ID,
        "title": "entmoot-query-contracts",
        "description": "Generated query and candidate-retrieval structure. Predicate semantics and eligibility invariants remain normative in Rust and ADR-0013/ADR-0014.",
        "$defs": definitions,
    });
    let mut output = serde_json::to_string_pretty(&document)?;
    output.push('\n');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::common::{ResultBounds, SchemaJson};
    use crate::query::find::{FindPayload, SubjectFindResult};
    use crate::{SourceSnapshot, SourceState};

    fn sample_snapshot() -> SourceSnapshot {
        SourceSnapshot {
            repository_id: "repo:example".into(),
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        }
    }

    #[test]
    fn validates_and_round_trips_typed_find_payload() {
        let result = QueryResult::new(
            QueryStatus::Known,
            QueryEvidenceReceipt::inventory_only(sample_snapshot()),
            QueryPayload::Find(FindPayload::Subject(SubjectFindResult {
                selection: None,
                snapshot: SchemaJson::new(sample_snapshot()),
                items: vec![SubjectCandidate {
                    selection_match: None,
                    node_id: "n1".into(),
                    kind: "callable".into(),
                    label: "main".into(),
                    path: Some("src/main.rs".into()),
                    start_line: Some(1),
                    display_ref: "src/main.rs:1".into(),
                    fqn: None,
                    address_id: None,
                    logic_address: None,
                    structural_hash: None,
                    match_score: 4,
                }],
                gaps: Vec::new(),
                bounds: ResultBounds::page(20, 1, 1),
            })),
        );
        let value = result.clone().into_value().expect("serialize result");
        assert_eq!(value["operation"], "find");
        assert_eq!(value["view"], "subject");
        assert_eq!(value["status"], "known");
        let decoded = QueryResult::from_value(value).expect("valid result");
        assert_eq!(decoded, result);
    }

    #[test]
    fn rejects_unknown_status() {
        let error = QueryResult::from_value(serde_json::json!({
            "schema_version": QUERY_RESULT_SCHEMA,
            "status": "maybe",
            "receipt": QueryEvidenceReceipt::inventory_only(sample_snapshot()),
            "operation": "find",
            "view": "subject",
            "snapshot": sample_snapshot(),
            "items": [],
            "gaps": [],
            "bounds": {
                "offset": 0,
                "limit": 0,
                "total_matches": 0,
                "truncated": false
            }
        }))
        .expect_err("unknown status must fail");
        assert!(matches!(error, QueryContractError::Invalid(_)));
    }

    #[test]
    fn every_public_result_requires_an_operation_evidence_receipt() {
        let value = serde_json::json!({
            "schema_version": QUERY_RESULT_SCHEMA, "status": "unknown", "operation": "find",
            "view": "subject", "snapshot": sample_snapshot(), "items": [], "gaps": [],
            "bounds": {"offset": 0, "limit": 1, "total_matches": 0, "truncated": false}
        });
        assert!(QueryResult::from_value(value).is_err());
    }

    #[test]
    fn typed_results_reject_wrong_versions_and_extra_fields() {
        let value = serde_json::json!({
            "schema_version": QUERY_RESULT_SCHEMA, "status": "unknown", "operation": "find",
            "receipt": QueryEvidenceReceipt::inventory_only(sample_snapshot()),
            "view": "subject", "snapshot": sample_snapshot(), "items": [], "gaps": [],
            "bounds": {"offset": 0, "limit": 1, "total_matches": 0, "truncated": false}
        });
        QueryResult::from_value(value.clone()).expect("valid typed fixture");
        let mut wrong_version = value.clone();
        wrong_version["schema_version"] = serde_json::json!("entmoot.query.result.invalid");
        assert!(serde_json::from_value::<QueryResult>(wrong_version).is_err());
        let mut extra = value;
        extra["invented_truth"] = serde_json::json!(true);
        assert!(QueryResult::from_value(extra).is_err());
    }

    #[test]
    fn embedded_core_records_have_real_schemas_and_catalog_resource_ids() {
        let schema = serde_json::to_value(schemars::schema_for!(QueryResult)).expect("schema");
        let snapshot = &schema["$defs"]["SourceSnapshot"];
        assert_eq!(snapshot["properties"]["repository_id"]["type"], "string");
        assert!(schema["$defs"].get("SchemaJson").is_none());
        let catalog: serde_json::Value =
            serde_json::from_str(&emit_query_contract_schemas_json().expect("emit"))
                .expect("catalog");
        assert!(catalog["$defs"]["QueryResult"]["$id"]
            .as_str()
            .expect("resource id")
            .contains("?type=QueryResult"));
    }

    #[test]
    fn candidate_requests_require_exact_eligibility_scope() {
        let request = CandidateRequest {
            query_text: "authentication boundary".into(),
            eligibility: CandidateEligibility {
                repository_id: "repo:example".into(),
                snapshot_id: String::new(),
                scope_id: "scope:all".into(),
                path_prefixes: Vec::new(),
                subject_kinds: Vec::new(),
            },
            max_candidates: 20,
        };
        assert!(matches!(
            request.validate(),
            Err(CandidateRetrievalError::InvalidRequest(_))
        ));
    }

    #[test]
    fn query_contract_schemas_match_golden() {
        let generated = emit_query_contract_schemas_json().expect("emit");
        let golden = include_str!("../../generated/entmoot_query_contracts.schema.json");
        assert_eq!(
            generated, golden,
            "schema drift: run `cargo run -p entmoot-contracts --example emit_query_json_schema`"
        );
    }
}
