//! Shared stdin/stdout JSON protocol for external native resolved sidecars.
//!
//! Go, Roslyn, Java, Clang, and Python workers speak the same envelope. Process-local
//! compiler handles never cross this boundary.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use entmoot_contracts::{
    NativeCallFact, NativeCallTarget, NativeRawObservation, NativeSidecarOutput, NativeSpanRef,
};

use crate::native_resolver::pipeline::NativeProducerError;

/// Request body written to a sidecar's stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarRequest {
    #[serde(default)]
    pub canon: bool,
    pub root: String,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub plan: bool,
}

/// Response body read from a sidecar's stdout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarResponse {
    #[serde(default)]
    pub tool_environment: Vec<entmoot_contracts::ToolEnvironment>,
    /// Transport capture is assigned by the host, never accepted from provider JSON.
    #[serde(skip)]
    pub native_output: Option<NativeSidecarOutput>,
    #[serde(skip)]
    pub budget_exceeded: bool,
    #[serde(default)]
    pub build_context: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<String>,
    #[serde(default)]
    pub calls: Vec<SidecarCall>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub invalid_environment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarCall {
    #[serde(default)]
    pub dispatch: entmoot_contracts::CallDispatch,
    pub path: String,
    pub start_byte: i64,
    pub end_byte: i64,
    #[serde(default)]
    pub targets: Vec<SidecarTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<SidecarTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(default)]
    pub intrinsic: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarTarget {
    /// Compiler-selected physical declaration input; retained only in native evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub start_byte: i64,
    #[serde(default)]
    pub end_byte: i64,
    #[serde(default)]
    pub symbol: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
}

fn default_kind() -> String {
    "callable".into()
}

fn valid_span(start: i64, end: i64) -> bool {
    start >= 0 && start <= end && end <= u32::MAX as i64
}

fn map_target(target: &SidecarTarget) -> Option<NativeCallTarget> {
    if !valid_span(target.start_byte, target.end_byte) {
        return None;
    }
    let path = target
        .path
        .as_ref()
        .map(|value| value.replace('\\', "/"))
        .filter(|value| !value.is_empty());
    Some(NativeCallTarget {
        location: entmoot_contracts::NativeTargetLocation::from_provider(
            target.origin_file.clone(),
            path,
            entmoot_contracts::SiteSpan {
                start_byte: usize::try_from(target.start_byte).ok()?,
                end_byte: usize::try_from(target.end_byte).ok()?,
            },
        ),
        symbol: target.symbol.clone(),
        kind: target.kind.clone(),
        package_role: target.package_role.clone(),
        package_purl: target.package_purl.clone(),
        ecosystem: target.ecosystem.clone(),
        module_path: target.module_path.clone(),
        package_version: target.package_version.clone(),
        path_segments: Vec::new(),
    })
}

fn map_owner(owner: &SidecarTarget) -> Option<NativeSpanRef> {
    if !valid_span(owner.start_byte, owner.end_byte) {
        return None;
    }
    let path = owner
        .path
        .as_ref()
        .map(|value| value.replace('\\', "/"))
        .filter(|value| !value.is_empty())?;
    Some(NativeSpanRef {
        path,
        span: entmoot_contracts::SiteSpan {
            start_byte: usize::try_from(owner.start_byte).ok()?,
            end_byte: usize::try_from(owner.end_byte).ok()?,
        },
    })
}

/// Convert a sidecar response into the engine raw observation bag.
pub fn observation_from_sidecar(response: SidecarResponse) -> NativeRawObservation {
    let mut calls = Vec::with_capacity(response.calls.len());
    for call in &response.calls {
        if !valid_span(call.start_byte, call.end_byte) {
            continue;
        }
        let (Ok(start_byte), Ok(end_byte)) = (
            usize::try_from(call.start_byte),
            usize::try_from(call.end_byte),
        ) else {
            continue;
        };
        let mut targets: Vec<NativeCallTarget> =
            call.targets.iter().filter_map(map_target).collect();
        let owner = call.owner.as_ref().and_then(map_owner);
        let malformed =
            targets.len() != call.targets.len() || (call.owner.is_some() && owner.is_none());
        // Filtering an invalid alternative must never turn an ambiguous binding
        // into a unique one, or an invalid owner into an owner-free assertion.
        // Byte-exact provider output remains in native_output for inspection.
        if malformed {
            targets.clear();
        }
        if call.intrinsic && targets.is_empty() && !malformed {
            targets.push(NativeCallTarget {
                location: entmoot_contracts::NativeTargetLocation::from_provider(
                    None,
                    None,
                    entmoot_contracts::SiteSpan {
                        start_byte: 0,
                        end_byte: 0,
                    },
                ),
                symbol: "intrinsic".into(),
                kind: "intrinsic".into(),
                package_role: None,
                package_purl: None,
                ecosystem: None,
                module_path: None,
                package_version: None,
                path_segments: Vec::new(),
            });
        }
        calls.push(NativeCallFact {
            dispatch: call.dispatch.clone(),
            site: entmoot_contracts::NativeSpanRef {
                path: call.path.replace('\\', "/"),
                span: entmoot_contracts::SiteSpan {
                    start_byte,
                    end_byte,
                },
            },
            targets,
            owner,
            gap: call
                .gap
                .clone()
                .or_else(|| malformed.then(|| "unsupported_construct".into())),
            limitation: malformed
                .then(|| "invalid native target or owner span; binding unassessed".into()),
            intrinsic: call.intrinsic && !malformed,
        });
    }
    calls.sort_by(|a, b| {
        (&a.site.path, a.site.span.start_byte).cmp(&(&b.site.path, b.site.span.start_byte))
    });
    let mut inputs = response.inputs;
    inputs.sort();
    inputs.dedup();
    let mut diagnostics = response.diagnostics;
    diagnostics.sort();
    diagnostics.dedup();
    NativeRawObservation {
        expressions: Vec::new(),
        tool_environment: response.tool_environment,
        sidecar_runs: Vec::new(),
        sidecar_output: response.native_output,
        build_context: response.build_context,
        calls,
        references: Vec::new(),
        diagnostics,
        invalid_environment: response.invalid_environment,
        budget_exceeded: response.budget_exceeded,
        compiler_version: response.compiler_version,
        inputs,
    }
}

fn repo_relative_candidates(relative_candidates: &[&str]) -> bool {
    relative_candidates.iter().any(|candidate| {
        candidate.starts_with(".tmp/")
            || candidate.starts_with(".entmoot/")
            || candidate.starts_with("scripts/")
    })
}

/// Locate an adapter binary: exclusive `env_var` when set, else workspace
/// adapter profile, else `ENTMOOT_ADAPTERS_ROOT` / catalog relatives.
pub fn resolve_sidecar_binary(env_var: &str, relative_candidates: &[&str]) -> Option<PathBuf> {
    crate::native_resolver::adapter_profile::resolve_with_profile(
        env_var,
        relative_candidates,
        repo_relative_candidates(relative_candidates),
    )
}

/// Build the selected host command without running it; explicit host overrides are exclusive.
pub fn command_for_binary(
    binary: &Path,
) -> Result<(Command, Option<entmoot_contracts::ToolEnvironment>), NativeProducerError> {
    let binary = compiler_root(binary);
    let extension = binary
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let spec = match extension.as_str() {
        "js" | "cjs" => Some(("node", "ENTMOOT_NODE", None)),
        "dll" => Some(("dotnet", "ENTMOOT_DOTNET", Some("DOTNET_ROOT"))),
        "jar" => Some(("java", "ENTMOOT_JAVA", Some("JAVA_HOME"))),
        "py" => Some(("python", "ENTMOOT_PYTHON_EXECUTABLE", None)),
        _ => None,
    };
    let Some((name, explicit, home)) = spec else {
        return Ok((Command::new(binary), None));
    };
    let tool = super::super::environment::select_tool(name, explicit, home)
        .map_err(NativeProducerError::Message)?;
    let mut command = Command::new(&tool.executable);
    if extension == "jar" {
        command.arg("-jar");
    } else if extension == "py" {
        command.args(["-X", "utf8"]);
    }
    command.arg(binary);
    Ok((command, Some(tool)))
}

/// Spawn a sidecar, feed it `request`, and parse the JSON response.
pub fn run_sidecar(
    binary: &Path,
    request: &SidecarRequest,
    wall: Duration,
) -> Result<SidecarResponse, NativeProducerError> {
    run_sidecar_with_env(binary, request, wall, &[])
}

/// Like [`run_sidecar`], with extra child environment pairs (e.g. resolved
/// `ENTMOOT_PYTHON_ENVIRONMENT` when the parent process left it unset).
pub fn run_sidecar_with_env(
    binary: &Path,
    request: &SidecarRequest,
    wall: Duration,
    child_env: &[(&str, PathBuf)],
) -> Result<SidecarResponse, NativeProducerError> {
    // File-backed streams avoid pipe backpressure and inherited-pipe EOF waits.
    // Include request serialization and process startup in the same wall budget.
    let start = std::time::Instant::now();
    let mut input = tempfile::tempfile()?;
    serde_json::to_writer(&mut input, request)?;
    input.seek(SeekFrom::Start(0))?;
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let (mut command, mut tool) = command_for_binary(binary)?;
    if let Some(tool) = &mut tool {
        super::super::environment::probe_version(
            tool,
            wall.saturating_sub(start.elapsed())
                .min(Duration::from_secs(2)),
        );
    }
    for (key, value) in child_env {
        command.env(key, compiler_root(value));
    }
    command
        .stdin(input)
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    let result = crate::process::run(command, wall.saturating_sub(start.elapsed()))?;
    let timed_out = matches!(result.status, Err(crate::process::ProcessError::TimedOut));
    let status = result.status.map_err(|error| error.to_string());
    let kill_error = result.cleanup_error;
    fn capture(file: &mut std::fs::File) -> std::io::Result<Vec<u8>> {
        // Snapshot length bounds the read even if an escaped writer remains alive.
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(length).read_to_end(&mut bytes)?;
        Ok(bytes)
    }
    let out = capture(&mut stdout)?;
    let err = capture(&mut stderr)?;
    let mut response = match status {
        Ok(status) => decode_sidecar_output(out, err, status.success(), &status.to_string()),
        Err(message) => SidecarResponse {
            budget_exceeded: timed_out,
            invalid_environment: !timed_out,
            diagnostics: vec![message],
            native_output: Some(NativeSidecarOutput {
                stdout: out.into(),
                stderr: err.into(),
            }),
            ..Default::default()
        },
    };
    if let Some(error) = kill_error {
        // A successful parent may have already exhausted its process group.
        if timed_out {
            response
                .diagnostics
                .push(format!("sidecar process-tree cleanup failed: {error}"));
        }
    }
    if let Some(tool) = tool {
        response.inputs.push(tool.executable.clone());
        response.tool_environment.push(tool);
    }
    Ok(response)
}

fn decode_sidecar_output(
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    success: bool,
    status: &str,
) -> SidecarResponse {
    let mut response: SidecarResponse = match serde_json::from_slice(&stdout) {
        Ok(response) => response,
        Err(error) => SidecarResponse {
            invalid_environment: true,
            diagnostics: vec![format!(
                "sidecar returned invalid JSON: {error}; original output retained"
            )],
            ..Default::default()
        },
    };
    if !success {
        response.invalid_environment = true;
        response.diagnostics.push(format!(
            "sidecar exited {status}: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    response.native_output = Some(NativeSidecarOutput {
        stdout: stdout.into(),
        stderr: stderr.into(),
    });
    response
}

/// Windows canonicalization produces verbatim paths while libclang's CompDB
/// uses ordinary drive/UNC paths. Keep one spelling at the process boundary.
pub(crate) fn compiler_root(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        if let Some(Component::Prefix(prefix)) = components.next() {
            let plain = match prefix.kind() {
                Prefix::VerbatimDisk(drive) => {
                    Some(PathBuf::from(format!("{}:\\", char::from(drive))))
                }
                Prefix::VerbatimUNC(server, share) => Some(PathBuf::from(format!(
                    "\\\\{}\\{}",
                    server.to_string_lossy(),
                    share.to_string_lossy()
                ))),
                _ => None,
            };
            if let Some(mut plain) = plain {
                for part in components {
                    if let Component::Normal(value) = part {
                        plain.push(value);
                    }
                }
                return plain;
            }
        }
    }
    path.to_owned()
}

/// Run a configured sidecar or return an invalid_environment observation.
pub fn collect_via_sidecar(
    source_root: &Path,
    env_var: &str,
    relative_candidates: &[&str],
    missing_message: &str,
    wall: Duration,
    canon: bool,
) -> NativeRawObservation {
    collect_via_sidecar_with_env(
        source_root,
        env_var,
        relative_candidates,
        missing_message,
        wall,
        &[],
        canon,
    )
}

/// Like [`collect_via_sidecar`], injecting `child_env` into the sidecar process.
pub fn collect_via_sidecar_with_env(
    source_root: &Path,
    env_var: &str,
    relative_candidates: &[&str],
    missing_message: &str,
    wall: Duration,
    child_env: &[(&str, PathBuf)],
    canon: bool,
) -> NativeRawObservation {
    let Some(binary) = resolve_sidecar_binary(env_var, relative_candidates) else {
        return NativeRawObservation {
            invalid_environment: true,
            diagnostics: vec![missing_message.into()],
            ..Default::default()
        };
    };
    let root = match std::fs::canonicalize(source_root) {
        Ok(path) => path,
        Err(error) => {
            return NativeRawObservation {
                invalid_environment: true,
                diagnostics: vec![format!("cannot canonicalize source root: {error}")],
                ..Default::default()
            };
        }
    };
    let request = SidecarRequest {
        canon,
        root: compiler_root(&root).to_string_lossy().into_owned(),
        patterns: Vec::new(),
        flags: Vec::new(),
        plan: false,
    };
    match run_sidecar_with_env(&binary, &request, wall, child_env) {
        Ok(response) => observation_from_sidecar(response),
        Err(NativeProducerError::Budget(_)) => NativeRawObservation {
            budget_exceeded: true,
            diagnostics: vec!["native analysis budget exhausted".into()],
            ..Default::default()
        },
        Err(error) => NativeRawObservation {
            invalid_environment: true,
            diagnostics: vec![error.to_string()],
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {

    #[cfg(windows)]
    #[test]
    fn compiler_paths_match_ordinary_compilation_database_spelling() {
        for (canonical, ordinary) in [
            (r"\\?\G:\source\project", r"G:\source\project"),
            (r"\\?\UNC\server\share\project", r"\\server\share\project"),
            (r"G:\source\project", r"G:\source\project"),
            (
                r"\\?\G:\repo\.tmp\adapter-build\pyright-program\entmoot-pyright.cjs",
                r"G:\repo\.tmp\adapter-build\pyright-program\entmoot-pyright.cjs",
            ),
        ] {
            assert_eq!(
                super::compiler_root(std::path::Path::new(canonical)),
                std::path::PathBuf::from(ordinary)
            );
        }
    }
    use super::*;

    #[test]
    fn retains_exact_provider_output_build_context_and_failed_interpretations() {
        use entmoot_contracts::CapturedOutput;
        let stdout = br#" { "build_context":{"GOOS":"linux","tags":["x","x"]}, "calls":[{"path":"a.go","start_byte":-1,"end_byte":-1}],"inputs":["a.go","a.go"] } "#.to_vec();
        let stderr = vec![0xff, 0x00, 0x80];
        let raw = observation_from_sidecar(decode_sidecar_output(
            stdout.clone(),
            stderr.clone(),
            true,
            "0",
        ));
        assert!(!raw.invalid_environment);
        assert!(raw.calls.is_empty()); // Invalid spans never become invented bindings.
        assert_eq!(raw.build_context["GOOS"], "linux");
        assert_eq!(raw.build_context["tags"], serde_json::json!(["x", "x"]));
        assert_eq!(
            raw.sidecar_output.as_ref().unwrap().stdout,
            CapturedOutput::Utf8(String::from_utf8(stdout).unwrap())
        );
        assert_eq!(
            raw.sidecar_output.as_ref().unwrap().stderr,
            CapturedOutput::Bytes(stderr)
        );
        let bytes = serde_json::to_vec(&raw).unwrap();
        assert_eq!(
            serde_json::from_slice::<NativeRawObservation>(&bytes).unwrap(),
            raw
        );
        let unknown = br#"{"future_provider_field":{"value":7}}"#.to_vec();
        let raw =
            observation_from_sidecar(decode_sidecar_output(unknown.clone(), vec![], true, "0"));
        assert!(raw.invalid_environment);
        assert_eq!(
            raw.sidecar_output.unwrap().stdout,
            CapturedOutput::Utf8(String::from_utf8(unknown).unwrap())
        );
        let invalid = vec![0xff, 0x80];
        let raw = observation_from_sidecar(decode_sidecar_output(
            invalid.clone(),
            b"failed".to_vec(),
            false,
            "1",
        ));
        assert!(raw.invalid_environment);
        assert_eq!(
            raw.sidecar_output.unwrap().stdout,
            CapturedOutput::Bytes(invalid)
        );
    }

    #[test]
    fn maps_negative_owner_spans_to_none() {
        let response = SidecarResponse {
            calls: vec![SidecarCall {
                dispatch: Default::default(),
                path: "a.go".into(),
                start_byte: 1,
                end_byte: 2,
                targets: vec![],
                owner: Some(SidecarTarget {
                    path: Some("a.go".into()),
                    start_byte: -1,
                    end_byte: -1,
                    symbol: String::new(),
                    kind: "callable".into(),
                    ..Default::default()
                }),
                gap: Some("indirect_call".into()),
                intrinsic: false,
            }],
            ..Default::default()
        };
        let raw = observation_from_sidecar(response);
        assert_eq!(raw.calls.len(), 1);
        assert!(raw.calls[0].owner.is_none());
        assert_eq!(raw.calls[0].gap.as_deref(), Some("indirect_call"));
    }
}
