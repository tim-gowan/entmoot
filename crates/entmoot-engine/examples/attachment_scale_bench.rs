//! Reproducible 100k-location attachment benchmark.
//!
//! Run in release mode: `cargo run --release -p entmoot-engine
//! --example attachment_scale_bench -- 100000`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use entmoot_contracts::{
    ArtifactFormat, ArtifactManifest, ArtifactProducer, AttachmentDisposition,
    AttachmentTargetKind, LocationExtent, LocationRecord, ScopeCompleteness, ScopeUniverse,
    SourceRegion, SourceSnapshot, SourceState,
};
use entmoot_engine::{resolve_location, AttachmentIndexes, NodeKind, TopologyGraph, TopologyNode};

struct TrackingAllocator;

static LIVE_HEAP_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_HEAP_BYTES: AtomicUsize = AtomicUsize::new(0);

fn account_allocation(size: usize) {
    let live = LIVE_HEAP_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    let mut peak = PEAK_HEAP_BYTES.load(Ordering::Relaxed);
    while live > peak {
        match PEAK_HEAP_BYTES.compare_exchange_weak(
            peak,
            live,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

// The benchmark owns the process-global allocator. Sizes come from the same
// Layout values used by System, so this tracks live/peak Rust heap bytes without
// changing the library's production allocator.
#[allow(unsafe_code)]
// SAFETY: methods forward to System with the same Layout; accounting is advisory only.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` is forwarded unchanged to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            account_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE_HEAP_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `pointer` was allocated with System using this same layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `pointer` and `layout` came from System; `new_size` is the
        // caller-requested replacement allocation size.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            LIVE_HEAP_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
            account_allocation(new_size);
        }
        replacement
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let location_count = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("100000")
        .parse::<usize>()?;
    if location_count == 0 || location_count > u32::MAX as usize {
        return Err("location count must be between 1 and u32::MAX".into());
    }

    let fixture_started = Instant::now();
    let path = "benchmark.rs";
    let snapshot = SourceSnapshot {
        repository_id: "entmoot:attachment-scale-benchmark".into(),
        revision: "a".repeat(40),
        source_state: SourceState::Committed,
        snapshot_digest: None,
        estate_id: None,
        subject: None,
        subject_aliases: Vec::new(),
    };
    let scope = ScopeUniverse::from_selectors(
        "attachment-scale-benchmark",
        vec![],
        vec![path.into()],
        vec![],
        ScopeCompleteness::Partial,
        None,
    );
    let artifact = ArtifactManifest::build(
        ArtifactFormat::Sarif21,
        ArtifactProducer {
            name: "entmoot-attachment-scale-benchmark".into(),
            version: Some("1".into()),
            information_uri: None,
            ruleset_id: None,
            config_class: None,
        },
        snapshot.clone(),
        scope,
        entmoot_contracts::blake3_hex(b"attachment-scale-benchmark"),
        None,
    )?;

    let mut graph = TopologyGraph::new("attachment-scale-benchmark");
    graph.snapshot = snapshot;
    let mut file = TopologyNode::shell(
        entmoot_contracts::topo_file_id(path),
        NodeKind::File,
        path,
        None,
    );
    file.path = Some(path.into());
    graph.upsert_node(file);

    let mut source = String::with_capacity(location_count * 12);
    let mut locations = Vec::with_capacity(location_count);
    for index in 0..location_count {
        let callee = format!("f{index:06}");
        let start = source.len();
        source.push_str(&callee);
        source.push_str("();\n");
        let end = source.len() - 1;

        let mut call = TopologyNode::shell(
            entmoot_contracts::topo_call_stub_id(path, start, &callee),
            NodeKind::CallStub,
            &callee,
            Some(entmoot_contracts::topo_file_id(path)),
        );
        call.path = Some(path.into());
        call.start_byte = Some(start);
        call.end_byte = Some(end);
        graph.upsert_node(call);

        locations.push(LocationRecord {
            location_id: format!("location-{index:012}"),
            artifact_id: artifact.artifact_id.clone(),
            path: path.into(),
            extent: LocationExtent::Region {
                region: SourceRegion::Bytes {
                    span: entmoot_contracts::SiteSpan {
                        start_byte: start,
                        end_byte: end,
                    },
                },
            },

            provider_pointer: format!("/results/{index}/locations/0"),
            provider_confidence: None,
        });
    }
    let fixture_time = fixture_started.elapsed();

    graph
        .snapshot_files
        .push(entmoot_engine::graph::SnapshotFileDigest {
            path: path.into(),
            content_digest: entmoot_contracts::blake3_hex(source.as_bytes()),
        });
    let sources = vec![(path.to_string(), source)];
    let index_started = Instant::now();
    let indexes = AttachmentIndexes::new(&graph, &sources);
    let index_time = index_started.elapsed();

    let resolve_started = Instant::now();
    let attachments: Vec<_> = locations
        .iter()
        .map(|location| resolve_location(location, &artifact, &indexes))
        .collect();
    let resolve_time = resolve_started.elapsed();
    let attached = attachments
        .iter()
        .filter(|attachment| attachment.disposition == AttachmentDisposition::Attached)
        .count();
    let call_sites = attachments
        .iter()
        .filter(|attachment| {
            attachment
                .selected_target
                .as_ref()
                .is_some_and(|target| target.kind == AttachmentTargetKind::CallSite)
        })
        .count();
    if attached != location_count || call_sites != location_count {
        return Err(format!(
            "attachment count mismatch: attached={attached}, call_sites={call_sites}, expected={location_count}"
        )
        .into());
    }

    let serialize_started = Instant::now();
    let serialized = serde_json::to_vec(&attachments)?;
    let serialize_time = serialize_started.elapsed();

    println!("locations={location_count}");
    println!("spans={location_count}");
    println!("attached={attached}");
    println!("call_site_attached={call_sites}");
    println!("fixture_ms={:.3}", fixture_time.as_secs_f64() * 1000.0);
    println!("index_build_ms={:.3}", index_time.as_secs_f64() * 1000.0);
    println!("resolve_ms={:.3}", resolve_time.as_secs_f64() * 1000.0);
    println!(
        "resolve_ns_per_location={:.1}",
        resolve_time.as_nanos() as f64 / location_count as f64
    );
    println!("serialize_ms={:.3}", serialize_time.as_secs_f64() * 1000.0);
    println!("output_bytes={}", serialized.len());
    println!(
        "peak_tracked_heap_bytes={}",
        PEAK_HEAP_BYTES.load(Ordering::Relaxed)
    );
    std::hint::black_box(serialized);
    Ok(())
}
