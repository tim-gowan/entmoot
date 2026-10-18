//! Package-local derivation fence, independent of checkout paths.
mod package_fingerprint;
fn main() -> std::io::Result<()> {
    package_fingerprint::emit()
}
