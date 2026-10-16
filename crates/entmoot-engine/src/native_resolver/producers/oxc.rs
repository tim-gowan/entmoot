//! Oxc (`oxc` semantic) JS/TS lexical producer.
//!
//! In-process Rust crates; no TypeScript type checker. Unique lexical identifier
//! callees may Close under `entmoot.oxc.direct-identifier.v1`; member and
//! indirect forms stay open. Process-local NodeId/ScopeId/SymbolId never cross
//! the artifact boundary.

#[cfg(feature = "resolved-oxc")]
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::ProducerCapabilityProfile;
use crate::graph::TopologyGraph;
use crate::native_resolver::budget::BudgetWatch;
use crate::native_resolver::pipeline::NativeProducerError;
use entmoot_contracts::NativeRawObservation;

pub const PROFILE: ProducerCapabilityProfile = ProducerCapabilityProfile {
    language: "javascript",
    producer_id: "oxc_semantic",
    semantic_surface: "lexical_scopes_symbols_module_resolution",
    supports_types: false,
    supports_dispatch: true,
    supports_cfg: false,
    isolation: "in_process_rust",
    limitations: &[
        "no_typescript_type_checker",
        "closed_dispatch_unique_identifier_only",
        "relative_explicit_exports_unique_inventory_callable",
        "bare_package_imports_emit_external_symbol",
        "node_builtins_require_explicit_runtime_or_node_specifier",
        "process_local_ids_not_wire_identity",
        "cfg_not_emitted_non_goal",
    ],
};

/// Pinned `oxc` crate version used when `resolved-oxc` is enabled.
pub const OXC_PIN: &str = "0.153.0";

pub(crate) fn configured_runtime() -> String {
    std::env::var("ENTMOOT_OXC_RUNTIME").unwrap_or_else(|_| "unspecified".into())
}

#[cfg(feature = "resolved-oxc")]
mod collect_impl {
    use super::*;
    use std::path::{Path, PathBuf};

    use crate::graph::NodeKind;
    use entmoot_contracts::{NativeCallFact, NativeCallTarget, NativeReferenceFact, NativeSpanRef};

    use oxc::allocator::Allocator;
    use oxc::ast::ast::{
        Argument, ArrowFunctionExpression, BindingPattern, CallExpression, Expression, Function,
        IdentifierReference, ImportDeclaration, ImportDeclarationSpecifier, ModuleExportName,
        VariableDeclarator,
    };
    use oxc::ast_visit::{walk, Visit};
    use oxc::parser::Parser;
    use oxc::semantic::{Scoping, SemanticBuilder, SymbolId};
    use oxc::span::{SourceType, Span};

    /// Extract the npm package name from a bare module request (`lodash`, `@scope/pkg/sub`).
    fn npm_package_name_from_request(request: &str) -> Option<String> {
        let trimmed = request.trim();
        if trimmed.is_empty()
            || trimmed == "."
            || trimmed == ".."
            || trimmed.starts_with("./")
            || trimmed.starts_with("../")
            || trimmed.starts_with('/')
            || trimmed.starts_with("node:")
            || trimmed.starts_with("nodejs:")
        {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix('@') {
            let (scope, name) = rest.split_once('/')?;
            if scope.is_empty() || name.is_empty() || scope == "." || scope == ".." {
                return None;
            }
            // Subpath imports: @scope/name/sub → package @scope/name
            let package = name.split('/').next()?;
            if package.is_empty() || package == "." || package == ".." {
                return None;
            }
            Some(format!("@{scope}/{package}"))
        } else {
            let name = trimmed.split('/').next()?;
            if name == "." || name == ".." {
                return None;
            }
            Some(name.to_owned())
        }
    }

    // Bounded, versioned Node core specifiers. Unknown subpaths never inherit
    // builtin status merely from their first segment.
    fn node_builtin(request: &str) -> bool {
        matches!(
            request.strip_prefix("node:").unwrap_or(request),
            "assert"
                | "assert/strict"
                | "async_hooks"
                | "buffer"
                | "child_process"
                | "cluster"
                | "console"
                | "constants"
                | "crypto"
                | "dgram"
                | "diagnostics_channel"
                | "dns"
                | "dns/promises"
                | "domain"
                | "events"
                | "fs"
                | "fs/promises"
                | "http"
                | "http2"
                | "https"
                | "module"
                | "net"
                | "os"
                | "path"
                | "path/posix"
                | "path/win32"
                | "perf_hooks"
                | "process"
                | "punycode"
                | "querystring"
                | "readline"
                | "readline/promises"
                | "repl"
                | "stream"
                | "stream/consumers"
                | "stream/promises"
                | "stream/web"
                | "string_decoder"
                | "sys"
                | "timers"
                | "timers/promises"
                | "tls"
                | "trace_events"
                | "tty"
                | "url"
                | "util"
                | "util/types"
                | "v8"
                | "vm"
                | "wasi"
                | "worker_threads"
                | "zlib"
        )
    }

    fn builtin_role(request: &str, runtime: &str) -> Option<&'static str> {
        node_builtin(request).then_some(
            if runtime == "node" || (runtime == "unspecified" && request.starts_with("node:")) {
                "stdlib"
            } else {
                "unknown"
            },
        )
    }

    #[cfg(test)]
    pub(super) fn npm_package_name_from_request_for_test(request: &str) -> Option<String> {
        npm_package_name_from_request(request)
    }

    struct SymbolDecl {
        path: String,
        start: u32,
        end: u32,
        name: String,
    }

    #[derive(Clone)]
    struct ImportBinding {
        module_request: String,
        imported_name: String,
    }

    #[derive(Clone)]
    struct PendingCall {
        span: Span,
        targets: Vec<NativeCallTarget>,
        gap: Option<String>,
        owner: Option<NativeSpanRef>,
        callee_ref: Option<NativeReferenceFact>,
    }

    struct SymbolCollector<'s> {
        scoping: &'s Scoping,
        relative: String,
        decls_by_symbol: BTreeMap<SymbolId, SymbolDecl>,
        imports_by_symbol: BTreeMap<SymbolId, ImportBinding>,
    }

    impl<'a> Visit<'a> for SymbolCollector<'_> {
        fn visit_import_declaration(&mut self, decl: &ImportDeclaration<'a>) {
            if decl.import_kind.is_type() {
                return;
            }
            let source = decl.source.value.as_str();
            let Some(specifiers) = &decl.specifiers else {
                walk::walk_import_declaration(self, decl);
                return;
            };
            for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(spec) => {
                        if spec.import_kind.is_type() {
                            continue;
                        }
                        let imported_name = match &spec.imported {
                            ModuleExportName::IdentifierName(name) => name.name.to_string(),
                            ModuleExportName::IdentifierReference(name) => name.name.to_string(),
                            ModuleExportName::StringLiteral(literal) => literal.value.to_string(),
                        };
                        self.imports_by_symbol.insert(
                            spec.local.symbol_id(),
                            ImportBinding {
                                module_request: source.to_string(),
                                imported_name,
                            },
                        );
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) => {
                        self.imports_by_symbol.insert(
                            spec.local.symbol_id(),
                            ImportBinding {
                                module_request: source.to_string(),
                                imported_name: "default".into(),
                            },
                        );
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
                }
            }
            walk::walk_import_declaration(self, decl);
        }

        fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
            let (call, member) = match &declaration.init {
                Some(Expression::CallExpression(call)) => (call, None),
                Some(Expression::StaticMemberExpression(member)) => match &member.object {
                    Expression::CallExpression(call) => {
                        (call, Some(member.property.name.to_string()))
                    }
                    _ => {
                        walk::walk_variable_declarator(self, declaration);
                        return;
                    }
                },
                _ => {
                    walk::walk_variable_declarator(self, declaration);
                    return;
                }
            };
            let global_require = matches!(&call.callee, Expression::Identifier(id) if id.name == "require"
                && self.scoping.get_reference(id.reference_id()).symbol_id().is_none());
            if global_require && !call.optional {
                if let [Argument::StringLiteral(request)] = call.arguments.as_slice() {
                    match (&declaration.id, member) {
                        (BindingPattern::BindingIdentifier(id), Some(imported_name)) => {
                            self.imports_by_symbol.insert(
                                id.symbol_id(),
                                ImportBinding {
                                    module_request: request.value.to_string(),
                                    imported_name,
                                },
                            );
                        }
                        // `var accepts = require('accepts')` — CJS default, same as ESM default import.
                        (BindingPattern::BindingIdentifier(id), None) => {
                            self.imports_by_symbol.insert(
                                id.symbol_id(),
                                ImportBinding {
                                    module_request: request.value.to_string(),
                                    imported_name: "default".into(),
                                },
                            );
                        }
                        (BindingPattern::ObjectPattern(pattern), None)
                            if pattern.rest.is_none() =>
                        {
                            for property in &pattern.properties {
                                if property.computed {
                                    continue;
                                }
                                if let (Some(name), BindingPattern::BindingIdentifier(id)) =
                                    (property.key.static_name(), &property.value)
                                {
                                    self.imports_by_symbol.insert(
                                        id.symbol_id(),
                                        ImportBinding {
                                            module_request: request.value.to_string(),
                                            imported_name: name.to_string(),
                                        },
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            walk::walk_variable_declarator(self, declaration);
        }

        fn visit_function(&mut self, func: &Function<'a>, flags: oxc::syntax::scope::ScopeFlags) {
            if let Some(id) = &func.id {
                let symbol_id = id.symbol_id();
                self.decls_by_symbol.insert(
                    symbol_id,
                    SymbolDecl {
                        path: self.relative.clone(),
                        start: func.span.start,
                        end: func.span.end,
                        name: id.name.to_string(),
                    },
                );
            }
            walk::walk_function(self, func, flags);
        }
    }

    struct FileCollector<'a> {
        relative: String,
        scoping: &'a Scoping,
        decls_by_symbol: BTreeMap<SymbolId, SymbolDecl>,
        imports_by_symbol: BTreeMap<SymbolId, ImportBinding>,
        inventory_callables: &'a BTreeMap<(String, String), NativeCallTarget>,
        graph_paths: &'a BTreeMap<String, ()>,
        runtime: &'a str,
        enclosing: Vec<(u32, u32)>,
        pending: Vec<PendingCall>,
    }

    impl<'a> FileCollector<'a> {
        fn owner_ref(&self) -> Option<NativeSpanRef> {
            self.enclosing.last().map(|(start, end)| NativeSpanRef {
                path: self.relative.clone(),
                span: entmoot_contracts::SiteSpan {
                    start_byte: *start as usize,
                    end_byte: *end as usize,
                },
            })
        }

        fn resolve_relative_module(&self, request: &str) -> Option<String> {
            if !(request.starts_with("./") || request.starts_with("../")) {
                return None;
            }
            let from_dir = Path::new(&self.relative)
                .parent()
                .unwrap_or_else(|| Path::new(""));
            let joined = from_dir.join(request);
            let mut candidates = Vec::new();
            let mut normalized = PathBuf::new();
            for component in joined.components() {
                match component {
                    std::path::Component::ParentDir => {
                        // An out-of-scope module must not alias an in-root file.
                        if !normalized.pop() {
                            return None;
                        }
                    }
                    std::path::Component::CurDir => {}
                    std::path::Component::Normal(part) => normalized.push(part),
                    _ => return None,
                }
            }
            let base = normalized.to_string_lossy().replace('\\', "/");
            for suffix in [
                "",
                ".js",
                ".jsx",
                ".mjs",
                ".cjs",
                ".ts",
                ".tsx",
                "/index.js",
                "/index.ts",
                "/index.tsx",
                "/index.jsx",
            ] {
                let candidate = if suffix.is_empty() {
                    base.clone()
                } else if base.ends_with(suffix.trim_start_matches('/')) && suffix.starts_with('.')
                {
                    continue;
                } else {
                    format!("{base}{suffix}")
                };
                if self.graph_paths.contains_key(&candidate) {
                    candidates.push(candidate);
                }
            }
            if candidates.len() == 1 {
                Some(candidates.remove(0))
            } else {
                None
            }
        }

        fn resolve_import_target(&self, binding: &ImportBinding) -> Option<NativeCallTarget> {
            if let Some(module_path) = self.resolve_relative_module(&binding.module_request) {
                return self
                    .inventory_callables
                    .get(&(module_path, binding.imported_name.clone()))
                    .cloned();
            }
            // Bare package imports: external module target with npm PURL coordinates.
            if binding.module_request.starts_with("./")
                || binding.module_request.starts_with("../")
                || binding.module_request.starts_with('/')
            {
                return None;
            }
            let builtin = builtin_role(&binding.module_request, self.runtime);
            let package_name = if builtin.is_some() {
                None
            } else {
                Some(npm_package_name_from_request(&binding.module_request)?)
            };
            let symbol = format!("{}#{}", binding.module_request, binding.imported_name);
            Some(NativeCallTarget {
                location: entmoot_contracts::NativeTargetLocation::from_provider(
                    None,
                    None,
                    entmoot_contracts::SiteSpan {
                        start_byte: 0,
                        end_byte: 0,
                    },
                ),
                symbol,
                kind: "callable".into(),
                package_role: Some(builtin.unwrap_or("module").into()),
                package_purl: None,
                ecosystem: Some("javascript".into()),
                module_path: package_name,
                package_version: None,
                path_segments: Vec::new(),
            })
        }

        fn resolve_ident(&self, ident: &IdentifierReference<'_>) -> Option<NativeCallTarget> {
            let reference = self.scoping.get_reference(ident.reference_id());
            let symbol_id = reference.symbol_id()?;
            if self.scoping.symbol_is_mutated(symbol_id) {
                return None;
            }
            if let Some(decl) = self.decls_by_symbol.get(&symbol_id) {
                return Some(NativeCallTarget {
                    location: entmoot_contracts::NativeTargetLocation::from_provider(
                        None,
                        Some(decl.path.clone()),
                        entmoot_contracts::SiteSpan {
                            start_byte: decl.start as usize,
                            end_byte: decl.end as usize,
                        },
                    ),
                    symbol: decl.name.clone(),
                    kind: "callable".into(),
                    package_role: None,
                    package_purl: None,
                    ecosystem: None,
                    module_path: None,
                    package_version: None,
                    path_segments: Vec::new(),
                });
            }
            let binding = self.imports_by_symbol.get(&symbol_id)?;
            self.resolve_import_target(binding)
        }

        fn push_call(
            &mut self,
            span: Span,
            targets: Vec<NativeCallTarget>,
            gap: Option<&str>,
            callee_ref: Option<NativeReferenceFact>,
        ) {
            self.pending.push(PendingCall {
                span,
                targets,
                gap: gap.map(str::to_string),
                owner: self.owner_ref(),
                callee_ref,
            });
        }
    }

    impl<'a> Visit<'a> for FileCollector<'a> {
        fn visit_function(&mut self, func: &Function<'a>, flags: oxc::syntax::scope::ScopeFlags) {
            self.enclosing.push((func.span.start, func.span.end));
            walk::walk_function(self, func, flags);
            self.enclosing.pop();
        }

        fn visit_arrow_function_expression(&mut self, func: &ArrowFunctionExpression<'a>) {
            self.enclosing.push((func.span.start, func.span.end));
            walk::walk_arrow_function_expression(self, func);
            self.enclosing.pop();
        }

        fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
            match &call.callee {
                Expression::Identifier(ident) => {
                    let target = self.resolve_ident(ident);
                    let callee_ref = target.as_ref().map(|resolved| NativeReferenceFact {
                        site: entmoot_contracts::NativeSpanRef {
                            path: self.relative.clone(),
                            span: entmoot_contracts::SiteSpan {
                                start_byte: ident.span.start as usize,
                                end_byte: ident.span.end as usize,
                            },
                        },
                        kind: if self
                            .scoping
                            .get_reference(ident.reference_id())
                            .symbol_id()
                            .is_some_and(|id| self.imports_by_symbol.contains_key(&id))
                        {
                            "import_callee"
                        } else {
                            "call_callee"
                        }
                        .into(),
                        spelling: ident.name.to_string(),
                        targets: vec![resolved.clone()],
                        owner: self.owner_ref(),
                        gap: None,
                        limitation: None,
                    });
                    match target {
                        Some(resolved) => {
                            self.push_call(call.span, vec![resolved], None, callee_ref)
                        }
                        None => {
                            self.push_call(call.span, vec![], Some("unsupported_construct"), None)
                        }
                    }
                }
                Expression::StaticMemberExpression(_) => {
                    self.push_call(call.span, vec![], Some("dynamic_dispatch"), None);
                }
                _ => {
                    self.push_call(call.span, vec![], Some("indirect_call"), None);
                }
            }
            walk::walk_call_expression(self, call);
        }
    }

    fn project_to_inventory(
        relative: &str,
        wanted: &BTreeMap<(usize, usize), ()>,
        pending: &[PendingCall],
    ) -> (Vec<NativeCallFact>, Vec<NativeReferenceFact>) {
        let mut calls = Vec::new();
        let mut references = Vec::new();
        // Iterate facts once, retaining every alternative at an exact span.
        // The shared join, not traversal order, decides uniqueness.
        for call in pending {
            let (inv_start, inv_end) = (call.span.start as usize, call.span.end as usize);
            if !wanted.contains_key(&(inv_start, inv_end)) {
                continue;
            }
            let mut fact = NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: relative.to_string(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: inv_start,
                        end_byte: inv_end,
                    },
                },
                targets: call.targets.clone(),
                owner: call.owner.clone(),
                gap: call.gap.clone(),
                limitation: None,
                intrinsic: false,
            };
            if fact.gap.is_none()
                && fact.owner.is_some()
                && fact.targets.len() == 1
                && fact.targets[0].kind == "callable"
            {
                fact.dispatch = entmoot_contracts::CallDispatch::Closed {
                    policy_id: "entmoot.oxc.direct-identifier.v1".into(),
                    evidence_pointer: format!("/calls/{}/dispatch", calls.len()),
                    assumptions: vec![
                        "selected module graph and lexical scopes".into(),
                        "source-level unique identifier callee; member, runtime mutation, and foreign replacement are outside this qualification".into(),
                    ],
                };
            }
            calls.push(fact);
            if let Some(reference) = &call.callee_ref {
                references.push(reference.clone());
            }
        }
        (calls, references)
    }

    type FileAnalysis = (Vec<NativeCallFact>, Vec<NativeReferenceFact>, Vec<String>);

    fn analyze_file(
        relative: &str,
        source: &str,
        wanted: &BTreeMap<(usize, usize), ()>,
        inventory_callables: &BTreeMap<(String, String), NativeCallTarget>,
        graph_paths: &BTreeMap<String, ()>,
        runtime: &str,
    ) -> Result<FileAnalysis, NativeProducerError> {
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(relative)
            .map_err(|error| NativeProducerError::Message(error.to_string()))?;
        let ret = Parser::new(&allocator, source, source_type).parse();
        let mut diagnostics = Vec::new();
        for diagnostic in &ret.diagnostics {
            diagnostics.push(format!("{relative}: {diagnostic}"));
        }
        let built = SemanticBuilder::new().build(&ret.program);
        for error in &built.diagnostics {
            diagnostics.push(format!("{relative}: {error}"));
        }
        // Recovered syntax is not sufficient authority for a binding.
        if !diagnostics.is_empty() {
            return Ok((Vec::new(), Vec::new(), diagnostics));
        }
        let scoping = built.semantic.scoping();
        let mut symbols = SymbolCollector {
            scoping,
            relative: relative.into(),
            decls_by_symbol: BTreeMap::new(),
            imports_by_symbol: BTreeMap::new(),
        };
        symbols.visit_program(&ret.program);
        let mut collector = FileCollector {
            relative: relative.to_string(),
            scoping,
            decls_by_symbol: symbols.decls_by_symbol,
            imports_by_symbol: symbols.imports_by_symbol,
            inventory_callables,
            graph_paths,
            runtime,
            enclosing: Vec::new(),
            pending: Vec::new(),
        };
        collector.visit_program(&ret.program);
        let (calls, references) = project_to_inventory(relative, wanted, &collector.pending);
        Ok((calls, references, diagnostics))
    }

    fn is_js_ts(path: &str) -> bool {
        path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".mjs")
            || path.ends_with(".cjs")
            || path.ends_with(".ts")
            || path.ends_with(".tsx")
            || path.ends_with(".mts")
            || path.ends_with(".cts")
    }

    pub fn collect(
        source_root: &Path,
        graph: &TopologyGraph,
        watch: &BudgetWatch,
    ) -> Result<NativeRawObservation, NativeProducerError> {
        let root = std::fs::canonicalize(source_root)?;
        let runtime = configured_runtime();
        if !matches!(runtime.as_str(), "node" | "browser" | "unspecified") {
            return Err(NativeProducerError::Message(
                "ENTMOOT_OXC_RUNTIME must be node, browser or unspecified".into(),
            ));
        }
        let graph_paths: BTreeMap<String, ()> = graph
            .nodes
            .iter()
            .filter_map(|node| node.path.clone().map(|path| (path, ())))
            .collect();
        // Build qualified export routes once. Local imports use the compiler
        // declaration's exact source range, including aliases and reexports.
        let mut inventory_callables = BTreeMap::new();
        let mut export_inputs = BTreeMap::new();
        let mut export_cache = BTreeMap::new();
        let declaration_spans: BTreeMap<_, _> = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Callable)
            .filter_map(|node| {
                Some((
                    (
                        node.path.as_ref()?.as_str(),
                        node.start_byte?,
                        node.end_byte?,
                    ),
                    node,
                ))
            })
            .collect();
        for path in graph_paths.keys().filter(|path| is_js_ts(path)) {
            let exports = super::super::oxc_package_api::module_exports(
                &root,
                &root.join(path),
                &mut export_inputs,
                watch,
                &mut export_cache,
                &mut BTreeSet::new(),
            )?;
            for (name, export) in exports {
                let Some(target_path) = export["path"].as_str() else {
                    continue;
                };
                let (Some(start), Some(end)) =
                    (export["start_byte"].as_u64(), export["end_byte"].as_u64())
                else {
                    continue;
                };
                let (Ok(start_byte), Ok(end_byte)) = (usize::try_from(start), usize::try_from(end))
                else {
                    continue;
                };
                let Some(node) = declaration_spans.get(&(target_path, start_byte, end_byte)) else {
                    continue;
                };
                inventory_callables.insert(
                    (path.clone(), name),
                    NativeCallTarget {
                        location: entmoot_contracts::NativeTargetLocation::from_provider(
                            None,
                            Some(target_path.into()),
                            entmoot_contracts::SiteSpan {
                                start_byte,
                                end_byte,
                            },
                        ),
                        symbol: node.label.clone(),
                        kind: "callable".into(),
                        package_role: None,
                        package_purl: None,
                        ecosystem: None,
                        module_path: None,
                        package_version: None,
                        path_segments: Vec::new(),
                    },
                );
            }
        }
        let mut sites_by_file: BTreeMap<String, BTreeMap<(usize, usize), ()>> = BTreeMap::new();
        for node in &graph.nodes {
            if node.kind != NodeKind::CallStub {
                continue;
            }
            let Some(path) = node.path.as_ref() else {
                continue;
            };
            if !is_js_ts(path) {
                continue;
            }
            let (Some(start), Some(end)) = (node.start_byte, node.end_byte) else {
                continue;
            };
            sites_by_file
                .entry(path.clone())
                .or_default()
                .insert((start, end), ());
        }

        // Parse represented modules once, including export-only files with no calls.
        for path in graph_paths.keys().filter(|path| is_js_ts(path)) {
            sites_by_file.entry(path.clone()).or_default();
        }
        let mut raw = NativeRawObservation {
            compiler_version: Some(format!("oxc@{OXC_PIN}")),
            ..Default::default()
        };
        raw.build_context
            .insert("javascript_runtime".into(), runtime.clone().into());
        raw.build_context.insert(
            "builtin_policy".into(),
            "oxc.node_core_specifiers.v1".into(),
        );
        if sites_by_file.is_empty() {
            raw.diagnostics
                .push("no JavaScript/TypeScript call stubs to analyze".into());
        }

        for (relative, wanted) in sites_by_file {
            watch.check_wall()?;
            let absolute = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            let source = match std::fs::read_to_string(&absolute) {
                Ok(value) => value,
                Err(error) => {
                    raw.build_context.insert("partial".into(), true.into());
                    raw.diagnostics
                        .push(format!("failed to read {relative}: {error}"));
                    continue;
                }
            };
            raw.inputs.push(absolute.to_string_lossy().into_owned());
            match analyze_file(
                &relative,
                &source,
                &wanted,
                &inventory_callables,
                &graph_paths,
                &runtime,
            ) {
                Ok((calls, references, diagnostics)) => {
                    if !diagnostics.is_empty() {
                        raw.build_context.insert("partial".into(), true.into());
                    }
                    raw.calls.extend(calls);
                    raw.references.extend(references);
                    raw.diagnostics.extend(diagnostics);
                }
                Err(NativeProducerError::Budget(_)) => {
                    raw.budget_exceeded = true;
                    raw.diagnostics
                        .push("native analysis budget exhausted".into());
                    break;
                }
                Err(error) => {
                    raw.build_context.insert("partial".into(), true.into());
                    raw.diagnostics.push(error.to_string());
                }
            }
        }

        for (path, digest) in export_inputs {
            if entmoot_contracts::blake3_hex(&std::fs::read(&path)?) != digest {
                return Err(NativeProducerError::Drift(format!(
                    "module export input changed: {}",
                    path.display()
                )));
            }
            raw.inputs.push(path.to_string_lossy().into_owned());
        }
        super::super::oxc_package_api::collect(&root, &mut raw, watch)?;
        raw.inputs.sort();
        raw.inputs.dedup();
        raw.calls.sort_by(|a, b| {
            (&a.site.path, a.site.span.start_byte).cmp(&(&b.site.path, b.site.span.start_byte))
        });
        raw.references.sort_by(|a, b| {
            (&a.site.path, a.site.span.start_byte).cmp(&(&b.site.path, b.site.span.start_byte))
        });
        Ok(raw)
    }
    #[cfg(test)]
    mod projection_tests {
        use super::*;
        #[test]
        fn builtin_metadata_requires_runtime_evidence_and_exact_specifiers() {
            for request in ["path", "node:path", "fs/promises"] {
                assert_eq!(builtin_role(request, "node"), Some("stdlib"));
                assert_eq!(builtin_role(request, "browser"), Some("unknown"));
            }
            assert_eq!(builtin_role("node:path", "unspecified"), Some("stdlib"));
            assert_eq!(builtin_role("path", "unspecified"), Some("unknown"));
            for request in ["path/other", "path-extra", "@scope/path", "./path"] {
                assert_eq!(builtin_role(request, "node"), None);
            }
        }
        #[test]
        fn projection_retains_conflicting_facts_at_the_same_span() {
            let pending = ["indirect_call", "dynamic_dispatch"].map(|gap| PendingCall {
                span: Span::new(10, 20),
                targets: vec![],
                gap: Some(gap.into()),
                owner: None,
                callee_ref: None,
            });
            let (calls, _) =
                project_to_inventory("main.js", &BTreeMap::from([((10, 20), ())]), &pending);
            assert_eq!(calls.len(), 2);
            assert_ne!(calls[0].gap, calls[1].gap);
            assert!(
                project_to_inventory("main.js", &BTreeMap::from([((10, 19), ())]), &pending)
                    .0
                    .is_empty()
            );
        }
    }
}

/// Collect Oxc lexical observations for inventoried JS/TS call stubs.
pub fn collect_oxc_observation(
    source_root: &Path,
    graph: &TopologyGraph,
    watch: &BudgetWatch,
) -> Result<NativeRawObservation, NativeProducerError> {
    #[cfg(feature = "resolved-oxc")]
    {
        collect_impl::collect(source_root, graph, watch)
    }
    #[cfg(not(feature = "resolved-oxc"))]
    {
        let _ = (source_root, graph, watch);
        Err(NativeProducerError::AdapterUnavailableOxc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_declares_lexical_closed_identifier() {
        const {
            assert!(!PROFILE.supports_types);
            assert!(PROFILE.supports_dispatch);
            assert!(!PROFILE.supports_cfg);
        }
        assert_eq!(PROFILE.isolation, "in_process_rust");
    }
}

#[cfg(all(test, feature = "resolved-oxc"))]
mod package_name_tests {
    use super::collect_impl::npm_package_name_from_request_for_test as name;

    #[test]
    fn rejects_relative_and_node_protocol_requests() {
        assert_eq!(name(".."), None);
        assert_eq!(name("."), None);
        assert_eq!(name("../lib"), None);
        assert_eq!(name("./x"), None);
        assert_eq!(name("node:assert"), None);
        assert_eq!(name("nodejs:fs"), None);
        assert_eq!(name("accepts"), Some("accepts".into()));
        assert_eq!(name("@scope/pkg/sub"), Some("@scope/pkg".into()));
    }
}
