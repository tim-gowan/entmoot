//! The grammar manifest owns this private build contract. No grammar generators,
//! package managers or language SDKs run during a normal Cargo build.
use std::{
    collections::BTreeSet,
    fs, io,
    io::Read,
    path::{Component, Path, PathBuf},
};

struct Language {
    constant: &'static str,
    symbol: &'static str,
    abi: u32,
    parser: &'static str,
}

struct Source {
    path: &'static str,
    digest: &'static str,
    bytes: u64,
    compressed_digest: &'static str,
    compressed_bytes: u64,
}

struct Notice {
    path: &'static str,
    digest: &'static str,
    bytes: u64,
}

struct Package {
    name: &'static str,
    qualification: &'static str,
    qualification_digest: &'static str,
    languages: &'static [Language],
    sources: &'static [Source],
    notices: &'static [Notice],
}

include!("generated/grammars/manifest.rs");

pub(crate) struct Library {
    pub name: &'static str,
    pub sources: Vec<PathBuf>,
    pub includes: Vec<PathBuf>,
}

pub(crate) struct PreparedBundle {
    pub libraries: Vec<Library>,
    pub bindings: String,
    pub fingerprint: String,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn relative(path: &str) -> io::Result<&Path> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid(format!("invalid bundle-relative path: {path}")));
    }
    Ok(Path::new(path))
}

fn regular_path(root: &Path, path: &str) -> io::Result<PathBuf> {
    let mut result = root.to_owned();
    for component in relative(path)?.components() {
        result.push(component);
        match fs::symlink_metadata(&result) {
            Ok(meta) if meta.file_type().is_symlink() || is_reparse_point(&meta) => {
                return Err(invalid(format!("linked bundle path: {}", result.display())));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(result)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn valid_digest(expected: &str) -> io::Result<()> {
    let hex = expected
        .strip_prefix("blake3:")
        .ok_or_else(|| invalid("missing BLAKE3 scheme"))?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("invalid BLAKE3 commitment"));
    }
    Ok(())
}

fn verify(bytes: &[u8], expected: &str, size: u64, name: &str) -> io::Result<()> {
    valid_digest(expected)?;
    if bytes.len() as u64 != size || format!("blake3:{}", blake3::hash(bytes).to_hex()) != expected {
        return Err(invalid(format!("grammar bundle size/digest mismatch: {name}")));
    }
    Ok(())
}

fn read_committed(root: &Path, path: &str, digest: &str, size: u64) -> io::Result<Vec<u8>> {
    // Bound even a replaced input file before allocating/reading it.
    let file = fs::File::open(regular_path(root, path)?)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() != size {
        return Err(invalid(format!("grammar bundle input size mismatch: {path}")));
    }
    let limit = size.checked_add(1).ok_or_else(|| invalid("bundle size overflow"))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    verify(&bytes, digest, size, path)?;
    Ok(bytes)
}

fn unpack(source: &Source, compressed: &[u8]) -> io::Result<Vec<u8>> {
    let limit = source
        .bytes
        .checked_add(1)
        .ok_or_else(|| invalid("bundle size overflow"))?;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(compressed)
        .take(limit)
        .read_to_end(&mut bytes)?;
    verify(&bytes, source.digest, source.bytes, source.path)?;
    Ok(bytes)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit()))
}

/// Verify every compressed source and notice before returning any compiler inputs.
/// Only the supplied Cargo output directory is written; linked paths are rejected.
pub(crate) fn prepare(root: &Path, out: &Path) -> io::Result<PreparedBundle> {
    for directory in [root, out] {
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(invalid("grammar bundle roots must be ordinary directories"));
        }
    }
    let mut digest = blake3::Hasher::new();
    digest.update(b"entmoot.grammar_bundle.v1\0");
    digest.update(include_bytes!("generated/grammars/manifest.rs"));
    let fingerprint = digest.finalize().to_hex().to_string();
    let destination = format!("grammar-bundle-{fingerprint}");
    let mut libraries = Vec::new();
    let mut bindings = String::from("// Verified static Tree-sitter languages. Generated at build time.\n");
    let mut names = BTreeSet::new();
    for package in PACKAGES {
        if !identifier(package.name) || !names.insert(package.name) {
            return Err(invalid("invalid/duplicate grammar package"));
        }
        relative(package.qualification)?;
        valid_digest(package.qualification_digest)?;
        let mut library = Library {
            name: package.name,
            sources: Vec::new(),
            includes: Vec::new(),
        };
        let mut paths = BTreeSet::new();
        for source in package.sources {
            relative(source.path)?;
            if !paths.insert(source.path) || !(source.path.ends_with(".c") || source.path.ends_with(".h")) {
                return Err(invalid("invalid/duplicate grammar source"));
            }
            let input = format!("{}/{}.gz", package.name, source.path);
            let packed = read_committed(root, &input, source.compressed_digest, source.compressed_bytes)?;
            let bytes = unpack(source, &packed)?;
            let output = regular_path(out, &format!("{destination}/{}/{}", package.name, source.path))?;
            let parent = output.parent().ok_or_else(|| invalid("grammar source has no parent"))?;
            fs::create_dir_all(parent)?;
            // Keep timestamps stable when Cargo reruns for an unrelated input.
            if fs::read(&output).ok().as_deref() != Some(&bytes) {
                fs::write(&output, &bytes)?;
            }
            if source.path.ends_with(".c") {
                library.sources.push(output);
            }
            for language in package
                .languages
                .iter()
                .filter(|language| language.parser == source.path)
            {
                let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
                let declaration = format!("#define LANGUAGE_VERSION {}", language.abi);
                if !text.lines().any(|line| line == declaration)
                    || !text.contains(&format!("TS_PUBLIC const TSLanguage *{}(void)", language.symbol))
                {
                    return Err(invalid("grammar ABI/export differs from manifest"));
                }
            }
        }
        for language in package.languages {
            if !identifier(language.constant) || !identifier(language.symbol) || !paths.contains(language.parser) {
                return Err(invalid("invalid grammar language/export/parser"));
            }
            let parser = regular_path(out, &format!("{destination}/{}/{}", package.name, language.parser))?;
            library.includes.push(
                parser
                    .parent()
                    .ok_or_else(|| invalid("parser has no parent"))?
                    .to_owned(),
            );
            bindings.push_str(&format!(
                "/// Qualified parser ABI {}; evidence: {} ({}).\n\
                 pub const {}: tree_sitter_language::LanguageFn = unsafe {{\n\
                 unsafe extern \"C\" {{ fn {}() -> *const (); }}\n\
                 // SAFETY: the verified, statically linked generated parser exports this ABI.\n\
                 tree_sitter_language::LanguageFn::from_raw({})\n}};\n",
                language.abi,
                package.qualification,
                package.qualification_digest,
                language.constant,
                language.symbol,
                language.symbol,
            ));
        }
        for notice in package.notices {
            read_committed(
                root,
                &format!("{}/{}", package.name, notice.path),
                notice.digest,
                notice.bytes,
            )?;
        }
        libraries.push(library);
    }
    Ok(PreparedBundle {
        libraries,
        bindings,
        fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths_and_incomplete_commitments() {
        for path in ["", "/file", "a//b", "a/../b", "a/./b", "C:/file", "a\\b"] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert!(relative("typescript/common/scanner.h").is_ok());
        assert!(valid_digest("blake3:abc").is_err());
        assert!(valid_digest(&format!("blake3:{}", "G".repeat(64))).is_err());
    }

    #[test]
    fn rejects_replaced_compressed_sources_and_wrong_plaintext() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/grammars");
        let package = &PACKAGES[0];
        let source = &package.sources[0];
        let path = format!("{}/{}.gz", package.name, source.path);
        let bytes = read_committed(&root, &path, source.compressed_digest, source.compressed_bytes).unwrap();
        assert!(unpack(source, &bytes).is_ok());
        let mut replaced = bytes.clone();
        replaced[0] ^= 1;
        assert!(verify(&replaced, source.compressed_digest, source.compressed_bytes, &path).is_err());
        let wrong = Source { bytes: 1, ..*source };
        assert!(unpack(&wrong, &bytes).is_err());
        assert!(read_committed(&root, &path, source.compressed_digest, source.compressed_bytes - 1).is_err());
    }

    #[test]
    fn materializes_the_complete_bundle_reproducibly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/grammars");
        let output = tempfile::tempdir().unwrap();
        let first = prepare(&root, output.path()).unwrap();
        let second = prepare(&root, output.path()).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.bindings, second.bindings);
        assert_eq!(first.libraries.len(), 9);
        assert_eq!(
            first
                .libraries
                .iter()
                .map(|library| library.sources.len())
                .sum::<usize>(),
            17
        );
        for library in &first.libraries {
            assert!(identifier(library.name));
            assert!(!library.includes.is_empty());
        }
    }
}
