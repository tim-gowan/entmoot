//! TypeScript language plugin (.ts and .tsx).

pub mod normalizer;
pub mod scope_analyzer;
pub mod visitor;

pub use normalizer::{language, language_tsx, normalizer, TypeScriptNormalizer};
