//! Entmoot's native capture boundary; provider bytes retain their own semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ImplementationFingerprint, ProviderPathSegment, SiteSpan, SourceSnapshot};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPolicy {
    /// Metadata with build scripts and proc macros disabled.
    #[default]
    Safe,
    /// Explicitly permits bounded build scripts and proc-macro execution.
    Trusted,
}

/// Requested Cargo inputs, not proof of compiler acceptance or a Rust version.
/// Paths in this machine-local build context are not structural identities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RustBuildProfile {
    pub features: RustFeatureSelection,
    pub target_triple: Option<String>,
    pub sysroot: Option<PathBuf>,
    pub analysis: RustAnalysisMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RustFeatureSelection {
    All {},
    Selected {
        names: BTreeSet<String>,
        no_default_features: bool,
    },
}

impl Default for RustFeatureSelection {
    fn default() -> Self {
        Self::Selected {
            names: BTreeSet::new(),
            no_default_features: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RustAnalysisMode {
    /// Cargo workspace model, without enabling cfg(test).
    Workspace {},
    /// Exactly one Cargo target compiled as a test, plus ordinary dependencies.
    /// `crate_root` is a canonical repository-relative source path.
    Test { crate_root: String },
}

impl Default for RustAnalysisMode {
    fn default() -> Self {
        Self::Workspace {}
    }
}

impl RustBuildProfile {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let RustFeatureSelection::Selected { names, .. } = &self.features {
            if names.iter().any(|name| {
                name.is_empty() || name.chars().any(char::is_whitespace) || name.contains(',')
            }) {
                return Err("Rust feature names must be nonempty individual Cargo features");
            }
        }
        if self
            .target_triple
            .as_ref()
            .is_some_and(|target| target.is_empty() || target.chars().any(char::is_whitespace))
        {
            return Err("Rust target triple must be nonempty and contain no whitespace");
        }
        if self
            .sysroot
            .as_ref()
            .is_some_and(|path| !path.is_absolute() || path.to_str().is_none())
        {
            return Err("Rust sysroot must be an absolute UTF-8 path");
        }
        if let RustAnalysisMode::Test { crate_root } = &self.analysis {
            if crate::normalize_repo_path(crate_root).as_ref().ok() != Some(crate_root)
                || !crate_root.ends_with(".rs")
            {
                return Err(
                    "Rust test crate root must be a canonical repository-relative .rs path",
                );
            }
        }
        Ok(())
    }
}

/// Adapter identity and requested execution policy, distinct from a compiler's
/// reported version and from an independently qualified source language mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeProducerContext {
    pub adapter_pin: String,
    pub method_id: String,
    pub language: String,
    pub execution_policy: ExecutionPolicy,
}

/// Why a machine-local tool was selected. Selection is not language qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolSelection {
    Environment { variable: String },
    Project { path: String },
    Path { command: String },
}

/// Observed executable and version, separate from the adapter and syntax target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolEnvironment {
    pub role: String,
    pub executable: String,
    pub selection: ToolSelection,
    /// None when a bounded version probe could not establish a version.
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum NativeRawSeries {
    #[serde(rename = "entmoot.native_compiler_raw.v1")]
    V1,
}

/// Single-copy, write-only capture. The original process bytes and interpreted
/// provider facts are both retained; neither is synthesized from the other.
/// Readers must check `implementation` against their current implementation
/// before treating this Entmoot envelope as compatible.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NativeRawEnvelope<'a> {
    pub schema_version: NativeRawSeries,
    pub implementation: ImplementationFingerprint,
    pub snapshot: &'a SourceSnapshot,
    pub producer: &'a NativeProducerContext,
    pub input_digests: &'a BTreeMap<String, String>,
    pub limitations: &'a [&'a str],
    #[serde(flatten)]
    pub raw: &'a NativeRawObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum NativeReceiptSeries {
    #[serde(rename = "entmoot.native_receipt.v1")]
    V1,
}

/// Semantic coverage is independent of process success and never a confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeSemanticStatus {
    Ok,
    Partial,
    Failed,
    BudgetExceeded,
    StructuralFallback,
}

impl NativeSemanticStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::BudgetExceeded => "budget_exceeded",
            Self::StructuralFallback => "structural_fallback",
        }
    }
}

impl std::fmt::Display for NativeSemanticStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeReceipt {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_environment: Vec<ToolEnvironment>,
    pub schema_version: NativeReceiptSeries,
    pub implementation: ImplementationFingerprint,
    pub producer: NativeProducerContext,
    pub wall_seconds: f64,
    pub artifact_bytes: usize,
    pub raw_bytes: usize,
    pub inventory_calls: usize,
    /// Call sites observed by the native compiler, typechecker or lexical adapter.
    /// This measures observation coverage, independently of runtime dispatch.
    pub observed_calls: usize,
    /// Counts of normalized call-site gap categories; mechanism details remain
    /// in attributed resolver observations and targets are retained.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub call_gaps: BTreeMap<String, usize>,
    pub input_files: usize,
    pub semantic_status: NativeSemanticStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}

impl NativeReceipt {
    pub fn validate(&self, current: &ImplementationFingerprint) -> Result<(), &'static str> {
        if &self.implementation != current {
            return Err("native receipt implementation fingerprint mismatch");
        }
        if !self.wall_seconds.is_finite() || self.wall_seconds < 0.0 {
            return Err("native receipt wall time must be finite and nonnegative");
        }
        let mut remaining = self.observed_calls;
        for (gap, count) in &self.call_gaps {
            if gap.trim().is_empty() || *count == 0 || *count > remaining {
                return Err(
                    "native receipt call gaps must be nonempty counts within observed calls",
                );
            }
            remaining -= count;
        }
        if [
            &self.producer.adapter_pin,
            &self.producer.method_id,
            &self.producer.language,
        ]
        .iter()
        .any(|field| field.trim().is_empty())
        {
            return Err("native receipt requires complete producer context");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_profiles_require_complete_modes_and_canonical_inputs() {
        let default = RustBuildProfile::default();
        default.validate().unwrap();
        let value = serde_json::to_value(&default).unwrap();
        for invalid in [
            serde_json::json!({"kind":"test"}),
            serde_json::json!({"kind":"workspace","crate_root":"src/lib.rs"}),
            serde_json::json!({"kind":"all_tests"}),
        ] {
            let mut changed = value.clone();
            changed["analysis"] = invalid;
            assert!(serde_json::from_value::<RustBuildProfile>(changed).is_err());
        }
        let mut contradictory_features = value;
        contradictory_features["features"] =
            serde_json::json!({"kind":"all", "names":[], "no_default_features":true});
        assert!(serde_json::from_value::<RustBuildProfile>(contradictory_features).is_err());
        for path in [
            "../lib.rs",
            "/lib.rs",
            "src\\lib.rs",
            "src/./lib.rs",
            "src/lib.rs ",
            "src/lib",
            "",
        ] {
            let profile = RustBuildProfile {
                analysis: RustAnalysisMode::Test {
                    crate_root: path.into(),
                },
                ..default.clone()
            };
            assert!(profile.validate().is_err(), "{path}");
        }
        for feature in ["", "two features", "one,two"] {
            let profile = RustBuildProfile {
                features: RustFeatureSelection::Selected {
                    names: [feature.into()].into(),
                    no_default_features: false,
                },
                ..default.clone()
            };
            assert!(profile.validate().is_err(), "{feature}");
        }
        let invalid_sysroot = RustBuildProfile {
            sysroot: Some("relative/sysroot".into()),
            ..default
        };
        assert!(invalid_sysroot.validate().is_err());
    }

    fn receipt() -> NativeReceipt {
        NativeReceipt {
            tool_environment: Vec::new(),
            schema_version: NativeReceiptSeries::V1,
            implementation: ImplementationFingerprint {
                contracts: crate::contract_fingerprint(),
                derivation: crate::ContentDigest::parse(crate::blake3_hex(
                    b"fixture implementation",
                ))
                .unwrap(),
            },
            producer: NativeProducerContext {
                adapter_pin: "fixture@1".into(),
                method_id: "fixture.native.v1".into(),
                language: "python".into(),
                execution_policy: ExecutionPolicy::Safe,
            },
            wall_seconds: 0.5,
            artifact_bytes: 1,
            raw_bytes: 1,
            inventory_calls: 1,
            observed_calls: 0,
            call_gaps: BTreeMap::new(),
            input_files: 1,
            semantic_status: NativeSemanticStatus::Failed,
            limitations: vec!["compiler unavailable; version unknown".into()],
        }
    }

    #[test]
    fn dynamic_call_gaps_are_observation_counts_not_environment_failures() {
        let mut value = receipt();
        value.semantic_status = NativeSemanticStatus::Partial;
        value.observed_calls = 3;
        value.call_gaps.insert("indirect_call".into(), 1);
        value.call_gaps.insert("unresolved_binding".into(), 1);
        value.validate(&value.implementation).unwrap();
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json["observed_calls"], 3);
        assert_eq!(json["call_gaps"]["indirect_call"], 1);
        assert!(json.get("compiler_calls").is_none());
        value.call_gaps.insert("provider_error".into(), 2);
        assert!(value.validate(&value.implementation).is_err());
    }

    #[test]
    fn native_receipt_requires_fingerprint_context_and_known_status_without_aliases() {
        let original = receipt();
        let current = original.implementation.clone();
        original.validate(&current).unwrap();
        let value = serde_json::to_value(&original).unwrap();
        let restored: NativeReceipt = serde_json::from_value(value.clone()).unwrap();
        restored.validate(&current).unwrap();
        for field in [
            "implementation",
            "producer",
            "schema_version",
            "semantic_status",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<NativeReceipt>(missing).is_err(),
                "{field}"
            );
        }
        for field in ["solver_pin", "ra_ap_pin", "adapter_pin"] {
            let mut legacy = value.clone();
            legacy[field] = "fixture@1".into();
            assert!(
                serde_json::from_value::<NativeReceipt>(legacy).is_err(),
                "{field}"
            );
        }
        for (field, invalid) in [
            ("schema_version", "entmoot.native_receipt.v2"),
            ("semantic_status", "complete"),
        ] {
            let mut changed = value.clone();
            changed[field] = invalid.into();
            assert!(serde_json::from_value::<NativeReceipt>(changed).is_err());
        }
        for field in ["contracts", "derivation"] {
            let mut changed = value.clone();
            changed["implementation"][field] = crate::blake3_hex(b"stale implementation").into();
            let stale: NativeReceipt = serde_json::from_value(changed).unwrap();
            assert!(stale.validate(&current).is_err());
        }
    }

    #[test]
    fn native_receipt_does_not_accept_empty_context_or_invalid_measurements() {
        let original = receipt();
        for elapsed in [-1.0, f64::NAN, f64::INFINITY] {
            let mut changed = original.clone();
            changed.wall_seconds = elapsed;
            assert!(changed.validate(&original.implementation).is_err());
        }
        for field in ["adapter_pin", "method_id", "language"] {
            let mut value = serde_json::to_value(&original).unwrap();
            value["producer"][field] = " ".into();
            let changed: NativeReceipt = serde_json::from_value(value).unwrap();
            assert!(changed.validate(&original.implementation).is_err());
        }
    }

    #[test]
    fn native_locations_require_complete_tagged_coordinates_without_flat_aliases() {
        use serde_json::json;
        let repository = json!({"kind":"repository", "site":{
            "path":"src/λ.py", "span":{"start_byte":7,"end_byte":19}
        }, "origin_file":"provider/input.py"});
        let unmapped = json!({"kind":"unmapped", "span":{"start_byte":7,"end_byte":19},
            "origin_file":"provider/input.py"});
        for value in [&repository, &unmapped] {
            let location: NativeTargetLocation = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(&location).unwrap(), *value);
            assert_eq!(location.origin_file(), Some("provider/input.py"));
            assert_eq!(location.span().start_byte, 7);
            assert_eq!(location.span().end_byte, 19);
        }
        for invalid in [
            json!({"path":"src/λ.py","start_byte":7,"end_byte":19}),
            json!({"kind":"repository","span":{"start_byte":7,"end_byte":19}}),
            json!({"kind":"repository","site":{"path":"src/λ.py"}}),
            json!({"kind":"repository","site":{"span":{"start_byte":7,"end_byte":19}}}),
            json!({"kind":"unmapped","span":{"start_byte":7}}),
            json!({"kind":"unmapped","site":repository["site"],"span":unmapped["span"]}),
        ] {
            assert!(serde_json::from_value::<NativeTargetLocation>(invalid).is_err());
        }
        let fact = json!({"site":repository["site"],"targets":[{
            "location":repository,"symbol":"leaf"
        }]});
        assert!(serde_json::from_value::<NativeCallFact>(fact.clone()).is_ok());
        for field in ["path", "start_byte", "end_byte"] {
            let mut invalid = fact.clone();
            invalid[field] = json!(0);
            assert!(serde_json::from_value::<NativeCallFact>(invalid).is_err());
            let mut invalid = fact.clone();
            invalid["targets"][0][field] = json!(0);
            assert!(serde_json::from_value::<NativeCallFact>(invalid).is_err());
        }
    }
}

/// One native-tool-observed call site keyed by exact inventory span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeCallFact {
    /// Producer-qualified dispatch, independently of definition lookup.
    #[serde(default)]
    pub dispatch: crate::CallDispatch,
    pub site: NativeSpanRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<NativeCallTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<NativeSpanRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitation: Option<String>,
    #[serde(default)]
    pub intrinsic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeSpanRef {
    pub path: String,
    /// Provider-reported UTF-8 bytes, not a digest-verified structural span.
    pub span: SiteSpan,
}

/// Repository mapping and provider coordinates travel together. Unmapped
/// coordinates remain lossless evidence; they never establish a repository bind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeTargetLocation {
    Repository {
        site: NativeSpanRef,
        /// Physical compiler input, retained for explicit lock-placement evidence.
        #[serde(skip_serializing_if = "Option::is_none")]
        origin_file: Option<String>,
    },
    Unmapped {
        span: SiteSpan,
        #[serde(skip_serializing_if = "Option::is_none")]
        origin_file: Option<String>,
    },
}

impl NativeTargetLocation {
    pub fn from_provider(
        origin_file: Option<String>,
        path: Option<String>,
        span: SiteSpan,
    ) -> Self {
        match path {
            Some(path) => Self::Repository {
                site: NativeSpanRef { path, span },
                origin_file,
            },
            None => Self::Unmapped { span, origin_file },
        }
    }

    pub fn repository(&self) -> Option<&NativeSpanRef> {
        match self {
            Self::Repository { site, .. } => Some(site),
            Self::Unmapped { .. } => None,
        }
    }

    pub fn repository_mut(&mut self) -> Option<&mut NativeSpanRef> {
        match self {
            Self::Repository { site, .. } => Some(site),
            Self::Unmapped { .. } => None,
        }
    }

    pub fn span(&self) -> &SiteSpan {
        match self {
            Self::Repository { site, .. } => &site.span,
            Self::Unmapped { span, .. } => span,
        }
    }

    pub fn origin_file(&self) -> Option<&str> {
        match self {
            Self::Repository { origin_file, .. } | Self::Unmapped { origin_file, .. } => {
                origin_file.as_deref()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeCallTarget {
    pub location: NativeTargetLocation,
    pub symbol: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    /// `stdlib` | `module` | `application` | `unknown`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_role: Option<String>,
    /// Optional prebuilt canonical PURL; join re-validates / prefers builders when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,
    /// Go module path, npm package name, etc. (not a repository file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    /// In-package callable ancestry; never repository file paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_segments: Vec<ProviderPathSegment>,
}

fn default_kind() -> String {
    "callable".into()
}

/// Exact process output. UTF-8 stays compact; invalid UTF-8 stays byte-exact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
pub enum CapturedOutput {
    Utf8(String),
    Bytes(Vec<u8>),
}

impl From<Vec<u8>> for CapturedOutput {
    fn from(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(text) => Self::Utf8(text),
            Err(error) => Self::Bytes(error.into_bytes()),
        }
    }
}

/// Original sidecar bytes before normalization, including failed/partial output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeSidecarOutput {
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

/// One isolated module invocation; paths in the original output remain untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeSidecarRun {
    pub source_prefix: String,
    pub output: Option<NativeSidecarOutput>,
    pub compiler_version: Option<String>,
    pub invalid_environment: bool,
    pub budget_exceeded: bool,
    pub diagnostics: Vec<String>,
}

/// Raw observation bag emitted by a language backend before inventory join.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeRawObservation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<crate::CanonExpressionFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_environment: Vec<ToolEnvironment>,
    /// Ordered isolated native-tool invocations and their byte-exact output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecar_runs: Vec<NativeSidecarRun>,
    /// Original provider output; discarded/unsupported fields remain attributable here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_output: Option<NativeSidecarOutput>,
    /// Provider-declared build context, without promoting it into graph truth.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub build_context: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub calls: Vec<NativeCallFact>,
    /// Optional reference-binding facts (imports, plain references) for resolved identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<NativeReferenceFact>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub invalid_environment: bool,
    #[serde(default)]
    pub budget_exceeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
}

/// One native-tool-observed name binding (import or reference), keyed by exact span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeReferenceFact {
    pub site: NativeSpanRef,
    pub kind: String,
    pub spelling: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<NativeCallTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<NativeSpanRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitation: Option<String>,
}
