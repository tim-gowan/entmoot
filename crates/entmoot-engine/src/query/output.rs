//! Shared information delivery. Projection never makes evidence stronger.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use entmoot_contracts::query::*;
use entmoot_contracts::{address_identity_key, blake3_hex, Address, ContentDigest, RelationEdge};
use serde::Serialize;

use super::{QueryEngine, QueryError};
use crate::KnowledgeRepository;

pub(super) fn validate(options: &OutputOptions) -> Result<(), QueryError> {
    if !(1..=1000).contains(&options.limit)
        || !(128..=64 * 1024 * 1024).contains(&options.byte_limit())
        || (options.tier != InformationTier::Compact && options.after.is_some())
    {
        return Err(QueryError::InvalidInput("output requires limit 1..=1000, max_bytes 128..=67108864, and continuations only for compact".into()));
    }
    Ok(())
}

#[derive(Default)]
struct ByteCounter(usize);
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn byte_len(value: &impl Serialize) -> Result<usize, QueryError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}
pub(super) fn check_bytes(
    value: &impl Serialize,
    options: &OutputOptions,
) -> Result<(), QueryError> {
    let needed = byte_len(value)?;
    if needed > options.byte_limit() {
        return Err(QueryError::InvalidInput(format!("output byte budget {} cannot contain this response ({needed} bytes); narrow the query or raise output.max_bytes", options.byte_limit())));
    }
    Ok(())
}

fn cap_details(request: &mut QueryRequest, tier: InformationTier) {
    if let QueryRequest::Traverse { request, .. } = request {
        let budget = match request {
            TraverseQuery::Path { budget, .. } | TraverseQuery::Neighborhood { budget, .. } => {
                budget
            }
        };
        budget.max_evidence_per_hop = match tier {
            InformationTier::Compact => 0,
            InformationTier::Explain => budget.max_evidence_per_hop.min(3),
            InformationTier::Export => budget.max_evidence_per_hop,
        };
    }
}

impl QueryEngine {
    pub(super) fn publication_generation_digest(
        &self,
        repository: &crate::SqliteKnowledgeRepository,
    ) -> Result<ContentDigest, QueryError> {
        Ok(ContentDigest::from_trusted(blake3_hex(
            &serde_json::to_vec(&(
                "entmoot.query.generation.v2",
                self.output_generation_digest(repository)?,
                repository.read_state_digest()?,
            ))?,
        )))
    }

    fn output_generation_digest(
        &self,
        repository: &crate::SqliteKnowledgeRepository,
    ) -> Result<ContentDigest, QueryError> {
        if let Some(digest) = self.output_generation.get() {
            return Ok(digest.clone());
        }
        if let Some(digest) = repository.inventory_export_digest()? {
            let digest = ContentDigest::from_trusted(digest);
            let _ = self.output_generation.set(digest.clone());
            return Ok(digest);
        }
        struct DigestWriter(blake3::Hasher);
        impl Write for DigestWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = DigestWriter(blake3::Hasher::new());
        serde_json::to_writer(&mut writer, self.graph.as_ref())?;
        let digest =
            ContentDigest::from_trusted(format!("blake3:{}", writer.0.finalize().to_hex()));
        let _ = self.output_generation.set(digest.clone());
        Ok(digest)
    }

    pub(super) fn deliver_query(
        &self,
        mut request: QueryRequest,
    ) -> Result<QueryResponse, QueryError> {
        let options = request.output().clone();
        validate(&options)?;
        request.set_output(OutputOptions::default());
        let comparison = matches!(
            &request,
            QueryRequest::Find {
                request: FindQuery::ChangedSubject { .. },
                ..
            }
        );
        let mut cursor = (options.tier == InformationTier::Compact)
            .then(|| self.output_cursor(&request, &options, comparison))
            .transpose()?;
        let expanded_state = if cursor.is_none() {
            Some(self.repository()?.read_state_digest()?)
        } else {
            None
        };
        let mut execution = request.clone();
        cap_details(&mut execution, options.tier);
        if let Some(token) = cursor.as_ref().and_then(|c| c.search_slice.as_ref()) {
            match &mut execution {
                QueryRequest::Traverse { request, .. } if request.resumable() => {
                    request.set_continuation(Some(token.clone()))
                }
                _ => {
                    return Err(QueryError::InvalidInput(
                        "search slice cursor requires its original resumable traverse request"
                            .into(),
                    ))
                }
            }
        }
        // Detail policy is applied before per-hop arrays are built. Claims still
        // participate in classification regardless of which records are displayed.
        let result = match execution {
            QueryRequest::Find { request, .. } => self.find(request),
            QueryRequest::Inspect { request, .. } => self.inspect(request),
            QueryRequest::Traverse { request, .. } => self.traverse(request),
            QueryRequest::Read { request, .. } => self.read(request),
        }?;
        if let Some(cursor) = &mut cursor {
            cursor.search_slice = traversal_receipt(&result).and_then(|r| r.slice_token.clone());
        }
        if let Some(cursor) = cursor {
            self.check_output_visibility(&cursor)?;
            return self
                .compact_query(result, request, &options, cursor)
                .map(QueryResponse::Compact);
        }
        if expanded_state.as_ref() != Some(&self.repository()?.read_state_digest()?) {
            return Err(QueryError::InvalidInput(
                "knowledge changed during output delivery; restart the query".into(),
            ));
        }
        let expanded = ExpandedQueryResponse {
            schema_version: QueryResponseSchema::V1,
            details_deferred: detail_deferred(&result),
            result,
        };
        let response = if options.tier == InformationTier::Explain {
            QueryResponse::Explain(expanded)
        } else {
            QueryResponse::Export(expanded)
        };
        check_bytes(&response, &options)?;
        Ok(response)
    }

    pub(super) fn check_output_visibility(&self, cursor: &OutputCursor) -> Result<(), QueryError> {
        let repository = self.repository()?;
        if repository.current_visibility_cursor()? != cursor.visibility_cursor
            || self.publication_generation_digest(&repository)? != cursor.generation_digest
        {
            return Err(QueryError::InvalidInput(
                "knowledge changed during output delivery; restart the query".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn output_cursor(
        &self,
        request: &impl Serialize,
        options: &OutputOptions,
        comparison: bool,
    ) -> Result<OutputCursor, QueryError> {
        let repository = self.repository()?;
        let cursor = OutputCursor {
            search_slice: options.after.as_ref().and_then(|c| c.search_slice.clone()),
            snapshot: self.graph.snapshot.clone(),
            query_digest: ContentDigest::from_trusted(blake3_hex(&serde_json::to_vec(&(
                request,
                comparison.then_some(self.compare_graph.as_deref()),
            ))?)),
            generation_digest: self.publication_generation_digest(&repository)?,
            visibility_cursor: repository.current_visibility_cursor()?,
            offset: options.after.as_ref().map_or(0, |c| c.offset),
        };
        if options.after.as_ref().is_some_and(|prior| prior != &cursor) {
            return Err(QueryError::InvalidInput("output cursor does not match the query, source snapshot or knowledge visibility state".into()));
        }
        Ok(cursor)
    }

    fn compact_query(
        &self,
        result: QueryResult,
        request: QueryRequest,
        options: &OutputOptions,
        cursor: OutputCursor,
    ) -> Result<CompactQueryResponse, QueryError> {
        let (bounds, gaps) = result_bounds(&result);
        let receipt = compact_receipt(&result.receipt);
        let mut builder = Projection::new(self);
        builder.collect(&result.payload)?;
        let prototype = CompactQueryResponse {
            limitations: match &result.payload {
                QueryPayload::Find(FindPayload::Facet(f)) => f.limitations.clone(),
                _ => Vec::new(),
            },
            completeness: match &result.payload {
                QueryPayload::Inspect(v) => match v.as_ref() {
                    InspectPayload::Calls(c) => Some(c.completeness.clone()),
                    _ => None,
                },
                QueryPayload::Find(FindPayload::Facet(f)) => f.completeness.clone(),
                _ => None,
            },
            search_receipt: traversal_receipt(&result).cloned().map(Box::new),
            schema_version: QueryResponseSchema::V1,
            status: result.status,
            snapshot: self.graph.snapshot.clone(),
            query: Box::new(request),
            receipt,
            search_truncated: search_truncated(&result),
            result_bounds: bounds,
            gaps,
            unresolved_bounds: match &result.payload {
                QueryPayload::Traverse(TraversePayload::Neighborhood(n)) => {
                    Some(n.unresolved_call_bounds.clone())
                }
                QueryPayload::Inspect(v) => match v.as_ref() {
                    InspectPayload::Calls(c) => Some(c.unresolved_bounds.clone()),
                    InspectPayload::Unresolved(u) => Some(u.bounds.clone()),
                    _ => None,
                },
                _ => None,
            },
            unresolved_continuation: (match &result.payload {
                QueryPayload::Inspect(payload) => match payload.as_ref() {
                    // Compact calls display concrete edges only, so start with
                    // the first attempt rather than skipping the deferred page.
                    InspectPayload::Calls(c) if c.unresolved_bounds.total_matches > 0 => {
                        Some(InspectQuery::Unresolved {
                            id: c.subject.clone(),
                            after: None,
                            limit: Some(c.unresolved_bounds.limit),
                        })
                    }
                    InspectPayload::Unresolved(u) => {
                        u.next_cursor.clone().map(|after| InspectQuery::Unresolved {
                            id: u.subject.clone(),
                            after: Some(after),
                            limit: Some(u.bounds.limit),
                        })
                    }
                    _ => None,
                },
                _ => None,
            })
            .map(Box::new),
            role_selection: match &result.payload {
                QueryPayload::Traverse(TraversePayload::Neighborhood(n)) => {
                    n.role_selection.as_ref().map(|r| CompactRoleSelection {
                        selector: r.selector.clone(),
                        matching_assertions: r.matching_assertions,
                        excluded_hypotheses: r.excluded_hypotheses,
                        unmapped_assertions: r.unmapped_assertion_ids.len(),
                        selection_truncated: r.truncated,
                        colocated_count: r.colocated_bounds.total_matches,
                    })
                }
                _ => None,
            },
            details_deferred: true,
            page: OutputPage {
                offset: cursor.offset,
                returned: 0,
                total_items: builder.items.len(),
                next_cursor: None,
            },
            items: vec![],
            subjects: BTreeMap::new(),
            edges: BTreeMap::new(),
            scopes: BTreeMap::new(),
        };
        let total = builder.items.len();
        if cursor.offset > total {
            return Err(QueryError::InvalidInput(
                "output cursor offset exceeds the bounded result".into(),
            ));
        }
        let end = total.min(cursor.offset.saturating_add(options.limit));
        let make = |end: usize| {
            let mut response = prototype.clone();
            response.items = builder.items[cursor.offset..end].to_vec();
            response.page.returned = end - cursor.offset;
            response.page.next_cursor = (end < total).then(|| OutputCursor {
                offset: end,
                ..cursor.clone()
            });
            builder.dictionary_for(&mut response);
            response
        };
        // Monotone prefix search avoids repeated cloning of unbounded record sets.
        if byte_len(&QueryResponse::Compact(make(end)))? <= options.byte_limit() {
            return Ok(make(end));
        }
        let mut low = cursor.offset;
        let mut high = end;
        while low < high {
            let mid = low + (high - low).div_ceil(2);
            if byte_len(&QueryResponse::Compact(make(mid)))? <= options.byte_limit() {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let response = make(low);
        if low == cursor.offset && cursor.offset < total {
            check_bytes(&QueryResponse::Compact(make(cursor.offset + 1)), options)?;
        }
        check_bytes(&QueryResponse::Compact(response.clone()), options)?;
        Ok(response)
    }

    pub(super) fn deliver_seeds(
        &self,
        mut request: SeededPathRequest,
    ) -> Result<SeededPathResponse, QueryError> {
        let options = request.output.clone();
        validate(&options)?;
        request.output = OutputOptions::default();
        let cursor = (options.tier == InformationTier::Compact)
            .then(|| self.output_cursor(&request, &options, false))
            .transpose()?;
        let mut execution = request.clone();
        execution.walk.max_evidence_per_hop = match options.tier {
            InformationTier::Compact => 0,
            InformationTier::Explain => execution.walk.max_evidence_per_hop.min(3),
            InformationTier::Export => execution.walk.max_evidence_per_hop,
        };
        let report = self.trace_seeds(execution)?;
        // The expanded receipt records effective execution/detail budgets.
        // Compact retains the original request as its explicit expansion recipe.
        if options.tier != InformationTier::Compact {
            let response = if options.tier == InformationTier::Explain {
                SeededPathResponse::Explain { report }
            } else {
                SeededPathResponse::Export { report }
            };
            check_bytes(&response, &options)?;
            return Ok(response);
        }
        let cursor = cursor.ok_or_else(|| {
            QueryError::InvalidInput("compact output cursor is unavailable".into())
        })?;
        self.check_output_visibility(&cursor)?;
        let total = report.routes.len();
        if cursor.offset > total {
            return Err(QueryError::InvalidInput(
                "output cursor offset exceeds the bounded route result".into(),
            ));
        }
        let mut compact = CompactSeededResponse {
            snapshot: self.graph.snapshot.clone(),
            request,
            matching_assertions: report.matching_assertions,
            excluded_hypotheses: report.excluded_hypotheses,
            unmapped_assertions: report.unmapped_assertion_ids.len(),
            colocated_count: report.colocated.len(),
            seeds_truncated: report.seeds_truncated,
            discovery_truncated: report.discovery_truncated,
            limitations: report.limitations,
            details_deferred: true,
            page: OutputPage {
                offset: cursor.offset,
                returned: 0,
                total_items: total,
                next_cursor: None,
            },
            routes: vec![],
        };
        let mut route_bytes = 0usize;
        for (index, route) in report
            .routes
            .into_iter()
            .enumerate()
            .skip(cursor.offset)
            .take(options.limit)
        {
            let query = QueryRequest::Traverse {
                request: TraverseQuery::Path {
                    resumable: false,
                    continuation: None,
                    start: route.from_callable.clone(),
                    target: route.to_callable.clone(),
                    predicates: vec!["calls".into()],
                    budget: compact.request.walk,
                },
                output: OutputOptions::default(),
            };
            let nested_options = OutputOptions {
                limit: 1000,
                max_bytes: Some(options.byte_limit()),
                ..Default::default()
            };
            let nested_cursor = self.output_cursor(&query, &nested_options, false)?;
            let projected =
                self.compact_query(route.query, query, &nested_options, nested_cursor)?;
            let anchors = |id: &str, role: &str| {
                report
                    .anchors
                    .iter()
                    .filter(|a| a.callable_id == id && a.role == role)
                    .map(compact_role)
                    .collect()
            };
            let compact_route = CompactSeededRoute {
                from_anchors: anchors(&route.from_callable, &compact.request.from_role),
                to_anchors: anchors(&route.to_callable, &compact.request.to_role),
                from_callable: route.from_callable,
                to_callable: route.to_callable,
                query: projected,
            };
            route_bytes = route_bytes.saturating_add(byte_len(&compact_route)?);
            compact.routes.push(compact_route);
            compact.page.returned = compact.routes.len();
            compact.page.next_cursor = (index + 1 < total).then(|| OutputCursor {
                offset: index + 1,
                ..cursor.clone()
            });
            // Size each route once. Re-serializing all prior routes here makes
            // page construction quadratic at large explicit delivery limits.
            let routes = std::mem::take(&mut compact.routes);
            let needed = byte_len(&SeededPathResponse::Compact(compact.clone()))?
                .saturating_add(route_bytes)
                .saturating_add(routes.len().saturating_sub(1));
            compact.routes = routes;
            if needed > options.byte_limit() {
                if compact.routes.len() == 1 {
                    return Err(QueryError::InvalidInput("single seeded route exceeds output.max_bytes; narrow the walk or raise the byte budget".into()));
                }
                compact.routes.pop();
                compact.page.returned = compact.routes.len();
                compact.page.next_cursor = Some(OutputCursor {
                    offset: index,
                    ..cursor.clone()
                });
                break;
            }
        }
        self.check_output_visibility(&cursor)?;
        let response = SeededPathResponse::Compact(compact);
        check_bytes(&response, &options)?;
        Ok(response)
    }
}

pub(super) fn compact_role(anchor: &SeedAnchor) -> CompactRole {
    CompactRole {
        assertion_id: anchor.assertion_id.clone(),
        role: anchor.role.clone(),
        epistemic_status: anchor.assertion.get().epistemic_status,
    }
}

fn search_truncated(result: &QueryResult) -> bool {
    match &result.payload {
        QueryPayload::Traverse(TraversePayload::Path(p)) => {
            p.bounds.truncated
                || p.search_receipt.fanout_truncated
                || p.search_receipt.expansion_budget_exhausted
                || p.search_receipt.edge_budget_exhausted
        }
        QueryPayload::Traverse(TraversePayload::Neighborhood(n)) => {
            n.search_receipt.fanout_truncated
                || n.search_receipt.expansion_budget_exhausted
                || n.search_receipt.edge_budget_exhausted
                || n.result_budget_exhausted
                || n.depth_frontier_count > 0
                || n.role_selection.as_ref().is_some_and(|r| r.truncated)
        }
        _ => false,
    }
}

fn detail_deferred(result: &QueryResult) -> bool {
    if result.receipt.references_truncated
        || result.receipt.providers_truncated
        || result.receipt.resolver_coverage_truncated
        || result.receipt.relations.truncated
        || result.receipt.unresolved.truncated
    {
        return true;
    }
    let path = |p: &TraversePath| {
        p.hops.iter().any(|h| {
            h.relation_claims_truncated
                || h.endpoint_assertion_bounds.truncated
                || h.unresolved_call_bounds.truncated
        })
    };
    match &result.payload {
        QueryPayload::Traverse(TraversePayload::Path(p)) => p.paths.iter().any(path),
        QueryPayload::Traverse(TraversePayload::Neighborhood(n)) => {
            n.unresolved_call_bounds.truncated
                || n.role_selection
                    .as_ref()
                    .is_some_and(|r| r.colocated_bounds.truncated)
                || n.items.iter().any(|i| {
                    path(&i.witness) || i.role_anchor_bounds.as_ref().is_some_and(|b| b.truncated)
                })
        }
        _ => {
            result.receipt.references_truncated
                || result.receipt.providers_truncated
                || result.receipt.resolver_coverage_truncated
        }
    }
}

fn compact_receipt(receipt: &QueryEvidenceReceipt) -> CompactEvidenceReceipt {
    let r = &receipt.references;
    CompactEvidenceReceipt {
        attachments: receipt.attachments.clone(),
        active_resolution_states: receipt.relations.active_resolution_states.clone(),
        reference_count: [
            &r.finding_ids,
            &r.assertion_ids,
            &r.artifact_ids,
            &r.trace_ids,
            &r.relation_edge_ids,
            &r.relation_claim_ids,
            &r.proposition_ids,
        ]
        .iter()
        .map(|v| v.len())
        .sum(),
        receipt_truncated: receipt.references_truncated
            || receipt.providers_truncated
            || receipt.resolver_coverage_truncated
            || receipt.relations.truncated
            || receipt.unresolved.truncated,
        providers: receipt
            .providers
            .iter()
            .map(|p| CompactProvider {
                name: p.name.clone(),
                version: p.version.clone(),
            })
            .collect(),
        resolver_coverage: receipt
            .resolver_coverage
            .iter()
            .map(|r| r.get().clone())
            .collect(),
        unresolved_reasons: receipt.unresolved.reasons.clone(),
        source_verification: receipt.source_verification.clone(),
    }
}

fn result_bounds(result: &QueryResult) -> (ResultBounds, Vec<QueryGapCode>) {
    macro_rules! bounds {
        ($v:expr) => {
            ($v.bounds.clone(), $v.gaps.clone())
        };
    }
    match &result.payload {
        QueryPayload::Find(v) => match v {
            FindPayload::Subject(v) => bounds!(v),
            FindPayload::Location(v) => bounds!(v),
            FindPayload::Finding(v) => bounds!(v),
            FindPayload::EvidenceRole(v) => bounds!(v),
            FindPayload::Facet(v) => bounds!(v),
        },
        QueryPayload::Inspect(v) => match v.as_ref() {
            InspectPayload::Subject(v) => bounds!(v),
            InspectPayload::Evidence(v) => bounds!(v),
            InspectPayload::Trace(v) => bounds!(v),
            InspectPayload::Proposition(v) => bounds!(v),
            InspectPayload::RelationClaims(v) => bounds!(v),
            InspectPayload::Calls(v) => bounds!(v),
            InspectPayload::Unresolved(v) => bounds!(v),
        },
        QueryPayload::Traverse(v) => match v {
            TraversePayload::Path(v) => bounds!(v),
            TraversePayload::Neighborhood(v) => bounds!(v),
        },
        QueryPayload::Read(ReadPayload::Source(v)) => bounds!(v),
    }
}

struct Projection<'a> {
    engine: &'a QueryEngine,
    items: Vec<CompactItem>,
    subjects: BTreeMap<String, CompactSubject>,
    edges: BTreeMap<String, CompactEdge>,
    scopes: BTreeMap<String, entmoot_contracts::Scope>,
}
impl<'a> Projection<'a> {
    fn new(engine: &'a QueryEngine) -> Self {
        Self {
            engine,
            items: vec![],
            subjects: BTreeMap::new(),
            edges: BTreeMap::new(),
            scopes: BTreeMap::new(),
        }
    }
    fn candidate(&mut self, n: &SubjectCandidate) -> String {
        self.subjects
            .entry(n.node_id.clone())
            .or_insert_with(|| CompactSubject {
                display_ref: n.display_ref.clone(),
                kind: n.kind.clone(),
                path: n.path.clone(),
                start_line: n.start_line,
                external_address: None,
            });
        n.node_id.clone()
    }
    fn address(&mut self, address: &Address) -> Result<String, QueryError> {
        let key = address_identity_key(address);
        let candidates = self.engine.indexes.nodes_by_address.get(&key);
        if let Some(index) = candidates.filter(|v| v.len() == 1).and_then(|v| v.first()) {
            let node = &self.engine.graph.nodes[*index];
            let candidate: SubjectCandidate =
                serde_json::from_value(super::subject_candidate(node, 0))?;
            return Ok(self.candidate(&candidate));
        }
        self.subjects
            .entry(key.clone())
            .or_insert_with(|| CompactSubject {
                display_ref: key.clone(),
                kind: "external_address".into(),
                path: None,
                start_line: None,
                external_address: Some(address.clone()),
            });
        Ok(key)
    }
    fn edge(
        &mut self,
        edge: &RelationEdge,
        hop: Option<&TraverseHop>,
    ) -> Result<String, QueryError> {
        let id = edge.id.as_str().to_owned();
        if self.edges.contains_key(&id) {
            return Ok(id);
        }
        let subject = self.address(&edge.subject)?;
        let object = self.address(&edge.object)?;
        let scope = self
            .scopes
            .iter()
            .find(|(_, s)| **s == edge.scope)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| format!("scope{}", self.scopes.len()));
        self.scopes
            .entry(scope.clone())
            .or_insert_with(|| edge.scope.clone());
        self.edges.insert(
            id.clone(),
            CompactEdge {
                subject,
                object,
                predicate: edge.predicate,
                scope,
                states: hop
                    .map(|h| {
                        serde_json::from_value(serde_json::to_value(&h.active_resolution_states)?)
                    })
                    .transpose()?,
                claim_count: hop.map(|h| h.relation_claim_count),
                active_claim_count: hop.map(|h| h.active_claim_count),
                endpoint_assertions: hop.map(|h| h.endpoint_assertion_bounds.total_matches),
                unresolved_calls: hop.map(|h| h.unresolved_call_bounds.total_matches),
            },
        );
        Ok(id)
    }
    fn path(
        &mut self,
        path: &TraversePath,
        endpoint: Option<&Address>,
        roles: &[SeedAnchor],
    ) -> Result<(), QueryError> {
        let edges = path
            .hops
            .iter()
            .map(|hop| self.edge(hop.relation_edge.get(), Some(hop)))
            .collect::<Result<Vec<_>, _>>()?;
        let endpoint = endpoint.map(|a| self.address(a)).transpose()?;
        self.items.push(CompactItem::Path {
            classification: path.classification,
            edges,
            endpoint,
            roles: roles
                .iter()
                .map(|a| CompactRole {
                    assertion_id: a.assertion_id.clone(),
                    role: a.role.clone(),
                    epistemic_status: a.assertion.get().epistemic_status,
                })
                .collect(),
        });
        Ok(())
    }
    fn evidence(
        &mut self,
        record_kind: CompactRecordKind,
        id: impl Into<String>,
        label: Option<String>,
    ) {
        self.items.push(CompactItem::Evidence {
            record_kind,
            id: id.into(),
            label,
        });
    }
    fn subject(&mut self, n: &SubjectCandidate) {
        let subject = self.candidate(n);
        self.items.push(CompactItem::Subject { subject });
    }
    fn collect(&mut self, payload: &QueryPayload) -> Result<(), QueryError> {
        match payload {
            QueryPayload::Find(v) => match v {
                FindPayload::Subject(v) => {
                    for n in &v.items {
                        self.subject(n);
                    }
                }
                FindPayload::Location(v) => {
                    for n in &v.selected {
                        self.subject(n);
                    }
                    if v.selected.is_empty() {
                        if let Some(n) = &v.file {
                            self.subject(n);
                        }
                    }
                }
                FindPayload::Facet(v) => {
                    for i in &v.items {
                        self.evidence(
                            CompactRecordKind::Facet,
                            i.id.clone(),
                            Some(i.label.as_ref().map_or_else(
                                || i.reason.clone(),
                                |label| {
                                    format!(
                                        "{}: {label} ({})",
                                        i.path.as_deref().unwrap_or(""),
                                        i.reason
                                    )
                                },
                            )),
                        );
                    }
                }
                FindPayload::Finding(v) => {
                    for i in &v.items {
                        self.evidence(
                            CompactRecordKind::Finding,
                            &i.finding.get().observation_id,
                            i.finding.get().message.clone(),
                        );
                    }
                }
                FindPayload::EvidenceRole(v) => {
                    for i in &v.items {
                        self.evidence(
                            CompactRecordKind::Assertion,
                            &i.assertion.get().assertion_id,
                            Some(format!(
                                "{} ({:?})",
                                i.role,
                                i.assertion.get().epistemic_status
                            )),
                        );
                    }
                }
            },
            QueryPayload::Traverse(v) => match v {
                TraversePayload::Path(v) => {
                    for p in &v.paths {
                        self.path(p, None, &[])?;
                    }
                }
                TraversePayload::Neighborhood(v) => {
                    for i in &v.items {
                        self.path(&i.witness, Some(i.subject.get()), &i.role_anchors)?;
                        if let Some(CompactItem::Path { roles, .. }) = self.items.last_mut() {
                            *roles = i.role_summaries.clone();
                        }
                    }
                }
            },
            QueryPayload::Read(ReadPayload::Source(v)) => {
                for line in &v.items {
                    self.items.push(CompactItem::SourceLine {
                        line: line.line,
                        text: line.text.clone(),
                    });
                }
            }
            QueryPayload::Inspect(v) => match v.as_ref() {
                InspectPayload::Subject(v) => {
                    if let SubjectInspectTarget::Resolved(n) = &v.subject {
                        self.subject(n);
                    }
                    for n in &v.items {
                        self.subject(n);
                    }
                    for n in &v.neighbors {
                        self.subject(n);
                    }
                    for e in &v.relations {
                        let edge = self.edge(e.get(), None)?;
                        self.items.push(CompactItem::Relation { edge });
                    }
                    for i in &v.direct_findings {
                        self.evidence(
                            CompactRecordKind::Finding,
                            &i.finding.get().observation_id,
                            i.finding.get().message.clone(),
                        );
                    }
                    for i in &v.unresolved_relations {
                        self.evidence(
                            CompactRecordKind::Unresolved,
                            i.get().id.as_str(),
                            Some(format!("{:?}", i.get().reason)),
                        );
                    }
                }
                InspectPayload::Evidence(v) => match &v.body {
                    EvidenceInspectBody::Finding { finding, .. } => self.evidence(
                        CompactRecordKind::Finding,
                        &finding.get().observation_id,
                        finding.get().message.clone(),
                    ),
                    EvidenceInspectBody::Assertions { assertions } => {
                        for a in assertions {
                            self.evidence(
                                CompactRecordKind::Assertion,
                                &a.get().assertion_id,
                                Some(format!(
                                    "{} ({:?})",
                                    a.get().predicate,
                                    a.get().epistemic_status
                                )),
                            );
                        }
                    }
                    EvidenceInspectBody::Missing { .. } => {}
                },
                InspectPayload::Trace(v) => {
                    for i in &v.items {
                        self.evidence(CompactRecordKind::TraceStep, &i.step.get().step_id, None);
                    }
                }
                InspectPayload::Proposition(v) => {
                    if v.proposition.is_some() {
                        self.evidence(CompactRecordKind::Proposition, &v.proposition_id, None);
                    }
                    for e in &v.occurrences {
                        let edge = self.edge(e.get(), None)?;
                        self.items.push(CompactItem::Relation { edge });
                    }
                    for c in &v.relation_claims {
                        self.evidence(
                            CompactRecordKind::RelationClaim,
                            c.get().id.as_str(),
                            Some(format!("{:?}", c.get().state)),
                        );
                    }
                }
                InspectPayload::RelationClaims(v) => {
                    for c in &v.relation_claims {
                        self.evidence(
                            CompactRecordKind::RelationClaim,
                            c.get().id.as_str(),
                            Some(format!("{:?}", c.get().state)),
                        );
                    }
                }
                InspectPayload::Calls(v) => {
                    for i in &v.items {
                        let edge = self.edge(i.relation.get(), None)?;
                        if let Some(e) = self.edges.get_mut(&edge) {
                            e.claim_count = Some(i.claims.len());
                            e.active_claim_count = Some(
                                i.claims
                                    .iter()
                                    .filter(|c| {
                                        c.get().state
                                            == entmoot_contracts::RelationClaimState::Active
                                    })
                                    .count(),
                            );
                            e.states = Some(
                                i.claims
                                    .iter()
                                    .filter_map(|c| {
                                        super::applicable_resolution_state(
                                            c.get(),
                                            i.relation.get(),
                                        )
                                    })
                                    .collect(),
                            );
                        }
                        self.items.push(CompactItem::Relation { edge });
                    }
                }
                InspectPayload::Unresolved(v) => {
                    for i in &v.items {
                        self.evidence(
                            CompactRecordKind::Unresolved,
                            i.get().id.as_str(),
                            Some(format!("{:?}", i.get().reason)),
                        );
                    }
                }
            },
        }
        Ok(())
    }
    fn dictionary_for(&self, response: &mut CompactQueryResponse) {
        let mut subjects = BTreeSet::new();
        let mut edges = BTreeSet::new();
        for item in &response.items {
            match item {
                CompactItem::Subject { subject } => {
                    subjects.insert(subject.clone());
                }
                CompactItem::Path {
                    edges: ids,
                    endpoint,
                    ..
                } => {
                    edges.extend(ids.iter().cloned());
                    subjects.extend(endpoint.iter().cloned());
                }
                CompactItem::Relation { edge } => {
                    edges.insert(edge.clone());
                }
                _ => {}
            }
        }
        for id in edges {
            if let Some(edge) = self.edges.get(&id) {
                subjects.insert(edge.subject.clone());
                subjects.insert(edge.object.clone());
                if let Some(scope) = self.scopes.get(&edge.scope) {
                    response.scopes.insert(edge.scope.clone(), scope.clone());
                }
                response.edges.insert(id, edge.clone());
            }
        }
        for id in subjects {
            if let Some(subject) = self.subjects.get(&id) {
                response.subjects.insert(id, subject.clone());
            }
        }
    }
}

fn traversal_receipt(result: &QueryResult) -> Option<&TraverseSearchReceipt> {
    match &result.payload {
        QueryPayload::Traverse(TraversePayload::Path(path)) => Some(&path.search_receipt),
        QueryPayload::Traverse(TraversePayload::Neighborhood(neighborhood)) => {
            Some(&neighborhood.search_receipt)
        }
        _ => None,
    }
}
