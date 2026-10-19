//! Measure retained reads without linking a producer or workspace orchestrator.
//! Usage: measure_reads <publication.view.json> <query-request.json>
//! First request is cold in this process; five repeats share the query handle.
//! OS caches are uncontrolled and phase RSS endpoints are not peak measurements.
use entmoot_contracts::query::{QueryRequest, QueryService};
use entmoot_query::QueryEngine;
use entmoot_store::measurement::PhaseTimer;
use serde_json::json;
use std::{io::Write, path::PathBuf, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let store = args
        .next()
        .map(PathBuf::from)
        .ok_or("missing store locator")?;
    let request_path = args
        .next()
        .map(PathBuf::from)
        .ok_or("missing query request file")?;
    if args.next().is_some() {
        return Err("expected exactly a store locator and query request file".into());
    }
    let request: QueryRequest =
        serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(request_path)?))?;
    let opening = PhaseTimer::start("open");
    let engine =
        QueryEngine::from_retained(&store, Default::default())?.with_source_store(Arc::new(
            entmoot_store::source_store::PublicationSourceStore::new(&store),
        ));
    let opening = opening.finish();
    let mut measurements = Vec::new();
    let mut first = None;
    let mut first_digest = None;
    for repetition in 0..6 {
        let timer = PhaseTimer::start(if repetition == 0 {
            "cold_query"
        } else {
            "warm_query"
        });
        let response = engine.execute(request.clone())?;
        let bytes = serde_json::to_vec(&response)?;
        let measurement = timer.finish();
        let digest = entmoot_contracts::blake3_hex(&bytes);
        let same_as_first = first_digest
            .as_ref()
            .is_none_or(|expected| *expected == digest);
        measurements.push(json!({"repetition": repetition, "measurement": measurement,
            "output_bytes": bytes.len(), "output_digest": digest, "same_as_first": same_as_first}));
        if first_digest.is_none() {
            first_digest = Some(digest);
            first = Some(response);
        }
    }
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(
        &mut output,
        &json!({
            "implementation": entmoot_store::implementation_fingerprint(),
            "request": request, "opening": opening, "measurements": measurements,
            "first_response": first,
            "limitations": ["one process and publication", "OS cache state uncontrolled",
                "RSS endpoints are not peaks", "timings include query delivery and serialization",
                "repeated response equality is not independent semantic qualification"]
        }),
    )?;
    output.write_all(b"\n")?;
    Ok(())
}
