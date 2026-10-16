//! Granular hashing with Merkle tree structure for function/method/class level hashing.

use std::collections::HashMap;

use entmoot_contracts::{DeclarationMarker, FileMarkers, OccurrenceRef, PolicyId, SiteSpan, SourceOccurrenceRef};

use crate::callable_context::CallableContext;
use crate::logic_address::{AddressMeta, LogicAddress, LogicKind};

#[derive(Clone)]
pub struct CallableHash {
    pub name: String,
    /// Structural hash of body + imports. Format: `blake3:…` (LogicId / LAH digest half).
    pub structural_hash: String,
    /// Exact observed source bytes for the callable span (`blake3:…`). Distinct from `structural_hash`.
    pub exact_body_id: Option<String>,
    /// Structural hash of signature IR only. Format: `blake3:…`
    pub signature_hash: String,
    pub imports: Vec<String>,
    pub signature: String,
    pub node_type: String,
    pub kind: LogicKind,
    pub scope: Vec<String>,
    /// Logic address envelope (`logic|repository|lang|…`). See `docs/architecture/ARCHITECTURE.md`.
    pub logic_address: String,
    /// BLAKE3 hex of `logic_address` (address id). Not a structural hash.
    pub address_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

pub struct GranularHashResult {
    pub policy_id: PolicyId,
    pub file_hash: String,
    pub module_shell_hash: String,
    pub callables: HashMap<String, CallableHash>,
    pub imports: Vec<String>,
    /// Analysis markers from the unified AST walk (`None` on default hash-only API).
    pub markers: Option<FileMarkers>,
}

/// BLAKE3 of full canonical IR from granular serialization.
pub fn compute_module_shell_hash(serialized_ir: &str) -> String {
    format!("blake3:{}", blake3::hash(serialized_ir.as_bytes()).to_hex())
}

fn merkle_hex_layer(hash: &str) -> String {
    hash.strip_prefix("blake3:").unwrap_or(hash).to_string()
}

/// Assign logic addresses on callables: prefer span-free; add bytes when address collides.
pub fn seal_callable_addresses(callables: &mut [CallableContext], meta: &AddressMeta) {
    for ctx in callables.iter_mut() {
        ctx.seal_address(meta, false);
    }

    let mut seen: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, ctx) in callables.iter().enumerate() {
        seen.entry(ctx.logic_address.clone()).or_default().push(i);
    }
    for indices in seen.values() {
        if indices.len() > 1 {
            for &i in indices {
                callables[i].seal_address(meta, true);
            }
        }
    }
}

/// Seal syntax-owned declaration identities and connect assigned bindings to
/// callable occurrences produced by the same AST walk.
pub fn seal_declaration_addresses(
    unsealed: crate::analysis::context::UnsealedMarkers,
    callables: &[CallableHash],
    meta: &AddressMeta,
) -> Result<FileMarkers, String> {
    let crate::analysis::context::UnsealedMarkers {
        mut markers,
        declarations,
    } = unsealed;
    if !markers.declarations.is_empty() {
        return Err("portable declarations cannot enter the unsealed visitor output".into());
    }
    let mut callable_by_site = HashMap::new();
    for callable in callables {
        let key = (callable.name.as_str(), callable.start_byte, callable.scope.as_slice());
        if callable_by_site.insert(key, callable).is_some() {
            return Err("ambiguous callable occurrence while sealing declarations".into());
        }
    }
    let mut canonical = Vec::with_capacity(declarations.len());
    for declaration in declarations.iter() {
        let kind = LogicKind::parse(&declaration.kind)
            .ok_or_else(|| format!("unsupported declaration kind `{}`", declaration.kind))?;
        canonical.push(LogicAddress::new(
            meta,
            declaration.scope.clone(),
            declaration.name.clone(),
            kind,
            None,
            None,
        ));
    }
    let mut counts = HashMap::<String, usize>::new();
    for address in &canonical {
        *counts.entry(address.canonical()).or_default() += 1;
    }
    let mut sealed_addresses = std::collections::HashSet::new();
    for (declaration, address) in declarations.into_iter().zip(canonical) {
        let mut address = address;
        if counts.get(&address.canonical()).copied().unwrap_or_default() > 1 {
            address = address.with_span(declaration.span.start_byte, declaration.span.end_byte);
        }
        let callable = declaration
            .callable_start_byte
            .map(|callable_start| {
                let callable = callable_by_site
                    .get(&(declaration.name.as_str(), callable_start, declaration.scope.as_slice()))
                    .ok_or_else(|| {
                        format!(
                            "declaration `{}` has no callable at byte {}",
                            declaration.name, callable_start
                        )
                    })?;
                let occurrence = OccurrenceRef {
                    address_id: entmoot_contracts::AddressId::from_trusted(callable.address_id.clone()),
                    logic_address: callable.logic_address.clone(),
                };
                occurrence.validate()?;
                Ok::<_, String>(SourceOccurrenceRef {
                    occurrence,
                    span: SiteSpan {
                        start_byte: callable.start_byte,
                        end_byte: callable.end_byte,
                    },
                })
            })
            .transpose()?;
        let marker = DeclarationMarker {
            name: declaration.name,
            kind: declaration.kind,
            scope: declaration.scope,
            syntax: declaration.syntax,
            site: SourceOccurrenceRef {
                occurrence: OccurrenceRef::new(address.canonical())?,
                span: declaration.span,
            },
            callable,
        };
        marker.validate()?;
        if !sealed_addresses.insert(marker.site.occurrence.address_id.clone()) {
            return Err("ambiguous declaration occurrence after span disambiguation".into());
        }
        markers.declarations.push(marker);
    }
    Ok(markers)
}

/// Build per-callable map and full Merkle input list from visitor contexts.
///
/// Duplicate declared names keep the first map key bare; later siblings use `{name}@{start_byte}`.
/// Addresses are assigned using `meta` (language is required).
/// When `source` is provided, each callable gets an ExactBodyId from its sealed byte span.
pub fn build_granular_callables(
    mut callables_contexts: Vec<CallableContext>,
    meta: &AddressMeta,
    source: Option<&[u8]>,
) -> (HashMap<String, CallableHash>, Vec<CallableHash>) {
    seal_callable_addresses(&mut callables_contexts, meta);

    let mut callable_list = Vec::with_capacity(callables_contexts.len());
    let mut callables_dict = HashMap::new();
    let mut seen_names = HashMap::<String, bool>::new();

    for ctx in callables_contexts {
        let exact_body_id = source.and_then(|bytes| {
            crate::location_verify::exact_body_id_of_span(bytes, ctx.start_byte, ctx.end_byte).map(|id| id.into_inner())
        });
        let start_byte = ctx.start_byte;
        let end_byte = ctx.end_byte;
        let callable_hash = CallableHash {
            name: ctx.name.clone(),
            structural_hash: ctx.structural_hash,
            exact_body_id,
            signature_hash: ctx.signature_hash,
            imports: ctx.imports.into_iter().collect(),
            signature: ctx.signature,
            node_type: ctx.node_type,
            kind: ctx.kind,
            scope: ctx.scope,
            logic_address: ctx.logic_address,
            address_id: ctx.address_id,
            start_byte,
            end_byte,
        };
        callable_list.push(callable_hash.clone());

        let key = if seen_names.contains_key(&ctx.name) {
            format!("{}@{start_byte}", ctx.name)
        } else {
            seen_names.insert(ctx.name.clone(), true);
            ctx.name.clone()
        };
        callables_dict.insert(key, callable_hash);
    }

    (callables_dict, callable_list)
}

/// Compute Merkle root from callable hashes, file imports, and module shell hash.
///
/// Three layers (flat combiner): imports | callables | module shell.
pub fn compute_merkle_hash(callable_hashes: &[CallableHash], imports: &[String], module_shell_hash: &str) -> String {
    let mut sorted_imports: Vec<String> = imports.to_vec();
    sorted_imports.sort();
    sorted_imports.dedup();
    let import_hash = blake3::hash(sorted_imports.join("|").as_bytes()).to_hex().to_string();

    let mut sorted_callables = callable_hashes.to_vec();
    sorted_callables.sort_by(|a, b| a.name.cmp(&b.name));
    let callable_hash_strings: Vec<String> = sorted_callables
        .iter()
        .map(|c| format!("{}:{}", c.name, c.structural_hash))
        .collect();
    let callables_hash = blake3::hash(callable_hash_strings.join("|").as_bytes())
        .to_hex()
        .to_string();

    let shell_hash = merkle_hex_layer(module_shell_hash);
    let combined = format!("{import_hash}|{callables_hash}|{shell_hash}");
    let root_hash = blake3::hash(combined.as_bytes()).to_hex().to_string();

    format!("blake3:{root_hash}")
}

#[cfg(test)]
mod tests {
    use super::{
        build_granular_callables, compute_merkle_hash, compute_module_shell_hash, seal_declaration_addresses,
        CallableHash, SiteSpan,
    };
    use crate::logic_address::{AddressMeta, LogicKind};

    const SHELL_A: &str = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHELL_B: &str = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn ch(name: &str, structural_hash: &str) -> CallableHash {
        CallableHash {
            name: name.to_string(),
            structural_hash: structural_hash.to_string(),
            exact_body_id: None,
            signature_hash: String::new(),
            imports: vec![],
            signature: String::new(),
            node_type: "function".to_string(),
            kind: LogicKind::Function,
            scope: vec![],
            logic_address: String::new(),
            address_id: String::new(),
            start_byte: 0,
            end_byte: 0,
        }
    }

    #[test]
    fn merkle_three_layer_deterministic() {
        let callables = vec![ch("foo", "blake3:abc123"), ch("bar", "blake3:def456")];
        let imports = vec!["os".to_string()];
        let root = compute_merkle_hash(&callables, &imports, SHELL_A);
        assert!(root.starts_with("blake3:"));
        assert_eq!(root.len(), 71);

        let callables_reordered = vec![callables[1].clone(), callables[0].clone()];
        assert_eq!(root, compute_merkle_hash(&callables_reordered, &imports, SHELL_A));
    }

    #[test]
    fn merkle_module_shell_affects_root() {
        let callables = vec![ch("foo", "blake3:abc")];
        let root_a = compute_merkle_hash(&callables, &[], SHELL_A);
        let root_b = compute_merkle_hash(&callables, &[], SHELL_B);
        assert_ne!(root_a, root_b);
    }

    #[test]
    fn merkle_duplicate_callable_names_both_contribute() {
        let callables = vec![
            CallableHash {
                name: "foo".to_string(),
                structural_hash: "blake3:aaa".to_string(),
                exact_body_id: None,
                signature_hash: String::new(),
                imports: vec![],
                signature: String::new(),
                node_type: "method".to_string(),
                kind: LogicKind::Method,
                scope: vec![],
                logic_address: String::new(),
                address_id: String::new(),
                start_byte: 0,
                end_byte: 0,
            },
            CallableHash {
                name: "foo".to_string(),
                structural_hash: "blake3:bbb".to_string(),
                exact_body_id: None,
                signature_hash: String::new(),
                imports: vec![],
                signature: String::new(),
                node_type: "method".to_string(),
                kind: LogicKind::Method,
                scope: vec![],
                logic_address: String::new(),
                address_id: String::new(),
                start_byte: 0,
                end_byte: 0,
            },
        ];
        let root_both = compute_merkle_hash(&callables, &[], SHELL_A);
        let root_one = compute_merkle_hash(&[callables[0].clone()], &[], SHELL_A);
        assert_ne!(root_both, root_one);
    }

    #[test]
    fn module_shell_hash_from_ir() {
        let h1 = compute_module_shell_hash("module_a");
        let h2 = compute_module_shell_hash("module_b");
        assert_ne!(h1, h2);
        assert!(h1.starts_with("blake3:"));
    }

    #[test]
    fn build_granular_callables_seals_addresses() {
        use crate::callable_context::CallableContext;

        let contexts = vec![
            CallableContext::with_scope("foo", "function", vec![], false, 10, 20),
            CallableContext::with_scope("foo", "function", vec![], false, 30, 40),
        ];
        let mut contexts = contexts;
        contexts[0].structural_hash = "blake3:first".to_string();
        contexts[1].structural_hash = "blake3:second".to_string();

        let meta = AddressMeta::new("python").with_path("mod.py");
        let (map, list) = build_granular_callables(contexts, &meta, None);
        assert_eq!(list.len(), 2);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("foo"));
        assert!(map.keys().any(|k| k.starts_with("foo@")));
        assert!(list[0].logic_address.contains("|10|20") || list[1].logic_address.contains("|10|20"));
        assert_ne!(list[0].address_id, list[1].address_id);
        assert!(list[0].logic_address.starts_with("logic|-|python|mod.py|"));
    }

    #[test]
    fn unique_callable_omits_span() {
        use crate::callable_context::CallableContext;

        let mut ctx = CallableContext::with_scope("only", "function", vec![], false, 1, 2);
        ctx.structural_hash = "blake3:x".to_string();
        let meta = AddressMeta::new("go");
        let (_, list) = build_granular_callables(vec![ctx], &meta, None);
        assert!(list[0].logic_address.ends_with("|-|-"));
        assert_eq!(list[0].kind, LogicKind::Function);
    }

    #[test]
    fn declaration_sealing_requires_one_matching_production_occurrence() {
        let source = "handler = lambda: 1\n";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&crate::grammars::PYTHON.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let crate::normalizer::VisitorOutput {
            callables: contexts,
            markers: pending,
            ..
        } = crate::languages::python::PythonNormalizer
            .visit_tree(&tree, source.as_bytes(), Some(&crate::StructuralHashPolicy::default()))
            .unwrap();
        assert!(pending.markers.declarations.is_empty());
        assert_eq!(pending.declarations.len(), 1);
        let meta = AddressMeta::new("python")
            .with_path("fixture.py")
            .with_repository("fixture");
        let (_, callables) = build_granular_callables(contexts, &meta, Some(source.as_bytes()));
        assert_eq!(callables.len(), 1);
        let sealed = seal_declaration_addresses(pending.clone(), &callables, &meta).unwrap();
        let declaration = &sealed.declarations[0];
        declaration.validate().unwrap();
        assert_eq!(
            declaration.site.span,
            SiteSpan {
                start_byte: 0,
                end_byte: 7
            }
        );
        let target = declaration.callable.as_ref().unwrap();
        assert_eq!(
            target.span,
            SiteSpan {
                start_byte: 10,
                end_byte: 19
            }
        );
        assert_eq!(target.occurrence.address_id.as_str(), callables[0].address_id);
        assert!(seal_declaration_addresses(pending.clone(), &[], &meta).is_err());
        for mutation in 0..6 {
            let mut invalid = callables.clone();
            match mutation {
                0 => invalid.push(callables[0].clone()),
                1 => invalid[0].scope.push("other".into()),
                2 => invalid[0].start_byte += 1,
                3 => invalid[0].end_byte = invalid[0].start_byte,
                4 => invalid[0].logic_address.push('x'),
                _ => invalid[0].name = "other".into(),
            }
            assert!(
                seal_declaration_addresses(pending.clone(), &invalid, &meta).is_err(),
                "mutation {mutation}"
            );
        }
        let mut duplicate = pending.clone();
        duplicate.declarations.push(duplicate.declarations[0].clone());
        assert!(seal_declaration_addresses(duplicate, &callables, &meta).is_err());
        let mut already_sealed = pending;
        already_sealed.markers = sealed;
        assert!(seal_declaration_addresses(already_sealed, &callables, &meta).is_err());
    }
}
