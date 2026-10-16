//! Native Rust resolver qualification against the mini fixture crate.

#![cfg(feature = "resolved-rust-analyzer")]

use std::path::PathBuf;
use std::time::Duration;

use entmoot_contracts::ResolverObservationOutcome;
use entmoot_engine::native_resolver::{
    produce_rust_resolver, ExecutionPolicy, NativeProduceRequest, ProducerBudget, RA_AP_PIN,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, write_graph_json, TopologyBuildOptions,
};
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/entmoot-engine/tests/fixtures/callsites/rust/smoke")
}

#[test]
fn canon_rust_uses_native_parameter_types() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='canon-control'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    let source = "pub fn f(x: bool) -> bool { !x }";
    std::fs::write(root.join("src/lib.rs"), source).unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: true,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget {
            wall: Duration::from_secs(180),
            ..Default::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    assert!(
        !result.raw.invalid_environment,
        "{:?}",
        result.raw.diagnostics
    );
    assert!(result
        .raw
        .expressions
        .iter()
        .any(|f| f.site.span.start_byte == 9
            && f.site.span.end_byte == 10
            && f.sort == entmoot_contracts::CanonSort::Bool));
    let hasher = entmoot_core::StructuralHasher::new();
    let inventory = hasher
        .compute_granular_hash_with_analysis(source, "rust", None)
        .unwrap();
    let canon = hasher
        .canon_file(
            source,
            "rust",
            &inventory.markers.unwrap().code_contracts,
            &result.raw.expressions,
            100,
        )
        .unwrap();
    assert!(
        canon.forms[0].complete(),
        "{:?}",
        canon.forms[0].limitations
    );
    canon.forms[0].validate().unwrap();
}

#[test]
fn function_values_are_observed_uses_without_claiming_indirect_dispatch() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='callback-control'\nversion='0.1.0'\nedition='2021'\n[features]\nextra=[]\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn callback()->i32{1}\nfn unused()->i32{0}\npub fn pass()->fn()->i32{#[cfg(feature=\"extra\")]let f=unused;callback}\npub fn invoke(f:fn()->i32)->i32{#[cfg(feature=\"extra\")]unused();f()}\n#[cfg(feature=\"extra\")]const UNUSED:fn()->i32=unused;\n#[cfg(feature=\"extra\")]fn inactive()->fn()->i32{unused}\n").unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget::default(),
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let callback = graph
        .nodes
        .iter()
        .find(|n| n.label == "callback" && n.kind == entmoot_engine::NodeKind::Callable)
        .unwrap();
    let observations = &result.artifact.assessments;
    assert_eq!(observations.len(), 1, "{observations:?}");
    assert!(
        matches!(&observations[0], entmoot_contracts::AssessmentObservation::CodeUse { subject, usage: entmoot_contracts::CodeUseKind::NonCallReference }
        if Some(subject.declaration.occurrence.address_id.as_str()) == callback.address_id.as_deref())
    );
    assert!(
        result
            .artifact
            .observations
            .iter()
            .all(|o| !o.dispatch.is_closed()),
        "callback / fn-pointer sites stay Open (product Open claim: dispatch_expectations rust/callback)"
    );
}

#[test]
fn edition_hops_keep_traits_named_types_macros_and_visibility_scoped() {
    for edition in ["2018", "2021", "2024"] {
        let dir = tempdir().unwrap();
        let root = dir.path().join("source");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), format!("[package]\nname='declaration-audit'\nversion='0.1.0'\nedition='{edition}'\n[features]\nextra=[]\n")).unwrap();
        std::fs::write(root.join("src/lib.rs"),
            "pub struct Api { pub value: i32 }\npub trait Read { fn value(&self) -> i32; fn default_value(&self) -> i32 { 1 } }\nimpl Api { pub fn direct(&self) -> i32 { self.value } }\n#[macro_export]\nmacro_rules! identity { ($x:expr) => { $x }; }\n#[cfg(feature=\"extra\")]\npub fn extra() -> i32 { 1 }\npub(crate) fn internal() -> i32 { 0 }\n").unwrap();
        let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
        let graph_path = dir.path().join("graph.json");
        write_graph_json(&graph, &graph_path).unwrap();
        for extra in [true, false] {
            let result = produce_rust_resolver(NativeProduceRequest {
                canon: false,
                graph_path: graph_path.clone(),
                source_root: root.clone(),
                output_dir: dir
                    .path()
                    .join(if extra { "feature-on" } else { "feature-off" }),
                budget: ProducerBudget {
                    wall: Duration::from_secs(180),
                    ..Default::default()
                },
                execution_policy: ExecutionPolicy::Safe,
                rust_profile: entmoot_contracts::RustBuildProfile {
                    features: entmoot_contracts::RustFeatureSelection::Selected {
                        names: if extra {
                            ["extra".into()].into()
                        } else {
                            Default::default()
                        },
                        no_default_features: false,
                    },
                    ..Default::default()
                },
            })
            .unwrap();
            let api = result.artifact.package_api.unwrap();
            let symbols: Vec<_> = api
                .surfaces
                .iter()
                .flat_map(|surface| surface.exports.iter())
                .map(|export| export.target.symbol.spelling.as_str())
                .collect();
            for expected in [
                "::Api",
                "::Read",
                "::Read::value",
                "::Read::default_value",
                "::Api::direct",
                "::identity",
            ] {
                assert!(
                    symbols.iter().any(|symbol| symbol.ends_with(expected)),
                    "{edition}: missing {expected}: {symbols:?}"
                );
            }
            assert!(
                !symbols.iter().any(|symbol| symbol.ends_with("::internal")),
                "{edition}"
            );
            assert_eq!(
                symbols.iter().any(|symbol| symbol.ends_with("::extra")),
                extra,
                "{edition}: feature selection"
            );
            assert!(api.surfaces.iter().all(
                |surface| surface.completeness == entmoot_contracts::ScopeCompleteness::Partial
            ));
        }
    }
}

#[test]
fn public_reexports_keep_the_definition_address_and_module_visibility() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = 'alias-audit'\nversion = '0.1.0'\nedition = '2021'\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "mod api;\npub use api::leaf as renamed;\npub use api::Count as Amount;\npub use api::Item as PublicItem;\npub mod facade { pub use crate::api::leaf as exposed; }\nmod private { pub use crate::api::leaf as hidden; }\npub fn caller() { renamed(); facade::exposed(); }\n").unwrap();
    std::fs::write(
        root.join("src/api.rs"),
        "pub fn leaf() -> i32 { 1 }\npub type Count = u64;\npub struct Item { pub value: Count }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/rust-aliases"),
        },
    )
    .unwrap();
    let leaf = graph
        .nodes
        .iter()
        .find(|node| {
            node.kind == entmoot_engine::graph::NodeKind::Callable
                && node.path.as_deref() == Some("src/api.rs")
        })
        .unwrap();
    let expected = entmoot_contracts::topology_node_address(&leaf.id);
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("resolver"),
        budget: ProducerBudget {
            wall: Duration::from_secs(180),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let api = result.artifact.package_api.unwrap();
    let exports: Vec<_> = api
        .surfaces
        .iter()
        .flat_map(|surface| &surface.exports)
        .collect();
    for symbol in ["alias-audit::renamed", "alias-audit::facade::exposed"] {
        let export = exports
            .iter()
            .find(|export| export.target.symbol.spelling == symbol)
            .unwrap_or_else(|| panic!("missing {symbol}: {api:?}"));
        assert_eq!(export.declaration, expected);
    }
    assert!(exports
        .iter()
        .all(|export| !export.target.symbol.spelling.ends_with("::hidden")));
    for symbol in ["alias-audit::Amount", "alias-audit::PublicItem"] {
        assert!(
            exports
                .iter()
                .any(|export| export.target.symbol.spelling == symbol),
            "{api:?}"
        );
    }
    assert!(
        exports
            .iter()
            .all(|export| !export.target.symbol.spelling.contains("::api::")),
        "{api:?}"
    );
}

#[test]
fn rust_smoke_receipt_reports_aligned_ra_pin() {
    // Product Open/Closed rows for this fixture live in expectations_smoke /
    // dispatch_expectations (`qualify_smoke("rust", …)`). This control only
    // fences the in-process adapter pin used by deeper RA suites.
    let root = fixture_root();
    assert!(
        root.join("Cargo.toml").is_file(),
        "missing fixture at {}",
        root.display()
    );
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/rust-native"),
        },
    )
    .expect("index fixture");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("resolver-out"),
        budget: ProducerBudget {
            wall: Duration::from_secs(180),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .expect("produce rust resolver");
    assert_eq!(result.receipt.producer.adapter_pin, RA_AP_PIN);
}

#[test]
fn cargo_features_do_not_implicitly_enable_test_cfg() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests/common")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='test-scope'\nversion='0.1.0'\nedition='2021'\nautotests=false\n[features]\nfull-tests=[]\n[[test]]\nname='calls'\npath='tests/calls.rs'\nrequired-features=['full-tests']\n").unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub struct Hasher;\nimpl Hasher { pub fn with_policy() -> Self { Self } }\n",
    )
    .unwrap();
    std::fs::write(root.join("tests/common/mod.rs"), "pub fn helper() {}\n").unwrap();
    let ordinary = "fn feature_body() { let _h = Hasher::with_policy(); helper(); }";
    let test_only = "fn check() { let _h = Hasher::with_policy(); helper(); }";
    let calls = format!("#![cfg(feature=\"full-tests\")]\nuse test_scope::Hasher;\n#[path=\"common/mod.rs\"] mod common;\nuse common::helper;\n{ordinary}\n#[test] {test_only}\n");
    std::fs::write(root.join("tests/calls.rs"), &calls).unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    for (case, enabled, test_mode) in [
        ("default", false, false),
        ("features", true, false),
        ("test", true, true),
        ("missing-required-feature", false, true),
    ] {
        let result = produce_rust_resolver(NativeProduceRequest {
            canon: false,
            graph_path: graph_path.clone(),
            source_root: root.clone(),
            output_dir: dir.path().join(case),
            budget: ProducerBudget {
                wall: Duration::from_secs(180),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy::Safe,
            rust_profile: entmoot_contracts::RustBuildProfile {
                analysis: if test_mode {
                    entmoot_contracts::RustAnalysisMode::Test {
                        crate_root: "tests/calls.rs".into(),
                    }
                } else {
                    entmoot_contracts::RustAnalysisMode::Workspace {}
                },
                features: entmoot_contracts::RustFeatureSelection::Selected {
                    names: if enabled {
                        ["full-tests".into()].into()
                    } else {
                        Default::default()
                    },
                    no_default_features: false,
                },
                ..Default::default()
            },
        })
        .unwrap();
        if test_mode && !enabled {
            assert!(result.raw.invalid_environment);
            assert!(
                result
                    .raw
                    .diagnostics
                    .iter()
                    .any(|reason| reason.contains("inactive required features")),
                "{:?}",
                result.raw.diagnostics
            );
            assert!(result
                .artifact
                .observations
                .iter()
                .all(|o| o.outcome != ResolverObservationOutcome::RepositoryCallable));
            continue;
        }
        assert!(
            !result.raw.invalid_environment,
            "{:?}",
            result.raw.diagnostics
        );
        assert_eq!(
            result.raw.build_context["rust_profile"]["features"]["names"],
            serde_json::json!(if enabled { vec!["full-tests"] } else { vec![] })
        );
        for (definition, selected) in [(ordinary, enabled), (test_only, enabled && test_mode)] {
            let start = calls.find(definition).unwrap();
            let owners: Vec<_> = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == entmoot_engine::NodeKind::Callable
                        && node.path.as_deref() == Some("tests/calls.rs")
                        && node.start_byte == Some(start)
                        && node.end_byte == Some(start + definition.len())
                })
                .collect();
            assert_eq!(owners.len(), 1, "authored callable occurrence");
            let owner = entmoot_contracts::Address::Opaque {
                system: "entmoot.topology_node".into(),
                value: owners[0].id.clone(),
            };
            for name in ["Hasher::with_policy", "helper"] {
                let observation = result
                    .artifact
                    .observations
                    .iter()
                    .find(|o| {
                        o.source.syntactic_callee == name && o.source.relation_source == owner
                    })
                    .unwrap();
                if selected {
                    assert_eq!(
                        observation.outcome,
                        ResolverObservationOutcome::RepositoryCallable,
                        "{observation:?}"
                    );
                    assert!(observation.targets[0].relation_target.is_some());
                } else {
                    assert_ne!(
                        observation.outcome,
                        ResolverObservationOutcome::RepositoryCallable,
                        "enabled={enabled} definition={definition} name={name} {observation:?}"
                    );
                    assert!(observation
                        .targets
                        .iter()
                        .all(|t| t.relation_target.is_none()));
                }
                assert_eq!(
                    observation.dispatch.is_closed(),
                    selected,
                    "{observation:?}"
                );
            }
        }
    }
}

#[test]
fn test_profiles_keep_dependency_cfg_separate_and_match_cargo_oracles() {
    use entmoot_contracts::{RustAnalysisMode, RustBuildProfile};
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    for path in ["src", "dep/src", "tests"] {
        std::fs::create_dir_all(root.join(path)).unwrap();
    }
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='profile-scope'\nversion='0.1.0'\nedition='2021'\n[features]\ninvalid-dependency-test=[]\n[dependencies]\nprofile-dep={path='dep'}\n").unwrap();
    std::fs::write(
        root.join("dep/Cargo.toml"),
        "[package]\nname='profile-dep'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    let dependency = "#[cfg(not(test))] pub fn value() -> i32 { 17 }\n#[cfg(test)] pub fn value() -> i32 { 999 }\n#[cfg(test)] pub fn test_only() {}\n";
    std::fs::write(root.join("dep/src/lib.rs"), dependency).unwrap();
    let library = "mod one; mod two;\n#[cfg(not(test))] pub fn value() -> i32 { 23 }\n#[cfg(test)] pub fn value() -> i32 { 999 }\n#[test] fn library_check() { let local = value(); let dep = profile_dep::value(); assert_eq!(local, 999); assert_eq!(dep, 17); assert_eq!(one::run(), 1); assert_eq!(two::run(), 2); }\n";
    std::fs::write(root.join("src/lib.rs"), library).unwrap();
    for (name, value) in [("one", 1), ("two", 2)] {
        std::fs::write(root.join(format!("src/{name}.rs")), format!("fn leaf() -> i32 {{ {value} }}\n#[path=\"shared.rs\"] mod shared;\npub fn run() -> i32 {{ shared::run() }}\n")).unwrap();
    }
    std::fs::write(
        root.join("src/shared.rs"),
        "pub fn run() -> i32 { super::leaf() }\n",
    )
    .unwrap();
    let calls = "#[test] fn integration_check() { let local = profile_scope::value(); let dep = profile_dep::value(); assert_eq!(local, 23); assert_eq!(dep, 17); }\n#[cfg(feature=\"invalid-dependency-test\")] #[test] fn invalid() { profile_dep::test_only(); }\n";
    std::fs::write(root.join("tests/calls.rs"), calls).unwrap();

    // Cargo is an independent runtime oracle; expected values and occurrences
    // above are authored, not learned from Entmoot or rust-analyzer output.
    for (args, accepted) in [
        (vec!["--lib"], true),
        (vec!["--test", "calls"], true),
        (
            vec!["--test", "calls", "--features", "invalid-dependency-test"],
            false,
        ),
    ] {
        let output = std::process::Command::new("cargo")
            .args(["test", "--offline", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(dir.path().join("oracle-target"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            accepted,
            "cargo {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !accepted {
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("cannot find function `test_only`"));
        }
    }
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    for (case, crate_root, invalid) in [
        ("library", "src/lib.rs", false),
        ("integration", "tests/calls.rs", false),
        ("negative", "tests/calls.rs", true),
    ] {
        let profile = RustBuildProfile {
            analysis: RustAnalysisMode::Test {
                crate_root: crate_root.into(),
            },
            features: entmoot_contracts::RustFeatureSelection::Selected {
                names: if invalid {
                    ["invalid-dependency-test".into()].into()
                } else {
                    Default::default()
                },
                no_default_features: false,
            },
            ..Default::default()
        };
        let result = produce_rust_resolver(NativeProduceRequest {
            canon: false,
            graph_path: graph_path.clone(),
            source_root: root.clone(),
            output_dir: dir.path().join(case),
            budget: ProducerBudget {
                wall: Duration::from_secs(180),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy::Safe,
            rust_profile: profile.clone(),
        })
        .unwrap();
        assert!(
            !result.raw.invalid_environment,
            "{:?}",
            result.raw.diagnostics
        );
        assert_eq!(
            result.raw.build_context["rust_profile"],
            serde_json::to_value(profile).unwrap()
        );
        let checks: &[(&str, &str, &str)] = if crate_root == "src/lib.rs" {
            &[
                ("value", "src/lib.rs", "fn value() -> i32 { 999 }"),
                (
                    "profile_dep::value",
                    "dep/src/lib.rs",
                    "fn value() -> i32 { 17 }",
                ),
            ]
        } else {
            &[
                (
                    "profile_scope::value",
                    "src/lib.rs",
                    "fn value() -> i32 { 23 }",
                ),
                (
                    "profile_dep::value",
                    "dep/src/lib.rs",
                    "fn value() -> i32 { 17 }",
                ),
            ]
        };
        for (callee, target_path, definition) in checks {
            let caller_source = if crate_root == "src/lib.rs" {
                library
            } else {
                calls
            };
            let call_selector = format!("= {callee}()");
            assert_eq!(caller_source.matches(&call_selector).count(), 1);
            let call_start = caller_source.find(&call_selector).unwrap() + 2;
            let raw: Vec<_> = result
                .raw
                .calls
                .iter()
                .filter(|call| {
                    call.site.path == crate_root && call.site.span.start_byte == call_start
                })
                .collect();
            assert_eq!(
                raw.len(),
                1,
                "{case} {callee}: {:?}",
                result.raw.diagnostics
            );
            assert_eq!(raw[0].targets.len(), 1, "{case} {callee}: {:?}", raw[0]);
            let target = raw[0].targets[0].location.repository().unwrap();
            let text = if *target_path == "src/lib.rs" {
                library
            } else {
                dependency
            };
            // Provider function ranges include visibility, independently located
            // inside the single authored declaration selected by cfg.
            let start = text.find(definition).unwrap();
            assert_eq!(target.path, *target_path);
            assert!(
                target.span.start_byte <= start && target.span.end_byte == start + definition.len(),
                "{case} {callee}: {target:?}"
            );
            let nodes: Vec<_> = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == entmoot_engine::NodeKind::CallStub
                        && node.path.as_deref() == Some(crate_root)
                        && node.start_byte == Some(call_start)
                })
                .collect();
            assert_eq!(nodes.len(), 1);
            let call_site = entmoot_contracts::Address::Opaque {
                system: "entmoot.topology_node".into(),
                value: nodes[0].id.clone(),
            };
            let observation = result
                .artifact
                .observations
                .iter()
                .find(|o| o.source.call_site == call_site)
                .unwrap();
            assert_eq!(
                observation.outcome,
                ResolverObservationOutcome::RepositoryCallable,
                "{case} {callee}: {observation:?}"
            );
            assert!(
                observation.dispatch.is_closed(),
                "{case} {callee}: {observation:?}"
            );
        }
        assert!(result
            .raw
            .diagnostics
            .iter()
            .any(|reason| reason.contains("no unique active module context: src/shared.rs")));
        let shared_calls: Vec<_> = result
            .artifact
            .observations
            .iter()
            .filter(|o| o.source.syntactic_callee == "super::leaf")
            .collect();
        assert_eq!(shared_calls.len(), 1);
        assert_ne!(
            shared_calls[0].outcome,
            ResolverObservationOutcome::RepositoryCallable
        );
        assert!(shared_calls[0]
            .targets
            .iter()
            .all(|target| target.relation_target.is_none()));
        if invalid {
            let site = result
                .artifact
                .observations
                .iter()
                .find(|o| o.source.syntactic_callee == "profile_dep::test_only")
                .unwrap();
            assert_ne!(site.outcome, ResolverObservationOutcome::RepositoryCallable);
            assert!(site
                .targets
                .iter()
                .all(|target| target.relation_target.is_none()));
        }
    }
}

#[test]
fn macro_expansions_and_trait_implementations_keep_physical_identity() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='macro-audit'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        r#"
pub struct Api;
pub trait First { fn value(&self) -> i32; }
pub trait Second { fn value(&self) -> i32; }
impl First for Api { fn value(&self) -> i32 { 1 } }
impl Second for Api { fn value(&self) -> i32 { 2 } }
macro_rules! generated { () => { pub fn virtual_leaf() -> i32 { 3 } }; }
generated!();
mod hidden { #[macro_export] macro_rules! root_macro { () => { 1 }; } }
pub fn caller(value: &dyn First) -> i32 { value.value() + virtual_leaf() }
"#,
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("resolver"),
        budget: ProducerBudget {
            wall: Duration::from_secs(180),
            ..Default::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(result.raw_path).unwrap()).unwrap();
    let captures = raw["build_context"]["macro_expansions"].as_array().unwrap();
    assert!(
        captures
            .iter()
            .any(|capture| capture["outcome"] == "expanded_virtual_source"
                && capture["expanded_text"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("virtual_leaf")),
        "{captures:?}"
    );
    assert!(result
        .artifact
        .observations
        .iter()
        .filter(|observation| observation.source.syntactic_callee.contains("virtual_leaf"))
        .all(|observation| observation.outcome != ResolverObservationOutcome::RepositoryCallable));
    let api = result.artifact.package_api.unwrap();
    let symbols: Vec<_> = api
        .surfaces
        .iter()
        .flat_map(|surface| &surface.exports)
        .map(|export| export.target.symbol.spelling.as_str())
        .collect();
    assert!(
        symbols.iter().any(|symbol| symbol.contains("as First>")),
        "{symbols:?}"
    );
    assert!(
        symbols.iter().any(|symbol| symbol.contains("as Second>")),
        "{symbols:?}"
    );
    assert!(
        !symbols
            .iter()
            .any(|symbol| symbol.ends_with("::virtual_leaf")),
        "{symbols:?}"
    );
    assert!(
        !symbols.iter().any(|symbol| symbol.ends_with("::generated")),
        "{symbols:?}"
    );
    assert!(symbols.contains(&"macro-audit::root_macro"), "{symbols:?}");
}

#[test]
fn trusted_generated_source_is_retained_without_inventing_inventory() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='generated-audit'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("build.rs"),
        r#"fn main() {
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("generated.rs"), "pub fn generated_leaf() -> i32 { 1 }").unwrap();
}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        r#"include!(concat!(env!("OUT_DIR"), "/generated.rs"));
pub fn caller() -> i32 { generated_leaf() }
"#,
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("resolver"),
        budget: ProducerBudget {
            wall: Duration::from_secs(180),
            ..Default::default()
        },
        execution_policy: ExecutionPolicy::Trusted,
        rust_profile: Default::default(),
    })
    .unwrap();
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(result.raw_path).unwrap()).unwrap();
    assert!(!raw["invalid_environment"].as_bool().unwrap(), "{raw}");
    assert!(
        raw["build_context"]["workspace_source_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|workspace| workspace["input_digests"]
                .as_object()
                .unwrap()
                .keys()
                .any(|file| file.ends_with("generated.rs"))),
        "{raw}"
    );
    let generated: Vec<_> = result
        .artifact
        .observations
        .iter()
        .filter(|observation| {
            observation
                .source
                .syntactic_callee
                .contains("generated_leaf")
        })
        .collect();
    assert!(!generated.is_empty());
    assert!(generated
        .iter()
        .all(|observation| observation.outcome != ResolverObservationOutcome::RepositoryCallable));
}

#[test]
fn external_path_dep_emits_package_api_reference_without_consumer_local_sources() {
    // Sibling path dep is outside the consumer source_root inventory; package coords
    // still qualify seeking or assessed refs via fill / rust_boundary.
    let dir = tempdir().unwrap();
    let helper = dir.path().join("helper");
    let root = dir.path().join("consumer");
    std::fs::create_dir_all(helper.join("src")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        helper.join("Cargo.toml"),
        "[package]\nname='helper'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    std::fs::write(helper.join("src/lib.rs"), "pub fn greet() -> i32 { 1 }\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='consumer'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nhelper={path='../helper'}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn caller() -> i32 { helper::greet() }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/rust-external-seek"),
        },
    )
    .unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_rust_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("resolver"),
        budget: ProducerBudget {
            wall: Duration::from_secs(180),
            ..Default::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let api = result.artifact.package_api.expect("package_api evidence");
    let reference = api
        .references
        .iter()
        .find(|reference| reference.target.package.name == "helper")
        .expect("expected helper package_api reference for external path dep");
    assert!(
        reference.boundary_policy == "unassessed"
            || reference.expected_boundary.is_some()
            || !reference.boundary_policy.is_empty(),
        "expected seeking or assessed helper ref, got {reference:?}"
    );
    assert!(
        reference.target.symbol.spelling.contains("greet"),
        "{:?}",
        reference.target.symbol.spelling
    );
}
