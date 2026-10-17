//! Process-level controls for the optional Python/SQLite campaign harness.
//! Run with `cargo test -p entmoot --test campaign_harness -- --ignored`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn python(module: &str, args: &[&str]) -> Output {
    Command::new(std::env::var("ENTMOOT_CAMPAIGN_PYTHON").unwrap_or_else(|_| "python".into()))
        .args(["-X", "utf8", "-m", module])
        .args(args)
        .current_dir(workspace())
        .output()
        .expect("the requested campaign lane requires Python and evals/harness/requirements.txt")
}

fn successful(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn write(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
#[ignore = "requires the Python campaign dependencies"]
fn knowledge_reader_is_scoped_lazy_read_only_and_fingerprint_qualified() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("knowledge.sqlite3");
    let locator = temp.path().join("publication.view.json");
    let expected = successful(
        Command::new(env!("CARGO_BIN_EXE_entmoot"))
            .arg("fingerprint")
            .output()
            .unwrap(),
    );
    let view = format!("blake3:{}", "a".repeat(64));
    let query = |path: &Path, fingerprint: &Value, sql: &str| {
        Command::new(std::env::var("ENTMOOT_CAMPAIGN_PYTHON").unwrap_or_else(|_| "python".into()))
            .args(["-X", "utf8", "-c", "import json,sys; from pathlib import Path; from evals.harness.knowledge_sql import connect_store; c=connect_store(Path(sys.argv[1]),expected_fingerprint=json.loads(sys.argv[2])); rows=c.execute(sys.argv[3]).fetchall(); print(json.dumps([[x.decode('utf-8') if isinstance(x,bytes) else x for x in row] for row in rows])); c.close()"])
            .arg(path).arg(fingerprint.to_string()).arg(sql).current_dir(workspace()).output().unwrap()
    };
    assert!(!query(&db, &expected, "SELECT 1").status.success());
    assert!(!db.exists(), "read failure created a store");
    let writer = rusqlite::Connection::open(&db).unwrap();
    writer.execute_batch("PRAGMA user_version=1;
        CREATE TABLE storage_fingerprint(singleton INTEGER PRIMARY KEY,body TEXT);
        CREATE TABLE shared_payloads(payload_id INTEGER PRIMARY KEY,digest BLOB,body BLOB);
        CREATE TABLE shared_metadata(metadata_id INTEGER PRIMARY KEY,body BLOB);
        CREATE TABLE scoped_records(view_id TEXT,collection TEXT,record_id TEXT,artifact_id TEXT,subject_id TEXT,object_id TEXT,proposition_id TEXT,predicate TEXT,location_id TEXT,payload INTEGER,visible INTEGER,ownership_managed INTEGER);
        CREATE INDEX knowledge_by_subject_predicate ON scoped_records(view_id,collection,subject_id,predicate);
        CREATE TABLE scoped_metadata(view_id TEXT,key TEXT,value TEXT);").unwrap();
    writer
        .execute(
            "INSERT INTO storage_fingerprint VALUES(1,?)",
            [expected.to_string()],
        )
        .unwrap();
    let canonical = br#"{"float":1e-5,"scope":{"repository":"fixture"}}"#;
    let digest = entmoot_contracts::blake3_hex(canonical);
    let bytes: Vec<u8> = digest.as_bytes()[7..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap())
        .collect();
    writer
        .execute(
            "INSERT INTO shared_metadata VALUES(1,?)",
            [br#"{"repository":"fixture"}"#.as_slice()],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO shared_payloads VALUES(1,?,?)",
            rusqlite::params![
                bytes,
                br#"{"body":{"float":1e-5},"metadata":{"scope":1}}"#.as_slice()
            ],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO shared_payloads VALUES(2,?,?)",
            rusqlite::params![b"broken".as_slice(), b"broken".as_slice()],
        )
        .unwrap();
    for (scope, id, payload) in [
        (&view, "selected", 1),
        (&view, "unrelated", 2),
        (&"default".into(), "foreign", 1),
    ] {
        writer.execute("INSERT INTO scoped_records(view_id,collection,record_id,payload,visible) VALUES(?,'assertions',?,?,1)",rusqlite::params![scope,id,payload]).unwrap();
    }
    drop(writer);
    write(
        &locator,
        &json!({"database":"knowledge.sqlite3","view":view}),
    );
    let before = std::fs::read(&db).unwrap();
    assert_eq!(
        successful(query(
            &locator,
            &expected,
            "SELECT payload FROM knowledge_records WHERE record_id='selected'"
        )),
        json!([[std::str::from_utf8(canonical).unwrap()]])
    );
    assert_eq!(
        successful(query(
            &locator,
            &expected,
            "SELECT count(*) FROM knowledge_records"
        )),
        json!([[2]])
    );
    assert!(!query(
        &locator,
        &expected,
        "SELECT payload FROM knowledge_records WHERE record_id='unrelated'"
    )
    .status
    .success());
    assert!(
        !query(&locator, &expected, "DELETE FROM main.scoped_records")
            .status
            .success()
    );
    for component in ["contracts", "derivation", "layout", "series"] {
        let mut wrong = expected.clone();
        if ["contracts", "derivation"].contains(&component) {
            wrong["implementation"][component] = json!(format!("blake3:{}", "f".repeat(64)));
        } else if component == "layout" {
            wrong[component] = json!(format!("blake3:{}", "f".repeat(64)));
        } else {
            wrong[component] = json!("entmoot.storage.obsolete");
        }
        assert!(
            !query(&locator, &wrong, "SELECT 1").status.success(),
            "{component}"
        );
    }
    assert_eq!(
        before,
        std::fs::read(&db).unwrap(),
        "reader mutated source storage"
    );
    if cfg!(windows) {
        write(
            &locator,
            &json!({"database":format!(r"\\?\{}",db.display()),"view":view}),
        );
        assert_eq!(
            successful(query(
                &locator,
                &expected,
                "SELECT count(*) FROM knowledge_records"
            )),
            json!([[2]])
        );
    }
    for mutation in [
        "UPDATE storage_fingerprint SET body='{}'",
        "DELETE FROM storage_fingerprint",
        "PRAGMA user_version=4",
    ] {
        let writer = rusqlite::Connection::open(&db).unwrap();
        writer.execute_batch(mutation).unwrap();
        drop(writer);
        let before = std::fs::read(&db).unwrap();
        assert!(
            !query(&locator, &expected, "SELECT 1").status.success(),
            "{mutation}"
        );
        assert_eq!(before, std::fs::read(&db).unwrap());
    }
}

#[test]
#[ignore = "requires the Python campaign dependencies"]
fn sqlite_reopen_retry_context_fence_and_atomic_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("campaign.sqlite3");
    let context = tmp.path().join("context.json");
    let rows = tmp.path().join("rows.json");
    write(
        &context,
        &json!({"repository":"authored/repo", "revision":"a", "tool":"one"}),
    );
    let invoke = |operation: &str, extra: &[&str]| {
        let mut args = vec![
            operation,
            "--db",
            db.to_str().unwrap(),
            "--context",
            context.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        python("evals.harness.revision_campaign", &args)
    };
    successful(invoke("create", &[]));
    write(
        &rows,
        &json!({"build":{"status":"failed", "reason":"missing tool"}}),
    );
    successful(invoke("put", &["--rows", rows.to_str().unwrap()]));
    // A retried operation replaces its result in the same, still-qualified run.
    write(&rows, &json!({"build":{"status":"complete"}}));
    successful(invoke("put", &["--rows", rows.to_str().unwrap()]));
    std::fs::write(
        &context,
        "{\n \"tool\":\"one\", \"revision\":\"a\", \"repository\":\"authored/repo\"\n}",
    )
    .unwrap();
    assert_eq!(
        successful(invoke("get", &["--key", "build"])),
        json!({"status":"complete"})
    );

    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_bad BEFORE INSERT ON receipts WHEN NEW.receipt_key = 'bad' BEGIN SELECT RAISE(ABORT, 'authored failure'); END;").unwrap();
    write(&rows, &json!({"a-first":{"value":1}, "bad":{"value":2}}));
    assert!(!invoke("put", &["--rows", rows.to_str().unwrap()])
        .status
        .success());
    assert_eq!(
        successful(invoke("get", &["--key", "a-first"])),
        Value::Null
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM receipts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(connection);

    write(
        &context,
        &json!({"repository":"authored/repo", "revision":"b", "tool":"one"}),
    );
    assert!(!invoke("get", &["--key", "build"]).status.success());
    assert!(!invoke("put", &["--rows", rows.to_str().unwrap()])
        .status
        .success());
}

#[test]
#[ignore = "requires the Python campaign dependencies"]
fn foreign_sqlite_database_is_not_adopted_or_migrated() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("foreign.sqlite3");
    let context = tmp.path().join("context.json");
    write(&context, &json!({"run":"foreign control"}));
    rusqlite::Connection::open(&db).unwrap().execute_batch("CREATE TABLE owned_elsewhere(value TEXT); INSERT INTO owned_elsewhere VALUES ('keep');").unwrap();
    for operation in ["create", "get"] {
        let result = python(
            "evals.harness.revision_campaign",
            &[
                operation,
                "--db",
                db.to_str().unwrap(),
                "--context",
                context.to_str().unwrap(),
                "--key",
                "missing",
            ],
        );
        assert!(!result.status.success());
    }
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM owned_elsewhere", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'campaign_context'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn repository(root: &Path, lock: Value) -> String {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Campaign fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "core.autocrlf", "false"]);
    // Fixture repositories have no inherited hooks or signing requirements.
    git(root, &["config", "core.hooksPath", "empty-hooks"]);
    std::fs::write(
        root.join("source.py"),
        "def leaf(value: int) -> int:\n    return value\n\ndef caller():\n    return leaf(1)\n",
    )
    .unwrap();
    write(&root.join("package-lock.json"), &lock);
    git(root, &["add", "."]);
    git(
        root,
        &[
            "commit",
            "--no-gpg-sign",
            "--quiet",
            "-m",
            "authored source",
        ],
    );
    git(root, &["rev-parse", "HEAD"])
}

#[test]
#[ignore = "requires Python campaign dependencies and Git; runs the real Entmoot CLI"]
fn pinned_repositories_build_and_publish_at_bounded_dependency_depth() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("original-root");
    let dep = tmp.path().join("original-dependency");
    let root_sha = repository(
        &root,
        json!({"lockfileVersion":3,"packages":{
            "":{"dependencies":{"library":"1", "missing":"1"}},
            "node_modules/library":{"version":"1.0.0","dependencies":{"transitive":"2"}},
            "node_modules/missing":{"version":"1.0.0"},
            "node_modules/transitive":{"version":"2.0.0"}
        }}),
    );
    let dep_sha = repository(
        &dep,
        json!({"lockfileVersion":3,"packages":{
            "":{"dependencies":{"root":"1"}}, "node_modules/root":{"version":"1.0.0"}
        }}),
    );
    let config = tmp.path().join("repos.json");
    let run = tmp.path().join("run");
    let build = json!([[
        "{python}",
        "-c",
        "from pathlib import Path; import sys; Path(sys.argv[1]).write_text('built')",
        "{build}/built.txt"
    ]]);
    write(
        &config,
        &json!({"roots":["root"],"repositories":{
            "root":{"url":root,"revision":root_sha,"build":build,"lock":"package-lock.json"},
            "library":{"url":dep,"revision":dep_sha,"build":build,"lock":"package-lock.json"}
        },"packages":{"pkg:npm/library@1.0.0":"library", "pkg:npm/root@1.0.0":"root"},
        "checks":[
            {"name":"find-caller","arguments":["request","--graph","{graph:root}","--store","{store:root}","--request","{request}"],"request":{"operation":"find","request":{"kind":"subject","query":"caller","limit":2}}},
            {"name":"direct-boundary","arguments":["assess","compatibility","--consumer","{root:root}","--provider","{root:library}"]}
        ]}),
    );
    let invoke = |depth: &str, destination: &Path| {
        python(
            "evals.harness.repository_campaign",
            &[
                "run",
                "--config",
                config.to_str().unwrap(),
                "--run-dir",
                destination.to_str().unwrap(),
                "--cli",
                env!("CARGO_BIN_EXE_entmoot"),
                "--depth",
                depth,
                "--structural-only",
            ],
        )
    };
    let summary = successful(invoke("1", &run));
    assert!(summary["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["status"] == "complete"));
    assert!(summary["checks"][0]["response"].is_object());
    assert_eq!(summary["repositories"].as_array().unwrap().len(), 2);
    assert!(summary["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .all(|repo| repo["build_status"] == "complete" && repo["inventory_status"] == "complete"));
    assert!(summary["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["package"] == "pkg:npm/missing@1.0.0" && row["status"] == "unmapped"));
    assert!(!summary["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["package"] == "pkg:npm/transitive@2.0.0"));
    assert!(run.join("build/root/built.txt").is_file());
    assert!(summary["repositories"][0]["publication"].is_object());
    assert!(!run.join("sources/root/.entmoot").exists());
    let repeated = successful(invoke("1", &run));
    assert!(repeated["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["build_reused"] == true));
    assert_eq!(git(&root, &["status", "--porcelain"]), "");
    assert!(!root.join(".entmoot").exists());
    let cycle = successful(invoke("2", &tmp.path().join("cycle")));
    assert_eq!(cycle["repositories"].as_array().unwrap().len(), 2);
    assert!(cycle["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["status"] == "already_selected"));
    let shallow = successful(invoke("0", &tmp.path().join("shallow")));
    assert_eq!(shallow["checks"][1]["status"], "unavailable");
    assert_eq!(shallow["repositories"].as_array().unwrap().len(), 1);
    assert!(shallow["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["status"] == "depth_limit"));
}

#[test]
#[ignore = "requires Python campaign dependencies and Git"]
fn direct_parent_selection_never_substitutes_the_previous_selected_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("source");
    let a = repository(&root, json!({"lockfileVersion":3,"packages":{"":{}}}));
    let mut revisions = vec![a];
    for value in [2, 3] {
        std::fs::write(
            root.join("source.py"),
            format!("def value():\n    return {value}\n"),
        )
        .unwrap();
        git(&root, &["add", "."]);
        git(
            &root,
            &[
                "commit",
                "--no-gpg-sign",
                "--quiet",
                "-m",
                "authored change",
            ],
        );
        revisions.push(git(&root, &["rev-parse", "HEAD"]));
    }
    let call = |args: &[&str]| {
        let mut command = vec!["comparisons", "--repository", root.to_str().unwrap()];
        command.extend_from_slice(args);
        python("evals.harness.revision_campaign", &command)
    };
    let parent = successful(call(&["--head", &revisions[2]]));
    assert_eq!(parent["comparisons"][0]["base_revision"], revisions[1]);
    assert_eq!(
        parent["comparisons"][0]["relation"]["kind"],
        "direct_parent"
    );
    let window = successful(call(&[
        "--window",
        &revisions[0],
        &revisions[2],
        "skip middle intentionally",
    ]));
    assert_eq!(window["comparisons"][0]["base_revision"], revisions[0]);
    assert_eq!(
        window["comparisons"][0]["relation"]["kind"],
        "selected_window"
    );
    assert!(!call(&["--head", &revisions[0]]).status.success());
    assert!(!call(&["--head", "HEAD"]).status.success());
    assert!(!call(&["--head", &revisions[2], "--head", &revisions[2]])
        .status
        .success());
}

#[test]
#[ignore = "requires the Python campaign dependencies"]
fn lock_inventory_preserves_multiple_versions_and_ambiguous_or_unsupported_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let invoke = |name| {
        python(
            "evals.harness.repository_campaign",
            &["locks", "--root", root.to_str().unwrap(), "--lock", name],
        )
    };
    write(
        &root.join("package-lock.json"),
        &json!({"lockfileVersion":3,"packages":{
            "":{"dependencies":{"lib":"1","alias":"npm:lib@2"},"devDependencies":{"tool":"3"}},
            "node_modules/lib":{"version":"1.0.0"},
            "node_modules/alias":{"name":"lib","version":"2.0.0"},
            "node_modules/tool":{"version":"3.0.0"}
        }}),
    );
    assert_eq!(
        successful(invoke("package-lock.json"))["packages"],
        json!(["pkg:npm/lib@1.0.0", "pkg:npm/lib@2.0.0"])
    );
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='authored'\nversion='1.0.0'\n",
    )
    .unwrap();
    std::fs::write(root.join("Cargo.lock"), "version=4\n[[package]]\nname='authored'\nversion='1.0.0'\ndependencies=['lib']\n[[package]]\nname='lib'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\n[[package]]\nname='lib'\nversion='2.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\n").unwrap();
    let ambiguous = successful(invoke("Cargo.lock"));
    assert_eq!(ambiguous["packages"], json!([]));
    assert!(ambiguous["limitations"]
        .as_array()
        .unwrap()
        .contains(&json!("lib:ambiguous_or_missing_lock_entry")));
    std::fs::write(root.join("uv.lock"), "version=1\n[[package]]\nname='authored'\nversion='1.0.0'\nsource={editable='.'}\ndependencies=[{name='lib',version='1.0.0'},{name='lib',version='2.0.0'}]\n[[package]]\nname='lib'\nversion='1.0.0'\nsource={registry='https://pypi.org/simple'}\n[[package]]\nname='lib'\nversion='2.0.0'\nsource={registry='https://pypi.org/simple'}\n").unwrap();
    assert_eq!(
        successful(invoke("uv.lock"))["packages"],
        json!(["pkg:pypi/lib@1.0.0", "pkg:pypi/lib@2.0.0"])
    );
    assert_eq!(
        successful(invoke("unavailable.lock"))["limitations"],
        json!(["lock_unavailable:unavailable.lock"])
    );
    std::fs::write(root.join("unknown.lock"), "").unwrap();
    assert_eq!(
        successful(invoke("unknown.lock"))["limitations"],
        json!(["unsupported_lock_format:unknown.lock"])
    );
    assert!(!invoke("../outside.lock").status.success());
    write(
        &root.join("package-lock.json"),
        &json!({"lockfileVersion":3,"packages":{"node_modules/lib":{"version":"1.0.0"}}}),
    );
    assert_eq!(
        successful(invoke("package-lock.json"))["limitations"],
        json!(["lock_root_unavailable"])
    );
    std::fs::write(root.join("go.mod"),"module authored.test/root\ngo 1.24\nrequire (\n example.org/lib v1.2.3\n example.org/indirect v2.0.0 // indirect\n example.org/replaced v1.0.0\n)\nreplace example.org/replaced => ../local\n").unwrap();
    let go = successful(invoke("go.mod"));
    assert_eq!(go["packages"], json!(["pkg:golang/example.org/lib@v1.2.3"]));
    assert!(go["limitations"]
        .as_array()
        .unwrap()
        .contains(&json!("example.org/replaced:replacement_not_qualified")));
    write(
        &root.join("packages.lock.json"),
        &json!({"version":1,"dependencies":{
            "net8.0":{"Example":{"type":"Direct","resolved":"1.0.0"},"Hidden":{"type":"Transitive","resolved":"2.0.0"}},
            "net9.0":{"Example":{"type":"Direct","resolved":"2.0.0"}}
        }}),
    );
    assert_eq!(
        successful(invoke("packages.lock.json"))["packages"],
        json!(["pkg:nuget/example@1.0.0", "pkg:nuget/example@2.0.0"])
    );
}

#[test]
#[cfg(feature = "producer")]
#[ignore = "requires Python campaign dependencies and Git; runs production revision accounting"]
fn revision_campaign_joins_actual_occurrences_and_preserves_unconsumed_frontiers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    repository(&root, json!({"lockfileVersion":3,"packages":{"":{}}}));
    // Remove the setup source before authoring the independently expected pairs.
    git(&root, &["rm", "source.py", "package-lock.json"]);
    // In-process oxc only (no Cargo.toml for rust-analyzer). Sidecar families
    // stay exclusive-unavailable so host tools are not used under require_native.
    let sources = [
        ("source.js", "function value() { return VALUE; }\nfunction caller() { return value(); }\nfunction another() { return value(); }\n"),
        ("source.ts", "function value(): number { return VALUE; }\n"),
        ("source.tsx", "function value() { return <div>{VALUE}</div>; }\n"),
    ];
    let mut revisions = Vec::new();
    for value in [10, 20] {
        for (name, source) in sources {
            std::fs::write(root.join(name), source.replace("VALUE", &value.to_string())).unwrap();
        }
        git(&root, &["add", "."]);
        git(
            &root,
            &["commit", "--no-gpg-sign", "--quiet", "-m", "revision pair"],
        );
        revisions.push(git(&root, &["rev-parse", "HEAD"]));
    }
    let mut worktrees = serde_json::Map::new();
    for (number, revision) in revisions.iter().enumerate() {
        let path = tmp.path().join(format!("checkout-{number}"));
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                path.to_str().unwrap(),
                revision,
            ],
        );
        // Existing local data is not owned by the campaign, even in its worktrees.
        std::fs::create_dir(path.join(".entmoot")).unwrap();
        std::fs::write(path.join(".entmoot/sentinel"), b"unrelated local data").unwrap();
        worktrees.insert(revision.clone(), json!(path));
    }
    let mut environment: serde_json::Map<String, Value> = std::env::vars()
        .filter(|(key, _)| {
            [
                "PATH",
                "SYSTEMROOT",
                "WINDIR",
                "TEMP",
                "TMP",
                "HOME",
                "USERPROFILE",
            ]
            .contains(&key.to_uppercase().as_str())
        })
        .map(|(key, value)| (key, json!(value)))
        .collect();
    // The runner must select its own volume explicitly, overriding an inherited
    // data root without writing to that root or either analyzed checkout.
    environment.insert("ENTMOOT_DATA_DIR".into(), json!(tmp.path().join("data")));
    for family in ["GO", "JAVA", "ROSLYN", "PYRIGHT", "CLANG"] {
        // Keep sidecar families exclusive-unavailable so host tools are not used.
        environment.insert(
            format!("ENTMOOT_{family}_ADAPTER"),
            json!(tmp.path().join(format!("unavailable-{family}"))),
        );
    }
    let config = tmp.path().join("execution.json");
    write(
        &config,
        &json!({"repository":root,"repository_id":"authored/revision","cli":env!("CARGO_BIN_EXE_entmoot"),
        "sources":worktrees,"materials":[],"environment":environment,"heads":[revisions[1]],"windows":[],
        "relay_request":{"evidence_filter":"any","output":{"limit":1},"callers_output":{"limit":1}},
        "budget":{"producer_timeout_seconds":30,"subject_pages":1,"caller_pages_per_subject":0}}),
    );
    let directory = tmp.path().join("run");
    let invoke = |operation| {
        python(
            "evals.harness.run_revision_campaign",
            &[
                operation,
                "--config",
                config.to_str().unwrap(),
                "--run-dir",
                directory.to_str().unwrap(),
            ],
        )
    };
    let prepared = invoke("prepare");
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let limited = |extra: &[&str]| {
        let mut args = vec![
            "run",
            "--config",
            config.to_str().unwrap(),
            "--run-dir",
            directory.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        successful(python("evals.harness.run_revision_campaign", &args))
    };
    let empty = limited(&["--wall-seconds", "0"]);
    assert_eq!(empty["state"], "paused");
    assert_eq!(empty["reason"], "wall_budget");
    let first = limited(&["--max-operations", "1"]);
    assert_eq!(first["state"], "paused");
    assert_eq!(first["completed_revisions"].as_array().unwrap().len(), 1);
    let second = limited(&["--max-operations", "1"]);
    assert_eq!(second["state"], "paused");
    assert_eq!(second["completed_revisions"].as_array().unwrap().len(), 2);
    std::fs::write(directory.join("stop.request"), "stop").unwrap();
    assert_eq!(limited(&[])["reason"], "stop_requested");
    std::fs::remove_file(directory.join("stop.request")).unwrap();
    let result = successful(invoke("run"));
    assert_eq!(result["process_success"], true);
    assert_eq!(
        result["pairs"][0]["occurrences"]["structural"]["changed"],
        3
    );
    assert_eq!(result["pairs"][0]["callers"]["eligible_subjects"], 3);
    assert_eq!(result["pairs"][0]["callers"]["unconsumed_subjects"], 2);
    assert_eq!(successful(invoke("run")), result);
    assert!(!tmp.path().join("data").exists());
    let owned_data = directory.join("data").canonicalize().unwrap();
    for revision in &revisions {
        let receipt = &result["revisions"][revision];
        for field in ["graph", "store"] {
            assert!(Path::new(receipt[field].as_str().unwrap())
                .canonicalize()
                .unwrap()
                .starts_with(&owned_data));
        }
        let local_data = Path::new(worktrees[revision].as_str().unwrap()).join(".entmoot");
        assert_eq!(std::fs::read_dir(&local_data).unwrap().count(), 1);
        assert_eq!(
            std::fs::read(local_data.join("sentinel")).unwrap(),
            b"unrelated local data"
        );
    }
    let source = Path::new(worktrees[&revisions[0]].as_str().unwrap()).join("source.js");
    std::fs::write(source, "function changed() {}\n").unwrap();
    assert!(!invoke("run").status.success());
}

#[test]
#[ignore = "requires the Python campaign dependencies"]
fn npm_locations_keep_nested_instances_aliases_optional_precedence_and_links_explicit() {
    let query = |lock: Value| {
        Command::new(std::env::var("ENTMOOT_CAMPAIGN_PYTHON").unwrap_or_else(|_| "python".into()))
            .args(["-X","utf8","-c","import json,sys; from evals.harness.npm_lock_inventory import dependency_rows; print(json.dumps(dependency_rows(json.loads(sys.argv[1]))))"])
            .arg(lock.to_string()).current_dir(workspace()).output().unwrap()
    };
    let rows = successful(query(json!({"lockfileVersion":3,"packages":{
        "":{"dependencies":{"a":"^1"}},
        "node_modules/a":{"version":"1.2.0","dependencies":{"b":"^2","@x/c":"3"}},
        "node_modules/b":{"version":"1.0.0"},
        "node_modules/a/node_modules/b":{"version":"2.4.0"},
        "node_modules/@x/c":{"version":"3.0.0"}
    }})));
    let actual: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| json!([r["name"], r["target_path"], r["version"]]))
        .collect();
    assert_eq!(
        actual,
        vec![
            json!(["a", "node_modules/a", "1.2.0"]),
            json!(["@x/c", "node_modules/@x/c", "3.0.0"]),
            json!(["b", "node_modules/a/node_modules/b", "2.4.0"])
        ]
    );
    let rows = successful(query(json!({"lockfileVersion":3,"packages":{"":{
        "dependencies":{"a":"1"},"optionalDependencies":{"a":"2"},
        "peerDependencies":{"b":"^1"},"peerDependenciesMeta":{"b":{"optional":true}}
    }}})));
    let actual: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| json!([r["kind"], r["requested"], r["status"]]))
        .collect();
    assert_eq!(
        actual,
        vec![
            json!(["optional", "2", "missing"]),
            json!(["optional_peer", "^1", "missing"])
        ]
    );
    let rows = successful(query(json!({"lockfileVersion":3,"packages":{
        "":{"dependencies":{"a":"*"}}, "node_modules/a":{"link":true,"resolved":"packages/a"},"packages/a":{"version":"1.0.0"}
    }})));
    assert_eq!(rows[0]["status"], "unsupported_link");
    let rows = successful(query(json!({"lockfileVersion":3,"packages":{
        "":{"dependencies":{"alias":"npm:real@1"}},"node_modules/alias":{"name":"real","version":"1.0.0"}
    }})));
    assert_eq!(rows[0]["name"], "alias");
    assert_eq!(rows[0]["package_name"], "real");
    for lock in [
        json!({"lockfileVersion":1,"dependencies":{}}),
        json!({"lockfileVersion":3,"packages":{"../escape":{}}}),
    ] {
        assert!(!query(lock).status.success());
    }
}

#[test]
#[ignore = "requires the Python campaign dependencies"]
fn native_input_identity_fences_selected_adapter_and_environment_contents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("tools/resolver-adapters")).unwrap();
    std::fs::write(root.join("tools/resolver-adapters/catalog.toml"),"[[family]]\nid='fixture'\nenv='ENTMOOT_FIXTURE_ADAPTER'\nrelative_artifacts=['fixture/adapter.dll']\n").unwrap();
    let adapter = root.join(".tmp/adapter-build/resolver-adapters/fixture/adapter.dll");
    std::fs::create_dir_all(adapter.parent().unwrap()).unwrap();
    std::fs::write(&adapter, b"version one").unwrap();
    let profile = root.join("profile.json");
    std::fs::write(&profile, b"{\"python_version\":\"3.13\"}").unwrap();
    let query = |environment: Value| {
        successful(Command::new(std::env::var("ENTMOOT_CAMPAIGN_PYTHON").unwrap_or_else(|_| "python".into()))
        .args(["-X","utf8","-c","import json,sys; from pathlib import Path; from evals.harness.repository_campaign import native_inputs; print(json.dumps(native_inputs(json.loads(sys.argv[1]),Path(sys.argv[2]))))"])
        .arg(environment.to_string()).arg(root).current_dir(workspace()).output().unwrap())
    };
    let env = json!({"ENTMOOT_PYTHON_ENVIRONMENT":profile});
    let original = query(env.clone());
    assert_ne!(original, query(json!({})));
    std::fs::write(&profile, b"{\"python_version\":\"3.14\"}").unwrap();
    assert_ne!(original, query(env.clone()));
    let before = query(env.clone());
    std::fs::write(&adapter, b"version two").unwrap();
    assert_ne!(before, query(env.clone()));
    let before = query(env.clone());
    std::fs::write(
        adapter.with_file_name("dependency.dll"),
        b"new runtime dependency",
    )
    .unwrap();
    assert_ne!(before, query(env));
    let exclusive = json!({"ENTMOOT_FIXTURE_ADAPTER":root.join("missing")});
    let before = query(exclusive.clone());
    std::fs::write(&adapter, b"unused fallback change").unwrap();
    assert_eq!(before, query(exclusive));
}
