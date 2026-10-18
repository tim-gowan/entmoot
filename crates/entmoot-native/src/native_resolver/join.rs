//! Exact-span join from native raw facts onto inventoried call stubs.
//!
//! Joins attributable provider facts to production inventory without creating identities.

use std::collections::BTreeMap;

use entmoot_contracts::{
    display_from_path, path_from_go_qualified_name, purl_address, purl_golang_module,
    purl_golang_stdlib, purl_npm, purl_pypi, topology_node_address, Address, CallDispatch,
    DispatchGap, ProviderCallableKind, ProviderCallableRef, ProviderPackageRef,
    ProviderPackageRole, ProviderPathKind, ProviderPathSegment, ProviderSymbolSpelling,
    ResolverCallSource, ResolverCallableAssociation, ResolverDeclaration, ResolverDeclarationKind,
    ResolverDeclarationOrigin, ResolverObservation, ResolverObservationOutcome, ResolverTarget,
    PROVIDER_CALLABLE_REF_SCHEMA,
};
use entmoot_contracts::{
    NativeCallFact, NativeCallTarget, NativeRawObservation, NativeReferenceFact, NativeSpanRef,
};

use crate::graph::{NodeKind, TopologyGraph, TopologyNode};

pub const METHOD_ID: &str = "entmoot.native_compiler_exact_spans.v1";

/// Positive reference uses only; provider strings never close the use universe.
pub fn normalize_reference_uses(
    graph: &TopologyGraph,
    raw: &NativeRawObservation,
    language: &str,
) -> Vec<entmoot_contracts::AssessmentObservation> {
    use entmoot_contracts::*;
    if raw.invalid_environment || raw.budget_exceeded {
        return vec![];
    }
    let mut declarations = BTreeMap::<_, Vec<_>>::new();
    let files: BTreeMap<_, _> = graph
        .snapshot_files
        .iter()
        .map(|f| (f.path.as_str(), &f.content_digest))
        .collect();
    for node in &graph.nodes {
        if node.kind != NodeKind::Callable {
            continue;
        }
        if let (Some(path), Some(start), Some(end)) =
            (node.path.as_deref(), node.start_byte, node.end_byte)
        {
            declarations
                .entry((path, start, end))
                .or_default()
                .push(node);
        }
    }
    let mut sites = BTreeMap::<_, Vec<_>>::new();
    for reference in &raw.references {
        let group = sites
            .entry((
                &reference.site.path,
                reference.site.span.start_byte,
                reference.site.span.end_byte,
            ))
            .or_default();
        if group.len() < 2 && !group.contains(&reference) {
            group.push(reference);
        }
    }
    let mut observations = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for refs in sites.values() {
        if refs.len() != 1 {
            continue;
        }
        let reference = refs[0];
        if !matches!(reference.kind.as_str(), "reference" | "import")
            || reference.gap.is_some()
            || reference.targets.len() != 1
            || !language_matches(language, &reference.site.path)
            || !files.contains_key(reference.site.path.as_str())
        {
            continue;
        }
        let target = &reference.targets[0];
        if target.kind != "callable" {
            continue;
        }
        let Some(site) = target.location.repository() else {
            continue;
        };
        let Some(nodes) =
            declarations.get(&(site.path.as_str(), site.span.start_byte, site.span.end_byte))
        else {
            continue;
        };
        if nodes.len() != 1 {
            continue;
        }
        let Some(logic_address) = &nodes[0].logic_address else {
            continue;
        };
        let Some(digest) = files.get(site.path.as_str()) else {
            continue;
        };
        let (Ok(occurrence), Ok(source_digest)) = (
            OccurrenceRef::new(logic_address.clone()),
            ContentDigest::parse((*digest).clone()),
        ) else {
            continue;
        };
        let observation = AssessmentObservation::CodeUse {
            subject: AssessmentDeclaration {
                path: site.path.clone(),
                source_digest,
                declaration: SourceOccurrenceRef {
                    occurrence,
                    span: site.span,
                },
            },
            usage: CodeUseKind::NonCallReference,
        };
        if seen.insert(logic_address) {
            observations.push(observation);
        }
    }
    observations
}

pub use entmoot_core::provider_language::language_matches;

fn declaration_for_target_kind(kind: &str) -> ResolverDeclaration {
    let (decl_kind, association) = match kind {
        "callable" => (
            ResolverDeclarationKind::Callable,
            ResolverCallableAssociation::Direct,
        ),
        "callable_signature" => (
            ResolverDeclarationKind::Callable,
            ResolverCallableAssociation::Unknown,
        ),
        "type" => (
            ResolverDeclarationKind::Type,
            ResolverCallableAssociation::Unknown,
        ),
        "module" => (
            ResolverDeclarationKind::Module,
            ResolverCallableAssociation::Unknown,
        ),
        "intrinsic" | "value" => (
            ResolverDeclarationKind::Value,
            ResolverCallableAssociation::Direct,
        ),
        _ => (
            ResolverDeclarationKind::Unknown,
            ResolverCallableAssociation::Unknown,
        ),
    };
    ResolverDeclaration {
        kind: decl_kind,
        origin: ResolverDeclarationOrigin::Source,
        callable_association: association,
        classification_method_id: METHOD_ID.into(),
        provider_kind: Some(kind.to_owned()),
    }
}

/// Select an explicit callee anchored at this site, never an argument or import.
fn covering_reference<'a>(
    refs: &BTreeMap<(&str, usize), Vec<&'a NativeReferenceFact>>,
    path: &str,
    start_byte: usize,
    end_byte: usize,
) -> Option<&'a NativeReferenceFact> {
    let mut matches = refs
        .get(&(path, start_byte))?
        .iter()
        .copied()
        .filter(|reference| reference.site.span.end_byte <= end_byte);
    let only = matches.next()?;
    (matches.next().is_none() && only.site.span.start_byte <= only.site.span.end_byte)
        .then_some(only)
}

fn build_resolver_target(
    language: &str,
    observation_index: usize,
    target_index: usize,
    target: &NativeCallTarget,
    declarations: &BTreeMap<(String, usize, usize), Vec<&TopologyNode>>,
) -> ResolverTarget {
    let local = target.location.repository();
    let address = if let Some(site) = local {
        Address::RepoPath {
            repository_id: None,
            path: site.path.clone(),
            start_byte: u32::try_from(site.span.start_byte).ok(),
            end_byte: u32::try_from(site.span.start_byte).ok(),
        }
    } else {
        Address::Opaque {
            system: format!("{language}.compiler.symbol"),
            value: target.symbol.clone(),
        }
    };
    let native = ProviderSymbolSpelling {
        scheme: format!("{language}.compiler.symbol"),
        // Reference spelling may be a local alias. Native target identity must
        // retain the compiler's declaration symbol; source spelling is separate.
        spelling: target.symbol.clone(),
    };
    let provider_callable = build_provider_callable(language, target, &native);
    let mut value = ResolverTarget {
        address,
        relation_target: None,
        provider_pointer: format!("/observations/{observation_index}/targets/{target_index}"),
        provider_symbol: Some(native),
        provider_callable,
        declaration: Some(declaration_for_target_kind(&target.kind)),
    };
    if let Some(site) = local.filter(|_| target.kind == "callable") {
        let matches = declarations
            .get(&(site.path.clone(), site.span.start_byte, site.span.end_byte))
            .cloned()
            .unwrap_or_default();
        if matches.len() == 1 {
            value.relation_target = Some(topology_node_address(&matches[0].id));
        }
    }
    value
}

fn build_provider_callable(
    language: &str,
    target: &NativeCallTarget,
    native: &ProviderSymbolSpelling,
) -> Option<ProviderCallableRef> {
    let has_package_meta = target.package_role.is_some()
        || target.package_purl.is_some()
        || target.module_path.is_some()
        || !target.path_segments.is_empty();
    let local = target.location.repository().is_some();
    // Prefer structured refs for external/intrinsic targets and whenever solvers
    // already attached package metadata. In-repo joins keep occurrence RepoPath.
    if local && !has_package_meta {
        return None;
    }
    let role = parse_package_role(target.package_role.as_deref());
    let purl = resolve_package_purl(language, target);
    let path = if target.path_segments.is_empty() {
        path_segments_for_language(language, target)
    } else {
        target.path_segments.clone()
    };
    let ecosystem = target
        .ecosystem
        .clone()
        .or_else(|| Some(language.to_owned()));
    let display = if path.is_empty() {
        Some(native.spelling.clone())
    } else {
        Some(display_from_path(&path))
    };
    let value = ProviderCallableRef {
        schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
        kind: callable_kind_from_target(&target.kind),
        package: ProviderPackageRef {
            role,
            purl,
            ecosystem,
        },
        path,
        display,
        native: native.clone(),
    };
    value.validate().ok()?;
    Some(value)
}

fn parse_package_role(raw: Option<&str>) -> ProviderPackageRole {
    match raw {
        Some("stdlib") => ProviderPackageRole::Stdlib,
        Some("module") => ProviderPackageRole::Module,
        Some("application") => ProviderPackageRole::Application,
        _ => ProviderPackageRole::Unknown,
    }
}

fn callable_kind_from_target(kind: &str) -> ProviderCallableKind {
    match kind {
        "callable" | "callable_signature" => ProviderCallableKind::Callable,
        "type" => ProviderCallableKind::Type,
        "value" => ProviderCallableKind::Value,
        "module" => ProviderCallableKind::Module,
        "intrinsic" => ProviderCallableKind::Intrinsic,
        _ => ProviderCallableKind::Unknown,
    }
}

pub fn resolve_package_purl(language: &str, target: &NativeCallTarget) -> Option<String> {
    if let Some(purl) = target
        .package_purl
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return match purl_address(purl) {
            Ok(Address::Purl { purl }) => Some(purl),
            _ => None,
        };
    }
    let version = target.package_version.as_deref();
    let module_path = target.module_path.as_deref();
    match language {
        "go" if target.package_role.as_deref() == Some("stdlib") => {
            purl_golang_stdlib(version).ok()
        }
        "go" => module_path.and_then(|module| purl_golang_module(module, version).ok()),
        "python" => module_path.and_then(|name| purl_pypi(name, version).ok()),
        "javascript" | "typescript" | "tsx" => {
            module_path.and_then(|name| purl_npm(name, version).ok())
        }
        _ => None,
    }
}

fn path_segments_for_language(
    language: &str,
    target: &NativeCallTarget,
) -> Vec<ProviderPathSegment> {
    match language {
        "go" => path_from_go_qualified_name(&target.symbol),
        "javascript" | "typescript" => {
            if let Some((module, export)) = target.symbol.split_once('#') {
                let mut path = Vec::new();
                if !module.is_empty() {
                    path.push(ProviderPathSegment {
                        kind: ProviderPathKind::Namespace,
                        name: module.to_owned(),
                        disambiguator: None,
                    });
                }
                if !export.is_empty() {
                    path.push(ProviderPathSegment {
                        kind: ProviderPathKind::Method,
                        name: export.to_owned(),
                        disambiguator: None,
                    });
                }
                path
            } else if target.symbol.trim().is_empty() {
                Vec::new()
            } else {
                vec![ProviderPathSegment {
                    kind: ProviderPathKind::Method,
                    name: target.symbol.clone(),
                    disambiguator: None,
                }]
            }
        }
        _ => {
            if target.symbol.trim().is_empty() {
                Vec::new()
            } else {
                vec![ProviderPathSegment {
                    kind: ProviderPathKind::Method,
                    name: target.symbol.clone(),
                    disambiguator: None,
                }]
            }
        }
    }
}

/// Match a declaration within a path/end index. Python provider ranges must be
/// exact: a class and its final method can share an end offset. Other adapters
/// can include leading documentation, but still need one unambiguous match.
pub(super) fn matching_declaration<'a>(
    nodes: &[&'a TopologyNode],
    start: usize,
    end: usize,
    language: &str,
) -> Option<&'a TopologyNode> {
    if start >= end {
        return None;
    }
    let exact = nodes
        .iter()
        .any(|node| node.start_byte == Some(start) && node.end_byte == Some(end));
    let mut matches = nodes.iter().copied().filter(|node| {
        node.end_byte == Some(end)
            && node.start_byte.is_some_and(|candidate| {
                if exact || language == "python" {
                    candidate == start
                } else {
                    start <= candidate && candidate < end
                }
            })
    });
    let only = matches.next()?;
    matches.next().is_none().then_some(only)
}

/// Project native calls with the exact inventoried call range. Conflicting
/// facts survive projection for explicit ambiguity; identical repeats coalesce.
/// Python declaration ranges are exact; other adapters may include leading
/// documentation. Arbitrary containment never establishes callable identity.
pub fn project_calls_onto_inventory(
    graph: &TopologyGraph,
    raw: &NativeRawObservation,
    language: &str,
) -> Vec<NativeCallFact> {
    let mut callables: BTreeMap<(&str, usize), Vec<&TopologyNode>> = BTreeMap::new();
    for node in &graph.nodes {
        if node.kind == NodeKind::Callable {
            if let (Some(path), Some(end)) = (node.path.as_deref(), node.end_byte) {
                callables.entry((path, end)).or_default().push(node);
            }
        }
    }
    let snap_owner = |owner: &NativeSpanRef| -> Option<NativeSpanRef> {
        let nodes = callables.get(&(owner.path.as_str(), owner.span.end_byte))?;
        let node =
            matching_declaration(nodes, owner.span.start_byte, owner.span.end_byte, language)?;
        Some(NativeSpanRef {
            path: owner.path.clone(),
            span: entmoot_contracts::SiteSpan {
                start_byte: node.start_byte?,
                end_byte: owner.span.end_byte,
            },
        })
    };
    let mut calls: BTreeMap<(&str, usize), Vec<&NativeCallFact>> = BTreeMap::new();
    for call in &raw.calls {
        calls
            .entry((&call.site.path, call.site.span.start_byte))
            .or_default()
            .push(call);
    }
    let mut projected = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::CallStub {
            continue;
        }
        let (Some(path), Some(start), Some(end)) =
            (node.path.as_deref(), node.start_byte, node.end_byte)
        else {
            continue;
        };
        if !language_matches(language, path) {
            continue;
        }
        let Some(candidates) = calls.get(&(path, start)) else {
            continue;
        };
        let mut unique = Vec::new();
        for call in candidates
            .iter()
            .copied()
            .filter(|call| call.site.span.end_byte == end)
        {
            if unique.contains(&call) {
                continue;
            }
            unique.push(call);
            let mut fact = call.clone();
            fact.site.span.end_byte = end;
            if let Some(owner) = fact.owner.take() {
                fact.owner = Some(snap_owner(&owner).unwrap_or(owner));
            }
            for target in &mut fact.targets {
                if target.kind != "callable" {
                    continue;
                }
                if let Some(site) = target.location.repository_mut() {
                    if let Some(snapped) = snap_owner(site) {
                        site.span.start_byte = snapped.span.start_byte;
                    }
                }
            }
            projected.push(fact);
        }
    }
    projected.sort_by(|a, b| {
        (&a.site.path, a.site.span.start_byte, a.site.span.end_byte).cmp(&(
            &b.site.path,
            b.site.span.start_byte,
            b.site.span.end_byte,
        ))
    });
    projected
}

/// Join every inventoried call stub for `language` onto native facts.
pub fn normalize_observations(
    graph: &TopologyGraph,
    raw: &NativeRawObservation,
    language: &str,
) -> Vec<ResolverObservation> {
    if !matches!(
        language,
        "rust"
            | "python"
            | "go"
            | "java"
            | "csharp"
            | "typescript"
            | "tsx"
            | "javascript"
            | "cxx"
            | "c"
            | "cpp"
    ) {
        return Vec::new();
    }

    let projected = project_calls_onto_inventory(graph, raw, language);

    let mut declarations: BTreeMap<(String, usize, usize), Vec<&TopologyNode>> = BTreeMap::new();
    for node in &graph.nodes {
        if node.kind == NodeKind::Callable {
            if let (Some(path), Some(start), Some(end)) =
                (node.path.as_ref(), node.start_byte, node.end_byte)
            {
                declarations
                    .entry((path.clone(), start, end))
                    .or_default()
                    .push(node);
            }
        }
    }

    let mut by_call: BTreeMap<(String, usize, usize), Vec<&NativeCallFact>> = BTreeMap::new();
    for call in &projected {
        by_call
            .entry((
                call.site.path.clone(),
                call.site.span.start_byte,
                call.site.span.end_byte,
            ))
            .or_default()
            .push(call);
    }

    let mut callee_references: BTreeMap<(&str, usize), Vec<&NativeReferenceFact>> = BTreeMap::new();
    for reference in &raw.references {
        if reference.kind == "call_callee" {
            callee_references
                .entry((&reference.site.path, reference.site.span.start_byte))
                .or_default()
                .push(reference);
        }
    }

    let mut sites: Vec<&TopologyNode> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == NodeKind::CallStub
                && node
                    .path
                    .as_deref()
                    .is_some_and(|path| language_matches(language, path))
        })
        .collect();
    sites.sort_by(|a, b| a.id.cmp(&b.id));

    let mut observations = Vec::with_capacity(sites.len());
    for site in sites {
        let parent = site.parent.clone().unwrap_or_default();
        let mut observation = ResolverObservation {
            dispatch: CallDispatch::Unassessed,
            occurrence_id: site.id.clone(),
            source: ResolverCallSource {
                call_site: topology_node_address(&site.id),
                relation_source: topology_node_address(&parent),
                syntactic_callee: site.label.clone(),
            },
            outcome: ResolverObservationOutcome::Unsupported,
            targets: Vec::new(),
            limitation: Some(
                "native tool did not assess this exact inventory call; implicit calls and dispatch completeness remain unassessed"
                    .into(),
            ),
        };

        if raw.budget_exceeded {
            observation.dispatch = CallDispatch::Open {
                reason: DispatchGap::BudgetExceeded,
            };
            observation.limitation = Some("native analysis budget exhausted".into());
            observations.push(observation);
            continue;
        }
        if raw.invalid_environment {
            observation.outcome = ResolverObservationOutcome::Failed;
            observation.dispatch = CallDispatch::Open {
                reason: DispatchGap::InvalidEnvironment,
            };
            observation.limitation = Some(
                "native diagnostics or workspace loading failure; see native diagnostics".into(),
            );
            observations.push(observation);
            continue;
        }

        let key = (
            site.path.clone().unwrap_or_default(),
            site.start_byte.unwrap_or(0),
            site.end_byte.unwrap_or(0),
        );
        let calls = by_call.get(&key).cloned().unwrap_or_default();
        if calls.is_empty() {
            observations.push(observation);
            continue;
        }
        if calls.len() != 1 {
            observation.outcome = ResolverObservationOutcome::Ambiguous;
            observation.limitation =
                Some("multiple native call facts match this inventory span".into());
            observations.push(observation);
            continue;
        }
        let call = calls[0];
        if let Some(owner) = &call.owner {
            let owners = declarations
                .get(&(
                    owner.path.clone(),
                    owner.span.start_byte,
                    owner.span.end_byte,
                ))
                .cloned()
                .unwrap_or_default();
            if owners.len() != 1 || owners[0].id != parent {
                observation.dispatch = CallDispatch::Open {
                    reason: DispatchGap::UnsupportedConstruct,
                };
                observation.limitation = Some(
                    "native enclosing callable does not match inventory owner; do not lift nested callback calls to the outer function"
                        .into(),
                );
                observations.push(observation);
                continue;
            }
        }

        observation.outcome = ResolverObservationOutcome::Unresolved;
        observation.limitation = Some(call.limitation.clone().unwrap_or_else(|| {
            "native definition binding is a candidate, not execution or complete dispatch".into()
        }));
        if let CallDispatch::Open { .. } = &call.dispatch {
            observation.dispatch = call.dispatch.clone();
        }
        if let Some(gap) = call.gap.as_deref() {
            observation.dispatch = CallDispatch::Open {
                reason: DispatchGap::from_provider(language, gap),
            };
        }

        // Prefer call-fact targets; when empty, adopt the unique covering reference.
        let target_facts: &[NativeCallTarget] = if !call.targets.is_empty() {
            call.targets.as_slice()
        } else if let Some(reference) = covering_reference(&callee_references, &key.0, key.1, key.2)
        {
            if let Some(gap) = reference.gap.as_deref() {
                observation.dispatch = CallDispatch::Open {
                    reason: DispatchGap::from_provider(language, gap),
                };
            }
            if let Some(limitation) = reference.limitation.as_ref() {
                observation.limitation = Some(limitation.clone());
            }
            reference.targets.as_slice()
        } else {
            call.targets.as_slice()
        };

        // Do not discard an invalid alternative and accidentally establish a
        // unique target. These are provider coordinates, not verified offsets.
        if target_facts.iter().any(|target| {
            let span = target.location.span();
            span.start_byte > span.end_byte
                || span.end_byte > u32::MAX as usize
                || target.location.repository().is_some_and(|site| {
                    !entmoot_contracts::normalize_repo_path(&site.path)
                        .is_ok_and(|path| path == site.path)
                })
        }) {
            observation.outcome = ResolverObservationOutcome::Unsupported;
            observation.dispatch = CallDispatch::Open {
                reason: DispatchGap::UnsupportedConstruct,
            };
            observation.limitation =
                Some("invalid native target location; binding unassessed".into());
            observations.push(observation);
            continue;
        }

        let mut targets = Vec::new();
        for (target_index, target) in target_facts.iter().enumerate() {
            targets.push(build_resolver_target(
                language,
                observations.len(),
                target_index,
                target,
                &declarations,
            ));
        }
        observation.targets = targets;

        if language == "python"
            && target_facts
                .iter()
                .zip(&observation.targets)
                .any(|(raw, joined)| {
                    raw.kind == "callable"
                        && raw.location.repository().is_some()
                        && joined.relation_target.is_none()
                })
        {
            let detail = "native callable declaration range is unmatched or ambiguous in inventory";
            observation.limitation = Some(match observation.limitation.take() {
                Some(previous) => format!("{previous}; {detail}"),
                None => detail.into(),
            });
        }

        if observation.targets.len() > 1 {
            observation.outcome = ResolverObservationOutcome::Ambiguous;
        } else if let Some(target) = observation.targets.first() {
            observation.outcome = if target.relation_target.is_some() {
                ResolverObservationOutcome::RepositoryCallable
            } else if matches!(target.address, Address::RepoPath { .. }) {
                ResolverObservationOutcome::RepositorySymbol
            } else if call.intrinsic {
                ResolverObservationOutcome::Intrinsic
            } else {
                ResolverObservationOutcome::ExternalSymbol
            };
        } else if let CallDispatch::Open { reason } = &observation.dispatch {
            observation.outcome = match reason {
                DispatchGap::UnsupportedConstruct => ResolverObservationOutcome::Unsupported,
                DispatchGap::ProviderError | DispatchGap::InvalidEnvironment => {
                    ResolverObservationOutcome::Failed
                }
                _ => ResolverObservationOutcome::Unresolved,
            };
        }

        if let CallDispatch::Closed {
            policy_id,
            evidence_pointer,
            assumptions,
        } = &call.dispatch
        {
            // Closure belongs to the exact call fact. A covering reference or
            // unresolved declaration cannot supply its missing target or owner.
            if call.gap.is_none()
                && call.owner.is_some()
                && call.targets.len() == 1
                && observation.outcome == ResolverObservationOutcome::RepositoryCallable
                && !policy_id.trim().is_empty()
                && !evidence_pointer.trim().is_empty()
                && !assumptions.is_empty()
                && assumptions.iter().all(|value| !value.trim().is_empty())
            {
                observation.dispatch = CallDispatch::Closed {
                    policy_id: policy_id.clone(),
                    evidence_pointer: format!("/observations/{}/dispatch", observations.len()),
                    assumptions: assumptions.clone(),
                };
                observation.limitation.clone_from(&call.limitation);
            }
        }
        if observation.outcome != ResolverObservationOutcome::RepositoryCallable {
            for target in &mut observation.targets {
                target.relation_target = None;
            }
        }
        // RepositoryCallable requires Direct/AssignedValue association (contract INV).
        if observation.outcome == ResolverObservationOutcome::RepositoryCallable {
            for target in &mut observation.targets {
                if let Some(declaration) = target.declaration.as_mut() {
                    declaration.callable_association = ResolverCallableAssociation::Direct;
                    declaration.kind = ResolverDeclarationKind::Callable;
                }
            }
        }
        observations.push(observation);
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::TopologyNode;

    fn graph_with_call() -> TopologyGraph {
        let mut graph = TopologyGraph::new("/tmp/fixture");
        let mut caller = TopologyNode::shell("caller", NodeKind::Callable, "caller", None);
        caller.path = Some("src/lib.rs".into());
        caller.start_byte = Some(10);
        caller.end_byte = Some(40);
        let mut callee = TopologyNode::shell("help", NodeKind::Callable, "help", None);
        callee.path = Some("src/sibling.rs".into());
        callee.start_byte = Some(0);
        callee.end_byte = Some(20);
        let mut site =
            TopologyNode::shell("call", NodeKind::CallStub, "help", Some("caller".into()));
        site.path = Some("src/lib.rs".into());
        site.start_byte = Some(20);
        site.end_byte = Some(24);
        site.parent = Some("caller".into());
        graph.upsert_node(caller);
        graph.upsert_node(callee);
        graph.upsert_node(site);
        graph
    }

    #[test]
    fn joins_unique_callable_target() {
        let graph = graph_with_call();
        let raw = NativeRawObservation {
            calls: vec![NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 20,
                        end_byte: 24,
                    },
                },

                targets: vec![NativeCallTarget {
                    location: entmoot_contracts::NativeTargetLocation::from_provider(
                        None,
                        Some("src/sibling.rs".into()),
                        entmoot_contracts::SiteSpan {
                            start_byte: 0,
                            end_byte: 20,
                        },
                    ),

                    symbol: "help".into(),
                    kind: "callable".into(),
                    package_role: None,
                    package_purl: None,
                    ecosystem: None,
                    module_path: None,
                    package_version: None,
                    path_segments: Vec::new(),
                }],
                owner: Some(NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 10,
                        end_byte: 40,
                    },
                }),
                gap: None,
                limitation: None,
                intrinsic: false,
            }],
            ..Default::default()
        };
        let observations = normalize_observations(&graph, &raw, "rust");
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].outcome,
            ResolverObservationOutcome::RepositoryCallable
        );
        assert!(observations[0].targets[0].relation_target.is_some());
        let declaration = observations[0].targets[0]
            .declaration
            .as_ref()
            .expect("declaration");
        assert_eq!(declaration.kind, ResolverDeclarationKind::Callable);
        assert_eq!(
            declaration.callable_association,
            ResolverCallableAssociation::Direct
        );
        assert_eq!(declaration.classification_method_id, METHOD_ID);
    }

    #[test]
    fn qualified_dispatch_requires_exact_unambiguous_evidence() {
        let graph = graph_with_call();
        let mut raw = NativeRawObservation {
            calls: vec![NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 20,
                        end_byte: 24,
                    },
                },

                targets: vec![NativeCallTarget {
                    location: entmoot_contracts::NativeTargetLocation::from_provider(
                        None,
                        Some("src/sibling.rs".into()),
                        entmoot_contracts::SiteSpan {
                            start_byte: 0,
                            end_byte: 20,
                        },
                    ),

                    symbol: "help".into(),
                    kind: "callable".into(),
                    package_role: None,
                    package_purl: None,
                    ecosystem: None,
                    module_path: None,
                    package_version: None,
                    path_segments: Vec::new(),
                }],
                owner: Some(NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 10,
                        end_byte: 40,
                    },
                }),
                gap: None,
                limitation: None,
                intrinsic: false,
            }],
            ..Default::default()
        };
        raw.calls[0].dispatch = CallDispatch::Closed {
            policy_id: "fixture.direct.v1".into(),
            evidence_pointer: "/calls/0".into(),
            assumptions: vec!["selected build context".into()],
        };
        assert!(normalize_observations(&graph, &raw, "rust")[0]
            .dispatch
            .is_closed());
        for problem in [
            "gap",
            "owner",
            "target",
            "budget",
            "environment",
            "duplicate",
            "pointer",
        ] {
            let mut invalid = raw.clone();
            match problem {
                "gap" => invalid.calls[0].gap = Some("runtime_mutation".into()),
                "owner" => invalid.calls[0].owner = None,
                "target" => invalid.calls[0].targets.clear(),
                "budget" => invalid.budget_exceeded = true,
                "environment" => invalid.invalid_environment = true,
                "duplicate" => {
                    let mut other = invalid.calls[0].clone();
                    other.targets[0].symbol = "different".into();
                    invalid.calls.push(other);
                }
                "pointer" => {
                    invalid.calls[0].dispatch = CallDispatch::Closed {
                        policy_id: "".into(),
                        evidence_pointer: "".into(),
                        assumptions: vec![],
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                !normalize_observations(&graph, &invalid, "rust")[0]
                    .dispatch
                    .is_closed(),
                "{problem}"
            );
        }
    }

    #[test]
    fn external_go_target_gets_provider_callable_with_stdlib_purl() {
        let mut graph = TopologyGraph::new("/tmp/fixture-go");
        let mut caller = TopologyNode::shell("caller", NodeKind::Callable, "caller", None);
        caller.path = Some("main.go".into());
        caller.start_byte = Some(10);
        caller.end_byte = Some(40);
        let mut site =
            TopologyNode::shell("call", NodeKind::CallStub, "Errorf", Some("caller".into()));
        site.path = Some("main.go".into());
        site.start_byte = Some(20);
        site.end_byte = Some(24);
        site.parent = Some("caller".into());
        graph.upsert_node(caller);
        graph.upsert_node(site);

        let raw = NativeRawObservation {
            calls: vec![NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: "main.go".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 20,
                        end_byte: 24,
                    },
                },

                targets: vec![NativeCallTarget {
                    location: entmoot_contracts::NativeTargetLocation::from_provider(
                        None,
                        None,
                        entmoot_contracts::SiteSpan {
                            start_byte: 0,
                            end_byte: 0,
                        },
                    ),

                    symbol: "fmt.Errorf".into(),
                    kind: "callable".into(),
                    package_role: Some("stdlib".into()),
                    package_purl: None,
                    ecosystem: Some("go".into()),
                    module_path: None,
                    package_version: Some("1.22.0".into()),
                    path_segments: Vec::new(),
                }],
                owner: Some(NativeSpanRef {
                    path: "main.go".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 10,
                        end_byte: 40,
                    },
                }),
                gap: None,
                limitation: None,
                intrinsic: false,
            }],
            ..Default::default()
        };
        let observations = normalize_observations(&graph, &raw, "go");
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].outcome,
            ResolverObservationOutcome::ExternalSymbol
        );
        let provider_callable = observations[0].targets[0]
            .provider_callable
            .as_ref()
            .expect("provider_callable");
        assert_eq!(
            provider_callable.package.purl.as_deref(),
            Some("pkg:golang/stdlib@1.22.0")
        );
        assert!(!provider_callable.path.is_empty());
        // Repo path stays off the callable path[] (occurrence-only).
        assert!(provider_callable
            .path
            .iter()
            .all(|segment| !segment.name.contains("main.go")));
    }

    #[test]
    fn javascript_language_matches_typescript_paths() {
        assert!(language_matches("javascript", "routes/login.ts"));
        assert!(language_matches("javascript", "app.tsx"));
        assert!(language_matches("javascript", "lib/util.js"));
        assert!(!language_matches("javascript", "main.rs"));
    }

    #[test]
    fn covering_reference_supplies_targets_when_call_empty() {
        let graph = graph_with_call();
        let raw = NativeRawObservation {
            calls: vec![NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 20,
                        end_byte: 24,
                    },
                },

                targets: vec![],
                owner: Some(NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 10,
                        end_byte: 40,
                    },
                }),
                gap: None,
                limitation: None,
                intrinsic: false,
            }],
            references: vec![NativeReferenceFact {
                site: entmoot_contracts::NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 20,
                        end_byte: 24,
                    },
                },

                kind: "call_callee".into(),
                spelling: "local_alias".into(),
                targets: vec![NativeCallTarget {
                    location: entmoot_contracts::NativeTargetLocation::from_provider(
                        None,
                        Some("src/sibling.rs".into()),
                        entmoot_contracts::SiteSpan {
                            start_byte: 0,
                            end_byte: 20,
                        },
                    ),

                    symbol: "help".into(),
                    kind: "callable".into(),
                    package_role: None,
                    package_purl: None,
                    ecosystem: None,
                    module_path: None,
                    package_version: None,
                    path_segments: Vec::new(),
                }],
                owner: Some(NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 10,
                        end_byte: 40,
                    },
                }),
                gap: None,
                limitation: None,
            }],
            ..Default::default()
        };
        let observations = normalize_observations(&graph, &raw, "rust");
        assert_eq!(
            observations[0].outcome,
            ResolverObservationOutcome::RepositoryCallable
        );
        assert_eq!(
            observations[0].targets[0]
                .provider_symbol
                .as_ref()
                .map(|s| s.spelling.as_str()),
            Some("help")
        );
        assert!(observations[0].targets[0].declaration.is_some());
    }

    #[test]
    fn indirect_call_without_targets_is_an_unresolved_binding() {
        let graph = graph_with_call();
        let mut raw = NativeRawObservation {
            calls: vec![NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: "src/lib.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 20,
                        end_byte: 24,
                    },
                },

                targets: vec![],
                owner: None,
                gap: Some("indirect_call".into()),
                limitation: Some("fn pointer".into()),
                intrinsic: false,
            }],
            ..Default::default()
        };
        let observations = normalize_observations(&graph, &raw, "rust");
        assert_eq!(
            observations[0].outcome,
            ResolverObservationOutcome::Unresolved
        );
        assert_eq!(
            observations[0].dispatch,
            CallDispatch::Open {
                reason: DispatchGap::IndirectCall("rust.indirect_call".into()),
            }
        );
        for (gap, outcome, category) in [
            (
                "future_provider_reason",
                ResolverObservationOutcome::Unresolved,
                "unresolved_binding",
            ),
            (
                "unresolved_or_dynamic",
                ResolverObservationOutcome::Unresolved,
                "unresolved_binding",
            ),
            (
                "unsupported_declaration_or_stub",
                ResolverObservationOutcome::Unsupported,
                "unsupported_construct",
            ),
            (
                "provider_error",
                ResolverObservationOutcome::Failed,
                "provider_error",
            ),
            (
                "indirect_call:future_dispatch",
                ResolverObservationOutcome::Unresolved,
                "indirect_call",
            ),
        ] {
            raw.calls[0].gap = Some(gap.into());
            let rows = normalize_observations(&graph, &raw, "rust");
            assert_eq!(rows[0].outcome, outcome);
            let CallDispatch::Open { reason } = &rows[0].dispatch else {
                panic!("missing gap: {gap}")
            };
            assert_eq!(reason.category(), category);
            assert_eq!(rows[0].source, observations[0].source);
        }
        raw.calls[0].gap = None;
        raw.calls[0].dispatch = CallDispatch::Open {
            reason: DispatchGap::IndirectCall("rust.future_dispatch".into()),
        };
        let rows = normalize_observations(&graph, &raw, "rust");
        assert_eq!(rows[0].dispatch, raw.calls[0].dispatch);
        assert_eq!(rows[0].outcome, ResolverObservationOutcome::Unresolved);
    }
}
