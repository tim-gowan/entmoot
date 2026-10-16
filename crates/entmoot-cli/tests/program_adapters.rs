//! Opt-in native Program and lock-provider controls. Assertions live in Cargo;
//! fixture sources and provider facts remain independently authored here.
//! Product Open/Closed call-site claims are owned by engine
//! `dispatch_expectations`; this suite keeps Program API, lock, and provider
//! `gap` controls (normalized at the Entmoot boundary into `DispatchGap`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};
#[path = "support/canon.rs"]
mod canon;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn adapter(name: &str, relative: &str) -> PathBuf {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tmp/adapter-build")
                .join(relative)
        });
    assert!(
        path.is_file(),
        "requested native lane requires {}",
        path.display()
    );
    let canonical = path.canonicalize().unwrap();
    let spelling = canonical.to_string_lossy();
    PathBuf::from(spelling.strip_prefix(r"\\?\").unwrap_or(&spelling))
}

fn node(path: &Path, input: &Value, env: &[(&str, String)]) -> Value {
    parsed(node_output(path, input, env, &[]))
}

fn node_output(path: &Path, input: &Value, env: &[(&str, String)], remove: &[&str]) -> Output {
    let (mut command, _) =
        entmoot_engine::native_resolver::producers::sidecar::command_for_binary(path).unwrap();
    for key in remove {
        command.env_remove(key);
    }
    let mut child = command
        .envs(env.iter().map(|(key, value)| (*key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn parsed(result: Output) -> Value {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
#[ignore = "requires the pinned Acorn package in the TypeScript historical tools"]
fn javascript_parser_modes_reject_optional_chaining_at_the_es2015_floor() {
    let parser = adapter(
        "ENTMOOT_ACORN_LIBRARY",
        "typescript-versions/node_modules/acorn/dist/acorn.js",
    );
    let check = |version: u32, source: &str| {
        let result = Command::new("node")
            .args(["-e", "const acorn=require(process.argv[1]); acorn.parse(process.argv[3], {sourceType:'module',ecmaVersion:Number(process.argv[2])});"])
            .arg(&parser).arg(version.to_string()).arg(source).output().unwrap();
        result.status.success()
    };
    for version in [2015, 2020, 2022, 2024] {
        let mut source =
            String::from("export function leaf(x) { return x; }\nexport {leaf as publicLeaf};\n");
        if version >= 2020 {
            source.push_str("export function maybe(x) { return x?.value ?? 0; }\n");
        }
        if version >= 2022 {
            source.push_str("export class Box { #value = 1; get() { return this.#value; } }\n");
        }
        assert!(check(version, &source), "ES{version}");
    }
    assert!(!check(2015, "export function leaf(x) { return x?.value; }"));
}

fn write(root: &Path, name: &str, source: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    path
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().expect("provider array")
}

#[test]
fn canon_programs_export_native_scalar_sites_and_qualify_shared_lowering() {
    let python = Python::new();
    let typescript = TypeScript::new();
    std::fs::write(
        &python.environment,
        serde_json::to_vec(&python.settings).unwrap(),
    )
    .unwrap();
    for (language, root, filename, source, key, default, env) in [
        (
            "python",
            &python.root,
            "a.py",
            "def f(x: bool) -> bool:\n    return not x\n",
            "ENTMOOT_PYRIGHT_ADAPTER",
            "pyright-program/entmoot-pyright.cjs",
            vec![(
                "ENTMOOT_PYTHON_ENVIRONMENT",
                python.environment.to_string_lossy().into_owned(),
            )],
        ),
        (
            "typescript",
            &typescript.root,
            "a.ts",
            "export function f(x:boolean):boolean { return !x; }",
            "ENTMOOT_TYPESCRIPT_ADAPTER",
            "typescript-program/entmoot-typescript.cjs",
            vec![],
        ),
    ] {
        write(root, filename, source);
        let result = node(
            &adapter(key, default),
            &json!({"root":root,"canon":true}),
            &env,
        );
        canon::lower(source, language, filename, &result);
    }
}
fn api(value: &Value) -> &Value {
    assert_eq!(
        value["invalid_environment"], false,
        "diagnostics: {}",
        value["diagnostics"]
    );
    &value["build_context"]["package_api"]
}
fn surface(value: &Value) -> &Value {
    &api(value)["surfaces"][0]
}
fn symbols(value: &Value) -> BTreeSet<&str> {
    rows(&value["exports"])
        .iter()
        .map(|row| row["symbol"].as_str().unwrap())
        .collect()
}
fn export<'a>(value: &'a Value, symbol: &str) -> &'a Value {
    rows(&value["exports"])
        .iter()
        .find(|row| row["symbol"] == symbol)
        .unwrap()
}

struct Python {
    _temp: tempfile::TempDir,
    root: PathBuf,
    environment: PathBuf,
    settings: Value,
}
impl Python {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        std::fs::create_dir(&root).unwrap();
        let executable = Command::new("python")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        assert!(executable.status.success());
        let interpreter = String::from_utf8(executable.stdout)
            .unwrap()
            .trim()
            .to_string();
        let environment = temp.path().join("environment.json");
        let settings = json!({"interpreter":interpreter,"import_roots":[],"packages":[{"name":"Python_Fixture","module":"","root":root}]});
        Self {
            _temp: temp,
            root,
            environment,
            settings,
        }
    }
    fn run(&self) -> Value {
        std::fs::write(
            &self.environment,
            serde_json::to_vec(&self.settings).unwrap(),
        )
        .unwrap();
        node(
            &adapter(
                "ENTMOOT_PYRIGHT_ADAPTER",
                "pyright-program/entmoot-pyright.cjs",
            ),
            &json!({"root":self.root}),
            &[(
                "ENTMOOT_PYTHON_ENVIRONMENT",
                self.environment.to_string_lossy().into_owned(),
            )],
        )
    }
}

#[test]
fn pyright_explicit_ignored_checkout_retains_static_and_dynamic_sites() {
    let mut fixture = Python::new();
    let parent = fixture.root.clone();
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .arg(&parent)
        .status()
        .unwrap()
        .success());
    write(&parent, ".gitignore", ".tmp/\n");
    write(
        &parent,
        "excluded.py",
        "# tracked only in the enclosing repository\n",
    );
    assert!(Command::new("git")
        .arg("-C")
        .arg(&parent)
        .args(["add", "excluded.py"])
        .status()
        .unwrap()
        .success());
    fixture.root = parent.join(".tmp/checkout");
    fixture.settings["packages"] = json!([]);
    fixture.settings["import_roots"] = json!([fixture.root]);
    let source = "def leaf(x):\n    return x\ndef caller():\n    return leaf(1)\ndef dynamic(fn):\n    return fn(2)\n";
    write(&fixture.root, "api.py", source);
    write(&fixture.root, ".gitignore", "excluded.py\n");
    write(&fixture.root, "excluded.py", "this is not valid Python !!!");
    write(
        &fixture.root,
        ".venv/noise.py",
        "this is not valid Python !!!",
    );
    let value = fixture.run();
    assert_eq!(
        value["invalid_environment"], false,
        "{}",
        value["diagnostics"]
    );
    let calls = rows(&value["calls"]);
    assert_eq!(
        calls.len(),
        2,
        "explicit source root must survive an enclosing scratch ignore"
    );
    assert_eq!(calls[0]["targets"][0]["symbol"], "api.leaf");
    assert_eq!(calls[1]["gap"], "indirect_call");
    assert_eq!(value["build_context"]["partial"], true);
}

#[test]
fn pyright_unicode_offsets_header_identity_and_version_modes() {
    let mut fixture = Python::new();
    let source = "# Unicode: é🍃\r\ndef leaf(x: int) -> int:\r\n    return x\r\n\r\ndef caller():\r\n    return leaf(1)\r\n";
    write(&fixture.root, "api.py", source);
    let first = fixture.run();
    assert_eq!(first, fixture.run());
    assert_eq!(first["diagnostics"], json!([]));
    let call = &first["calls"][0];
    assert_eq!(
        &source.as_bytes()[usize::try_from(call["start_byte"].as_u64().unwrap()).unwrap()
            ..usize::try_from(call["end_byte"].as_u64().unwrap()).unwrap()],
        b"leaf(1)"
    );
    assert_eq!(call["targets"][0]["symbol"], "api.leaf");
    assert_eq!(
        surface(&first)["completeness"],
        "complete_relative_to_universe"
    );
    write(
        &fixture.root,
        "api.py",
        &source.replace("return x", "return 3"),
    );
    let second = fixture.run();
    assert_eq!(
        export(surface(&first), "api.leaf")["boundary_digest"],
        export(surface(&second), "api.leaf")["boundary_digest"]
    );
    assert_ne!(
        first["build_context"]["input_digests"],
        second["build_context"]["input_digests"]
    );
    for (version, source) in [
        ("3.8", "def leaf(x, /):\n    return x\n"),
        (
            "3.10",
            "def leaf(x):\n    match x:\n        case 1: return 1\n        case _: return 0\n",
        ),
        ("3.12", "def leaf[T](x: T) -> T:\n    return x\n"),
        ("3.14", "def leaf[T](x: T) -> T:\n    return x\n"),
    ] {
        fixture.settings["python_version"] = json!(version);
        write(&fixture.root, "api.py", source);
        let value = fixture.run();
        assert_eq!(value["build_context"]["syntax_python_version"], version);
        assert!(!rows(&surface(&value)["exports"]).is_empty());
        assert!(!rows(&value["diagnostics"])
            .iter()
            .any(|row| row.as_str().unwrap().starts_with("parse_error:")));
    }
    fixture.settings["python_version"] = json!("3.8");
    let rejected = fixture.run();
    assert_eq!(
        rejected["build_context"]["package_api"]["surfaces"][0]["completeness"],
        "partial"
    );
    assert!(rows(&rejected["diagnostics"])
        .iter()
        .any(|row| row.as_str().unwrap().starts_with("parse_error:")));
    fixture.settings["python_version"] = json!("2.7");
    assert_eq!(fixture.run()["invalid_environment"], true);
    fixture
        .settings
        .as_object_mut()
        .unwrap()
        .remove("python_version");
    fixture.settings["interpreter"] = json!(fixture.root.join("missing-interpreter"));
    assert_eq!(fixture.run()["invalid_environment"], true);
}

#[test]
fn pyright_discovers_project_environment_and_rejects_bad_overrides_and_encoding() {
    let fixture = Python::new();
    let venv = fixture.root.join(".venv");
    assert!(
        Command::new(fixture.settings["interpreter"].as_str().unwrap())
            .args(["-m", "venv", "--without-pip"])
            .arg(&venv)
            .status()
            .unwrap()
            .success()
    );
    let python = venv.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    let site = Command::new(python)
        .args(["-I", "-c", "import site; print(site.getsitepackages()[0])"])
        .output()
        .unwrap();
    assert!(site.status.success());
    let site = PathBuf::from(String::from_utf8(site.stdout).unwrap().trim());
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(
        site.join("unused_legacy_encoding.py"),
        b"# coding: latin-1\n# \xe9\ndef unused(): return absent()\n",
    )
    .unwrap();
    write(
        &fixture.root,
        "api.py",
        "def leaf(x):\n    return x\ndef caller():\n    return leaf(1)\n",
    );
    let adapter = adapter(
        "ENTMOOT_PYRIGHT_ADAPTER",
        "pyright-program/entmoot-pyright.cjs",
    );
    let removed = [
        "ENTMOOT_PYTHON_ENVIRONMENT",
        "ENTMOOT_PYTHON_EXECUTABLE",
        "VIRTUAL_ENV",
        "CONDA_PREFIX",
    ];
    let request = json!({"root":fixture.root});
    let result = parsed(node_output(&adapter, &request, &[], &removed));
    assert_eq!(
        result["tool_environment"][0]["selection"]["kind"],
        "project"
    );
    assert_eq!(result["tool_environment"][0]["selection"]["path"], ".venv");
    assert!(!rows(&result["calls"]).is_empty());
    assert!(rows(&result["calls"])
        .iter()
        .all(|call| call["path"] == "api.py"));
    let failed = node_output(
        &adapter,
        &request,
        &[(
            "ENTMOOT_PYTHON_EXECUTABLE",
            fixture
                .root
                .join("missing.exe")
                .to_string_lossy()
                .into_owned(),
        )],
        &removed,
    );
    assert_eq!(parsed(failed)["invalid_environment"], true);
    std::fs::write(
        fixture.root.join("api.py"),
        b"# invalid utf8: \xff\ndef f(): pass\n",
    )
    .unwrap();
    let invalid = parsed(node_output(&adapter, &request, &[], &removed));
    assert_eq!(invalid["invalid_environment"], true);
    assert!(invalid["diagnostics"].to_string().contains("api.py"));
}

#[test]
fn pyright_missing_ambiguous_dynamic_and_empty_evidence_stays_open() {
    let mut fixture = Python::new();
    for source in [
        "from absent_package import leaf\ndef caller():\n    return leaf(1)\ndef __getattr__(name):\n    return caller\n",
        "if unknown:\n    def leaf(x): return x\nelse:\n    def leaf(x): return 0\ndef caller():\n    return leaf(1)\n",
        "NAMES = ['leaf']\n__all__ = list(NAMES)\ndef leaf(): return 1\n",
        "def leaf(): return 1\ndef __getattr__(name): return leaf\n",
    ] {
        write(&fixture.root, "api.py", source);
        let value = fixture.run();
        assert_eq!(surface(&value)["completeness"], "partial", "{value}");
        if source.contains("caller") {
            assert_eq!(api(&value)["references_complete"], false);
            assert_ne!(rows(&value["calls"][0]["targets"]).len(), 1);
        }
    }
    write(
        &fixture.root,
        "api.py",
        "def leaf(x):\n    import absent_dependency\n    return absent_dependency.run(x)\n",
    );
    let body_gap = fixture.run();
    assert_eq!(
        surface(&body_gap)["completeness"],
        "complete_relative_to_universe"
    );
    assert_eq!(api(&body_gap)["references_complete"], false);
    write(
        &fixture.root,
        "api.py",
        "import absent_dependency\ndef leaf(x): return x\n",
    );
    assert_eq!(surface(&fixture.run())["completeness"], "partial");
    fixture.settings["packages"][0]["module"] = json!("invented");
    write(&fixture.root, "api.py", "# empty\n");
    assert!(rows(&surface(&fixture.run())["limitations"])
        .contains(&json!("package_module_coordinate_mismatch")));
    std::fs::remove_file(fixture.root.join("api.py")).unwrap();
    assert!(
        rows(&surface(&fixture.run())["limitations"]).contains(&json!("no_native_package_modules"))
    );
}

#[test]
fn pyright_public_aliases_star_routes_and_stub_authorities_remain_distinct() {
    let mut fixture = Python::new();
    let package = fixture.root.join("pkg");
    write(
        &package,
        "core.py",
        "__all__ = ['leaf']\ndef leaf(x: int) -> int:\n    return x\n",
    );
    write(
        &package,
        "__init__.py",
        "from .core import leaf as public\n__all__ = ['public']\n",
    );
    fixture.settings["packages"] = json!([{"name":"fixture","module":"pkg","root":package}]);
    let first = fixture.run();
    assert_eq!(
        symbols(surface(&first)),
        BTreeSet::from(["pkg.public", "pkg.core.leaf"])
    );
    assert_eq!(
        export(surface(&first), "pkg.public")["boundary_digest"],
        export(surface(&first), "pkg.core.leaf")["boundary_digest"]
    );
    write(&package, "__init__.py", "__all__ = []\n");
    assert_eq!(
        symbols(surface(&fixture.run())),
        BTreeSet::from(["pkg.core.leaf"])
    );
    write(
        &package,
        "__init__.py",
        "from .core import *\n__all__ = ['leaf']\n",
    );
    let star = fixture.run();
    assert_eq!(
        surface(&star)["completeness"],
        "complete_relative_to_universe"
    );
    assert_eq!(
        symbols(surface(&star)),
        BTreeSet::from(["pkg.leaf", "pkg.core.leaf"])
    );

    let stub = Python::new();
    write(&stub.root, "api.py", "def leaf(x): return x\n");
    write(&stub.root, "api.pyi", "def leaf(x: int) -> int: ...\n");
    write(
        &stub.root,
        "use.py",
        "from api import leaf\ndef caller(): return leaf(1)\n",
    );
    let value = stub.run();
    assert_eq!(value["calls"][0]["targets"][0]["path"], "api.py");
    let attempt = &value["build_context"]["declaration_attempts"][0]["declarations"][0];
    assert!(attempt["resolved"]["uri"]
        .as_str()
        .unwrap()
        .ends_with(".pyi"));
    assert!(attempt["navigation"][0]["uri"]
        .as_str()
        .unwrap()
        .ends_with(".py"));
    let authorities: BTreeSet<_> = rows(&surface(&value)["exports"])
        .iter()
        .filter(|row| row["symbol"] == "api.leaf")
        .map(|row| row["boundary_policy"].as_str().unwrap())
        .collect();
    assert_eq!(
        authorities,
        BTreeSet::from([
            "pyright.function_header_utf8.v1",
            "pyright.stub_function_header_utf8.v1"
        ])
    );
    std::fs::remove_file(stub.root.join("api.py")).unwrap();
    let missing = stub.run();
    assert_eq!(missing["calls"][0]["targets"][0]["kind"], "stub");
    assert_eq!(missing["calls"][0]["gap"], "unsupported_construct");
}

#[test]
fn pyright_dispatch_decorators_reexports_and_type_only_uses_remain_conservative() {
    let mut fixture = Python::new();
    let dependency = fixture._temp.path().join("dependency");
    write(&dependency, "api.py", "class Client:\n    def __init__(self, value: int): pass\n    def get(self, x: int): return x\n    @property\n    def token(self): return 1\nclass Derived(Client): pass\n");
    write(
        &dependency,
        "facade.py",
        "from api import Client as PublicClient\n__all__ = ['PublicClient']\n",
    );
    fixture.settings["import_roots"] = json!([dependency]);
    fixture.settings["packages"] = json!([{"name":"fixture-api","module":"","root":dependency}]);
    write(&fixture.root, "main.py", "from facade import PublicClient\nfrom api import Derived\ndef caller(client: PublicClient):\n    client.get(1)\n    PublicClient(1)\n    Derived(1)\n");
    let caller = fixture.run();
    let refs: BTreeSet<_> = rows(&api(&caller)["references"])
        .iter()
        .map(|row| row["target"]["symbol"]["spelling"].as_str().unwrap())
        .collect();
    assert_eq!(
        refs,
        BTreeSet::from(["api.Client.get", "facade.PublicClient"])
    );
    assert_eq!(api(&caller)["references_complete"], false);
    fixture.root = dependency;
    let provider = fixture.run();
    assert_eq!(surface(&provider)["completeness"], "partial");
    assert_eq!(
        export(surface(&provider), "api.Client.token")["boundary_policy"],
        "pyright.builtin_decorated_method_headers.v1"
    );
    assert_eq!(
        export(surface(&provider), "api.Derived.get")["boundary_policy"],
        "pyright.inherited_member_route.v1"
    );
    assert_ne!(
        export(surface(&provider), "api.Derived.get")["boundary_digest"],
        export(surface(&provider), "api.Client.get")["boundary_digest"]
    );

    let distinct = Python::new();
    write(&distinct.root, "api.py", "class Base:\n    def get(self, x): return x\n    def __enter__(self): return self\nclass A(Base): pass\nclass B(Base): pass\nclass Different:\n    def get(self, x): return 2\ndef same(client: A | B): return client.get(1)\ndef distinct(client: A | Different): return client.get(1)\n");
    let value = distinct.run();
    assert_eq!(
        rows(&value["calls"])
            .iter()
            .map(|call| rows(&call["targets"]).len())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(rows(&value["build_context"]["declaration_attempts"][0]["declarations"]).len() >= 2);
    assert!(symbols(surface(&value)).contains("api.Base.__enter__"));
    write(&distinct.root, "api.py", "def staticmethod(fn): return fn\nclass Api:\n    @staticmethod\n    def method(x): return x\n");
    assert!(!symbols(surface(&distinct.run())).contains("api.Api.method"));
    write(&distinct.root, "api.py", "class Meta(type):\n    def __call__(cls, x): return x\nclass Api(metaclass=Meta):\n    def method(self, x: int): return x\ndef caller(): return Api(1)\n");
    let meta = distinct.run();
    assert_eq!(
        export(surface(&meta), "api.Api")["boundary_policy"],
        "pyright.class_mro_declarations.v1"
    );
    assert!(rows(&meta["calls"])
        .iter()
        .any(|call| call["gap"] == "dynamic_dispatch"));
    write(
        &distinct.root,
        "api.py",
        "from pathlib import Path\ndef caller(path: Path): return path.is_file()\n",
    );
    let stub = distinct.run();
    assert_eq!(stub["calls"][0]["targets"][0]["kind"], "stub");
    assert_eq!(stub["calls"][0]["gap"], "unsupported_construct");

    let mut cross = Python::new();
    let implementation = cross._temp.path().join("impl");
    let facade = cross._temp.path().join("facade");
    write(&implementation, "impl.py", "def leaf(x): return x\n");
    write(
        &facade,
        "facade.py",
        "from impl import leaf as public\n__all__ = ['public']\n",
    );
    cross.settings["import_roots"] = json!([implementation, facade]);
    cross.settings["packages"] = json!([{"name":"implementation","module":"","root":implementation},{"name":"facade","module":"","root":facade}]);
    write(
        &cross.root,
        "main.py",
        "from facade import public\ndef caller(): return public(1)\n",
    );
    let value = cross.run();
    assert!(!rows(&value["calls"][0]["targets"]).is_empty());
    assert_eq!(api(&value)["references"], json!([]));
    assert_eq!(api(&value)["references_complete"], false);
    write(&cross.root, "main.py", "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from impl import leaf\ndef unused():\n    from impl import leaf as renamed\n");
    let value = cross.run();
    assert_eq!(value["calls"], json!([]));
    assert_eq!(api(&value)["references"], json!([]));
}

struct TypeScript {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl TypeScript {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        write(
            &root,
            "package.json",
            r#"{"name":"fixture","version":"1.0.0"}"#,
        );
        write(
            &root,
            "tsconfig.json",
            r#"{"compilerOptions":{"target":"ES2020","module":"NodeNext","moduleResolution":"NodeNext","strict":true,"allowJs":true,"checkJs":true,"maxNodeModuleJsDepth":2}}"#,
        );
        Self { _temp: temp, root }
    }
    fn run(&self, environment: &[(&str, String)]) -> Value {
        node(
            &adapter(
                "ENTMOOT_TYPESCRIPT_ADAPTER",
                "typescript-program/entmoot-typescript.cjs",
            ),
            &json!({"root":self.root}),
            environment,
        )
    }
}

#[test]
fn typescript_callback_signature_retains_an_indirect_gap_and_static_sibling() {
    let fixture = TypeScript::new();
    write(&fixture.root, "api.ts", "export function leaf(x:number):number { return x+1; }\nexport function caller():number { return leaf(1); }\nexport function indirect(fn:(x:number)=>number):number { return fn(2); }\n");
    let value = fixture.run(&[]);
    assert_eq!(value["invalid_environment"], false);
    let calls = rows(&value["calls"]);
    assert_eq!(calls.len(), 2);
    assert!(calls[0]["targets"][0]["symbol"]
        .as_str()
        .unwrap()
        .ends_with("#leaf"));
    assert!(calls[0].get("gap").is_none());
    assert_eq!(calls[1]["gap"], "indirect_call");
    assert_eq!(calls[1]["targets"][0]["kind"], "type");
}

#[test]
fn typescript_routes_offsets_types_and_body_independent_contracts() {
    let fixture = TypeScript::new();
    let source = "\u{feff}// Unicode 🍃\r\nexport type Count = number;\r\nexport interface Box { value: number }\r\nexport function leaf(x: number): number { return x; }\r\n";
    write(&fixture.root, "leaf.ts", source);
    write(
        &fixture.root,
        "api.ts",
        "export {leaf as renamed} from './leaf.js'; export * from './leaf.js';\n",
    );
    let usage = "import {renamed} from './api.js'; import * as ns from './api.js'; export function run() { return renamed(1) + ns.leaf(2); }\n";
    write(&fixture.root, "use.ts", usage);
    let first = fixture.run(&[]);
    let exports = symbols(surface(&first));
    for suffix in ["#Count", "#Box", "#renamed"] {
        assert!(exports.iter().any(|symbol| symbol.ends_with(suffix)));
    }
    assert_eq!(rows(&first["calls"]).len(), 2);
    for call in rows(&first["calls"]) {
        assert_eq!(rows(&call["targets"]).len(), 1);
        let text = &usage[usize::try_from(call["start_byte"].as_u64().unwrap()).unwrap()
            ..usize::try_from(call["end_byte"].as_u64().unwrap()).unwrap()];
        assert!(["renamed(1)", "ns.leaf(2)"].contains(&text));
    }
    assert_eq!(first, fixture.run(&[]));
    write(
        &fixture.root,
        "leaf.ts",
        &source.replace("return x", "return 3"),
    );
    let second = fixture.run(&[]);
    for before in rows(&surface(&first)["exports"]) {
        let after = export(surface(&second), before["symbol"].as_str().unwrap());
        assert_eq!(before["boundary_digest"], after["boundary_digest"]);
    }
    assert_ne!(
        first["build_context"]["input_digests"],
        second["build_context"]["input_digests"]
    );
}

#[test]
fn typescript_missing_inputs_declarations_and_stdlib_never_invent_runtime_binds() {
    let fixture = TypeScript::new();
    write(
        &fixture.root,
        "use.ts",
        "import {leaf} from './missing.js'; export function run() { return leaf(1); }\n",
    );
    let missing = fixture.run(&[]);
    assert_eq!(api(&missing)["references_complete"], false);
    assert!(rows(&missing["build_context"]["lookup_observations"])
        .iter()
        .any(|row| row["operation"] == "fileExists"
            && row["result"] == false
            && row["arguments"][0].as_str().unwrap().contains("missing")));
    write(
        &fixture.root,
        "missing.d.ts",
        "export declare function leaf(x: number): number;\n",
    );
    let declared = fixture.run(&[]);
    assert_eq!(declared["calls"][0]["targets"][0]["kind"], "type");
    assert_eq!(declared["calls"][0]["gap"], "unsupported_construct");
    assert_eq!(api(&declared)["references_complete"], false);
    std::fs::remove_file(fixture.root.join("missing.d.ts")).unwrap();
    write(
        &fixture.root,
        "missing.ts",
        "export function leaf(x: number) { return x; }\n",
    );
    let found = fixture.run(&[]);
    assert!(!rows(&found["calls"][0]["targets"]).is_empty());
    assert_ne!(
        found["build_context"]["input_digests"],
        missing["build_context"]["input_digests"]
    );
    assert_eq!(
        fixture.run(&[(
            "ENTMOOT_TYPESCRIPT_CONFIG",
            fixture
                .root
                .join("absent.json")
                .to_string_lossy()
                .into_owned()
        )])["invalid_environment"],
        true
    );
    write(
        &fixture.root,
        "use.ts",
        "export function run() { return parseInt('1'); }\n",
    );
    let stdlib = fixture.run(&[]);
    assert_eq!(stdlib["calls"][0]["targets"][0]["package_role"], "stdlib");
    assert_eq!(api(&stdlib)["references"], json!([]));
}

#[test]
fn typescript_class_commonjs_conditional_routes_and_lock_selection_keep_their_owners() {
    let fixture = TypeScript::new();
    write(&fixture.root,"api.ts","function tag(value: any) { return value; }\nexport class Base<T> { method(x: T): T { return x; } private hidden() {} }\n@tag export class Child extends Base<number> { field = 1; own(): number { return this.method(this.field); } }\n");
    let value = fixture.run(&[]);
    let exported = symbols(surface(&value));
    assert!(exported.iter().any(|name| name.ends_with("#Child.method")));
    assert!(!exported.iter().any(|name| name.ends_with(".hidden")));
    let child = rows(&surface(&value)["exports"])
        .iter()
        .find(|row| row["symbol"].as_str().unwrap().ends_with("#Child"))
        .unwrap();
    for text in ["@tag", "extends Base<number>", "field = 1"] {
        assert!(child["boundary_text"].as_str().unwrap().contains(text));
    }
    assert!(rows(&api(&value)["limitations"])
        .contains(&json!("runtime_dispatch_and_decorator_effects_open")));
    std::fs::remove_file(fixture.root.join("api.ts")).unwrap();
    write(
        &fixture.root,
        "api.cjs",
        "function leaf(x) { return x; }\nmodule.exports = {leaf};\n",
    );
    write(&fixture.root,"use.cjs","const {leaf} = require('./api.cjs'); module.exports.run = function() { return leaf(1); };\n");
    let common = fixture.run(&[]);
    assert!(symbols(surface(&common))
        .iter()
        .any(|name| name.ends_with("#leaf")));
    assert!(rows(&common["calls"])
        .iter()
        .any(|call| call["targets"][0]["path"] == "api.cjs"));

    let slots = TypeScript::new();
    let mut packages = serde_json::Map::new();
    for (alias, version) in [("older", "1.0.0"), ("newer", "2.0.0")] {
        write(
            &slots.root,
            &format!("node_modules/{alias}/package.json"),
            &json!({"name":"dep","version":version,"type":"module","exports":"./api.js"})
                .to_string(),
        );
        write(
            &slots.root,
            &format!("node_modules/{alias}/api.js"),
            "export function leaf(x) { return x; }\n",
        );
        packages.insert(format!("node_modules/{alias}"), json!({"name":"dep","version":version,"resolved":"https://registry.npmjs.org/dep.tgz","integrity":"sha512-fixture"}));
    }
    write(
        &slots.root,
        "package-lock.json",
        &json!({"lockfileVersion":3,"packages":packages}).to_string(),
    );
    write(&slots.root,"use.mts","import {leaf as old} from 'older'; import {leaf as newer} from 'newer'; export function run() { return old(1) + newer(2); }\n");
    let value = slots.run(&[]);
    let refs = &api(&value)["references"];
    assert_eq!(rows(refs).len(), 2);
    assert!(rows(refs)
        .iter()
        .all(|row| row.get("lock_instance").is_none()));
    write(&slots.root, "package-lock.json", "invalid lock");
    assert_eq!(&api(&slots.run(&[]))["references"], refs);
    std::fs::remove_file(slots.root.join("package-lock.json")).unwrap();
    assert_eq!(&api(&slots.run(&[]))["references"], refs);

    let conditional = TypeScript::new();
    write(&conditional.root,"node_modules/dep/package.json",&json!({"name":"dep","version":"1.0.0","exports":{".":{"import":"./esm.mjs","require":"./cjs.cjs"}}}).to_string());
    write(
        &conditional.root,
        "node_modules/dep/esm.mjs",
        "export function leaf(x) { return x; }\n",
    );
    write(
        &conditional.root,
        "node_modules/dep/cjs.cjs",
        "exports.leaf = function leaf(x) { return x; };\n",
    );
    write(
        &conditional.root,
        "use.mts",
        "import {leaf} from 'dep'; export function run() { return leaf(1); }\n",
    );
    write(
        &conditional.root,
        "use.cts",
        "import dep = require('dep'); export function run() { return dep.leaf(1); }\n",
    );
    let value = conditional.run(&[]);
    api(&value);
    let routes = rows(&value["build_context"]["public_routes"]);
    assert!(routes
        .iter()
        .any(|row| row["entry_file"].as_str().unwrap().ends_with("esm.mjs")));
    assert!(routes
        .iter()
        .any(|row| row["entry_file"].as_str().unwrap().ends_with("cjs.cjs")));
}

#[test]
fn typescript_input_drift_during_a_native_batch_invalidates_the_result() {
    let fixture = TypeScript::new();
    let source = write(
        &fixture.root,
        "api.ts",
        "export function leaf(x: number) { return x; }",
    );
    let controls = tempfile::tempdir().unwrap();
    let preload = write(controls.path(),"drift.cjs",&format!("const fs = require('node:fs'); const path = require('node:path');\nconst read = fs.readFileSync; let changed = false;\nfs.readFileSync = function(file, ...rest) {{ const result = read.call(this, file, ...rest);\nif (!changed && typeof file === 'string' && path.resolve(file) === {}) {{\nchanged = true; fs.writeFileSync(file, 'export function leaf(x: string) {{ return x; }}'); }} return result; }};\n",json!(source)));
    let value = fixture.run(&[("NODE_OPTIONS", format!("--require={}", preload.display()))]);
    assert_eq!(value["invalid_environment"], true);
    assert!(value["build_context"].get("package_api").is_none());
    assert!(rows(&value["diagnostics"])
        .iter()
        .any(|row| row.as_str().unwrap().contains("changed during batch")));
}

#[test]
#[ignore = "requires the pinned historical TypeScript controls and Program adapter"]
fn typescript_historical_compilers_keep_tsx_and_reject_newer_syntax() {
    let fixture = TypeScript::new();
    let base = std::env::var_os("ENTMOOT_TYPESCRIPT_VERSION_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tmp/adapter-build/typescript-versions")
        });
    let run = |alias: &str| {
        let compiler = base
            .join("node_modules")
            .join(alias)
            .join("lib/typescript.js");
        assert!(
            compiler.is_file(),
            "missing requested historical compiler {}",
            compiler.display()
        );
        fixture.run(&[(
            "ENTMOOT_TYPESCRIPT_LIBRARY",
            compiler.to_string_lossy().into_owned(),
        )])
    };
    write(
        &fixture.root,
        "tsconfig.json",
        r#"{"compilerOptions":{"target":"ES2020","module":"NodeNext","moduleResolution":"NodeNext","strict":false,"jsx":"preserve"}}"#,
    );
    for (alias, version) in [
        ("ts49", "4.9.5"),
        ("ts50", "5.0.4"),
        ("ts55", "5.5.4"),
        ("ts59", "5.9.3"),
    ] {
        write(
            &fixture.root,
            "api.ts",
            "export function leaf<T extends number>(x: T): T { return x; }\n",
        );
        write(&fixture.root,"view.tsx","export function View() { return <section>{leaf(1)}</section>; }\nfunction leaf(x: number) { return x; }\n");
        let value = run(alias);
        assert_eq!(value["compiler_version"], format!("typescript/{version}"));
        assert!(!rows(&surface(&value)["exports"]).is_empty());
        assert!(rows(&value["calls"])
            .iter()
            .any(|call| call["path"] == "view.tsx" && !rows(&call["targets"]).is_empty()));
    }
    write(
        &fixture.root,
        "api.ts",
        "export function leaf<const T>(x: T): T { return x; }\n",
    );
    assert_eq!(run("ts49")["invalid_environment"], true);
    assert_eq!(run("ts50")["invalid_environment"], false);
}

#[test]
fn npm_instances_preserve_aliases_versions_and_reject_unqualified_slots() {
    let runtime = adapter(
        "ENTMOOT_TYPESCRIPT_ADAPTER",
        "typescript-program/entmoot-typescript.cjs",
    );
    let provider = runtime.with_file_name("npm-lock-provider.cjs");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut packages = serde_json::Map::new();
    for (slot, name, version) in [
        ("older", "dep", "1.0.0"),
        ("newer", "dep", "2.0.0"),
        ("older/node_modules/nested", "dep", "2.0.0"),
        ("@scope/pkg", "@scope/pkg", "1.0.0"),
    ] {
        let location = format!("node_modules/{slot}");
        write(
            root,
            &(location.clone() + "/package.json"),
            &json!({"name":name,"version":version}).to_string(),
        );
        packages.insert(location, json!({"name":name,"version":version,"integrity":"sha512-fixture","resolved":"https://registry.npmjs.org/fixture.tgz"}));
    }
    let invoke = || {
        Command::new("node")
            .arg(&provider)
            .arg(root)
            .output()
            .unwrap()
    };
    assert_eq!(parsed(invoke())["packages"], json!([]));
    for version in [2, 3] {
        write(
            root,
            "package-lock.json",
            &json!({"lockfileVersion":version,"packages":packages}).to_string(),
        );
        let value = parsed(invoke());
        assert_eq!(value["policy"], "npm.registry_slots.v1");
        let identities: BTreeSet<_> = rows(&value["packages"])
            .iter()
            .map(|row| row["instance"]["instance_id"].as_str().unwrap())
            .collect();
        assert_eq!(identities.len(), 4);
        assert!(rows(&value["packages"])
            .iter()
            .any(|row| row["instance"]["purl"] == "pkg:npm/%40scope/pkg@1.0.0"));
        for row in rows(&value["packages"]) {
            let instance = &row["instance"];
            assert_eq!(instance["placement"], row["root"]);
            assert_eq!(
                instance["lock_digest"],
                entmoot_contracts::blake3_hex(
                    &std::fs::read(root.join("package-lock.json")).unwrap()
                )
            );
            let mut commitment = Vec::new();
            for part in [
                "entmoot.lock_instance.v1",
                instance["purl"].as_str().unwrap(),
                instance["lock_path"].as_str().unwrap(),
                instance["lock_digest"].as_str().unwrap(),
                instance["entry"].as_str().unwrap(),
                instance["placement"].as_str().unwrap(),
            ] {
                commitment.extend_from_slice(&(part.len() as u64).to_le_bytes());
                commitment.extend_from_slice(part.as_bytes());
            }
            assert_eq!(
                instance["instance_id"],
                entmoot_contracts::blake3_hex(&commitment)
                    .strip_prefix("blake3:")
                    .unwrap()
            );
        }
    }
    packages["node_modules/older"]["version"] = json!("99");
    packages["node_modules/newer"]["link"] = json!(true);
    packages["node_modules/older/node_modules/nested"]["resolved"] = json!("file:../source");
    packages["node_modules/@scope/pkg"]
        .as_object_mut()
        .unwrap()
        .remove("integrity");
    write(
        root,
        "package-lock.json",
        &json!({"lockfileVersion":3,"packages":packages}).to_string(),
    );
    assert_eq!(parsed(invoke())["packages"], json!([]));
    write(
        root,
        "package-lock.json",
        &json!({"lockfileVersion":99,"packages":packages}).to_string(),
    );
    assert!(!invoke().status.success());
    std::fs::File::create(root.join("package-lock.json"))
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    assert!(String::from_utf8_lossy(&invoke().stderr).contains("input byte budget"));
}
