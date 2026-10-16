//! Java language plugin.

pub mod normalizer;
pub mod scope_analyzer;
pub mod visitor;

pub use normalizer::{language, normalizer, JavaNormalizer};
