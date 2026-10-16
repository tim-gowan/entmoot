//! File-level structural hashing demo.
//!
//! Usage:
//!   cargo run --example hash_file -- <lang> <file_a> [file_b]
//!
//! Prints the structural hash for each file. When two files are given,
//! also prints whether the hashes are equal or different.

use std::env;
use std::fs;
use std::path::Path;
use std::process;

use entmoot_core::StructuralHasher;

fn main() {
    let args: Vec<String> = env::args().collect();
    let prog = args.first().map(String::as_str).unwrap_or("hash_file");

    if args.len() < 3 || args.len() > 4 {
        eprintln!("Usage: {prog} <lang> <file_a> [file_b]");
        process::exit(1);
    }

    let lang = &args[1];
    let path_a = Path::new(&args[2]);
    let path_b = args.get(3).map(|p| Path::new(p.as_str()));

    let hasher = StructuralHasher::new();

    let hash_a = hash_one(&hasher, lang, path_a);
    println!("{}: {hash_a}", path_a.display());

    if let Some(path_b) = path_b {
        let hash_b = hash_one(&hasher, lang, path_b);
        println!("{}: {hash_b}", path_b.display());
        if hash_a == hash_b {
            println!("equal");
        } else {
            println!("different");
            process::exit(2);
        }
    }
}

fn hash_one(hasher: &StructuralHasher, lang: &str, path: &Path) -> String {
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {e}", path.display());
        process::exit(1);
    });

    hasher.compute_hash(&source, lang, None).unwrap_or_else(|e| {
        eprintln!("Failed to hash {}: {e}", path.display());
        process::exit(1);
    })
}
