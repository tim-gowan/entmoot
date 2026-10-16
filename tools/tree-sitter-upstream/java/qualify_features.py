"""Keep Java CST, target mode, actual execution and native resolution distinct."""
from __future__ import annotations

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
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--jdk", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    fixture_path = ROOT / "modern-syntax/cases.py"
    spec = importlib.util.spec_from_file_location("java_cases", fixture_path)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    jdk = args.jdk.resolve(strict=True)
    suffix = ".exe" if os.name == "nt" else ""
    compiler, runtime = (jdk / "bin" / (name + suffix) for name in ("javac", "java"))
    env = {k: v for k, v in os.environ.items() if k not in ("JDK_JAVA_OPTIONS", "JDK_JAVAC_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")}
    env.update(JAVA_HOME=str(jdk))
    version = subprocess.check_output([compiler, "-version"], env=env, text=True).strip()
    if version != "javac 27":
        raise ValueError("pinned JDK 27 required")
    libraries = [args.baseline.resolve(strict=True), args.candidate.resolve(strict=True)]
    handles = [load_language(path, name) for path, name in zip(libraries, ("java", "java_orchard"))]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    rows = []
    identifiers = set()
    for case in fixture.cases():
        if case["id"] in identifiers or not case["id"].replace("-", "").isalnum():
            raise ValueError("invalid/duplicate fixture id")
        identifiers.add(case["id"])
        directory = out / case["id"]
        directory.mkdir()
        classes, empty = directory / "classes", directory / "empty"
        classes.mkdir()
        empty.mkdir()
        raw = case["source"].encode("utf-8")
        source = directory / "Fixture.java"
        source.write_bytes(raw)
        command = [str(compiler), "-XDrawDiagnostics", "-J-Duser.language=en", "-J-Duser.country=US",
                   "-encoding", "UTF-8", "-proc:none", "--release", str(case["release"]),
                   "--class-path", str(empty), "--source-path", str(empty),
                   "-d", str(classes)]
        # The module system does not exist in Java 8. javac rejects --module-path
        # with that target even when it names an empty directory.
        if case["release"] >= 9:
            command.extend(["--module-path", str(empty)])
        if case["preview"]:
            command.append("--enable-preview")
        command.append(str(source))
        start = time.perf_counter()
        compiled = subprocess.run(command, cwd=directory, env=env, capture_output=True, timeout=60)
        elapsed = time.perf_counter() - start
        diagnostic = (compiled.stdout + compiled.stderr).decode("utf-8", "replace")
        (directory / "compile.log").write_text(diagnostic, encoding="utf-8")
        accepted = compiled.returncode == 0
        target_ok = accepted == case["compiler_accepts"] and all(code in diagnostic for code in case["diagnostic_codes"])
        execution = None
        if accepted:
            run_command = [str(runtime)] + (["--enable-preview"] if case["preview"] else [])
            run_command.extend(["--class-path", str(classes), "Fixture"])
            run = subprocess.run(run_command, cwd=directory, env=env, capture_output=True, timeout=30)
            output = (run.stdout + run.stderr).decode("utf-8", "replace")
            execution = {"command": run_command, "exit_code": run.returncode, "output": output,
                         "passed": run.returncode == 0 and run.stdout.strip() == b"oracle:ok"}
        observations = []
        for parser in parsers:
            tree = parser.parse(raw)
            valid = not tree.root_node.has_error
            after = [n for n in tree.root_node.named_children if n.type == "class_declaration"
                     and n.child_by_field_name("name").text == b"After"]
            sibling_ok = len(after) == 1 and after[0].start_byte == raw.index(b"class After")
            ok = valid == case["parse_accepts"] and sibling_ok
            sites = []
            for expected in case["sites"]:
                text = expected["text"].encode("utf-8")
                if raw.count(text) != 1:
                    raise ValueError("selector must identify exactly one span: " + expected["text"])
                begin = raw.index(text)
                matches = [n for n in nodes(tree.root_node) if n.start_byte == begin and n.end_byte == begin + len(text)
                           and n.type == expected["kind"]]
                site_ok = len(matches) == 1
                fields = {}
                named_children = None
                if site_ok:
                    site_ok = tuple(matches[0].start_point) == endpoint(raw[:begin])
                    named_children = [n.type for n in matches[0].named_children]
                    if "named_children" in expected:
                        site_ok = site_ok and named_children == expected["named_children"]
                    for field, value in expected["fields"].items():
                        selected = matches[0].child_by_field_name(field)
                        fields[field] = selected.text.decode("utf-8") if selected is not None else None
                        site_ok = site_ok and fields[field] == value
                sites.append({"expected": expected, "passed": site_ok, "observed_fields": fields,
                              "named_children": named_children})
                ok = ok and site_ok
            recovery = None
            if valid:
                # Insert an incomplete declaration immediately before the sibling,
                # then remove it; compare every node/field/span to a fresh parse.
                begin = raw.index(b"class After")
                insertion = b"class Broken { void missing(\n"
                dirty = raw[:begin] + insertion + raw[begin:]
                tree.edit(begin, begin, begin + len(insertion), endpoint(raw[:begin]), endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]))
                edited = parser.parse(dirty, tree)
                malformed = edited.root_node.has_error
                edited.edit(begin, begin + len(insertion), begin, endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]), endpoint(raw[:begin]))
                recovery = malformed and shape(parser.parse(raw, edited)) == shape(parser.parse(raw))
            observations.append({"has_error": not valid, "expected_cst_met": ok, "sibling_preserved": sibling_ok,
                                 "sites": sites, "incremental_recovery": recovery,
                                 "cst_digest": digest(json.dumps(shape(parser.parse(raw)), separators=(",", ":")).encode())})
        rows.append({"expectation": case, "source_digest": digest(raw), "baseline": observations[0], "candidate": observations[1],
                     "target_mode": {"accepted": accepted, "passed": target_ok, "command": command,
                                     "exit_code": compiled.returncode, "wall_seconds": elapsed, "diagnostic": diagnostic},
                     "runtime": execution, "native_resolver": "not_run"})
        print(json.dumps({"case": case["id"], "compiler": target_ok, "runtime": execution["passed"] if execution else None,
                          "baseline": observations[0]["expected_cst_met"], "candidate": observations[1]["expected_cst_met"]}), flush=True)
    passed = all(r["target_mode"]["passed"] and (r["runtime"] is None or r["runtime"]["passed"])
                 and r["candidate"]["expected_cst_met"] and r["candidate"]["incremental_recovery"] is not False for r in rows)
    report = {"format": "entmoot.java_feature_qualification.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "fixture_digest": digest(fixture_path.read_bytes()),
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries],
              "jdk": {"version": version, "compiler_digest": digest(compiler.read_bytes()), "runtime_digest": digest(runtime.read_bytes()),
                      "modules_digest": digest((jdk / "lib/modules").read_bytes())},
              "cases": rows, "passed": passed,
              "limits": ["Historical --release mode is distinct from executing an actual historical JDK",
                         "Preview controls never imply stable-language qualification",
                         "Compiler rejection may coexist with a valid CST; grammars are not semantic type checkers",
                         "No native Entmoot resolver or production visitor qualification is claimed"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
