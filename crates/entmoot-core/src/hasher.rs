//! StructuralHasher - Main entry point for computing structural hashes.

use std::cell::RefCell;
use std::collections::HashMap;

use tree_sitter::{Parser, Tree};

use crate::granular::{build_granular_callables, compute_merkle_hash, GranularHashResult};
use crate::languages;
use crate::logic_address::AddressMeta;
use crate::registry::LanguageRegistry;
use crate::sound_cst::{hash_module, hash_sound_callable, CallableNodes, SoundStructuralProfile};
use entmoot_contracts::StructuralHashPolicy;

/// Main class for computing structural hashes of source code.
pub struct StructuralHasher {
    registry: LanguageRegistry,
    parsers: HashMap<String, RefCell<Parser>>,
    /// Policy used when a call site passes `None` (sound v1).
    default_policy: StructuralHashPolicy,
}

impl StructuralHasher {
    pub fn canon_file(
        &self,
        source: &str,
        language: &str,
        contracts: &[entmoot_contracts::CodeContract],
        facts: &[entmoot_contracts::CanonExpressionFact],
        max_patterns: usize,
    ) -> crate::CoreResult<crate::canon::CanonAnalysis> {
        let tree = self.parse_source_code(source, language)?;
        Self::validate_parse_errors(&tree)?;
        let index = CallableNodes::new(
            tree.root_node(),
            contracts.iter().map(|c| (c.site.span.start_byte, c.site.span.end_byte)),
        );
        let mut clones = crate::clones::analyze(source.as_bytes(), &index, language, contracts, 1, true, max_patterns);
        let forms = crate::canon::lower(source.as_bytes(), language, &index, contracts, facts, max_patterns);
        clones.truncated |= forms.len()
            < contracts
                .iter()
                .filter(|c| matches!(c.shape, entmoot_contracts::DeclaredContract::Callable { .. }))
                .count();
        Ok(crate::canon::CanonAnalysis { clones, forms })
    }
    /// Parse once per file and derive review-only clone fingerprints at real inventory spans.
    pub fn clone_patterns(
        &self,
        source: &str,
        language: &str,
        contracts: &[entmoot_contracts::CodeContract],
        min_tokens: usize,
        statements: bool,
        max_patterns: usize,
    ) -> crate::CoreResult<crate::clones::CloneAnalysis> {
        let tree = self.parse_source_code(source, language)?;
        Self::validate_parse_errors(&tree)?;
        let index = CallableNodes::new(
            tree.root_node(),
            contracts.iter().map(|c| (c.site.span.start_byte, c.site.span.end_byte)),
        );
        Ok(crate::clones::analyze(
            source.as_bytes(),
            &index,
            language,
            contracts,
            min_tokens,
            statements,
            max_patterns,
        ))
    }
    pub fn new() -> Self {
        Self::with_policy(StructuralHashPolicy::v1())
    }

    /// Hasher whose `None` policy argument resolves to `default_policy`.
    ///
    /// Only the sound v1 policy is representable.
    pub fn with_policy(default_policy: StructuralHashPolicy) -> Self {
        let mut hasher = Self {
            registry: LanguageRegistry::new(),
            parsers: HashMap::new(),
            default_policy,
        };
        hasher.initialize_languages();
        hasher
    }

    pub fn default_policy(&self) -> &StructuralHashPolicy {
        &self.default_policy
    }

    fn initialize_languages(&mut self) {
        languages::register_all(&mut self.registry);

        for name in self.registry.get_supported_languages() {
            if let Some(plugin) = self.registry.get_language(&name) {
                let mut parser = Parser::new();
                if parser.set_language(&plugin.language).is_ok() {
                    self.parsers.insert(name, RefCell::new(parser));
                }
            }
        }
    }

    fn parse_source_code(&self, source_code: &str, language: &str) -> crate::CoreResult<Tree> {
        let lang = language.to_lowercase();
        let parser_cell = self
            .parsers
            .get(&lang)
            .ok_or_else(|| crate::CoreError::UnsupportedLanguage(format!("'{language}' not supported")))?;

        let mut parser = parser_cell.borrow_mut();
        let tree = parser
            .parse(source_code, None)
            .ok_or_else(|| crate::CoreError::Parse(format!("Failed to parse source code for language '{language}'")))?;
        Ok(tree)
    }

    fn validate_parse_errors(tree: &Tree) -> crate::CoreResult<()> {
        let root = tree.root_node();
        if !root.has_error() {
            return Ok(());
        }

        Err(crate::CoreError::Parse(
            "Source code contains syntax errors and cannot be hashed reliably".into(),
        ))
    }

    fn hash_source(
        &self,
        source_code: &str,
        language: &str,
        policy: Option<&StructuralHashPolicy>,
        address_meta: Option<&AddressMeta>,
        include_markers: bool,
        debug_ir: Option<&mut String>,
    ) -> crate::CoreResult<GranularHashResult> {
        let policy = policy.cloned().unwrap_or_else(|| self.default_policy.clone());
        let plugin = self.registry.get_language(language).ok_or_else(|| {
            let supported = self.get_supported_languages().join(", ");
            crate::CoreError::UnsupportedLanguage(format!(
                "'{language}' not supported. Supported languages: {supported}"
            ))
        })?;

        let tree = self.parse_source_code(source_code, language)?;
        // Validate the same language tree used by inventory, spans and identity.
        // Another grammar's acceptance cannot qualify a recovered tree.
        Self::validate_parse_errors(&tree)?;

        // Inventory and markers use the original tree; identity comes from retained CST.
        let crate::normalizer::VisitorOutput {
            callables: mut callables_contexts,
            imports: file_imports,
            markers: mut file_markers,
        } = plugin
            .normalizer
            .visit_tree(&tree, source_code.as_bytes(), Some(&policy))
            .map_err(|e| crate::CoreError::Serialize(e.to_string()))?;

        let meta = address_meta.cloned().unwrap_or_else(|| AddressMeta::new(language));
        let policy_id = policy.fingerprint();
        let imports: Vec<String> = file_imports.into_iter().collect();

        let nodes = CallableNodes::new(
            tree.root_node(),
            callables_contexts
                .iter()
                .map(|ctx| (ctx.start_byte, ctx.end_byte))
                .chain(
                    file_markers
                        .declarations
                        .iter()
                        .map(|d| (d.span.start_byte, d.span.end_byte)),
                ),
        );
        let (module_shell_hash, callables_dict, callable_list) = {
            let profile = SoundStructuralProfile::for_language(language);
            let root = tree.root_node();
            let source_bytes = source_code.as_bytes();
            for ctx in &mut callables_contexts {
                let node = nodes
                    .get(ctx.start_byte, ctx.end_byte)
                    .ok_or_else(|| crate::CoreError::Message("callable span is not in the source CST".into()))?;
                if matches!(
                    language,
                    "rust" | "go" | "javascript" | "typescript" | "tsx" | "csharp" | "python" | "java"
                ) {
                    let signature = crate::sound_cst::callable_signature(source_bytes, node, &profile);
                    ctx.signature_hash = crate::hash::hash_structural_ir(&signature);
                    ctx.signature = format!("({signature})");
                }
                let imports_for_callable: Vec<String> = ctx.imports.iter().cloned().collect();
                ctx.structural_hash = hash_sound_callable(source_bytes, node, &imports_for_callable, &profile);
            }
            let module_shell_hash = hash_module(source_bytes, root, &profile, debug_ir);
            let (callables_dict, callable_list) =
                build_granular_callables(callables_contexts, &meta, Some(source_bytes));
            (module_shell_hash, callables_dict, callable_list)
        };

        if include_markers {
            crate::declaration_syntax::collect(tree.root_node(), language, source_code.as_bytes(), &mut file_markers);
        }
        let mut file_markers = crate::granular::seal_declaration_addresses(file_markers, &callable_list, &meta)
            .map_err(crate::CoreError::Message)?;
        if include_markers {
            crate::code_contracts::collect(
                &nodes,
                language,
                source_code.as_bytes(),
                &callable_list,
                &mut file_markers,
            )
            .map_err(crate::CoreError::Message)?;
        }
        let file_hash = compute_merkle_hash(&callable_list, &imports, &module_shell_hash);

        Ok(GranularHashResult {
            policy_id,
            file_hash,
            module_shell_hash,
            callables: callables_dict,
            imports,
            markers: if include_markers { Some(file_markers) } else { None },
        })
    }

    pub fn compute_hash(
        &self,
        source_code: &str,
        language: &str,
        policy: Option<&StructuralHashPolicy>,
    ) -> crate::CoreResult<String> {
        Ok(self
            .hash_source(source_code, language, policy, None, false, None)?
            .file_hash)
    }

    pub fn compute_hash_with_debug(
        &self,
        source_code: &str,
        language: &str,
    ) -> crate::CoreResult<(String, HashMap<String, String>)> {
        let mut serialized_ir = String::new();
        let result = self.hash_source(source_code, language, None, None, false, Some(&mut serialized_ir))?;

        let mut debug_info = HashMap::new();
        debug_info.insert("file_hash".to_string(), result.file_hash.clone());
        debug_info.insert("module_shell_hash".to_string(), result.module_shell_hash.clone());
        debug_info.insert("serialized_ir".to_string(), serialized_ir);
        debug_info.insert("callable_count".to_string(), result.callables.len().to_string());
        debug_info.insert("source_code".to_string(), source_code.to_string());
        debug_info.insert("language".to_string(), language.to_string());

        Ok((result.file_hash, debug_info))
    }

    pub fn compute_granular_hash(
        &self,
        source_code: &str,
        language: &str,
        policy: Option<&StructuralHashPolicy>,
    ) -> crate::CoreResult<GranularHashResult> {
        self.hash_source(source_code, language, policy, None, false, None)
    }

    /// Granular hash plus analysis markers from the unified AST walk.
    pub fn compute_granular_hash_with_analysis(
        &self,
        source_code: &str,
        language: &str,
        policy: Option<&StructuralHashPolicy>,
    ) -> crate::CoreResult<GranularHashResult> {
        self.hash_source(source_code, language, policy, None, true, None)
    }

    /// Granular hash with analysis markers and explicit path/repository for logic addresses.
    pub fn compute_granular_hash_with_analysis_at(
        &self,
        source_code: &str,
        language: &str,
        path: Option<&str>,
        repository: Option<&str>,
        policy: Option<&StructuralHashPolicy>,
    ) -> crate::CoreResult<GranularHashResult> {
        let mut meta = AddressMeta::new(language);
        if let Some(p) = path {
            meta = meta.with_path(p);
        }
        if let Some(r) = repository {
            meta = meta.with_repository(r);
        }
        self.hash_source(source_code, language, policy, Some(&meta), true, None)
    }

    /// Resolve language name from a file extension (e.g. `.py` → `python`).
    pub fn language_for_extension(&self, extension: &str) -> Option<String> {
        self.registry.get_language(extension).map(|p| p.name.clone())
    }

    /// Per-callable file hashing with explicit repository/path for logic addresses.
    pub fn compute_granular_hash_at(
        &self,
        source_code: &str,
        language: &str,
        path: Option<&str>,
        repository: Option<&str>,
        policy: Option<&StructuralHashPolicy>,
    ) -> crate::CoreResult<GranularHashResult> {
        let mut meta = AddressMeta::new(language);
        if let Some(p) = path {
            meta = meta.with_path(p);
        }
        if let Some(r) = repository {
            meta = meta.with_repository(r);
        }
        self.hash_source(source_code, language, policy, Some(&meta), false, None)
    }

    pub fn get_supported_languages(&self) -> Vec<String> {
        self.registry.get_supported_languages()
    }
}

impl Default for StructuralHasher {
    fn default() -> Self {
        Self::new()
    }
}
