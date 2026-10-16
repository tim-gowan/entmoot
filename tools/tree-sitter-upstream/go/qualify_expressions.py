"""Independent Go expression fixtures: CST, language mode, and actual execution."""
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
FIXTURES = {"new-expressions": ROOT / "new-expressions/cases.py",
            "method-values": ROOT / "generic-methods/method_value_cases.py",
            "declarations": ROOT / "generic-methods/declaration_cases.py"}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--feature", choices=tuple(FIXTURES), required=True)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--go", type=Path, required=True)
    ap.add_argument("--cache", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    fixture_path = FIXTURES[args.feature]
    spec = importlib.util.spec_from_file_location("feature_cases", fixture_path)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    compiler = args.go.resolve(strict=True)
    env = {**os.environ, "GOTOOLCHAIN": "local", "GOPROXY": "off", "GOSUMDB": "off", "GOENV": "off",
           "GOROOT": str(compiler.parent.parent), "GOFLAGS": "", "GOEXPERIMENT": "", "CGO_ENABLED": "0",
           "GOCACHE": str(args.cache.resolve())}
    host = subprocess.check_output([compiler, "env", "GOHOSTOS", "GOHOSTARCH"], env=env, text=True).split()
    env.update(GOOS=host[0], GOARCH=host[1])
    version = subprocess.check_output([compiler, "version"], env=env, text=True).strip()
    if not version.startswith("go version go1.27.2 "):
        raise ValueError("Go 1.27.2 required")
    libraries = [p.resolve(strict=True) for p in (args.baseline, args.candidate)]
    handles = [load_language(p, "go") for p in libraries]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    rows = []
    identifiers = set()
    for case in fixture.cases():
        if case["id"] in identifiers or not case["id"].replace("-", "").isalnum():
            raise ValueError("invalid or duplicate fixture identity")
        identifiers.add(case["id"])
        raw = case["source"].encode("utf-8")
        file = out / (case["id"] + ".go")
        file.write_bytes(raw)
        command = [str(compiler), "run", "-gcflags=-lang=" + case["mode"], str(file)]
        start = time.perf_counter()
        result = subprocess.run(command, cwd=out, env=env, capture_output=True, timeout=120)
        elapsed = time.perf_counter() - start
        diagnostic = (result.stdout + result.stderr).decode("utf-8", "replace")
        accepted = result.returncode == 0
        compiler_ok = accepted == case["compiler_accepts"]
        if case["compiler_accepts"]:
            compiler_ok = compiler_ok and "oracle:ok" in diagnostic
        else:
            # A tool failure or unrelated diagnostic must not satisfy a negative.
            compiler_ok = compiler_ok and all(s.lower() in diagnostic.lower() for s in case["diagnostic_contains"])
        (out / (case["id"] + ".log")).write_text(diagnostic, encoding="utf-8")
        observations = []
        for parser in parsers:
            tree = parser.parse(raw)
            valid = not tree.root_node.has_error
            ok = valid == case["parse_accepts"]
            site_results = []
            recovery = None
            if valid:
                declarations = [n for n in tree.root_node.named_children if n.type in ("function_declaration", "method_declaration")]
                names = [n.child_by_field_name("name").text.decode() for n in declarations]
                ok = ok and names == case["declarations"]
                after = [n for n in declarations if n.child_by_field_name("name").text == b"After"]
                ok = ok and len(after) == 1 and after[0].start_byte == raw.index(b"func After")
                for expected in case.get("sites", []):
                    text = expected["text"].encode()
                    if raw.count(text) != 1:
                        raise ValueError("fixture selector must identify exactly one byte span: " + expected["text"])
                    begin = raw.index(text)
                    matches = [n for n in nodes(tree.root_node) if n.start_byte == begin and n.end_byte == begin + len(text)
                               and n.type == expected["kind"]]
                    site_ok = len(matches) == 1
                    observed = {}
                    if site_ok:
                        for field, field_text in expected.get("fields", {}).items():
                            selected = matches[0].child_by_field_name(field)
                            observed[field] = selected.text.decode() if selected is not None else None
                            site_ok = site_ok and observed[field] == field_text
                    site_results.append({"expected": expected, "passed": site_ok, "observed_fields": observed})
                    ok = ok and site_ok
                dirty = raw + b"func Broken(\n"
                tree.edit(len(raw), len(raw), len(dirty), endpoint(raw), endpoint(raw), endpoint(dirty))
                edited = parser.parse(dirty, tree)
                malformed = edited.root_node.has_error
                edited.edit(len(raw), len(dirty), len(raw), endpoint(raw), endpoint(dirty), endpoint(raw))
                recovery = malformed and shape(parser.parse(raw, edited)) == shape(parser.parse(raw))
            observations.append({"has_error": not valid, "expected_cst_met": ok,
                                 "sites": site_results, "incremental_recovery": recovery})
        row = {"expectation": case, "source_digest": digest(raw), "baseline": observations[0], "candidate": observations[1],
               "compiler": {"accepted": accepted, "passed": compiler_ok, "command": command,
                            "exit_code": result.returncode, "wall_seconds": elapsed, "diagnostic": diagnostic},
               "native_resolver": "not_run"}
        rows.append(row)
        print(json.dumps({"case": case["id"], "compiler": compiler_ok, "baseline": observations[0]["expected_cst_met"],
                          "candidate": observations[1]["expected_cst_met"]}), flush=True)
    passed = all(r["compiler"]["passed"] and r["candidate"]["expected_cst_met"]
                 and r["candidate"]["incremental_recovery"] is not False for r in rows)
    binary = compiler.parent.parent / "pkg/tool" / (host[0] + "_" + host[1]) / ("compile.exe" if os.name == "nt" else "compile")
    report = {"format": "entmoot.go_expression_qualification.v1", "feature": args.feature,
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "fixture_digest": digest(fixture_path.read_bytes()), "fixture_path": str(fixture_path),
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries],
              "compiler": {"version": version, "tool_digest": digest(compiler.read_bytes()), "compiler_digest": digest(binary.read_bytes())},
              "cases": rows, "passed": passed,
              "limits": ["Fixture expectations are independently authored; compiler failures require the intended diagnostic",
                         "Historical source mode is not an actual historical compiler installation",
                         "CST field spelling does not establish semantic type, builtin identity or native resolution",
                         "Feature controls are separate from full corpus, platforms and example qualification"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
