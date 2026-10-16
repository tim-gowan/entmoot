// Shared, independently authored visitor controls. No compiler dependency.
use entmoot_core::{CallableHash, GranularHashResult, StructuralHasher};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub format: String,
    pub repository: String,
    pub pairs: Vec<RevisionPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionPair {
    pub id: String,
    pub language: String,
    pub profile: String,
    pub base: Revision,
    pub head: Revision,
    pub correspondence: Vec<Correspondence>,
    #[serde(default)]
    pub declaration_correspondence: Vec<DeclarationCorrespondence>,
    /// Whole-file syntax expectations include directives outside callables.
    #[serde(default)]
    pub file_structure: Option<Relation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub path: String,
    pub source: String,
    pub parse: ParseOutcome,
    pub occurrences: Vec<Occurrence>,
    pub calls: Vec<Call>,
    /// None leaves declaration inventory unqualified for this fixture.
    #[serde(default)]
    pub declarations: Option<Vec<Declaration>>,
    /// None leaves lexical binding marker coverage unqualified for this fixture.
    #[serde(default)]
    pub bindings: Option<Vec<Binding>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub name: String,
    pub kind: entmoot_core::BindingKind,
    pub start: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub key: String,
    pub name: String,
    pub span: Span,
    pub callable: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationCorrespondence {
    pub base: String,
    pub head: String,
    pub address: Relation,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParseOutcome {
    Accepted,
    Rejected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub key: String,
    pub name: String,
    pub span: Span,
    #[serde(default)]
    pub scope: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub callee: String,
    pub span: Span,
    pub owner: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Correspondence {
    pub base: String,
    pub head: String,
    pub structure: Relation,
    pub address: Relation,
    #[serde(default)]
    pub signature: Option<Relation>,
    #[serde(default)]
    pub exact_body: Option<Relation>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Same,
    Different,
}

pub fn load() -> Corpus {
    let corpus: Corpus = serde_json::from_str(include_str!("../fixtures/revision_pairs.json"))
        .unwrap_or_else(|error| panic!("revision-pair fixture contract: {error}"));
    assert_eq!(corpus.format, "entmoot.visitor_revision_pairs.v1");
    assert!(!corpus.repository.is_empty());
    corpus
}

pub fn inventory(pair: &RevisionPair, revision: &Revision, repository: &str) -> Option<GranularHashResult> {
    assert!(!pair.profile.is_empty(), "{}: missing source profile", pair.id);
    let result = StructuralHasher::new().compute_granular_hash_with_analysis_at(
        &revision.source,
        &pair.language,
        Some(&revision.path),
        Some(repository),
        None,
    );
    if revision.parse == ParseOutcome::Rejected {
        assert!(
            matches!(result, Err(entmoot_core::CoreError::Parse(_))),
            "{}: malformed source must produce a parse failure, not an unsupported/missing implementation",
            pair.id
        );
        assert!(revision.occurrences.is_empty() && revision.calls.is_empty());
        return None;
    }
    let result = result.unwrap_or_else(|e| panic!("{}: {e}", pair.id));
    assert_eq!(
        result.callables.len(),
        revision.occurrences.len(),
        "{}: complete occurrence inventory",
        pair.id
    );
    let mut keys = std::collections::BTreeSet::new();
    let mut identities = std::collections::BTreeSet::new();
    for occurrence in &revision.occurrences {
        assert!(keys.insert(&occurrence.key), "{}: duplicate selector key", pair.id);
        let callable = select(&result, occurrence);
        if let Some(scope) = &occurrence.scope {
            assert_eq!(&callable.scope, scope, "{}: occurrence scope", pair.id);
        }
        assert!(
            revision
                .source
                .get(occurrence.span.start..occurrence.span.end)
                .is_some(),
            "{}: selector must use UTF-8 byte boundaries",
            pair.id
        );
        assert!(
            identities.insert(&callable.address_id),
            "{}: occurrence collision",
            pair.id
        );
        assert_eq!(
            callable.address_id,
            blake3::hash(callable.logic_address.as_bytes()).to_hex().as_str()
        );
    }
    let markers = result
        .markers
        .as_ref()
        .unwrap_or_else(|| panic!("production visitor markers"));
    if let Some(expected) = &revision.bindings {
        assert_eq!(markers.bindings.len(), expected.len(), "{}: binding inventory", pair.id);
        for binding in expected {
            assert_eq!(
                markers
                    .bindings
                    .iter()
                    .filter(|actual| {
                        actual.name == binding.name
                            && actual.binding_kind == binding.kind
                            && actual.start_byte == Some(binding.start)
                    })
                    .count(),
                1,
                "{}: binding selector {binding:?}",
                pair.id
            );
        }
    }
    assert_eq!(
        markers.call_sites.len(),
        revision.calls.len(),
        "{}: call inventory",
        pair.id
    );
    for expected in &revision.calls {
        let sites: Vec<_> = markers
            .call_sites
            .iter()
            .filter(|site| {
                site.callee == expected.callee
                    && site.start_byte == expected.span.start
                    && site.end_byte == expected.span.end
            })
            .collect();
        assert_eq!(sites.len(), 1, "{}: call selector {expected:?}", pair.id);
        let owner = select_key(&result, revision, &expected.owner);
        let actual_owner = sites[0]
            .owner
            .as_ref()
            .unwrap_or_else(|| panic!("callable-scoped call"));
        assert_eq!(actual_owner.start_byte, owner.start_byte);
        assert_eq!(actual_owner.name, owner.name);
        sites[0]
            .validate()
            .unwrap_or_else(|error| panic!("invalid production call ports: {error}"));
    }
    if let Some(declarations) = &revision.declarations {
        assert_eq!(
            markers.declarations.len(),
            declarations.len(),
            "{}: declaration inventory",
            pair.id
        );
        let mut keys = std::collections::BTreeSet::new();
        for selector in declarations {
            assert!(
                keys.insert(&selector.key),
                "{}: duplicate declaration selector",
                pair.id
            );
            let declaration = select_declaration(&result, selector);
            declaration
                .validate()
                .unwrap_or_else(|error| panic!("invalid production declaration: {error}"));
            assert!(revision.source.get(selector.span.start..selector.span.end).is_some());
            match (&selector.callable, &declaration.callable) {
                (Some(key), Some(target)) => {
                    let callable = select_key(&result, revision, key);
                    assert_eq!(target.occurrence.address_id.as_str(), callable.address_id);
                    assert_eq!(target.occurrence.logic_address, callable.logic_address);
                    assert_eq!(target.span.start_byte, callable.start_byte);
                    assert_eq!(target.span.end_byte, callable.end_byte);
                }
                (None, None) => {}
                _ => panic!("{}: declaration callable association {selector:?}", pair.id),
            }
        }
    }
    Some(result)
}

pub fn select_declaration<'a>(
    inventory: &'a GranularHashResult,
    selector: &Declaration,
) -> &'a entmoot_core::DeclarationMarker {
    let matches: Vec<_> = inventory
        .markers
        .as_ref()
        .unwrap_or_else(|| panic!("production declaration markers"))
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.name == selector.name
                && declaration.site.span.start_byte == selector.span.start
                && declaration.site.span.end_byte == selector.span.end
        })
        .collect();
    assert_eq!(matches.len(), 1, "declaration selector {selector:?}");
    matches[0]
}

fn select_declaration_key<'a>(
    inventory: &'a GranularHashResult,
    revision: &Revision,
    key: &str,
) -> &'a entmoot_core::DeclarationMarker {
    let selectors: Vec<_> = revision
        .declarations
        .as_ref()
        .unwrap_or_else(|| panic!("authored declaration selectors"))
        .iter()
        .filter(|selector| selector.key == key)
        .collect();
    assert_eq!(selectors.len(), 1, "missing/duplicate declaration correspondence {key}");
    select_declaration(inventory, selectors[0])
}

pub fn select<'a>(inventory: &'a GranularHashResult, selector: &Occurrence) -> &'a CallableHash {
    let matches: Vec<_> = inventory
        .callables
        .values()
        .filter(|callable| {
            callable.name == selector.name
                && callable.start_byte == selector.span.start
                && callable.end_byte == selector.span.end
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "selector {selector:?}; observed {:?}",
        inventory
            .callables
            .values()
            .map(|c| (&c.name, c.start_byte, c.end_byte))
            .collect::<Vec<_>>()
    );
    matches[0]
}

pub fn select_key<'a>(inventory: &'a GranularHashResult, revision: &Revision, key: &str) -> &'a CallableHash {
    let selectors: Vec<_> = revision.occurrences.iter().filter(|s| s.key == key).collect();
    assert_eq!(selectors.len(), 1, "missing/duplicate correspondence selector {key}");
    select(inventory, selectors[0])
}

pub fn verify(pair: &RevisionPair, repository: &str) {
    let base = inventory(pair, &pair.base, repository);
    let head = inventory(pair, &pair.head, repository);
    if let Some(expected) = &pair.file_structure {
        assert_eq!(
            base.as_ref()
                .unwrap_or_else(|| panic!("accepted base for file relation"))
                .file_hash
                == head
                    .as_ref()
                    .unwrap_or_else(|| panic!("accepted head for file relation"))
                    .file_hash,
            *expected == Relation::Same,
            "{}: whole-file structural correspondence",
            pair.id
        );
    }
    for correspondence in &pair.correspondence {
        let a = select_key(
            base.as_ref()
                .unwrap_or_else(|| panic!("correspondence needs accepted base")),
            &pair.base,
            &correspondence.base,
        );
        let b = select_key(
            head.as_ref()
                .unwrap_or_else(|| panic!("correspondence needs accepted head")),
            &pair.head,
            &correspondence.head,
        );
        assert_eq!(
            a.structural_hash == b.structural_hash,
            correspondence.structure == Relation::Same,
            "{}: structural correspondence",
            pair.id
        );
        if let Some(expected) = &correspondence.signature {
            assert_eq!(
                a.signature_hash == b.signature_hash,
                *expected == Relation::Same,
                "{}: signature correspondence",
                pair.id
            );
        }
        if let Some(expected) = &correspondence.exact_body {
            let (Some(base_id), Some(head_id)) = (&a.exact_body_id, &b.exact_body_id) else {
                panic!(
                    "{}: exact-body relation needs verified source identities on both sides",
                    pair.id
                );
            };
            assert_eq!(
                base_id == head_id,
                *expected == Relation::Same,
                "{}: exact source body correspondence",
                pair.id
            );
        }
        assert_eq!(
            a.address_id == b.address_id,
            correspondence.address == Relation::Same,
            "{}: occurrence correspondence",
            pair.id
        );
        assert_eq!(
            a.logic_address == b.logic_address,
            correspondence.address == Relation::Same,
            "{}: address envelope correspondence",
            pair.id
        );
    }
    for correspondence in &pair.declaration_correspondence {
        // Resolve authored keys independently on each side; never use map order or body hashes.
        let a = select_declaration_key(
            base.as_ref()
                .unwrap_or_else(|| panic!("declaration correspondence needs accepted base")),
            &pair.base,
            &correspondence.base,
        );
        let b = select_declaration_key(
            head.as_ref()
                .unwrap_or_else(|| panic!("declaration correspondence needs accepted head")),
            &pair.head,
            &correspondence.head,
        );
        assert_eq!(
            a.site.occurrence == b.site.occurrence,
            correspondence.address == Relation::Same,
            "{}: declaration correspondence",
            pair.id
        );
    }
}
