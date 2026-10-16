//! Language registry for managing language-specific parsers and normalizers.

use std::collections::HashMap;

use tree_sitter::Language;

use crate::normalizer::ASTNormalizer;

/// Semantic family for shared patterns when adding languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageFamily {
    Scripting,
    Jvm,
    Systems,
}

/// Feature flags advertised by a language plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageCapabilities {
    pub granular: bool,
    pub import_tracking: bool,
    /// Static languages: type signatures are logical contracts.
    pub type_contracts: bool,
}

impl LanguageCapabilities {
    pub const fn scripting() -> Self {
        Self {
            granular: true,
            import_tracking: true,
            type_contracts: false,
        }
    }

    pub const fn jvm() -> Self {
        Self {
            granular: true,
            import_tracking: true,
            type_contracts: true,
        }
    }

    pub const fn systems() -> Self {
        Self {
            granular: true,
            import_tracking: true,
            type_contracts: true,
        }
    }
}

/// Metadata and implementation for a supported language.
pub struct LanguagePlugin {
    pub name: String,
    pub extensions: Vec<String>,
    pub language: Language,
    pub normalizer: Box<dyn ASTNormalizer>,
    pub family: LanguageFamily,
    pub capabilities: LanguageCapabilities,
}

impl LanguagePlugin {
    pub fn new(
        name: impl Into<String>,
        extensions: Vec<String>,
        language: Language,
        normalizer: Box<dyn ASTNormalizer>,
        family: LanguageFamily,
        capabilities: LanguageCapabilities,
    ) -> Self {
        Self {
            name: name.into(),
            extensions,
            language,
            normalizer,
            family,
            capabilities,
        }
    }
}

/// Registry for managing language support.
pub struct LanguageRegistry {
    by_name: HashMap<String, LanguagePlugin>,
    by_extension: HashMap<String, String>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self {
            by_name: HashMap::new(),
            by_extension: HashMap::new(),
        }
    }

    pub fn register(&mut self, plugin: LanguagePlugin) {
        for ext in &plugin.extensions {
            self.by_extension.insert(ext.clone(), plugin.name.clone());
        }
        self.by_name.insert(plugin.name.clone(), plugin);
    }

    pub fn get_supported_languages(&self) -> Vec<String> {
        let mut names: Vec<String> = self.by_name.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn get_language(&self, name_or_ext: &str) -> Option<&LanguagePlugin> {
        if let Some(plugin) = self.by_name.get(name_or_ext) {
            return Some(plugin);
        }
        self.by_extension
            .get(name_or_ext)
            .and_then(|name| self.by_name.get(name))
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_presets_match_plan() {
        assert!(!LanguageCapabilities::scripting().type_contracts);
        assert!(LanguageCapabilities::jvm().type_contracts);
        assert!(LanguageCapabilities::systems().type_contracts);
    }
}
