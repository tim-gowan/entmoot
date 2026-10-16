//! Index emission records for Phase 2 indexer input.

use entmoot_contracts::{CallableBoundaryRecord, DefinitionRecord, FileIndexRecord, OccurrenceRecord};

use crate::granular::GranularHashResult;

/// Build per-file index records from a granular hash result.
pub fn build_file_index(result: &GranularHashResult) -> FileIndexRecord {
    let mut definitions: Vec<DefinitionRecord> = Vec::new();
    let mut seen_bodies = std::collections::HashSet::new();
    let mut occurrences = Vec::new();
    let mut callable_boundaries = Vec::new();

    for callable in result.callables.values() {
        if seen_bodies.insert(callable.structural_hash.clone()) {
            definitions.push(DefinitionRecord {
                structural_hash: callable.structural_hash.clone(),
            });
        }
        occurrences.push(OccurrenceRecord {
            address_id: callable.address_id.clone(),
            logic_address: callable.logic_address.clone(),
            structural_hash: callable.structural_hash.clone(),
            exact_body_id: callable.exact_body_id.clone(),
        });
        callable_boundaries.push(CallableBoundaryRecord {
            address_id: callable.address_id.clone(),
            signature_hash: callable.signature_hash.clone(),
            syntactic_signature: callable.signature.clone(),
        });
    }

    definitions.sort_by(|a, b| a.structural_hash.cmp(&b.structural_hash));
    occurrences.sort_by(|a, b| a.address_id.cmp(&b.address_id));
    callable_boundaries.sort_by(|a, b| a.address_id.cmp(&b.address_id));

    FileIndexRecord {
        policy_id: result.policy_id.clone(),
        file_hash: result.file_hash.clone(),
        module_shell_hash: result.module_shell_hash.clone(),
        definitions,
        occurrences,
        callable_boundaries,
        resolved_identities: Vec::new(),
        markers: result.markers.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::granular::CallableHash;
    use crate::logic_address::LogicKind;
    use entmoot_contracts::{DefMarker, FileMarkers, PolicyId};
    use std::collections::HashMap;

    fn sample_result() -> GranularHashResult {
        let mut callables = HashMap::new();
        callables.insert(
            "a".into(),
            CallableHash {
                name: "a".into(),
                structural_hash: "blake3:aaa".into(),
                exact_body_id: Some("blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into()),
                signature_hash: "blake3:sig1".into(),
                imports: vec![],
                signature: String::new(),
                node_type: "function".into(),
                kind: LogicKind::Function,
                scope: vec![],
                logic_address: "logic|repo|python|a.py|-|-|function|-|-".into(),
                address_id: "addr_a".into(),
                start_byte: 0,
                end_byte: 10,
            },
        );
        callables.insert(
            "b".into(),
            CallableHash {
                name: "b".into(),
                structural_hash: "blake3:aaa".into(),
                exact_body_id: Some("blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into()),
                signature_hash: "blake3:sig1".into(),
                imports: vec![],
                signature: String::new(),
                node_type: "function".into(),
                kind: LogicKind::Function,
                scope: vec![],
                logic_address: "logic|repo|python|b.py|-|-|function|-|-".into(),
                address_id: "addr_b".into(),
                start_byte: 0,
                end_byte: 10,
            },
        );
        GranularHashResult {
            policy_id: PolicyId::from("pol"),
            file_hash: "blake3:file".into(),
            module_shell_hash: "blake3:shell".into(),
            callables,
            imports: vec![],
            markers: None,
        }
    }

    #[test]
    fn dedupes_definitions() {
        let index = build_file_index(&sample_result());
        assert_eq!(index.definitions.len(), 1);
        assert_eq!(index.occurrences.len(), 2);
    }

    #[test]
    fn boundary_data_is_occurrence_scoped() {
        let mut result = sample_result();
        let callable = result.callables.get_mut("b").unwrap();
        callable.signature_hash = "blake3:sig2".into();
        callable.signature = "function_signature(parameter(identifier))".into();

        let index = build_file_index(&result);
        assert_eq!(index.definitions.len(), 1);
        assert!(index
            .callable_boundaries
            .iter()
            .any(|boundary| boundary.signature_hash == "blake3:sig2"
                && boundary.syntactic_signature == "function_signature(parameter(identifier))"));
    }

    #[test]
    fn passes_through_markers() {
        let mut result = sample_result();
        result.markers = Some(FileMarkers {
            definitions: vec![DefMarker {
                name: "a".into(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 0,
                end_byte: 10,
            }],
            ..FileMarkers::new()
        });
        let index = build_file_index(&result);
        assert!(index.markers.is_some());
        assert_eq!(index.markers.as_ref().unwrap().definitions.len(), 1);
    }
}
