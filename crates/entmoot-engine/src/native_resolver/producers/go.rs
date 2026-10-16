//! Go (`go/packages` + `go/types`) sidecar producer.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use super::sidecar::collect_via_sidecar_with_env;
use super::ProducerCapabilityProfile;
use entmoot_contracts::{NativeRawObservation, NativeSidecarRun};

/// Host policy isolates each Go module's token position universe.
pub const ADAPTER_PIN: &str = "go_types_sidecar.isolated_modules.v1";

pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "go",
    producer_id: "go_types_sidecar",
    semantic_surface: "uses_defs_selections_method_sets_imports",
    supports_types: true,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "go_sidecar_process",
    limitations: &[
        "dispatch_closure_for_unique_direct_free_functions_and_concrete_methods",
        "one_build_configuration_per_run",
        "callgraph_algorithms_are_labeled_projections",
        "reflection_unsafe_cgo_boundaries",
        "no_c_abi_into_rust",
    ],
};

const ENV_VAR: &str = "ENTMOOT_GO_ADAPTER";

const CANDIDATES: &[&str] = &["go/entmoot-go-resolver.exe", "go/entmoot-go-resolver"];

/// Discover module roots without following links outside the declared source tree.
fn module_roots(root: &Path, started: Instant, wall: Duration) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_owned()];
    let mut modules = Vec::new();
    while let Some(directory) = pending.pop() {
        if started.elapsed() >= wall {
            return Err("budget_exceeded: Go module discovery".into());
        }
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("invalid_environment: cannot read Go module tree: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("invalid_environment: {error}"))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entmoot_core::scan::should_skip_dir(&name)
                || matches!(
                    name.as_ref(),
                    "vendor" | "testdata" | "_examples" | "examples" | "example"
                )
            {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| format!("invalid_environment: {error}"))?;
            if kind.is_symlink() {
                // The sidecar follows files during compilation. Refuse an unbounded
                // discovery universe instead of silently missing a linked module.
                return Err("invalid_environment: linked Go source tree requires an isolated materialization".into());
            }
            if kind.is_dir() {
                if !matches!(
                    entry.file_name().to_str(),
                    Some(
                        ".git"
                            | "vendor"
                            | "node_modules"
                            | "testdata"
                            | "_examples"
                            | "examples"
                            | "example"
                    )
                ) {
                    pending.push(entry.path());
                }
            } else if entry.file_name() == "go.mod" {
                modules.push(directory.clone());
            }
        }
    }
    modules.sort();
    if modules.is_empty() {
        modules.push(root.to_owned());
    }
    Ok(modules)
}

fn prefixed(prefix: &str, path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.contains(':')
        || Path::new(&normalized)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(if prefix.is_empty() {
        normalized
    } else {
        format!("{prefix}/{normalized}")
    })
}

fn rebase_calls(raw: &mut NativeRawObservation, prefix: &str) -> Result<(), String> {
    for call in &mut raw.calls {
        call.site.path = prefixed(prefix, &call.site.path).ok_or("invalid sidecar call path")?;
        if let Some(owner) = &mut call.owner {
            owner.path = prefixed(prefix, &owner.path).ok_or("invalid sidecar owner path")?;
        }
        for target in &mut call.targets {
            if let Some(site) = target.location.repository_mut() {
                site.path = prefixed(prefix, &site.path).ok_or("invalid sidecar target path")?;
            }
        }
    }
    for input in &mut raw.inputs {
        // Container-side absolute paths are provider inputs, even on Windows.
        if !input.starts_with('/') && !Path::new(input).is_absolute() {
            *input = prefixed(prefix, input).ok_or("invalid sidecar input path")?;
        }
    }
    Ok(())
}

/// Each invocation gets one FileSet universe. Numeric token positions from
/// different go/packages loads must never share a compiler target map.
fn collect_isolated(
    source_root: &Path,
    wall: Duration,
    collect: impl Fn(&Path, Duration) -> NativeRawObservation + Sync + Send,
) -> NativeRawObservation {
    let started = Instant::now();
    let modules = match module_roots(source_root, started, wall) {
        Ok(modules) => modules,
        Err(message) => {
            return NativeRawObservation {
                budget_exceeded: message.starts_with("budget_exceeded"),
                invalid_environment: !message.starts_with("budget_exceeded"),
                diagnostics: vec![message],
                ..Default::default()
            }
        }
    };
    // Keep the single-root envelope; its paths already use repository coordinates.
    if modules.len() == 1 && modules[0] == source_root {
        return collect(source_root, wall.saturating_sub(started.elapsed()));
    }
    let mut merged = NativeRawObservation::default();
    let mut contexts = serde_json::Map::new();
    let results = crate::native_resolver::parallel::ordered_map(&modules, |module| {
        let remaining = wall.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            None
        } else {
            Some(collect(module, remaining))
        }
    });
    for (module, result) in modules.iter().zip(results) {
        let Some(mut raw) = result else {
            merged.budget_exceeded = true;
            merged
                .diagnostics
                .push("budget_exceeded: Go module invocations remain unexamined".into());
            continue;
        };
        let prefix = match module.strip_prefix(source_root) {
            Ok(path) => path.to_string_lossy().replace('\\', "/"),
            Err(error) => {
                merged.invalid_environment = true;
                merged.diagnostics.push(format!(
                    "invalid_environment: module outside source root: {error}"
                ));
                break;
            }
        };
        merged.sidecar_runs.push(NativeSidecarRun {
            source_prefix: prefix.clone(),
            output: raw.sidecar_output.take(),
            compiler_version: raw.compiler_version.clone(),
            invalid_environment: raw.invalid_environment,
            budget_exceeded: raw.budget_exceeded,
            diagnostics: raw.diagnostics.clone(),
        });
        contexts.insert(
            prefix.clone(),
            serde_json::Value::Object(std::mem::take(&mut raw.build_context)),
        );
        if let Err(error) = rebase_calls(&mut raw, &prefix) {
            merged.invalid_environment = true;
            merged
                .diagnostics
                .push(format!("invalid_environment: {prefix}: {error}"));
            // Original bytes remain in sidecar_runs; no malformed paths become facts.
            raw.calls.clear();
        }
        merged.invalid_environment |= raw.invalid_environment;
        merged.budget_exceeded |= raw.budget_exceeded;
        merged.diagnostics.extend(
            raw.diagnostics
                .into_iter()
                .map(|message| format!("{prefix}: {message}")),
        );
        merged.inputs.extend(raw.inputs);
        for tool in raw.tool_environment {
            if !merged.tool_environment.contains(&tool) {
                merged.tool_environment.push(tool);
            }
        }
        merged.calls.extend(raw.calls);
    }
    if let Err(error) =
        crate::native_resolver::package_api::merge_module_api(&mut merged, &contexts)
    {
        merged.invalid_environment = true;
        merged.diagnostics.push(error.to_string());
    }
    merged.build_context.insert(
        "isolated_modules".into(),
        serde_json::Value::Object(contexts),
    );
    merged.compiler_version = Some(ADAPTER_PIN.into());
    merged.calls.sort_by(|a, b| {
        (&a.site.path, a.site.span.start_byte, a.site.span.end_byte).cmp(&(
            &b.site.path,
            b.site.span.start_byte,
            b.site.span.end_byte,
        ))
    });
    merged.inputs.sort();
    merged.inputs.dedup();
    merged
}

/// Collect Go typed observations via the pinned sidecar in isolated module loads.
pub fn collect_go_observation(
    source_root: &Path,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    collect_isolated(source_root, wall, |module, remaining| {
        let missing = super::super::family_registry::missing_adapter_message("go", ENV_VAR);
        let mut raw = collect_via_sidecar_with_env(
            module,
            ENV_VAR,
            CANDIDATES,
            &missing,
            remaining,
            &[("ENTMOOT_GO_SINGLE_MODULE", PathBuf::from("1"))],
            canon,
        );
        if !raw.invalid_environment
            && raw
                .build_context
                .get("module_scope")
                .and_then(serde_json::Value::as_str)
                != Some("single_module.v1")
        {
            raw.invalid_environment = true;
            raw.calls.clear();
            raw.references.clear();
            raw.build_context.remove("package_api");
            raw.diagnostics.push("invalid_environment: Go sidecar must acknowledge single_module.v1; rebuild the pinned staging adapter".into());
        }
        raw
    })
}

/// Locate the configured adapter without starting a process.
pub fn configured_binary() -> Option<std::path::PathBuf> {
    super::sidecar::resolve_sidecar_binary(ENV_VAR, CANDIDATES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_discovery_uses_the_source_inventory_boundary() {
        let root = tempfile::tempdir().unwrap();
        for directory in [
            "src",
            ".tmp/old-checkout",
            "target/build",
            "node_modules/pkg",
            ".venv/lib",
            "_examples/rest",
            "examples/demo",
        ] {
            let path = root.path().join(directory);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("go.mod"), "module example.test/control\n").unwrap();
        }
        let modules = module_roots(root.path(), Instant::now(), Duration::from_secs(5)).unwrap();
        assert_eq!(modules, vec![root.path().join("src")]);
    }

    #[test]
    fn isolates_module_positions_rebases_paths_and_retains_each_response() {
        use entmoot_contracts::{CapturedOutput, NativeCallFact, NativeSidecarOutput};

        let root = tempfile::tempdir().unwrap();
        for module in ["a", "b"] {
            std::fs::create_dir(root.path().join(module)).unwrap();
            std::fs::write(
                root.path().join(module).join("go.mod"),
                format!("module {module}"),
            )
            .unwrap();
        }
        let invocations = std::sync::Mutex::new(Vec::new());
        let raw = collect_isolated(root.path(), Duration::from_secs(5), |module, remaining| {
            assert!(remaining <= Duration::from_secs(5));
            invocations.lock().unwrap().push(module.to_owned());
            NativeRawObservation {
                inputs: vec!["/usr/local/go/src/builtin/builtin.go".into()],
                calls: vec![NativeCallFact {
                    dispatch: Default::default(),
                    site: entmoot_contracts::NativeSpanRef {
                        path: "main.go".into(),
                        span: entmoot_contracts::SiteSpan {
                            start_byte: 1,
                            end_byte: 2,
                        },
                    },

                    targets: vec![],
                    owner: None,
                    gap: None,
                    limitation: None,
                    intrinsic: false,
                }],
                sidecar_output: Some(NativeSidecarOutput {
                    stdout: CapturedOutput::Utf8(module.display().to_string()),
                    stderr: CapturedOutput::Utf8(String::new()),
                }),
                ..Default::default()
            }
        });
        let mut invocations = invocations.into_inner().unwrap();
        invocations.sort();
        assert_eq!(invocations, [root.path().join("a"), root.path().join("b")]);
        assert_eq!(
            raw.calls
                .iter()
                .map(|c| c.site.path.as_str())
                .collect::<Vec<_>>(),
            ["a/main.go", "b/main.go"]
        );
        assert_eq!(raw.sidecar_runs.len(), 2);
        assert_eq!(raw.inputs, ["/usr/local/go/src/builtin/builtin.go"]);
        assert!(!raw.invalid_environment);
        for (index, run) in raw.sidecar_runs.iter().enumerate() {
            assert_eq!(
                run.output.as_ref().unwrap().stdout,
                CapturedOutput::Utf8(invocations[index].display().to_string())
            );
        }
        assert_eq!(prefixed("a", "../escape.go"), None);
        assert_eq!(prefixed("a", "/escape.go"), None);
        assert_eq!(prefixed("a", "C:/escape.go"), None);
        assert_eq!(prefixed("a", "sub\\file.go"), Some("a/sub/file.go".into()));
    }

    #[test]
    fn nested_parent_and_leaf_run_in_separate_position_universes_and_budget_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("go.mod"), "module outer").unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/go.mod"), "module inner").unwrap();
        let invocations = std::sync::Mutex::new(Vec::new());
        let raw = collect_isolated(root.path(), Duration::from_secs(5), |module, _| {
            invocations.lock().unwrap().push(module.to_owned());
            NativeRawObservation::default()
        });
        assert!(!raw.invalid_environment);
        let mut invocations = invocations.into_inner().unwrap();
        invocations.sort();
        assert_eq!(
            invocations,
            vec![root.path().to_owned(), root.path().join("nested")]
        );
        assert!(raw.diagnostics.is_empty());
        assert_eq!(raw.sidecar_runs.len(), 2);
        let raw = collect_isolated(root.path(), Duration::ZERO, |_, _| {
            panic!("must not invoke")
        });
        assert!(raw.budget_exceeded);
        assert!(!raw.invalid_environment);
    }

    #[test]
    fn profile_requires_sidecar() {
        assert_eq!(PROFILE.isolation, "go_sidecar_process");
        const { assert!(PROFILE.supports_types) };
    }
}
