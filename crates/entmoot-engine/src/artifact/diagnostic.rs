//! Stable machine-readable diagnostic codes for artifact processing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_pointer: Option<String>,
}

impl ArtifactDiagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            artifact_id: None,
            provider_pointer: None,
        }
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            artifact_id: None,
            provider_pointer: None,
        }
    }

    pub fn info(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Info,
            message: message.into(),
            artifact_id: None,
            provider_pointer: None,
        }
    }

    pub fn with_artifact(mut self, artifact_id: &str) -> Self {
        self.artifact_id = Some(artifact_id.into());
        self
    }

    pub fn with_pointer(mut self, pointer: &str) -> Self {
        self.provider_pointer = Some(pointer.into());
        self
    }

    pub fn severity_label(&self) -> &'static str {
        match self.severity {
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Info => "info",
        }
    }
}
