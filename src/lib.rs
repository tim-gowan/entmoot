//! Entmoot — continuous-defense substrate (placeholder).
//!
//! This crate reserves the public name; substantive APIs will follow.

/// Placeholder so the package is a valid library.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }
}
