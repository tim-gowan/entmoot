//! Bounded typed batches from an already materialized knowledge projection.
use super::{KnowledgeRepositoryError, OwnerBatch, PublicationBuffer};
use crate::KnowledgeStore;
use std::collections::{BTreeMap, BTreeSet};

/// Pull iterator used while producers are migrated to direct batch emission.
///
/// It moves records out of the projection. The publisher validates references
/// across all batches; no repeated artifact/component/proposition copies are needed.
pub struct KnowledgeStoreBatches {
    remaining: KnowledgeStore,
    buffer: PublicationBuffer,
    phase: u8,
    failed: bool,
}

impl KnowledgeStoreBatches {
    pub fn new(
        remaining: KnowledgeStore,
        buffer: PublicationBuffer,
    ) -> Result<Self, KnowledgeRepositoryError> {
        if buffer.max_records == 0 || buffer.max_bytes == 0 {
            return Err(KnowledgeRepositoryError::BufferLimit(
                "positive batch record and byte limits required",
            ));
        }
        Ok(Self {
            remaining,
            buffer,
            phase: 0,
            failed: false,
        })
    }

    /// Emit graph records without constructing a second complete knowledge view.
    pub fn from_graph(
        graph: &crate::TopologyGraph,
        buffer: PublicationBuffer,
    ) -> Result<GraphBatches<'_>, KnowledgeRepositoryError> {
        Self::new(KnowledgeStore::default(), buffer)?;
        Ok(GraphBatches {
            graph,
            buffer,
            phase: 0,
            cursor: 0,
            current: None,
            failed: false,
            digests: graph
                .snapshot_files
                .iter()
                .map(|file| (file.path.as_str(), &file.content_digest))
                .collect(),
            propositions: BTreeSet::new(),
        })
    }
}

macro_rules! plain_batch {
    ($self:ident, $field:ident) => {{
        if !$self.remaining.$field.is_empty() {
            let take = match bounded_record_count(
                &$self.remaining.$field,
                $self.buffer,
                concat!("single record payload bytes: ", stringify!($field)),
            ) {
                Ok(take) => take,
                Err(error) => {
                    $self.failed = true;
                    return Some(Err(error));
                }
            };
            let at = $self.remaining.$field.len() - take;
            let mut store = KnowledgeStore::default();
            store.$field = $self.remaining.$field.split_off(at);
            return Some(Ok(OwnerBatch {
                store,
                dependencies: vec![],
            }));
        }
        $self.phase += 1;
    }};
}

/// Match the repository's serialized payload budget before grouping plain rows.
/// Schema references are validated over the complete staged publication.
fn bounded_record_count<T: serde::Serialize>(
    values: &[T],
    buffer: PublicationBuffer,
    oversized_message: &'static str,
) -> Result<usize, KnowledgeRepositoryError> {
    let mut bytes = 0usize;
    let mut count = 0usize;
    for value in values.iter().rev().take(buffer.max_records) {
        let size = serde_json::to_vec(value)?.len();
        if size > buffer.max_bytes.saturating_sub(bytes) {
            if count == 0 {
                return Err(KnowledgeRepositoryError::BufferLimit(oversized_message));
            }
            break;
        }
        bytes += size;
        count += 1;
    }
    Ok(count)
}

impl Iterator for KnowledgeStoreBatches {
    type Item = Result<OwnerBatch, KnowledgeRepositoryError>;

    #[allow(clippy::too_many_lines)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            match self.phase {
                0 => plain_batch!(self, input_roots),
                1 => plain_batch!(self, artifacts),
                2 => plain_batch!(self, capabilities),
                3 => plain_batch!(self, findings),
                4 => plain_batch!(self, resources),
                5 => plain_batch!(self, components),
                6 => plain_batch!(self, component_dependencies),
                7 => plain_batch!(self, component_vulnerabilities),
                8 => plain_batch!(self, bom_compositions),
                9 => plain_batch!(self, locations),
                10 => plain_batch!(self, traces),
                11 => plain_batch!(self, trace_steps),
                12 => plain_batch!(self, attachments),
                13 => plain_batch!(self, assertions),
                14 => plain_batch!(self, resolutions),
                15 => plain_batch!(self, resolver_coverages),
                16 => plain_batch!(self, resolved_identities),
                17 => plain_batch!(self, propositions),
                18 => plain_batch!(self, relation_edges),
                19 => plain_batch!(self, relation_claims),
                20 => plain_batch!(self, unresolved_relations),
                21 => plain_batch!(self, diagnostics),
                22 => plain_batch!(self, structural_edges),
                23 => plain_batch!(self, call_sites),
                24 => plain_batch!(self, coverages),
                25 => plain_batch!(self, declaration_sites),
                26 => plain_batch!(self, code_contracts),
                _ => return None,
            }
        }
    }
}

/// Borrowed graph source; only one admitted record chunk is owned at a time.
pub struct GraphBatches<'a> {
    graph: &'a crate::TopologyGraph,
    digests: BTreeMap<&'a str, &'a String>,
    propositions: BTreeSet<&'a str>,
    buffer: PublicationBuffer,
    phase: u8,
    cursor: usize,
    current: Option<KnowledgeStoreBatches>,
    failed: bool,
}

macro_rules! graph_chunk {
    ($self:ident, $field:ident, $target:ident) => {{
        let values = &$self.graph.$field;
        let end = $self
            .cursor
            .saturating_add($self.buffer.max_records.min(1024))
            .min(values.len());
        if $self.cursor < end {
            let mut store = KnowledgeStore::default();
            store.$target = values[$self.cursor..end].to_vec();
            $self.cursor = end;
            return Some(store);
        }
        $self.phase += 1;
        $self.cursor = 0;
    }};
}

impl GraphBatches<'_> {
    fn projection(&mut self) -> Option<KnowledgeStore> {
        loop {
            match self.phase {
                0 => {
                    let end = self.cursor.saturating_add(32).min(self.graph.nodes.len());
                    if self.cursor < end {
                        let store = KnowledgeStore::syntax_from_nodes(
                            self.graph,
                            &self.graph.nodes[self.cursor..end],
                            &self.digests,
                        );
                        self.cursor = end;
                        return Some(store);
                    }
                    self.phase += 1;
                    self.cursor = 0;
                }
                1 => graph_chunk!(self, input_roots, input_roots),
                2 => graph_chunk!(self, artifacts, artifacts),
                3 => graph_chunk!(self, capabilities, capabilities),
                4 => graph_chunk!(self, findings, findings),
                5 => graph_chunk!(self, resources, resources),
                6 => graph_chunk!(self, components, components),
                7 => graph_chunk!(self, component_dependencies, component_dependencies),
                8 => graph_chunk!(self, component_vulnerabilities, component_vulnerabilities),
                9 => graph_chunk!(self, bom_compositions, bom_compositions),
                10 => graph_chunk!(self, locations, locations),
                11 => graph_chunk!(self, traces, traces),
                12 => graph_chunk!(self, trace_steps, trace_steps),
                13 => graph_chunk!(self, attachments, attachments),
                14 => graph_chunk!(self, assertions, assertions),
                15 => graph_chunk!(self, resolutions, resolutions),
                16 => graph_chunk!(self, resolver_coverages, resolver_coverages),
                17 => graph_chunk!(self, resolved_identities, resolved_identities),
                18 => graph_chunk!(self, relation_edges, relation_edges),
                19 => graph_chunk!(self, relation_claims, relation_claims),
                20 => graph_chunk!(self, unresolved_relations, unresolved_relations),
                21 => graph_chunk!(self, diagnostics, diagnostics),
                22 => graph_chunk!(self, coverages, coverages),
                23 => graph_chunk!(self, edges, structural_edges),
                24 => {
                    let mut store = KnowledgeStore::default();
                    while self.cursor < self.graph.relation_edges.len()
                        && store.propositions.len() < self.buffer.max_records.min(1024)
                    {
                        let edge = &self.graph.relation_edges[self.cursor];
                        self.cursor += 1;
                        if self.propositions.insert(edge.proposition_id.as_str()) {
                            store.propositions.push(edge.proposition());
                        }
                    }
                    if !store.propositions.is_empty() {
                        return Some(store);
                    }
                    self.phase += 1;
                }
                _ => return None,
            }
        }
    }
}

impl Iterator for GraphBatches<'_> {
    type Item = Result<OwnerBatch, KnowledgeRepositoryError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(batch) = self.current.as_mut().and_then(Iterator::next) {
                self.failed = batch.is_err();
                return Some(batch);
            }
            let store = self.projection()?;
            match KnowledgeStoreBatches::new(store, self.buffer) {
                Ok(batches) => self.current = Some(batches),
                Err(error) => {
                    self.failed = true;
                    return Some(Err(error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EdgeKind, KnowledgeRead, KnowledgeRepository, PublicationBuffer, SqliteKnowledgeRepository,
        StructuralEdge,
    };

    #[test]
    fn graph_batches_preserve_complete_record_membership_with_single_record_admission() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("entry.py"), "def entmoot_leaf(value):\n    return value + 1\n\ndef entmoot_caller():\n    return entmoot_leaf(1)\n").unwrap();
        let graph = crate::build_topology_from_repo_with_options(
            source.path(),
            crate::TopologyBuildOptions::default(),
        )
        .unwrap();
        let buffer = PublicationBuffer {
            max_records: 1,
            max_bytes: 8 * 1024 * 1024,
        };
        let mut streamed = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let receipt = streamed
            .replace_owner_buffered(
                "fixture/graph",
                &mut KnowledgeStoreBatches::from_graph(&graph, buffer).unwrap(),
                buffer,
            )
            .unwrap();
        assert_eq!(receipt.peak_batch_records, 1);
        let observed = streamed.load().unwrap();
        assert_eq!(observed.code_contracts.len(), 2);
        assert_eq!(observed.call_sites.len(), 1);
        // Aggregate and exact-site edges share a proposition, not an occurrence.
        assert_eq!(observed.relation_edges.len(), 2);
        assert_eq!(observed.propositions.len(), 1);
        assert_eq!(
            observed
                .relation_edges
                .iter()
                .filter(|edge| edge.occurrence_id.is_some())
                .count(),
            1
        );
        assert!(observed
            .relation_edges
            .iter()
            .all(|edge| edge.proposition_id == observed.propositions[0].id));
        let mut materialized = SqliteKnowledgeRepository::open_in_memory().unwrap();
        materialized
            .replace_owner_buffered(
                "fixture/graph",
                &mut KnowledgeStoreBatches::new(KnowledgeStore::extract_from_graph(&graph), buffer)
                    .unwrap(),
                buffer,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&observed).unwrap(),
            serde_json::to_value(materialized.load().unwrap()).unwrap()
        );
        let repeated = streamed
            .replace_owner_buffered(
                "fixture/graph",
                &mut KnowledgeStoreBatches::from_graph(&graph, buffer).unwrap(),
                buffer,
            )
            .unwrap();
        assert!(repeated.outcome.is_unchanged());
    }

    #[test]
    fn materialized_store_is_pulled_in_bounded_batches() {
        let store = KnowledgeStore {
            structural_edges: (0..5)
                .map(|index| {
                    StructuralEdge::structural(
                        format!("from-{index}"),
                        format!("to-{index}"),
                        EdgeKind::Contains,
                    )
                })
                .collect(),
            ..Default::default()
        };
        let mut batches = KnowledgeStoreBatches::new(
            store,
            PublicationBuffer {
                max_records: 2,
                ..PublicationBuffer::default()
            },
        )
        .unwrap();
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let receipt = repository
            .replace_owner_buffered(
                "fixture/snapshot",
                &mut batches,
                PublicationBuffer {
                    max_records: 2,
                    max_bytes: 1024 * 1024,
                },
            )
            .unwrap();
        assert_eq!(receipt.batches, 3);
        assert_eq!(receipt.peak_batch_records, 2);
        assert_eq!(repository.load().unwrap().structural_edges.len(), 5);
    }

    #[test]
    fn zero_batch_limit_is_rejected_before_iteration() {
        for buffer in [
            PublicationBuffer {
                max_records: 0,
                max_bytes: 1,
            },
            PublicationBuffer {
                max_records: 1,
                max_bytes: 0,
            },
        ] {
            assert!(KnowledgeStoreBatches::new(KnowledgeStore::default(), buffer).is_err());
        }
    }

    #[test]
    fn large_records_split_before_publication_byte_limit() {
        let store = KnowledgeStore {
            structural_edges: (0..3)
                .map(|index| {
                    StructuralEdge::structural(
                        format!("{index}-{}", "x".repeat(3_000)),
                        "target",
                        EdgeKind::Contains,
                    )
                })
                .collect(),
            ..Default::default()
        };
        let buffer = PublicationBuffer {
            max_records: 1024,
            max_bytes: 4096,
        };
        let mut batches = KnowledgeStoreBatches::new(store, buffer).unwrap();
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        let receipt = repository
            .replace_owner_buffered("fixture/bytes", &mut batches, buffer)
            .unwrap();
        assert_eq!(receipt.batches, 3);
        assert!(receipt.peak_batch_bytes <= buffer.max_bytes);
        assert_eq!(repository.load().unwrap().structural_edges.len(), 3);
    }

    #[test]
    fn oversized_record_aborts_without_publishing_earlier_batches() {
        let buffer = PublicationBuffer {
            max_records: 1,
            max_bytes: 4096,
        };
        let edge = StructuralEdge::structural("small", "target", EdgeKind::Contains);
        let original = KnowledgeStore {
            structural_edges: vec![edge.clone()],
            ..Default::default()
        };
        let mut repository = SqliteKnowledgeRepository::open_in_memory().unwrap();
        repository
            .replace_owner_buffered(
                "fixture/bytes",
                &mut KnowledgeStoreBatches::new(original, buffer).unwrap(),
                buffer,
            )
            .unwrap();
        let replacement = KnowledgeStore {
            structural_edges: vec![
                StructuralEdge::structural("x".repeat(5000), "target", EdgeKind::Contains),
                StructuralEdge::structural("new", "target", EdgeKind::Contains),
            ],
            ..Default::default()
        };
        let mut batches = KnowledgeStoreBatches::new(replacement, buffer).unwrap();
        assert!(matches!(
            repository.replace_owner_buffered("fixture/bytes", &mut batches, buffer),
            Err(KnowledgeRepositoryError::BufferLimit(
                "single record payload bytes: structural_edges"
            ))
        ));
        assert!(batches.next().is_none());
        assert_eq!(
            serde_json::to_value(repository.load().unwrap().structural_edges).unwrap(),
            serde_json::to_value(vec![edge]).unwrap()
        );
    }
}
