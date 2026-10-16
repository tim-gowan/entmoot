//! Read-only grammar qualification: CST acceptance is separate from inventory.
use std::{env, fs};

use entmoot_core::{LanguageRegistry, StructuralHasher};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        return Err("usage: grammar_probe <language> <source-file>".into());
    }
    let bytes = fs::read(&args[2])?;
    let source = std::str::from_utf8(&bytes)?;
    let mut registry = LanguageRegistry::new();
    entmoot_core::languages::register_all(&mut registry);
    let plugin = registry.get_language(&args[1]).ok_or("unsupported language")?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&plugin.language)?;
    let tree = parser.parse(&bytes, None).ok_or("parser returned no tree")?;
    let result = StructuralHasher::new().compute_granular_hash_with_analysis(source, &args[1], None);
    let inventory = match result {
        Ok(result) => json!({
            "outcome": "accepted", "policy_id": result.policy_id,
            "callable_count": result.callables.len(), "markers": result.markers,
        }),
        Err(error) => json!({"outcome": "rejected", "reason": error.to_string()}),
    };
    println!(
        "{}",
        serde_json::to_string(&json!({
            "format": "entmoot.grammar_probe.v1", "language": args[1],
            "source_digest": format!("blake3:{}", blake3::hash(&bytes).to_hex()),
            "parser_abi": plugin.language.abi_version(),
            "parse_has_error": tree.root_node().has_error(),
            "cst": tree.root_node().to_sexp(), "inventory": inventory,
            "native_target_mode": "not_run", "compiler_runtime": "not_run",
        }))?
    );
    Ok(())
}
