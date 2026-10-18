//! Provider-neutral, deterministic location attachment.

use std::collections::{HashMap, HashSet};

use entmoot_contracts::{
    attachment_id, utf16_column_to_codepoint, utf8_column_to_codepoint, ArtifactManifest,
    AttachmentDisposition, AttachmentFreshness, AttachmentReason, AttachmentTarget,
    AttachmentTargetKind, ColumnEncoding, ExactBodyId, LocationAttachment, LocationRecord,
    LocationRef, LocationResolveStatus,
};

use crate::graph::{NodeKind, TopologyGraph};
use crate::span_index::{RangeIndex, SpanIndex};

#[derive(Clone)]
struct ResourceSpanHit {
    start_byte: usize,
    end_byte: usize,
    node_id: String,
    resource_id: Option<String>,
}

#[derive(Default)]
struct ResourceSpanData {
    hits: Vec<ResourceSpanHit>,
    ranges: RangeIndex,
}

struct SourceTextIndex<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceTextIndex<'a> {
    fn new(source: &'a str) -> Self {
        let mut line_starts = vec![if source.starts_with('\u{feff}') { 3 } else { 0 }];
        line_starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        );
        Self {
            source,
            line_starts,
        }
    }

    fn line(&self, one_based_line: usize) -> Option<(&'a str, usize)> {
        let index = one_based_line.checked_sub(1)?;
        let start = *self.line_starts.get(index)?;
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.source.len());
        let text = self.source.get(start..end)?;
        let line = match text.strip_suffix('\n') {
            Some(line) => line.strip_suffix('\r').unwrap_or(line),
            None => text,
        };
        Some((line, start))
    }
}

pub struct AttachmentIndexes<'a> {
    graph: &'a TopologyGraph,
    sources: HashMap<String, SourceTextIndex<'a>>,
    source_digests: HashMap<String, entmoot_contracts::ContentDigest>,
    verified_sources: HashSet<String>,
    file_ids: HashMap<&'a str, &'a str>,
    spans: SpanIndex,
    resource_hits_by_path: HashMap<String, ResourceSpanData>,
    port_targets: HashMap<String, HashMap<(usize, usize), Vec<AttachmentTarget>>>,
}

impl<'a> AttachmentIndexes<'a> {
    pub fn new(graph: &'a TopologyGraph, sources: &'a [(String, String)]) -> Self {
        let sources_map: HashMap<String, SourceTextIndex<'a>> = sources
            .iter()
            .map(|(p, s)| (p.replace('\\', "/"), SourceTextIndex::new(s)))
            .collect();
        let mut spans = SpanIndex::new();
        let mut markers_by_path: HashMap<String, entmoot_contracts::FileMarkers> = HashMap::new();
        let mut occ_by_path: HashMap<String, Vec<entmoot_contracts::OccurrenceRecord>> =
            HashMap::new();
        let mut resource_hits: HashMap<String, Vec<ResourceSpanHit>> = HashMap::new();
        for node in &graph.nodes {
            let Some(path) = node.path.as_deref() else {
                continue;
            };
            match node.kind {
                NodeKind::Callable => {
                    if let (Some(start), Some(end)) = (node.start_byte, node.end_byte) {
                        markers_by_path
                            .entry(path.to_string())
                            .or_default()
                            .definitions
                            .push(entmoot_contracts::DefMarker {
                                name: node.label.clone(),
                                kind: node.logic_kind.clone().unwrap_or_else(|| "function".into()),
                                scope: node.scope.clone(),
                                start_byte: start,
                                end_byte: end,
                            });
                    }
                    if let (Some(address_id), Some(logic_address)) =
                        (node.address_id.clone(), node.logic_address.clone())
                    {
                        occ_by_path.entry(path.to_string()).or_default().push(
                            entmoot_contracts::OccurrenceRecord {
                                address_id,
                                logic_address,
                                structural_hash: node.structural_hash.clone().unwrap_or_default(),
                                exact_body_id: node.exact_body_id.clone(),
                            },
                        );
                    }
                }
                NodeKind::CallStub => {
                    if let (Some(start), Some(end)) = (node.start_byte, node.end_byte) {
                        markers_by_path
                            .entry(path.to_string())
                            .or_default()
                            .call_sites
                            .push(entmoot_contracts::CallSiteMarker {
                                ports: None,
                                callee: node.label.clone(),
                                start_byte: start,
                                end_byte: end,
                                owner: node.parent.as_deref().and_then(|parent_id| {
                                    let owner = graph.node(parent_id)?;
                                    if owner.kind != NodeKind::Callable {
                                        return None;
                                    }
                                    Some(entmoot_contracts::DefinitionSiteRef {
                                        name: owner.label.clone(),
                                        scope: owner.scope.clone(),
                                        start_byte: owner.start_byte?,
                                    })
                                }),
                            });
                    }
                }
                NodeKind::Resource => {
                    if let (Some(start_byte), Some(end_byte)) = (node.start_byte, node.end_byte) {
                        resource_hits
                            .entry(path.to_string())
                            .or_default()
                            .push(ResourceSpanHit {
                                start_byte,
                                end_byte,
                                node_id: node.id.clone(),
                                resource_id: node.address_id.clone(),
                            });
                    }
                }
                _ => {}
            }
        }
        let paths: HashSet<_> = markers_by_path
            .keys()
            .cloned()
            .chain(sources_map.keys().cloned())
            .collect();
        for path in paths {
            let markers = markers_by_path.get(&path);
            let occ = occ_by_path.get(&path).map(Vec::as_slice).unwrap_or(&[]);
            let source = sources_map.get(&path).map(|index| index.source);
            spans.ingest_file(&path, markers, occ, source);
        }

        let resource_hits_by_path = resource_hits
            .into_iter()
            .map(|(path, mut hits)| {
                hits.sort_by_key(|hit| (hit.start_byte, hit.end_byte, hit.node_id.clone()));
                let ranges = RangeIndex::from_ranges(
                    hits.iter()
                        .enumerate()
                        .map(|(index, hit)| (hit.start_byte, hit.end_byte, index)),
                );
                (path, ResourceSpanData { hits, ranges })
            })
            .collect();

        let digests: HashMap<_, _> = graph
            .snapshot_files
            .iter()
            .map(|f| (f.path.as_str(), f.content_digest.as_str()))
            .collect();
        let verified: HashSet<_> = sources_map
            .iter()
            .filter_map(|(path, source)| {
                (digests.get(path.as_str()).copied()
                    == Some(entmoot_contracts::blake3_hex(source.source.as_bytes()).as_str()))
                .then_some(path.as_str())
            })
            .collect();
        let mut port_targets: HashMap<String, HashMap<(usize, usize), Vec<AttachmentTarget>>> =
            HashMap::new();
        for node in &graph.nodes {
            let Some(path) = node.path.as_deref().filter(|p| verified.contains(p)) else {
                continue;
            };
            let Some(marker) = node.call_site.as_ref().filter(|m| m.validate().is_ok()) else {
                continue;
            };
            let Some(ports) = marker.ports.as_ref() else {
                continue;
            };
            if ports
                .limitations
                .contains(&entmoot_contracts::SiteLimitation::InvalidSyntax)
            {
                continue;
            }
            let mut add = |span: entmoot_contracts::SiteSpan, port| {
                port_targets
                    .entry(path.to_owned())
                    .or_default()
                    .entry((span.start_byte, span.end_byte))
                    .or_default()
                    .push(AttachmentTarget {
                        kind: AttachmentTargetKind::CallSite,
                        node_id: Some(node.id.clone()),
                        address_id: None,
                        port: Some(port),
                    });
            };
            add(ports.callee, entmoot_contracts::CallPort::Callee);
            if let Some(receiver) = ports.receiver {
                add(receiver, entmoot_contracts::CallPort::Receiver);
            }
            for argument in &ports.arguments {
                add(
                    argument.span,
                    entmoot_contracts::CallPort::Argument {
                        position: argument.position,
                    },
                );
            }
            add(ports.result, entmoot_contracts::CallPort::Result);
        }
        for spans in port_targets.values_mut() {
            for targets in spans.values_mut() {
                targets.sort_by_cached_key(|target| {
                    (target.node_id.clone(), format!("{:?}", target.port))
                });
                targets.dedup();
            }
        }

        let verified_sources = verified.into_iter().map(str::to_owned).collect();
        let source_digests = digests
            .into_iter()
            .filter_map(|(path, digest)| {
                entmoot_contracts::ContentDigest::parse(digest)
                    .ok()
                    .map(|digest| (path.to_owned(), digest))
            })
            .collect();
        Self {
            source_digests,
            verified_sources,
            graph,
            sources: sources_map,
            file_ids: graph
                .nodes
                .iter()
                .filter(|node| node.kind == NodeKind::File)
                .filter_map(|node| node.path.as_deref().map(|path| (path, node.id.as_str())))
                .collect(),
            spans,
            resource_hits_by_path,
            port_targets,
        }
    }
}

pub fn resolve_location(
    location: &LocationRecord,
    artifact: &ArtifactManifest,
    indexes: &AttachmentIndexes<'_>,
) -> LocationAttachment {
    let graph_snapshot = &indexes.graph.snapshot;
    let graph_key = graph_snapshot
        .snapshot_digest
        .as_deref()
        .unwrap_or(&graph_snapshot.revision);
    let unresolved = |disposition, freshness, reason| LocationAttachment {
        verified_span: None,
        attachment_id: attachment_id(graph_key, &location.location_id, "none"),
        artifact_id: artifact.artifact_id.clone(),
        location_id: location.location_id.clone(),
        disposition,
        selected_target: None,
        candidate_targets: Vec::new(),
        freshness,
        reason_codes: vec![reason],
        conflicts: Vec::new(),
        location_ref: None,
    };
    if location.artifact_id != artifact.artifact_id {
        return unresolved(
            AttachmentDisposition::Invalid,
            AttachmentFreshness::Current,
            AttachmentReason::ArtifactIdentityMismatch,
        );
    }
    if artifact.verify_against_snapshot(graph_snapshot).is_err() {
        return unresolved(
            AttachmentDisposition::Unresolved,
            AttachmentFreshness::StaleSnapshot,
            AttachmentReason::RevisionMismatch,
        );
    }
    match artifact.path_in_scope(&location.path) {
        Ok(false) => {
            return unresolved(
                AttachmentDisposition::Excluded,
                AttachmentFreshness::Current,
                AttachmentReason::ScopeExcludeMatch,
            )
        }
        Err(_) => {
            return unresolved(
                AttachmentDisposition::Invalid,
                AttachmentFreshness::Current,
                AttachmentReason::RegionInvalid,
            )
        }
        Ok(true) => {}
    }
    let path_key = location.path.replace('\\', "/");
    if !indexes.file_ids.contains_key(location.path.as_str()) {
        return unresolved(
            AttachmentDisposition::Unresolved,
            AttachmentFreshness::Current,
            AttachmentReason::PathMissing,
        );
    }
    if location.extent.is_file() {
        let file_id = crate::graph::file_node_id(&location.path);
        return LocationAttachment {
            verified_span: None,
            attachment_id: attachment_id(graph_key, &location.location_id, &file_id),
            artifact_id: artifact.artifact_id.clone(),
            location_id: location.location_id.clone(),
            disposition: AttachmentDisposition::Attached,
            selected_target: Some(AttachmentTarget {
                port: None,
                kind: AttachmentTargetKind::File,
                node_id: Some(file_id),
                address_id: None,
            }),
            candidate_targets: Vec::new(),
            freshness: AttachmentFreshness::Current,
            reason_codes: vec![AttachmentReason::ExactFile],
            conflicts: Vec::new(),
            location_ref: None,
        };
    }
    let Some(source) = indexes.sources.get(&path_key) else {
        return unresolved(
            AttachmentDisposition::Unresolved,
            AttachmentFreshness::Current,
            AttachmentReason::PathMissing,
        );
    };
    let Some(source_digest) = indexes.source_digests.get(&path_key) else {
        return unresolved(
            AttachmentDisposition::Unresolved,
            AttachmentFreshness::Current,
            AttachmentReason::SourceDigestUnavailable,
        );
    };
    if !indexes.verified_sources.contains(&path_key) {
        return unresolved(
            AttachmentDisposition::Unresolved,
            AttachmentFreshness::StaleSnapshot,
            AttachmentReason::SourceDigestMismatch,
        );
    }
    let span = match location_span(location, source) {
        Ok(span) => span,
        Err(reason) => {
            return unresolved(
                if reason == AttachmentReason::RegionInvalid {
                    AttachmentDisposition::Invalid
                } else {
                    AttachmentDisposition::Unresolved
                },
                AttachmentFreshness::Current,
                reason,
            )
        }
    };
    let start_byte = span.start_byte;
    let end_byte = span.end_byte;
    let path = location.path.replace('\\', "/");
    let call_target = indexes
        .spans
        .resolve_call_overlapping(&path, start_byte, end_byte)
        .and_then(|matched| {
            let node_id =
                crate::graph::call_stub_id(&path, matched.call.start_byte, &matched.call.callee);
            indexes.graph.node(&node_id).map(|node| AttachmentTarget {
                port: None,
                kind: AttachmentTargetKind::CallSite,
                node_id: Some(node.id.clone()),
                address_id: node.address_id.clone(),
            })
        });
    let callable_target = indexes
        .spans
        .resolve_range(&path, start_byte, end_byte)
        .map(|matched| AttachmentTarget {
            port: None,
            kind: AttachmentTargetKind::Callable,
            node_id: Some(matched.node_id),
            address_id: matched.address_id,
        });
    let resource_target = resource_overlap_target(indexes, &path, start_byte, end_byte);
    let (mut target, mut reason) = call_target
        .map(|target| (target, AttachmentReason::ExactRegionOverlap))
        .or_else(|| callable_target.map(|target| (target, AttachmentReason::EnclosingCallableSpan)))
        .or_else(|| resource_target.map(|target| (target, AttachmentReason::EnclosingResourceSpan)))
        .unwrap_or_else(|| {
            (
                AttachmentTarget {
                    port: None,
                    kind: AttachmentTargetKind::File,
                    node_id: Some(crate::graph::file_node_id(&location.path)),
                    address_id: None,
                },
                AttachmentReason::FileFallback,
            )
        });

    let mut candidate_targets = Vec::new();
    let exact_ports = indexes
        .port_targets
        .get(&path)
        .and_then(|spans| spans.get(&(start_byte, end_byte)))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    match exact_ports {
        [port] => {
            target = port.clone();
            reason = AttachmentReason::ExactPortSpan;
        }
        [] => {}
        candidates => {
            candidate_targets = candidates.to_vec();
            target = AttachmentTarget {
                port: None,
                kind: AttachmentTargetKind::File,
                node_id: Some(crate::graph::file_node_id(&path)),
                address_id: None,
            };
            reason = AttachmentReason::AmbiguousCandidates;
        }
    }
    let mut reason_codes = vec![reason];
    let conflicts = Vec::new();

    let mut freshness = AttachmentFreshness::Current;
    let mut location_ref = None;
    if let Some(node_id) = target.node_id.as_ref() {
        if let Some(node) = indexes.graph.node(node_id) {
            if let Some(expected_str) = node.exact_body_id.as_ref() {
                let expected = ExactBodyId::from_trusted(expected_str.clone());
                let loc_ref = LocationRef {
                    repository_id: Some(graph_snapshot.repository_id.clone()),
                    path: location.path.replace('\\', "/"),
                    region: location.extent.region().cloned(),
                    expected_exact_body: expected,
                    logic_address: node.logic_address.clone(),
                    snapshot_revision: Some(graph_snapshot.revision.clone()),
                };
                let current = match (node.start_byte, node.end_byte) {
                    (Some(start), Some(end)) => indexes.sources.get(&path).and_then(|source| {
                        entmoot_core::exact_body_id_of_span(source.source.as_bytes(), start, end)
                    }),
                    _ => None,
                };
                if entmoot_core::verify_location_ref(&loc_ref, current.as_ref())
                    == LocationResolveStatus::Stale
                {
                    freshness = AttachmentFreshness::StaleBody;
                    reason_codes.push(AttachmentReason::ExactBodyMismatch);
                }
                location_ref = Some(loc_ref);
            }
        }
    }

    LocationAttachment {
        verified_span: Some(entmoot_contracts::VerifiedSourceSpan {
            source_digest: source_digest.clone(),
            span,
        }),
        attachment_id: attachment_id(
            graph_key,
            &location.location_id,
            &target.port.as_ref().map_or_else(
                || target.node_id.clone().unwrap_or_else(|| "file".into()),
                |port| format!("{}:{port:?}", target.node_id.as_deref().unwrap_or("file")),
            ),
        ),
        artifact_id: artifact.artifact_id.clone(),
        location_id: location.location_id.clone(),
        disposition: AttachmentDisposition::Attached,
        selected_target: Some(target),
        candidate_targets,
        freshness,
        reason_codes,
        conflicts,
        location_ref,
    }
}

fn resource_overlap_target(
    indexes: &AttachmentIndexes<'_>,
    path: &str,
    start_byte: usize,
    end_byte: usize,
) -> Option<AttachmentTarget> {
    let data = indexes.resource_hits_by_path.get(path)?;
    data.ranges
        .overlapping(start_byte, end_byte)
        .into_iter()
        .filter_map(|index| data.hits.get(index))
        .filter(|hit| hit.start_byte <= start_byte && hit.end_byte >= end_byte)
        .min_by_key(|hit| (hit.end_byte.saturating_sub(hit.start_byte), &hit.node_id))
        .map(|hit| AttachmentTarget {
            port: None,
            kind: AttachmentTargetKind::Resource,
            node_id: Some(hit.node_id.clone()),
            address_id: hit.resource_id.clone(),
        })
}

fn location_span(
    location: &LocationRecord,
    source_index: &SourceTextIndex<'_>,
) -> Result<entmoot_contracts::SiteSpan, AttachmentReason> {
    use entmoot_contracts::{SiteSpan, SourceRegion, SourceTextEnd};
    let region = location
        .extent
        .region()
        .ok_or(AttachmentReason::RegionInvalid)?;
    region
        .validate()
        .map_err(|_| AttachmentReason::RegionInvalid)?;
    let span = match region {
        SourceRegion::Bytes { span } => *span,
        SourceRegion::Text { range, encoding } => {
            let start = coordinate_byte(
                source_index,
                range.start.line,
                range.start.column,
                *encoding,
            )?;
            let end = match range.end {
                SourceTextEnd::Position { position } => {
                    coordinate_byte(source_index, position.line, position.column, *encoding)?
                }
                SourceTextEnd::LineEnd { line } => {
                    let (text, start) = source_index
                        .line(line as usize)
                        .ok_or(AttachmentReason::RegionInvalid)?;
                    start + text.len()
                }
                SourceTextEnd::Unspecified => return Err(AttachmentReason::CoordinateEndUnknown),
            };
            SiteSpan {
                start_byte: start,
                end_byte: end,
            }
        }
    };
    let source = source_index.source;
    if span.start_byte > span.end_byte
        || span.end_byte > source.len()
        || !source.is_char_boundary(span.start_byte)
        || !source.is_char_boundary(span.end_byte)
    {
        return Err(AttachmentReason::RegionInvalid);
    }
    Ok(span)
}

fn coordinate_byte(
    source: &SourceTextIndex<'_>,
    line: u32,
    column: u32,
    encoding: ColumnEncoding,
) -> Result<usize, AttachmentReason> {
    if line == 0 || column == 0 {
        return Err(AttachmentReason::RegionInvalid);
    }
    let (line, line_start) = source
        .line(line as usize)
        .ok_or(AttachmentReason::RegionInvalid)?;
    let codepoint =
        match encoding {
            ColumnEncoding::Utf8CodeUnits => utf8_column_to_codepoint(line, column)
                .map_err(|_| AttachmentReason::RegionInvalid)?,
            ColumnEncoding::Utf16CodeUnits => utf16_column_to_codepoint(line, column)
                .map_err(|_| AttachmentReason::RegionInvalid)?,
            ColumnEncoding::Utf32CodePoints => column,
            // Column one is encoding-independent. Never guess other columns.
            ColumnEncoding::Unknown if column == 1 => 1,
            ColumnEncoding::Unknown => return Err(AttachmentReason::CoordinateEncodingUnknown),
        };
    let byte_in_line = line
        .char_indices()
        .nth((codepoint - 1) as usize)
        .map(|(i, _)| i)
        .or_else(|| (codepoint as usize == line.chars().count() + 1).then_some(line.len()))
        .ok_or(AttachmentReason::RegionInvalid)?;
    Ok(line_start + byte_in_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::TopologyNode;
    use entmoot_contracts::{
        ArtifactFormat, ArtifactProducer, LocationExtent, ScopeCompleteness, ScopeUniverse,
        SourceRegion, SourceSnapshot, SourceState,
    };

    #[test]
    fn coordinate_ranges_preserve_unicode_newlines_bom_and_points() {
        use entmoot_contracts::{SiteSpan, SourcePosition, SourceTextEnd, SourceTextRange};
        let (_, _, mut location, _) = setup();
        let source = SourceTextIndex::new("\u{feff}a😀b\r\nc\r");
        for (encoding, column, expected) in [
            (ColumnEncoding::Utf8CodeUnits, 6, Ok(8)),
            (ColumnEncoding::Utf16CodeUnits, 4, Ok(8)),
            (ColumnEncoding::Utf32CodePoints, 3, Ok(8)),
            (
                ColumnEncoding::Utf8CodeUnits,
                3,
                Err(AttachmentReason::RegionInvalid),
            ),
            (
                ColumnEncoding::Utf16CodeUnits,
                3,
                Err(AttachmentReason::RegionInvalid),
            ),
            (
                ColumnEncoding::Utf32CodePoints,
                0,
                Err(AttachmentReason::RegionInvalid),
            ),
            (
                ColumnEncoding::Unknown,
                3,
                Err(AttachmentReason::CoordinateEncodingUnknown),
            ),
            (ColumnEncoding::Unknown, 1, Ok(3)),
        ] {
            assert_eq!(coordinate_byte(&source, 1, column, encoding), expected);
        }
        for (start, end, span) in [
            (
                SourcePosition { line: 1, column: 1 },
                SourceTextEnd::LineEnd { line: 1 },
                SiteSpan {
                    start_byte: 3,
                    end_byte: 9,
                },
            ),
            (
                SourcePosition { line: 2, column: 1 },
                SourceTextEnd::LineEnd { line: 2 },
                SiteSpan {
                    start_byte: 11,
                    end_byte: 13,
                },
            ),
            (
                SourcePosition { line: 1, column: 3 },
                SourceTextEnd::Position {
                    position: SourcePosition { line: 1, column: 3 },
                },
                SiteSpan {
                    start_byte: 8,
                    end_byte: 8,
                },
            ),
        ] {
            location.extent = LocationExtent::Region {
                region: SourceRegion::Text {
                    range: SourceTextRange { start, end },
                    encoding: ColumnEncoding::Utf32CodePoints,
                },
            };
            assert_eq!(location_span(&location, &source), Ok(span));
        }
        location.extent = LocationExtent::Region {
            region: SourceRegion::Text {
                range: SourceTextRange {
                    start: SourcePosition { line: 1, column: 1 },
                    end: SourceTextEnd::Unspecified,
                },
                encoding: ColumnEncoding::Unknown,
            },
        };
        assert_eq!(
            location_span(&location, &source),
            Err(AttachmentReason::CoordinateEndUnknown)
        );
        for (start_byte, end_byte) in [(5, 6), (3, 14), (9, 3)] {
            location.extent = LocationExtent::Region {
                region: SourceRegion::Bytes {
                    span: SiteSpan {
                        start_byte,
                        end_byte,
                    },
                },
            };
            assert_eq!(
                location_span(&location, &source),
                Err(AttachmentReason::RegionInvalid)
            );
        }
    }

    fn setup() -> (
        TopologyGraph,
        ArtifactManifest,
        LocationRecord,
        Vec<(String, String)>,
    ) {
        let snapshot = SourceSnapshot {
            repository_id: "repo".into(),
            revision: "a".repeat(40),
            source_state: SourceState::Committed,
            snapshot_digest: None,
            estate_id: None,
            subject: None,
            subject_aliases: Vec::new(),
        };
        let scope = ScopeUniverse::from_selectors(
            "test",
            vec![],
            vec!["src/a.rs".into()],
            vec![],
            ScopeCompleteness::Partial,
            None,
        );
        let manifest = ArtifactManifest::build(
            ArtifactFormat::Sarif21,
            ArtifactProducer {
                name: "test".into(),
                version: Some("1".into()),
                information_uri: None,
                ruleset_id: None,
                config_class: None,
            },
            snapshot.clone(),
            scope,
            entmoot_contracts::blake3_hex(b"x"),
            None,
        )
        .unwrap();
        let mut graph = TopologyGraph::new("C:/repo");
        graph.snapshot = snapshot;
        graph.snapshot_files.push(crate::graph::SnapshotFileDigest {
            path: "src/a.rs".into(),
            content_digest: entmoot_contracts::blake3_hex(b"abcdefghijabcdefghij"),
        });
        let mut file = TopologyNode::shell(
            crate::graph::file_node_id("src/a.rs"),
            NodeKind::File,
            "a.rs",
            None,
        );
        file.path = Some("src/a.rs".into());
        graph.nodes.push(file);
        let mut callable = TopologyNode::shell(
            crate::graph::callable_node_id("src/a.rs", "a", 0),
            NodeKind::Callable,
            "a",
            Some(crate::graph::file_node_id("src/a.rs")),
        );
        callable.path = Some("src/a.rs".into());
        callable.start_byte = Some(0);
        callable.end_byte = Some(20);
        graph.nodes.push(callable);
        let mut call = TopologyNode::shell(
            crate::graph::call_stub_id("src/a.rs", 15, "far"),
            NodeKind::CallStub,
            "far",
            Some(crate::graph::callable_node_id("src/a.rs", "a", 0)),
        );
        call.path = Some("src/a.rs".into());
        call.start_byte = Some(15);
        call.end_byte = Some(18);
        graph.nodes.push(call);
        let location = LocationRecord {
            location_id: "loc".into(),
            artifact_id: manifest.artifact_id.clone(),
            path: "src/a.rs".into(),
            extent: LocationExtent::Region {
                region: SourceRegion::Text {
                    range: entmoot_contracts::SourceTextRange {
                        start: entmoot_contracts::SourcePosition { line: 1, column: 2 },
                        end: entmoot_contracts::SourceTextEnd::Position {
                            position: entmoot_contracts::SourcePosition { line: 1, column: 3 },
                        },
                    },
                    encoding: entmoot_contracts::ColumnEncoding::Utf32CodePoints,
                },
            },

            provider_pointer: "/x".into(),
            provider_confidence: None,
        };
        (
            graph,
            manifest,
            location,
            vec![("src/a.rs".into(), "abcdefghijabcdefghij".into())],
        )
    }

    #[test]
    fn precise_location_never_uses_nearby_call() {
        let (graph, manifest, location, sources) = setup();
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(
            attached.selected_target.unwrap().kind,
            AttachmentTargetKind::Callable
        );
    }

    #[test]
    fn valid_location_falls_back_to_file() {
        let (mut graph, manifest, mut location, sources) = setup();
        graph.nodes.retain(|n| n.kind == NodeKind::File);
        location.extent = LocationExtent::Region {
            region: SourceRegion::Bytes {
                span: entmoot_contracts::SiteSpan {
                    start_byte: 9,
                    end_byte: 10,
                },
            },
        };
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(
            attached.selected_target.unwrap().kind,
            AttachmentTargetKind::File
        );
        assert!(attached
            .reason_codes
            .contains(&AttachmentReason::FileFallback));
    }

    #[test]
    fn zero_length_regions_attach_to_the_represented_file_without_inventing_a_span() {
        let (mut graph, manifest, mut location, sources) = setup();
        graph.nodes.retain(|n| n.kind == NodeKind::File);
        let indexes = AttachmentIndexes::new(&graph, &sources);
        for offset in [0u64, 20] {
            location.extent = LocationExtent::Region {
                region: SourceRegion::Bytes {
                    span: entmoot_contracts::SiteSpan {
                        start_byte: usize::try_from(offset).unwrap(),
                        end_byte: usize::try_from(offset).unwrap(),
                    },
                },
            };
            let attached = resolve_location(&location, &manifest, &indexes);
            assert_eq!(attached.disposition, AttachmentDisposition::Attached);
            assert_eq!(
                attached.selected_target.unwrap().kind,
                AttachmentTargetKind::File
            );
        }
        location.extent = LocationExtent::Region {
            region: SourceRegion::Text {
                range: entmoot_contracts::SourceTextRange {
                    start: entmoot_contracts::SourcePosition { line: 1, column: 1 },
                    end: entmoot_contracts::SourceTextEnd::Position {
                        position: entmoot_contracts::SourcePosition { line: 1, column: 1 },
                    },
                },
                encoding: ColumnEncoding::Utf32CodePoints,
            },
        };
        assert_eq!(
            resolve_location(&location, &manifest, &indexes).disposition,
            AttachmentDisposition::Attached
        );
    }

    #[test]
    fn file_grain_attaches_exact_file_without_span() {
        let (graph, manifest, mut location, sources) = setup();
        location.extent = LocationExtent::File;

        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(attached.disposition, AttachmentDisposition::Attached);
        assert_eq!(
            attached.selected_target.unwrap().kind,
            AttachmentTargetKind::File
        );
        assert_eq!(attached.reason_codes, vec![AttachmentReason::ExactFile]);
        assert!(attached.location_ref.is_none());
    }

    #[test]
    fn file_grain_does_not_require_decoded_text_but_region_does() {
        let (graph, manifest, mut location, _) = setup();
        let sources = vec![];
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let region = resolve_location(&location, &manifest, &indexes);
        assert_eq!(region.disposition, AttachmentDisposition::Unresolved);
        location.extent = LocationExtent::File;

        let file = resolve_location(&location, &manifest, &indexes);
        assert_eq!(file.disposition, AttachmentDisposition::Attached);
        assert_eq!(file.reason_codes, vec![AttachmentReason::ExactFile]);
    }

    #[test]
    fn file_grain_path_missing_is_unresolved() {
        let (graph, mut manifest, mut location, sources) = setup();
        manifest.scope.included_paths = vec!["missing.rs".into()];
        location.extent = LocationExtent::File;
        location.path = "missing.rs".into();
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(attached.disposition, AttachmentDisposition::Unresolved);
        assert!(attached
            .reason_codes
            .contains(&AttachmentReason::PathMissing));
    }

    #[test]
    fn excluded_path_is_scope_exclude() {
        let (graph, mut manifest, location, sources) = setup();
        manifest.scope.excluded_paths = vec!["src/a.rs".into()];
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(attached.disposition, AttachmentDisposition::Excluded);
        assert!(attached
            .reason_codes
            .contains(&AttachmentReason::ScopeExcludeMatch));
    }

    #[test]
    fn snapshot_mismatch_is_revision_mismatch() {
        let (graph, mut manifest, location, sources) = setup();
        manifest.snapshot.revision = "b".repeat(40);
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(attached.disposition, AttachmentDisposition::Unresolved);
        assert_eq!(attached.freshness, AttachmentFreshness::StaleSnapshot);
        assert!(attached
            .reason_codes
            .contains(&AttachmentReason::RevisionMismatch));
    }

    #[test]
    fn malformed_region_line_zero_is_invalid() {
        let (graph, manifest, mut location, sources) = setup();
        location.extent = LocationExtent::Region {
            region: SourceRegion::Text {
                range: entmoot_contracts::SourceTextRange {
                    start: entmoot_contracts::SourcePosition { line: 0, column: 1 },
                    end: entmoot_contracts::SourceTextEnd::LineEnd { line: 0 },
                },
                encoding: entmoot_contracts::ColumnEncoding::Utf32CodePoints,
            },
        };
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(attached.disposition, AttachmentDisposition::Invalid);
        assert!(attached
            .reason_codes
            .contains(&AttachmentReason::RegionInvalid));
    }

    #[test]
    fn spans_from_another_file_are_never_candidates() {
        let (mut graph, manifest, location, sources) = setup();
        let mut other = TopologyNode::shell(
            crate::graph::callable_node_id("src/other.rs", "other", 0),
            NodeKind::Callable,
            "other",
            Some(crate::graph::file_node_id("src/other.rs")),
        );
        other.path = Some("src/other.rs".into());
        other.start_byte = Some(0);
        other.end_byte = Some(20);
        graph.nodes.insert(1, other);
        let keep_file = crate::graph::file_node_id("src/a.rs");
        graph
            .nodes
            .retain(|node| node.id == keep_file || node.path.as_deref() == Some("src/other.rs"));
        graph.nodes.retain(|node| {
            node.kind == NodeKind::File
                || (node.kind == NodeKind::Callable && node.path.as_deref() == Some("src/other.rs"))
        });
        let indexes = AttachmentIndexes::new(&graph, &sources);
        let attached = resolve_location(&location, &manifest, &indexes);
        assert_eq!(
            attached.selected_target.unwrap().kind,
            AttachmentTargetKind::File
        );
    }
}
