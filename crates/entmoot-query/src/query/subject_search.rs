//! Subject matching and bounded ranking shared by retained and graph reads.
use super::*;
use entmoot_contracts::query::{SelectionMatch, SelectionReceipt, SubjectSelector};

pub(super) struct SubjectSearch<'a> {
    query: Option<&'a str>,
    lower: Option<String>,
    exact: Vec<String>,
    pub path: Option<String>,
    line: Option<usize>,
    limit: usize,
    selector: Option<&'a SubjectSelector>,
    compiled: Option<selection::CompiledSelector>,
    receipt: SelectionReceipt,
    total: usize,
    best: Vec<(TopologyNode, u8, Option<SelectionMatch>)>,
}

impl<'a> SubjectSearch<'a> {
    pub fn new(
        query: Option<&'a str>,
        path: Option<&str>,
        line: Option<usize>,
        limit: Option<usize>,
        selector: Option<&'a SubjectSelector>,
    ) -> Result<Self, QueryError> {
        let compiled = selector.map(selection::CompiledSelector::new).transpose()?;
        let query = query.map(str::trim).filter(|s| !s.is_empty());
        let path = path
            .map(|p| p.trim().replace('\\', "/"))
            .filter(|p| !p.is_empty());
        if query.is_none() && path.is_none() && selector.is_none() {
            return Err(QueryError::InvalidInput(
                "query, path or selector is required".into(),
            ));
        }
        Ok(Self {
            query,
            lower: query.map(str::to_lowercase),
            exact: query.map_or_else(Vec::new, structural_hash_exact_selectors),
            path,
            line,
            limit: bounded_limit(limit),
            selector,
            compiled,
            receipt: Default::default(),
            total: 0,
            best: Vec::new(),
        })
    }

    fn score(&self, node: &TopologyNode) -> Option<u8> {
        if self
            .path
            .as_deref()
            .is_some_and(|p| node.path.as_deref() != Some(p))
        {
            return None;
        }
        let Some(raw) = self.query else {
            return Some(1);
        };
        let exact = |key: &str| {
            node.id == key
                || node.display_ref == key
                || node.address_id.as_deref() == Some(key)
                || node.logic_address.as_deref() == Some(key)
                || node
                    .structural_hash
                    .as_deref()
                    .is_some_and(|h| structural_hash_exact_selectors(h).iter().any(|s| s == key))
        };
        if !self.exact.is_empty() {
            return self.exact.iter().any(|key| exact(key)).then_some(4);
        }
        if exact(raw) {
            return Some(4);
        }
        let lower = self.lower.as_deref().unwrap_or_default();
        if node.label.to_lowercase() == lower
            || node.fqn.as_ref().is_some_and(|f| f.to_lowercase() == lower)
        {
            return Some(3);
        }
        searchable_node_text(node).contains(lower).then_some(2)
    }

    pub fn consider(&mut self, node: &TopologyNode) {
        let Some(score) = self.score(node) else {
            return;
        };
        let explanation = if let Some(compiled) = &self.compiled {
            self.receipt.examined += 1;
            let (state, explanation) = compiled.matches(node);
            match state {
                selection::MatchState::Yes => {
                    self.receipt.matched += 1;
                    Some(explanation)
                }
                selection::MatchState::Unknown => {
                    self.receipt.unknown += 1;
                    return;
                }
                selection::MatchState::No => return,
            }
        } else {
            None
        };
        self.total += 1;
        // Keep only the requested best cards; exact counts do not require
        // retaining every matching node or selector explanation.
        let key = (
            std::cmp::Reverse(score),
            line_distance(node, self.line),
            &node.id,
        );
        let at = self.best.partition_point(|(other, rank, _)| {
            (
                std::cmp::Reverse(*rank),
                line_distance(other, self.line),
                &other.id,
            ) < key
        });
        if at < self.limit {
            self.best.insert(at, (node.clone(), score, explanation));
            self.best.truncate(self.limit);
        }
    }

    pub fn finish(self, snapshot: &entmoot_contracts::SourceSnapshot) -> Value {
        let items: Vec<_> = self
            .best
            .into_iter()
            .map(|(node, score, explanation)| {
                let mut card = subject_candidate(&node, score);
                if let Some(explanation) = explanation {
                    card["selection_match"] = json!(explanation);
                }
                card
            })
            .collect();
        json!({ "schema_version": QUERY_RESULT_SCHEMA, "operation": "find", "view": "subject", "snapshot": snapshot,
            "status": if self.total > self.limit { "truncated" } else if self.total == 0 { "unknown" } else { "known" },
            "selection": self.selector.map(|_| self.receipt), "items": items,
            "gaps": if self.total == 0 { vec!["no_inventory_subject_matched"] } else { Vec::<&str>::new() },
            "bounds": ResultBounds::page(self.limit, self.limit.min(self.total), self.total) })
    }
}

impl QueryEngine {
    pub(super) fn retained_subject_find(
        &self,
        request: &FindQuery,
    ) -> Result<Option<QueryResult>, QueryError> {
        let FindQuery::Subject {
            selector,
            query,
            path,
            line,
            limit,
        } = request
        else {
            return Ok(None);
        };
        let mut search = SubjectSearch::new(
            query.as_deref(),
            path.as_deref(),
            *line,
            *limit,
            selector.as_ref(),
        )?;
        let repository = self.repository()?;
        let mut after = None;
        loop {
            let Some(page) = repository.inventory_nodes_matching(
                search.path.as_deref(),
                &search.exact,
                after,
                512,
            )?
            else {
                return Ok(None);
            };
            for node in page.nodes {
                search.consider(&node);
            }
            after = page.next_ordinal;
            if after.is_none() {
                break;
            }
        }
        self.finish_result(search.finish(&self.graph.snapshot))
            .map(Some)
    }
}
