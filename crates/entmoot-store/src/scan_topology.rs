//! Source observation for topology construction.
use entmoot_contracts::{FileIndexRecord, SourceSnapshot};
use entmoot_core::RepoScanResult;
use entmoot_topology::{graph::file_node_id, MarkerTopologyExtractor, TopologyGraph};
use std::path::Path;
pub fn build_topology_from_scan(root: &Path, scan: &RepoScanResult) -> TopologyGraph {
    build_topology_from_scan_with_snapshot(root, scan, None)
}

pub fn build_topology_from_scan_with_snapshot(
    root: &Path,
    scan: &RepoScanResult,
    snapshot: Option<SourceSnapshot>,
) -> TopologyGraph {
    let root_str = root.to_string_lossy();
    let records: Vec<(String, FileIndexRecord, Option<String>)> = scan
        .files
        .iter()
        .map(|f| {
            let rel = f
                .path
                .strip_prefix(root)
                .unwrap_or(&f.path)
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(&f.path).ok();
            (rel, f.record.clone(), source)
        })
        .collect();
    let mut extractor = match snapshot {
        Some(snapshot) => MarkerTopologyExtractor::with_snapshot(root_str.as_ref(), snapshot),
        None => MarkerTopologyExtractor::new(root_str.as_ref()),
    };
    extractor.on_repo_begin(&root_str);
    for (path, record, source) in &records {
        let file_id = file_node_id(path);
        extractor.on_file(path, record, source.as_deref());
        extractor.on_file_end(&file_id);
    }
    extractor.finish()
}
