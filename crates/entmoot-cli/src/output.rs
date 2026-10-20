//! Parsing only; shared query operations own information-tier semantics.
use anyhow::Result;
use clap::{Args, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Detail {
    /// Small navigation page; full evidence stays addressable (default).
    Compact,
    /// Expanded selected evidence, at most three traversal details per list.
    Explain,
    /// Full original result within the explicit search/detail budgets.
    Export,
}

#[derive(Debug, Default, Args)]
pub(super) struct InformationArgs {
    /// Override request output.tier. Defaults to compact when absent from the request.
    #[arg(long, value_enum)]
    detail: Option<Detail>,
    /// Maximum serialized JSON bytes per response; excludes transport framing.
    #[arg(long, value_parser = clap::value_parser!(u64).range(128..=67_108_864))]
    output_max_bytes: Option<u64>,
    /// Compact presentation items per page (default 10), separate from search limits.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=1000))]
    output_limit: Option<u16>,
    /// JSON page.next_cursor from the same query/publication; compact only.
    #[arg(long)]
    output_after: Option<PathBuf>,
}

static JSON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(super) fn configure(json: bool) {
    JSON.store(json, std::sync::atomic::Ordering::Relaxed);
}

/// Machine consumers always receive the typed JSON contract. Terminal presentation
/// hides storage implementation paths; --json retains every field for diagnosis.
pub(super) fn emit(value: &impl serde::Serialize) -> Result<()> {
    use std::io::IsTerminal;
    if JSON.load(std::sync::atomic::Ordering::Relaxed) || !std::io::stdout().is_terminal() {
        println!("{}", serde_json::to_string(value)?);
    } else {
        render(&serde_json::to_value(value)?, 0);
    }
    Ok(())
}
fn render(value: &serde_json::Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "store" | "graph" | "pointer" | "artifact_path"
                ) || value.is_null()
                {
                    continue;
                }
                if value.is_object() || value.is_array() {
                    if value.as_array().is_some_and(Vec::is_empty) {
                        continue;
                    }
                    println!("{pad}{}:", key.replace('_', " "));
                    render(value, indent + 2);
                } else {
                    println!("{pad}{}: {}", key.replace('_', " "), text(value));
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if item.is_object() {
                    render(item, indent);
                    println!();
                } else {
                    println!("{pad}- {}", text(item));
                }
            }
        }
        _ => println!("{pad}{}", text(value)),
    }
}
fn text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}
