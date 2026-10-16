//! Production occurrences and independently authored provider comparison controls.
//! Authored resolver records test accounting; they are not compiler qualification.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use entmoot_contracts::query::*;
use entmoot_contracts::*;
use entmoot_engine::*;

#[test]
fn declared_contracts_survive_reopen_and_report_revision_changes_without_break_verdicts() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let db = directory.path().join("knowledge.sqlite3");
    let mut versions = Vec::new();
    for ty in ["int", "str"] {
        std::fs::write(source.join("case.py"), format!("class Box:\n    value: {ty}\ndef echo(value: {ty}) -> {ty}:\n    return value\ndef caller():\n    return echo(1)\n")).unwrap();
        let graph = build_topology_from_repo_with_options(
            &source,
            TopologyBuildOptions {
                repository: Some("contract-lifecycle"),
                ..Default::default()
            },
        )
        .unwrap();
        let view = blake3_hex(ty.as_bytes());
        let mut writer = SqliteKnowledgeRepository::open_view(&db, &view).unwrap();
        writer
            .replace_all(&KnowledgeStore::extract_from_graph(&graph))
            .unwrap();
        writer.store_inventory(&graph, None).unwrap();
        drop(writer);
        let reader = SqliteKnowledgeRepository::open_view_read_only(&db, &view).unwrap();
        let eager = reader.load_inventory(false).unwrap().unwrap();
        let lazy = reader.load_inventory(true).unwrap().unwrap();
        let locator = directory.path().join(format!("{ty}.view.json"));
        std::fs::write(
            &locator,
            serde_json::to_vec(&serde_json::json!({"database":db,"view":view})).unwrap(),
        )
        .unwrap();
        let node = eager
            .nodes
            .iter()
            .find(|n| n.label == "echo" && n.kind == NodeKind::Callable)
            .unwrap();
        let value = serde_json::to_value(node).unwrap();
        assert_eq!(
            value["code_contract"]["shape"]["parameters"][0]["declared_type"]["syntax"],
            ty
        );
        for graph in [&eager, &lazy] {
            let query = QueryEngine::new(graph.clone(), &locator);
            let request = serde_json::from_value(serde_json::json!({"operation":"inspect", "request":{"kind":"subject","id":node.id,"limit":20},"output":{"tier":"export"}})).unwrap();
            let response = query.execute(request).unwrap().into_value().unwrap();
            assert_eq!(
                response["result"]["code_contract"], value["code_contract"],
                "{response}"
            );
        }
        versions.push(eager);
    }
    let impact =
        serde_json::to_value(compare_revision_impact(&versions[0], &versions[1]).unwrap()).unwrap();
    let changes = impact["contracts"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{impact}");
    for name in ["echo", "Box"] {
        let change = changes.iter().find(|c| c["base"]["name"] == name).unwrap();
        assert_eq!(change["kind"], "changed");
        assert!(change.get("breaking").is_none());
    }
    let old = SqliteKnowledgeRepository::open_view_read_only(&db, &blake3_hex(b"int")).unwrap();
    assert_eq!(
        serde_json::to_value(old.load_inventory(false).unwrap().unwrap()).unwrap(),
        serde_json::to_value(&versions[0]).unwrap()
    );

    let mut partial = versions[1].clone();
    partial
        .nodes
        .iter_mut()
        .find(|n| n.label == "echo" && n.kind == NodeKind::Callable)
        .unwrap()
        .code_contract = None;
    let impact = compare_revision_impact(&versions[0], &partial).unwrap();
    let echo = impact
        .contracts
        .iter()
        .find(|c| c.base.as_ref().is_some_and(|b| b.name == "echo"))
        .unwrap();
    assert_eq!(echo.kind, CodeContractImpactKind::NotComparable);

    let mut forged = versions[1].clone();
    forged
        .nodes
        .iter_mut()
        .find(|n| n.label == "echo" && n.kind == NodeKind::Callable)
        .unwrap()
        .code_contract
        .as_mut()
        .unwrap()
        .site
        .span
        .end_byte -= 1;
    assert!(
        forged.validate().is_err(),
        "shifted contract must not attach to the occurrence"
    );
}

mod revision_fixture {
    include!("../../entmoot-core/tests/support/revision_pairs.rs");
}

fn inventory(
    root: &std::path::Path,
    revision: &revision_fixture::Revision,
    cpp: bool,
) -> TopologyGraph {
    let path = if cpp {
        revision.path.replace(".c", ".cpp")
    } else {
        revision.path.clone()
    };
    std::fs::create_dir_all(root.join(&path).parent().unwrap()).unwrap();
    std::fs::write(root.join(&path), &revision.source).unwrap();
    build_topology_from_repo_with_options(
        root,
        TopologyBuildOptions {
            repository: Some("visitor-fixtures"),
            ..Default::default()
        },
    )
    .unwrap()
}

fn evidence(
    graph: &mut TopologyGraph,
    revision: &revision_fixture::Revision,
    case: &str,
    head: bool,
) {
    if case == "missing" {
        return;
    }
    let version = if case == "series_drift" && head {
        "2"
    } else {
        "1"
    };
    let producers = if case == "secondary_change" {
        vec![
            "authored-accounting-control",
            "secondary-accounting-control",
        ]
    } else {
        vec!["authored-accounting-control"]
    };
    for producer in producers {
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Resolver,
            ArtifactProducer {
                name: producer.into(),
                version: Some(version.into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            graph.snapshot.clone(),
            ScopeUniverse::from_selectors(
                "fixture",
                vec![],
                vec!["**".into()],
                vec![],
                ScopeCompleteness::Partial,
                None,
            ),
            blake3_hex(format!("authored {producer} {case} {head}").as_bytes()),
            None,
        )
        .unwrap();
        for selector in &revision.occurrences {
            let matches: Vec<_> = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == NodeKind::Callable
                        && node.label == selector.name
                        && node.start_byte == Some(selector.span.start)
                        && node.end_byte == Some(selector.span.end)
                })
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "explicit fixture selector {}",
                selector.key
            );
            let source = ResolverEvidenceRef {
                artifact: ArtifactRef::from_manifest(&manifest).unwrap(),
                producer: ProducerIdentity {
                    producer_id: producer.into(),
                    version: Some(version.into()),
                    tool_name: None,
                },
            };
            let reference_count =
                u64::from(case != "empty" && !(case == "mixed_empty" && selector.key == "focus"));
            let binding = if case == "partial" && head {
                ResolvedBindingEvidence::Partial {
                    source,
                    reference_count,
                    unresolved_count: 1,
                    ambiguous_count: 0,
                    reason_codes: vec!["authored_missing_reference".into()],
                }
            } else {
                ResolvedBindingEvidence::Complete {
                    source,
                    reference_count,
                }
            };
            let changed = head
                && ((case == "changed_focus" && selector.key == "focus")
                    || (case == "changed_sibling" && selector.key == "sibling")
                    || (case == "secondary_change"
                        && producer == "secondary-accounting-control"
                        && selector.key == "focus"));
            graph.resolved_identities.push(ResolvedIdentityRecord {
                address_id: matches[0].address_id.clone().unwrap(),
                resolved_hash: blake3_hex(if changed {
                    b"changed binding"
                } else {
                    b"same binding"
                }),
                resolution_policy_id: "a".repeat(64),
                binding,
            });
        }
        graph.artifacts.push(manifest);
    }
    graph.validate().unwrap();
}

#[test]
fn current_receipts_preserve_occurrences_fingerprints_and_authored_binding_controls() {
    revision_controls(None);
}

fn revision_controls(output: Option<&std::path::Path>) {
    let corpus = revision_fixture::load();
    if let Some(path) = output {
        std::fs::create_dir(path).unwrap();
    }
    for language in [
        "rust",
        "python",
        "javascript",
        "typescript",
        "tsx",
        "go",
        "java",
        "csharp",
        "c",
        "cpp",
    ] {
        let cpp = language == "cpp";
        let id = format!("{}-body", if cpp { "c" } else { language });
        let pair = corpus.pairs.iter().find(|pair| pair.id == id).unwrap();
        revision_fixture::verify(pair, &corpus.repository);
        for case in [
            "missing",
            "empty",
            "unchanged",
            "changed_focus",
            "changed_sibling",
            "partial",
            "series_drift",
            "mixed_empty",
            "secondary_change",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut base = inventory(&directory.path().join("base"), &pair.base, cpp);
            let mut head = inventory(&directory.path().join("head"), &pair.head, cpp);
            evidence(&mut base, &pair.base, case, false);
            evidence(&mut head, &pair.head, case, true);
            let impact = compare_revision_impact(&base, &head).unwrap();
            assert_eq!(impact.implementation, implementation_fingerprint());
            assert_eq!(impact.callables.len(), 1);
            let change = &impact.callables[0];
            change.occurrence.validate().unwrap();
            assert_eq!(change.kind, CallableImpactKind::BodyChanged);
            for (graph, id) in [(&base, &change.base_node_id), (&head, &change.head_node_id)] {
                let node = graph
                    .nodes
                    .iter()
                    .find(|node| Some(&node.id) == id.as_ref())
                    .unwrap();
                assert_eq!(
                    Some(change.occurrence.address_id.as_str()),
                    node.address_id.as_deref()
                );
                assert_eq!(
                    Some(change.occurrence.logic_address.as_str()),
                    node.logic_address.as_deref()
                );
            }
            let mut missing = serde_json::to_value(&impact).unwrap();
            missing.as_object_mut().unwrap().remove("implementation");
            assert!(serde_json::from_value::<RevisionImpactManifest>(missing).is_err());
            let mut legacy = serde_json::to_value(change).unwrap();
            legacy.as_object_mut().unwrap().remove("occurrence");
            legacy["logic_address"] = change.occurrence.logic_address.clone().into();
            assert!(serde_json::from_value::<CallableImpact>(legacy).is_err());
            for component in ["contracts", "derivation"] {
                let mut stale = head.clone();
                let changed = ContentDigest::from_trusted(blake3_hex(b"retired implementation"));
                if component == "contracts" {
                    stale.implementation.contracts = changed;
                } else {
                    stale.implementation.derivation = changed;
                }
                assert!(compare_revision_impact(&base, &stale).is_err());
            }
            let store = directory.path().join("knowledge.sqlite3");
            SqliteKnowledgeRepository::open(&store)
                .unwrap()
                .replace_all(&KnowledgeStore::extract_from_graph(&head))
                .unwrap();
            let engine = QueryEngine::new(head.clone(), store).with_compare_graph(base.clone());
            let relay = engine.relay_impact(ImpactRelayRequest::default()).unwrap();
            assert_eq!(relay.implementation, impact.implementation);
            assert_eq!(relay.items[0].callable.occurrence, change.occurrence);
            let mut missing = serde_json::to_value(&relay).unwrap();
            missing.as_object_mut().unwrap().remove("implementation");
            assert!(serde_json::from_value::<ImpactRelayResult>(missing).is_err());
            if let Some(output) = &output {
                let packet = serde_json::json!({"language":language,"case":case,"fixture":pair.id,
                    "base":base,"head":head,"impact":impact,"relay":relay});
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output.join(format!("{language}-{case}.json")))
                    .unwrap()
                    .write_all(&serde_json::to_vec(&packet).unwrap())
                    .unwrap();
            }
        }
    }
    delivery_controls(output);
}

fn delivery_controls(output: Option<&std::path::Path>) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let before = "fn alpha() -> i32 { 1 }\nfn beta() -> i32 { 1 }\nfn first() { alpha(); }\nfn second() { alpha(); }\nfn gone() {}\n";
    std::fs::write(source.join("lib.rs"), before).unwrap();
    let build = || {
        build_topology_from_repo_with_options(
            &source,
            TopologyBuildOptions {
                repository: Some("visitor-fixtures"),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let base = build();
    std::fs::write(
        source.join("lib.rs"),
        before
            .replace("{ 1 }", "{ 2 }")
            .replace("fn gone() {}\n", ""),
    )
    .unwrap();
    let head = build();
    let impact = compare_revision_impact(&base, &head).unwrap();
    assert_eq!(impact.callables.len(), 3);
    let store = directory.path().join("knowledge.sqlite3");
    SqliteKnowledgeRepository::open(&store)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&head))
        .unwrap();
    let engine = QueryEngine::new(head.clone(), store).with_compare_graph(base.clone());
    for case in ["paged", "truncated"] {
        let mut request = ImpactRelayRequest {
            evidence_filter: HopEvidenceFilter::Any,
            ..Default::default()
        };
        request.output.limit = 1;
        request.callers_output.limit = 1;
        if case == "truncated" {
            request.budget.max_examined_edges = 1;
        }
        let mut pages = Vec::new();
        let mut callers = std::collections::BTreeMap::new();
        loop {
            let page = engine.relay_impact(request.clone()).unwrap();
            for item in &page.items {
                if let Some(first) = &item.callers {
                    let mut next = first.page.next_cursor.clone();
                    let mut more = Vec::new();
                    while let Some(cursor) = next {
                        let mut query = (*first.query).clone();
                        query.set_output(OutputOptions {
                            after: Some(cursor),
                            limit: 1,
                            ..Default::default()
                        });
                        let response = engine.execute(query).unwrap();
                        let QueryResponse::Compact(compact) = &response else {
                            panic!("expected compact continuation")
                        };
                        next = compact.page.next_cursor.clone();
                        more.push(response);
                    }
                    if !more.is_empty() {
                        callers.insert(item.callable.occurrence.address_id.to_string(), more);
                    }
                }
            }
            request.output.after = page.page.next_cursor.clone();
            pages.push(page);
            if request.output.after.is_none() {
                break;
            }
        }
        assert_eq!(pages.len(), 3);
        if let Some(output) = output {
            let packet = serde_json::json!({"case":case,"base":base,"head":head,"impact":impact,
                "pages":pages,"caller_pages":callers});
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output.join(format!("delivery-{case}.json")))
                .unwrap()
                .write_all(&serde_json::to_vec(&packet).unwrap())
                .unwrap();
        }
    }
}

#[test]
#[ignore = "requires Python and evals/harness/requirements.txt"]
fn campaign_accounting_uses_production_occurrences_and_rejects_incomparable_receipts() {
    use serde_json::{json, Value};
    let directory = tempfile::tempdir().unwrap();
    let packets = directory.path().join("packets");
    revision_controls(Some(&packets));
    let load = |name: &str| -> Value {
        serde_json::from_slice(&std::fs::read(packets.join(format!("{name}.json"))).unwrap())
            .unwrap()
    };
    let mut jobs = Vec::new();
    let mut expected = Vec::new();
    let mut add = |mode: &str, packet: Value, expectation: Value| {
        jobs.push(json!({"mode": mode, "packet": packet}));
        expected.push(expectation);
    };
    for language in [
        "rust",
        "python",
        "javascript",
        "typescript",
        "tsx",
        "go",
        "java",
        "csharp",
        "c",
        "cpp",
    ] {
        for (case, planes, denominator) in [
            ("missing", json!({"not_comparable":2}), 0),
            ("empty", json!({"complete_empty":2}), 0),
            ("unchanged", json!({"structural_only":1,"neither":1}), 2),
            ("changed_focus", json!({"both":1,"neither":1}), 2),
            (
                "changed_sibling",
                json!({"structural_only":1,"resolved_only":1}),
                2,
            ),
            ("partial", json!({"not_comparable":2}), 0),
            ("series_drift", json!({"not_comparable":2}), 0),
            ("mixed_empty", json!({"complete_empty":1,"neither":1}), 1),
            ("secondary_change", json!({"both":1,"neither":1}), 2),
        ] {
            add(
                "account",
                load(&format!("{language}-{case}")),
                json!({
                    "/occurrences":2, "/structural":{"changed":1,"unchanged":1},
                    "/planes":planes, "/reference_comparable":denominator
                }),
            );
        }
        let original = load(&format!("{language}-unchanged"));
        for owner in ["base", "head", "impact"] {
            for component in ["missing", "contracts", "derivation"] {
                let mut packet = original.clone();
                if component == "missing" {
                    packet[owner]
                        .as_object_mut()
                        .unwrap()
                        .remove("implementation");
                } else {
                    packet[owner]["implementation"][component] =
                        format!("blake3:{}", "f".repeat(64)).into();
                }
                add("account", packet, Value::Null);
            }
        }
        for path in [
            "/impact/base_snapshot/revision",
            "/impact/head_snapshot/revision",
        ] {
            let mut packet = original.clone();
            *packet.pointer_mut(path).unwrap() = "f".repeat(40).into();
            add("account", packet, Value::Null);
        }
        let mut packet = original.clone();
        packet["impact"]["head_policy_ids"] = json!(["a".repeat(64)]);
        add("account", packet, Value::Null);
        for mutation in [
            "duplicate",
            "forged",
            "omitted",
            "namespace",
            "binding_missing",
            "first_record",
            "secondary",
        ] {
            let mut packet = load(&format!(
                "{language}-{}",
                if mutation == "secondary" {
                    "secondary_change"
                } else {
                    "changed_focus"
                }
            ));
            match mutation {
                "duplicate" => {
                    let nodes = packet["base"]["nodes"].as_array_mut().unwrap();
                    let node = nodes
                        .iter()
                        .find(|n| n["kind"] == "callable")
                        .unwrap()
                        .clone();
                    nodes.push(node);
                }
                "forged" => {
                    packet["impact"]["callables"][0]["occurrence"]["address_id"] =
                        "a".repeat(64).into()
                }
                "omitted" => packet["impact"]["callables"] = json!([]),
                "namespace" => {
                    packet["head"]["snapshot"]["repository_id"] = "other-repository".into();
                    packet["impact"]["head_snapshot"]["repository_id"] = "other-repository".into();
                }
                "binding_missing" => {
                    packet["impact"]["bindings"].as_array_mut().unwrap().pop();
                }
                "secondary" => {
                    packet["impact"]["bindings"][0]["base"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                _ => packet["impact"]["bindings"][0]["base"] = json!([]),
            }
            add("account", packet, Value::Null);
        }
        let mut packet = original;
        let path = packet["impact"]["callables"][0]["path"].clone();
        packet["head"]["diagnostics"]
            .as_array_mut()
            .unwrap()
            .push(json!({"code":"source.parse_failed","provider_pointer":path}));
        add(
            "account",
            packet,
            json!({"/structural":{"not_comparable":2},"/structural_comparable":0,"/reasons/head_source_inventory_incomplete":2}),
        );
        let mut packet = load(&format!("{language}-missing"));
        packet["pages"] = json!([]);
        add(
            "relay",
            packet.clone(),
            json!({"/query_not_run":true,"/unconsumed_subjects":1}),
        );
        packet["pages"] = json!([packet["relay"]]);
        add(
            "relay",
            packet,
            json!({"/counts/returned_callers":0,"/statuses":{"unknown":1},"/counts/subjects_with_returned_callers":0,"/unconsumed_subjects":0}),
        );
    }
    let original = load("delivery-paged");
    add(
        "relay",
        original.clone(),
        json!({"/eligible_subjects":3,"/returned_subjects":3,"/unconsumed_subjects":0,"/counts/returned_callers":2,"/counts/returned_witness_paths":2,"/counts/removed_subjects":1,"/counts/unconsumed_caller_items":0}),
    );
    let mut first = original.clone();
    first["pages"].as_array_mut().unwrap().truncate(1);
    first["caller_pages"] = json!({});
    add(
        "relay",
        first,
        json!({"/unconsumed_subjects":2,"/counts/returned_callers":1,"/counts/unconsumed_caller_items":1}),
    );
    add(
        "relay",
        load("delivery-truncated"),
        json!({"/counts/returned_callers":1,"/unconsumed_subjects":0}),
    );
    for mutation in [
        "duplicate",
        "out_of_order",
        "lost_cursor",
        "returned_count",
        "fingerprint",
        "snapshot",
        "foreign_subject",
        "foreign_callers",
        "meaning",
        "witness",
    ] {
        let mut packet = original.clone();
        match mutation {
            "duplicate" => {
                let last = packet["pages"].as_array().unwrap().last().unwrap().clone();
                packet["pages"].as_array_mut().unwrap().push(last);
            }
            "out_of_order" => packet["pages"].as_array_mut().unwrap().reverse(),
            "lost_cursor" => packet["pages"][0]["page"]["next_cursor"] = Value::Null,
            "returned_count" => packet["pages"][0]["page"]["returned"] = 0.into(),
            "fingerprint" => {
                packet["pages"][0]["implementation"]["derivation"] =
                    format!("blake3:{}", "f".repeat(64)).into()
            }
            "snapshot" => packet["pages"][0]["head_snapshot"]["revision"] = "f".repeat(40).into(),
            "foreign_subject" => {
                packet["pages"][0]["items"][0]["callable"]["occurrence"]["address_id"] =
                    "f".repeat(64).into()
            }
            "foreign_callers" => {
                packet["pages"][0]["items"][0]["callers"]["query"]["request"]["start"] =
                    "f".repeat(64).into()
            }
            "meaning" => {
                packet["caller_pages"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()[0]["search_truncated"] = true.into()
            }
            _ => {
                let first = packet["pages"][0]["items"][0]["callers"].clone();
                let continuation = &mut packet["caller_pages"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()[0];
                for key in ["items", "subjects", "edges"] {
                    continuation[key] = first[key].clone();
                }
            }
        }
        add("relay", packet, Value::Null);
    }

    let record = json!({"address_id":"a".repeat(64), "resolved_hash":format!("blake3:{}","b".repeat(64)),
        "resolution_policy_id":"c".repeat(64), "binding":{"completeness":"complete","reference_count":0,
        "source":{"artifact":{"artifact_id":"d".repeat(64),"content_digest":format!("blake3:{}","e".repeat(64))},
        "producer":{"producer_id":"fixture","version":"1"}}}});
    add("schema", record.clone(), json!({"":true}));
    for mutation in [
        "missing_digest",
        "unavailable_with_source",
        "complete_with_gaps",
        "legacy",
    ] {
        let mut invalid = record.clone();
        match mutation {
            "missing_digest" => {
                invalid["binding"]["source"]["artifact"]
                    .as_object_mut()
                    .unwrap()
                    .remove("content_digest");
            }
            "unavailable_with_source" => invalid["binding"]["completeness"] = "unavailable".into(),
            "complete_with_gaps" => invalid["binding"]["unresolved_count"] = 1.into(),
            _ => {
                invalid.as_object_mut().unwrap().remove("binding");
                invalid["completeness"] = "complete".into();
            }
        }
        add("schema", invalid, Value::Null);
    }
    let request = directory.path().join("jobs.json");
    std::fs::write(
        &request,
        serde_json::to_vec(&json!({"expected":implementation_fingerprint(),"jobs":jobs})).unwrap(),
    )
    .unwrap();
    // Only the language bridge lives here; assertions and adversarial mutations
    // are Rust-owned alongside the production inventory fixture above.
    let output = std::process::Command::new(std::env::var("ENTMOOT_CAMPAIGN_PYTHON").unwrap_or_else(|_| "python".into()))
        .args(["-X", "utf8", "-c", r#"
import json,sys
from evals.harness.revision_accounting import occurrence_accounting,summarize_occurrences,summarize_relay
from evals.harness.contracts import ContractError
from evals.harness.wire_schema import validate_wire
request=json.load(open(sys.argv[1],encoding='utf-8')); results=[]
for job in request['jobs']:
    p=job['packet']
    try:
        if job['mode']=='account': result=summarize_occurrences(occurrence_accounting(p['base'],p['head'],p['impact'],expected=request['expected']))
        elif job['mode']=='schema': validate_wire(p,'ResolvedIdentityRecord'); result=True
        else: result=summarize_relay(p['impact'],p['pages'],expected=request['expected'],caller_pages=p.get('caller_pages',{}))
        results.append({'ok':result})
    except ContractError as error: results.append({'error':str(error)})
print(json.dumps(results))
"#]).arg(request).current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(results.len(), expected.len());
    for (index, (result, expect)) in results.iter().zip(expected).enumerate() {
        if expect.is_null() {
            assert!(
                result.get("error").is_some(),
                "mutation {index} accepted: {result}"
            );
        } else {
            assert!(result.get("ok").is_some(), "job {index}: {result}");
            for (path, value) in expect.as_object().unwrap() {
                assert_eq!(
                    result["ok"].pointer(path),
                    Some(value),
                    "job {index} {path}"
                );
            }
        }
    }

    let report = directory.path().join("report.md");
    let packet = load("python-empty");
    for name in ["base", "head", "impact", "relay"] {
        std::fs::write(
            directory.path().join(format!("{name}.json")),
            serde_json::to_vec(&packet[name]).unwrap(),
        )
        .unwrap();
    }
    let write_report = || {
        std::process::Command::new(std::env::var("ENTMOOT_CAMPAIGN_PYTHON").unwrap_or_else(|_| "python".into()))
            .args(["-X","utf8","-c",r#"
import json,sys
from pathlib import Path
from evals.harness.report_revision_pair import write_report
root=Path(sys.argv[1]); expected=json.loads(sys.argv[2])
write_report(root/'base.json',root/'head.json',root/'impact.json',[root/'relay.json'],root/'report.md',expected=expected)
"#]).arg(directory.path()).arg(serde_json::to_string(&implementation_fingerprint()).unwrap())
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).output().unwrap()
    };
    let output = write_report();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = std::fs::read_to_string(report).unwrap();
    for text in [
        "ungraded (zero comparable observations)",
        "Complete empty reference sets: 2",
        "Subjects with caller status unknown: 1",
    ] {
        assert!(report.contains(text), "{report}");
    }
    assert!(!write_report().status.success());
}
