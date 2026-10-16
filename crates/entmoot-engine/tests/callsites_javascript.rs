//! Oxc JS resolver qualification against the mini fixture.

#![cfg(feature = "resolved-oxc")]

use std::path::PathBuf;
use std::time::Duration;

use entmoot_contracts::{CallDispatch, ResolverObservationOutcome};
use entmoot_engine::native_resolver::{
    oxc::OXC_PIN, produce_oxc_resolver, ExecutionPolicy, NativeProduceRequest, ProducerBudget,
};
use entmoot_engine::{
    build_topology_from_repo_with_options, write_graph_json, TopologyBuildOptions,
};
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/entmoot-engine/tests/fixtures/callsites/javascript/smoke")
}

#[test]
fn aliases_reexports_commonjs_and_syntax_hops_bind_only_immutable_routes() {
    for (extension, header) in [
        ("js", "export function leaf(x)"),
        ("ts", "export function leaf<T>(x: T)"),
        ("tsx", "export function leaf<T,>(x: T)"),
        ("mts", "export function leaf<T>(x: T)"),
        ("cts", "export function leaf<T>(x: T)"),
    ] {
        let dir = tempdir().unwrap();
        let root = dir.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join(format!("impl.{extension}")),
            format!("{header} {{ return x; }}"),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("api.{extension}")),
            format!("export {{leaf as publicLeaf}} from './impl.{extension}';"),
        )
        .unwrap();
        std::fs::write(root.join(format!("main.{extension}")), format!(
            "import {{publicLeaf as renamed}} from './api.{extension}';\nconst {{leaf: required}} = require('./dep.cjs');\nexport function caller() {{ return renamed(1) + required(2); }}")).unwrap();
        std::fs::write(
            root.join("dep.cjs"),
            "function leaf(x) { return x; } exports.leaf = leaf;",
        )
        .unwrap();
        let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
        let graph_path = dir.path().join("graph.json");
        write_graph_json(&graph, &graph_path).unwrap();
        let result = produce_oxc_resolver(NativeProduceRequest {
            canon: false,
            graph_path,
            source_root: root,
            output_dir: dir.path().join("native"),
            budget: ProducerBudget::default(),
            execution_policy: ExecutionPolicy::Safe,
            rust_profile: Default::default(),
        })
        .unwrap();
        for callee in ["renamed", "required"] {
            let site = result
                .artifact
                .observations
                .iter()
                .find(|site| site.source.syntactic_callee == callee)
                .unwrap_or_else(|| panic!("missing {extension} {callee}"));
            if extension == "cts" && callee == "renamed" {
                // The lexical tier does not claim TypeScript's ESM-to-CJS
                // export transformation. The explicit Program tier owns it.
                assert_eq!(site.outcome, ResolverObservationOutcome::Unsupported);
                assert_eq!(
                    site.dispatch,
                    CallDispatch::Open {
                        reason: entmoot_contracts::DispatchGap::UnsupportedConstruct,
                    }
                );
                assert!(site.targets.is_empty());
                continue;
            }
            assert_eq!(
                site.outcome,
                ResolverObservationOutcome::RepositoryCallable,
                "{extension} {callee}: {site:?}"
            );
            // Closed product claim: dispatch_expectations javascript/identifier.
        }
    }
}

#[test]
fn member_calls_stay_unbound_and_open() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("main.js"),
        "export const api = { leaf() { return 1; } };\nexport function caller() { return api.leaf(); }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget {
            wall: Duration::from_secs(60),
            ..Default::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    assert_eq!(
        result.artifact.observations.len(),
        1,
        "{:?}",
        result.artifact.observations
    );
    let site = &result.artifact.observations[0];
    assert!(
        site.targets.is_empty(),
        "Oxc member calls are unbound today (Closed product claim: dispatch_expectations javascript/member): {site:?}"
    );
    assert!(
        matches!(&site.dispatch, CallDispatch::Open { .. }),
        "{site:?}"
    );
    assert!(!site.dispatch.is_closed());
}

#[test]
fn hoisted_functions_and_late_imports_bind_before_textual_declaration() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("main.js"),
        "export function caller() { return leaf() + imported() + hidden() + publicLeaf(); }\nexport function leaf() { return 1; }\nimport { remote as imported } from 'example-api';\nimport { hidden, publicLeaf } from './dep.js';\n").unwrap();
    std::fs::write(
        root.join("dep.js"),
        "function hidden() { return 0; }\nexport function publicLeaf() { return 1; }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/hoisted"),
        },
    )
    .unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget::default(),
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let local = result
        .artifact
        .observations
        .iter()
        .find(|row| row.source.syntactic_callee == "leaf")
        .unwrap();
    assert_eq!(
        local.outcome,
        ResolverObservationOutcome::RepositoryCallable
    );
    let external = result
        .artifact
        .observations
        .iter()
        .find(|row| row.source.syntactic_callee == "imported")
        .unwrap();
    assert_eq!(external.outcome, ResolverObservationOutcome::ExternalSymbol);
    let hidden = result
        .artifact
        .observations
        .iter()
        .find(|row| row.source.syntactic_callee == "hidden")
        .unwrap();
    assert!(
        hidden.targets.is_empty(),
        "a private declaration cannot satisfy an ESM import"
    );
    let public = result
        .artifact
        .observations
        .iter()
        .find(|row| row.source.syntactic_callee == "publicLeaf")
        .unwrap();
    assert_eq!(
        public.outcome,
        ResolverObservationOutcome::RepositoryCallable
    );
}

#[test]
fn js_smoke_keeps_declaration_and_relative_import_joins() {
    // Construct Open/Closed product rows: dispatch_expectations javascript/*.
    // This control fences the Oxc pin plus declaration / relative-import joins
    // that are not expressible as shared product fields alone.
    let root = fixture_root();
    assert!(
        root.join("caller.js").is_file(),
        "missing fixture at {}",
        root.display()
    );
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/js-oxc"),
        },
    )
    .expect("index fixture");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let result = produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("resolver-out"),
        budget: ProducerBudget {
            wall: Duration::from_secs(60),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .expect("produce oxc resolver");

    assert_eq!(result.receipt.producer.adapter_pin, OXC_PIN);
    let help = result
        .artifact
        .observations
        .iter()
        .find(|observation| {
            observation.source.syntactic_callee.contains("help")
                && observation.outcome == ResolverObservationOutcome::RepositoryCallable
        })
        .expect("expected repository_callable for help");
    assert!(
        help.targets
            .first()
            .and_then(|target| target.declaration.as_ref())
            .is_some(),
        "repository_callable must carry ResolverDeclaration"
    );
    assert!(
        result.artifact.observations.iter().any(|observation| {
            observation.source.syntactic_callee.contains("callback")
                && matches!(
                    observation.outcome,
                    ResolverObservationOutcome::Ambiguous
                        | ResolverObservationOutcome::Unsupported
                        | ResolverObservationOutcome::Unresolved
                )
        }),
        "indirect callback must remain non-callable"
    );
    assert!(
        result.artifact.observations.iter().any(|observation| {
            observation.source.syntactic_callee.contains("remoteHelp")
                && observation.outcome == ResolverObservationOutcome::RepositoryCallable
                && observation.targets.first().is_some_and(|target| {
                    target.declaration.is_some() && target.relation_target.is_some()
                })
        }),
        "relative named import remoteHelp must join unique inventory callable"
    );
}

#[test]
fn ts_fixture_joins_under_javascript_pipeline() {
    let root = fixture_root();
    assert!(
        root.join("caller.ts").is_file(),
        "missing fixture at {}",
        root.join("caller.ts").display()
    );
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/js-oxc-ts"),
        },
    )
    .expect("index fixture");
    assert!(
        graph.nodes.iter().any(|node| {
            node.path.as_deref() == Some("caller.ts")
                && node.kind == entmoot_engine::graph::NodeKind::CallStub
        }),
        "expected CallStub nodes under caller.ts"
    );
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let out = dir.path().join("resolver-out");

    let result = produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: out,
        budget: ProducerBudget {
            wall: Duration::from_secs(60),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .expect("produce oxc resolver");

    assert!(
        result.artifact.observations.iter().any(|observation| {
            observation.source.syntactic_callee.contains("help")
                && observation.outcome == ResolverObservationOutcome::RepositoryCallable
                && observation.targets.first().is_some_and(|target| {
                    target.declaration.is_some() && target.relation_target.is_some()
                })
                && matches!(
                    &observation.dispatch,
                    CallDispatch::Closed { policy_id, .. }
                        if policy_id == "entmoot.oxc.direct-identifier.v1"
                )
        }),
        "TS help call must join Closed under javascript pipeline; status={} limitations={:?}",
        result.receipt.semantic_status.as_str(),
        result.receipt.limitations
    );
}

#[test]
fn bare_package_import_emits_external_symbol_with_npm_purl() {
    let root = fixture_root();
    assert!(
        root.join("package_caller.js").is_file(),
        "missing fixture at {}",
        root.join("package_caller.js").display()
    );
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/js-oxc-pkg"),
        },
    )
    .expect("index fixture");
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).expect("write graph");
    let out = dir.path().join("resolver-out");

    let result = produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: out,
        budget: ProducerBudget {
            wall: Duration::from_secs(60),
            ..ProducerBudget::default()
        },
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .expect("produce oxc resolver");

    let external = result
        .artifact
        .observations
        .iter()
        .find(|observation| {
            observation.source.syntactic_callee.contains("debounce")
                && observation.outcome == ResolverObservationOutcome::ExternalSymbol
        })
        .expect("expected ExternalSymbol for lodash debounce");
    let provider_callable = external
        .targets
        .first()
        .and_then(|target| target.provider_callable.as_ref())
        .expect("provider_callable on external target");
    assert_eq!(
        provider_callable.package.purl.as_deref(),
        Some("pkg:npm/lodash")
    );
    assert!(external.targets[0].relation_target.is_none());
}

#[test]
fn cjs_default_require_binding_emits_unassessed_package_api_reference() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"consumer","type":"commonjs"}"#,
    )
    .unwrap();
    // Classic Express pattern: BindingIdentifier + require('pkg'), then call through binding.
    std::fs::write(
        root.join("app.js"),
        "var accepts = require('accepts');\nfunction handle(req) { return accepts(req); }\n",
    )
    .unwrap();
    let graph = build_topology_from_repo_with_options(
        &root,
        TopologyBuildOptions {
            policy: None,
            repository: Some("fixture/cjs-default-require"),
        },
    )
    .unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    let result = produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget::default(),
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap();
    let api = result
        .artifact
        .package_api
        .expect("package_api evidence expected");
    let reference = api
        .references
        .iter()
        .find(|reference| {
            reference.target.symbol.spelling.contains("accepts#default")
                && reference.boundary_policy == "unassessed"
        })
        .expect("expected unassessed accepts#default package_api reference");
    assert_eq!(reference.target.package.name, "accepts");
    assert!(
        reference.expected_boundary.is_none(),
        "seeking refs must not invent expected_boundary without package-root enrich"
    );
}

#[allow(clippy::unwrap_used)]
fn produce_sources(files: &[(&str, &str)]) -> entmoot_engine::native_resolver::NativeProduceResult {
    let dir = tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir(&root).unwrap();
    for (name, source) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let graph = build_topology_from_repo_with_options(&root, Default::default()).unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph, &graph_path).unwrap();
    produce_oxc_resolver(NativeProduceRequest {
        canon: false,
        graph_path,
        source_root: root,
        output_dir: dir.path().join("native"),
        budget: ProducerBudget::default(),
        execution_policy: ExecutionPolicy::Safe,
        rust_profile: Default::default(),
    })
    .unwrap()
}

#[test]
fn relative_imports_cannot_escape_or_choose_ambiguous_in_root_modules() {
    for extension in ["js", "ts", "tsx"] {
        for (caller_path, request, expected) in [
            (
                format!("main.{extension}"),
                format!("../dep.{extension}"),
                false,
            ),
            (
                format!("sub/main.{extension}"),
                format!("../../dep.{extension}"),
                false,
            ),
            (
                format!("sub/main.{extension}"),
                format!("../dep.{extension}"),
                true,
            ),
            (format!("main.{extension}"), "./ambiguous".into(), false),
        ] {
            let source = format!(
                "import {{leaf}} from '{request}'; export function caller() {{ return leaf(); }}"
            );
            let dep = format!("dep.{extension}");
            let result = produce_sources(&[
                (&caller_path, &source),
                (&dep, "export function leaf() { return 1; }"),
                ("ambiguous.js", "export function leaf() {}"),
                ("ambiguous.ts", "export function leaf() {}"),
            ]);
            let site = result
                .artifact
                .observations
                .iter()
                .find(|row| row.source.syntactic_callee == "leaf")
                .unwrap();
            assert_eq!(
                site.outcome == ResolverObservationOutcome::RepositoryCallable,
                expected,
                "{extension} {request}: {site:?}"
            );
            if !expected {
                assert!(site.targets.is_empty());
            }
        }
    }
}

#[test]
fn property_assigned_functions_keep_exact_owners_without_property_dispatch_claims() {
    for extension in ["js", "ts", "tsx"] {
        let result = produce_sources(&[(
            &format!("main.{extension}"),
            r#"
import accepts from 'accepts';
const req = {};
req.accepts = function(types) { return accepts(types); };
req.arrow = (types) => { return accepts(types); };
req.chain = req.alias = function(types) { return accepts(types); };
function outer() {
    req.nested = function(types) { return accepts(types); };
    [1].map(function callback(types) { return accepts(types); });
    [1].map((types) => accepts(types));
    req.accepts('text');
}
"#,
        )]);
        let sites: Vec<_> = result
            .artifact
            .observations
            .iter()
            .filter(|row| row.source.syntactic_callee == "accepts")
            .collect();
        assert_eq!(sites.len(), 6);
        assert_eq!(
            sites
                .iter()
                .filter(|row| row.outcome == ResolverObservationOutcome::ExternalSymbol)
                .count(),
            4,
            "{extension}: {sites:?}"
        );
        // Unrepresented callbacks must remain unknown; never lift their calls to outer.
        assert_eq!(
            sites
                .iter()
                .filter(|row| row.outcome == ResolverObservationOutcome::Unsupported)
                .count(),
            2
        );
        for site in result
            .artifact
            .observations
            .iter()
            .filter(|row| row.source.syntactic_callee.contains("req.accepts"))
        {
            assert!(site.targets.is_empty());
        }
    }
}

#[test]
fn declaration_files_parse_and_invalid_modules_are_explicitly_partial() {
    for declaration in ["api.d.ts", "api.d.mts", "api.d.cts"] {
        let result = produce_sources(&[
            (
                declaration,
                "declare const version: string; export {version};",
            ),
            (
                "main.ts",
                "function leaf() {} export function caller() { leaf(); }",
            ),
        ]);
        assert!(
            result.raw.diagnostics.is_empty(),
            "{declaration}: {:?}",
            result.raw.diagnostics
        );
        assert_eq!(result.receipt.semantic_status.as_str(), "ok");
    }
    let result = produce_sources(&[
        (
            "bad.ts",
            "const broken; function call() { leaf(); } function leaf() {}",
        ),
        (
            "good.ts",
            "function leaf() {} export function caller() { leaf(); }",
        ),
    ]);
    assert_eq!(result.receipt.semantic_status.as_str(), "partial");
    assert!(!result.raw.diagnostics.is_empty());
    assert!(result.raw.calls.iter().all(|row| row.site.path != "bad.ts"));
    assert!(result
        .artifact
        .observations
        .iter()
        .any(|row| row.outcome == ResolverObservationOutcome::RepositoryCallable));
}

#[test]
fn default_identifier_exports_bind_assigned_callables_with_open_dispatch() {
    for extension in ["js", "ts", "tsx"] {
        for initializer in ["async () => {}", "function () {}", "() => 1"] {
            let target =
                format!("const FrontendTracer = {initializer}; export default FrontendTracer;");
            let result = produce_sources(&[
                (
                    &format!("caller.{extension}"),
                    "import trace from './target'; trace();",
                ),
                (&format!("target.{extension}"), &target),
            ]);
            let site = result
                .artifact
                .observations
                .iter()
                .find(|site| site.source.syntactic_callee == "trace")
                .unwrap();
            assert_eq!(
                site.outcome,
                ResolverObservationOutcome::RepositoryCallable,
                "{extension}: {site:?}"
            );
            assert_eq!(site.targets.len(), 1);
            assert!(site.targets[0].relation_target.is_some());
            assert!(!site.dispatch.is_closed());
        }
    }
    for (caller, target, ambiguous) in [
        (
            "import trace from './target'; trace();",
            "let trace = () => {}; trace = other; export default trace;",
            false,
        ),
        (
            "import type trace from './target'; trace();",
            "const trace = () => {}; export default trace;",
            false,
        ),
        (
            "import('./target').then(trace => trace());",
            "const trace = () => {}; export default trace;",
            false,
        ),
        (
            "import trace from './target'; trace();",
            "const trace = () => {}; export default trace;",
            true,
        ),
    ] {
        let mut files = vec![("caller.ts", caller), ("target.ts", target)];
        if ambiguous {
            files.push(("target.js", target));
        }
        let result = produce_sources(&files);
        assert!(
            result
                .artifact
                .observations
                .iter()
                .all(|site| site.outcome != ResolverObservationOutcome::RepositoryCallable),
            "{:?}",
            result.artifact.observations
        );
    }
}

#[test]
fn builtin_imports_never_fabricate_npm_coordinates() {
    let result = produce_sources(&[("main.cjs", "const {resolve} = require('path'); const {join} = require('node:path'); const external = require('path-extra'); function caller() { resolve('a'); join('b'); external(); }")]);
    for name in ["resolve", "join", "external"] {
        let site = result
            .artifact
            .observations
            .iter()
            .find(|site| site.source.syntactic_callee == name)
            .unwrap();
        assert_eq!(
            site.outcome,
            ResolverObservationOutcome::ExternalSymbol,
            "{site:?}"
        );
        let target = serde_json::to_value(&site.targets[0]).unwrap();
        let serialized = target.to_string();
        let package = &site.targets[0].provider_callable.as_ref().unwrap().package;
        let runtime = result.raw.build_context["javascript_runtime"]
            .as_str()
            .unwrap();
        let expected_role = if name == "external" {
            entmoot_contracts::ProviderPackageRole::Module
        } else if runtime == "node" || (runtime == "unspecified" && name == "join") {
            entmoot_contracts::ProviderPackageRole::Stdlib
        } else {
            entmoot_contracts::ProviderPackageRole::Unknown
        };
        assert_eq!(package.role, expected_role);
        if name == "external" {
            assert!(serialized.contains("pkg:npm/path-extra"), "{serialized}");
        } else {
            assert!(!serialized.contains("pkg:npm/"), "{serialized}");
        }
        assert!(!site.dispatch.is_closed());
    }
}
