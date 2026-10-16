//! Language-specific implementations.

mod ecmascript;
pub mod plugin;
pub mod python;

pub mod c_family;
pub mod csharp;
pub mod go;
pub mod java;
pub mod javascript;
pub mod rust;
pub mod typescript;

use crate::registry::{LanguageCapabilities, LanguageFamily, LanguageRegistry};

/// Register all implemented language plugins.
pub fn register_all(registry: &mut LanguageRegistry) {
    for (name, extensions, language) in [
        ("c", vec![".c", ".h"], crate::grammars::C.into()),
        (
            "cpp",
            vec![".cc", ".cpp", ".cxx", ".hh", ".hpp", ".hxx"],
            crate::grammars::CPP.into(),
        ),
    ] {
        registry.register(plugin::build_plugin(
            name,
            &extensions,
            language,
            Box::new(c_family::CFamilyNormalizer),
            LanguageFamily::Systems,
            LanguageCapabilities::systems(),
        ));
    }
    registry.register(plugin::build_plugin(
        "python",
        &[".py", ".pyi"],
        python::language(),
        python::normalizer(),
        LanguageFamily::Scripting,
        LanguageCapabilities::scripting(),
    ));
    registry.register(plugin::build_plugin(
        "javascript",
        &[".js", ".jsx", ".mjs", ".cjs"],
        javascript::language(),
        javascript::normalizer(),
        LanguageFamily::Scripting,
        LanguageCapabilities::scripting(),
    ));
    registry.register(plugin::build_plugin(
        "typescript",
        &[".ts", ".mts", ".cts"],
        typescript::language(),
        typescript::normalizer(),
        LanguageFamily::Scripting,
        LanguageCapabilities::scripting(),
    ));
    registry.register(plugin::build_plugin(
        "tsx",
        &[".tsx"],
        typescript::language_tsx(),
        typescript::normalizer(),
        LanguageFamily::Scripting,
        LanguageCapabilities::scripting(),
    ));
    registry.register(plugin::build_plugin(
        "java",
        &[".java"],
        java::language(),
        java::normalizer(),
        LanguageFamily::Jvm,
        LanguageCapabilities::jvm(),
    ));
    registry.register(plugin::build_plugin(
        "csharp",
        &[".cs"],
        csharp::language(),
        csharp::normalizer(),
        LanguageFamily::Jvm,
        LanguageCapabilities::jvm(),
    ));
    registry.register(plugin::build_plugin(
        "rust",
        &[".rs"],
        rust::language(),
        rust::normalizer(),
        LanguageFamily::Systems,
        LanguageCapabilities::systems(),
    ));
    registry.register(plugin::build_plugin(
        "go",
        &[".go"],
        go::language(),
        go::normalizer(),
        LanguageFamily::Systems,
        LanguageCapabilities::systems(),
    ));
}
