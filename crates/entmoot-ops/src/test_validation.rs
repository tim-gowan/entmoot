//! Explicit target execution. Reads never invoke this lane; ownership is never reduced by it.
use entmoot_contracts::*;
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
pub enum TestValidationError {
    #[error("invalid test validation request: {0}")]
    Invalid(String),
    #[error("source snapshot changed during test validation")]
    SourceChanged,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn verify_sources(graph: &crate::TopologyGraph) -> Result<(), TestValidationError> {
    let root = Path::new(&graph.root);
    let expected: BTreeMap<_, _> = graph
        .snapshot_files
        .iter()
        .map(|f| (f.path.clone(), f.content_digest.clone()))
        .collect();
    let actual: BTreeMap<_, _> = entmoot_core::scan::snapshot_file_digests(root)
        .map_err(|e| TestValidationError::Invalid(e.to_string()))?
        .into_iter()
        .map(|f| {
            (
                f.path
                    .strip_prefix(root)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .replace('\\', "/"),
                f.content_digest,
            )
        })
        .collect();
    if actual != expected {
        return Err(TestValidationError::SourceChanged);
    }
    Ok(())
}

fn tool(command: &str, root: &Path) -> Result<ToolEnvironment, String> {
    let path = Path::new(command);
    if path.is_absolute() || path.components().count() > 1 {
        let path = root.join(path).canonicalize().map_err(|e| e.to_string())?;
        return Ok(ToolEnvironment {
            role: "test-runner".into(),
            executable: crate::native_resolver::producers::sidecar::compiler_root(&path)
                .to_string_lossy()
                .into_owned(),
            selection: ToolSelection::Project {
                path: command.into(),
            },
            version: None,
        });
    }
    crate::native_resolver::environment::select_tool(
        command.trim_end_matches(".exe"),
        "ENTMOOT_TEST_RUNNER",
        None,
    )
}

pub fn validate_tests(
    graph: &crate::TopologyGraph,
    request: TestValidationRequest,
) -> Result<AssessmentEvidence, TestValidationError> {
    if request.targets.is_empty()
        || request.targets.len() > 100
        || request.wall_seconds == 0
        || request.wall_seconds > 3600
    {
        return Err(TestValidationError::Invalid(
            "require 1..100 targets and a 1..3600 second per-target budget".into(),
        ));
    }
    // One shared target validator; a same-snapshot read selects nothing and runs nothing.
    crate::query::assess_test_impact(
        graph,
        graph,
        TestImpactRequest {
            targets: request.targets.clone(),
            ..Default::default()
        },
    )
    .map_err(|e| TestValidationError::Invalid(e.to_string()))?;
    ContentDigest::parse(request.build_context.as_str())
        .map_err(|e| TestValidationError::Invalid(e.to_string()))?;
    verify_sources(graph)?;
    let mut observations = Vec::new();
    let mut tools = Vec::new();
    let mut context = vec![
        request.build_context.to_string(),
        std::env::consts::OS.into(),
        std::env::consts::ARCH.into(),
    ];
    for key in [
        "RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "JAVA_HOME",
        "DOTNET_ROOT",
        "PYTHONPATH",
        "VIRTUAL_ENV",
        "NODE_OPTIONS",
        "GOFLAGS",
    ] {
        // Hash relevant configuration without copying environment contents into receipts.
        context.extend([key.into(), std::env::var(key).unwrap_or_default()]);
    }
    for target in request.targets {
        let mut limitations = vec!["target process status only; execution and assertion coverage are unavailable without instrumentation".into()];
        let outcome = match tool(&target.command[0], Path::new(&graph.root)) {
            Err(error) => {
                limitations.push(error);
                TestExecutionOutcome::Unavailable
            }
            Ok(mut tool) => {
                crate::native_resolver::environment::probe_version(
                    &mut tool,
                    Duration::from_secs(2),
                );
                context.extend([
                    tool.executable.clone(),
                    tool.version.clone().unwrap_or_default(),
                    blake3_hex(&std::fs::read(&tool.executable)?),
                ]);
                let mut output = tempfile::tempfile()?;
                let mut command = Command::new(&tool.executable);
                command
                    .args(&target.command[1..])
                    .current_dir(&graph.root)
                    .stdin(Stdio::null())
                    .stdout(output.try_clone()?)
                    .stderr(output.try_clone()?);
                let result =
                    crate::process::run(command, Duration::from_secs(request.wall_seconds));
                let outcome = match result {
                    Err(error) => {
                        limitations.push(error.to_string());
                        TestExecutionOutcome::Unavailable
                    }
                    Ok(result) => {
                        if let Some(error) = result.cleanup_error {
                            limitations.push(format!("process-tree cleanup: {error}"));
                        }
                        match result.status {
                            Ok(status) if status.success() => TestExecutionOutcome::Passed,
                            Ok(status) => {
                                limitations.push(status.to_string());
                                TestExecutionOutcome::Failed
                            }
                            Err(crate::process::ProcessError::TimedOut) => {
                                TestExecutionOutcome::TimedOut
                            }
                            Err(error) => {
                                limitations.push(error.to_string());
                                TestExecutionOutcome::Unavailable
                            }
                        }
                    }
                };
                // A bounded diagnostic tail is enough to identify failures; no permanent logs required.
                if outcome != TestExecutionOutcome::Passed {
                    let length = output.metadata()?.len();
                    output.seek(SeekFrom::Start(length.saturating_sub(4096)))?;
                    let mut bytes = Vec::new();
                    output.take(4096).read_to_end(&mut bytes)?;
                    limitations.push(String::from_utf8_lossy(&bytes).into_owned());
                }
                tools.push(tool);
                outcome
            }
        };
        verify_sources(graph)?;
        observations.push(AssessmentObservation::TestExecution {
            target,
            outcome,
            covered: vec![],
            limitations,
        });
    }
    let build_context = ContentDigest::from_trusted(format!(
        "blake3:{}",
        hash_parts(&context.iter().map(String::as_str).collect::<Vec<_>>())
    ));
    Ok(AssessmentEvidence {
        schema_version: "entmoot.assessment-evidence.v1".into(),
        implementation: crate::implementation_fingerprint(),
        snapshot: graph.snapshot.clone(),
        producer: ProducerRef {
            id: "entmoot.test-validation.v1".into(),
            name: "entmoot".into(),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            config_fingerprint: Some(request.build_context.to_string()),
            environment_fingerprint: Some(build_context.to_string()),
            capabilities: vec![ProducerCapabilityKind::RuntimeObserver],
        },
        tools,
        build_context,
        observations,
    })
}

pub fn write_evidence(
    artifact: &AssessmentEvidence,
    directory: &Path,
) -> Result<PathBuf, TestValidationError> {
    std::fs::create_dir_all(directory)?;
    let value = serde_json::to_value(artifact)?;
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    let mut manifest = ArtifactManifest::build(
        ArtifactFormat::AssessmentEvidence,
        ArtifactProducer {
            name: artifact.producer.name.clone(),
            version: artifact.producer.version.clone(),
            information_uri: None,
            ruleset_id: Some("entmoot.test-validation.v1".into()),
            config_class: None,
        },
        artifact.snapshot.clone(),
        ScopeUniverse::from_selectors(
            "test-validation",
            vec![],
            vec![],
            vec![],
            ScopeCompleteness::Partial,
            None,
        ),
        blake3_hex(&bytes),
        Some(artifact.build_context.to_string()),
    )
    .map_err(|e| TestValidationError::Invalid(e.to_string()))?;
    manifest.artifact_path = Some("evidence.json".into());
    let write = |path: &Path, value: &serde_json::Value| -> Result<(), TestValidationError> {
        let mut pending = crate::AtomicGraphWrite::begin(path)
            .map_err(|e| TestValidationError::Invalid(e.to_string()))?;
        pending
            .write_json(value)
            .map_err(|e| TestValidationError::Invalid(e.to_string()))?;
        pending
            .commit()
            .map_err(|e| TestValidationError::Invalid(e.to_string()))
    };
    write(&directory.join("evidence.json"), &value)?;
    let path = directory.join("manifest.json");
    write(&path, &serde_json::to_value(&manifest)?)?;
    Ok(path)
}
