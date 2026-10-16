//! Scope-free structural decisions with the exact symbol-index reads that
//! justify replay. Absent and ambiguous lookup results are inputs too.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CallSummary {
    reads: BTreeSet<LookupKey>,
    digest: String,
    decisions: Vec<CallDecision>,
}

impl CallSummary {
    pub(crate) fn build(
        index: &StructuralCallIndex,
        path: &str,
        record: Option<&FileIndexRecord>,
    ) -> Self {
        let mut lookup = Lookup {
            index,
            reads: Some(BTreeSet::new()),
        };
        let decisions = record
            .map(|record| resolve_repo_calls(&[(path, record)]))
            .unwrap_or_default()
            .iter()
            .map(|call| decide_call(call, &mut lookup))
            .collect();
        let reads = lookup.reads.unwrap_or_default();
        let digest = index.digest_reads(&reads);
        Self {
            reads,
            digest,
            decisions,
        }
    }

    pub(crate) fn matches(&self, index: &StructuralCallIndex) -> bool {
        self.digest == index.digest_reads(&self.reads)
    }

    pub(crate) fn apply(&self, graph: &mut TopologyGraph) {
        for decision in &self.decisions {
            decision.apply(graph);
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CallDecision {
    pub caller: String,
    pub occurrence: String,
    pub outcome: CallOutcome,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum CallOutcome {
    Resolved {
        target: String,
        confidence: ResolveConfidence,
    },
    Unresolved {
        target: TargetConstraint,
        reason: UnresolvedReason,
    },
}

impl CallDecision {
    pub(super) fn apply(&self, graph: &mut TopologyGraph) {
        match &self.outcome {
            CallOutcome::Resolved { target, confidence } => {
                graph.try_add_edge(StructuralEdge::resolved_call(
                    &self.caller,
                    target,
                    *confidence,
                ));
                record_structural_resolved_call(graph, &self.caller, target);
                crate::relation_claims::record_structural_resolved_call_site(
                    graph,
                    &self.caller,
                    target,
                    &self.occurrence,
                );
            }
            CallOutcome::Unresolved { target, reason } => record_structural_unresolved_call(
                graph,
                &self.caller,
                target.clone(),
                *reason,
                &self.occurrence,
            ),
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
enum LookupKey {
    PathName(String, String),
    Fqn(String),
    Callable(String),
}

/// Every read used by target selection passes through this context. References
/// borrow the immutable index, so tracking another read cannot invalidate them.
pub(super) struct Lookup<'a> {
    index: &'a StructuralCallIndex,
    reads: Option<BTreeSet<LookupKey>>,
}
impl<'a> Lookup<'a> {
    pub(super) fn plain(index: &'a StructuralCallIndex) -> Self {
        Self { index, reads: None }
    }
    pub(super) fn path_name(&mut self, path: &str, name: &str) -> Option<&'a Vec<String>> {
        if let Some(reads) = &mut self.reads {
            reads.insert(LookupKey::PathName(path.into(), name.into()));
        }
        self.index.by_path_name.get(&(path.into(), name.into()))
    }
    pub(super) fn fqn(&mut self, fqn: &str) -> Option<&'a str> {
        if let Some(reads) = &mut self.reads {
            reads.insert(LookupKey::Fqn(fqn.into()));
        }
        unique_node(&self.index.by_fqn, fqn)
    }
    pub(super) fn callable(&mut self, id: &str) -> bool {
        if let Some(reads) = &mut self.reads {
            reads.insert(LookupKey::Callable(id.into()));
        }
        self.index.callable_ids.contains(id)
    }
}

impl StructuralCallIndex {
    fn digest_reads(&self, reads: &BTreeSet<LookupKey>) -> String {
        // Length-framed UTF-8 and explicit variant tags preserve empty/absent
        // distinctions without serializing the complete index for every file.
        fn text(hash: &mut blake3::Hasher, value: &str) {
            hash.update(&(value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        }
        let mut hash = blake3::Hasher::new();
        hash.update(b"entmoot.structural_call_inputs.v1");
        hash.update(&(reads.len() as u64).to_le_bytes());
        for key in reads {
            match key {
                LookupKey::PathName(path, name) => {
                    hash.update(&[0]);
                    text(&mut hash, path);
                    text(&mut hash, name);
                    match self.by_path_name.get(&(path.clone(), name.clone())) {
                        None => {
                            hash.update(&[0]);
                        }
                        Some(ids) => {
                            hash.update(&[1]);
                            hash.update(&(ids.len() as u64).to_le_bytes());
                            for id in ids {
                                text(&mut hash, id);
                            }
                        }
                    }
                }
                LookupKey::Fqn(fqn) => {
                    hash.update(&[1]);
                    text(&mut hash, fqn);
                    match self.by_fqn.get(fqn) {
                        None => {
                            hash.update(&[0]);
                        }
                        Some(UniqueNodeId::Ambiguous) => {
                            hash.update(&[1]);
                        }
                        Some(UniqueNodeId::One(id)) => {
                            hash.update(&[2]);
                            text(&mut hash, id);
                        }
                    }
                }
                LookupKey::Callable(id) => {
                    hash.update(&[2]);
                    text(&mut hash, id);
                    hash.update(&[u8::from(self.callable_ids.contains(id))]);
                }
            }
        }
        format!("blake3:{}", hash.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> ResolvedCall {
        ResolvedCall {
            file: "client.ts".into(),
            start_byte: 10,
            end_byte: 16,
            syntactic_callee: "leaf".into(),
            owner: None,
            occurrence_fqn: None,
            resolved_fqn: Some("./leaf.leaf".into()),
            confidence: ResolveConfidence::ImportLocal,
        }
    }

    fn index(ts: bool, js: bool, fallback: usize) -> StructuralCallIndex {
        let mut index = StructuralCallIndex {
            by_path_name: HashMap::new(),
            by_fqn: HashMap::new(),
            callable_ids: HashSet::new(),
        };
        for (exists, path, id) in [(ts, "leaf.ts", "ts"), (js, "leaf.js", "js")] {
            if exists {
                index
                    .by_path_name
                    .insert((path.into(), "leaf".into()), vec![id.into()]);
            }
        }
        match fallback {
            1 => {
                index
                    .by_fqn
                    .insert("./leaf.leaf".into(), UniqueNodeId::One("fallback".into()));
            }
            2 => {
                index
                    .by_fqn
                    .insert("./leaf.leaf".into(), UniqueNodeId::Ambiguous);
            }
            _ => {}
        }
        index
    }

    fn summary(index: &StructuralCallIndex) -> CallSummary {
        let mut lookup = Lookup {
            index,
            reads: Some(BTreeSet::new()),
        };
        let decision = decide_call(&call(), &mut lookup);
        let reads = lookup.reads.unwrap();
        CallSummary {
            digest: index.digest_reads(&reads),
            reads,
            decisions: vec![decision],
        }
    }

    #[test]
    fn finite_revision_pairs_preserve_decisions_only_when_observed_inputs_match() {
        let states: Vec<_> = [false, true]
            .into_iter()
            .flat_map(|ts| {
                [false, true]
                    .into_iter()
                    .flat_map(move |js| (0..3).map(move |f| (ts, js, f)))
            })
            .collect();
        for &(ts, js, f) in &states {
            let before = summary(&index(ts, js, f));
            match (&before.decisions[0].outcome, ts, js, f) {
                (CallOutcome::Resolved { target, .. }, true, false, _) => assert_eq!(target, "ts"),
                (CallOutcome::Resolved { target, .. }, false, true, _) => assert_eq!(target, "js"),
                (CallOutcome::Resolved { target, .. }, false, false, 1) => {
                    assert_eq!(target, "fallback")
                }
                (CallOutcome::Unresolved { reason, .. }, true, true, _) => {
                    assert_eq!(*reason, UnresolvedReason::AmbiguousTarget)
                }
                (CallOutcome::Unresolved { reason, .. }, false, false, _) => {
                    assert_eq!(*reason, UnresolvedReason::TargetNotInInventory)
                }
                _ => panic!("unexpected independently specified target"),
            }
            let decoded: CallSummary =
                serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
            for &(next_ts, next_js, next_f) in &states {
                let after = index(next_ts, next_js, next_f);
                if decoded.matches(&after) {
                    assert_eq!(
                        serde_json::to_value(&decoded.decisions).unwrap(),
                        serde_json::to_value(summary(&after).decisions).unwrap()
                    );
                }
                if (ts, js) != (next_ts, next_js) {
                    assert!(
                        !decoded.matches(&after),
                        "missing/ambiguous path changes must invalidate"
                    );
                }
                if ts || js {
                    assert_eq!(
                        decoded.matches(&after),
                        (ts, js) == (next_ts, next_js),
                        "unread fallback must not invalidate"
                    );
                }
            }
            let mut unrelated = index(ts, js, f);
            unrelated
                .by_path_name
                .insert(("other.ts".into(), "leaf".into()), vec!["unrelated".into()]);
            unrelated
                .by_fqn
                .insert("other.leaf".into(), UniqueNodeId::One("unrelated".into()));
            unrelated.callable_ids.insert("unrelated".into());
            assert!(decoded.matches(&unrelated));
        }
    }

    #[test]
    fn digest_distinguishes_lookup_presence_order_and_callable_membership() {
        let mut index = index(false, false, 0);
        let reads = BTreeSet::from([
            LookupKey::PathName("leaf.ts".into(), "leaf".into()),
            LookupKey::Callable("caller".into()),
        ]);
        let mut seen = HashSet::new();
        assert!(seen.insert(index.digest_reads(&reads)));
        for ids in [
            vec![],
            vec!["a".into()],
            vec!["a".into(), "b".into()],
            vec!["b".into(), "a".into()],
        ] {
            index
                .by_path_name
                .insert(("leaf.ts".into(), "leaf".into()), ids);
            assert!(seen.insert(index.digest_reads(&reads)));
        }
        index.callable_ids.insert("caller".into());
        assert!(seen.insert(index.digest_reads(&reads)));
    }
}
