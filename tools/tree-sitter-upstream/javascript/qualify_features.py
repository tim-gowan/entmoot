"""Compare exact JS/TS/TSX CST occurrences with actual Node and compiler behavior."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, endpoint, load_language, nodes, runtime_digest, shape
import tree_sitter

ROOT = Path(__file__).resolve().parent


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--libraries", type=Path, required=True, help="JSON mapping language to named variant/library paths")
    ap.add_argument("--node", type=Path, required=True)
    ap.add_argument("--typescript6", type=Path, required=True, help="Installed typescript package root")
    ap.add_argument("--typescript7", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    node = args.node.resolve(strict=True)
    env = {k: v for k, v in os.environ.items() if k not in ("NODE_OPTIONS", "NODE_PATH", "TS_NODE_PROJECT", "TS_NODE_COMPILER_OPTIONS")}
    node_version = subprocess.check_output([node, "--version"], env=env, text=True).strip()
    if node_version != "v26.11.1":
        raise ValueError("Node 26.11.1 required")
    tools = [{"name": "node", "version": node_version, "digest": digest(node.read_bytes())}]
    compilers = {}
    for version, package in (("6.0.3", args.typescript6), ("7.0.2", args.typescript7)):
        package = package.resolve(strict=True)
        if json.loads((package / "package.json").read_text(encoding="utf-8"))["version"] != version:
            raise ValueError("wrong compiler package")
        command = [str(node), str(package / "bin/tsc")]
        actual = subprocess.check_output([*command, "--version"], env=env, text=True).strip()
        if actual != "Version " + version:
            raise ValueError("compiler executable disagrees with metadata")
        compilers[version] = command
        paths = sorted(package.rglob("*.js")) + sorted(package.rglob("*.d.ts")) + [package / "package.json"]
        if version.startswith("7"):
            paths += sorted((package.parent / "@typescript").rglob("*.exe"))
        tools.append({"name": "typescript", "version": version, "inputs": [{"path": str(p), "digest": digest(p.read_bytes())} for p in paths]})
    manifest = json.loads(args.libraries.read_text(encoding="utf-8"))
    parsers, libraries, handles = {}, [], []
    for language, variants in manifest.items():
        parsers[language] = {}
        for variant, path in variants.items():
            path = Path(path).resolve(strict=True)
            handle, grammar = load_language(path, language)
            handles.append(handle)
            parsers[language][variant] = tree_sitter.Parser(grammar)
            libraries.append({"language": language, "variant": variant, "path": str(path), "digest": digest(path.read_bytes())})
    fixture_path = ROOT / "cases.py"
    spec = importlib.util.spec_from_file_location("ecmascript_cases", fixture_path)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    rows, identities = [], set()

    def run(command, directory, name):
        result = subprocess.run(command, cwd=directory, env=env, capture_output=True, timeout=60)
        raw = result.stdout + result.stderr
        (directory / (name + ".log")).write_bytes(raw)
        return {"command": list(map(str, command)), "exit_code": result.returncode,
                "output": raw.decode("utf-8", "replace"), "log_digest": digest(raw)}

    for case in fixture.cases():
        if case["id"] in identities or not re.fullmatch("[a-z0-9-]+", case["id"]):
            raise ValueError("invalid or duplicate case id")
        identities.add(case["id"])
        raw = case["source"].encode("utf-8")
        for language in case["languages"]:
            directory = out / case["id"] / language
            directory.mkdir(parents=True)
            extension = case.get("extension", {"javascript": ".mjs", "typescript": ".mts", "tsx": ".tsx"}[language])
            if extension not in (".mjs", ".mts", ".ts", ".tsx"):
                raise ValueError("unqualified source extension")
            source_path = directory / ("fixture" + extension)
            source_path.write_bytes(raw)
            (directory / "package.json").write_text('{"type":"module"}\n', encoding="utf-8")
            for path, value in case["files"].items():
                target = (directory / path).resolve()
                if not target.is_relative_to(directory) or path in (source_path.name, "package.json", "tsconfig.json"):
                    raise ValueError("invalid authored support file")
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(bytes.fromhex(value["hex"]) if isinstance(value, dict) else value.encode("utf-8"))
            csts = []
            for variant, parser in parsers[language].items():
                tree = parser.parse(raw)
                after = [n for n in tree.root_node.named_children if n.type == "function_declaration"
                         and n.child_by_field_name("name") is not None and n.child_by_field_name("name").text == b"after"]
                sibling = len(after) == 1 and after[0].start_byte == raw.index(fixture.AFTER.encode())
                valid = not tree.root_node.has_error
                sites, ok = [], valid == case["parse_accepts"] and sibling
                for expected in case["sites"]:
                    text = expected["text"].encode("utf-8")
                    # Explicit selectors choose the authored source occurrence, never the first callable.
                    offsets = [match.start() for match in re.finditer(re.escape(text), raw)]
                    ordinal = expected.get("occurrence", 0)
                    if ordinal >= len(offsets):
                        raise ValueError("authored site is absent")
                    if len(offsets) > 1 and "occurrence" not in expected:
                        raise ValueError("site needs an explicit occurrence ordinal: " + case["id"] + ":" + expected["text"])
                    begin = offsets[ordinal]
                    matches = [n for n in nodes(tree.root_node) if n.start_byte == begin and n.end_byte == begin + len(text) and n.type == expected["kind"]]
                    fields = {}
                    site_ok = not matches if expected.get("absent") else len(matches) == 1
                    if site_ok and not expected.get("absent"):
                        site_ok = tuple(matches[0].start_point) == endpoint(raw[:begin]) and tuple(matches[0].end_point) == endpoint(raw[:begin + len(text)])
                        for field, value in expected["fields"].items():
                            child = matches[0].child_by_field_name(field)
                            fields[field] = child.text.decode("utf-8") if child else None
                            site_ok = site_ok and fields[field] == value
                    sites.append({"expected": expected, "passed": site_ok, "fields": fields})
                    ok = ok and site_ok
                recovery = None
                if valid:
                    begin, insertion = raw.index(fixture.AFTER.encode()), b"const broken = ;\n"
                    dirty = raw[:begin] + insertion + raw[begin:]
                    tree.edit(begin, begin, begin + len(insertion), endpoint(raw[:begin]), endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]))
                    edited = parser.parse(dirty, tree)
                    malformed = edited.root_node.has_error
                    edited.edit(begin, begin + len(insertion), begin, endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]), endpoint(raw[:begin]))
                    recovery = malformed and shape(parser.parse(raw, edited)) == shape(parser.parse(raw))
                csts.append({"variant": variant, "has_error": not valid, "sibling_preserved": sibling, "sites": sites,
                             "expected_cst_met": ok, "incremental_recovery": recovery,
                             "shape_digest": digest(json.dumps(shape(parser.parse(raw)), separators=(",", ":")).encode())})
            qualifications = []
            if language == "javascript":
                check = run([str(node), "--check", str(source_path)], directory, "node-check")
                accepted = check["exit_code"] == 0
                check.update(accepted=accepted, passed=accepted == case["node_accepts"])
                execution = run([str(node), str(source_path)], directory, "node-runtime") if accepted and case["execute"] else None
                if execution:
                    execution["passed"] = execution["exit_code"] == 0 and execution["output"].strip() == "oracle:ok"
                qualifications.append({"tool": "node", "target": check, "runtime": execution})
            else:
                for version, command in compilers.items():
                    emitted = directory / ("emitted-" + version)
                    options = {"target": "esnext", "module": "esnext", "moduleResolution": "bundler", "types": [],
                               "strict": False, "skipLibCheck": False, "noEmitOnError": True, "resolveJsonModule": True,
                               "moduleDetection": "force", "outDir": str(emitted), "rootDir": str(directory),
                               "lib": ["esnext", "dom"], **case["options"]}
                    config = {"compilerOptions": options, "files": [source_path.name]}
                    (directory / "tsconfig.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
                    check = run([*command, "--project", str(directory / "tsconfig.json"), "--pretty", "false"], directory, "compile-" + version)
                    accepted = check["exit_code"] == 0
                    check.update(accepted=accepted, passed=accepted == case["compiler_accepts"], options=options)
                    execution = None
                    if accepted and case["execute"]:
                        for path, value in case["files"].items():
                            if path.endswith((".mjs", ".json", ".wasm")):
                                target = emitted / path
                                target.parent.mkdir(parents=True, exist_ok=True)
                                target.write_bytes(bytes.fromhex(value["hex"]) if isinstance(value, dict) else value.encode("utf-8"))
                        output_file = emitted / ("fixture.mjs" if extension == ".mts" else "fixture.js")
                        execution = run([str(node), str(output_file)], directory, "runtime-" + version)
                        execution.update(passed=execution["exit_code"] == 0 and execution["output"].strip() == "oracle:ok", emitted_digest=digest(output_file.read_bytes()))
                    qualifications.append({"tool": "typescript-" + version, "target": check, "runtime": execution})
            row = {"case": case["id"], "language": language, "source_digest": digest(raw), "cst": csts, "qualifications": qualifications,
                   "native_resolver": "not_run", "expected": case}
            rows.append(row)
            print(json.dumps({"case": case["id"], "language": language, "cst": {c["variant"]: c["expected_cst_met"] for c in csts},
                              "oracles": all(q["target"]["passed"] and (q["runtime"] is None or q["runtime"]["passed"]) for q in qualifications)}), flush=True)
    report = {"format": "entmoot.ecmascript_feature_qualification.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "fixture_digest": digest(fixture_path.read_bytes()), "runtime_driver_digest": runtime_digest(), "tools": tools,
              "libraries": libraries, "cases": rows,
              "oracles_passed": all(q["target"]["passed"] and (q["runtime"] is None or q["runtime"]["passed"]) for r in rows for q in r["qualifications"]),
              "all_cst_expectations_met": all(c["expected_cst_met"] and c["incremental_recovery"] is not False for r in rows for c in r["cst"]),
              "limits": ["Exact authored sources; no foreign code execution, ambient types or installed-package resolution",
                         "Current compiler target modes and actual runtimes are separate from historical compiler-version controls",
                         "TypeScript 7 CLI does not qualify the Program adapter; 6.0.3 remains its independent API pin",
                         "No native producer or structural binding inference from a builtin spelling or successful parse",
                         "RegExp sublanguage legality belongs to the runtime, not the outer grammar",
                         "Deferred imports are compiler checked without Node execution; actual unsupported runtime behavior remains explicit"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if report["oracles_passed"] and report["all_cst_expectations_met"] else 1)


if __name__ == "__main__":
    main()
