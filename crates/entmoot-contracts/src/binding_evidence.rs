//! Family identity and receipt admission shared by production and recovery.
use crate::{BindingFamily, ExecutionPolicy, NativeReceipt};
/// Pinned Rust adapter identity used in receipt validation and production.
pub const RUST_ADAPTER_PIN: &str = "0.0.358";

pub struct BindingEvidence {
    pub language: &'static str,
    pub producer: &'static str,
    pub adapter_pin: &'static str,
}
impl BindingEvidence {
    pub const fn for_family(family: BindingFamily) -> Self {
        use BindingFamily::*;
        let (language, producer, adapter_pin) = match family {
            Rust => ("rust", "rust", RUST_ADAPTER_PIN),
            Oxc => ("javascript", "javascript", "0.153.0"),
            Typescript => ("javascript", "typescript", "typescript.program/1"),
            Python => ("python", "python", "pyright-program/1.1.414"),
            Go => ("go", "go", "go_types_sidecar.isolated_modules.v1"),
            Roslyn => ("csharp", "csharp", "roslyn_semantic_model"),
            Java => ("java", "java", "javac_trees"),
            Clang => ("cxx", "cxx", "clang_libtooling"),
        };
        Self {
            language,
            producer,
            adapter_pin,
        }
    }
    pub fn validate(
        family: BindingFamily,
        receipt: &NativeReceipt,
        artifact: &crate::ArtifactManifest,
    ) -> Result<(), &'static str> {
        validate_binding_evidence(family, receipt, artifact)
    }
}
pub fn validate_binding_evidence(
    family: crate::BindingFamily,
    receipt: &NativeReceipt,
    artifact: &crate::ArtifactManifest,
) -> Result<(), &'static str> {
    let config = BindingEvidence::for_family(family);
    if receipt.producer.language != config.language
        || receipt.producer.adapter_pin != config.adapter_pin
        || receipt.producer.method_id != "entmoot.native_compiler_exact_spans.v1"
        || receipt.producer.execution_policy != ExecutionPolicy::Safe
        || artifact.producer.name != format!("native-{}", config.producer)
        || artifact.producer.version.as_deref() != Some(config.adapter_pin)
        || artifact.format != crate::ArtifactFormat::Resolver
    {
        return Err("binding receipt and artifact must describe the selected native family");
    }
    Ok(())
}
