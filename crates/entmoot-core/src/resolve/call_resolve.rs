//! Resolve call sites against import bindings and same-file definitions.

use std::collections::HashMap;

use entmoot_contracts::{FileIndexRecord, FileMarkers, ImportKind, ImportMarker};

/// Confidence for a resolved (or unresolved) call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveConfidence {
    /// Call site known; no package/def binding claimed.
    SyntacticOnly,
    /// Callee matched a static import binding.
    ImportLocal,
    /// Callee matched a same-file definition (shadows import).
    SameFileDef,
    /// More than one same-file definition is plausible; no target is selected.
    Ambiguous,
    /// Dynamic / unknown; never invent a package member.
    Unresolved,
}

/// One call site with optional derived FQNs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedCall {
    pub file: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub syntactic_callee: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<entmoot_contracts::DefinitionSiteRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_fqn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_fqn: Option<String>,
    pub confidence: ResolveConfidence,
}

/// Build occurrence-style FQN: `path::symbol` or `path::scope.joined.symbol`.
pub fn build_occurrence_fqn(file_path: &str, scope: &[String], symbol: &str) -> Option<String> {
    if symbol.is_empty() {
        return None;
    }
    if scope.is_empty() {
        Some(format!("{file_path}::{symbol}"))
    } else {
        Some(format!("{file_path}::{}.{symbol}", scope.join(".")))
    }
}

/// Resolve all call sites in a single file index record.
pub fn resolve_file_calls(file: &str, record: &FileIndexRecord) -> Vec<ResolvedCall> {
    let Some(markers) = record.markers.as_ref() else {
        return Vec::new();
    };
    if matches!(
        std::path::Path::new(file).extension().and_then(|e| e.to_str()),
        Some("c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx")
    ) {
        return markers
            .call_sites
            .iter()
            .map(|call| ResolvedCall {
                file: file.into(),
                start_byte: call.start_byte,
                end_byte: call.end_byte,
                syntactic_callee: call.callee.clone(),
                owner: call.owner.clone(),
                occurrence_fqn: call
                    .owner
                    .as_ref()
                    .and_then(|owner| build_occurrence_fqn(file, &owner.scope, &owner.name)),
                resolved_fqn: None,
                confidence: ResolveConfidence::SyntacticOnly,
            })
            .collect();
    }
    resolve_markers(file, markers)
}

/// Resolve call sites across many files (no cross-file typed resolution).
pub fn resolve_repo_calls(files: &[(&str, &FileIndexRecord)]) -> Vec<ResolvedCall> {
    let mut resolved: Vec<_> = files
        .iter()
        .flat_map(|(path, record)| resolve_file_calls(path, record))
        .collect();
    resolved.sort_by(|left, right| {
        (&left.file, left.start_byte, left.end_byte, &left.syntactic_callee).cmp(&(
            &right.file,
            right.start_byte,
            right.end_byte,
            &right.syntactic_callee,
        ))
    });
    resolved
}

fn resolve_markers(file: &str, markers: &FileMarkers) -> Vec<ResolvedCall> {
    let bindings = build_import_bindings(&markers.imports);
    let has_star_import = markers.imports.iter().any(|import| import.kind == ImportKind::Star);
    let definitions = DefinitionIndexes::new(&markers.definitions);

    markers
        .call_sites
        .iter()
        .map(|call| resolve_one(file, call, &bindings, &markers.bindings, has_star_import, &definitions))
        .collect()
}

fn resolve_one(
    file: &str,
    call: &entmoot_contracts::CallSiteMarker,
    bindings: &HashMap<String, ImportBindingState>,
    lexical_bindings: &[entmoot_contracts::BindingMarker],
    has_star_import: bool,
    definitions: &DefinitionIndexes<'_>,
) -> ResolvedCall {
    let owner_definition = call.owner.as_ref().and_then(|owner| {
        definitions
            .by_site
            .get(&(owner.name.as_str(), owner.scope.as_slice(), owner.start_byte))
            .copied()
    });
    let validated_owner = call.owner.as_ref().filter(|_| owner_definition.is_some());
    let occurrence_fqn = validated_owner.and_then(|owner| build_occurrence_fqn(file, &owner.scope, &owner.name));

    let (head, rest) = split_callee(&call.callee);
    if head.is_empty() || call.callee.starts_with("<oversized-callee:") {
        return ResolvedCall {
            file: file.to_string(),
            start_byte: call.start_byte,
            end_byte: call.end_byte,
            syntactic_callee: call.callee.clone(),
            owner: call.owner.clone(),
            occurrence_fqn,
            resolved_fqn: None,
            confidence: ResolveConfidence::Unresolved,
        };
    }

    // A lexical binding with the same local name can replace either an imported
    // symbol or a same-file definition. Mark the call open instead of emitting
    // a false edge. Binding markers do not yet carry owner identity, so constrain
    // callable-local checks to the validated owner's source extent.
    let shadowed = lexical_bindings.iter().any(|binding| {
        if binding.name != head {
            return false;
        }
        match (owner_definition, binding.start_byte) {
            (Some(owner), Some(start)) => start >= owner.start_byte && start <= owner.end_byte,
            (Some(_), None) => true,
            (None, Some(start)) => start <= call.start_byte,
            (None, None) => true,
        }
    });
    if shadowed {
        return ResolvedCall {
            file: file.to_string(),
            start_byte: call.start_byte,
            end_byte: call.end_byte,
            syntactic_callee: call.callee.clone(),
            owner: call.owner.clone(),
            occurrence_fqn,
            resolved_fqn: None,
            confidence: ResolveConfidence::Unresolved,
        };
    }

    // Same-file def shadows import (exact name or head of qualified).
    if let Some(name_binding) = definitions.by_name.get(head.as_str()) {
        let owner_scope = match (&call.owner, validated_owner) {
            (None, _) => Some([].as_slice()),
            (Some(_), Some(owner)) => Some(owner.scope.as_slice()),
            (Some(_), None) => None,
        };
        let selected = name_binding.unique().or_else(|| {
            owner_scope
                .and_then(|scope| definitions.by_name_scope.get(&(head.as_str(), scope)))
                .and_then(DefinitionBinding::unique)
        });
        let Some(def) = selected else {
            return ResolvedCall {
                file: file.to_string(),
                start_byte: call.start_byte,
                end_byte: call.end_byte,
                syntactic_callee: call.callee.clone(),
                owner: call.owner.clone(),
                occurrence_fqn,
                resolved_fqn: None,
                confidence: ResolveConfidence::Ambiguous,
            };
        };
        let resolved = if rest.is_empty() {
            build_occurrence_fqn(file, &def.scope, &def.name)
        } else {
            build_occurrence_fqn(file, &def.scope, &def.name).map(|base| format!("{base}.{rest}"))
        };
        return ResolvedCall {
            file: file.to_string(),
            start_byte: call.start_byte,
            end_byte: call.end_byte,
            syntactic_callee: call.callee.clone(),
            owner: call.owner.clone(),
            occurrence_fqn,
            resolved_fqn: resolved,
            confidence: ResolveConfidence::SameFileDef,
        };
    }

    if let Some(binding_state) = bindings.get(&head) {
        let ImportBindingState::Unique(binding) = binding_state else {
            return ResolvedCall {
                file: file.to_string(),
                start_byte: call.start_byte,
                end_byte: call.end_byte,
                syntactic_callee: call.callee.clone(),
                owner: call.owner.clone(),
                occurrence_fqn,
                resolved_fqn: None,
                confidence: ResolveConfidence::Ambiguous,
            };
        };
        if binding.star {
            // Star / glob: never invent a member FQN.
            return ResolvedCall {
                file: file.to_string(),
                start_byte: call.start_byte,
                end_byte: call.end_byte,
                syntactic_callee: call.callee.clone(),
                owner: call.owner.clone(),
                occurrence_fqn,
                resolved_fqn: None,
                confidence: ResolveConfidence::SyntacticOnly,
            };
        }
        let resolved = compose_import_fqn(binding, &rest);
        let confidence = if resolved.is_some() {
            ResolveConfidence::ImportLocal
        } else {
            ResolveConfidence::SyntacticOnly
        };
        return ResolvedCall {
            file: file.to_string(),
            start_byte: call.start_byte,
            end_byte: call.end_byte,
            syntactic_callee: call.callee.clone(),
            owner: call.owner.clone(),
            occurrence_fqn,
            resolved_fqn: resolved,
            confidence,
        };
    }

    // Qualified callee with no local binding for head — keep syntactic only.
    if !rest.is_empty() || call.callee.contains('.') || call.callee.contains("::") {
        return ResolvedCall {
            file: file.to_string(),
            start_byte: call.start_byte,
            end_byte: call.end_byte,
            syntactic_callee: call.callee.clone(),
            owner: call.owner.clone(),
            occurrence_fqn,
            resolved_fqn: None,
            confidence: ResolveConfidence::SyntacticOnly,
        };
    }

    ResolvedCall {
        file: file.to_string(),
        start_byte: call.start_byte,
        end_byte: call.end_byte,
        syntactic_callee: call.callee.clone(),
        owner: call.owner.clone(),
        occurrence_fqn,
        resolved_fqn: None,
        confidence: if has_star_import {
            ResolveConfidence::Ambiguous
        } else {
            ResolveConfidence::Unresolved
        },
    }
}

struct DefinitionIndexes<'a> {
    by_name: HashMap<&'a str, DefinitionBinding<'a>>,
    by_name_scope: HashMap<(&'a str, &'a [String]), DefinitionBinding<'a>>,
    by_site: HashMap<(&'a str, &'a [String], usize), &'a entmoot_contracts::DefMarker>,
}

impl<'a> DefinitionIndexes<'a> {
    fn new(definitions: &'a [entmoot_contracts::DefMarker]) -> Self {
        let mut indexes = Self {
            by_name: HashMap::new(),
            by_name_scope: HashMap::new(),
            by_site: HashMap::new(),
        };
        for definition in definitions {
            insert_definition(&mut indexes.by_name, definition.name.as_str(), definition);
            insert_definition(
                &mut indexes.by_name_scope,
                (definition.name.as_str(), definition.scope.as_slice()),
                definition,
            );
            indexes.by_site.insert(
                (
                    definition.name.as_str(),
                    definition.scope.as_slice(),
                    definition.start_byte,
                ),
                definition,
            );
        }
        indexes
    }
}

#[derive(Debug, Clone, Copy)]
enum DefinitionBinding<'a> {
    Unique(&'a entmoot_contracts::DefMarker),
    Ambiguous,
}

impl<'a> DefinitionBinding<'a> {
    fn unique(&self) -> Option<&'a entmoot_contracts::DefMarker> {
        match self {
            Self::Unique(definition) => Some(definition),
            Self::Ambiguous => None,
        }
    }
}

fn insert_definition<'a, K>(
    index: &mut HashMap<K, DefinitionBinding<'a>>,
    key: K,
    definition: &'a entmoot_contracts::DefMarker,
) where
    K: std::hash::Hash + Eq,
{
    use std::collections::hash_map::Entry;

    match index.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(DefinitionBinding::Unique(definition));
        }
        Entry::Occupied(mut entry) => {
            entry.insert(DefinitionBinding::Ambiguous);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportBinding {
    /// Provider-independent semantic target named by the import.
    target: String,
    /// If true, this binding came from a star/glob import — do not expand members.
    star: bool,
    /// An unaliased dotted module binds its first namespace segment. Resolution
    /// must consume the already-imported tail instead of appending it twice.
    namespace_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImportBindingState {
    Unique(ImportBinding),
    Ambiguous,
}

fn build_import_bindings(imports: &[ImportMarker]) -> HashMap<String, ImportBindingState> {
    let mut map = HashMap::new();
    for imp in imports {
        match imp.kind {
            ImportKind::Star => {
                // Local names under star are unknown; record module only if alias present (rare).
                if let Some(alias) = &imp.alias {
                    insert_import_binding(
                        &mut map,
                        alias.clone(),
                        ImportBinding {
                            target: imp.path.clone(),
                            star: true,
                            namespace_prefix: None,
                        },
                    );
                }
                // Also map empty sentinel? Skip — callees won't match module path as head usually.
            }
            ImportKind::Alias => {
                if let Some(alias) = &imp.alias {
                    insert_import_binding(
                        &mut map,
                        alias.clone(),
                        ImportBinding {
                            target: imp.path.clone(),
                            star: false,
                            namespace_prefix: None,
                        },
                    );
                }
            }
            ImportKind::Named | ImportKind::Static => {
                let local = imp.alias.clone().unwrap_or_else(|| last_segment(&imp.path).to_string());
                insert_import_binding(
                    &mut map,
                    local,
                    ImportBinding {
                        target: imp.path.clone(),
                        star: false,
                        namespace_prefix: None,
                    },
                );
            }
            ImportKind::Module => {
                let local = imp.alias.clone().unwrap_or_else(|| last_segment(&imp.path).to_string());
                let namespace_prefix = imp
                    .path
                    .strip_prefix(&local)
                    .and_then(|tail| tail.strip_prefix('.'))
                    .map(str::to_string);
                insert_import_binding(
                    &mut map,
                    local,
                    ImportBinding {
                        target: imp.path.clone(),
                        star: false,
                        namespace_prefix,
                    },
                );
            }
        }
    }
    map
}

fn insert_import_binding(bindings: &mut HashMap<String, ImportBindingState>, local: String, binding: ImportBinding) {
    bindings
        .entry(local)
        .and_modify(|state| {
            if matches!(state, ImportBindingState::Unique(existing) if existing != &binding) {
                *state = ImportBindingState::Ambiguous;
            }
        })
        .or_insert(ImportBindingState::Unique(binding));
}

fn compose_import_fqn(binding: &ImportBinding, rest: &str) -> Option<String> {
    if let Some(imported_tail) = binding.namespace_prefix.as_deref() {
        if rest == imported_tail {
            return Some(binding.target.clone());
        }
        let suffix = rest.strip_prefix(imported_tail)?.strip_prefix('.')?;
        return Some(format!("{}.{}", binding.target, suffix));
    }

    let base = binding.target.clone();
    if rest.is_empty() {
        Some(base)
    } else {
        Some(format!("{base}.{rest}"))
    }
}

fn split_callee(callee: &str) -> (String, String) {
    // Prefer :: then .
    if let Some(idx) = callee.find("::") {
        let head = callee[..idx].to_string();
        let rest = callee[idx + 2..].to_string();
        return (head, rest);
    }
    if let Some(idx) = callee.find('.') {
        let head = callee[..idx].to_string();
        let rest = callee[idx + 1..].to_string();
        return (head, rest);
    }
    (callee.to_string(), String::new())
}

fn last_segment(path: &str) -> &str {
    path.rsplit(['.', '/', ':'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_file_index, StructuralHasher};
    use entmoot_contracts::{CallSiteMarker, DefMarker, FileIndexRecord, FileMarkers, ImportMarker};

    fn empty_record(markers: FileMarkers) -> FileIndexRecord {
        FileIndexRecord {
            policy_id: "test".into(),
            file_hash: String::new(),
            module_shell_hash: String::new(),
            definitions: vec![],
            occurrences: vec![],
            callable_boundaries: vec![],
            resolved_identities: vec![],
            markers: Some(markers),
        }
    }

    #[test]
    fn import_local_named_binding() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "django.http.HttpResponse".into(),
                alias: Some("HttpResponse".into()),
                kind: ImportKind::Named,
                start_byte: 0,
                end_byte: 40,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "HttpResponse".into(),
                start_byte: 50,
                end_byte: 62,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].confidence, ResolveConfidence::ImportLocal);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("django.http.HttpResponse"));
    }

    #[test]
    fn same_file_def_shadows_import() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "other".into(),
                alias: Some("helper".into()),
                kind: ImportKind::Named,
                start_byte: 0,
                end_byte: 20,
            }],
            definitions: vec![DefMarker {
                name: "helper".into(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 21,
                end_byte: 80,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "helper".into(),
                start_byte: 40,
                end_byte: 46,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::SameFileDef);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("a.py::helper"));
    }

    #[test]
    fn lexical_binding_shadows_import_inside_owning_callable() {
        let owner = DefMarker {
            name: "main".into(),
            kind: "function".into(),
            scope: vec![],
            start_byte: 20,
            end_byte: 100,
        };
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "package.helper".into(),
                alias: Some("helper".into()),
                kind: ImportKind::Named,
                start_byte: 0,
                end_byte: 18,
            }],
            definitions: vec![owner.clone()],
            bindings: vec![entmoot_contracts::BindingMarker {
                name: "helper".into(),
                normalized: Some("v0".into()),
                binding_kind: entmoot_contracts::BindingKind::Local,
                start_byte: Some(40),
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "helper".into(),
                start_byte: 60,
                end_byte: 68,
                owner: Some(entmoot_contracts::DefinitionSiteRef {
                    name: owner.name,
                    scope: owner.scope,
                    start_byte: owner.start_byte,
                }),
            }],
            ..FileMarkers::new()
        };

        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Unresolved);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn python_visitor_binding_prevents_false_import_edge() {
        let source = "from package import helper\n\ndef main():\n    helper = lambda: None\n    helper()\n";
        let analysis = StructuralHasher::new()
            .compute_granular_hash_with_analysis_at(source, "python", Some("a.py"), Some("repo"), None)
            .unwrap();
        let record = build_file_index(&analysis);

        let resolved = resolve_file_calls("a.py", &record);
        let helper = resolved.iter().find(|call| call.syntactic_callee == "helper").unwrap();
        assert_eq!(helper.confidence, ResolveConfidence::Unresolved);
        assert!(helper.resolved_fqn.is_none());
    }

    #[test]
    fn star_import_keeps_unknown_unqualified_call_ambiguous() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "package.exports".into(),
                alias: None,
                kind: ImportKind::Star,
                start_byte: 0,
                end_byte: 29,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "possibly_exported".into(),
                start_byte: 40,
                end_byte: 59,
                owner: None,
            }],
            ..FileMarkers::new()
        };

        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Ambiguous);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn unresolved_unknown_callee() {
        let markers = FileMarkers {
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "mystery".into(),
                start_byte: 0,
                end_byte: 7,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Unresolved);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn oversized_digest_label_never_matches_a_definition() {
        let label = format!("<oversized-callee:{}>", entmoot_contracts::blake3_hex(b"expression"));
        let markers = FileMarkers {
            definitions: vec![DefMarker {
                name: label.clone(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 0,
                end_byte: 5,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: label,
                start_byte: 6,
                end_byte: 10,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("bundle.js", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Unresolved);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn predeclared_spelling_is_not_intrinsic_without_binding_evidence() {
        let markers = FileMarkers {
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "len".into(),
                start_byte: 0,
                end_byte: 6,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Unresolved);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn same_file_definition_can_shadow_a_predeclared_spelling() {
        let markers = FileMarkers {
            definitions: vec![DefMarker {
                name: "len".into(),
                kind: "function".into(),
                scope: vec![],
                start_byte: 0,
                end_byte: 10,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "len".into(),
                start_byte: 20,
                end_byte: 26,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::SameFileDef);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("a.py::len"));
    }

    #[test]
    fn star_import_makes_unqualified_call_ambiguous() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "m".into(),
                alias: None,
                kind: ImportKind::Star,
                start_byte: 0,
                end_byte: 15,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "anything".into(),
                start_byte: 20,
                end_byte: 28,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Ambiguous);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn js_named_alias_binding() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "pkg.a".into(),
                alias: Some("b".into()),
                kind: ImportKind::Named,
                start_byte: 0,
                end_byte: 30,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "b".into(),
                start_byte: 40,
                end_byte: 41,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.js", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::ImportLocal);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("pkg.a"));
    }

    #[test]
    fn java_static_import() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "pkg.Util.method".into(),
                alias: None,
                kind: ImportKind::Static,
                start_byte: 0,
                end_byte: 30,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "method".into(),
                start_byte: 40,
                end_byte: 46,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("A.java", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::ImportLocal);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("pkg.Util.method"));
    }

    #[test]
    fn go_alias_selector() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "fmt".into(),
                alias: Some("alias".into()),
                kind: ImportKind::Alias,
                start_byte: 0,
                end_byte: 20,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "alias.Println".into(),
                start_byte: 30,
                end_byte: 43,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.go", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::ImportLocal);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("fmt.Println"));
    }

    #[test]
    fn rust_alias_use() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "foo::bar".into(),
                alias: Some("Baz".into()),
                kind: ImportKind::Alias,
                start_byte: 0,
                end_byte: 25,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "Baz".into(),
                start_byte: 30,
                end_byte: 33,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("a.rs", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::ImportLocal);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("foo::bar"));
    }

    #[test]
    fn csharp_using_alias() {
        let markers = FileMarkers {
            imports: vec![ImportMarker {
                path: "System.IO".into(),
                alias: Some("X".into()),
                kind: ImportKind::Alias,
                start_byte: 0,
                end_byte: 22,
            }],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "X".into(),
                start_byte: 30,
                end_byte: 31,
                owner: None,
            }],
            ..FileMarkers::new()
        };
        let resolved = resolve_file_calls("A.cs", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::ImportLocal);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("System.IO"));
    }

    #[test]
    fn duplicate_same_file_definitions_do_not_resolve_by_ingestion_order() {
        let definitions = vec![
            DefMarker {
                name: "helper".into(),
                kind: "method".into(),
                scope: vec!["First".into()],
                start_byte: 0,
                end_byte: 40,
            },
            DefMarker {
                name: "helper".into(),
                kind: "method".into(),
                scope: vec!["Second".into()],
                start_byte: 50,
                end_byte: 90,
            },
        ];
        let markers = FileMarkers {
            definitions,
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "helper".into(),
                start_byte: 100,
                end_byte: 108,
                owner: None,
            }],
            ..FileMarkers::new()
        };

        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Ambiguous);
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn visitor_owner_scope_disambiguates_same_named_definitions() {
        let markers = FileMarkers {
            definitions: vec![
                DefMarker {
                    name: "helper".into(),
                    kind: "method".into(),
                    scope: vec!["First".into()],
                    start_byte: 0,
                    end_byte: 40,
                },
                DefMarker {
                    name: "helper".into(),
                    kind: "method".into(),
                    scope: vec!["Second".into()],
                    start_byte: 50,
                    end_byte: 90,
                },
                DefMarker {
                    name: "caller".into(),
                    kind: "method".into(),
                    scope: vec!["Second".into()],
                    start_byte: 91,
                    end_byte: 120,
                },
            ],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "helper".into(),
                start_byte: 100,
                end_byte: 108,
                owner: Some(entmoot_contracts::DefinitionSiteRef {
                    name: "caller".into(),
                    scope: vec!["Second".into()],
                    start_byte: 91,
                }),
            }],
            ..FileMarkers::new()
        };

        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::SameFileDef);
        assert_eq!(resolved[0].resolved_fqn.as_deref(), Some("a.py::Second.helper"));
    }

    #[test]
    fn inconsistent_owner_reference_cannot_disambiguate_definitions() {
        let markers = FileMarkers {
            definitions: vec![
                DefMarker {
                    name: "helper".into(),
                    kind: "method".into(),
                    scope: vec!["First".into()],
                    start_byte: 0,
                    end_byte: 40,
                },
                DefMarker {
                    name: "helper".into(),
                    kind: "method".into(),
                    scope: vec!["Second".into()],
                    start_byte: 50,
                    end_byte: 90,
                },
                DefMarker {
                    name: "caller".into(),
                    kind: "method".into(),
                    scope: vec!["Second".into()],
                    start_byte: 91,
                    end_byte: 120,
                },
            ],
            call_sites: vec![CallSiteMarker {
                ports: None,
                callee: "helper".into(),
                start_byte: 100,
                end_byte: 108,
                owner: Some(entmoot_contracts::DefinitionSiteRef {
                    name: "caller".into(),
                    scope: vec!["Second".into()],
                    start_byte: 999,
                }),
            }],
            ..FileMarkers::new()
        };

        let resolved = resolve_file_calls("a.py", &empty_record(markers));
        assert_eq!(resolved[0].confidence, ResolveConfidence::Ambiguous);
        assert!(resolved[0].occurrence_fqn.is_none());
        assert!(resolved[0].resolved_fqn.is_none());
    }

    #[test]
    fn conflicting_import_bindings_are_ambiguous_in_any_ingestion_order() {
        let imports = vec![
            ImportMarker {
                path: "first.helper".into(),
                alias: Some("helper".into()),
                kind: ImportKind::Named,
                start_byte: 0,
                end_byte: 20,
            },
            ImportMarker {
                path: "second.helper".into(),
                alias: Some("helper".into()),
                kind: ImportKind::Named,
                start_byte: 21,
                end_byte: 42,
            },
        ];
        let resolve = |imports| {
            let markers = FileMarkers {
                imports,
                call_sites: vec![CallSiteMarker {
                    ports: None,
                    callee: "helper".into(),
                    start_byte: 50,
                    end_byte: 56,
                    owner: None,
                }],
                ..FileMarkers::new()
            };
            resolve_file_calls("a.py", &empty_record(markers)).remove(0)
        };

        let forward = resolve(imports.clone());
        let reverse = resolve(imports.into_iter().rev().collect());
        assert_eq!(forward.confidence, ResolveConfidence::Ambiguous);
        assert_eq!(reverse.confidence, ResolveConfidence::Ambiguous);
        assert!(forward.resolved_fqn.is_none());
        assert!(reverse.resolved_fqn.is_none());
    }

    #[test]
    fn repo_resolution_is_sorted_independently_of_input_order() {
        let record = |callee: &str| {
            empty_record(FileMarkers {
                call_sites: vec![CallSiteMarker {
                    ports: None,
                    callee: callee.into(),
                    start_byte: 10,
                    end_byte: 20,
                    owner: None,
                }],
                ..FileMarkers::new()
            })
        };
        let files = [("z.py".to_string(), record("z")), ("a.py".to_string(), record("a"))];
        let file_refs: Vec<_> = files.iter().map(|(path, record)| (path.as_str(), record)).collect();

        let resolved = resolve_repo_calls(&file_refs);
        assert_eq!(
            resolved.iter().map(|call| call.file.as_str()).collect::<Vec<_>>(),
            ["a.py", "z.py"]
        );
    }
}
