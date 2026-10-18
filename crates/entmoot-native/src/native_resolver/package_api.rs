//! Convert explicit native package declarations to inventory addresses. No name joins.
use super::pipeline::NativeProducerError;
use crate::{NodeKind, TopologyGraph};
use entmoot_contracts::NativeRawObservation;
use entmoot_contracts::{
    package_api::*, topology_node_address, ProviderSymbolSpelling, ResolverObservation,
    ScopeCompleteness,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeApi {
    surfaces: Vec<Surface>,
    references: Vec<Reference>,
    references_complete: bool,
    limitations: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Surface {
    package: PackageApiPackage,
    export_universes: Vec<PackageApiExportUniverse>,
    completeness: ScopeCompleteness,
    exports: Vec<Export>,
    limitations: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Export {
    path: String,
    start_byte: usize,
    end_byte: usize,
    symbol: String,
    boundary_policy: String,
    boundary_digest: Option<String>,
    /// Exact compiler-produced UTF-8 contract text, hashed by the adapter as
    /// BLAKE3(policy + newline + text). Original text remains in native capture.
    boundary_text: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    #[serde(default)]
    lock_instance: Option<LockedPackageInstance>,
    path: String,
    start_byte: usize,
    end_byte: usize,
    target: PackageApiTarget,
    boundary_policy: String,
    expected_boundary: Option<String>,
    boundary_text: Option<String>,
}

fn boundary(
    policy: &str,
    digest: Option<String>,
    text: Option<String>,
) -> Result<Option<String>, NativeProducerError> {
    let computed =
        text.map(|text| entmoot_contracts::blake3_hex(format!("{policy}\n{text}").as_bytes()));
    if let (Some(stated), Some(computed)) = (&digest, &computed) {
        if stated != computed {
            return Err(NativeProducerError::Artifact(
                "native boundary text/digest mismatch".into(),
            ));
        }
    }
    Ok(digest.or(computed))
}

/// Compose isolated module captures without losing package evidence or rebasing
/// provider symbol identity. Only repository-relative source spans are rebased.
pub(super) fn merge_module_api(
    raw: &mut NativeRawObservation,
    modules: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), NativeProducerError> {
    let mut merged = NativeApi {
        surfaces: Vec::new(),
        references: Vec::new(),
        references_complete: false,
        limitations: vec!["isolated_module_package_scope_partial".into()],
    };
    for (prefix, context) in modules {
        let Some(value) = context.get("package_api") else {
            continue;
        };
        if prefix.contains(['\\', ':'])
            || (!prefix.is_empty()
                && prefix
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == ".."))
        {
            return Err(NativeProducerError::Artifact(
                "invalid module package prefix".into(),
            ));
        }
        let mut api: NativeApi = serde_json::from_value(value.clone())?;
        let rebase = |path: &mut String| -> Result<(), NativeProducerError> {
            if path.is_empty()
                || path.contains(['\\', ':'])
                || path
                    .split('/')
                    .any(|p| p.is_empty() || p == ".." || p == ".")
            {
                return Err(NativeProducerError::Artifact(
                    "invalid module package span path".into(),
                ));
            }
            if !prefix.is_empty() {
                *path = format!("{prefix}/{path}");
            }
            Ok(())
        };
        for surface in &mut api.surfaces {
            for export in &mut surface.exports {
                rebase(&mut export.path)?;
            }
        }
        for reference in &mut api.references {
            rebase(&mut reference.path)?;
        }
        merged.surfaces.extend(api.surfaces);
        merged.references.extend(api.references);
        merged.limitations.extend(api.limitations);
    }
    if !merged.surfaces.is_empty() || !merged.references.is_empty() {
        raw.build_context
            .insert("package_api".into(), serde_json::to_value(merged)?);
    }
    Ok(())
}

pub(super) fn normalize(
    graph: &TopologyGraph,
    raw: &NativeRawObservation,
    language: &str,
    observations: &[ResolverObservation],
) -> Result<Option<PackageApiEvidence>, NativeProducerError> {
    let Some(value) = raw.build_context.get("package_api") else {
        return Ok(None);
    };
    let native: NativeApi = serde_json::from_value(value.clone())?;
    let mut api = PackageApiEvidence {
        surfaces: Vec::new(),
        references: Vec::new(),
        references_complete: native.references_complete
            && !raw.invalid_environment
            && !raw.budget_exceeded,
        limitations: native
            .limitations
            .into_iter()
            .map(|detail| {
                if let Some(name) = detail.strip_prefix("mapped_package_root_missing:") {
                    CompatibilityLimitation::coded(
                        CompatibilityLimitationCode::MappedPackageRootMissing,
                    )
                    .with_detail(name.to_owned())
                } else {
                    CompatibilityLimitation::producer_stated(detail)
                }
            })
            .collect(),
    };
    let mut declarations = std::collections::BTreeMap::<_, Vec<_>>::new();
    let mut sites = std::collections::BTreeMap::new();
    for node in &graph.nodes {
        if let (Some(path), Some(start), Some(end)) = (&node.path, node.start_byte, node.end_byte) {
            let key = (path.clone(), start, end);
            if matches!(node.kind, NodeKind::Callable | NodeKind::Declaration) {
                declarations
                    .entry((path.clone(), end))
                    .or_default()
                    .push(node);
            } else if node.kind == NodeKind::CallStub {
                sites
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .push(node.id.as_str());
            }
        }
    }
    let by_id: std::collections::BTreeMap<_, _> = observations
        .iter()
        .map(|row| (row.occurrence_id.as_str(), row))
        .collect();
    for surface in native.surfaces {
        let mut output = PackageApiSurface {
            package: surface.package.clone(),
            export_universes: surface.export_universes,
            completeness: surface.completeness,
            exports: Vec::new(),
            limitations: surface
                .limitations
                .into_iter()
                .map(CompatibilityLimitation::producer_stated)
                .collect(),
        };
        if raw.invalid_environment || raw.budget_exceeded {
            output.completeness = ScopeCompleteness::Partial;
        }
        if raw.invalid_environment {
            output
                .limitations
                .push(CompatibilityLimitation::producer_stated(
                    "invalid_environment_package_boundaries_unassessed",
                ));
            api.surfaces.push(output);
            continue;
        }
        for export in surface.exports {
            let boundary_digest = boundary(
                &export.boundary_policy,
                export.boundary_digest,
                export.boundary_text,
            )?
            .ok_or_else(|| {
                NativeProducerError::Artifact("native export boundary missing".into())
            })?;
            let candidates = declarations.get(&(export.path, export.end_byte));
            let Some(node) = candidates.and_then(|nodes| {
                super::join::matching_declaration(
                    nodes,
                    export.start_byte,
                    export.end_byte,
                    language,
                )
            }) else {
                output.completeness = ScopeCompleteness::Partial;
                output
                    .limitations
                    .push(CompatibilityLimitation::producer_stated(format!(
                        "export_inventory_span_unmatched:{}",
                        export.symbol
                    )));
                continue;
            };
            output.exports.push(PackageApiExport {
                target: PackageApiTarget {
                    package: surface.package.clone(),
                    symbol: ProviderSymbolSpelling {
                        scheme: format!("{language}.compiler.symbol"),
                        spelling: export.symbol,
                    },
                },
                declaration: topology_node_address(&node.id),
                boundary_policy: export.boundary_policy,
                boundary_digest,
            });
        }
        api.surfaces.push(output);
    }
    for reference in native.references {
        if raw.invalid_environment {
            continue;
        }
        // Verify the lossless native coordinate policy before admitting a join.
        // Mixed-case namespaces use the validated go_module_path qualifier.
        if reference.target.package.ecosystem == PackageApiEcosystem::Go {
            let purl = entmoot_contracts::purl_golang_module(&reference.target.package.name, None)
                .map_err(|error| NativeProducerError::Artifact(error.to_string()))?;
            let projected = PackageApiPackage::from_purl(&purl)
                .map_err(|error| NativeProducerError::Artifact(error.to_string()))?;
            if projected.as_ref() != Some(&reference.target.package) {
                api.references_complete = false;
                api.limitations
                    .push(CompatibilityLimitation::producer_stated(
                        "go_module_coordinate_not_lossless_in_purl",
                    ));
                continue;
            }
        }
        let expected_boundary = boundary(
            &reference.boundary_policy,
            reference.expected_boundary,
            reference.boundary_text,
        )?;
        let observation = sites
            .get(&(reference.path, reference.start_byte, reference.end_byte))
            .and_then(|ids| match ids.as_slice() {
                [id] => by_id.get(id),
                _ => None,
            });
        let Some(observation) = observation else {
            api.references_complete = false;
            api.limitations
                .push(CompatibilityLimitation::producer_stated(
                    "dependency_site_inventory_span_unmatched",
                ));
            continue;
        };
        let matching: Vec<_> = observation
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.provider_symbol.as_ref() == Some(&reference.target.symbol))
            .collect();
        if let [(index, _)] = matching.as_slice() {
            api.references.push(PackageApiReference {
                lock_instance: reference.lock_instance,
                observation_id: observation.occurrence_id.clone(),
                target_index: *index,
                target: reference.target,
                boundary_policy: reference.boundary_policy,
                expected_boundary,
            });
        } else {
            api.references_complete = false;
            api.limitations
                .push(CompatibilityLimitation::producer_stated(
                    "dependency_binding_not_unique",
                ));
        }
    }
    api.limitations.sort();
    api.references_complete &= api.references.iter().all(|r| r.lock_instance.is_some());
    api.limitations.dedup();
    Ok(Some(api))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_environments_preserve_raw_facts_without_usable_boundaries_in_every_family() {
        for (file, source, language, ecosystem) in [
            ("api.rs", "pub fn leaf() {}", "rust", "rust"),
            ("api.py", "def leaf():\n    pass\n", "python", "python"),
            (
                "api.js",
                "export function leaf() {}",
                "javascript",
                "javascript",
            ),
            (
                "api.ts",
                "export function leaf(): void {}",
                "typescript",
                "javascript",
            ),
            (
                "api.tsx",
                "export function leaf(): void {}",
                "tsx",
                "javascript",
            ),
            ("api.go", "package api\nfunc Leaf() {}\n", "go", "go"),
            (
                "Api.java",
                "public class Api { public static void leaf() {} }",
                "java",
                "java",
            ),
            (
                "Api.cs",
                "public class Api { public static void Leaf() {} }",
                "csharp",
                "csharp",
            ),
            ("api.c", "void leaf(void) {}", "c", "native"),
            ("api.cpp", "void leaf() {}", "cpp", "native"),
        ] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join(file), source).unwrap();
            let graph =
                crate::build_topology_from_repo_with_options(root.path(), Default::default())
                    .unwrap();
            let node = graph
                .nodes
                .iter()
                .find(|node| node.kind == NodeKind::Callable)
                .unwrap();
            let package = if ecosystem == "native" {
                serde_json::json!({"ecosystem": ecosystem, "name": "pkg:generic/fixture"})
            } else {
                serde_json::json!({"ecosystem": ecosystem, "name": "fixture"})
            };
            let mut raw = NativeRawObservation::default();
            raw.build_context.insert("package_api".into(), serde_json::json!({
                "surfaces": [{"package": package,
                    "export_universes": [{"symbol_scheme": format!("{language}.compiler.symbol"), "boundary_policy": "fixture.header.v1"}],
                    "completeness": "complete_relative_to_universe", "limitations": [],
                    "exports": [{"path": file, "start_byte": node.start_byte, "end_byte": node.end_byte,
                        "symbol": "leaf", "boundary_policy": "fixture.header.v1", "boundary_text": "header"}]}],
                "references": [], "references_complete": true, "limitations": []
            }));
            let valid = normalize(&graph, &raw, language, &[]).unwrap().unwrap();
            assert_eq!(valid.surfaces[0].exports.len(), 1, "{language}");
            let retained = raw.build_context.clone();
            raw.invalid_environment = true;
            let failed = normalize(&graph, &raw, language, &[]).unwrap().unwrap();
            assert!(failed.surfaces[0].exports.is_empty(), "{language}");
            assert_eq!(
                failed.surfaces[0].completeness,
                ScopeCompleteness::Partial,
                "{language}"
            );
            assert!(!failed.references_complete);
            assert_eq!(raw.build_context, retained);
        }
    }

    #[test]
    fn go_case_sensitive_module_boundary_preserves_native_coordinate() {
        use crate::native_resolver::{normalize_observations, NativeCallFact, NativeCallTarget};
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("main.go"),
            "package main\nfunc run() { dep.Leaf() }\n",
        )
        .unwrap();
        let graph =
            crate::build_topology_from_repo_with_options(root.path(), Default::default()).unwrap();
        let site = graph
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::CallStub)
            .unwrap();
        let module = "github.com/XSAM/otelsql";
        let symbol = format!("{module}.Leaf");
        let mut raw = NativeRawObservation::default();
        raw.calls.push(NativeCallFact {
            dispatch: Default::default(),
            site: entmoot_contracts::NativeSpanRef {
                path: site.path.clone().unwrap(),
                span: entmoot_contracts::SiteSpan {
                    start_byte: site.start_byte.unwrap(),
                    end_byte: site.end_byte.unwrap(),
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

                symbol: symbol.clone(),
                kind: "callable".into(),
                package_role: Some("module".into()),
                package_purl: None,
                ecosystem: Some("go".into()),
                module_path: Some(module.into()),
                package_version: Some("v0.40.0".into()),
                path_segments: vec![],
            }],
            owner: None,
            gap: None,
            intrinsic: false,
            limitation: None,
        });
        raw.build_context.insert("package_api".into(), serde_json::json!({
            "surfaces": [], "references_complete": true, "limitations": [],
            "references": [{"path":site.path,"start_byte":site.start_byte,"end_byte":site.end_byte,
                "target":{"package":{"ecosystem":"go","name":module},
                    "symbol":{"scheme":"go.compiler.symbol","spelling":symbol}},
                "boundary_policy":"go.types.free_function_signature.v1", "boundary_text":"func()"}]
        }));
        let observations = normalize_observations(&graph, &raw, "go");
        assert_eq!(
            observations[0].targets[0]
                .provider_symbol
                .as_ref()
                .unwrap()
                .spelling,
            symbol
        );
        let normalized = normalize(&graph, &raw, "go", &observations)
            .unwrap()
            .unwrap();
        assert_eq!(normalized.references.len(), 1);
        assert!(
            !normalized.references_complete,
            "coordinate spelling does not prove a selected lock instance"
        );
        assert!(normalized.limitations.is_empty());
        assert_eq!(normalized.references[0].target.package.name, module);
        assert_eq!(raw.calls[0].targets[0].module_path.as_deref(), Some(module));
        let mut ambiguous = graph.clone();
        let mut duplicate = ambiguous
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::CallStub)
            .unwrap()
            .clone();
        duplicate.id.push_str("-duplicate");
        ambiguous.nodes.push(duplicate);
        let normalized = normalize(&ambiguous, &raw, "go", &observations)
            .unwrap()
            .unwrap();
        assert!(
            normalized.references.is_empty(),
            "coincident inventory calls cannot choose the last span match"
        );
        assert!(!normalized.references_complete);
    }

    #[test]
    fn native_boundary_text_is_policy_scoped_and_cannot_disagree_with_digest() {
        let digest = boundary("header.v1", None, Some("fn f()".into()))
            .unwrap()
            .unwrap();
        assert_eq!(
            Some(digest.clone()),
            boundary("header.v1", Some(digest.clone()), Some("fn f()".into())).unwrap()
        );
        assert!(boundary("header.v2", Some(digest), Some("fn f()".into())).is_err());
        assert_eq!(None, boundary("header.v1", None, None).unwrap());
    }

    #[test]
    fn module_composition_rebases_only_source_spans_and_keeps_scope_partial() {
        let context = serde_json::json!({"package_api": {"surfaces": [], "references_complete": true, "limitations": [],
            "references": [{"path": "src/main.go", "start_byte": 1, "end_byte": 2,
                "target": {"package": {"ecosystem": "go", "name": "example.test/api"},
                    "symbol": {"scheme": "go.compiler.symbol", "spelling": "example.test/api.Leaf"}},
                "boundary_policy": "test.v1", "boundary_text": "func(int) int"}]}});
        let mut modules = serde_json::Map::new();
        modules.insert("module".into(), context.clone());
        let mut raw = NativeRawObservation::default();
        merge_module_api(&mut raw, &modules).unwrap();
        let api = &raw.build_context["package_api"];
        assert_eq!(api["references"][0]["path"], "module/src/main.go");
        assert_eq!(
            api["references"][0]["target"]["symbol"]["spelling"],
            "example.test/api.Leaf"
        );
        assert_eq!(api["references_complete"], false);
        modules.insert("../escape".into(), context);
        assert!(merge_module_api(&mut raw, &modules).is_err());
    }

    #[test]
    fn unjoined_or_failed_exports_cannot_close_an_absence() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("api.py"), "def leaf(x):\n    return x\n").unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            root.path(),
            crate::TopologyBuildOptions {
                repository: Some("fixture/python-api-join"),
                ..Default::default()
            },
        )
        .unwrap();
        let node = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Callable)
            .unwrap();
        let mut raw = NativeRawObservation::default();
        raw.build_context.insert("package_api".into(), serde_json::json!({
            "surfaces": [{"package": {"ecosystem": "python", "name": "fixture"},
                "export_universes": [{"symbol_scheme": "python.compiler.symbol", "boundary_policy": "test.header.v1"}],
                "completeness": "complete_relative_to_universe", "limitations": [],
                "exports": [{"path": "api.py", "start_byte": node.start_byte, "end_byte": node.end_byte,
                    "symbol": "api.leaf", "boundary_policy": "test.header.v1", "boundary_digest": entmoot_contracts::blake3_hex(b"header")}]}],
            "references": [], "references_complete": true, "limitations": []
        }));
        let complete = normalize(&graph, &raw, "python", &[]).unwrap().unwrap();
        assert_eq!(
            complete.surfaces[0].completeness,
            ScopeCompleteness::CompleteRelativeToUniverse
        );
        raw.invalid_environment = true;
        let failed = normalize(&graph, &raw, "python", &[]).unwrap().unwrap();
        assert_eq!(failed.surfaces[0].completeness, ScopeCompleteness::Partial);
        assert!(!failed.references_complete);
        raw.invalid_environment = false;
        raw.build_context.get_mut("package_api").unwrap()["surfaces"][0]["exports"][0]
            ["start_byte"] = (node.start_byte.unwrap() + 1).into();
        let mismatch = normalize(&graph, &raw, "python", &[]).unwrap().unwrap();
        assert_eq!(
            mismatch.surfaces[0].completeness,
            ScopeCompleteness::Partial
        );
        assert!(mismatch.surfaces[0].exports.is_empty());
    }

    #[test]
    fn exact_class_export_wins_over_its_same_end_constructor() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("api.py"),
            "class Client:\n    def __init__(self, x: int):\n        pass\n",
        )
        .unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            root.path(),
            crate::TopologyBuildOptions {
                repository: Some("fixture/class-export"),
                ..Default::default()
            },
        )
        .unwrap();
        let class = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Declaration)
            .unwrap();
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Callable && node.end_byte == class.end_byte));
        let mut raw = NativeRawObservation::default();
        raw.build_context.insert("package_api".into(), serde_json::json!({
            "surfaces": [{"package": {"ecosystem":"python","name":"fixture"},
                "export_universes":[{"symbol_scheme":"python.compiler.symbol","boundary_policy":"class.v1"}],
                "completeness":"complete_relative_to_universe","limitations":[],
                "exports":[{"path":"api.py","start_byte":class.start_byte,"end_byte":class.end_byte,
                    "symbol":"api.Client","boundary_policy":"class.v1","boundary_text":"class Client / __init__(int)"}]}],
            "references":[], "references_complete":true, "limitations":[]
        }));
        for reverse in [false, true] {
            let mut ordered = graph.clone();
            if reverse {
                ordered.nodes.reverse();
            }
            let normalized = normalize(&ordered, &raw, "python", &[]).unwrap().unwrap();
            assert_eq!(normalized.surfaces[0].exports.len(), 1);
            assert_eq!(
                normalized.surfaces[0].exports[0].declaration,
                topology_node_address(&class.id)
            );
            assert_eq!(
                normalized.surfaces[0].completeness,
                ScopeCompleteness::CompleteRelativeToUniverse
            );
        }
        raw.build_context.get_mut("package_api").unwrap()["surfaces"][0]["exports"][0]
            ["start_byte"] = (class.start_byte.unwrap() + 1).into();
        let mismatch = normalize(&graph, &raw, "python", &[]).unwrap().unwrap();
        assert!(mismatch.surfaces[0].exports.is_empty());
        assert_eq!(
            mismatch.surfaces[0].completeness,
            ScopeCompleteness::Partial
        );
    }
}
