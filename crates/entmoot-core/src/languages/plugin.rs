//! Shared helpers for constructing language plugins.

use tree_sitter::Language;

use crate::normalizer::ASTNormalizer;
use crate::registry::{LanguageCapabilities, LanguageFamily, LanguagePlugin};

pub fn build_plugin(
    name: &str,
    extensions: &[&str],
    language: Language,
    normalizer: Box<dyn ASTNormalizer>,
    family: LanguageFamily,
    capabilities: LanguageCapabilities,
) -> LanguagePlugin {
    LanguagePlugin::new(
        name,
        extensions.iter().map(|e| e.to_string()).collect(),
        language,
        normalizer,
        family,
        capabilities,
    )
}
