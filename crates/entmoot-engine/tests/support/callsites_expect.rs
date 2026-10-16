//! Shared call-site / dispatch qualification against authored fixtures.
//! Uses engine produce/attach/audit APIs — not CLI subprocesses.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use entmoot_contracts::{
    Address, BindingFamily, CallDispatch, DispatchGap, ResolverObservationOutcome,
};
use entmoot_engine::graph::{NodeKind, TopologyGraph, TopologyNode};
use entmoot_engine::native_resolver::pipeline::produce_family;
use entmoot_engine::native_resolver::producers::{python as pyright_program, typescript};
use entmoot_engine::native_resolver::{
    adapters_root_hint, resolve_adapter_binary, resolve_sidecar_family, ExecutionPolicy,
    NativeProduceRequest, ProducerBudget,
};
use entmoot_engine::resolver_accounting::audit_resolver_manifests;
use entmoot_engine::{
    build_topology_from_repo_with_options, ingest_artifact_manifest, write_graph_json,
    TopologyBuildOptions,
};

pub fn callsites_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/callsites")
}

pub fn read_json(path: impl AsRef<Path>) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
}

fn address_value(address: &Address) -> &str {
    match address {
        Address::Opaque { value, .. } => value.as_str(),
        other => panic!("expected opaque topology address, got {other:?}"),
    }
}

fn outcome_snake(outcome: &ResolverObservationOutcome) -> String {
    serde_json::to_value(outcome)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

fn selected<'a>(
    graph: &'a TopologyGraph,
    root: &Path,
    kind: NodeKind,
    selector: &serde_json::Value,
) -> &'a TopologyNode {
    let path = selector["path"].as_str().unwrap();
    let source = std::fs::read(root.join(path)).unwrap();
    let mut fragment = selector["text"].as_str().unwrap().as_bytes();
    let positions: Vec<_> = source
        .windows(fragment.len())
        .enumerate()
        .filter(|(_, window)| *window == fragment)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(positions.len(), 1, "ambiguous fixture selector {selector}");
    let mut start = positions[0];
    if let Some(within) = selector["within"].as_str() {
        let inner = within.as_bytes();
        let positions: Vec<_> = fragment
            .windows(inner.len())
            .enumerate()
            .filter(|(_, window)| *window == inner)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(positions.len(), 1, "ambiguous within selector {selector}");
        start += positions[0];
        fragment = inner;
    }
    let end = start + fragment.len();
    let exact: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == kind
                && node.path.as_deref() == Some(path)
                && node.start_byte == Some(start)
                && node.end_byte == Some(end)
        })
        .collect();
    if exact.len() == 1 {
        return exact[0];
    }
    // Authored selectors may include export/modifiers outside the inventoried
    // span. Accept a unique same-path node of the kind whose span equals, covers,
    // or is contained in the selected fragment.
    let covering: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            if node.kind != kind || node.path.as_deref() != Some(path) {
                return false;
            }
            let Some(node_start) = node.start_byte else {
                return false;
            };
            let Some(node_end) = node.end_byte else {
                return false;
            };
            (node_start == start && node_end == end)
                || (node_start <= start && node_end >= end)
                || (node_start >= start && node_end <= end)
        })
        .collect();
    assert_eq!(
        covering.len(),
        1,
        "{selector:?} kind={kind:?} exact={} covering={}",
        exact.len(),
        covering.len()
    );
    covering[0]
}

fn gap_matches(dispatch: &CallDispatch, category: &str, detail: Option<&str>) -> bool {
    let CallDispatch::Open { reason } = dispatch else {
        return false;
    };
    if reason.category() != category {
        return false;
    }
    detail.is_none_or(|expected| match reason {
        DispatchGap::IndirectCall(actual) | DispatchGap::UnresolvedBinding(actual) => {
            actual == expected
        }
        _ => false,
    })
}

fn target_ids_match(actual: &[&str], required: Option<&str>, allowed: &[&str]) -> bool {
    match required {
        Some(required) => actual == [required],
        None => {
            actual.iter().all(|id| allowed.contains(id))
                && actual.iter().collect::<BTreeSet<_>>().len() == actual.len()
        }
    }
}

#[test]
fn dispatch_expectations_do_not_promote_unrelated_details_to_categories() {
    for reason in [
        DispatchGap::IndirectCall("java.reflection".into()),
        DispatchGap::UnresolvedBinding("java.dynamic_dispatch".into()),
    ] {
        assert!(!gap_matches(
            &CallDispatch::Open { reason },
            "indirect_call",
            Some("java.dynamic_dispatch")
        ));
    }
    let future = CallDispatch::Open {
        reason: DispatchGap::IndirectCall("future.multiple_dispatch".into()),
    };
    assert!(gap_matches(&future, "indirect_call", None));
    assert!(gap_matches(
        &future,
        "indirect_call",
        Some("future.multiple_dispatch")
    ));
    assert!(!gap_matches(
        &future,
        "unresolved_binding",
        Some("future.multiple_dispatch")
    ));
    assert!(!gap_matches(
        &CallDispatch::Unassessed,
        "indirect_call",
        None
    ));
}

#[test]
fn optional_targets_permit_absence_but_reject_wrong_or_duplicate_targets() {
    assert!(target_ids_match(&[], None, &["expected"]));
    assert!(target_ids_match(&["expected"], None, &["expected"]));
    assert!(!target_ids_match(&["wrong"], None, &["expected"]));
    assert!(!target_ids_match(
        &["expected", "wrong"],
        None,
        &["expected"]
    ));
    assert!(!target_ids_match(
        &["expected", "expected"],
        None,
        &["expected"]
    ));
    assert!(!target_ids_match(&[], Some("expected"), &[]));
    assert!(target_ids_match(&["expected"], Some("expected"), &[]));
    assert!(!target_ids_match(&["wrong"], None, &[]));
}

/// Configure process env used by Go sidecar produces (matches operator defaults).
pub fn apply_sidecar_env() {
    std::env::set_var("ENTMOOT_JAVA_RELEASE", "17");
    std::env::set_var("ENTMOOT_GO_SINGLE_MODULE", "1");
    std::env::set_var("ENTMOOT_GO_TESTS", "exclude");
    std::env::set_var("GOENV", "off");
    // Prefer the selected floor toolchain when the host Go is older than go.mod.
    if std::env::var_os("GOTOOLCHAIN").is_none() {
        std::env::set_var("GOTOOLCHAIN", "go1.26.0");
    }
    std::env::set_var("GOWORK", "off");
    std::env::set_var("GOPROXY", "off");
    std::env::set_var("GOSUMDB", "off");
    std::env::set_var("CGO_ENABLED", "0");
    for key in [
        "GOFLAGS",
        "GOOS",
        "GOARCH",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
    ] {
        std::env::remove_var(key);
    }
}

/// Resolve catalog / default-build adapters into process env when unset.
/// Explicit `ENTMOOT_*_ADAPTER` paths stay exclusive (missing path still fails).
pub fn ensure_adapter_env(key: &str) {
    if let Some(value) = std::env::var_os(key) {
        let path = PathBuf::from(value);
        assert!(path.is_file(), "explicit {key} missing: {}", path.display());
        return;
    }
    let path = match key {
        "ENTMOOT_GO_ADAPTER" => resolve_sidecar_family("go"),
        "ENTMOOT_JAVA_ADAPTER" => resolve_sidecar_family("java"),
        "ENTMOOT_ROSLYN_ADAPTER" => resolve_sidecar_family("roslyn"),
        "ENTMOOT_CLANG_ADAPTER" => resolve_sidecar_family("clang"),
        "ENTMOOT_PYRIGHT_ADAPTER" => resolve_adapter_binary(key, pyright_program::CANDIDATES, true),
        "ENTMOOT_TYPESCRIPT_ADAPTER" => resolve_adapter_binary(key, typescript::CANDIDATES, true),
        _ => None,
    }
    .unwrap_or_else(|| {
        panic!(
            "{key} unset and no catalog adapter under {}",
            adapters_root_hint()
        )
    });
    assert!(
        path.is_file(),
        "{key} resolved missing path: {}",
        path.display()
    );
    std::env::set_var(key, &path);
}

pub struct QualifyOpts<'a> {
    pub family: BindingFamily,
    pub language: &'a str,
    pub fixture_prefix: &'a str,
    pub expectations_path: PathBuf,
    pub expectations_key: &'a str,
    pub adapter_env: Option<&'a str>,
    pub require_full_denominator: bool,
    pub full_pipeline: bool,
}

/// Qualify one fixture leaf / smoke root against authored expectations.
pub fn qualify_pipeline(opts: QualifyOpts<'_>) {
    apply_sidecar_env();
    if let Some(key) = opts.adapter_env {
        ensure_adapter_env(key);
    }
    let cases = read_json(&opts.expectations_path)[opts.expectations_key]
        .as_array()
        .unwrap_or_else(|| panic!("missing expectations key {}", opts.expectations_key))
        .clone();
    assert!(!cases.is_empty(), "no cases for {}", opts.expectations_key);

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    let fixture_src = callsites_fixtures().join(opts.fixture_prefix);
    assert!(
        fixture_src.is_dir(),
        "missing fixture {}",
        fixture_src.display()
    );
    copy_tree(&fixture_src, &root);

    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            repository: Some(&format!("fixture/callsites-{}", opts.language)),
            ..Default::default()
        },
    )
    .unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();

    let out_dir = dir.path().join("resolver-out");
    let produced = produce_family(
        opts.family,
        NativeProduceRequest {
            canon: false,
            graph_path: graph_path.clone(),
            source_root: root.clone(),
            output_dir: out_dir,
            budget: ProducerBudget {
                wall: Duration::from_secs(180),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy::Safe,
            rust_profile: Default::default(),
        },
    )
    .unwrap_or_else(|error| panic!("produce {}: {error}", opts.family.as_str()));

    assert_eq!(produced.artifact.implementation, graph.implementation);
    assert_eq!(produced.receipt.implementation, graph.implementation);
    assert!(!produced.raw.invalid_environment);
    assert!(!produced.raw.budget_exceeded);
    assert!(
        matches!(produced.receipt.semantic_status.as_str(), "ok" | "partial"),
        "semantic_status={}",
        produced.receipt.semantic_status.as_str()
    );

    let inventory: BTreeSet<_> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::CallStub)
        .map(|node| node.id.as_str())
        .collect();
    let actual: BTreeSet<_> = produced
        .artifact
        .observations
        .iter()
        .map(|row| row.occurrence_id.as_str())
        .collect();
    assert_eq!(inventory, actual);
    if opts.require_full_denominator {
        assert_eq!(produced.artifact.observations.len(), cases.len());
        assert_eq!(actual.len(), cases.len());
    }

    let mut selected_sites = BTreeSet::new();
    for case in &cases {
        let site = selected(&graph, &root, NodeKind::CallStub, &case["site"]);
        let owner = selected(&graph, &root, NodeKind::Callable, &case["owner"]);
        assert!(
            selected_sites.insert(site.id.as_str()),
            "duplicate site {}",
            case["name"]
        );
        let row = produced
            .artifact
            .observations
            .iter()
            .find(|row| row.occurrence_id == site.id)
            .unwrap_or_else(|| panic!("missing observation for {}", case["name"]));
        assert_eq!(site.parent.as_deref(), Some(owner.id.as_str()));
        assert_eq!(address_value(&row.source.relation_source), owner.id);
        assert_eq!(address_value(&row.source.call_site), site.id);

        let actual_outcome = outcome_snake(&row.outcome);
        let allowed = case["outcomes"].as_array().unwrap();
        assert!(
            allowed
                .iter()
                .any(|value| value.as_str() == Some(&actual_outcome)),
            "{}: outcome {actual_outcome} not in {allowed:?}; row={row:?}",
            case["name"]
        );

        let want_closed = case["closed_dispatch"] == true;
        assert_eq!(
            row.dispatch.is_closed(),
            want_closed,
            "{}: dispatch={:?}",
            case["name"],
            row.dispatch
        );
        if want_closed {
            let CallDispatch::Closed {
                policy_id,
                evidence_pointer,
                assumptions,
            } = &row.dispatch
            else {
                unreachable!()
            };
            let expected_policy = case["policy_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{}: closed_dispatch requires policy_id", case["name"]));
            assert_eq!(policy_id, expected_policy, "{}: policy_id", case["name"]);
            assert!(
                !assumptions.is_empty(),
                "{}: empty assumptions",
                case["name"]
            );
            assert!(
                !evidence_pointer.is_empty(),
                "{}: empty evidence",
                case["name"]
            );
            let joined = assumptions.join("\n");
            let needles = case["assumption_contains"].as_array().unwrap_or_else(|| {
                panic!(
                    "{}: closed_dispatch requires assumption_contains[]",
                    case["name"]
                )
            });
            assert!(
                !needles.is_empty(),
                "{}: empty assumption_contains",
                case["name"]
            );
            for needle in needles {
                let needle = needle.as_str().unwrap();
                assert!(
                    joined
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase()),
                    "{}: assumptions missing fence {needle:?}; got {assumptions:?}",
                    case["name"]
                );
            }
        }
        if let Some(expected_gap) = case["dispatch_gap"].as_str() {
            assert!(
                gap_matches(
                    &row.dispatch,
                    expected_gap,
                    case["dispatch_detail"].as_str()
                ),
                "{}: expected dispatch_gap {expected_gap}, got {:?}",
                case["name"],
                row.dispatch
            );
            assert!(!row.dispatch.is_closed(), "{}", case["name"]);
        }

        let required = case.get("target").map(|selector| {
            selected(&graph, &root, NodeKind::Callable, selector)
                .id
                .as_str()
        });
        let allowed: Vec<_> = case["allowed_targets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|selector| {
                selected(&graph, &root, NodeKind::Callable, selector)
                    .id
                    .as_str()
            })
            .collect();
        let actual: Vec<_> = row
            .targets
            .iter()
            .filter_map(|target| target.relation_target.as_ref())
            .map(address_value)
            .collect();
        assert!(
            target_ids_match(&actual, required, &allowed),
            "{}: targets {actual:?}; required {required:?}, allowed {allowed:?}",
            case["name"]
        );
        if required.is_some() {
            assert_eq!(row.targets.len(), 1, "{}", case["name"]);
        }
        if case["signature"] == true {
            assert_eq!(row.targets.len(), 1);
            assert_eq!(
                row.targets[0]
                    .declaration
                    .as_ref()
                    .map(|declaration| declaration.callable_association),
                Some(entmoot_contracts::ResolverCallableAssociation::Unknown)
            );
        }
    }

    if !opts.full_pipeline {
        return;
    }

    let mut attached = graph;
    ingest_artifact_manifest(&mut attached, &produced.manifest_path, true).unwrap();
    write_graph_json(&attached, &graph_path).unwrap();
    let audit =
        audit_resolver_manifests(&attached, std::slice::from_ref(&produced.manifest_path)).unwrap();
    assert_eq!(audit.producer_omissions, 0);
    if opts.require_full_denominator {
        assert_eq!(audit.call_sites.len(), cases.len());
        let expected_closed = cases
            .iter()
            .filter(|case| case["closed_dispatch"] == true)
            .count();
        assert_eq!(
            audit.coverage.first().map(|row| row.closed_sites),
            Some(expected_closed)
        );
    }
    assert!(audit.static_work_remaining > 0);
}

/// Smoke denominator under `fixtures/callsites/{language}/smoke`.
pub fn qualify_smoke(language: &str, family: BindingFamily, adapter: Option<&str>) {
    let fixtures = callsites_fixtures();
    let prefix = format!("{language}/smoke");
    qualify_pipeline(QualifyOpts {
        family,
        language,
        fixture_prefix: &prefix,
        expectations_path: fixtures.join("expectations_smoke.json"),
        expectations_key: language,
        adapter_env: adapter,
        require_full_denominator: true,
        full_pipeline: true,
    });
}

/// Construct leaf under `fixtures/callsites/{language}/{construct}`.
pub fn qualify_construct(
    language: &str,
    construct: &str,
    family: BindingFamily,
    adapter: Option<&str>,
) {
    let key = format!("{language}/{construct}");
    qualify_pipeline(QualifyOpts {
        family,
        language,
        fixture_prefix: &key,
        expectations_path: callsites_fixtures().join("expectations_dispatch.json"),
        expectations_key: &key,
        adapter_env: adapter,
        require_full_denominator: true,
        full_pipeline: false,
    });
}
