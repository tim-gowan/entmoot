//! Review assessments and explicit test ownership. Reachability is not assertion coverage.
use crate::{ImplementationFingerprint, SiteSpan, SourceSnapshot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessmentCompleteness {
    pub inventory: bool,
    pub reference_attachment: bool,
    pub call_targets: bool,
    pub entrypoints: bool,
}
impl AssessmentCompleteness {
    pub fn established(&self) -> bool {
        self.inventory && self.reference_attachment && self.call_targets && self.entrypoints
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct UnusedRequest {
    pub path: Option<String>,
    pub offset: usize,
    pub limit: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
}
impl Default for UnusedRequest {
    fn default() -> Self {
        Self {
            path: None,
            offset: 0,
            limit: 50,
            max_nodes: 100_000,
            max_edges: 200_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnusedSignal {
    CompilerDiagnostic,
    NoObservedIncomingCalls,
    MutuallyReferencingIsland,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessmentSubject {
    pub node_id: String,
    pub path: Option<String>,
    pub label: String,
    pub span: Option<SiteSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnusedCandidate {
    pub signal: UnusedSignal,
    pub members: Vec<AssessmentSubject>,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<crate::EvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnusedReport {
    pub method: String,
    pub implementation: ImplementationFingerprint,
    pub snapshot: SourceSnapshot,
    pub completeness: AssessmentCompleteness,
    pub summary: String,
    pub candidates: Vec<UnusedCandidate>,
    pub bounds: crate::query::ResultBounds,
    pub examined_nodes: usize,
    pub examined_edges: usize,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestRunner {
    Cargo,
    Pytest,
    Unittest,
    Node,
    Go,
    Junit,
    Dotnet,
    Ctest,
    Explicit,
}

/// Target-level identity and declared ownership; commands are data, never executed by reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestTarget {
    pub id: String,
    pub runner: TestRunner,
    pub command: Vec<String>,
    /// Exact files or directory prefixes ending in '/'. Include target sources.
    pub owners: Vec<String>,
    #[serde(default)]
    pub fixtures: Vec<String>,
    /// Exact callable IDs in either compared snapshot. Ambiguous discovery stays target-level.
    #[serde(default)]
    pub occurrences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TestImpactRequest {
    pub targets: Vec<TestTarget>,
    pub max_nodes: usize,
    pub max_edges: usize,
}
impl Default for TestImpactRequest {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            max_nodes: 100_000,
            max_edges: 200_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TestSelectionReason {
    ExecutionCoverage {
        side: String,
        changed_path: String,
        evidence: Vec<crate::EvidenceRef>,
    },
    Owner {
        path: String,
    },
    Fixture {
        path: String,
    },
    ReverseRelationship {
        side: String,
        changed_path: String,
        nodes: Vec<String>,
    },
    Widened {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectedTestTarget {
    pub target: TestTarget,
    pub reasons: Vec<TestSelectionReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestImpactReport {
    pub method: String,
    pub implementation: ImplementationFingerprint,
    pub base_snapshot: SourceSnapshot,
    pub head_snapshot: SourceSnapshot,
    pub selected: Vec<SelectedTestTarget>,
    /// Required when ownership is absent, a changed file is unmapped, or a budget is exhausted.
    pub full_suite_required: bool,
    pub changed_paths: Vec<String>,
    pub limitations: Vec<String>,
}

/// An exact inventoried declaration in a source file, never a provider symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessmentDeclaration {
    pub path: String,
    pub source_digest: crate::ContentDigest,
    pub declaration: crate::SourceOccurrenceRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodeUseKind {
    NonCallReference,
    Escape,
    Entrypoint,
    Exposure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestExecutionOutcome {
    Passed,
    Failed,
    Unavailable,
    TimedOut,
}

/// Positive observations only. No observation closes a language's use universe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssessmentObservation {
    CodeUse {
        subject: AssessmentDeclaration,
        usage: CodeUseKind,
    },
    CompilerUnused {
        subject: AssessmentDeclaration,
        diagnostic_code: String,
        message: String,
    },
    TestExecution {
        target: TestTarget,
        outcome: TestExecutionOutcome,
        /// Instrumented hits only. Empty means coverage unavailable, never not executed.
        covered: Vec<AssessmentDeclaration>,
        limitations: Vec<String>,
    },
}

impl AssessmentObservation {
    pub fn validate(&self) -> Result<(), AssessmentEvidenceError> {
        let invalid = |message: String| AssessmentEvidenceError::Invalid(message);
        for subject in self.declarations() {
            subject.declaration.validate().map_err(invalid)?;
            crate::ContentDigest::parse(subject.source_digest.as_str())
                .map_err(|e| invalid(e.to_string()))?;
            if !crate::normalize_repo_path(&subject.path).is_ok_and(|p| p == subject.path) {
                return Err(invalid("noncanonical assessment source path".into()));
            }
        }
        match self {
            Self::TestExecution {
                target,
                outcome,
                covered,
                limitations,
            } => {
                target.validate()?;
                if covered.len() > 100_000
                    || (covered.is_empty() && limitations.is_empty())
                    || (matches!(
                        outcome,
                        TestExecutionOutcome::Unavailable | TestExecutionOutcome::TimedOut
                    ) && !covered.is_empty())
                {
                    return Err(invalid("invalid test execution coverage".into()));
                }
            }
            Self::CompilerUnused {
                diagnostic_code,
                message,
                ..
            } if diagnostic_code.trim().is_empty() || message.trim().is_empty() => {
                return Err(invalid(
                    "compiler diagnostic requires code and message".into(),
                ))
            }
            _ => {}
        }
        Ok(())
    }
    pub fn declarations(&self) -> Vec<&AssessmentDeclaration> {
        match self {
            Self::CodeUse { subject, .. } | Self::CompilerUnused { subject, .. } => vec![subject],
            Self::TestExecution { covered, .. } => covered.iter().collect(),
        }
    }

    pub(crate) fn canonical(&self) -> String {
        // Length-delimited fields avoid ambiguous string concatenation. Serde layout
        // is not an identity input; enum discriminants are explicit here.
        let mut fields = Vec::new();
        match self {
            Self::CodeUse { usage, .. } => fields.push(
                match usage {
                    CodeUseKind::NonCallReference => "reference",
                    CodeUseKind::Escape => "escape",
                    CodeUseKind::Entrypoint => "entrypoint",
                    CodeUseKind::Exposure => "exposure",
                }
                .to_owned(),
            ),
            Self::CompilerUnused {
                diagnostic_code,
                message,
                ..
            } => fields.extend([
                "compiler_unused".into(),
                diagnostic_code.clone(),
                message.clone(),
            ]),
            Self::TestExecution {
                target,
                outcome,
                limitations,
                ..
            } => {
                fields.extend([
                    "test_execution".into(),
                    target.id.clone(),
                    target.runner.as_str().into(),
                    match outcome {
                        TestExecutionOutcome::Passed => "passed",
                        TestExecutionOutcome::Failed => "failed",
                        TestExecutionOutcome::Unavailable => "unavailable",
                        TestExecutionOutcome::TimedOut => "timed_out",
                    }
                    .into(),
                ]);
                for list in [
                    &target.command,
                    &target.owners,
                    &target.fixtures,
                    &target.occurrences,
                    limitations,
                ] {
                    fields.push(list.len().to_string());
                    fields.extend(list.iter().cloned());
                }
            }
        }
        for subject in self.declarations() {
            fields.extend([
                subject.path.clone(),
                subject.source_digest.to_string(),
                subject.declaration.occurrence.logic_address.clone(),
                subject.declaration.occurrence.address_id.to_string(),
                subject.declaration.span.start_byte.to_string(),
                subject.declaration.span.end_byte.to_string(),
            ]);
        }
        crate::hash_parts(&fields.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssessmentEvidenceError {
    #[error("invalid assessment evidence: {0}")]
    Invalid(String),
}

impl TestTarget {
    pub fn validate(&self) -> Result<(), AssessmentEvidenceError> {
        if self.id.trim().is_empty()
            || self.command.is_empty()
            || self.command.iter().any(|s| s.trim().is_empty())
        {
            return Err(AssessmentEvidenceError::Invalid(
                "test target requires identity and command".into(),
            ));
        }
        for path in self.owners.iter().chain(&self.fixtures) {
            let trimmed = path.trim_end_matches('/');
            if !crate::normalize_repo_path(trimmed).is_ok_and(|p| p == trimmed) {
                return Err(AssessmentEvidenceError::Invalid(format!(
                    "invalid target path: {path}"
                )));
            }
        }
        Ok(())
    }
}

impl TestRunner {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Pytest => "pytest",
            Self::Unittest => "unittest",
            Self::Node => "node",
            Self::Go => "go",
            Self::Junit => "junit",
            Self::Dotnet => "dotnet",
            Self::Ctest => "ctest",
            Self::Explicit => "explicit",
        }
    }
}

/// External facts or explicit validation results entering the existing artifact lane.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessmentEvidence {
    pub schema_version: String,
    pub implementation: ImplementationFingerprint,
    pub snapshot: SourceSnapshot,
    pub producer: crate::ProducerRef,
    pub tools: Vec<crate::ToolEnvironment>,
    pub build_context: crate::ContentDigest,
    pub observations: Vec<AssessmentObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestValidationRequest {
    pub targets: Vec<TestTarget>,
    pub build_context: crate::ContentDigest,
    pub wall_seconds: u64,
}
