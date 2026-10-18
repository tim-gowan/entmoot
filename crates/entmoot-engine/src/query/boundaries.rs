//! Query source syntax and independently attributed native boundary observations.
use super::{QueryEngine, QueryError, QUERY_RESULT_SCHEMA};
use entmoot_contracts::query::{CallBoundaryRequest, CallGraphCompleteness};
use entmoot_contracts::{ArtifactFormat, ContentDigest, RelationKind};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn token(value: &impl serde::Serialize) -> Result<String, QueryError> {
    Ok(serde_json::to_value(value)?
        .as_str()
        .unwrap_or("unknown")
        .to_owned())
}

// Preserve the public star-only syntax while delegating matching to globset.
fn compile_callee(pattern: &str) -> Result<globset::GlobMatcher, QueryError> {
    if pattern.len() > 1024 {
        return Err(QueryError::InvalidInput(
            "callee pattern exceeds 1024 bytes".into(),
        ));
    }
    let pattern = pattern.strip_suffix("()").unwrap_or(pattern);
    let escaped = pattern
        .split('*')
        .map(globset::escape)
        .collect::<Vec<_>>()
        .join("*");
    Ok(globset::GlobBuilder::new(&escaped)
        .literal_separator(false)
        .backslash_escape(true)
        .build()
        .map_err(|e| QueryError::InvalidInput(format!("invalid callee pattern: {e}")))?
        .compile_matcher())
}
#[cfg(test)]
fn matches_callee(pattern: &str, value: &str) -> bool {
    compile_callee(pattern).unwrap().is_match(value)
}

impl QueryEngine {
    pub(super) fn call_boundaries(
        &self,
        request: &CallBoundaryRequest,
    ) -> Result<Value, QueryError> {
        let path_filter = super::selection::CompiledPath::from_request(
            request.path_selector.as_ref(),
            request.path.as_deref(),
        )?;
        let callee_filter = request.callee.as_deref().map(compile_callee).transpose()?;
        let limit = request.limit.unwrap_or(50);
        if !(1..=1000).contains(&limit) || request.callee.as_ref().is_some_and(|s| s.len() > 1024) {
            return Err(QueryError::InvalidInput(
                "boundary limit must be 1..=1000; callee pattern at most 1024 bytes".into(),
            ));
        }
        let objects = crate::FilesystemArtifactObjectRepository::new(
            crate::workspace::artifact_store_path(&self.store_path)
                .map_err(|e| QueryError::Unavailable(e.to_string()))?,
        );
        let mut inputs = Vec::new();
        let mut limitations = Vec::new();
        let mut remaining = 256 * 1024 * 1024_u64;
        for manifest in self
            .graph
            .artifacts
            .iter()
            .filter(|m| m.format == ArtifactFormat::Resolver)
        {
            let digest = ContentDigest::parse(&manifest.content_digest)
                .map_err(|e| QueryError::InvalidInput(e.to_string()))?;
            match objects.get_bounded(&digest, remaining.min(64 * 1024 * 1024)) {
                Ok(bytes) => {
                    remaining -= bytes.len() as u64;
                    inputs.push((manifest, bytes));
                }
                Err(crate::ArtifactStoreError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    limitations.push(format!(
                        "native_detail_not_retained:{}",
                        manifest.artifact_id
                    ))
                }
                Err(error) => return Err(QueryError::Unavailable(error.to_string())),
            }
        }
        let audit = crate::resolver_accounting::audit_resolvers(
            &self.graph,
            &inputs
                .iter()
                .map(|(m, b)| (*m, b.as_slice()))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| QueryError::Unavailable(e.to_string()))?;
        let artifacts: BTreeMap<_, _> = audit
            .artifacts
            .iter()
            .map(|a| (a.manifest.artifact_id.as_str(), a))
            .collect();
        let sites: BTreeMap<_, _> = audit
            .call_sites
            .iter()
            .map(|s| (s.node_id.as_str(), s))
            .collect();
        if request.build_context.as_ref().is_some_and(|context| {
            !audit
                .artifacts
                .iter()
                .any(|a| &a.build_context_digest == context)
        }) {
            return Err(QueryError::InvalidInput(
                "build context is not present in retained evidence".into(),
            ));
        }
        let files: BTreeMap<_, _> = self
            .graph
            .snapshot_files
            .iter()
            .map(|f| (f.path.as_str(), &f.content_digest))
            .collect();
        let mut attempts = BTreeMap::<&str, Vec<&entmoot_contracts::UnresolvedRelation>>::new();
        for attempt in &self.graph.unresolved_relations {
            if attempt.predicate != RelationKind::Calls
                || request.build_context.as_ref().is_some_and(|context| {
                    attempt
                        .scope
                        .context_id
                        .as_deref()
                        .is_some_and(|id| id != context.as_str())
                })
            {
                continue;
            }
            for key in [
                attempt.occurrence_id.as_deref(),
                attempt.scope.occurrence_id.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            {
                attempts.entry(key).or_default().push(attempt);
            }
        }
        let mut items = Vec::new();
        let mut counts = BTreeMap::<(String, String), usize>::new();
        for node in &self.graph.nodes {
            let Some(syntax) = &node.call_site else {
                continue;
            };
            if callee_filter
                .as_ref()
                .is_some_and(|p| !p.is_match(&syntax.callee))
                || path_filter
                    .as_ref()
                    .is_some_and(|p| !node.path.as_deref().is_some_and(|path| p.matches(path)))
            {
                continue;
            }
            let evaluations: Vec<_> = sites
                .get(node.id.as_str())
                .into_iter()
                .flat_map(|s| &s.evaluations)
                .filter(|e| {
                    request.build_context.as_ref().is_none_or(|context| {
                        artifacts
                            .get(e.artifact_id.as_str())
                            .is_some_and(|a| &a.build_context_digest == context)
                    })
                })
                .collect();
            let unresolved = attempts.get(node.id.as_str()).cloned().unwrap_or_default();
            let mut outcomes = BTreeSet::new();
            for evaluation in &evaluations {
                outcomes.insert(token(&evaluation.observation.outcome)?);
            }
            if outcomes.is_empty() {
                outcomes.insert("unreported".into());
            }
            if request
                .outcome
                .as_ref()
                .is_some_and(|o| !outcomes.contains(o))
            {
                continue;
            }
            if let Some(reason) = &request.reason {
                let native = evaluations.iter().any(|e| match &e.observation.dispatch {
                    entmoot_contracts::CallDispatch::Open { reason: gap } => {
                        gap.category() == reason
                    }
                    _ => false,
                });
                if !native
                    && !unresolved
                        .iter()
                        .any(|a| entmoot_contracts::unresolved_reason_key(a.reason) == reason)
                {
                    continue;
                }
            }
            for outcome in outcomes {
                *counts.entry((syntax.callee.clone(), outcome)).or_default() += 1;
            }
            items.push((node, syntax, evaluations, unresolved));
        }
        items.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        let total = items.len();
        let page: Vec<_> = items.into_iter().skip(request.offset).take(limit).map(|(node,syntax,evaluations,unresolved)| {
            let digest = node.path.as_deref().and_then(|p| files.get(p));
            json!({"id":node.id,"kind":"call_site","label":syntax.callee,"path":node.path,
                "reason":"source_call_boundary", "boundary":{"syntax":syntax,"content_digest":digest,
                "evaluations":evaluations,"unresolved":unresolved,"limitations":limitations}})
        }).collect();
        let truncated = request.offset.saturating_add(page.len()) < total;
        let counts: Vec<_> = counts.into_iter().map(|((callee,outcome),sites)| json!({"callee":callee,"outcome":outcome,"sites":sites})).collect();
        Ok(
            json!({"schema_version":QUERY_RESULT_SCHEMA,"operation":"find","view":"facet","facet":"call_boundary",
            "snapshot":self.graph.snapshot,"status":if truncated {"truncated"} else if total == 0 {"unknown"} else {"known"},
            "items":page,"boundary_counts":counts,"limitations":limitations,"completeness":CallGraphCompleteness::default(),
            "bounds":{"offset":request.offset,"limit":limit,"total_matches":total,"truncated":truncated},
            "gaps":["repository_call_graph_completeness_not_established","dependency_call_graph_completeness_not_established","native_library_call_graph_completeness_not_established"]}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callee_selection_is_syntax_not_native_type_inference() {
        assert!(matches_callee("*.toString()", "object.toString"));
        assert!(matches_callee("object.toString()", "object.toString"));
        assert!(!matches_callee("object.toString()", "other.toString"));
        assert!(!matches_callee("*.toString", "toString"));
        assert!(matches_callee("*", "x.y"));
    }
}
