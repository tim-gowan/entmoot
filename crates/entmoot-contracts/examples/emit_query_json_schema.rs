use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = entmoot_contracts::query::emit_query_contract_schemas_json()?;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("generated/entmoot_query_contracts.schema.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, output)?;
    println!("wrote {}", path.display());
    Ok(())
}
