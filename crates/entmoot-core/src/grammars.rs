//! Qualified grammar implementations. This identity commits parser/scanner/header
//! bytes and their evidence packets; it is separate from the structural policy.
//! No parsing success establishes compiler acceptance or a semantic binding.

// Generated Tree-sitter `LanguageFn` FFI; SAFETY comments live in the build output.
#[allow(unsafe_code)]
mod generated_grammar_bindings {
    include!(concat!(env!("OUT_DIR"), "/grammar_bindings.rs"));
}
pub use generated_grammar_bindings::*;

/// Bare BLAKE3 implementation fingerprint of the verified grammar manifest.
pub const IMPLEMENTATION_FINGERPRINT: &str = env!("ENTMOOT_GRAMMAR_BUNDLE_FINGERPRINT");
