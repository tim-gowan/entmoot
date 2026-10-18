//! Bounded literal npm export routes with Oxc declaration evidence.
use crate::native_resolver::{
    budget::BudgetWatch,
    package_api_seek::{EnrichedBoundary, PackageRootResolver},
    pipeline::NativeProducerError,
};
use entmoot_contracts::{package_api::PackageApiPackage, NativeRawObservation};
use oxc::{
    allocator::Allocator,
    ast::ast::{
        AssignmentTarget, BindingPattern, Class, ClassElement, Declaration,
        ExportDefaultDeclarationKind, Expression, Function, Statement,
    },
    parser::Parser,
    semantic::SemanticBuilder,
    span::{GetSpan, SourceType},
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const POLICY: &str = "oxc.esm_function_header_utf8.v1";
const CLASS_POLICY: &str = "oxc.class_method_headers_utf8.v1";

/// npm layout package-root enricher (`node_modules/<name>` under the consumer tree).
pub struct NpmPackageRootResolver<'a> {
    root: &'a Path,
    watch: &'a BudgetWatch,
    inputs: RefCell<BTreeMap<PathBuf, String>>,
    packages: RefCell<BTreeMap<PathBuf, Option<Package>>>,
    /// Package names attempted under an observed `node_modules` layout.
    attempted: RefCell<BTreeSet<String>>,
    /// Package names successfully enriched from a package root.
    resolved: RefCell<BTreeSet<String>>,
    /// True when any `node_modules` directory was observed under the consumer.
    saw_node_modules: RefCell<bool>,
}

impl<'a> NpmPackageRootResolver<'a> {
    pub fn new(root: &'a Path, watch: &'a BudgetWatch) -> Self {
        Self {
            root,
            watch,
            inputs: RefCell::new(BTreeMap::new()),
            packages: RefCell::new(BTreeMap::new()),
            attempted: RefCell::new(BTreeSet::new()),
            resolved: RefCell::new(BTreeSet::new()),
            saw_node_modules: RefCell::new(false),
        }
    }

    pub fn finish(self, raw: &mut NativeRawObservation) -> Result<(), NativeProducerError> {
        let Self {
            root: _,
            watch,
            inputs,
            packages: _,
            attempted,
            resolved,
            saw_node_modules,
        } = self;
        let inputs = inputs.into_inner();
        for (path, digest) in &inputs {
            watch.check_wall()?;
            if entmoot_contracts::blake3_hex(&std::fs::read(path)?) != *digest {
                return Err(NativeProducerError::Drift(format!(
                    "package input changed: {}",
                    path.display()
                )));
            }
            raw.inputs.push(path.to_string_lossy().into_owned());
        }
        if !inputs.is_empty() {
            let mut digests = raw
                .build_context
                .get("package_input_digests")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Some(object) = digests.as_object_mut() {
                for (path, digest) in &inputs {
                    object.insert(path.to_string_lossy().into_owned(), json!(digest));
                }
            }
            raw.build_context
                .insert("package_input_digests".into(), digests);
        }
        if saw_node_modules.into_inner() {
            let resolved = resolved.into_inner();
            let missing: Vec<String> = attempted
                .into_inner()
                .into_iter()
                .filter(|name| !resolved.contains(name))
                .collect();
            if !missing.is_empty() {
                let api = raw
                    .build_context
                    .entry("package_api".to_string())
                    .or_insert_with(|| {
                        json!({
                            "surfaces": [],
                            "references": [],
                            "references_complete": false,
                            "limitations": []
                        })
                    });
                let limitations = api
                    .as_object_mut()
                    .ok_or_else(|| {
                        NativeProducerError::Artifact("package_api must be an object".into())
                    })?
                    .entry("limitations")
                    .or_insert_with(|| json!([]));
                let list = limitations.as_array_mut().ok_or_else(|| {
                    NativeProducerError::Artifact("package_api.limitations must be an array".into())
                })?;
                for name in missing {
                    list.push(json!(format!("mapped_package_root_missing:{name}")));
                }
            }
        }
        Ok(())
    }
}

impl PackageRootResolver for NpmPackageRootResolver<'_> {
    fn enrich(
        &self,
        source_root: &Path,
        pkg: &PackageApiPackage,
        export_spelling: &str,
        call_path: &str,
    ) -> Result<Option<EnrichedBoundary>, NativeProducerError> {
        if pkg.ecosystem != entmoot_contracts::package_api::PackageApiEcosystem::Javascript {
            return Ok(None);
        }
        let name = &pkg.name;
        if name.contains(['\\', ':']) || name.split('/').any(|part| part == ".." || part.is_empty())
        {
            return Ok(None);
        }
        self.attempted.borrow_mut().insert(name.clone());
        let mut inputs = self.inputs.borrow_mut();
        let mut packages = self.packages.borrow_mut();
        for parent in source_root.join(call_path).ancestors().skip(1) {
            if !parent.starts_with(self.root) {
                break;
            }
            let node_modules = parent.join("node_modules");
            if node_modules.is_dir() {
                *self.saw_node_modules.borrow_mut() = true;
            }
            let directory = node_modules.join(name);
            if !directory.join("package.json").is_file() {
                continue;
            }
            let directory = directory.canonicalize()?;
            if !directory.starts_with(self.root) {
                break;
            }
            if !packages.contains_key(&directory) {
                packages.insert(
                    directory.clone(),
                    package(&directory, &mut inputs, self.watch)?,
                );
            }
            if let Some(Some(loaded)) = packages.get(&directory) {
                if &loaded.name == name {
                    if let Some(export) = loaded.exports.get(export_spelling) {
                        let Some(policy) = export["boundary_policy"].as_str() else {
                            return Ok(None);
                        };
                        let Some(text) = export["boundary_text"].as_str() else {
                            return Ok(None);
                        };
                        self.resolved.borrow_mut().insert(name.clone());
                        return Ok(Some(EnrichedBoundary {
                            boundary_policy: policy.to_owned(),
                            boundary_text: text.to_owned(),
                        }));
                    }
                    // Package root present but export spelling not in menu — still
                    // install-observed; do not treat as mapped-root missing.
                    self.resolved.borrow_mut().insert(name.clone());
                }
            }
            break;
        }
        Ok(None)
    }
}

// Without a runtime condition set, only condition-invariant routes qualify.
// Distinct import/require/types/browser branches never collapse by key order.
fn invariant_route(value: &Value) -> Option<&str> {
    if let Some(path) = value.as_str() {
        return Some(path);
    }
    let branches = value.as_object()?;
    if !branches.contains_key("default") {
        return None;
    }
    let mut selected = None;
    for branch in branches.values() {
        let path = invariant_route(branch)?;
        if selected.is_some_and(|prior| prior != path) {
            return None;
        }
        selected = Some(path);
    }
    selected
}

fn function_export(relative: &str, source: &str, function: &Function<'_>) -> Option<Value> {
    let body = function.body.as_ref()?;
    let header = source.get(function.span.start as usize..body.span.start as usize)?;
    Some(
        json!({"path": relative, "start_byte": function.span.start, "end_byte": function.span.end,
        "boundary_policy": POLICY, "boundary_text": header}),
    )
}

fn class_export(relative: &str, source: &str, class: &Class<'_>) -> Option<Value> {
    if !class.decorators.is_empty() || class.heritage.is_some() {
        return None;
    }
    let mut header = source
        .get(class.span.start as usize..class.body.span.start as usize)?
        .to_owned();
    for element in &class.body.body {
        let ClassElement::MethodDefinition(method) = element else {
            return None;
        };
        if method.computed || !method.decorators.is_empty() {
            return None;
        }
        let end = method
            .value
            .body
            .as_ref()
            .map_or(method.span.end, |body| body.span.start);
        header.push('\n');
        header.push_str(source.get(method.span.start as usize..end as usize)?);
    }
    Some(
        json!({"path": relative, "start_byte": class.span.start, "end_byte": class.span.end,
        "boundary_policy": CLASS_POLICY, "boundary_text": header}),
    )
}

pub fn module_exports(
    root: &Path,
    file: &Path,
    inputs: &mut BTreeMap<PathBuf, String>,
    watch: &BudgetWatch,
    cache: &mut BTreeMap<PathBuf, BTreeMap<String, Value>>,
    active: &mut BTreeSet<PathBuf>,
) -> Result<BTreeMap<String, Value>, NativeProducerError> {
    watch.check_wall()?;
    let file = file.canonicalize()?;
    if !file.starts_with(root) || active.contains(&file) || active.len() >= 64 {
        return Ok(BTreeMap::new());
    }
    if let Some(exports) = cache.get(&file) {
        return Ok(exports.clone());
    }
    active.insert(file.clone());
    let relative = file
        .strip_prefix(root)
        .map_err(|e| NativeProducerError::Message(e.to_string()))?
        .to_string_lossy()
        .replace('\\', "/");
    let mut module_kind = match file.extension().and_then(|ext| ext.to_str()) {
        Some("mjs" | "mts") => Some(true),
        Some("cjs" | "cts") => Some(false),
        _ => None,
    };
    if module_kind.is_none() {
        for directory in file
            .ancestors()
            .skip(1)
            .take_while(|path| path.starts_with(root))
        {
            let metadata = directory.join("package.json");
            if metadata.is_file() {
                let metadata: Value = serde_json::from_str(&read(&metadata, inputs, watch)?)?;
                module_kind = match metadata["type"].as_str() {
                    Some("module") => Some(true),
                    Some("commonjs") => Some(false),
                    _ => None,
                };
                break;
            }
        }
    }
    let source = read(&file, inputs, watch)?;
    let Ok(source_type) = SourceType::from_path(&file) else {
        return Ok(BTreeMap::new());
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Ok(BTreeMap::new());
    }
    let semantic = SemanticBuilder::new().build(&parsed.program);
    if !semantic.diagnostics.is_empty() {
        return Ok(BTreeMap::new());
    }
    let scoping = semantic.semantic.scoping();
    let mut declarations = BTreeMap::new();
    let mut exports = BTreeMap::new();
    for statement in &parsed.program.body {
        let variable = match statement {
            Statement::VariableDeclaration(variable) => Some(variable),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(variable) => Some(variable),
                _ => None,
            },
            _ => None,
        };
        if let Some(variable) = variable {
            for binding in &variable.declarations {
                let BindingPattern::BindingIdentifier(id) = &binding.id else {
                    continue;
                };
                if scoping.symbol_is_mutated(id.symbol_id()) {
                    continue;
                }
                let value = match &binding.init {
                    Some(Expression::FunctionExpression(function)) => function_export(&relative, &source, function),
                    Some(Expression::ArrowFunctionExpression(function)) => {
                        source.get(function.span.start as usize..function.body.span().start as usize)
                            .map(|header| json!({
                                "path": relative, "start_byte": function.span.start, "end_byte": function.span.end,
                                "boundary_policy": "oxc.arrow_header_utf8.v1", "boundary_text": header,
                            }))
                    }
                    _ => None,
                };
                if let Some(value) = value {
                    declarations.insert(id.name.to_string(), value.clone());
                    if matches!(statement, Statement::ExportDeclaration(_)) {
                        exports.insert(id.name.to_string(), value);
                    }
                }
            }
        }
        let declaration = match statement {
            Statement::FunctionDeclaration(function) => function
                .id
                .as_ref()
                .filter(|id| !scoping.symbol_is_mutated(id.symbol_id()))
                .and_then(|id| {
                    function_export(&relative, &source, function)
                        .map(|value| (id.name.to_string(), value))
                }),
            Statement::ClassDeclaration(class) => class
                .id
                .as_ref()
                .filter(|id| !scoping.symbol_is_mutated(id.symbol_id()))
                .and_then(|id| {
                    class_export(&relative, &source, class)
                        .map(|value| (id.name.to_string(), value))
                }),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(function) => function
                    .id
                    .as_ref()
                    .filter(|id| !scoping.symbol_is_mutated(id.symbol_id()))
                    .and_then(|id| {
                        function_export(&relative, &source, function)
                            .map(|value| (id.name.to_string(), value))
                    }),
                Declaration::ClassDeclaration(class) => class
                    .id
                    .as_ref()
                    .filter(|id| !scoping.symbol_is_mutated(id.symbol_id()))
                    .and_then(|id| {
                        class_export(&relative, &source, class)
                            .map(|value| (id.name.to_string(), value))
                    }),
                _ => None,
            },
            _ => None,
        };
        if let Some((name, value)) = declaration {
            declarations.insert(name.clone(), value.clone());
            if matches!(statement, Statement::ExportDeclaration(_)) {
                exports.insert(name, value);
            }
        }
    }
    let mut cjs = BTreeMap::new();
    let mut cjs_uses = BTreeMap::<String, usize>::new();
    let mut cjs_invalid = false;
    for statement in &parsed.program.body {
        match statement {
            Statement::ExportNamedDeclaration(export) if !export.export_kind.is_type() => {
                for specifier in &export.specifiers {
                    if specifier.export_kind.is_type() {
                        continue;
                    }
                    if let Some(value) = declarations.get(specifier.local.name().as_str()) {
                        exports.insert(specifier.exported.name().to_string(), value.clone());
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                let value = match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function)
                        if function
                            .id
                            .as_ref()
                            .is_none_or(|id| !scoping.symbol_is_mutated(id.symbol_id())) =>
                    {
                        function_export(&relative, &source, function)
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class)
                        if class
                            .id
                            .as_ref()
                            .is_none_or(|id| !scoping.symbol_is_mutated(id.symbol_id())) =>
                    {
                        class_export(&relative, &source, class)
                    }
                    ExportDefaultDeclarationKind::Identifier(id) => {
                        declarations.get(id.name.as_str()).cloned()
                    }
                    _ => None,
                };
                if let Some(value) = value {
                    exports.insert("default".into(), value);
                }
            }
            Statement::ExportFromDeclaration(export) if !export.export_kind.is_type() => {
                let request = export.source.value.as_str();
                if !request.starts_with("./") && !request.starts_with("../") {
                    continue;
                }
                if request.contains(['\\', ':']) {
                    continue;
                }
                let target = file.parent().unwrap_or(root).join(request);
                if !target.is_file() {
                    continue;
                }
                let forwarded = module_exports(root, &target, inputs, watch, cache, active)?;
                for specifier in &export.specifiers {
                    if specifier.export_kind.is_type() {
                        continue;
                    }
                    if let Some(value) = forwarded.get(specifier.local.name().as_str()) {
                        exports.insert(specifier.exported.name().to_string(), value.clone());
                    }
                }
            }
            Statement::ExpressionStatement(statement) => {
                let Expression::AssignmentExpression(assignment) = &statement.expression else {
                    continue;
                };
                if assignment.operator.as_str() != "=" {
                    continue;
                }
                let AssignmentTarget::StaticMemberExpression(member) = &assignment.left else {
                    continue;
                };
                // Named: exports.x / module.exports.x. Whole-module: module.exports = id → "default".
                let (base, export_name) = match &member.object {
                    Expression::Identifier(id) if id.name == "exports" => {
                        ("exports", member.property.name.to_string())
                    }
                    Expression::Identifier(id)
                        if id.name == "module" && member.property.name == "exports" =>
                    {
                        ("module", "default".into())
                    }
                    Expression::StaticMemberExpression(parent)
                        if parent.property.name == "exports"
                            && matches!(&parent.object, Expression::Identifier(id) if id.name == "module") =>
                    {
                        ("module", member.property.name.to_string())
                    }
                    _ => continue,
                };
                let Expression::Identifier(id) = &assignment.right else {
                    cjs_invalid = true;
                    continue;
                };
                let Some(value) = declarations.get(id.name.as_str()) else {
                    cjs_invalid = true;
                    continue;
                };
                *cjs_uses.entry(base.into()).or_default() += 1;
                if cjs.insert(export_name, value.clone()).is_some() {
                    cjs_invalid = true;
                }
            }
            _ => {}
        }
    }
    for base in ["exports", "module"] {
        let actual = scoping
            .root_unresolved_references()
            .get(base)
            .map_or(0, |uses| uses.len());
        if actual != cjs_uses.get(base).copied().unwrap_or(0) {
            cjs_invalid = true;
        }
        // A shadowed CommonJS global cannot be an export object.
        if actual == 0 && cjs_uses.contains_key(base) {
            cjs_invalid = true;
        }
    }
    if module_kind == Some(false) {
        exports.clear();
    }
    if !cjs_invalid && module_kind != Some(true) {
        exports.extend(cjs);
    }
    active.remove(&file);
    cache.insert(file, exports.clone());
    Ok(exports)
}

#[derive(Default)]
struct Package {
    name: String,
    exports: BTreeMap<String, Value>,
}

fn read(
    path: &Path,
    inputs: &mut BTreeMap<PathBuf, String>,
    watch: &BudgetWatch,
) -> Result<String, NativeProducerError> {
    watch.check_wall()?;
    watch.check_files(inputs.len() + usize::from(!inputs.contains_key(path)))?;
    let bytes = std::fs::read(path)?;
    let digest = entmoot_contracts::blake3_hex(&bytes);
    if inputs.get(path).is_some_and(|previous| previous != &digest) {
        return Err(NativeProducerError::Drift(format!(
            "package input changed: {}",
            path.display()
        )));
    }
    inputs.insert(path.to_owned(), digest);
    String::from_utf8(bytes).map_err(|error| NativeProducerError::Message(error.to_string()))
}

fn package(
    root: &Path,
    inputs: &mut BTreeMap<PathBuf, String>,
    watch: &BudgetWatch,
) -> Result<Option<Package>, NativeProducerError> {
    if !root.join("package.json").is_file() {
        return Ok(None);
    }
    let metadata: Value = serde_json::from_str(&read(&root.join("package.json"), inputs, watch)?)?;
    let Some(name) = metadata["name"].as_str() else {
        return Ok(None);
    };
    if entmoot_contracts::purl_npm(name, None).is_err() {
        return Ok(None);
    }
    let invariant = if metadata["exports"]
        .as_object()
        .is_some_and(|object| object.keys().any(|key| key.starts_with('.')))
    {
        None
    } else {
        invariant_route(&metadata["exports"])
    };
    let mut routes: Vec<(String, String)> = if let Some(path) = invariant {
        vec![(name.into(), path.into())]
    } else if let Some(routes) = metadata["exports"].as_object() {
        routes
            .iter()
            .filter_map(|(key, value)| {
                let path = invariant_route(value)?;
                if key == "." {
                    Some((name.into(), path.into()))
                } else {
                    key.strip_prefix("./")
                        .filter(|key| !key.contains('*'))
                        .map(|key| (format!("{name}/{key}"), path.into()))
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    // Classic packages without an `exports` map: string main/module, then Node index.js.
    // Do not fall back when `exports` is present but condition-invariant routes are empty.
    let exports_absent = metadata.get("exports").is_none_or(|value| value.is_null());
    if routes.is_empty() && exports_absent {
        for field in ["main", "module"] {
            let Some(path) = metadata[field].as_str().filter(|path| !path.is_empty()) else {
                continue;
            };
            let path = if path.starts_with("./") {
                path.to_owned()
            } else {
                format!("./{path}")
            };
            routes.push((name.into(), path));
            break;
        }
    }
    if routes.is_empty() && exports_absent && root.join("index.js").is_file() {
        routes.push((name.into(), "./index.js".into()));
    }
    let mut result = Package {
        name: name.into(),
        ..Default::default()
    };
    let mut cache = BTreeMap::new();
    let mut active = BTreeSet::new();
    for (route, relative) in routes {
        watch.check_wall()?;
        let Some(relative) = relative.strip_prefix("./") else {
            continue;
        };
        if relative
            .split('/')
            .any(|part| part.is_empty() || part == ".." || part == ".")
            || relative.contains(['\\', ':', '*'])
        {
            continue;
        }
        let file = root.join(relative);
        if !file.is_file() {
            continue;
        }
        let canonical = file.canonicalize()?;
        if !canonical.starts_with(root) {
            continue;
        }
        for (name, mut value) in
            module_exports(root, &canonical, inputs, watch, &mut cache, &mut active)?
        {
            let symbol = format!("{route}#{name}");
            value["symbol"] = symbol.clone().into();
            result.exports.insert(symbol, value);
        }
    }
    Ok(Some(result))
}

pub fn collect(
    root: &Path,
    raw: &mut NativeRawObservation,
    watch: &BudgetWatch,
) -> Result<(), NativeProducerError> {
    let mut inputs = BTreeMap::new();
    let mut surfaces = Vec::new();
    if let Some(provider) = package(root, &mut inputs, watch)? {
        surfaces.push(json!({"package": {"ecosystem": "javascript", "name": provider.name},
            "export_universes": [{"symbol_scheme": "javascript.compiler.symbol", "boundary_policy": POLICY},
                {"symbol_scheme": "javascript.compiler.symbol", "boundary_policy": CLASS_POLICY}],
            "completeness": "partial", "exports": provider.exports.into_values().collect::<Vec<_>>(),
            "limitations": ["static_export_routes_only", "condition_dependent_routes_dynamic_exports_and_runtime_dispatch_unassessed"]}));
    }
    for (path, digest) in &inputs {
        watch.check_wall()?;
        if entmoot_contracts::blake3_hex(&std::fs::read(path)?) != *digest {
            return Err(NativeProducerError::Drift(format!(
                "package input changed: {}",
                path.display()
            )));
        }
        raw.inputs.push(path.to_string_lossy().into_owned());
    }
    watch.check_files(
        raw.inputs
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
    )?;
    raw.build_context.insert(
        "package_input_digests".into(),
        serde_json::to_value(inputs)?,
    );
    // Seeking / package-root enrich refs are filled by package_api_seek in the pipeline.
    raw.build_context.insert("package_api".into(), json!({"surfaces": surfaces, "references": [],
        "references_complete": false, "limitations": ["static_literal_export_routes_only", "external_and_negative_lookup_closure_unassessed", "no_typescript_typechecking_or_runtime_resolution_claim"]}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exports(metadata: &str, files: &[(&str, &str)]) -> Package {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), metadata).unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        package(
            &dir.path().canonicalize().unwrap(),
            &mut BTreeMap::new(),
            &BudgetWatch::start(Default::default()),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn aliases_reexports_and_class_headers_preserve_public_routes_without_bodies() {
        let metadata = r#"{"name":"fixture","type":"module","exports":{".":"./index.js","./alias":"./index.js"}}"#;
        let package = exports(metadata, &[
            ("index.js", "export {leaf as publicLeaf} from './impl.js'; export class Client { get(x) { return x; } }"),
            ("impl.js", "function leaf(x) { return x; } export {leaf};"),
        ]);
        assert_eq!(package.exports.len(), 4);
        let leaf = &package.exports["fixture#publicLeaf"];
        assert_eq!(leaf["path"], "impl.js");
        assert_eq!(leaf["boundary_text"], "function leaf(x) ");
        assert_eq!(
            package.exports["fixture#Client"]["boundary_policy"],
            CLASS_POLICY
        );
        assert!(!package.exports["fixture#Client"]["boundary_text"]
            .as_str()
            .unwrap()
            .contains("return"));
    }

    #[test]
    fn conditional_exports_need_one_destination_and_a_default() {
        for (routes, expected) in [
            (r#"{"import":"./index.js","default":"./index.js"}"#, 1),
            (
                r#"{"import":"./index.js","require":"./other.js","default":"./index.js"}"#,
                0,
            ),
            (r#"{"import":"./index.js"}"#, 0),
        ] {
            let metadata = format!(r#"{{"name":"fixture","type":"module","exports":{routes}}}"#);
            let package = exports(
                &metadata,
                &[
                    ("index.js", "export function leaf() {}"),
                    ("other.js", "export function leaf(x) {}"),
                ],
            );
            assert_eq!(package.exports.len(), expected, "{routes}");
        }
    }

    #[test]
    fn commonjs_static_assignments_reject_shadowing_mutation_and_escaping_exports() {
        let metadata = r#"{"name":"fixture","exports":"./index.cjs"}"#;
        for (source, expected) in [
            ("function leaf(x) { return x; } exports.leaf = leaf;", 1),
            (
                "function leaf(x) { return x; } module.exports.leaf = leaf;",
                1,
            ),
            ("function leaf(x) { return x; } module.exports = leaf;", 1),
            (
                "const exports = {}; function leaf(x) { return x; } exports.leaf = leaf;",
                0,
            ),
            (
                "function leaf(x) { return x; } exports.leaf = leaf; leaf = other;",
                0,
            ),
            (
                "function leaf(x) { return x; } exports.leaf = leaf; mutate(exports);",
                0,
            ),
            (
                "function leaf(x) { return x; } exports.leaf = leaf; exports.leaf = leaf;",
                0,
            ),
        ] {
            assert_eq!(
                exports(metadata, &[("index.cjs", source)]).exports.len(),
                expected,
                "{source}"
            );
        }
        let whole = exports(
            r#"{"name":"fixture","exports":"./index.cjs"}"#,
            &[(
                "index.cjs",
                "function Accepts(req) { return req; } module.exports = Accepts;",
            )],
        );
        assert!(whole.exports.contains_key("fixture#default"));
        assert_eq!(
            whole.exports["fixture#default"]["boundary_text"],
            "function Accepts(req) "
        );
    }

    #[test]
    fn main_and_index_js_fallback_without_exports_map() {
        let with_main = exports(
            r#"{"name":"fixture","main":"lib.js"}"#,
            &[(
                "lib.js",
                "function leaf(x) { return x; } module.exports = leaf;",
            )],
        );
        assert_eq!(with_main.exports.len(), 1);
        assert!(with_main.exports.contains_key("fixture#default"));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"accepts-like"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("index.js"),
            "function Accepts(req) { return req; } module.exports = Accepts;",
        )
        .unwrap();
        let package = package(
            &dir.path().canonicalize().unwrap(),
            &mut BTreeMap::new(),
            &BudgetWatch::start(Default::default()),
        )
        .unwrap()
        .unwrap();
        assert!(package.exports.contains_key("accepts-like#default"));
    }

    #[test]
    fn cycles_and_type_only_reexports_do_not_invent_export_targets() {
        let metadata = r#"{"name":"fixture","exports":"./index.ts"}"#;
        let package = exports(
            metadata,
            &[
                (
                    "index.ts",
                    "export {leaf} from './other.ts'; export type {Thing} from './other.ts';",
                ),
                (
                    "other.ts",
                    "export {leaf} from './index.ts'; export class Thing {}",
                ),
            ],
        );
        assert!(package.exports.is_empty());
    }

    #[test]
    fn explicit_module_kinds_and_mutable_defaults_cannot_publish_false_routes() {
        for (kind, source) in [
            ("commonjs", "export function leaf() {}"),
            ("module", "function leaf() {} exports.leaf = leaf;"),
            ("module", "export default function leaf() {} leaf = other;"),
            ("module", "export default class Client {} Client = other;"),
        ] {
            let metadata =
                format!(r#"{{"name":"fixture","type":"{kind}","exports":"./index.js"}}"#);
            assert!(
                exports(&metadata, &[("index.js", source)])
                    .exports
                    .is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn dependency_metadata_and_export_sources_consume_the_file_budget() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("package.json"),
            r#"{"name":"dep","type":"module","exports":"./index.js"}"#,
        )
        .unwrap();
        std::fs::write(root.path().join("index.js"), "export function leaf() {}").unwrap();
        let watch = BudgetWatch::start(crate::native_resolver::budget::ProducerBudget {
            max_files: 1,
            ..Default::default()
        });
        let result = collect(
            &root.path().canonicalize().unwrap(),
            &mut NativeRawObservation::default(),
            &watch,
        );
        assert!(matches!(result, Err(NativeProducerError::Budget(_))));
    }
}
