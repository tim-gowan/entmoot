//! Granular (Merkle) structural hashing demo for Python source files.
//!
//! Usage:
//!   cargo run --example granular_demo -- path/to/file.py

use std::env;
use std::fs;
use std::path::Path;

use entmoot_core::StructuralHasher;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!(
            "Usage: {} <path-to-python-file>",
            args.first().map(String::as_str).unwrap_or("granular_demo")
        );
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {e}", path.display());
        std::process::exit(1);
    });

    let hasher = StructuralHasher::new();
    let path_str = path.to_string_lossy();
    let result = hasher
        .compute_granular_hash_at(&source, "python", Some(path_str.as_ref()), None, None)
        .unwrap_or_else(|e| {
            eprintln!("Failed to compute granular hash: {e}");
            std::process::exit(1);
        });

    println!("File: {}", path.display());
    println!("Merkle file_hash: {}", result.file_hash);
    println!("Module shell hash: {}", result.module_shell_hash);
    println!();

    if result.imports.is_empty() {
        println!("File imports: (none)");
    } else {
        println!("File imports:");
        for import in &result.imports {
            println!("  - {import}");
        }
    }
    println!();

    if result.callables.is_empty() {
        println!("Callables: (none)");
    } else {
        println!("Callables ({}):", result.callables.len());
        let mut names: Vec<_> = result.callables.keys().collect();
        names.sort();
        for name in names {
            let callable = &result.callables[name];
            println!("  {name} [{} / {}]", callable.node_type, callable.kind);
            println!("    logic_address: {}", callable.logic_address);
            println!("    address_id: {}", callable.address_id);
            println!("    structural_hash: {}", callable.structural_hash);
            println!("    signature_hash: {}", callable.signature_hash);
            if callable.imports.is_empty() {
                println!("    imports: (none)");
            } else {
                println!("    imports: {}", callable.imports.join(", "));
            }
            if !callable.signature.is_empty() {
                println!("    signature: {}", callable.signature);
            }
        }
    }
}
