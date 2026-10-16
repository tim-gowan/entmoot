//! Print Entmoot BLAKE3 digests for immutable producer inputs.
use std::{
    env, fs,
    io::{self, Read},
    path::Path,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: blake3_digest <file|--stdin|--files-stdin>")?;
    if path == "--scope-stdin" {
        // [profile, include selectors, exclude selectors, exact observed paths]
        let (profile, includes, excludes, paths): (String, Vec<String>, Vec<String>, Vec<String>) =
            serde_json::from_reader(io::stdin().lock())?;
        let scope = entmoot_contracts::ScopeUniverse::from_selectors(
            &profile,
            vec![],
            includes,
            excludes,
            entmoot_contracts::ScopeCompleteness::CompleteRelativeToUniverse,
            Some(&paths),
        );
        scope.validate()?;
        println!("{}", serde_json::to_string(&scope)?);
    } else if path == "--manifest-stdin" {
        let entries: Vec<(String, String)> = serde_json::from_reader(io::stdin().lock())?;
        println!("{}", entmoot_contracts::content_manifest_digest(&entries));
    } else if path == "--files-stdin" {
        let paths: Vec<String> = serde_json::from_reader(io::stdin().lock())?;
        let digests = paths
            .iter()
            .map(|path| fs::read(path).map(|bytes| entmoot_contracts::blake3_hex(&bytes)))
            .collect::<Result<Vec<_>, _>>()?;
        println!("{}", serde_json::to_string(&digests)?);
    } else {
        let bytes = if path == "--stdin" {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes)?;
            bytes
        } else {
            fs::read(Path::new(&path))?
        };
        println!("{}", entmoot_contracts::blake3_hex(&bytes));
    }
    Ok(())
}
