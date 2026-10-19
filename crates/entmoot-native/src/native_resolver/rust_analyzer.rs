//! In-process rust-analyzer workspace solver (opt-in `resolved-rust-analyzer`).
//!
//! Strategy mirrors CodeQL’s build-mode-none orchestration without copying its
//! source: discover manifests, qualify/load one workspace model, resolve
//! inventoried call sites through HIR APIs, and preserve explicit gaps.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ra_ap_base_db::salsa::Setter;
use ra_ap_base_db::SourceDatabase;
use ra_ap_hir::{
    AsAssocItem, AssocItemContainer, HasAttrs, HasVisibility, PathResolution, Semantics,
};
use ra_ap_ide_db::RootDatabase;
use ra_ap_load_cargo::{load_workspace, LoadCargoConfig, ProcMacroServerChoice};
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::{
    CargoConfig, CargoFeatures, InvocationStrategy, ProjectManifest, ProjectWorkspace,
    ProjectWorkspaceKind, RustLibSource, TargetKind,
};
use ra_ap_syntax::ast::HasName;
use ra_ap_syntax::{ast, AstNode, Edition, SyntaxKind, TextRange, TextSize};
use ra_ap_vfs::{FileId, Vfs};

use super::input::{NativeInventory, NativeSite};
use crate::graph::NodeKind;
use crate::native_resolver::budget::BudgetWatch;
use entmoot_contracts::{
    NativeCallFact, NativeCallTarget, NativeRawObservation, NativeReferenceFact, NativeSpanRef,
};

use crate::native_resolver::pipeline::{NativeProduceRequest, NativeProducerError};
use entmoot_contracts::{ExecutionPolicy, RustAnalysisMode, RustFeatureSelection};

#[derive(Debug, Clone)]
struct WorkspaceFiles {
    root: PathBuf,
    files: Vec<String>,
}

fn discover_workspaces(root: &Path, rust_files: &[String]) -> Vec<WorkspaceFiles> {
    let mut by_manifest: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for relative in rust_files {
        let absolute = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut cursor = absolute.parent().map(Path::to_path_buf);
        let mut manifest = None;
        while let Some(dir) = cursor {
            let candidate = dir.join("Cargo.toml");
            if candidate.is_file() {
                manifest = Some(candidate);
                break;
            }
            if dir == root {
                break;
            }
            cursor = dir.parent().map(Path::to_path_buf);
        }
        let key = manifest.unwrap_or_else(|| root.join("Cargo.toml"));
        by_manifest.entry(key).or_default().insert(relative.clone());
    }
    by_manifest
        .into_iter()
        .map(|(manifest, files)| {
            let workspace_root = manifest
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.to_path_buf());
            WorkspaceFiles {
                root: workspace_root,
                files: files.into_iter().collect(),
            }
        })
        .collect()
}

fn cargo_config(
    request: &NativeProduceRequest,
    workspace_root: &Path,
) -> Result<CargoConfig, NativeProducerError> {
    request
        .rust_profile
        .validate()
        .map_err(|error| NativeProducerError::Message(error.into()))?;
    let mut config = CargoConfig {
        wrap_rustc_in_build_scripts: false,
        // CargoConfig::default disables the sysroot entirely. Discover the
        // selected toolchain's installed sources so ordinary std/core methods
        // can resolve; loading never installs rust-src or runs target builds.
        sysroot: Some(RustLibSource::Discover),
        ..CargoConfig::default()
    };
    if matches!(request.execution_policy, ExecutionPolicy::Safe) {
        config.run_build_script_command = None;
    }
    config.features = match &request.rust_profile.features {
        RustFeatureSelection::All {} => CargoFeatures::All,
        RustFeatureSelection::Selected {
            names,
            no_default_features,
        } => CargoFeatures::Selected {
            features: names.iter().cloned().collect(),
            no_default_features: *no_default_features,
        },
    };
    config
        .target
        .clone_from(&request.rust_profile.target_triple);
    if let Some(sysroot) = &request.rust_profile.sysroot {
        let abs =
            AbsPathBuf::try_from(sysroot.to_str().ok_or_else(|| {
                NativeProducerError::Message("Rust sysroot must be UTF-8".into())
            })?)
            .map_err(|_| {
                NativeProducerError::Message("Rust sysroot must be absolute UTF-8".into())
            })?;
        config.sysroot = Some(RustLibSource::Path(abs));
    }
    config.invocation_strategy = InvocationStrategy::Once;
    // Cargo metadata writes Cargo.lock when none exists. Redirect that write under
    // .entmoot so inventorial snapshot digests stay stable across bind (rustc ≥1.97
    // honors CARGO_RESOLVER_LOCKFILE_PATH). Existing lockfiles are copied by RA.
    if !workspace_root.join("Cargo.lock").is_file() {
        let isolated = workspace_root.join(".entmoot").join("native-cargo");
        std::fs::create_dir_all(&isolated).map_err(|error| {
            NativeProducerError::Message(format!(
                "cannot isolate Cargo.lock under {}: {error}",
                isolated.display()
            ))
        })?;
        let lock = isolated.join("Cargo.lock");
        let lock = lock.to_str().ok_or_else(|| {
            NativeProducerError::Message("isolated Cargo.lock path is not UTF-8".into())
        })?;
        config
            .extra_env
            .insert("CARGO_RESOLVER_LOCKFILE_PATH".into(), Some(lock.into()));
    }
    Ok(config)
}

struct CargoTargetContext {
    root: PathBuf,
    selected_test: bool,
}

fn target_contexts(
    model: &ProjectWorkspace,
    mode: &RustAnalysisMode,
    root: &Path,
) -> Result<Vec<CargoTargetContext>, NativeProducerError> {
    let selected = match mode {
        RustAnalysisMode::Workspace {} => None,
        RustAnalysisMode::Test { crate_root } => {
            let path = canonical_source_path(root.join(crate_root))?;
            if !path.starts_with(root) {
                return Err(NativeProducerError::Message(
                    "Rust test crate root escapes source root".into(),
                ));
            }
            Some(path)
        }
    };
    let ProjectWorkspaceKind::Cargo { cargo, .. } = &model.kind else {
        return Err(NativeProducerError::Message(
            "Rust build profile requires a Cargo workspace".into(),
        ));
    };
    let mut contexts = Vec::new();
    for package in cargo.packages() {
        let package = &cargo[package];
        for target in &package.targets {
            let target = &cargo[*target];
            if !package.is_member && !matches!(target.kind, TargetKind::Lib { .. }) {
                continue;
            }
            let path = canonical_source_path(target.root.as_str())?;
            let selected_test = selected.as_ref() == Some(&path);
            if selected_test {
                if !package.is_member
                    || !matches!(
                        target.kind,
                        TargetKind::Lib { .. } | TargetKind::Bin | TargetKind::Test
                    )
                {
                    return Err(NativeProducerError::Message("Rust test profile requires a unique member library, binary or integration-test target".into()));
                }
                if target
                    .required_features
                    .iter()
                    .any(|feature| !package.active_features.contains(feature))
                {
                    return Err(NativeProducerError::Message(format!(
                        "Rust test target {} has inactive required features",
                        target.name
                    )));
                }
            }
            contexts.push(CargoTargetContext {
                root: path,
                selected_test,
            });
        }
    }
    if selected.is_some()
        && contexts
            .iter()
            .filter(|context| context.selected_test)
            .count()
            != 1
    {
        return Err(NativeProducerError::Message(
            "Rust test crate root must select exactly one Cargo target".into(),
        ));
    }
    Ok(contexts)
}

fn apply_target_contexts(
    db: &mut RootDatabase,
    vfs: &Vfs,
    contexts: &[CargoTargetContext],
    raw: &mut NativeRawObservation,
) -> Result<(), NativeProducerError> {
    let mut by_root = BTreeMap::new();
    for context in contexts {
        by_root
            .entry(&context.root)
            .and_modify(|entry| *entry = None)
            .or_insert(Some(context));
    }
    let crates = ra_ap_base_db::all_crates(db);
    let mut selected = None;
    let mut observed = Vec::new();
    for krate in crates.iter().copied() {
        let Some(path) = vfs.file_path(krate.data(db).root_file_id).as_path() else {
            continue;
        };
        let path = canonical_source_path(path.as_str())?;
        let Some(context) = by_root.get(&path) else {
            continue;
        };
        let Some(context) = context else {
            return Err(NativeProducerError::Message(format!(
                "ambiguous Cargo crate root: {}",
                path.display()
            )));
        };
        if context.selected_test {
            if selected.replace(krate).is_some() {
                return Err(NativeProducerError::Message(
                    "ambiguous loaded Rust test target".into(),
                ));
            }
            let mut cfg = krate.cfg_options(db).clone();
            cfg.insert_atom(ra_ap_intern::sym::test);
            krate.set_cfg_options(db).to(cfg);
        }
        observed.push(
            serde_json::json!({"root": path, "test_mode": context.selected_test,
            "edition": krate.data(db).edition.to_string(),
            "cfg": format!("{:?}", krate.cfg_options(db))}),
        );
    }
    if contexts.iter().any(|context| context.selected_test) {
        let selected = selected.ok_or_else(|| {
            NativeProducerError::Message("selected Rust test target was not loaded".into())
        })?;
        let dependencies: BTreeSet<_> = selected.transitive_deps(db).into_iter().collect();
        ra_ap_base_db::set_all_crates_with_durability(
            db,
            crates
                .iter()
                .copied()
                .filter(|krate| dependencies.contains(krate)),
            ra_ap_base_db::salsa::Durability::HIGH,
        );
    }
    raw.build_context
        .entry("target_contexts")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| NativeProducerError::Artifact("invalid target context capture".into()))?
        .extend(observed);
    Ok(())
}

fn load_config(policy: ExecutionPolicy) -> LoadCargoConfig {
    match policy {
        ExecutionPolicy::Safe => LoadCargoConfig {
            load_out_dirs_from_check: false,
            with_proc_macro_server: ProcMacroServerChoice::None,
            prefill_caches: false,
            num_worker_threads: 1,
            proc_macro_processes: 1,
        },
        ExecutionPolicy::Trusted => LoadCargoConfig {
            load_out_dirs_from_check: true,
            with_proc_macro_server: ProcMacroServerChoice::Sysroot,
            prefill_caches: false,
            num_worker_threads: 1,
            proc_macro_processes: 1,
        },
    }
}

fn relative_to(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
}

/// One immutable index joins path spellings through actual filesystem identity.
/// Cargo metadata and Windows directory enumeration may disagree on verbatim
/// prefixes or case. No files or synthetic crates are added to the compiler VFS.
fn canonical_source_path(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(|path| super::producers::sidecar::compiler_root(&path))
}

fn inventory_file_ids(vfs: &Vfs, root: &Path) -> BTreeMap<PathBuf, Option<FileId>> {
    let mut files = BTreeMap::new();
    for (id, path) in vfs.iter() {
        let Some(path) = path.as_path() else { continue };
        let Ok(canonical) = canonical_source_path(path.as_str()) else {
            continue;
        };
        if canonical.starts_with(root) {
            // Multiple compiler VFS entries for one physical file are ambiguous.
            // Never pick an arbitrary edition/source buffer by iterator order.
            files
                .entry(canonical)
                .and_modify(|entry| *entry = None)
                .or_insert(Some(id));
        }
    }
    files
}

fn function_span(
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    root: &Path,
    function: ra_ap_hir::Function,
    api: &mut RustPackageApi,
) -> Option<NativeCallTarget> {
    let source = sema.source(function)?;
    // Expanded syntax offsets belong to the expansion buffer, not the original
    // physical file. Never reuse those offsets as structural declaration spans.
    if source.file_id.is_macro() {
        return None;
    }
    let editioned = source.file_id.original_file(sema.db);
    let file_id = editioned.file_id(sema.db);
    let file_path = vfs.file_path(file_id);
    let abs = file_path.as_path()?;
    let path = canonical_source_path(abs.as_str()).ok()?;
    let relative = relative_to(root, &path).unwrap_or_default();
    let range = source.value.syntax().text_range();
    let name = function
        .name(sema.db)
        .display(sema.db, Edition::CURRENT)
        .to_string();
    let mut target = NativeCallTarget {
        location: entmoot_contracts::NativeTargetLocation::from_provider(
            Some(path.to_string_lossy().into_owned()),
            if relative.is_empty() {
                None
            } else {
                Some(relative)
            },
            entmoot_contracts::SiteSpan {
                start_byte: usize::from(range.start()),
                end_byte: usize::from(range.end()),
            },
        ),
        symbol: name,
        kind: if source.value.body().is_some() {
            "callable"
        } else {
            "value"
        }
        .into(),
        package_role: None,
        package_purl: None,
        ecosystem: None,
        module_path: None,
        package_version: None,
        path_segments: Vec::new(),
    };
    if let Some((package, symbol, header)) = rust_boundary(sema, function, &source.value) {
        target.symbol = symbol.clone();
        target.package_role = Some(
            if target.location.repository().is_some() {
                "application"
            } else {
                "module"
            }
            .into(),
        );
        target.package_purl = Some(format!("pkg:cargo/{package}"));
        target.ecosystem = Some("rust".into());
        api.boundaries.insert(symbol, (package, header));
    } else if target.location.repository().is_none() {
        // Package-qualified external without an assessed header: keep coords so
        // package_api_seek can emit seeking refs (do not invent boundary_text).
        let module = function.module(sema.db);
        if let Some(package) = match module.krate(sema.db).origin(sema.db) {
            ra_ap_base_db::CrateOrigin::Local {
                name: Some(name), ..
            }
            | ra_ap_base_db::CrateOrigin::Library { name, .. } => Some(name.to_string()),
            _ => None,
        } {
            if let Some(qualified) =
                ra_ap_hir::ModuleDef::Function(function).canonical_path(sema.db, Edition::CURRENT)
            {
                target.symbol = format!("{package}::{qualified}");
                target.package_role = Some("module".into());
                target.package_purl = Some(format!("pkg:cargo/{package}"));
                target.ecosystem = Some("rust".into());
            }
        }
    }
    Some(target)
}

const RUST_BOUNDARY_POLICY: &str = "rust_analyzer.function_and_container_headers.v1";
const RUST_TYPE_POLICY: &str = "rust_analyzer.public_declaration_source.v1";

#[derive(Default)]
struct RustPackageApi {
    boundaries: BTreeMap<String, (String, String)>,
    exports: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    references: Vec<serde_json::Value>,
}

struct RustUseContext<'a, 'db> {
    sema: &'a Semantics<'db, RootDatabase>,
    vfs: &'a Vfs,
    root: &'a Path,
    callable_index: &'a CallableIndex<'a>,
}

fn public_module_route(sema: &Semantics<'_, RootDatabase>, module: ra_ap_hir::Module) -> bool {
    module.path_to_root(sema.db).iter().all(|module| {
        module.is_crate_root(sema.db) || module.visibility(sema.db) == ra_ap_hir::Visibility::Public
    })
}

fn public_use_route(
    sema: &Semantics<'_, RootDatabase>,
    tree: &ast::UseTree,
    spelling: &str,
) -> Option<String> {
    if tree.star_token().is_some()
        || tree
            .syntax()
            .ancestors()
            .any(|node| ast::Fn::can_cast(node.kind()))
    {
        return None;
    }
    let module = sema.scope(tree.syntax())?.module();
    if !public_module_route(sema, module) {
        return None;
    }
    Some(if module.is_crate_root(sema.db) {
        spelling.into()
    } else {
        format!(
            "{}::{spelling}",
            ra_ap_hir::ModuleDef::Module(module).canonical_path(sema.db, Edition::CURRENT)?
        )
    })
}

/// A visible alias can expose a source declaration in a private module. Keep
/// source identity separate from the alias's public route and never map an
/// expansion buffer onto physical source coordinates.
fn named_use_target(
    context: &RustUseContext<'_, '_>,
    definition: ra_ap_hir::ModuleDef,
) -> Option<(NativeCallTarget, String, String)> {
    use ra_ap_hir::{Adt, ModuleDef};
    let sema = context.sema;
    if definition.visibility(sema.db) != ra_ap_hir::Visibility::Public {
        return None;
    }
    let source = match definition {
        ModuleDef::Adt(Adt::Struct(value)) => sema.source(value)?.map(|node| node.syntax().clone()),
        ModuleDef::Adt(Adt::Enum(value)) => sema.source(value)?.map(|node| node.syntax().clone()),
        ModuleDef::Adt(Adt::Union(value)) => sema.source(value)?.map(|node| node.syntax().clone()),
        ModuleDef::Trait(value) => sema.source(value)?.map(|node| node.syntax().clone()),
        ModuleDef::TypeAlias(value) => sema.source(value)?.map(|node| node.syntax().clone()),
        _ => return None,
    };
    if source.file_id.is_macro() {
        return None;
    }
    let module = definition.module(sema.db)?;
    let package = match module.krate(sema.db).origin(sema.db) {
        ra_ap_base_db::CrateOrigin::Local {
            name: Some(name), ..
        }
        | ra_ap_base_db::CrateOrigin::Library { name, .. } => name.to_string(),
        _ => return None,
    };
    let file = source.file_id.original_file(sema.db).file_id(sema.db);
    let physical = canonical_source_path(context.vfs.file_path(file).as_path()?.as_str()).ok()?;
    let relative = relative_to(context.root, &physical);
    let range = source.value.text_range();
    let qualified = definition.canonical_path(sema.db, Edition::CURRENT)?;
    let contract = declaration_contract(&source.value);
    Some((
        NativeCallTarget {
            location: entmoot_contracts::NativeTargetLocation::from_provider(
                Some(physical.to_string_lossy().into_owned()),
                relative,
                entmoot_contracts::SiteSpan {
                    start_byte: usize::from(range.start()),
                    end_byte: usize::from(range.end()),
                },
            ),
            symbol: format!("{package}::{qualified}"),
            kind: "type".into(),
            package_role: Some("module".into()),
            package_purl: Some(format!("pkg:cargo/{package}")),
            ecosystem: Some("rust".into()),
            module_path: None,
            package_version: None,
            path_segments: Vec::new(),
        },
        package,
        contract,
    ))
}

fn declaration_contract(node: &ra_ap_syntax::SyntaxNode) -> String {
    let range = node.text_range();
    let mut contract = node.text().to_string();
    let mut bodies: Vec<_> = node
        .descendants()
        .filter_map(ast::Fn::cast)
        .filter_map(|function| function.body())
        .map(|body| body.syntax().text_range())
        .collect();
    bodies.sort_by_key(|range| (range.start(), std::cmp::Reverse(range.end())));
    let mut outer = Vec::<TextRange>::new();
    for body in bodies {
        if !outer
            .last()
            .is_some_and(|parent| parent.contains_range(body))
        {
            outer.push(body);
        }
    }
    for body in outer.into_iter().rev() {
        let start = usize::from(body.start() - range.start());
        let end = usize::from(body.end() - range.start());
        if contract.get(start..end).is_some() {
            contract.replace_range(start..end, "{}");
        }
    }
    contract
}

fn collect_use_bindings(
    context: &RustUseContext<'_, '_>,
    relative: &str,
    use_item: &ast::Use,
    api: &mut RustPackageApi,
    raw: &mut NativeRawObservation,
) {
    use ra_ap_syntax::ast::HasVisibility as AstHasVisibility;
    let is_public = AstHasVisibility::visibility(use_item)
        .is_some_and(|visibility| visibility.syntax().text().to_string().trim() == "pub");
    let Some(tree) = use_item.use_tree() else {
        return;
    };
    collect_use_tree_bindings(context, relative, &tree, None, api, raw, is_public);
}

fn collect_use_tree_bindings(
    context: &RustUseContext<'_, '_>,
    relative: &str,
    tree: &ast::UseTree,
    prefix: Option<ast::Path>,
    api: &mut RustPackageApi,
    raw: &mut NativeRawObservation,
    is_public: bool,
) {
    let RustUseContext {
        sema,
        vfs,
        root,
        callable_index,
    } = *context;
    if let Some(list) = tree.use_tree_list() {
        let next_prefix = tree.path().or(prefix);
        for child in list.use_trees() {
            collect_use_tree_bindings(
                context,
                relative,
                &child,
                next_prefix.clone(),
                api,
                raw,
                is_public,
            );
        }
        return;
    }
    let Some(path) = tree.path().or(prefix) else {
        return;
    };
    let resolved = sema.resolve_path(&path);
    let spelling = tree
        .rename()
        .and_then(|rename| rename.name())
        .map(|name| name.text().to_string())
        .or_else(|| {
            path.segment()
                .and_then(|segment| segment.name_ref())
                .map(|name| name.text().to_string())
        })
        .unwrap_or_else(|| path.syntax().text().to_string());
    let start_byte = use_item_start(tree).unwrap_or(0);
    let end_byte = use_item_end(tree).unwrap_or(start_byte);
    match resolved {
        Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(function))) => {
            let Some(target) = function_span(sema, vfs, root, function, api) else {
                raw.references.push(NativeReferenceFact {
                    site: entmoot_contracts::NativeSpanRef {
                        path: relative.into(),
                        span: entmoot_contracts::SiteSpan {
                            start_byte,
                            end_byte,
                        },
                    },
                    kind: "import".into(),
                    spelling,
                    targets: Vec::new(),
                    owner: None,
                    gap: Some("unsupported_construct".into()),
                    limitation: Some("use resolves to a function without a callable span".into()),
                });
                return;
            };
            raw.references.push(NativeReferenceFact {
                site: entmoot_contracts::NativeSpanRef {
                    path: relative.into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte,
                        end_byte,
                    },
                },
                kind: "import".into(),
                spelling: spelling.clone(),
                targets: vec![target.clone()],
                owner: None,
                gap: None,
                limitation: None,
            });
            if !is_public {
                return;
            }
            // A public use inside a private module is not a package export.
            // Keep the alias's visible module route, but attach it to the
            // compiler-resolved declaration, never the use statement.
            if tree
                .syntax()
                .ancestors()
                .any(|node| ast::Fn::can_cast(node.kind()))
            {
                return;
            }
            let Some(scope) = sema.scope(tree.syntax()) else {
                return;
            };
            let module = scope.module();
            if module.path_to_root(sema.db).iter().any(|module| {
                !module.is_crate_root(sema.db)
                    && module.visibility(sema.db) != ra_ap_hir::Visibility::Public
            }) {
                return;
            }
            let route = if module.is_crate_root(sema.db) {
                spelling
            } else {
                let Some(path) =
                    ra_ap_hir::ModuleDef::Module(module).canonical_path(sema.db, Edition::CURRENT)
                else {
                    return;
                };
                format!("{path}::{spelling}")
            };
            let Some((package, header)) = api.boundaries.get(&target.symbol).cloned() else {
                return;
            };
            let Some(def_path) = target.location.repository().map(|site| site.path.as_str()) else {
                return;
            };
            let def_name = function
                .name(sema.db)
                .display(sema.db, Edition::CURRENT)
                .to_string();
            let Some(node) = join_inventory_callable(
                callable_index,
                def_path,
                &def_name,
                target.location.span().start_byte,
                target.location.span().end_byte,
            ) else {
                return;
            };
            let symbol = format!("{package}::{route}");
            api.exports.entry(package).or_default().insert(
                symbol.clone(),
                serde_json::json!({
                    "path": def_path,
                    "start_byte": node.start_byte,
                    "end_byte": node.end_byte,
                    "symbol": symbol,
                    "boundary_policy": RUST_BOUNDARY_POLICY,
                    "boundary_text": header
                }),
            );
        }
        Some(PathResolution::Def(definition)) => {
            if let Some((target, package, contract)) = named_use_target(context, definition) {
                if is_public {
                    if let (Some(route), Some(path)) = (
                        public_use_route(sema, tree, &spelling),
                        target.location.repository().map(|site| site.path.as_str()),
                    ) {
                        let symbol = format!("{package}::{route}");
                        api.exports.entry(package).or_default().insert(symbol.clone(), serde_json::json!({
                            "path": path, "start_byte": target.location.span().start_byte, "end_byte": target.location.span().end_byte,
                            "symbol": symbol, "boundary_policy": RUST_TYPE_POLICY, "boundary_text": contract
                        }));
                    }
                }
                raw.references.push(NativeReferenceFact {
                    site: entmoot_contracts::NativeSpanRef {
                        path: relative.into(),
                        span: entmoot_contracts::SiteSpan {
                            start_byte,
                            end_byte,
                        },
                    },
                    kind: "import".into(),
                    spelling,
                    targets: vec![target],
                    owner: None,
                    gap: None,
                    limitation: None,
                });
                return;
            }
            raw.references.push(NativeReferenceFact {
                site: entmoot_contracts::NativeSpanRef {
                    path: relative.into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte,
                        end_byte,
                    },
                },
                kind: "import".into(),
                spelling,
                targets: Vec::new(),
                owner: None,
                gap: Some("unsupported_construct".into()),
                limitation: Some(
                    "use target has no qualified physical declaration under the Rust bind policy"
                        .into(),
                ),
            });
        }
        Some(_) => {
            raw.references.push(NativeReferenceFact {
                site: entmoot_contracts::NativeSpanRef {
                    path: relative.into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte,
                        end_byte,
                    },
                },
                kind: "import".into(),
                spelling,
                targets: Vec::new(),
                owner: None,
                gap: Some("unresolved".into()),
                limitation: Some("use path did not resolve to a definition".into()),
            });
        }
        None => {
            raw.references.push(NativeReferenceFact {
                site: entmoot_contracts::NativeSpanRef {
                    path: relative.into(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte,
                        end_byte,
                    },
                },
                kind: "import".into(),
                spelling,
                targets: Vec::new(),
                owner: None,
                gap: Some("unresolved".into()),
                limitation: Some("rust-analyzer left use path open".into()),
            });
        }
    }
}

fn use_item_start(tree: &ast::UseTree) -> Option<usize> {
    Some(usize::from(tree.syntax().text_range().start()))
}

fn use_item_end(tree: &ast::UseTree) -> Option<usize> {
    Some(usize::from(tree.syntax().text_range().end()))
}

fn rust_boundary(
    sema: &Semantics<'_, RootDatabase>,
    function: ra_ap_hir::Function,
    node: &ast::Fn,
) -> Option<(String, String, String)> {
    let trait_implementation = function.as_assoc_item(sema.db).is_some_and(|item| {
        matches!(item.container(sema.db), AssocItemContainer::Impl(implementation)
            if implementation.trait_(sema.db).is_some_and(|trait_| trait_.visibility(sema.db) == ra_ap_hir::Visibility::Public))
    });
    if function.visibility(sema.db) != ra_ap_hir::Visibility::Public && !trait_implementation {
        return None;
    }
    let owner = if let Some(assoc) = function.as_assoc_item(sema.db) {
        Some(match assoc.container(sema.db) {
            AssocItemContainer::Impl(imp) => {
                let definition = ra_ap_hir::ModuleDef::Adt(imp.self_ty(sema.db).as_adt()?);
                if definition.visibility(sema.db) != ra_ap_hir::Visibility::Public {
                    return None;
                }
                let owner = definition.canonical_path(sema.db, Edition::CURRENT)?;
                if let Some(trait_) = imp.trait_(sema.db) {
                    if trait_.visibility(sema.db) != ra_ap_hir::Visibility::Public {
                        return None;
                    }
                    format!(
                        "<{owner} as {}>",
                        ra_ap_hir::ModuleDef::Trait(trait_)
                            .canonical_path(sema.db, Edition::CURRENT)?
                    )
                } else {
                    owner
                }
            }
            AssocItemContainer::Trait(trait_)
                if trait_.visibility(sema.db) == ra_ap_hir::Visibility::Public =>
            {
                ra_ap_hir::ModuleDef::Trait(trait_).canonical_path(sema.db, Edition::CURRENT)?
            }
            // Trait implementations have distinct dispatch and visibility rules.
            _ => return None,
        })
    } else {
        None
    };
    let module = function.module(sema.db);
    // Public functions in private modules may be exposed by a public use.
    // Direct export enumeration checks module visibility separately.
    let package = match module.krate(sema.db).origin(sema.db) {
        ra_ap_base_db::CrateOrigin::Local {
            name: Some(name), ..
        }
        | ra_ap_base_db::CrateOrigin::Library { name, .. } => name.to_string(),
        _ => return None,
    };
    let qualified = if let Some(owner) = owner {
        // ModuleDef::Function's canonical path omits its associated container.
        // Preserve native owner identity to distinguish A::get from B::get.
        format!(
            "{}::{}",
            owner,
            function.name(sema.db).display(sema.db, Edition::CURRENT)
        )
    } else {
        ra_ap_hir::ModuleDef::Function(function).canonical_path(sema.db, Edition::CURRENT)?
    };
    let offset = node.body().map_or_else(
        || usize::from(node.syntax().text_range().len()),
        |body| usize::from(body.syntax().text_range().start() - node.syntax().text_range().start()),
    );
    let text = node.syntax().text().to_string();
    let mut header = String::new();
    for parent in node.syntax().ancestors().skip(1) {
        let list = ast::Trait::cast(parent.clone())
            .and_then(|item| item.assoc_item_list())
            .or_else(|| ast::Impl::cast(parent.clone()).and_then(|item| item.assoc_item_list()));
        if let Some(list) = list {
            let end = usize::from(list.syntax().text_range().start() - parent.text_range().start());
            header.push_str(parent.text().to_string().get(..end)?);
            header.push('\n');
            break;
        }
    }
    header.push_str(text.get(..offset)?);
    Some((package.clone(), format!("{package}::{qualified}"), header))
}

fn resolve_call_expr(
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    root: &Path,
    call: &ast::CallExpr,
    api: &mut RustPackageApi,
) -> Result<Vec<NativeCallTarget>, Option<&'static str>> {
    let expr = call.expr().ok_or(None)?;
    if let Some(path_expr) = ast::PathExpr::cast(expr.syntax().clone()) {
        let path = path_expr.path().ok_or(None)?;
        match sema.resolve_path(&path) {
            Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(function))) => {
                Ok(function_span(sema, vfs, root, function, api)
                    .into_iter()
                    .collect())
            }
            Some(PathResolution::Local(_)) => Err(Some("indirect_call")),
            Some(_) => Err(Some("unsupported_construct")),
            None => Err(None),
        }
    } else {
        Err(Some("indirect_call"))
    }
}

fn resolve_method_call(
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    root: &Path,
    call: &ast::MethodCallExpr,
    api: &mut RustPackageApi,
) -> Result<Vec<NativeCallTarget>, Option<&'static str>> {
    match sema.resolve_method_call(call) {
        Some(function) => Ok(function_span(sema, vfs, root, function, api)
            .into_iter()
            .collect()),
        // Missing method binding is not proven trait/virtual dispatch; trait
        // containers still set dynamic_dispatch after a successful resolve.
        None => Err(Some("unresolved")),
    }
}

fn enclosing_fn_span(node: &ra_ap_syntax::SyntaxNode) -> Option<(usize, usize, String)> {
    for ancestor in node.ancestors() {
        if let Some(func) = ast::Fn::cast(ancestor) {
            let range = func.syntax().text_range();
            let name = func
                .name()
                .map(|name| name.text().to_string())
                .unwrap_or_default();
            return Some((usize::from(range.start()), usize::from(range.end()), name));
        }
    }
    None
}

type CallableIndex<'a> = BTreeMap<(String, String, usize), Vec<&'a NativeSite>>;

fn join_inventory_callable<'a>(
    index: &CallableIndex<'a>,
    path: &str,
    name: &str,
    compiler_start: usize,
    compiler_end: usize,
) -> Option<&'a NativeSite> {
    let mut matches: Vec<&NativeSite> = index
        .get(&(path.to_owned(), name.to_owned(), compiler_end))
        .into_iter()
        .flatten()
        .copied()
        .filter(|node| {
            matches!(node.kind, NodeKind::Callable | NodeKind::Declaration)
                && node.path.as_deref() == Some(path)
                && node.label == name
                && node.end_byte == Some(compiler_end)
                && node
                    .start_byte
                    .is_some_and(|start| compiler_start <= start && start < compiler_end)
        })
        .collect();
    if matches.len() == 1 {
        return matches.pop();
    }
    None
}

fn find_call_at_span(
    source_file: &ast::SourceFile,
    start: usize,
    end: usize,
) -> Option<ra_ap_syntax::SyntaxNode> {
    if start >= end {
        return None;
    }
    let want = TextRange::new(
        TextSize::from(u32::try_from(start).ok()?),
        TextSize::from(u32::try_from(end).ok()?),
    );
    if !source_file.syntax().text_range().contains_range(want) {
        return None;
    }
    let mut best: Option<ra_ap_syntax::SyntaxNode> = None;
    for node in source_file.syntax().covering_element(want).ancestors() {
        let range = node.text_range();
        if range != want {
            continue;
        }
        let interesting = ast::CallExpr::can_cast(node.kind())
            || ast::MethodCallExpr::can_cast(node.kind())
            || node.kind() == SyntaxKind::PATH
            || node.kind() == SyntaxKind::NAME_REF;
        if !interesting {
            continue;
        }
        let replace = best
            .as_ref()
            .map(|current| current.text_range().len() >= range.len())
            .unwrap_or(true);
        if replace {
            best = Some(node);
        }
    }
    best
}

fn collect_workspace(
    request: &NativeProduceRequest,
    root: &Path,
    workspace: &WorkspaceFiles,
    graph: &NativeInventory,
    watch: &BudgetWatch,
    raw: &mut NativeRawObservation,
) -> Result<(), NativeProducerError> {
    watch.check_wall()?;
    let cargo_config = cargo_config(request, &workspace.root)?;
    let load_config = load_config(request.execution_policy);
    let progress = |_message: String| {};
    let path = workspace
        .root
        .to_str()
        .ok_or_else(|| NativeProducerError::Message("Rust workspace path is not UTF-8".into()))?;
    let path = AbsPathBuf::try_from(path)
        .map_err(|_| NativeProducerError::Message("Rust workspace path is not absolute".into()))?;
    let mut model = ProjectManifest::discover_single(&path)
        .and_then(|manifest| ProjectWorkspace::load(manifest, &cargo_config, &progress))
        .map_err(|error| NativeProducerError::Message(format!("Rust workspace load: {error:#}")))?;
    // The IDE loader deliberately tolerates failed target queries and partial
    // metadata. Those fallbacks cannot qualify native evidence for a requested
    // target. Inspect its model before consuming the compiler database.
    let target_error = model.target.as_ref().err().map(ToString::to_string);
    let metadata_error = match &model.kind {
        ProjectWorkspaceKind::Cargo { error, .. } => error.as_ref().map(ToString::to_string),
        _ => None,
    };
    raw.build_context
        .entry("workspace_qualification")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| NativeProducerError::Artifact("invalid workspace qualification".into()))?
        .push(serde_json::json!({
            "root": workspace.root,
            "toolchain": model.toolchain.as_ref().map(ToString::to_string),
            "sysroot": model.sysroot.root().map(|path| path.as_str()),
            "sysroot_source": model.sysroot.rust_lib_src_root().map(|path| path.as_str()),
            "target_error": target_error, "metadata_error": metadata_error,
            "sysroot_error": model.sysroot.error(),
            "sysroot_metadata_error": model.sysroot.metadata_error(),
            "partial": model.sysroot.error().is_some() || model.sysroot.metadata_error().is_some()
        }));
    if let Some(error) = target_error.or(metadata_error) {
        return Err(NativeProducerError::Message(format!(
            "Rust target/metadata qualification failed: {error}"
        )));
    }
    if model.sysroot.is_rust_lib_src_empty() {
        return Err(NativeProducerError::Message(
            "Rust sysroot source unavailable; install rust-src explicitly for the selected toolchain".into(),
        ));
    }
    if load_config.load_out_dirs_from_check {
        let scripts = model
            .run_build_scripts(&cargo_config, &progress)
            .map_err(|error| {
                NativeProducerError::Message(format!("Rust build context: {error:#}"))
            })?;
        if let Some(error) = scripts.error() {
            return Err(NativeProducerError::Message(format!(
                "Rust build context: {error}"
            )));
        }
        model.set_build_scripts(scripts);
    }
    let contexts = target_contexts(&model, &request.rust_profile.analysis, root)?;
    let loaded = load_workspace(model, &cargo_config.extra_env, &load_config);
    let (mut db, vfs, _proc_macros) = match loaded {
        Ok(value) => value,
        Err(error) => {
            raw.invalid_environment = true;
            raw.diagnostics.push(format!(
                "failed to load workspace {}: {error}",
                workspace.root.display()
            ));
            return Ok(());
        }
    };

    apply_target_contexts(&mut db, &vfs, &contexts, raw)?;
    let file_ids = inventory_file_ids(&vfs, root);
    let mut input_digests = BTreeMap::new();
    for (file, path) in vfs.iter() {
        watch.check_wall()?;
        let Some(path) = path.as_path() else { continue };
        let bytes = std::fs::read(path.as_str())?;
        // The actual compiler database text must agree with retained physical
        // source. This includes dependencies and generated files in the VFS.
        if db.file_text(file).text(&db).as_bytes() != bytes {
            return Err(NativeProducerError::Message(format!(
                "Rust compiler/source input mismatch: {path}"
            )));
        }
        input_digests.insert(path.to_string(), entmoot_contracts::blake3_hex(&bytes));
    }
    watch.check_files(input_digests.len())?;
    let outcome = ra_ap_hir::attach_db(&db, || {
        collect_workspace_attached(
            root,
            workspace,
            graph,
            watch,
            raw,
            &db,
            &vfs,
            &file_ids,
            request.canon,
        )
    });
    for (path, expected) in &input_digests {
        watch.check_wall()?;
        if entmoot_contracts::blake3_hex(&std::fs::read(path)?) != *expected {
            return Err(NativeProducerError::Message(format!(
                "Rust input changed during bind: {path}"
            )));
        }
        raw.inputs.push(path.clone());
    }
    raw.build_context.entry("workspace_source_inputs").or_insert_with(|| serde_json::json!([]))
        .as_array_mut().ok_or_else(|| NativeProducerError::Artifact("invalid source input capture".into()))?
        .push(serde_json::json!({"root": workspace.root, "input_digests": input_digests,
            "coverage": "loaded_vfs_source_text_revalidated",
            "limitations": ["cargo_metadata_negative_lookups_and_build_process_environment_remain_open"]}));
    outcome
}

#[allow(clippy::too_many_arguments)] // One compiler database/VFS context per workspace.
fn collect_workspace_attached(
    root: &Path,
    workspace: &WorkspaceFiles,
    graph: &NativeInventory,
    watch: &BudgetWatch,
    raw: &mut NativeRawObservation,
    db: &RootDatabase,
    vfs: &Vfs,
    file_ids: &BTreeMap<PathBuf, Option<FileId>>,
    canon: bool,
) -> Result<(), NativeProducerError> {
    let sema = Semantics::new(db);
    let mut api = RustPackageApi::default();
    let mut retained_expansion_bytes = 0usize;
    let mut callable_index = CallableIndex::new();
    let workspace_files: BTreeSet<_> = workspace.files.iter().map(String::as_str).collect();
    for node in &graph.nodes {
        if matches!(node.kind, NodeKind::Callable | NodeKind::Declaration) {
            if let (Some(path), Some(end)) = (&node.path, node.end_byte) {
                callable_index
                    .entry((path.clone(), node.label.clone(), end))
                    .or_default()
                    .push(node);
            }
        }
    }

    let mut sites_by_file: BTreeMap<String, Vec<&NativeSite>> = BTreeMap::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::CallStub {
            continue;
        }
        let Some(path) = node.path.as_ref() else {
            continue;
        };
        if !path.ends_with(".rs") || !workspace_files.contains(path.as_str()) {
            continue;
        }
        sites_by_file.entry(path.clone()).or_default().push(node);
    }

    // Export-only files must participate even when they contain no call sites.
    for file in &workspace.files {
        sites_by_file.entry(file.clone()).or_default();
    }
    for (relative, sites) in sites_by_file {
        watch.check_wall()?;
        let absolute = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Some(file_id) = file_ids.get(&absolute).copied().flatten() else {
            raw.diagnostics.push(format!(
                "file not present in rust-analyzer VFS: {relative} (or ambiguous physical path)"
            ));
            raw.build_context.insert("partial".into(), true.into());
            continue;
        };
        raw.inputs.push(absolute.to_string_lossy().into_owned());
        if sema.file_to_module_defs(file_id).count() != 1 {
            raw.diagnostics.push(format!(
                "Rust source has no unique active module context: {relative}"
            ));
            raw.build_context.insert("partial".into(), true.into());
            continue;
        }
        let editioned = sema.attach_first_edition(file_id);
        let source_file = sema.parse(editioned);
        if canon {
            for syntax in source_file
                .syntax()
                .descendants()
                .filter(|n| ast::Expr::can_cast(n.kind()) || ast::IdentPat::can_cast(n.kind()))
                .take(100_000)
            {
                watch.check_wall()?;
                let ty = if let Some(expression) = ast::Expr::cast(syntax.clone()) {
                    sema.type_of_expr(&expression).map(|t| t.original)
                } else {
                    ast::IdentPat::cast(syntax.clone())
                        .and_then(|p| sema.type_of_binding_in_pat(&p))
                };
                let Some(ty) = ty else {
                    continue;
                };
                let Some(builtin) = ty.as_builtin() else {
                    continue;
                };
                let name = builtin
                    .name()
                    .display(sema.db, Edition::CURRENT)
                    .to_string();
                let sort = if ty.is_bool() {
                    serde_json::json!({"kind":"bool"})
                } else if ty.is_int_or_uint()
                    && matches!(
                        name.as_str(),
                        "i8" | "i16"
                            | "i32"
                            | "i64"
                            | "i128"
                            | "u8"
                            | "u16"
                            | "u32"
                            | "u64"
                            | "u128"
                    )
                {
                    serde_json::json!({"kind":"integer", "bits": name[1..].parse::<u16>().ok(), "signed":name.starts_with('i')})
                } else {
                    continue;
                };
                let range = syntax.text_range();
                raw.build_context.entry("canon_expressions").or_insert_with(|| serde_json::json!([]))
                    .as_array_mut().ok_or_else(|| NativeProducerError::Artifact("invalid canon scalar capture".into()))?
                    .push(serde_json::json!({"path":relative, "start_byte":usize::from(range.start()), "end_byte":usize::from(range.end()), "sort":sort, "overflow":null}));
            }
        }
        for call in source_file
            .syntax()
            .descendants()
            .filter_map(ast::MacroCall::cast)
        {
            watch.check_wall()?;
            let expansion = sema
                .expand_macro_call(&call)
                .map(|expanded| expanded.value.text().to_string());
            let range = call.syntax().text_range();
            let retained = expansion.as_ref().filter(|text| {
                text.len() <= 1_048_576 && retained_expansion_bytes + text.len() <= 32 * 1_048_576
            });
            retained_expansion_bytes += retained.map_or(0, String::len);
            raw.build_context.entry("macro_expansions").or_insert_with(|| serde_json::json!([]))
                .as_array_mut().ok_or_else(|| NativeProducerError::Artifact("invalid macro capture".into()))?
                .push(serde_json::json!({
                    "path": relative, "start_byte": usize::from(range.start()), "end_byte": usize::from(range.end()),
                    "spelling": call.syntax().text().to_string(),
                    "expanded_text": retained,
                    "expanded_bytes": expansion.as_ref().map(String::len),
                    "expanded_digest": expansion.as_ref().map(|text| entmoot_contracts::blake3_hex(text.as_bytes())),
                    "outcome": if retained.is_some() { "expanded_virtual_source" } else if expansion.is_some() { "expansion_capture_budget_exceeded" } else { "expansion_unavailable" },
                    "limitations": ["expansion_offsets_are_not_physical_inventory_addresses"]
                }));
        }
        for node in source_file.syntax().descendants() {
            use ra_ap_hir::{Adt, ModuleDef};
            let definition = if let Some(item) = ast::Struct::cast(node.clone()) {
                sema.to_def(&item)
                    .map(|item| ModuleDef::Adt(Adt::Struct(item)))
            } else if let Some(item) = ast::Enum::cast(node.clone()) {
                sema.to_def(&item)
                    .map(|item| ModuleDef::Adt(Adt::Enum(item)))
            } else if let Some(item) = ast::Union::cast(node.clone()) {
                sema.to_def(&item)
                    .map(|item| ModuleDef::Adt(Adt::Union(item)))
            } else if let Some(item) = ast::Trait::cast(node.clone()) {
                sema.to_def(&item).map(ModuleDef::Trait)
            } else if let Some(item) = ast::TypeAlias::cast(node.clone()) {
                sema.to_def(&item).map(ModuleDef::TypeAlias)
            } else if let Some(item) = ast::Macro::cast(node.clone()) {
                sema.to_def(&item).map(ModuleDef::Macro)
            } else {
                None
            };
            let Some(definition) = definition else {
                continue;
            };
            let exported_macro = match definition {
                ModuleDef::Macro(macro_) if macro_.attrs(sema.db).is_macro_export() => Some(macro_),
                ModuleDef::Macro(_) => continue,
                _ => None,
            };
            if definition.visibility(sema.db) != ra_ap_hir::Visibility::Public {
                continue;
            }
            let Some(module) = definition.module(sema.db) else {
                continue;
            };
            if exported_macro.is_none()
                && module.path_to_root(sema.db).iter().any(|module| {
                    !module.is_crate_root(sema.db)
                        && module.visibility(sema.db) != ra_ap_hir::Visibility::Public
                })
            {
                continue;
            }
            let package = match module.krate(sema.db).origin(sema.db) {
                ra_ap_base_db::CrateOrigin::Local {
                    name: Some(name), ..
                }
                | ra_ap_base_db::CrateOrigin::Library { name, .. } => name.to_string(),
                _ => continue,
            };
            let qualified = exported_macro
                .map(|macro_| {
                    macro_
                        .name(sema.db)
                        .display(sema.db, Edition::CURRENT)
                        .to_string()
                })
                .or_else(|| definition.canonical_path(sema.db, Edition::CURRENT));
            let Some(qualified) = qualified else {
                continue;
            };
            let range = node.text_range();
            let contract = declaration_contract(&node);
            let symbol = format!("{package}::{qualified}");
            api.exports.entry(package).or_default().insert(symbol.clone(), serde_json::json!({
                "path": relative, "start_byte": usize::from(range.start()), "end_byte": usize::from(range.end()),
                "symbol": symbol, "boundary_policy": RUST_TYPE_POLICY, "boundary_text": contract
            }));
        }
        for declaration in source_file.syntax().descendants().filter_map(ast::Fn::cast) {
            let Some(function) = sema.to_def(&declaration) else {
                continue;
            };
            if !public_module_route(&sema, function.module(sema.db)) {
                continue;
            }
            let Some(target) = function_span(&sema, vfs, root, function, &mut api) else {
                continue;
            };
            let Some((package, header)) = api.boundaries.get(&target.symbol) else {
                continue;
            };
            let Some(path) = target.location.repository().map(|site| site.path.as_str()) else {
                continue;
            };
            let name = declaration
                .name()
                .map(|name| name.text().to_string())
                .unwrap_or_default();
            let Some(node) = join_inventory_callable(
                &callable_index,
                path,
                &name,
                target.location.span().start_byte,
                target.location.span().end_byte,
            ) else {
                continue;
            };
            api.exports.entry(package.clone()).or_default().insert(target.symbol.clone(), serde_json::json!({
                "path": path, "start_byte": node.start_byte, "end_byte": node.end_byte,
                "symbol": target.symbol, "boundary_policy": RUST_BOUNDARY_POLICY, "boundary_text": header
            }));
        }
        for use_item in source_file
            .syntax()
            .descendants()
            .filter_map(ast::Use::cast)
        {
            collect_use_bindings(
                &RustUseContext {
                    sema: &sema,
                    vfs,
                    root,
                    callable_index: &callable_index,
                },
                &relative,
                &use_item,
                &mut api,
                raw,
            );
        }
        // Compiler-resolved function values are positive uses even when no call
        // edge can describe their eventual invocation. Never resolve fn pointers
        // or physical offsets from virtual macro expansions as declarations.
        for expression in source_file
            .syntax()
            .descendants()
            .filter_map(ast::PathExpr::cast)
        {
            watch.check_wall()?;
            if expression
                .syntax()
                .parent()
                .and_then(ast::CallExpr::cast)
                .and_then(|call| call.expr())
                .is_some_and(|callee| callee.syntax() == expression.syntax())
            {
                continue;
            }
            let Some(path) = expression.path() else {
                continue;
            };
            let Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(function))) =
                sema.resolve_path(&path)
            else {
                continue;
            };
            // Name resolution also works in disabled syntax. An inferred HIR
            // expression must exist in the selected body/configuration before
            // this can count as a use (including consts and cfg'd statements).
            if sema.type_of_expr(&expression.clone().into()).is_none() {
                continue;
            }
            let Some(mut target) = function_span(&sema, vfs, root, function, &mut api) else {
                continue;
            };
            let Some(site) = target.location.repository() else {
                continue;
            };
            let name = function
                .name(sema.db)
                .display(sema.db, Edition::CURRENT)
                .to_string();
            let Some(node) = join_inventory_callable(
                &callable_index,
                &site.path,
                &name,
                site.span.start_byte,
                site.span.end_byte,
            ) else {
                continue;
            };
            let (Some(start_byte), Some(end_byte)) = (node.start_byte, node.end_byte) else {
                continue;
            };
            if let Some(site) = target.location.repository_mut() {
                site.span = entmoot_contracts::SiteSpan {
                    start_byte,
                    end_byte,
                };
            }
            let range = expression.syntax().text_range();
            raw.references.push(NativeReferenceFact {
                site: NativeSpanRef {
                    path: relative.clone(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: usize::from(range.start()),
                        end_byte: usize::from(range.end()),
                    },
                },
                kind: "reference".into(),
                spelling: expression.syntax().text().to_string(),
                targets: vec![target],
                owner: None,
                gap: None,
                limitation: Some(
                    "positive function-value reference; eventual invocation remains open".into(),
                ),
            });
            watch.check_targets(raw.references.len())?;
        }
        for site in sites {
            let (Some(start), Some(end)) = (site.start_byte, site.end_byte) else {
                continue;
            };
            let Some(node) = find_call_at_span(&source_file, start, end) else {
                continue;
            };
            // IDE path lookup can still find a visible symbol from syntax in
            // an inactive function. Require the compiler to materialize every
            // enclosing function before treating that lookup as native evidence.
            if node
                .ancestors()
                .filter_map(ast::Fn::cast)
                .any(|function| sema.to_def(&function).is_none())
            {
                raw.build_context.insert("partial".into(), true.into());
                raw.diagnostics.push(format!(
                    "unassessed Rust call {relative}:{start}..{end}: enclosing function absent under selected cfg"
                ));
                continue;
            }
            let mut owner = None;
            if let Some((compiler_start, compiler_end, name)) = enclosing_fn_span(&node) {
                if let Some(joined) = join_inventory_callable(
                    &callable_index,
                    &relative,
                    &name,
                    compiler_start,
                    compiler_end,
                ) {
                    owner = Some(NativeSpanRef {
                        path: relative.clone(),
                        span: entmoot_contracts::SiteSpan {
                            start_byte: joined.start_byte.unwrap_or(compiler_start),
                            end_byte: joined.end_byte.unwrap_or(compiler_end),
                        },
                    });
                } else {
                    // Keep compiler span for diagnostics, but mark unsupported via join mismatch.
                    owner = Some(NativeSpanRef {
                        path: relative.clone(),
                        span: entmoot_contracts::SiteSpan {
                            start_byte: compiler_start,
                            end_byte: compiler_end,
                        },
                    });
                }
            }
            let resolved = if let Some(call) = ast::CallExpr::cast(node.clone()) {
                resolve_call_expr(&sema, vfs, root, &call, &mut api)
            } else if let Some(call) = ast::MethodCallExpr::cast(node.clone()) {
                resolve_method_call(&sema, vfs, root, &call, &mut api)
            } else if let Some(path_expr) = node
                .ancestors()
                .find_map(ast::CallExpr::cast)
                .and_then(|call| call.expr())
                .and_then(|expr| ast::PathExpr::cast(expr.syntax().clone()))
            {
                let path = match path_expr.path() {
                    Some(path) => path,
                    None => continue,
                };
                match sema.resolve_path(&path) {
                    Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(function))) => {
                        Ok(function_span(&sema, vfs, root, function, &mut api)
                            .into_iter()
                            .collect())
                    }
                    Some(PathResolution::Local(_)) => Err(Some("indirect_call")),
                    Some(_) => Err(Some("unsupported_construct")),
                    None => Err(None),
                }
            } else if let Some(method) = node.ancestors().find_map(ast::MethodCallExpr::cast) {
                resolve_method_call(&sema, vfs, root, &method, &mut api)
            } else {
                Err(None)
            };

            let mut fact = NativeCallFact {
                dispatch: Default::default(),
                site: entmoot_contracts::NativeSpanRef {
                    path: relative.clone(),
                    span: entmoot_contracts::SiteSpan {
                        start_byte: start,
                        end_byte: end,
                    },
                },
                targets: Vec::new(),
                owner,
                gap: None,
                limitation: None,
                intrinsic: false,
            };
            match resolved {
                Ok(targets) if !targets.is_empty() => {
                    fact.targets = targets;
                }
                Ok(_) => {
                    fact.gap = Some("unsupported_construct".into());
                    fact.limitation = Some("definition resolved without a callable span".into());
                }
                Err(Some(gap)) => {
                    fact.gap = Some(gap.into());
                    fact.limitation = Some(format!("rust-analyzer left call open: {gap}"));
                }
                // Retain the inventory call site as an unresolved binding rather
                // than dropping native observation for a silent miss.
                Err(None) => {
                    fact.gap = Some("unresolved".into());
                    fact.limitation =
                        Some("rust-analyzer left call open without a dispatch mechanism".into());
                }
            }
            let bound_function = node
                .ancestors()
                .find_map(ast::MethodCallExpr::cast)
                .and_then(|call| sema.resolve_method_call(&call))
                .or_else(|| {
                    node.ancestors()
                        .find_map(ast::CallExpr::cast)
                        .and_then(|call| call.expr())
                        .and_then(|expr| ast::PathExpr::cast(expr.syntax().clone()))
                        .and_then(|expr| expr.path())
                        .and_then(|path| match sema.resolve_path(&path) {
                            Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(function))) => {
                                Some(function)
                            }
                            _ => None,
                        })
                });
            if bound_function
                .and_then(|function| function.as_assoc_item(sema.db))
                .is_some_and(|item| matches!(item.container(sema.db), AssocItemContainer::Trait(_)))
            {
                fact.gap = Some("dynamic_dispatch".into());
                fact.limitation = Some(
                    "native trait declaration binding does not identify every implementation"
                        .into(),
                );
            }
            // Closed policies mirror floors: free functions, inherent methods, and
            // associated functions (UFCS). Trait methods already set dynamic_dispatch.
            if fact.gap.is_none() && fact.owner.is_some() && fact.targets.len() == 1 {
                let call_range_ok = |call_range, expr_range| {
                    node.text_range() == call_range || node.text_range() == expr_range
                };
                let free_function =
                    node.ancestors()
                        .find_map(ast::CallExpr::cast)
                        .and_then(|call| {
                            let expression = call.expr()?;
                            if !call_range_ok(
                                call.syntax().text_range(),
                                expression.syntax().text_range(),
                            ) {
                                return None;
                            }
                            let path = ast::PathExpr::cast(expression.syntax().clone())?.path()?;
                            match sema.resolve_path(&path) {
                                Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(
                                    function,
                                ))) if function.as_assoc_item(sema.db).is_none() => Some(function),
                                _ => None,
                            }
                        });
                let inherent_method = node
                    .ancestors()
                    .find_map(ast::MethodCallExpr::cast)
                    .and_then(|call| {
                        if node.text_range() != call.syntax().text_range()
                            && node.text_range()
                                != call.name_ref().map(|name| name.syntax().text_range())?
                        {
                            return None;
                        }
                        let function = sema.resolve_method_call(&call)?;
                        let assoc = function.as_assoc_item(sema.db)?;
                        match assoc.container(sema.db) {
                            AssocItemContainer::Impl(_) => Some(function),
                            AssocItemContainer::Trait(_) => None,
                        }
                    });
                let associated_function =
                    node.ancestors()
                        .find_map(ast::CallExpr::cast)
                        .and_then(|call| {
                            let expression = call.expr()?;
                            if !call_range_ok(
                                call.syntax().text_range(),
                                expression.syntax().text_range(),
                            ) {
                                return None;
                            }
                            let path = ast::PathExpr::cast(expression.syntax().clone())?.path()?;
                            match sema.resolve_path(&path) {
                                Some(PathResolution::Def(ra_ap_hir::ModuleDef::Function(
                                    function,
                                ))) => {
                                    let assoc = function.as_assoc_item(sema.db)?;
                                    match assoc.container(sema.db) {
                                        AssocItemContainer::Impl(_) => Some(function),
                                        AssocItemContainer::Trait(_) => None,
                                    }
                                }
                                _ => None,
                            }
                        });
                let closed = if free_function.is_some() {
                    Some((
                        "entmoot.rust.direct-function.v1",
                        "source-level direct free-function invocation; implicit calls and foreign replacement are outside this qualification",
                    ))
                } else if inherent_method.is_some() {
                    Some((
                        "entmoot.rust.direct-inherent.v1",
                        "source-level inherent method invocation on a concrete receiver; trait object, foreign, and dyn replacement are outside this qualification",
                    ))
                } else if associated_function.is_some() {
                    Some((
                        "entmoot.rust.direct-associated.v1",
                        "source-level associated function (UFCS) on an impl; trait methods, foreign, and dyn replacement are outside this qualification",
                    ))
                } else {
                    None
                };
                if let Some((policy_id, fence)) = closed {
                    fact.dispatch = entmoot_contracts::CallDispatch::Closed {
                        policy_id: policy_id.into(),
                        evidence_pointer: format!("/calls/{}/dispatch", raw.calls.len()),
                        assumptions: vec![
                            "selected Cargo target, features and cfg".into(),
                            fence.into(),
                        ],
                    };
                }
            }
            if let [target] = fact.targets.as_slice() {
                if target.location.repository().is_none() && fact.gap.is_none() {
                    if let Some((package, header)) = api.boundaries.get(&target.symbol) {
                        api.references.push(serde_json::json!({
                            "path": fact.site.path, "start_byte": fact.site.span.start_byte, "end_byte": fact.site.span.end_byte,
                            "target": {"package": {"ecosystem": "rust", "name": package},
                                "symbol": {"scheme": "rust.compiler.symbol", "spelling": target.symbol}},
                            "boundary_policy": RUST_BOUNDARY_POLICY, "boundary_text": header
                        }));
                    }
                }
            }
            raw.calls.push(fact);
        }
    }
    let evidence = raw.build_context.entry("package_api").or_insert_with(|| serde_json::json!({
        "surfaces": [], "references": [], "references_complete": false,
        "limitations": ["rust_public_source_declarations_and_qualified_reexports", "expansion_and_trait_dispatch_do_not_establish_runtime_or_build_closure"]
    }));
    if let Some(surfaces) = evidence
        .get_mut("surfaces")
        .and_then(serde_json::Value::as_array_mut)
    {
        for (package, exports) in api.exports {
            // Repeated workspace views must agree; never silently discard exports.
            if let Some(surface) = surfaces
                .iter_mut()
                .find(|surface| surface["package"]["name"].as_str() == Some(&package))
            {
                let existing = surface["exports"].as_array_mut().ok_or_else(|| {
                    NativeProducerError::Artifact("invalid Rust package surface".into())
                })?;
                for export in exports.into_values() {
                    if let Some(previous) = existing
                        .iter()
                        .find(|row| row["symbol"] == export["symbol"])
                    {
                        if previous != &export {
                            return Err(NativeProducerError::Artifact(
                                "conflicting Rust package export views".into(),
                            ));
                        }
                    } else {
                        existing.push(export);
                    }
                }
                existing.sort_by(|a, b| a["symbol"].as_str().cmp(&b["symbol"].as_str()));
                continue;
            }
            surfaces.push(serde_json::json!({"package": {"ecosystem": "rust", "name": package},
                "export_universes": [{"symbol_scheme": "rust.compiler.symbol", "boundary_policy": RUST_BOUNDARY_POLICY},
                    {"symbol_scheme": "rust.compiler.symbol", "boundary_policy": RUST_TYPE_POLICY}],
                "completeness": "partial", "exports": exports.into_values().collect::<Vec<_>>(),
                "limitations": ["selected_source_and_build_configuration_only"]}));
        }
    }
    if let Some(references) = evidence
        .get_mut("references")
        .and_then(serde_json::Value::as_array_mut)
    {
        references.extend(api.references);
    }
    Ok(())
}

/// Collect Rust call observations for inventoried sites.
pub fn collect(
    request: &NativeProduceRequest,
    graph: &NativeInventory,
    watch: &BudgetWatch,
) -> Result<NativeRawObservation, NativeProducerError> {
    // Cargo propagates its workspace spelling into OUT_DIR. MSVC rejects the
    // verbatim paths produced by Windows canonicalize when compiling grammars.
    let root = canonical_source_path(&request.source_root)?;
    let rust_files: Vec<String> = graph
        .snapshot_files
        .iter()
        .filter(|file| file.path.ends_with(".rs"))
        .map(|file| file.path.clone())
        .collect();
    watch.check_files(rust_files.len())?;

    let mut raw = NativeRawObservation::default();
    raw.build_context.insert(
        "rust_profile".into(),
        serde_json::to_value(&request.rust_profile)?,
    );
    if rust_files.is_empty() {
        raw.diagnostics
            .push("no Rust snapshot files to analyze".into());
        return Ok(raw);
    }

    request
        .rust_profile
        .validate()
        .map_err(|error| NativeProducerError::Message(error.into()))?;
    let mut workspaces = discover_workspaces(&root, &rust_files);
    if let RustAnalysisMode::Test { crate_root } = &request.rust_profile.analysis {
        workspaces.retain(|workspace| workspace.files.contains(crate_root));
        raw.build_context.insert("partial".into(), true.into());
        raw.diagnostics.push("Rust test profile enables cfg(test) only for one target; Cargo feature/dev-dependency resolution remains the IDE model, not compiler acceptance".into());
    }
    if workspaces.is_empty() {
        raw.invalid_environment = true;
        raw.diagnostics
            .push("no Cargo.toml manifests discovered for Rust inventory".into());
        return Ok(raw);
    }

    for workspace in &workspaces {
        if let Err(error) = collect_workspace(request, &root, workspace, graph, watch, &mut raw) {
            match error {
                NativeProducerError::Budget(_) => {
                    raw.budget_exceeded = true;
                    raw.diagnostics.push(error.to_string());
                    break;
                }
                other => {
                    raw.invalid_environment = true;
                    raw.diagnostics.push(other.to_string());
                }
            }
        }
    }

    let versions: BTreeSet<_> = raw
        .build_context
        .get("workspace_qualification")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|workspace| {
            workspace
                .get("toolchain")
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    if versions.len() == 1 {
        raw.compiler_version = versions.into_iter().next().map(str::to_owned);
    }
    raw.inputs.sort();
    raw.inputs.dedup();
    raw.calls.sort_by(|a, b| {
        (&a.site.path, a.site.span.start_byte).cmp(&(&b.site.path, b.site.span.start_byte))
    });
    raw.references.sort_by(|a, b| {
        (&a.site.path, a.site.span.start_byte, &a.kind).cmp(&(
            &b.site.path,
            b.site.span.start_byte,
            &b.kind,
        ))
    });
    Ok(raw)
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn site_filtered_lookup_matches_full_walk_for_nested_calls_and_paths() {
        let source = ast::SourceFile::parse(
            "fn f() { outer(inner(1), obj.method()); (factory())(); }",
            Edition::CURRENT,
        )
        .tree();
        for node in source.syntax().descendants() {
            let range = node.text_range();
            if range.is_empty() {
                continue;
            }
            let mut expected = None;
            for candidate in source.syntax().descendants() {
                if candidate.text_range() == range
                    && (ast::CallExpr::can_cast(candidate.kind())
                        || ast::MethodCallExpr::can_cast(candidate.kind())
                        || candidate.kind() == SyntaxKind::PATH
                        || candidate.kind() == SyntaxKind::NAME_REF)
                    && expected
                        .as_ref()
                        .is_none_or(|current: &ra_ap_syntax::SyntaxNode| {
                            current.text_range().len() > candidate.text_range().len()
                        })
                {
                    expected = Some(candidate);
                }
            }
            assert_eq!(
                find_call_at_span(
                    &source,
                    usize::from(range.start()),
                    usize::from(range.end())
                ),
                expected
            );
        }
        assert!(find_call_at_span(&source, 99, 100).is_none());
        assert!(find_call_at_span(&source, 2, 1).is_none());
    }

    #[test]
    fn vfs_physical_paths_are_scoped_and_duplicates_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = canonical_source_path(directory.path()).unwrap();
        let path = root.join("input.rs");
        std::fs::write(&path, b"fn f() {}").unwrap();
        let mut vfs = Vfs::default();
        let real = ra_ap_vfs::VfsPath::new_real_path(
            std::fs::canonicalize(&path)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
        vfs.set_file_contents(real, Some(b"fn f() {}".to_vec()));
        let files = inventory_file_ids(&vfs, &root);
        assert_eq!(files.len(), 1);
        assert!(files[&path].is_some());
        assert!(inventory_file_ids(&vfs, &root.join("other")).is_empty());
        #[cfg(windows)]
        {
            let plain = path.to_string_lossy().into_owned();
            vfs.set_file_contents(
                ra_ap_vfs::VfsPath::new_real_path(plain),
                Some(b"fn g() {}".to_vec()),
            );
            assert_eq!(vfs.iter().count(), 2);
            assert_eq!(inventory_file_ids(&vfs, &root)[&path], None);
        }
    }
}
