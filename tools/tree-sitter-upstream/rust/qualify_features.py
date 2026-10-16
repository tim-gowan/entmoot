"""Keep Rust CST, edition/compiler acceptance and actual execution independent."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, endpoint, load_language, nodes, runtime_digest, shape
import tree_sitter

ROOT = Path(__file__).resolve().parent


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--toolchains", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    if sys.platform != "linux":
        raise ValueError("this oracle lane qualifies Linux compiler/runtime behavior")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    fixture_path = ROOT / "cases.py"
    spec = importlib.util.spec_from_file_location("rust_cases", fixture_path)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    env = {k: v for k, v in os.environ.items() if k not in ("RUSTC_BOOTSTRAP", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    compilers, tools = {}, []
    for version in ("1.95.0", "1.99.0"):
        root = (args.toolchains / (version + "-x86_64-unknown-linux-gnu")).resolve(strict=True)
        compiler = root / "bin/rustc"
        details = subprocess.check_output([compiler, "-Vv"], env=env, text=True).strip()
        if not details.startswith("rustc " + version + " "):
            raise ValueError("unexpected compiler version")
        compilers[version] = compiler
        inputs = sorted([compiler, *root.glob("lib/librustc_driver*.so"), *root.glob("lib/rustlib/x86_64-unknown-linux-gnu/lib/*.rlib")])
        tools.append({"version": version, "details": details, "inputs": [{"path": p.relative_to(root).as_posix(), "digest": digest(p.read_bytes())} for p in inputs]})
    library = args.grammar.resolve(strict=True)
    handle, language = load_language(library, "rust")
    parser = tree_sitter.Parser(language)
    rows, identities = [], set()
    for case in fixture.cases():
        if case["id"] in identities or not case["id"].replace("-", "").isalnum():
            raise ValueError("invalid or duplicate case identity")
        identities.add(case["id"])
        directory = out / case["id"]
        directory.mkdir()
        source, binary = directory / "fixture.rs", directory / "fixture"
        raw = case["source"].encode("utf-8")
        source.write_bytes(raw)
        command = [str(compilers[case["compiler"]]), "--edition=" + case["edition"], "--error-format=json", "--crate-name=fixture",
                   "--crate-type=bin", str(source), "-o", str(binary)]
        start = time.perf_counter()
        compiled = subprocess.run(command, cwd=directory, env=env, capture_output=True, timeout=60)
        elapsed = time.perf_counter() - start
        diagnostic_raw = compiled.stdout + compiled.stderr
        (directory / "compile.jsonl").write_bytes(diagnostic_raw)
        diagnostics = []
        for line in diagnostic_raw.decode("utf-8").splitlines():
            event = json.loads(line)
            if event.get("$message_type") == "diagnostic":
                diagnostics.append({"level": event["level"], "code": event["code"]["code"] if event.get("code") else None,
                                    "message": event["message"]})
        errors = [d for d in diagnostics if d["level"] == "error"]
        accepted = compiled.returncode == 0
        target_ok = accepted == case["compiler_accepts"] and all(c in {d["code"] for d in errors} for c in case["diagnostic_codes"])
        error_text = "\n".join(d["message"] for d in errors)
        target_ok = target_ok and all(text in error_text for text in case["diagnostic_text"])
        execution = None
        if accepted:
            run = subprocess.run([binary], cwd=directory, env=env, capture_output=True, timeout=30)
            output = run.stdout + run.stderr
            (directory / "execute.log").write_bytes(output)
            execution = {"exit_code": run.returncode, "output": output.decode("utf-8", "replace"),
                         "binary_digest": digest(binary.read_bytes()), "passed": run.returncode == 0 and output.strip() == b"oracle:ok"}
        tree = parser.parse(raw)
        valid = not tree.root_node.has_error
        following = [n for n in tree.root_node.named_children if n.type == "function_item"
                     and n.child_by_field_name("name") is not None and n.child_by_field_name("name").text == b"after"]
        sibling = len(following) == 1 and following[0].start_byte == raw.index(b"fn after")
        ok, sites = valid == case["parse_accepts"] and sibling, []
        for expected in case["sites"]:
            text = expected["text"].encode("utf-8")
            if raw.count(text) != 1:
                raise ValueError("site must identify one exact source occurrence")
            begin = raw.index(text)
            matches = [n for n in nodes(tree.root_node) if n.start_byte == begin and n.end_byte == begin + len(text) and n.type == expected["kind"]]
            site_ok, fields = len(matches) == 1, {}
            if site_ok:
                site_ok = tuple(matches[0].start_point) == endpoint(raw[:begin]) and tuple(matches[0].end_point) == endpoint(raw[:begin + len(text)])
                for field, value in expected["fields"].items():
                    node = matches[0].child_by_field_name(field)
                    fields[field] = node.text.decode("utf-8") if node else None
                    site_ok = site_ok and fields[field] == value
            sites.append({"expected": expected, "passed": site_ok, "observed_fields": fields})
            ok = ok and site_ok
        recovery = None
        if valid:
            begin = raw.index(b"fn after")
            insertion = b"fn broken(\n"
            dirty = raw[:begin] + insertion + raw[begin:]
            tree.edit(begin, begin, begin + len(insertion), endpoint(raw[:begin]), endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]))
            edited = parser.parse(dirty, tree)
            malformed = edited.root_node.has_error
            edited.edit(begin, begin + len(insertion), begin, endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]), endpoint(raw[:begin]))
            recovery = malformed and shape(parser.parse(raw, edited)) == shape(parser.parse(raw))
        rows.append({"expectation": case, "source_digest": digest(raw),
                     "cst": {"has_error": not valid, "expected_cst_met": ok, "sibling_preserved": sibling, "sites": sites,
                             "incremental_recovery": recovery, "digest": digest(json.dumps(shape(parser.parse(raw)), separators=(",", ":")).encode())},
                     "compiler": {"command": command, "accepted": accepted, "passed": target_ok, "exit_code": compiled.returncode,
                                  "wall_seconds": elapsed, "diagnostics": diagnostics, "raw_diagnostic_digest": digest(diagnostic_raw)},
                     "runtime": execution, "native_resolver": "not_run"})
        print(json.dumps({"case": case["id"], "compiler": target_ok, "runtime": execution["passed"] if execution else None,
                          "cst": ok, "incremental_recovery": recovery}), flush=True)
    passed = all(row["compiler"]["passed"] and (row["runtime"] is None or row["runtime"]["passed"])
                 and row["cst"]["expected_cst_met"] and row["cst"]["incremental_recovery"] is not False for row in rows)
    report = {"format": "entmoot.rust_feature_qualification.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "fixture_digest": digest(fixture_path.read_bytes()),
              "library": {"path": str(library), "digest": digest(library.read_bytes())}, "tools": tools, "cases": rows, "passed": passed,
              "limits": ["Actual pinned Linux 1.95/1.99 compilers; edition modes are distinct from a historical compiler-version matrix",
                         "No RUSTC_BOOTSTRAP, feature gates, Cargo dependency resolution or foreign code execution",
                         "Runtime assertions cover the executed fixture behavior; dropped async futures are only compiler-qualified",
                         "Grammar acceptance cannot establish legal ABI, argument promotion, scope binding or macro expansion",
                         "No Entmoot native resolver, production visitor revision-pair migration or broad performance claim"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
