//! Read-only impact composition using the existing neighborhood and delivery engine.
use super::{output, validate_walk_budget, QueryEngine, QueryError};
use entmoot_contracts::query::*;

impl QueryEngine {
    pub(super) fn deliver_impact_relay(
        &self,
        mut request: ImpactRelayRequest,
    ) -> Result<ImpactRelayResult, QueryError> {
        output::validate(&request.output)?;
        output::validate(&request.callers_output)?;
        validate_walk_budget(request.budget)?;
        if request.output.tier != InformationTier::Compact
            || request.callers_output.tier != InformationTier::Compact
            || request.output.limit > 16
            || request.callers_output.after.is_some()
        {
            return Err(QueryError::InvalidInput("impact relay requires compact output, 1..=16 subjects, and no shared caller cursor; resume each caller page through its returned neighborhood query, and use single-subject explain/export for details".into()));
        }
        let options = request.output.clone();
        request.output = OutputOptions::default();
        let cursor = self.output_cursor(&request, &options, true)?;
        let base = self.compare_graph.as_deref().ok_or_else(|| {
            QueryError::InvalidInput(
                "impact relay requires a configured base/compare publication".into(),
            )
        })?;
        let impact = crate::compare_revision_impact(base, &self.complete_graph()?)
            .map_err(|error| QueryError::InvalidInput(error.to_string()))?;
        let total = impact.callables.len();
        if cursor.offset > total {
            return Err(QueryError::InvalidInput(
                "impact relay cursor exceeds changed callables".into(),
            ));
        }
        let mut gaps = impact.gaps;
        gaps.extend(
            [
                "repository_call_graph_completeness_not_established",
                "dependency_call_graph_completeness_not_established",
                "native_library_call_graph_completeness_not_established",
            ]
            .map(str::to_owned),
        );
        gaps.push("head_callers_only;removed_callables_have_no_head_neighborhood".into());
        let mut result = ImpactRelayResult {
            schema_version: "entmoot.impact_relay.v1".into(),
            implementation: impact.implementation,
            base_snapshot: impact.base_snapshot,
            head_snapshot: impact.head_snapshot,
            request: request.clone(),
            page: OutputPage {
                offset: cursor.offset,
                returned: 0,
                total_items: total,
                next_cursor: None,
            },
            items: Vec::new(),
            gaps,
            details_deferred: true,
        };
        for callable in impact
            .callables
            .into_iter()
            .skip(cursor.offset)
            .take(options.limit)
        {
            let (callers, gaps) = if let Some(id) = &callable.head_node_id {
                let response = self.deliver_query(QueryRequest::Traverse {
                    request: TraverseQuery::Neighborhood {
                        resumable: false,
                        continuation: None,
                        start: id.clone(),
                        direction: CallDirection::Callers,
                        predicates: vec!["calls".into()],
                        evidence_filter: request.evidence_filter,
                        endpoint_role: None,
                        budget: request.budget,
                    },
                    output: request.callers_output.clone(),
                })?;
                let QueryResponse::Compact(page) = response else {
                    return Err(QueryError::InvalidInput(
                        "impact relay expected compact callers".into(),
                    ));
                };
                (Some(page), Vec::new())
            } else {
                (None, vec!["removed_callable_no_head_neighborhood".into()])
            };
            result.items.push(ImpactRelayItem {
                callable,
                callers,
                gaps,
            });
            result.page.returned = result.items.len();
            let next = cursor.offset + result.page.returned;
            result.page.next_cursor = (next < total).then(|| OutputCursor {
                offset: next,
                ..cursor.clone()
            });
            if output::byte_len(&result)? > options.byte_limit() {
                if result.items.len() == 1 {
                    output::check_bytes(&result, &options)?;
                }
                result.items.pop();
                result.page.returned = result.items.len();
                result.page.next_cursor = Some(OutputCursor {
                    offset: cursor.offset + result.page.returned,
                    ..cursor.clone()
                });
                break;
            }
        }
        self.check_output_visibility(&cursor)?;
        output::check_bytes(&result, &options)?;
        Ok(result)
    }
}
