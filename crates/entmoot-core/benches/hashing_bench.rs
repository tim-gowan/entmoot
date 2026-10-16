//! Performance benchmarks for structural hashing across registered languages.

use criterion::Criterion;
use entmoot_core::StructuralHasher;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

fn bench_lang(c: &mut Criterion, name: &str, language: &str, code: &str) {
    let hasher = StructuralHasher::new();
    c.bench_function(name, |b| {
        b.iter(|| hasher.compute_hash(black_box(code), black_box(language), None));
    });
}

fn bench_compute_hash_all(c: &mut Criterion) {
    bench_lang(c, "compute_hash_python", "python", "def test(x):\n    return x + 1\n");
    bench_lang(
        c,
        "compute_hash_javascript",
        "javascript",
        "function test(x) { return x + 1; }\n",
    );
    bench_lang(
        c,
        "compute_hash_typescript",
        "typescript",
        "function test(x: number): number { return x + 1; }\n",
    );
    bench_lang(
        c,
        "compute_hash_java",
        "java",
        "public class Test { public int test(int x) { return x + 1; } }\n",
    );
    bench_lang(
        c,
        "compute_hash_go",
        "go",
        "package main\nfunc test(x int) int { return x + 1 }\n",
    );
    bench_lang(
        c,
        "compute_hash_csharp",
        "csharp",
        "public class Test { public int Test(int x) { return x + 1; } }\n",
    );
    bench_lang(c, "compute_hash_rust", "rust", "fn test(x: i32) -> i32 { x + 1 }\n");
}

// The corpus mode measures production inventory, independently of Criterion's
// tiny-input microbenchmarks. Inputs are pinned source paths, never copied trees.
#[derive(Deserialize)]
struct Input {
    path: String,
    language: String,
    repository: String,
    relative_path: String,
    digest: String,
}

fn corpus(manifest: &str, clones: bool) -> Result<(), Box<dyn Error>> {
    let inputs: Vec<Input> = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let sources = inputs
        .iter()
        .map(|input| {
            let source = std::fs::read_to_string(&input.path)?;
            let digest = format!("blake3:{}", blake3::hash(source.as_bytes()).to_hex());
            if digest != input.digest {
                return Err(format!("source drift: {}", input.path).into());
            }
            Ok(source)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let started = Instant::now();
    let hasher = StructuralHasher::new();
    let initialization_seconds = started.elapsed().as_secs_f64();
    // Inventory is setup for clone queries, not part of the timed clone operation.
    let contracts = if clones {
        inputs
            .iter()
            .zip(&sources)
            .map(|(input, source)| {
                hasher
                    .compute_granular_hash_with_analysis(source, &input.language, None)
                    .map(|result| result.markers.map(|m| m.code_contracts).unwrap_or_default())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let mut records = Vec::new();
    let mut measurements = Vec::new();
    for repetition in 0..2 {
        let mut seconds = BTreeMap::<&str, f64>::new();
        let mut current = Vec::new();
        for (i, (input, source)) in inputs.iter().zip(&sources).enumerate() {
            let started = Instant::now();
            if clones {
                let result = hasher.clone_patterns(source, &input.language, &contracts[i], 5, true, 500)?;
                *seconds.entry(&input.language).or_default() += started.elapsed().as_secs_f64();
                let patterns: Vec<_> = result
                    .patterns
                    .iter()
                    .map(|p| {
                        json!({
                            "owner":p.owner_start,"span":p.span,"extent":p.extent,
                            "tokens":p.token_count,"fingerprints":p.fingerprints
                        })
                    })
                    .collect();
                current.push(json!({"patterns":patterns,"truncated":result.truncated,"alpha_unavailable":result.alpha_unavailable_callables}));
                continue;
            }
            let result = hasher.compute_granular_hash_with_analysis_at(
                source,
                &input.language,
                Some(&input.relative_path),
                Some(&input.repository),
                None,
            );
            *seconds.entry(&input.language).or_default() += started.elapsed().as_secs_f64();
            let record = match result {
                Ok(result) => {
                    let mut imports = result.imports.clone();
                    imports.sort();
                    let callables: BTreeMap<_, _> = result
                        .callables
                        .iter()
                        .map(|(key, c)| {
                            let mut imports = c.imports.clone();
                            imports.sort();
                            (
                                key,
                                json!({"name":c.name,"node_type":c.node_type,"scope":c.scope,
                            "start":c.start_byte,"end":c.end_byte,"imports":imports}),
                            )
                        })
                        .collect();
                    json!({"index":entmoot_core::build_file_index(&result),
                        "callables":callables,"imports":imports})
                }
                Err(error) => json!({"error":format!("{error:?}")}),
            };
            current.push(record);
        }
        if repetition == 0 {
            records = current;
        } else if records != current {
            return Err("cold/warm inventory drift".into());
        }
        measurements.push(json!({"state":if repetition == 0 {"process_cold"} else {"warm"},
            "seconds_by_language":seconds,"total_seconds":seconds.values().sum::<f64>()}));
    }
    let errors = records.iter().filter(|record| record.get("error").is_some()).count();
    let records_bytes = serde_json::to_vec(&records)?;
    let receipt: Value = json!({"format":"entmoot.eval.structural_cost.v1", "files":inputs.len(),
        "operation":if clones {"clones"} else {"inventory"}, "timed_parser_invocations":inputs.len()*2,
        "source_bytes":sources.iter().map(String::len).sum::<usize>(),"errors":errors,
        "initialization_seconds":initialization_seconds,"measurements":measurements,
        "inventory_digest":format!("blake3:{}",blake3::hash(&records_bytes).to_hex()),"records":records,
        "limits":["Process-cold parsers; OS filesystem caches are not flushed",
            "File IO and result serialization are outside the inventory timer",
            "No native enrichment, database, transaction or incremental-cache measurement"]});
    println!("{}", serde_json::to_string(&receipt)?);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("--manifest" | "--clone-manifest")) {
        return corpus(
            args.get(2).ok_or("manifest path required")?,
            args[1] == "--clone-manifest",
        );
    }
    let mut criterion = Criterion::default().configure_from_args();
    bench_compute_hash_all(&mut criterion);
    criterion.final_summary();
    Ok(())
}
