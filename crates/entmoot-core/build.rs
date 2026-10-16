//! Compile the verified, pre-generated structural grammar bundle.
mod build_support;
mod package_fingerprint;

use std::{fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    package_fingerprint::emit()?;
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| io::Error::other("missing manifest directory"))?,
    );
    let out =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("missing Cargo output directory"))?);
    println!("cargo:rerun-if-changed=build_support.rs");
    println!("cargo:rerun-if-changed=generated/grammars");
    let bundle = build_support::prepare(&manifest.join("generated/grammars"), &out)?;
    for library in bundle.libraries {
        cc::Build::new()
            .files(library.sources)
            .includes(library.includes)
            .std("c11")
            .warnings(false)
            .try_compile(&format!("entmoot_grammar_{}", library.name))
            .map_err(io::Error::other)?;
    }
    fs::write(out.join("grammar_bindings.rs"), bundle.bindings)?;
    println!(
        "cargo:rustc-env=ENTMOOT_GRAMMAR_BUNDLE_FINGERPRINT={}",
        bundle.fingerprint
    );
    Ok(())
}
