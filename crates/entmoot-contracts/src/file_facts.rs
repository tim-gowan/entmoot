//! Canonical syntax facts emitted before any repository graph or binding work.
use crate::{
    CallableBoundaryRecord, DefinitionRecord, FileIndexRecord, FileMarkers, OccurrenceRecord,
    OccurrenceRef, PolicyId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One syntax-owned implementation occurrence. Boundary and content identity
/// share the occurrence key; native evidence is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallableFact {
    pub occurrence: OccurrenceRef,
    pub structural_hash: String,
    pub exact_body_id: Option<String>,
    pub signature_hash: String,
    pub syntactic_signature: String,
}

/// Reusable output of one parse under its source, path, language and policy
/// context. This is syntax evidence, not a claim that references are resolved.
/// A store must bind these facts to exact source bytes and an extraction
/// fingerprint before reuse. Graphs and legacy index records are projections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileFacts {
    pub policy_id: PolicyId,
    pub file_hash: String,
    pub module_shell_hash: String,
    pub callables: Vec<CallableFact>,
    pub markers: Option<FileMarkers>,
}

impl FileFacts {
    /// Admit legacy producer output only when it is a lossless syntax
    /// projection. Native identities and orphan boundary rows are rejected.
    pub fn from_index(record: FileIndexRecord) -> Result<Self, String> {
        if !record.resolved_identities.is_empty() {
            return Err("native evidence cannot enter canonical syntax facts".into());
        }
        let mut boundaries: std::collections::BTreeMap<_, _> = record
            .callable_boundaries
            .iter()
            .map(|boundary| (boundary.address_id.as_str(), boundary))
            .collect();
        if boundaries.len() != record.callable_boundaries.len() {
            return Err("duplicate callable boundaries".into());
        }
        let mut callables = Vec::with_capacity(record.occurrences.len());
        for occurrence in &record.occurrences {
            let boundary = boundaries
                .remove(occurrence.address_id.as_str())
                .ok_or("callable occurrence lacks its boundary")?;
            let fact = CallableFact {
                occurrence: OccurrenceRef {
                    address_id: crate::AddressId::from_trusted(occurrence.address_id.clone()),
                    logic_address: occurrence.logic_address.clone(),
                },
                structural_hash: occurrence.structural_hash.clone(),
                exact_body_id: occurrence.exact_body_id.clone(),
                signature_hash: boundary.signature_hash.clone(),
                syntactic_signature: boundary.syntactic_signature.clone(),
            };
            fact.occurrence.validate()?;
            callables.push(fact);
        }
        if !boundaries.is_empty() {
            return Err("boundary lacks an occurrence".into());
        }
        let facts = Self {
            policy_id: record.policy_id.clone(),
            file_hash: record.file_hash.clone(),
            module_shell_hash: record.module_shell_hash.clone(),
            callables,
            markers: record.markers.clone(),
        };
        if facts.to_index() != record {
            return Err("index is not a canonical syntax projection".into());
        }
        Ok(facts)
    }

    /// Compatibility projection for consumers of the original index interface.
    /// The definition list is derived; it is not another retained representation.
    pub fn to_index(&self) -> FileIndexRecord {
        let mut definitions: Vec<_> = self
            .callables
            .iter()
            .map(|callable| DefinitionRecord {
                structural_hash: callable.structural_hash.clone(),
            })
            .collect();
        definitions.sort_by(|a, b| a.structural_hash.cmp(&b.structural_hash));
        definitions.dedup();
        FileIndexRecord {
            policy_id: self.policy_id.clone(),
            file_hash: self.file_hash.clone(),
            module_shell_hash: self.module_shell_hash.clone(),
            definitions,
            occurrences: self
                .callables
                .iter()
                .map(|callable| OccurrenceRecord {
                    address_id: callable.occurrence.address_id.to_string(),
                    logic_address: callable.occurrence.logic_address.clone(),
                    structural_hash: callable.structural_hash.clone(),
                    exact_body_id: callable.exact_body_id.clone(),
                })
                .collect(),
            callable_boundaries: self
                .callables
                .iter()
                .map(|callable| CallableBoundaryRecord {
                    address_id: callable.occurrence.address_id.to_string(),
                    signature_hash: callable.signature_hash.clone(),
                    syntactic_signature: callable.syntactic_signature.clone(),
                })
                .collect(),
            resolved_identities: Vec::new(),
            markers: self.markers.clone(),
        }
    }
}
