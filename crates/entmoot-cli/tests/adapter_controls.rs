//! Opt-in sidecar qualification. Source, scope and negative controls are authored
//! here. Selected call-site Open/Closed product claims live in engine
//! `dispatch_expectations` / expectations JSON; this suite keeps adapter env,
//! scope, and raw-provider controls.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};
#[path = "support/canon.rs"]
mod canon;
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn write(root: &Path, name: &str, source: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    // Prefer the host's long/real path so Windows 8.3 TEMP aliases match adapters.
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn document(root: &Path, name: &str, value: Value) -> String {
    write(root, name, &value.to_string())
        .to_string_lossy()
        .into_owned()
}

fn retained(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_key(path: &Path) -> String {
    retained(path).to_string_lossy().into_owned()
}

fn run(command: &mut Command) {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn native(family: &str, mut input: Value, env: &[(&str, String)]) -> Value {
    let key = format!("ENTMOOT_{family}_ADAPTER");
    let path =
        entmoot_engine::native_resolver::resolve_sidecar_family(&family.to_ascii_lowercase())
            .unwrap_or_else(|| panic!("requested native lane requires {key} or a catalog adapter"));
    let path = path.to_string_lossy().into_owned();
    let (mut command, _) =
        entmoot_engine::native_resolver::producers::sidecar::command_for_binary(Path::new(&path))
            .unwrap();
    // The fixtures explicitly declare the environment. Ambient operator profiles
    // must not broaden scope or accidentally make a missing-classpath test pass.
    for (name, _) in std::env::vars().filter(|(name, _)| name.starts_with("ENTMOOT_")) {
        command.env_remove(name);
    }
    if let Some(root) = input.get("root").and_then(|value| value.as_str()) {
        input["root"] = json!(path_key(Path::new(root)));
    }
    let mut child = command
        .envs(env.iter().map(|(k, v)| (*k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input).unwrap())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().expect("provider array")
}

/// Closed is target-universe completeness: require policy + nonempty assumptions.
fn assert_closed_fences(row: &Value) {
    assert_eq!(row["dispatch"]["kind"], "closed", "{row}");
    let policy = row["dispatch"]["policy_id"].as_str().unwrap_or("");
    assert!(
        !policy.trim().is_empty(),
        "Closed requires policy_id: {row}"
    );
    let assumptions = row["dispatch"]["assumptions"]
        .as_array()
        .expect("Closed requires assumptions[]");
    assert!(
        !assumptions.is_empty()
            && assumptions
                .iter()
                .all(|value| value.as_str().is_some_and(|text| !text.trim().is_empty())),
        "Closed requires nonempty assumptions: {row}"
    );
}

#[test]
fn canon_adapters_export_native_scalar_sites_and_qualify_shared_lowering() {
    for (family, language, filename, source) in [
        (
            "GO",
            "go",
            "api.go",
            "package api\nfunc f(x bool) bool { return !x }\n",
        ),
        (
            "JAVA",
            "java",
            "Api.java",
            "public class Api { public boolean f(boolean x) { return !x; } }",
        ),
        (
            "ROSLYN",
            "csharp",
            "Api.cs",
            "public class Api { public bool F(bool x) { return !x; } }",
        ),
        ("CLANG", "c", "api.c", "_Bool f(_Bool x) { return x; }"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, filename, source);
        if family == "GO" {
            write(root, "go.mod", "module example/canon\ngo 1.18\n");
        }
        if family == "CLANG" {
            document(
                root,
                "compile_commands.json",
                json!([{"directory":root,"file":filename,"arguments":["clang","-std=c11","-c",filename]}]),
            );
        }
        let output = native(family, json!({"root":root,"canon":true}), &[]);
        canon::lower(source, language, filename, &output);
    }
}

#[test]
fn source_discovery_prunes_scratch_before_opening_projects() {
    for family in ["ROSLYN", "JAVA"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        if family == "ROSLYN" {
            write(
                root,
                "Api.cs",
                "public class Api { public int Run() { return System.Math.Abs(-1); } }",
            );
        } else {
            write(
                root,
                "Api.java",
                "public class Api { public int run() { return Math.abs(-1); } }",
            );
        }
        for directory in [
            ".entmoot",
            ".tmp",
            ".cache",
            "target",
            "node_modules",
            ".venv",
        ] {
            write(root, &format!("{directory}/broken.sln"), "not a solution");
            write(
                root,
                &format!("{directory}/Api.java"),
                "public class Api { invalid syntax }",
            );
            write(
                root,
                &format!("{directory}/Api.cs"),
                "public class Api { invalid syntax }",
            );
        }
        if family == "ROSLYN" {
            for directory in ["BIN", "Obj"] {
                write(root, &format!("{directory}/broken.sln"), "not a solution");
                write(root, &format!("{directory}/Api.cs"), "invalid syntax");
            }
        }
        let value = native(family, json!({"root":root}), &[]);
        assert_eq!(
            value["invalid_environment"], false,
            "{}",
            value["diagnostics"]
        );
        assert!(!rows(&value["calls"]).is_empty());
        assert!(rows(&value["calls"])
            .iter()
            .all(|c| !c["path"].as_str().unwrap().contains('/')));
    }
}

#[test]
fn go_module_discovery_prunes_local_data_but_keeps_visible_nested_modules() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "go.mod", "module example/main\ngo 1.26\n");
    write(
        root,
        "main.go",
        "package main\nfunc leaf() {}\nfunc main() { leaf() }\n",
    );
    write(root, "nested/go.mod", "module example/nested\ngo 1.26\n");
    write(
        root,
        "nested/api.go",
        "package api\nfunc leaf() {}\nfunc Run() { leaf() }\n",
    );
    for directory in [".entmoot", ".tmp", ".cache"] {
        write(
            root,
            &format!("{directory}/tool/go.mod"),
            "not a Go module\n",
        );
        write(
            root,
            &format!("{directory}/tool/main.go"),
            "invalid Go source\n",
        );
    }
    let value = native("GO", json!({"root":root}), &[]);
    assert_eq!(
        value["invalid_environment"], false,
        "{}",
        value["diagnostics"]
    );
    let paths: std::collections::BTreeSet<_> = rows(&value["calls"])
        .iter()
        .map(|call| call["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        std::collections::BTreeSet::from(["main.go", "nested/api.go"])
    );
}

fn exports(value: &Value) -> Vec<&Value> {
    assert_eq!(
        value["invalid_environment"], false,
        "{}",
        value["diagnostics"]
    );
    rows(&value["build_context"]["package_api"]["surfaces"])
        .iter()
        .flat_map(|surface| rows(&surface["exports"]))
        .collect()
}
fn digest(value: &Value, path: &Path) {
    assert_eq!(
        value,
        &json!(entmoot_contracts::blake3_hex(&std::fs::read(path).unwrap())),
        "digest for {}",
        path.display()
    );
}

fn lookup_digest<'a>(digests: &'a Value, path: &Path, field: &str) -> &'a Value {
    let digests = digests.as_object().unwrap_or_else(|| panic!("{field} object"));
    let key = path_key(path);
    if let Some(value) = digests.get(&key) {
        return value;
    }
    let wanted = retained(path);
    for (candidate, value) in digests {
        let got = retained(Path::new(candidate));
        if got == wanted
            || got.to_string_lossy().eq_ignore_ascii_case(&wanted.to_string_lossy())
        {
            return value;
        }
    }
    panic!(
        "missing {field} for {} among {:?}",
        path.display(),
        digests.keys().collect::<Vec<_>>()
    );
}

fn input_digest<'a>(build_context: &'a Value, path: &Path) -> &'a Value {
    lookup_digest(&build_context["input_digests"], path, "input_digests")
}

fn source_input_digest<'a>(build_context: &'a Value, path: &Path) -> &'a Value {
    lookup_digest(
        &build_context["source_input_digests"],
        path,
        "source_input_digests",
    )
}
fn clang_setup(root: &Path, source: &str, extra: &[&str]) -> Vec<(&'static str, String)> {
    write(root, "api.cpp", source);
    let args: Vec<_> = ["clang", "-std=c++20"]
        .into_iter()
        .chain(extra.iter().copied())
        .chain(["-c", "api.cpp"])
        .collect();
    document(
        root,
        "compile_commands.json",
        json!([{"directory":root,"file":"api.cpp","arguments":args}]),
    );
    vec![(
        "ENTMOOT_PACKAGE_API",
        document(
            root,
            "packages.json",
            json!({"packages":[{"ecosystem":"native","purl":"pkg:generic/api","root":root}]}),
        ),
    )]
}

#[test]
fn clang_template_body_layout_target_and_missing_compdb() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = "template<class T> T leaf(T x) { return x; }\nstruct Data { int value; };\nint run() { return leaf(1); }\n";
    let env = clang_setup(root, source, &[]);
    let first = native("CLANG", json!({"root":root}), &env);
    let first_exports = exports(&first);
    let template = first_exports
        .iter()
        .find(|r| r["boundary_policy"] == "libclang.template_declaration_headers.v1")
        .unwrap();
    let layout = first_exports
        .iter()
        .find(|r| r["boundary_policy"] == "libclang.record_layout_selected_target.v1")
        .unwrap();
    write(
        root,
        "api.cpp",
        &source.replace("return x;", "T copy = x; return copy;"),
    );
    let body = native("CLANG", json!({"root":root}), &env);
    assert_eq!(
        template["boundary_text"],
        exports(&body)
            .iter()
            .find(|r| r["symbol"] == template["symbol"])
            .unwrap()["boundary_text"]
    );
    write(
        root,
        "api.cpp",
        &source.replace("int value;", "long long value;"),
    );
    let changed = native("CLANG", json!({"root":root}), &env);
    assert_ne!(
        layout["boundary_text"],
        exports(&changed)
            .iter()
            .find(|r| r["symbol"] == layout["symbol"])
            .unwrap()["boundary_text"]
    );
    clang_setup(root, source, &["-target", "i686-pc-windows-msvc"]);
    let target = native("CLANG", json!({"root":root}), &env);
    assert_eq!(
        target["build_context"]["translation_unit_targets"][0]["pointer_width"],
        32
    );
    assert_ne!(
        layout["boundary_text"],
        exports(&target)
            .iter()
            .find(|r| r["symbol"] == layout["symbol"])
            .unwrap()["boundary_text"]
    );
    std::fs::remove_file(root.join("compile_commands.json")).unwrap();
    assert_eq!(
        native("CLANG", json!({"root":root}), &env)["invalid_environment"],
        true
    );
}

#[test]
fn clang_closed_vfs_negative_lookup_is_not_ambient_absence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let ambient = write(root, "outside/optional.h", "#define PRESENT 1\n");
    let source = format!("#if __has_include(\"{}\")\nint leaf(int x) {{ return x; }}\n#else\nlong leaf(long x) {{ return x; }}\n#endif\n",ambient.to_string_lossy().replace('\\',"/"));
    let mut env = clang_setup(root, &source, &[]);
    for (include, fallthrough, expected) in [
        (false, false, "long (long)"),
        (true, false, "int (int)"),
        (false, true, ""),
    ] {
        let mut files = vec![root.join("api.cpp")];
        if include {
            files.push(ambient.clone());
        }
        let overlay = json!({"version":0,"fallthrough":fallthrough,"use-external-names":false,"roots":files.iter().map(|p|json!({"type":"file","name":p,"external-contents":p})).collect::<Vec<_>>()});
        env.truncate(1);
        env.push(("ENTMOOT_CLANG_VFS", document(root, "overlay.json", overlay)));
        let value = native("CLANG", json!({"root":root}), &env);
        if fallthrough {
            assert_eq!(value["invalid_environment"], true);
        } else {
            assert_eq!(
                value["build_context"]["input_closure"],
                "declared_non_fallthrough_vfs_revalidated"
            );
            assert!(exports(&value)
                .iter()
                .any(|r| r["boundary_text"] == expected));
        }
    }
}

#[test]
fn go_named_types_tags_source_fences_and_test_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "go.mod", "module example.com/API\n\ngo 1.18\n");
    let source =
        "package api\ntype Count int\ntype Alias = Count\nfunc Leaf(x Count) Count { return x }\n";
    let file = write(root, "api.go", source);
    let tagged = write(
        root,
        "tagged.go",
        "//go:build extra\n\npackage api\nfunc Extra() int { return 1 }\n",
    );
    write(
        root,
        "api_test.go",
        "package api\nfunc ExportedHelper() { Leaf(1) }\n",
    );
    let first = native(
        "GO",
        json!({"root":root}),
        &[("ENTMOOT_GO_TESTS", "include".into())],
    );
    let public = exports(&first);
    assert!(public
        .iter()
        .any(|r| r["symbol"] == "example.com/API.Count"));
    assert!(!public
        .iter()
        .any(|r| r["symbol"].as_str().unwrap().ends_with(".Extra")
            || r["symbol"].as_str().unwrap().ends_with(".ExportedHelper")));
    digest(source_input_digest(&first["build_context"], &file), &file);
    assert!(
        lookup_digest(
            &first["build_context"]["source_input_digests"],
            &tagged,
            "source_input_digests",
        )
        .is_string()
    );
    let closed = rows(&first["calls"])
        .iter()
        .find(|r| r["path"] == "api_test.go" && r["dispatch"]["kind"] == "closed")
        .expect("expected Closed free-fn dispatch on api_test.go");
    assert_closed_fences(closed);
    let tagged = native(
        "GO",
        json!({"root":root}),
        &[
            ("ENTMOOT_GO_TAGS", "extra".into()),
            ("ENTMOOT_GO_TESTS", "exclude".into()),
        ],
    );
    assert!(exports(&tagged)
        .iter()
        .any(|r| r["symbol"].as_str().unwrap().ends_with(".Extra")));
    assert!(!rows(&tagged["calls"])
        .iter()
        .any(|r| r["path"].as_str().unwrap().ends_with("_test.go")));
    write(root, "api.go", &source.replace("Count int", "Count string"));
    let changed = native(
        "GO",
        json!({"root":root}),
        &[("ENTMOOT_GO_TESTS", "exclude".into())],
    );
    exports(&changed);
    assert_ne!(
        first["build_context"]["source_input_digests"],
        changed["build_context"]["source_input_digests"]
    );
}

#[test]
fn roslyn_missing_and_ambiguous_bindings_are_not_runtime_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Independently authored: MissingName has no symbol; the classic dual-null
    // overload is ambiguous. Neither establishes delegate or virtual dispatch.
    write(
        root,
        "Api.cs",
        r#"public static class Api {
    public static int Overload(string a, object b) { return 0; }
    public static int Overload(object a, string b) { return 1; }
    public static int Ambiguous() { return Overload(null, null); }
    public static int Missing() { return MissingName(1); }
}
"#,
    );
    let value = native("ROSLYN", json!({"root": root}), &[]);
    assert_eq!(
        value["invalid_environment"], false,
        "{}",
        value["diagnostics"]
    );
    let calls = rows(&value["calls"]);
    let ambiguous = calls
        .iter()
        .find(|r| r["path"] == "Api.cs" && rows(&r["targets"]).len() > 1)
        .expect("overload candidates retained");
    assert!(
        ambiguous["gap"].is_null(),
        "candidate symbols must not claim dynamic_dispatch: {ambiguous}"
    );
    assert!(
        rows(&ambiguous["targets"])
            .iter()
            .any(|t| t["symbol"].as_str().unwrap_or("").contains("Overload")),
        "{ambiguous}"
    );
    let missing = calls
        .iter()
        .find(|r| {
            r["gap"] == "unresolved" && rows(&r["targets"]).is_empty() && r["owner"].is_object()
        })
        .or_else(|| {
            calls
                .iter()
                .find(|r| r["gap"] == "unresolved" && rows(&r["targets"]).is_empty())
        })
        .expect("missing symbol retained as unresolved binding");
    assert_eq!(missing["gap"], "unresolved");
    assert!(
        !calls
            .iter()
            .any(|r| { r["gap"] == "dynamic_dispatch" && rows(&r["targets"]).len() > 1 }),
        "ambiguous bindings must not use dynamic_dispatch"
    );
    assert!(
        !calls
            .iter()
            .any(|r| r["gap"] == "indirect_call" && rows(&r["targets"]).is_empty()),
        "missing symbols must not use indirect_call"
    );
}

#[test]
fn roslyn_virtual_base_and_implicit_constructor_targets() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root,"Api.cs","public class Base { public virtual int Leaf() { return 1; } }\npublic class Child : Base { public override int Leaf() { return 2; } public int Direct() { return base.Leaf(); } }\npublic class Use { public int Run(Base value) { return value.Leaf(); } public Base Make() { return new Base(); } }\n");
    let value = native("ROSLYN", json!({"root":root}), &[]);
    assert_eq!(
        value["invalid_environment"], false,
        "{}",
        value["diagnostics"]
    );
    let calls = rows(&value["calls"]);
    let leaf: Vec<_> = calls
        .iter()
        .filter(|r| {
            rows(&r["targets"])
                .iter()
                .any(|t| t["symbol"] == "Base.Leaf()")
        })
        .collect();
    assert_eq!(leaf.len(), 2);
    assert!(leaf
        .iter()
        .any(|r| r["gap"] == "dynamic_dispatch" && rows(&r["targets"]).len() == 1));
    assert!(leaf.iter().any(|r| r["gap"].is_null()));
    let constructors: Vec<_> = calls
        .iter()
        .flat_map(|r| rows(&r["targets"]))
        .filter(|t| t["symbol"].as_str().unwrap().ends_with("Base()"))
        .collect();
    assert!(!constructors.is_empty());
    assert!(constructors.iter().all(|t| t["kind"] == "type"));
}

fn project(root: &Path, name: &str, assembly: &str, source: &str) {
    let path = write(root,&format!("{name}/{name}.csproj"),&format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net9.0</TargetFramework><AssemblyName>{assembly}</AssemblyName></PropertyGroup></Project>"));
    write(root, &format!("{name}/Api.cs"), source);
    run(Command::new("dotnet").args([
        "restore",
        path.to_str().unwrap(),
        "--ignore-failed-sources",
    ]));
}

#[test]
fn roslyn_project_load_failure_does_not_become_adhoc_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Broken.csproj", "<Project><malformed");
    write(
        root,
        "Api.cs",
        "public class Api { public int Leaf() => System.Math.Abs(-1); }",
    );
    let value = native("ROSLYN", json!({"root":root}), &[]);
    assert_eq!(
        value["invalid_environment"], true,
        "{}",
        value["diagnostics"]
    );
    assert_eq!(value["build_context"]["project_system"], "msbuild_project");
    assert!(!rows(&value["diagnostics"]).is_empty());
    assert!(rows(&value["calls"]).is_empty(), "{}", value["calls"]);
}

#[test]
fn roslyn_evaluated_projects_import_digests_and_conflicting_exports() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let props_source = "<Project><PropertyGroup><LangVersion>13</LangVersion><DefineConstants>FIRST</DefineConstants></PropertyGroup></Project>";
    let props = write(root, "Directory.Build.props", props_source);
    for name in ["One", "Two"] {
        project(root,name,name,&format!("public class {name} {{ public static int Leaf() {{ return 1; }} public int Run() {{ return Leaf(); }} }}"));
    }
    let first = native("ROSLYN", json!({"root":root}), &[]);
    assert_eq!(
        first["invalid_environment"], false,
        "{}",
        first["diagnostics"]
    );
    assert_eq!(
        rows(&first["calls"])
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["One/Api.cs", "Two/Api.cs"])
    );
    assert_eq!(rows(&first["build_context"]["evaluated_projects"]).len(), 2);
    let before = input_digest(&first["build_context"], &props).clone();
    digest(&before, &props);
    write(
        root,
        "Directory.Build.props",
        &props_source.replace("FIRST", "SECOND"),
    );
    let changed = native("ROSLYN", json!({"root":root}), &[]);
    assert_eq!(changed["invalid_environment"], false);
    assert_ne!(&before, input_digest(&changed["build_context"], &props));
    for (name, ty) in [("One", "int"), ("Two", "long")] {
        project(
            root,
            name,
            "Shared",
            &format!("public class Api {{ public static {ty} Leaf() {{ return 1; }} }}"),
        );
    }
    let mapping = document(
        root,
        "packages.json",
        json!({"packages":[{"ecosystem":"csharp","assembly":"Shared","name":"example.api"}]}),
    );
    let conflicting = native(
        "ROSLYN",
        json!({"root":root}),
        &[("ENTMOOT_PACKAGE_API", mapping)],
    );
    assert_eq!(conflicting["invalid_environment"], true);
    assert!(rows(&conflicting["diagnostics"])
        .iter()
        .any(|r| r.as_str().unwrap().contains("conflicting project export")));
}

fn java_env(root: &Path, release: &str) -> Vec<(&'static str, String)> {
    let root = retained(root);
    let mapping =
        json!({"packages":[{"ecosystem":"java","name":"example/api","root": root.as_path()}]});
    vec![
        ("ENTMOOT_JAVA_RELEASE", release.into()),
        ("ENTMOOT_JAVA_CLASSPATH", String::new()),
        (
            "ENTMOOT_PACKAGE_API",
            document(&root, "packages.json", mapping),
        ),
    ]
}

#[test]
fn java_unresolved_declaration_is_not_indirect_call() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Independently authored: MissingName is absent. Javac still emits the call site.
    write(
        root,
        "Api.java",
        "public class Api { public static int missing() { return MissingName.run(1); } }\n",
    );
    let value = native(
        "JAVA",
        json!({"root": root}),
        &[
            ("ENTMOOT_JAVA_RELEASE", "17".into()),
            ("ENTMOOT_JAVA_CLASSPATH", String::new()),
        ],
    );
    let calls = rows(&value["calls"]);
    assert!(
        calls.iter().any(|r| r["gap"] == "unresolved"),
        "expected unresolved binding, got {calls:?}; diagnostics={}",
        value["diagnostics"]
    );
    assert!(
        !calls
            .iter()
            .any(|r| r["gap"] == "indirect_call" || r["gap"] == "dynamic_dispatch"),
        "missing declaration must not claim runtime dispatch: {calls:?}"
    );
}

#[test]
fn java_dispatch_default_scope_and_unsupported_engine() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root,"Api.java","public class Api { public int leaf() { return 1; } }\nclass Child extends Api { public int leaf() { return 2; } public int direct() { return super.leaf(); } }\nclass Use { public int run(Api value) { return value.leaf(); } }\n");
    let value = native("JAVA", json!({"root":root}), &java_env(root, "17"));
    assert!(!exports(&value).is_empty());
    let calls: Vec<_> = rows(&value["calls"])
        .iter()
        .filter(|r| {
            rows(&r["targets"])
                .iter()
                .any(|t| t["symbol"] == "Api#leaf()")
        })
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().any(|r| r["gap"] == "dynamic_dispatch"));
    assert!(calls.iter().any(|r| r["gap"].is_null()));
    write(root, "Api.java", "public record Api(int value) {}");
    let default = native(
        "JAVA",
        json!({"root":root}),
        &[("ENTMOOT_JAVA_RELEASE", "17".into())],
    );
    assert_eq!(default["invalid_environment"], false);
    assert_eq!(default["build_context"]["project_system"], "javac_trees");
    for name in ["A", "B", "Excluded"] {
        write(
            root,
            &format!("{name}.java"),
            &format!("public class {name} {{ public static int api() {{ return 1; }} }}"),
        );
    }
    let value = native(
        "JAVA",
        json!({"root":root,"patterns":["A.java","B.java"]}),
        &java_env(root, "17"),
    );
    assert_eq!(
        exports(&value)
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["A.java", "B.java"])
    );
    // Retired ENTMOOT_JAVA_ENGINE (including spoon) and unknown values fail closed.
    for engine in ["typo", "spoon", "javac"] {
        let mut env = java_env(root, "17");
        env.push(("ENTMOOT_JAVA_ENGINE", engine.into()));
        assert_eq!(
            native("JAVA", json!({"root":root}), &env)["invalid_environment"],
            true
        );
    }
}

#[test]
fn javac_historical_modes_reject_newer_syntax_at_old_floor() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (release,source) in [
        ("8","public class Api<T extends Number> { public static int leaf(int x) { return x; } }"),
        ("11","public class Api { public static int leaf() { var x = 1; return x; } }"),
        ("17","public record Api(int value) { public int leaf() { return value; } }"),
        ("21","public record Api(int value) { public static int leaf(Object x) { return switch (x) { case Api(int n) -> n; default -> 0; }; } }"),
        ("25","public class Api { private final int x; public Api(int value) { int checked = Math.abs(value); super(); x = checked; } public int leaf() { return x; } }")
    ] {
        let file = write(root,"Api.java",source);
        let value = native("JAVA",json!({"root":root}),&java_env(root,release));
        assert!(!exports(&value).is_empty());
        assert_eq!(value["build_context"]["source_release"],release);
        digest(input_digest(&value["build_context"], &file), &file);
    }
    write(root, "Api.java", "public record Api(int value) {}");
    assert_eq!(
        native("JAVA", json!({"root":root}), &java_env(root, "8"))["invalid_environment"],
        true
    );
}

#[test]
fn javac_binary_authority_requires_the_declared_classpath() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let dependency = write(root,"dependency/Api.java","package dep; public final class Api { public static <T extends Number> T leaf(T x) { return x; } }");
    let classes = root.join("classes");
    let jar = root.join("api.jar");
    run(Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&classes)
        .arg(&dependency));
    run(Command::new("jar")
        .args(["--create", "--file"])
        .arg(&jar)
        .arg("-C")
        .arg(&classes)
        .arg("."));
    std::fs::remove_file(dependency).unwrap();
    write(
        root,
        "Use.java",
        "public class Use { public int run() { return dep.Api.leaf(1); } }",
    );
    let mapping = document(
        root,
        "packages.json",
        json!({"packages":[{"ecosystem":"java","name":"example/api","jar":jar}]}),
    );
    let profile = document(
        root,
        "java.json",
        json!({"release":"17","classpath":["api.jar"]}),
    );
    let env = [
        ("ENTMOOT_JAVA_ENVIRONMENT", profile),
        ("ENTMOOT_PACKAGE_API", mapping),
    ];
    let value = native("JAVA", json!({"root":root}), &env);
    assert_eq!(
        value["invalid_environment"], false,
        "{}",
        value["diagnostics"]
    );
    let refs = rows(&value["build_context"]["package_api"]["references"]);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0]["boundary_policy"],
        "javac.public_binary_method_contract.v1"
    );
    assert_eq!(refs[0]["target"]["package"]["name"], "example/api");
    digest(input_digest(&value["build_context"], &jar), &jar);
    document(root, "java.json", json!({"release":"17","classpath":[]}));
    assert_eq!(
        native("JAVA", json!({"root":root}), &env)["invalid_environment"],
        true
    );
}

#[test]
fn go_historical_modes_preserve_generics_and_reject_range_over_int_at_floor() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = "package api\ntype Box[T any] struct { Value T }\nfunc (box Box[T]) Get() T { return box.Value }\nfunc Leaf[T any](x T) T { return x }\n";
    for version in ["1.18", "1.20", "1.22", "1.24"] {
        write(
            root,
            "go.mod",
            &format!("module example.test/API\ngo {version}\n"),
        );
        write(root, "api.go", source);
        let value = native(
            "GO",
            json!({"root":root}),
            &[("ENTMOOT_GO_SINGLE_MODULE", "1".into())],
        );
        assert!(!exports(&value).is_empty(), "{version}");
    }
    write(root, "go.mod", "module example.test/API\ngo 1.18\n");
    write(
        root,
        "api.go",
        "package api\nfunc Leaf() { for range 3 {} }\n",
    );
    let rejected = native(
        "GO",
        json!({"root":root}),
        &[("ENTMOOT_GO_SINGLE_MODULE", "1".into())],
    );
    assert!(rejected["invalid_environment"] == true || !rows(&rejected["diagnostics"]).is_empty());
    assert!(rows(&rejected["build_context"]["package_api"]["surfaces"])
        .iter()
        .all(|s| rows(&s["exports"]).is_empty()));
}

#[test]
fn roslyn_historical_modes_preserve_generic_constraints() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root,"Api.cs","public class Api<T> where T : class, new() { [System.Obsolete] public T Leaf(T value) { return value; } public static int Stable(int value) { return value; } }");
    let mapping = document(
        root,
        "packages.json",
        json!({"packages":[{"ecosystem":"csharp","name":"example.api","assembly":"Api"}]}),
    );
    for version in ["7.3", "8", "10", "12", "13"] {
        let project = write(root,"Api.csproj",&format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net9.0</TargetFramework><LangVersion>{version}</LangVersion><Nullable>disable</Nullable></PropertyGroup></Project>"));
        run(Command::new("dotnet")
            .arg("restore")
            .arg(project)
            .arg("--ignore-failed-sources"));
        let value = native(
            "ROSLYN",
            json!({"root":root}),
            &[("ENTMOOT_PACKAGE_API", mapping.clone())],
        );
        assert!(!exports(&value).is_empty(), "{version}");
    }
}

#[test]
fn clang_historical_modes_keep_headers_macros_and_layout_inputs() {
    for (family, versions) in [
        ("c", vec!["c99", "c11", "c17", "c2x"]),
        ("cpp", vec!["c++11", "c++14", "c++17", "c++20", "c++23"]),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            root,
            "include/fixture.h",
            "#define STEP 1\nstruct Data { int value; };\n",
        );
        let class = if family == "cpp" {
            "class Api { public: int leaf(int x) const { return x + STEP; } };\n"
        } else {
            ""
        };
        let name = format!("api.{family}");
        write(root,&name,&format!("#include \"fixture.h\"\n{class}int leaf(int x) {{ return x + STEP; }}\nint caller(void) {{ return leaf(1); }}\n"));
        let mapping = document(
            root,
            "packages.json",
            json!({"packages":[{"ecosystem":"native","purl":"pkg:generic/api","root":root}]}),
        );
        for version in versions {
            document(
                root,
                "compile_commands.json",
                json!([{"directory":root,"file":name,"arguments":["clang",format!("-std={version}"),"-Iinclude","-c",name]}]),
            );
            let value = native(
                "CLANG",
                json!({"root":root}),
                &[("ENTMOOT_PACKAGE_API", mapping.clone())],
            );
            assert!(!exports(&value).is_empty(), "{version}");
            assert!(rows(&value["inputs"]).iter().any(|p| p
                .as_str()
                .unwrap()
                .replace('\\', "/")
                .ends_with("/include/fixture.h")));
            for field in ["macro_definitions", "record_layouts", "compile_invocations"] {
                assert!(
                    !rows(&value["build_context"][field]).is_empty(),
                    "{version}: {field}"
                );
            }
        }
    }
}

#[test]
fn clang_compilation_directories_keep_relative_header_origins_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut commands = Vec::new();
    let mut headers = BTreeSet::new();
    for (name, value) in [("first", 1), ("second", 2)] {
        let directory = root.join(name);
        let header = write(
            &directory,
            "include/config.h",
            &format!("#define VALUE {value}\n"),
        );
        headers.insert(header.to_string_lossy().replace('\\', "/"));
        write(
            &directory,
            "api.c",
            &format!("#include \"config.h\"\nint {name}(void) {{ return VALUE; }}\n"),
        );
        commands.push(json!({"directory":directory,"file":"api.c","arguments":["clang","-std=c17","-Iinclude","-c","api.c"]}));
    }
    document(root, "compile_commands.json", json!(commands));
    let mapping = document(
        root,
        "packages.json",
        json!({"packages":[{"ecosystem":"native","purl":"pkg:generic/api","root":root}]}),
    );
    let value = native(
        "CLANG",
        json!({"root":root}),
        &[("ENTMOOT_PACKAGE_API", mapping)],
    );
    assert_eq!(
        exports(&value)
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["first/api.c", "second/api.c"])
    );
    let normalize = |path: &str| {
        let unified = retained(Path::new(path))
            .to_string_lossy()
            .replace('\\', "/");
        if cfg!(windows) {
            unified.to_ascii_lowercase()
        } else {
            unified
        }
    };
    let inputs: BTreeSet<_> = rows(&value["inputs"])
        .iter()
        .map(|r| normalize(r.as_str().unwrap()))
        .collect();
    let headers: BTreeSet<_> = headers.iter().map(|path| normalize(path)).collect();
    assert!(
        headers.is_subset(&inputs),
        "headers={headers:?}, inputs={inputs:?}"
    );
    let macros: Vec<_> = rows(&value["build_context"]["macro_definitions"])
        .iter()
        .filter(|r| r["name"] == "VALUE")
        .collect();
    assert_eq!(macros.len(), 2);
    assert_ne!(macros[0]["tokens"], macros[1]["tokens"]);
    assert_eq!(
        macros
            .iter()
            .map(|m| normalize(m["source"].as_str().unwrap()))
            .collect::<BTreeSet<_>>(),
        headers
    );
    let database = root.join("compile_commands.json");
    let explicit = root.join("selected/compile_commands.json");
    std::fs::create_dir(explicit.parent().unwrap()).unwrap();
    std::fs::rename(&database, &explicit).unwrap();
    let selected = native(
        "CLANG",
        json!({"root":root}),
        &[(
            "ENTMOOT_CLANG_COMPDB",
            explicit.to_string_lossy().into_owned(),
        )],
    );
    assert_eq!(
        selected["invalid_environment"], false,
        "{}",
        selected["diagnostics"]
    );
    assert_eq!(
        rows(&selected["build_context"]["compile_invocations"]).len(),
        2
    );
    let invalid = native(
        "CLANG",
        json!({"root":root}),
        &[(
            "ENTMOOT_CLANG_COMPDB",
            database.to_string_lossy().into_owned(),
        )],
    );
    assert_eq!(invalid["invalid_environment"], true);
}

#[test]
fn native_declared_boundaries_ignore_bodies_but_retain_constraints_and_annotations() {
    use std::collections::BTreeMap;
    for family in ["java", "csharp"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut baseline = BTreeMap::new();
        for shape in ["base", "body", "constraint", "annotation"] {
            let body = if shape == "body" {
                "T local = value; return local;"
            } else {
                "return value;"
            };
            let value = if family == "java" {
                let constraint = if shape == "constraint" {
                    "CharSequence"
                } else {
                    "Number"
                };
                let annotation = if shape == "annotation" {
                    ""
                } else {
                    "@Deprecated "
                };
                write(root,"Api.java",&format!("public class Api<T extends {constraint}> {{ {annotation}public T leaf(T value) {{ {body} }} }}"));
                native("JAVA", json!({"root":root}), &java_env(root, "17"))
            } else {
                let constraint = if shape == "constraint" {
                    "class"
                } else {
                    "class, new()"
                };
                let annotation = if shape == "annotation" {
                    ""
                } else {
                    "[System.Obsolete] "
                };
                project(root,"Api","Api",&format!("public class Api<T> where T : {constraint} {{ {annotation}public T Leaf(T value) {{ {body} }} }}"));
                let mapping = document(
                    root,
                    "packages.json",
                    json!({"packages":[{"ecosystem":"csharp","name":"example.api","assembly":"Api"}]}),
                );
                native(
                    "ROSLYN",
                    json!({"root":root}),
                    &[("ENTMOOT_PACKAGE_API", mapping)],
                )
            };
            let contracts: BTreeMap<_, _> = exports(&value)
                .iter()
                .map(|r| {
                    (
                        r["symbol"].as_str().unwrap().to_owned(),
                        r["boundary_text"].clone(),
                    )
                })
                .collect();
            assert_eq!(contracts.len(), 2, "{family}/{shape}");
            match shape {
                "base" => baseline = contracts,
                "body" => assert_eq!(
                    contracts, baseline,
                    "{family}: implementation body changed declaration contract"
                ),
                _ => assert!(
                    contracts.iter().all(|(k, v)| baseline.get(k) != Some(v)),
                    "{family}/{shape}: omitted declaration change"
                ),
            }
        }
    }
}

#[test]
#[ignore = "requires installed Rust 1.79.0, 1.91 and 1.95.0 toolchains"]
fn pinned_rust_compilers_preserve_trait_macro_and_generic_syntax() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root,"lib.rs","pub struct Api<T>(pub T); pub trait Get<T> { fn get(&self) -> &T; }\nimpl<T> Get<T> for Api<T> { fn get(&self) -> &T { &self.0 } }\n#[macro_export] macro_rules! identity { ($x:expr) => { $x }; }\n");
    for version in ["1.79.0", "1.91", "1.95.0"] {
        run(Command::new("rustup")
            .args([
                "run",
                version,
                "rustc",
                "--edition=2021",
                "--crate-type=lib",
                "lib.rs",
            ])
            .current_dir(root));
    }
}

#[test]
fn rust_feature_selection_does_not_enable_test_only_declarations() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "cfg.rs",
        r#"
#[cfg(feature = "full-tests")] pub fn feature_only() {}
#[test] fn test_only() {}
#[cfg(feature = "probe_feature")] pub fn feature_probe() { feature_only(); }
#[cfg(feature = "probe_test")] pub fn test_probe() { test_only(); }
"#,
    );
    for (accepted, flags) in [
        (false, vec!["--cfg", "feature=\"probe_feature\""]),
        (
            true,
            vec![
                "--cfg",
                "feature=\"probe_feature\"",
                "--cfg",
                "feature=\"full-tests\"",
            ],
        ),
        (
            false,
            vec![
                "--cfg",
                "feature=\"probe_test\"",
                "--cfg",
                "feature=\"full-tests\"",
            ],
        ),
        (
            true,
            vec![
                "--test",
                "--cfg",
                "feature=\"probe_test\"",
                "--cfg",
                "feature=\"full-tests\"",
            ],
        ),
    ] {
        let output = Command::new("rustup")
            .args([
                "run",
                "1.95.0",
                "rustc",
                "--edition=2021",
                "--crate-type=lib",
                "--emit=metadata",
                "cfg.rs",
                "-o",
                "cfg.rmeta",
            ])
            .args(flags)
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            accepted,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !accepted {
            assert!(String::from_utf8_lossy(&output.stderr).contains("E0425"));
        }
    }
}
