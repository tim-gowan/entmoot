//! Transport-neutral query-serving capability.

use thiserror::Error;

use crate::query::{
    CallComponentsReport, CallComponentsRequest, FindQuery, InspectQuery, InventoryReport,
    InventoryRequest, QueryRequest, QueryResponse, QueryResult, ReadQuery, SeededPathRequest,
    SeededPathResponse, StatusReport, StatusRequest, TraverseQuery,
};

/// Failure categories exposed by a query-serving implementation.
///
/// These categories are deliberately independent of a storage engine, graph
/// projection, source-store implementation, or transport protocol.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QueryServiceError {
    /// The request violates the operation contract.
    #[error("query input is invalid: {0}")]
    InvalidRequest(String),
    /// Required authoritative state or a serving dependency is unavailable.
    #[error("query service is unavailable: {0}")]
    Unavailable(String),
    /// The implementation failed to execute or validate an otherwise valid request.
    #[error("query execution failed: {0}")]
    Execution(String),
}

impl QueryServiceError {
    /// Stable transport spelling for the failure category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::Unavailable(_) => "unavailable",
            Self::Execution(_) => "execution",
        }
    }
}

/// Public capability port for general queries and seeded-route composition.
///
/// Implementations hydrate only the DTOs owned by this crate. Consumers do not
/// need to know whether facts came from `SQLite`, an exported graph, a remote
/// service, or another conforming knowledge engine.
pub trait QueryService: Send + Sync {
    /// Rank unused-code review candidates. Missing references are never absence proofs.
    /// # Errors
    /// Returns unavailable state or invalid scope/budgets.
    fn unused(
        &self,
        _request: crate::UnusedRequest,
    ) -> Result<crate::UnusedReport, QueryServiceError> {
        Err(QueryServiceError::Unavailable(
            "unused assessment is not configured".into(),
        ))
    }
    /// Select test targets from both revisions with declared ownership as the minimum.
    /// # Errors
    /// Returns invalid ownership or unavailable revision pairs.
    fn test_impact(
        &self,
        _request: crate::TestImpactRequest,
    ) -> Result<crate::TestImpactReport, QueryServiceError> {
        Err(QueryServiceError::Unavailable(
            "test impact is not configured".into(),
        ))
    }
    /// Find source-verified repeated patterns for review, without asserting equivalence.
    ///
    /// # Errors
    /// Returns unavailable source, invalid requests, or snapshot verification failures.
    fn clones(
        &self,
        _request: crate::CloneRequest,
    ) -> Result<crate::CloneReport, QueryServiceError> {
        Err(QueryServiceError::Unavailable(
            "clone analysis is not configured".into(),
        ))
    }
    /// Relay changed head callables to bounded caller pages on a configured pair.
    ///
    /// # Errors
    /// Returns invalid requests (including byte/cursor refusal), unavailable
    /// publications, or execution failures. Never runs producers or mutates state.
    fn relay_impact(
        &self,
        _request: crate::query::ImpactRelayRequest,
    ) -> Result<crate::query::ImpactRelayResult, QueryServiceError> {
        Err(QueryServiceError::Unavailable(
            "impact relay is not configured".into(),
        ))
    }

    /// Assess explicit package API evidence across independent publications.
    ///
    /// # Errors
    /// Returns invalid input, unavailable publications, or evidence validation
    /// failures. Reads never run producers.
    fn compatibility(
        &self,
        _request: crate::query::CompatibilityRequest,
    ) -> Result<crate::query::CompatibilityResult, QueryServiceError> {
        Err(QueryServiceError::Unavailable(
            "package compatibility is not configured".into(),
        ))
    }

    /// Dispatch a portable request using the same implementation as task tools.
    /// Applies compact/explain/export before delivery. Consumers use this method;
    /// the low-level find/inspect/traverse/read methods return expanded internal
    /// results for composition and do not apply output byte/page controls.
    ///
    /// # Errors
    /// Returns the operation's original failure category without reinterpretation,
    /// including invalid output budgets, mismatched cursors and envelopes that
    /// cannot fit the requested byte ceiling. No silent evidence cutting.
    fn execute(&self, request: QueryRequest) -> Result<QueryResponse, QueryServiceError>;

    /// Discover bounded routes between provider-stated role locations.
    ///
    /// # Errors
    /// Returns invalid input, unavailable state, or execution failure through
    /// the same categories as ordinary queries. No transport may infer roles.
    fn trace_seeds(
        &self,
        request: SeededPathRequest,
    ) -> Result<SeededPathResponse, QueryServiceError>;

    /// Report publication health and bounded high-signal find selectors.
    ///
    /// # Errors
    /// Returns invalid input, unavailable state, or execution failure through
    /// the same categories as ordinary queries. Never mutates knowledge or
    /// invokes producers.
    fn status(&self, request: StatusRequest) -> Result<StatusReport, QueryServiceError>;

    /// Page materialized inventory subjects for review.
    ///
    /// # Errors
    /// Returns invalid input, unavailable state, or execution failure through
    /// the same categories as ordinary queries. Empty pages are not negatives.
    fn inventory(&self, request: InventoryRequest) -> Result<InventoryReport, QueryServiceError>;

    /// Page weakly connected call-relation components among callables.
    ///
    /// # Errors
    /// Returns invalid input (including unsupported predicates), unavailable
    /// state, or execution failure. Components are not unused-code claims.
    fn call_components(
        &self,
        request: CallComponentsRequest,
    ) -> Result<CallComponentsReport, QueryServiceError>;

    /// Find bounded inventory or evidence anchors.
    ///
    /// # Errors
    ///
    /// Returns [`QueryServiceError`] when the request is invalid or the selected
    /// implementation cannot serve or validate it.
    fn find(&self, request: FindQuery) -> Result<QueryResult, QueryServiceError>;

    /// Inspect an exact subject or evidence identity.
    ///
    /// # Errors
    ///
    /// Returns [`QueryServiceError`] when the request is invalid or the selected
    /// implementation cannot serve or validate it.
    fn inspect(&self, request: InspectQuery) -> Result<QueryResult, QueryServiceError>;

    /// Traverse bounded, typed relations between exact subjects.
    ///
    /// # Errors
    ///
    /// Returns [`QueryServiceError`] when the request is invalid or the selected
    /// implementation cannot serve or validate it.
    fn traverse(&self, request: TraverseQuery) -> Result<QueryResult, QueryServiceError>;

    /// Read a bounded, digest-verified source span.
    ///
    /// # Errors
    ///
    /// Returns [`QueryServiceError`] when the request is invalid or the selected
    /// implementation cannot serve or validate it.
    fn read(&self, request: ReadQuery) -> Result<QueryResult, QueryServiceError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct UnavailableService;

    impl QueryService for UnavailableService {
        fn execute(&self, _request: QueryRequest) -> Result<QueryResponse, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }
        fn trace_seeds(
            &self,
            _request: SeededPathRequest,
        ) -> Result<SeededPathResponse, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }
        fn status(&self, _request: StatusRequest) -> Result<StatusReport, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }
        fn inventory(
            &self,
            _request: InventoryRequest,
        ) -> Result<InventoryReport, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }
        fn call_components(
            &self,
            _request: CallComponentsRequest,
        ) -> Result<CallComponentsReport, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }
        fn find(&self, _request: FindQuery) -> Result<QueryResult, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }

        fn inspect(&self, _request: InspectQuery) -> Result<QueryResult, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }

        fn traverse(&self, _request: TraverseQuery) -> Result<QueryResult, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }

        fn read(&self, _request: ReadQuery) -> Result<QueryResult, QueryServiceError> {
            Err(QueryServiceError::Unavailable("fixture".into()))
        }
    }

    #[test]
    fn consumer_port_is_object_safe_and_uses_stable_failure_categories() {
        let service: Arc<dyn QueryService> = Arc::new(UnavailableService);
        let error = service
            .find(FindQuery::Subject {
                query: Some("main".into()),
                path: None,
                line: None,
                limit: Some(1),
            })
            .expect_err("fixture is unavailable");
        assert_eq!(error.code(), "unavailable");
    }
}
