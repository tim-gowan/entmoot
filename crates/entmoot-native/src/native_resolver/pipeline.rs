//! Shared native produce pipeline: validate → freeze → backend → join → receipt.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use entmoot_contracts::{
    blake3_hex, ContentDigest, ResolverArtifact, ResolverArtifactProducer, RESOLVER_ARTIFACT_SCHEMA,
};
use entmoot_contracts::{
    ExecutionPolicy, NativeProducerContext, NativeRawEnvelope, NativeRawSeries, NativeReceipt,
    NativeReceiptSeries, NativeSemanticStatus,
};
use serde::Serialize;
use thiserror::Error;

use crate::export::load_graph_json;
use crate::graph::TopologyGraph;
use crate::native_resolver::budget::{BudgetWatch, ProducerBudget, ProducerBudgetError};
use crate::native_resolver::join::{language_matches, normalize_observations, METHOD_ID};
use crate::native_resolver::manifest::{prepare_resolver_manifest, ManifestPrepError};
use crate::native_resolver::producers::{clang, go, java, oxc, python, roslyn, typescript};

use entmoot_contracts::NativeRawObservation;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProduceRequest {
    pub canon: bool,
    pub graph_path: PathBuf,
    pub source_root: PathBuf,
    pub output_dir: PathBuf,
    pub budget: ProducerBudget,
    pub execution_policy: ExecutionPolicy,
    pub rust_profile: entmoot_contracts::RustBuildProfile,
}

/// Shared family dispatch for ordinary binding and isolated execution workers.
pub fn produce_family(
    family: entmoot_contracts::BindingFamily,
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    use entmoot_contracts::BindingFamily::*;
    match family {
        Rust => produce_rust_resolver(request),
        Oxc => produce_oxc_resolver(request),
        Typescript => produce_typescript_resolver(request),
        Python => produce_python_resolver(request),
        Go => produce_go_resolver(request),
        Roslyn => produce_roslyn_resolver(request),
        Java => produce_java_resolver(request),
        Clang => produce_clang_resolver(request),
    }
}

#[derive(Debug, Clone)]
pub struct NativeProduceResult {
    pub raw: NativeRawObservation,
    pub artifact: ResolverArtifact,
    pub receipt: NativeReceipt,
    pub artifact_path: PathBuf,
    pub raw_path: PathBuf,
    pub receipt_path: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum NativeProducerError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("graph validation failed: {0}")]
    Graph(String),
    #[error("source snapshot drifted: {0}")]
    Drift(String),
    #[error(transparent)]
    Budget(#[from] ProducerBudgetError),
    #[error(transparent)]
    Manifest(#[from] ManifestPrepError),
    #[error("resolver artifact validation failed: {0}")]
    Artifact(String),
    #[error("output directory already exists: {0}")]
    OutputExists(PathBuf),
    #[error("native rust adapter unavailable; rebuild with feature `resolved-rust-analyzer`")]
    AdapterUnavailable,
    #[error("oxc adapter unavailable; rebuild with feature `resolved-oxc`")]
    AdapterUnavailableOxc,
    #[error("{0}")]
    Message(String),
}

fn digest_file(path: &Path) -> Result<String, NativeProducerError> {
    Ok(blake3_hex(&fs::read(path)?))
}

fn freeze_snapshot_digests(
    graph: &TopologyGraph,
    root: &Path,
) -> Result<BTreeMap<String, String>, NativeProducerError> {
    let mut observed = BTreeMap::new();
    for file in &graph.snapshot_files {
        let full = root.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let digest = digest_file(&full)?;
        if digest != file.content_digest {
            return Err(NativeProducerError::Drift(format!(
                "indexed source bytes changed at {}",
                file.path
            )));
        }
        observed.insert(file.path.clone(), digest);
    }
    Ok(observed)
}

fn recheck_snapshot_digests(
    expected: &BTreeMap<String, String>,
    root: &Path,
) -> Result<(), NativeProducerError> {
    for (path, digest) in expected {
        let full = root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let now = digest_file(&full)?;
        if &now != digest {
            return Err(NativeProducerError::Drift(format!(
                "source changed during analysis at {path}"
            )));
        }
    }
    Ok(())
}

fn write_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(usize, ContentDigest), NativeProducerError> {
    use std::io::Write;
    struct PayloadWriter {
        output: std::io::BufWriter<fs::File>,
        digest: blake3::Hasher,
        bytes: usize,
    }
    impl Write for PayloadWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.output.write_all(bytes)?;
            self.digest.update(bytes);
            self.bytes = self
                .bytes
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("native output length overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.output.flush()
        }
    }
    let mut writer = PayloadWriter {
        output: std::io::BufWriter::new(fs::File::create(path)?),
        digest: blake3::Hasher::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut writer, value)?;
    writer.flush()?;
    Ok((
        writer.bytes,
        ContentDigest::from_trusted(format!("blake3:{}", writer.digest.finalize().to_hex())),
    ))
}

fn write_atomic_dir<T>(
    output_dir: &Path,
    write: impl FnOnce(&Path) -> Result<T, NativeProducerError>,
) -> Result<T, NativeProducerError> {
    if output_dir.exists() {
        return Err(NativeProducerError::OutputExists(output_dir.to_path_buf()));
    }
    let parent = output_dir.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix("native.staging-")
        .tempdir_in(parent)?;
    let result = write(staging.path())?;
    let staging_path = staging.keep();
    fs::rename(&staging_path, output_dir).map_err(|error| {
        let _ = fs::remove_dir_all(&staging_path);
        NativeProducerError::Io(error)
    })?;
    Ok(result)
}

fn build_artifact(
    graph: &TopologyGraph,
    raw: &NativeRawObservation,
    language: &str,
    producer: &str,
    version: &str,
    build_context_digest: ContentDigest,
) -> Result<ResolverArtifact, NativeProducerError> {
    let observations = normalize_observations(graph, raw, language);
    let artifact = ResolverArtifact {
        assessments: super::join::normalize_reference_uses(graph, raw, language),
        package_api: super::package_api::normalize(graph, raw, language, &observations)?,
        schema_version: RESOLVER_ARTIFACT_SCHEMA.into(),
        implementation: crate::implementation_fingerprint(),
        producer: ResolverArtifactProducer {
            id: format!("entmoot.native.{producer}"),
            name: format!("native-{producer}"),
            version: version.into(),
            method_id: METHOD_ID.into(),
        },
        snapshot: graph.snapshot.clone(),
        language: language.into(),
        // Adapter pins do not establish a source language target mode.
        language_version: None,
        build_context_digest,
        observations,
    };
    artifact
        .validate()
        .map_err(|error| NativeProducerError::Artifact(error.to_string()))?;
    Ok(artifact)
}

struct ProduceConfig {
    language: &'static str,
    producer: &'static str,
    adapter_pin: String,
    limitations: &'static [&'static str],
}

// Family identity is shared by production and recovered checkpoint validation.
fn family_config(family: entmoot_contracts::BindingFamily) -> ProduceConfig {
    use entmoot_contracts::BindingFamily::*;
    let identity = entmoot_contracts::BindingEvidence::for_family(family);
    let limitations = match family {
        Rust => &[
            "explicit compilation universe; not a transactional filesystem snapshot",
            "no dispatch closure or runtime execution is established",
            "build-system plugins, implicit calls and generated source require separate evidence",
        ][..],
        Oxc => oxc::PROFILE.limitations,
        Typescript => typescript::PROFILE.limitations,
        Python => python::PROFILE.limitations,
        Go => go::PROFILE.limitations,
        Roslyn => roslyn::PROFILE.limitations,
        Java => java::PROFILE.limitations,
        Clang => clang::PROFILE.limitations,
    };
    ProduceConfig {
        language: identity.language,
        producer: identity.producer,
        adapter_pin: identity.adapter_pin.into(),
        limitations,
    }
}

fn produce_with_collector<F>(
    request: NativeProduceRequest,
    config: ProduceConfig,
    collect: F,
) -> Result<NativeProduceResult, NativeProducerError>
where
    F: FnOnce(
        &NativeProduceRequest,
        &TopologyGraph,
        &BudgetWatch,
    ) -> Result<NativeRawObservation, NativeProducerError>,
{
    let watch = BudgetWatch::start(request.budget);
    let graph = load_graph_json(&request.graph_path)
        .map_err(|error| NativeProducerError::Graph(error.to_string()))?;
    graph
        .validate()
        .map_err(|error| NativeProducerError::Graph(error.to_string()))?;
    let root = fs::canonicalize(&request.source_root)?;
    let before = freeze_snapshot_digests(&graph, &root)?;
    watch.check_files(before.len())?;
    let call_sites = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == crate::graph::NodeKind::CallStub
                && node
                    .path
                    .as_deref()
                    .is_some_and(|path| language_matches(config.language, path))
        })
        .count();
    watch.check_call_sites(call_sites)?;

    let lock_inputs = super::lock_instances::LockInputs::prepare(&root);
    let collected = match &lock_inputs {
        Ok(_) => collect(&request, &graph, &watch),
        Err(error) => Ok(NativeRawObservation {
            invalid_environment: true,
            diagnostics: vec![error.to_string()],
            ..Default::default()
        }),
    };
    let mut raw = match collected {
        Ok(value) => value,
        Err(NativeProducerError::Budget(ProducerBudgetError::WallClock)) => NativeRawObservation {
            budget_exceeded: true,
            diagnostics: vec!["native analysis budget exhausted".into()],
            ..Default::default()
        },
        Err(error)
            if matches!(
                error,
                NativeProducerError::AdapterUnavailable
                    | NativeProducerError::AdapterUnavailableOxc
            ) =>
        {
            return Err(error);
        }
        Err(error) => NativeRawObservation {
            invalid_environment: true,
            diagnostics: vec![error.to_string()],
            ..Default::default()
        },
    };
    if request.canon {
        super::canon::seal(&mut raw, &before);
    }
    {
        use crate::native_resolver::package_api_seek::{
            fill_references, AttachedDependencySurfacesResolver, PackageRootResolver,
        };
        let surfaces = crate::native_resolver::package_api_seek::surfaces_from_raw(&raw);
        // Mapped ENTMOOT_PACKAGE_API dependency surfaces already on the bag enrich
        // via AttachedDependencySurfacesResolver (no invented sibling roots).
        let attached = AttachedDependencySurfacesResolver {
            surfaces: &surfaces,
        };
        #[allow(unused_mut)]
        let mut resolvers: Vec<&dyn PackageRootResolver> = vec![&attached];
        #[cfg(feature = "resolved-oxc")]
        let npm = (matches!(config.language, "javascript" | "typescript" | "tsx")).then(|| {
            crate::native_resolver::producers::oxc_package_api::NpmPackageRootResolver::new(
                &root, &watch,
            )
        });
        #[cfg(feature = "resolved-oxc")]
        if let Some(resolver) = npm.as_ref() {
            resolvers.push(resolver);
        }
        fill_references(&mut raw, config.language, &root, &resolvers)?;
        #[cfg(feature = "resolved-oxc")]
        if let Some(resolver) = npm {
            resolver.finish(&mut raw)?;
        }
    }
    if let Ok(inputs) = lock_inputs {
        inputs.annotate(&mut raw)?;
    }
    recheck_snapshot_digests(&before, &root)?;

    let producer = NativeProducerContext {
        adapter_pin: config.adapter_pin.clone(),
        method_id: METHOD_ID.into(),
        language: config.language.into(),
        execution_policy: request.execution_policy,
    };
    let (artifact, receipt) = write_atomic_dir(&request.output_dir, |staging| {
        let (raw_bytes, build_digest) = write_json(
            &staging.join("native.raw.json"),
            &NativeRawEnvelope {
                schema_version: NativeRawSeries::V1,
                implementation: crate::implementation_fingerprint(),
                snapshot: &graph.snapshot,
                producer: &producer,
                input_digests: &before,
                limitations: config.limitations,
                raw: &raw,
            },
        )?;
        let artifact = build_artifact(
            &graph,
            &raw,
            config.language,
            config.producer,
            &config.adapter_pin,
            build_digest,
        )?;
        watch.check_targets(
            artifact
                .observations
                .iter()
                .map(|observation| observation.targets.len())
                .sum(),
        )?;

        let (artifact_bytes, _) = write_json(&staging.join("resolver.json"), &artifact)?;
        let receipt = NativeReceipt {
            tool_environment: raw.tool_environment.clone(),
            schema_version: NativeReceiptSeries::V1,
            implementation: crate::implementation_fingerprint(),
            producer,
            wall_seconds: watch.elapsed_secs(),
            artifact_bytes,
            raw_bytes,
            inventory_calls: artifact.observations.len(),
            observed_calls: raw.calls.len(),
            call_gaps: call_gaps(&raw, config.language),
            input_files: before.len(),
            semantic_status: semantic_status(&raw),
            limitations: raw.diagnostics.clone(),
        };
        receipt
            .validate(&crate::implementation_fingerprint())
            .map_err(|error| NativeProducerError::Artifact(error.into()))?;
        write_json(&staging.join("receipt.json"), &receipt)?;
        Ok((artifact, receipt))
    })?;

    let artifact_path = request.output_dir.join("resolver.json");
    let producer_label = format!("native-{}", config.producer);
    let manifest = prepare_resolver_manifest(
        &graph,
        &artifact_path,
        &producer_label,
        &config.adapter_pin,
        config.language,
        &[],
    )?;
    let manifest_path = request.output_dir.join("resolver.json.manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    Ok(NativeProduceResult {
        raw,
        artifact,
        receipt,
        artifact_path,
        raw_path: request.output_dir.join("native.raw.json"),
        receipt_path: request.output_dir.join("receipt.json"),
        manifest_path,
    })
}

#[cfg(feature = "resolved-rust-analyzer")]
fn collect_rust(
    request: &NativeProduceRequest,
    graph: &TopologyGraph,
    watch: &BudgetWatch,
) -> Result<NativeRawObservation, NativeProducerError> {
    crate::native_resolver::rust_analyzer::collect(request, graph, watch)
}

#[cfg(not(feature = "resolved-rust-analyzer"))]
fn collect_rust(
    _request: &NativeProduceRequest,
    _graph: &TopologyGraph,
    _watch: &BudgetWatch,
) -> Result<NativeRawObservation, NativeProducerError> {
    Err(NativeProducerError::AdapterUnavailable)
}

/// Produce a Rust resolver artifact for an indexed graph.
pub fn produce_rust_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Rust),
        collect_rust,
    )
}

/// Produce a JavaScript/TypeScript resolver artifact via Oxc semantic analysis.
pub fn produce_oxc_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Oxc),
        |request, graph, watch| oxc::collect_oxc_observation(&request.source_root, graph, watch),
    )
}

/// Produce JS/TS declarations using the separate, explicitly installed TypeScript tier.
pub fn produce_typescript_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let wall = request.budget.wall;
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Typescript),
        move |request, _graph, _watch| {
            Ok(typescript::collect_typescript_observation(
                &request.source_root,
                wall,
                request.canon,
            ))
        },
    )
}

/// Produce Python declarations via the explicitly configured Pyright batch adapter.
pub fn produce_python_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let wall = request.budget.wall;
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Python),
        move |request, _graph, _watch| {
            Ok(python::collect_python_observation(
                &request.source_root,
                wall,
                request.canon,
            ))
        },
    )
}

/// Produce a Go resolver artifact via the `go/packages` sidecar.
pub fn produce_go_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let wall = request.budget.wall;
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Go),
        move |request, _graph, _watch| {
            Ok(go::collect_go_observation(
                &request.source_root,
                wall,
                request.canon,
            ))
        },
    )
}

/// Produce a C# resolver artifact via the Roslyn sidecar.
pub fn produce_roslyn_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let wall = request.budget.wall;
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Roslyn),
        move |request, _graph, _watch| {
            Ok(roslyn::collect_roslyn_observation(
                &request.source_root,
                wall,
                request.canon,
            ))
        },
    )
}

fn call_gaps(raw: &NativeRawObservation, language: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for call in &raw.calls {
        let gap = call
            .gap
            .as_deref()
            .map(|reason| entmoot_contracts::DispatchGap::from_provider(language, reason));
        let reason = gap.as_ref().or(match &call.dispatch {
            entmoot_contracts::CallDispatch::Open { reason } => Some(reason),
            _ => None,
        });
        if let Some(reason) = reason {
            *counts.entry(reason.category().to_owned()).or_default() += 1;
        }
    }
    counts
}

/// An accepted native attempt is not necessarily a complete represented pass.
fn semantic_status(raw: &NativeRawObservation) -> NativeSemanticStatus {
    fn partial(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
                (matches!(key.as_str(), "partial" | "no_classpath" | "noClasspath")
                    && (value.as_bool() == Some(true) || value.as_str() == Some("true")))
                    || partial(value)
            }),
            serde_json::Value::Array(values) => values.iter().any(partial),
            _ => false,
        }
    }
    if raw.invalid_environment {
        NativeSemanticStatus::Failed
    } else if raw.budget_exceeded {
        NativeSemanticStatus::BudgetExceeded
    } else if raw.build_context.iter().any(|(key, value)| {
        (matches!(key.as_str(), "partial" | "no_classpath" | "noClasspath")
            && (value.as_bool() == Some(true) || value.as_str() == Some("true")))
            || partial(value)
    }) {
        NativeSemanticStatus::Partial
    } else if raw.calls.is_empty() {
        NativeSemanticStatus::StructuralFallback
    } else {
        NativeSemanticStatus::Ok
    }
}

/// Produce Java evidence through the shipped javac Trees materialization.
pub fn produce_java_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let wall = request.budget.wall;
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Java),
        move |request, _graph, _watch| {
            Ok(java::collect_java_observation(
                &request.source_root,
                wall,
                request.canon,
            ))
        },
    )
}

/// Produce a C/C++ resolver artifact via the Clang sidecar.
pub fn produce_clang_resolver(
    request: NativeProduceRequest,
) -> Result<NativeProduceResult, NativeProducerError> {
    let wall = request.budget.wall;
    produce_with_collector(
        request,
        family_config(entmoot_contracts::BindingFamily::Clang),
        move |request, _graph, _watch| {
            Ok(clang::collect_clang_observation(
                &request.source_root,
                wall,
                request.canon,
            ))
        },
    )
}

pub use entmoot_contracts::binding_evidence::validate_binding_evidence;

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn streamed_native_outputs_keep_exact_bytes_and_failed_directories_private() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("output");
        let value = serde_json::json!({"message": "é\r\n", "values": [1, 2, 3]});
        let expected = serde_json::to_vec(&value).unwrap();
        let (length, digest) = write_atomic_dir(&output, |staging| {
            write_json(&staging.join("payload.json"), &value)
        })
        .unwrap();
        assert_eq!(fs::read(output.join("payload.json")).unwrap(), expected);
        assert_eq!(length, expected.len());
        assert_eq!(digest.as_str(), blake3_hex(&expected));
        let rejected = directory.path().join("rejected");
        let result: Result<(), _> = write_atomic_dir(&rejected, |staging| {
            write_json(&staging.join("payload.json"), &value)?;
            Err(NativeProducerError::Artifact(
                "deliberate validation failure".into(),
            ))
        });
        assert!(result.is_err());
        assert!(!rejected.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    #[test]
    fn malformed_lock_config_is_invalid_environment_before_any_native_tool() {
        for (language, file, text) in [
            ("rust", "a.rs", "fn caller() { run(); }"),
            ("python", "a.py", "def caller():\n    run()\n"),
            ("javascript", "a.js", "function caller() { run(); }"),
            ("typescript", "a.ts", "function caller() { run(); }"),
            ("tsx", "a.tsx", "function caller() { run(); }"),
            ("go", "a.go", "package p\nfunc caller() { run() }"),
            ("java", "A.java", "class A { void caller() { run(); } }"),
            ("csharp", "A.cs", "class A { void caller() { run(); } }"),
            ("c", "a.c", "void caller() { run(); }"),
            ("cpp", "a.cpp", "void caller() { run(); }"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("source");
            fs::create_dir_all(root.join(".entmoot")).unwrap();
            fs::write(root.join(file), text).unwrap();
            fs::write(root.join(".entmoot/lock_bindings.json"), b"{invalid}").unwrap();
            let graph =
                crate::build_topology_from_repo_with_options(&root, Default::default()).unwrap();
            let graph_path = temp.path().join("graph.json");
            crate::write_graph_json(&graph, &graph_path).unwrap();
            let result = produce_with_collector(
                NativeProduceRequest {
                    canon: false,
                    graph_path,
                    source_root: root,
                    output_dir: temp.path().join("output"),
                    budget: Default::default(),
                    execution_policy: ExecutionPolicy::Safe,
                    rust_profile: Default::default(),
                },
                ProduceConfig {
                    language,
                    producer: language,
                    adapter_pin: "fixture".into(),
                    limitations: &[],
                },
                |_, _, _| panic!("invalid explicit lock config must not invoke a native tool"),
            )
            .unwrap();
            assert!(result.raw.invalid_environment, "{language}");
            assert!(
                result.raw.compiler_version.is_none(),
                "{language}: a failed tool has no observed version"
            );
            assert!(
                result.artifact.language_version.is_none(),
                "{language}: an adapter pin is not a language mode"
            );
            result.receipt.validate(&graph.implementation).unwrap();
            assert_eq!(result.artifact.implementation, graph.implementation);
            assert!(!result.artifact.observations.is_empty(), "{language}");
            assert!(
                result
                    .artifact
                    .observations
                    .iter()
                    .all(|o| o.targets.is_empty()),
                "{language}"
            );
        }
    }

    #[test]
    fn single_copy_envelope_retains_every_fact_and_exact_process_byte(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use entmoot_contracts::{CapturedOutput, NativeSidecarOutput};

        let raw = NativeRawObservation {
            sidecar_output: Some(NativeSidecarOutput {
                stdout: CapturedOutput::Utf8(" {\"unknown_provider_field\":[1,1]} \n".into()),
                stderr: CapturedOutput::Bytes(vec![0xff, 0, 0xfe]),
            }),
            build_context: serde_json::from_value(
                serde_json::json!({"vendor_extension":{"ordered":[3,1,3]}}),
            )?,
            invalid_environment: true,
            diagnostics: vec!["provider failed; exact bytes retained".into()],
            ..Default::default()
        };
        let snapshot = entmoot_contracts::SourceSnapshot {
            repository_id: "fixture/native-envelope".into(),
            revision: "a".repeat(40),
            source_state: entmoot_contracts::SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let raw_value = serde_json::to_value(&raw)?;
        for language in [
            "rust",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "python",
            "java",
            "csharp",
            "c",
            "cpp",
        ] {
            let value = serde_json::to_value(NativeRawEnvelope {
                schema_version: NativeRawSeries::V1,
                implementation: crate::implementation_fingerprint(),
                snapshot: &snapshot,
                producer: &NativeProducerContext {
                    adapter_pin: "fixture".into(),
                    method_id: METHOD_ID.into(),
                    language: language.into(),
                    execution_policy: ExecutionPolicy::Safe,
                },
                input_digests: &BTreeMap::new(),
                limitations: &[],
                raw: &raw,
            })?;
            assert!(
                value.get("raw").is_none(),
                "raw capture must not be duplicated"
            );
            assert_eq!(value["schema_version"], "entmoot.native_compiler_raw.v1");
            assert_eq!(
                value["implementation"],
                serde_json::to_value(crate::implementation_fingerprint())?
            );
            for obsolete in [
                "adapter_pin",
                "producer_version",
                "language",
                "execution_policy",
                "method_id",
            ] {
                assert!(
                    value.get(obsolete).is_none(),
                    "duplicated producer field: {obsolete}"
                );
            }
            if let serde_json::Value::Object(fields) = &raw_value {
                let retained: serde_json::Map<String, serde_json::Value> = fields
                    .keys()
                    .map(|key| (key.clone(), value[key].clone()))
                    .collect();
                assert_eq!(
                    serde_json::from_value::<NativeRawObservation>(retained.into())?,
                    raw
                );
            } else {
                panic!("native raw observation must be an object");
            }
        }
        Ok(())
    }

    #[test]
    fn dynamic_gaps_preserve_observations_without_failing_production() {
        let raw: NativeRawObservation = serde_json::from_value(serde_json::json!({
            "calls": [
                {"site":{"path":"api.py","span":{"start_byte":10,"end_byte":17}}},
                {"site":{"path":"api.py","span":{"start_byte":30,"end_byte":35}},"gap":"indirect_call"},
                {"site":{"path":"api.py","span":{"start_byte":50,"end_byte":55}},"gap":"dynamic_dispatch"}
            ], "build_context":{"partial":true}
        })).unwrap();
        assert_eq!(raw.calls.len(), 3);
        assert_eq!(
            call_gaps(&raw, "python"),
            BTreeMap::from([("indirect_call".into(), 2)])
        );
        assert_eq!(semantic_status(&raw), NativeSemanticStatus::Partial);
    }

    #[test]
    fn provider_partial_and_missing_vfs_are_never_ok_or_empty_negatives() {
        let mut raw = entmoot_contracts::NativeRawObservation::default();
        raw.build_context.insert("partial".into(), true.into());
        assert_eq!(semantic_status(&raw).as_str(), "partial");
        raw.build_context.insert("partial".into(), "true".into());
        assert_eq!(semantic_status(&raw).as_str(), "partial");
        raw.build_context.clear();
        raw.build_context.insert(
            "isolated_modules".into(),
            serde_json::json!({"nested":{"noClasspath":true}}),
        );
        assert_eq!(semantic_status(&raw).as_str(), "partial");
        raw.budget_exceeded = true;
        assert_eq!(semantic_status(&raw).as_str(), "budget_exceeded");
        raw.invalid_environment = true;
        assert_eq!(semantic_status(&raw).as_str(), "failed");
    }
}
