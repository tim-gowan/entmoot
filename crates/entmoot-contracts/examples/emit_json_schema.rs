//! Emit derived JSON Schema goldens for Entmoot interchange contracts.
//!
//! ```text
//! cargo run -p entmoot-contracts --example emit_json_schema
//! ```

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = entmoot_contracts::emit_contract_schemas_json()?;
    let out =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("generated/entmoot_contracts.schema.json");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, json)?;
    println!("wrote {}", out.display());
    Ok(())
}
