//! Bounded typed batches from an already materialized knowledge projection.
use super::{KnowledgeRepositoryError, OwnerBatch, PublicationBuffer};
use crate::KnowledgeStore;
use entmoot_contracts::{ArtifactManifest, ComponentOccurrence, Proposition};
use std::collections::BTreeMap;

/// Pull iterator used while producers are migrated to direct batch emission.
///
/// It moves records out of the projection and retains only small lookup maps for
/// required local context. It does not make graph extraction itself streaming.
pub struct KnowledgeStoreBatches {
    remaining: KnowledgeStore,
    artifacts: BTreeMap<String, ArtifactManifest>,
    components: BTreeMap<String, ComponentOccurrence>,
    propositions: BTreeMap<String, Proposition>,
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
        let artifacts = remaining
            .artifacts
            .iter()
            .cloned()
            .map(|value| (value.artifact_id.clone(), value))
            .collect();
        let components = remaining
            .components
            .iter()
            .cloned()
            .map(|value| (value.occurrence_id.as_str().to_owned(), value))
            .collect();
        let propositions = remaining
            .propositions
            .iter()
            .cloned()
            .map(|value| (value.id.as_str().to_owned(), value))
            .collect();
        Ok(Self {
            remaining,
            artifacts,
            components,
            propositions,
            buffer,
            phase: 0,
            failed: false,
        })
    }

    fn artifact(&self, id: &str) -> Result<ArtifactManifest, KnowledgeRepositoryError> {
        self.artifacts
            .get(id)
            .cloned()
            .ok_or_else(|| KnowledgeRepositoryError::InvalidRecord {
                collection: "artifacts",
                reason: format!("missing artifact {id}"),
            })
    }

    fn component(&self, id: &str) -> Result<ComponentOccurrence, KnowledgeRepositoryError> {
        self.components
            .get(id)
            .cloned()
            .ok_or_else(|| KnowledgeRepositoryError::InvalidRecord {
                collection: "components",
                reason: format!("missing component {id}"),
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
/// Context-bearing groups remain indivisible and are validated by the publisher.
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
                5 => {
                    if let Some(component) = self.remaining.components.pop() {
                        let artifact = match self.artifact(&component.artifact_id) {
                            Ok(value) => value,
                            Err(error) => {
                                self.failed = true;
                                return Some(Err(error));
                            }
                        };
                        return Some(Ok(OwnerBatch {
                            store: KnowledgeStore {
                                artifacts: vec![artifact],
                                components: vec![component],
                                ..Default::default()
                            },
                            dependencies: vec![],
                        }));
                    }
                    self.phase += 1;
                }
                6 => {
                    if let Some(dependency) = self.remaining.component_dependencies.pop() {
                        let artifact = match self.artifact(&dependency.artifact_id) {
                            Ok(value) => value,
                            Err(error) => {
                                self.failed = true;
                                return Some(Err(error));
                            }
                        };
                        let mut components = BTreeMap::new();
                        for id in [
                            dependency.subject_occurrence_id.as_deref(),
                            dependency.object_occurrence_id.as_deref(),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            match self.component(id) {
                                Ok(value) => {
                                    components.insert(id.to_owned(), value);
                                }
                                Err(error) => {
                                    self.failed = true;
                                    return Some(Err(error));
                                }
                            }
                        }
                        return Some(Ok(OwnerBatch {
                            store: KnowledgeStore {
                                artifacts: vec![artifact],
                                components: components.into_values().collect(),
                                component_dependencies: vec![dependency],
                                ..Default::default()
                            },
                            dependencies: vec![],
                        }));
                    }
                    self.phase += 1;
                }
                7 => {
                    if let Some(vulnerability) = self.remaining.component_vulnerabilities.pop() {
                        let artifact = match self.artifact(&vulnerability.artifact_id) {
                            Ok(value) => value,
                            Err(error) => {
                                self.failed = true;
                                return Some(Err(error));
                            }
                        };
                        let mut components = BTreeMap::new();
                        for id in vulnerability
                            .affects
                            .iter()
                            .filter_map(|affect| affect.component_occurrence_id.as_deref())
                        {
                            match self.component(id) {
                                Ok(value) => {
                                    components.insert(id.to_owned(), value);
                                }
                                Err(error) => {
                                    self.failed = true;
                                    return Some(Err(error));
                                }
                            }
                        }
                        return Some(Ok(OwnerBatch {
                            store: KnowledgeStore {
                                artifacts: vec![artifact],
                                components: components.into_values().collect(),
                                component_vulnerabilities: vec![vulnerability],
                                ..Default::default()
                            },
                            dependencies: vec![],
                        }));
                    }
                    self.phase += 1;
                }
                8 => {
                    if let Some(composition) = self.remaining.bom_compositions.pop() {
                        let artifact = match self.artifact(&composition.artifact_id) {
                            Ok(value) => value,
                            Err(error) => {
                                self.failed = true;
                                return Some(Err(error));
                            }
                        };
                        return Some(Ok(OwnerBatch {
                            store: KnowledgeStore {
                                artifacts: vec![artifact],
                                bom_compositions: vec![composition],
                                ..Default::default()
                            },
                            dependencies: vec![],
                        }));
                    }
                    self.phase += 1;
                }
                9 => plain_batch!(self, locations),
                10 => plain_batch!(self, traces),
                11 => plain_batch!(self, trace_steps),
                12 => plain_batch!(self, attachments),
                13 => plain_batch!(self, assertions),
                14 => plain_batch!(self, resolutions),
                15 => plain_batch!(self, resolver_coverages),
                16 => {
                    if let Some(record) = self.remaining.resolved_identities.pop() {
                        let artifacts = match record
                            .binding
                            .source()
                            .map(|source| source.artifact.artifact_id.as_str())
                        {
                            Some(id) => match self.artifact(id) {
                                Ok(value) => vec![value],
                                Err(error) => {
                                    self.failed = true;
                                    return Some(Err(error));
                                }
                            },
                            None => vec![],
                        };
                        return Some(Ok(OwnerBatch {
                            store: KnowledgeStore {
                                artifacts,
                                resolved_identities: vec![record],
                                ..Default::default()
                            },
                            dependencies: vec![],
                        }));
                    }
                    self.phase += 1;
                }
                17 => {
                    if let Some(edge) = self.remaining.relation_edges.pop() {
                        let proposition = match self.propositions.get(edge.proposition_id.as_str())
                        {
                            Some(value) => value.clone(),
                            None => {
                                self.failed = true;
                                return Some(Err(KnowledgeRepositoryError::InvalidRecord {
                                    collection: "relation_edges",
                                    reason: format!(
                                        "edge {} is missing proposition {}",
                                        edge.id, edge.proposition_id
                                    ),
                                }));
                            }
                        };
                        return Some(Ok(OwnerBatch {
                            store: KnowledgeStore {
                                propositions: vec![proposition],
                                relation_edges: vec![edge],
                                ..Default::default()
                            },
                            dependencies: vec![],
                        }));
                    }
                    self.remaining.propositions.clear();
                    self.phase += 1;
                }
                18 => plain_batch!(self, relation_claims),
                19 => plain_batch!(self, unresolved_relations),
                20 => plain_batch!(self, diagnostics),
                21 => plain_batch!(self, structural_edges),
                22 => plain_batch!(self, call_sites),
                23 => plain_batch!(self, coverages),
                24 => plain_batch!(self, declaration_sites),
                25 => plain_batch!(self, code_contracts),
                _ => return None,
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
