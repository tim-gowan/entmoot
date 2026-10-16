//! Language-agnostic seeking refs and optional package-root boundary enrich.
//!
//! Producers stamp package-qualified external calls; this fill attaches
//! `package_api.references` as seeking (`unassessed`) or assessed (policy +
//! `boundary_text`) without inventing fit. Fit stays in the compatibility reader.

use super::join::resolve_package_purl;
use super::pipeline::NativeProducerError;
use entmoot_contracts::package_api::{PackageApiEcosystem, PackageApiPackage};
use entmoot_contracts::{NativeCallTarget, NativeRawObservation};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

/// Assessed contract text from a proven package root (not a fit claim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrichedBoundary {
    pub boundary_policy: String,
    pub boundary_text: String,
}

/// Resolves export contracts from a proven dependency root under consumer policy.
pub trait PackageRootResolver {
    fn enrich(
        &self,
        source_root: &Path,
        package: &PackageApiPackage,
        export_spelling: &str,
        call_path: &str,
    ) -> Result<Option<EnrichedBoundary>, NativeProducerError>;
}

/// Symbol scheme for seeking refs when the call target does not already name one.
pub fn default_symbol_scheme(language: &str) -> &'static str {
    match language {
        "javascript" | "typescript" | "tsx" => "javascript.compiler.symbol",
        "python" => "python.compiler.symbol",
        "go" => "go.compiler.symbol",
        "rust" => "rust.compiler.symbol",
        "java" => "java.compiler.symbol",
        "csharp" => "csharp.compiler.symbol",
        "c" | "cpp" | "cxx" => "cxx.compiler.symbol",
        _ => "entmoot.compiler.symbol",
    }
}

fn ecosystem_wire(ecosystem: PackageApiEcosystem) -> &'static str {
    match ecosystem {
        PackageApiEcosystem::Rust => "rust",
        PackageApiEcosystem::Javascript => "javascript",
        PackageApiEcosystem::Go => "go",
        PackageApiEcosystem::Python => "python",
        PackageApiEcosystem::Java => "java",
        PackageApiEcosystem::Csharp => "csharp",
        PackageApiEcosystem::Native => "native",
    }
}

fn package_from_target(
    language: &str,
    target: &NativeCallTarget,
) -> Result<Option<PackageApiPackage>, NativeProducerError> {
    if target.package_role.as_deref() == Some("stdlib") {
        return Ok(None);
    }
    let Some(purl) = resolve_package_purl(language, target) else {
        return Ok(None);
    };
    PackageApiPackage::from_purl(&purl)
        .map_err(|error| NativeProducerError::Artifact(error.to_string()))
}

fn ensure_package_api(raw: &mut NativeRawObservation) -> &mut Value {
    raw.build_context
        .entry("package_api".to_string())
        .or_insert_with(|| {
            json!({
                "surfaces": [],
                "references": [],
                "references_complete": false,
                "limitations": []
            })
        })
}

fn reference_spans(api: &Value) -> BTreeSet<(String, u64, u64)> {
    let mut spans = BTreeSet::new();
    let Some(references) = api.get("references").and_then(Value::as_array) else {
        return spans;
    };
    for reference in references {
        let Some(path) = reference.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(start) = reference.get("start_byte").and_then(Value::as_u64) else {
            continue;
        };
        let Some(end) = reference.get("end_byte").and_then(Value::as_u64) else {
            continue;
        };
        spans.insert((path.to_owned(), start, end));
    }
    spans
}

/// Fill missing `package_api.references` for package-qualified external calls.
pub fn fill_references(
    raw: &mut NativeRawObservation,
    language: &str,
    source_root: &Path,
    resolvers: &[&dyn PackageRootResolver],
) -> Result<(), NativeProducerError> {
    if raw.invalid_environment || raw.budget_exceeded {
        return Ok(());
    }
    let scheme_default = default_symbol_scheme(language);
    let mut pending: Vec<Value> = Vec::new();
    {
        let api = ensure_package_api(raw);
        let covered = reference_spans(api);
        let mut consider = |site_path: &str,
                            start_byte: u64,
                            end_byte: u64,
                            target: &NativeCallTarget|
         -> Result<(), NativeProducerError> {
            if target.location.repository().is_some() {
                return Ok(());
            }
            let key = (site_path.to_owned(), start_byte, end_byte);
            if covered.contains(&key) {
                return Ok(());
            }
            let Some(package) = package_from_target(language, target)? else {
                return Ok(());
            };
            let spelling = if target.symbol.trim().is_empty() {
                return Ok(());
            } else {
                target.symbol.clone()
            };
            let mut enriched = None;
            for resolver in resolvers {
                if let Some(boundary) =
                    resolver.enrich(source_root, &package, &spelling, site_path)?
                {
                    enriched = Some(boundary);
                    break;
                }
            }
            let mut reference = json!({
                "path": site_path,
                "start_byte": start_byte,
                "end_byte": end_byte,
                "target": {
                    "package": {
                        "ecosystem": ecosystem_wire(package.ecosystem),
                        "name": package.name
                    },
                    "symbol": {
                        "scheme": scheme_default,
                        "spelling": spelling
                    }
                }
            });
            if let Some(boundary) = enriched {
                reference["boundary_policy"] = json!(boundary.boundary_policy);
                reference["boundary_text"] = json!(boundary.boundary_text);
            } else {
                reference["boundary_policy"] = json!("unassessed");
            }
            pending.push(reference);
            Ok(())
        };
        for call in &raw.calls {
            if call.gap.is_some() {
                continue;
            }
            let [target] = call.targets.as_slice() else {
                continue;
            };
            consider(
                &call.site.path,
                call.site.span.start_byte as u64,
                call.site.span.end_byte as u64,
                target,
            )?;
        }
        // Python (and other) producers may stamp import/use sites as NativeReferenceFact
        // with package coordinates before call resolution; admit those as seeking refs.
        for reference in &raw.references {
            if reference.gap.is_some() {
                continue;
            }
            let [target] = reference.targets.as_slice() else {
                continue;
            };
            consider(
                &reference.site.path,
                reference.site.span.start_byte as u64,
                reference.site.span.end_byte as u64,
                target,
            )?;
        }
    }
    if pending.is_empty() {
        return Ok(());
    }
    let api = ensure_package_api(raw);
    api["references_complete"] = json!(false);
    let references = api
        .as_object_mut()
        .ok_or_else(|| NativeProducerError::Artifact("package_api must be an object".into()))?
        .entry("references")
        .or_insert_with(|| json!([]));
    let list = references.as_array_mut().ok_or_else(|| {
        NativeProducerError::Artifact("package_api.references must be an array".into())
    })?;
    list.extend(pending);
    Ok(())
}

/// Look up exports from dependency surfaces already attached to the raw bag.
pub struct AttachedDependencySurfacesResolver<'a> {
    pub surfaces: &'a [Value],
}

impl PackageRootResolver for AttachedDependencySurfacesResolver<'_> {
    fn enrich(
        &self,
        _source_root: &Path,
        package: &PackageApiPackage,
        export_spelling: &str,
        _call_path: &str,
    ) -> Result<Option<EnrichedBoundary>, NativeProducerError> {
        for surface in self.surfaces {
            let Some(row) = surface.get("package") else {
                continue;
            };
            let Some(name) = row.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(ecosystem) = row.get("ecosystem").and_then(Value::as_str) else {
                continue;
            };
            if name != package.name || ecosystem != ecosystem_wire(package.ecosystem) {
                continue;
            }
            let Some(exports) = surface.get("exports").and_then(Value::as_array) else {
                continue;
            };
            for export in exports {
                let spelling = export
                    .pointer("/target/symbol/spelling")
                    .and_then(Value::as_str)
                    .or_else(|| export.get("symbol").and_then(Value::as_str));
                if spelling != Some(export_spelling) {
                    continue;
                }
                let Some(policy) = export.get("boundary_policy").and_then(Value::as_str) else {
                    continue;
                };
                let Some(text) = export.get("boundary_text").and_then(Value::as_str) else {
                    continue;
                };
                if policy == "unassessed" || text.is_empty() {
                    continue;
                }
                return Ok(Some(EnrichedBoundary {
                    boundary_policy: policy.to_owned(),
                    boundary_text: text.to_owned(),
                }));
            }
        }
        Ok(None)
    }
}

/// Collect resolvers that only need the current package_api surfaces value.
pub fn surfaces_from_raw(raw: &NativeRawObservation) -> Vec<Value> {
    raw.build_context
        .get("package_api")
        .and_then(|api| api.get("surfaces"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use entmoot_contracts::NativeCallFact;

    struct StubResolver {
        hit: Option<EnrichedBoundary>,
    }

    impl PackageRootResolver for StubResolver {
        fn enrich(
            &self,
            _source_root: &Path,
            _package: &PackageApiPackage,
            _export_spelling: &str,
            _call_path: &str,
        ) -> Result<Option<EnrichedBoundary>, NativeProducerError> {
            Ok(self.hit.clone())
        }
    }

    fn external_call(language_pkg: &str, spelling: &str) -> NativeRawObservation {
        NativeRawObservation {
            calls: vec![NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: "src/main.rs".into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: 10,
                        end_byte: 20,
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

                    symbol: spelling.into(),
                    kind: "callable".into(),
                    package_role: Some("module".into()),
                    package_purl: Some(format!("pkg:cargo/{language_pkg}@1.0.0")),
                    ecosystem: Some("rust".into()),
                    module_path: None,
                    package_version: None,
                    path_segments: Vec::new(),
                }],
                owner: None,
                gap: None,
                limitation: None,
                intrinsic: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn seeks_when_no_package_root_resolves() {
        let mut raw = external_call("serde", "serde::Serialize");
        let root = std::env::temp_dir();
        fill_references(&mut raw, "rust", &root, &[]).unwrap();
        let refs = raw.build_context["package_api"]["references"]
            .as_array()
            .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0]["boundary_policy"], "unassessed");
        assert_eq!(refs[0]["target"]["package"]["name"], "serde");
        assert_eq!(refs[0]["target"]["symbol"]["spelling"], "serde::Serialize");
    }

    #[test]
    fn enriches_when_resolver_returns_boundary_text() {
        let mut raw = external_call("serde", "serde::Serialize");
        let root = std::env::temp_dir();
        let stub = StubResolver {
            hit: Some(EnrichedBoundary {
                boundary_policy: "rust.fn_header.v1".into(),
                boundary_text: "pub fn serialize".into(),
            }),
        };
        fill_references(&mut raw, "rust", &root, &[&stub]).unwrap();
        let reference = &raw.build_context["package_api"]["references"][0];
        assert_eq!(reference["boundary_policy"], "rust.fn_header.v1");
        assert_eq!(reference["boundary_text"], "pub fn serialize");
    }

    #[test]
    fn does_not_double_emit_existing_reference_spans() {
        let mut raw = external_call("serde", "serde::Serialize");
        raw.build_context.insert(
            "package_api".into(),
            json!({
                "surfaces": [],
                "references": [{
                    "path": "src/main.rs",
                    "start_byte": 10,
                    "end_byte": 20,
                    "target": {"package": {"ecosystem": "rust", "name": "serde"},
                        "symbol": {"scheme": "rust.compiler.symbol", "spelling": "serde::Serialize"}},
                    "boundary_policy": "unassessed"
                }],
                "references_complete": false,
                "limitations": []
            }),
        );
        let root = std::env::temp_dir();
        fill_references(&mut raw, "rust", &root, &[]).unwrap();
        assert_eq!(
            raw.build_context["package_api"]["references"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn skips_ambiguous_multi_target_calls() {
        let mut raw = external_call("serde", "serde::Serialize");
        let duplicate = raw.calls[0].targets[0].clone();
        raw.calls[0].targets.push(duplicate);
        let root = std::env::temp_dir();
        fill_references(&mut raw, "rust", &root, &[]).unwrap();
        let refs = raw
            .build_context
            .get("package_api")
            .and_then(|api| api.get("references"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(refs.is_empty());
    }

    #[test]
    fn seeks_python_and_go_shaped_module_path_calls() {
        let root = std::env::temp_dir();
        for (language, module, spelling, name) in [
            ("python", "requests", "requests.get", "requests"),
            (
                "go",
                "github.com/example/mod",
                "github.com/example/mod.Do",
                "github.com/example/mod",
            ),
        ] {
            let mut raw = NativeRawObservation {
                calls: vec![NativeCallFact {
                    dispatch: Default::default(),
                    site: entmoot_contracts::NativeSpanRef {
                        path: "main.src".into(),
                        span: entmoot_contracts::SiteSpan {
                            start_byte: 1,
                            end_byte: 2,
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

                        symbol: spelling.into(),
                        kind: "callable".into(),
                        package_role: Some("module".into()),
                        package_purl: None,
                        ecosystem: Some(language.into()),
                        module_path: Some(module.into()),
                        package_version: None,
                        path_segments: Vec::new(),
                    }],
                    owner: None,
                    gap: None,
                    limitation: None,
                    intrinsic: false,
                }],
                ..Default::default()
            };
            fill_references(&mut raw, language, &root, &[]).unwrap();
            let reference = &raw.build_context["package_api"]["references"][0];
            assert_eq!(reference["boundary_policy"], "unassessed");
            assert_eq!(reference["target"]["package"]["name"], name);
            assert_eq!(reference["target"]["symbol"]["spelling"], spelling);
        }
    }
}
