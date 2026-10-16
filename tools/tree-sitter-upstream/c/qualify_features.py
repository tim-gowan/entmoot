"""Separate C/C++ CST, explicit compiler mode and actual runtime observations."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, endpoint, load_language, nodes, runtime_digest, shape
import tree_sitter

ROOT = Path(__file__).resolve().parent.parent


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--language", choices=("c", "cpp"), required=True)
    ap.add_argument("--libraries", type=Path, required=True, help="Variant to platform library path mapping")
    ap.add_argument("--candidate", default="composed", help="Variant whose qualification controls the exit status")
    ap.add_argument("--selection", type=Path, help="A change directory's authored fixture selection")
    ap.add_argument("--clang-root", type=Path, required=True, help="Prepared Linux release with bin/ and lib/")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    fixture_path = ROOT / args.language / "cases.py"
    spec = importlib.util.spec_from_file_location("authored_c_family_cases", fixture_path)
    fixtures = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixtures)
    cases = fixtures.CASES
    selection = None
    if args.selection:
        selection_path = args.selection.resolve(strict=True)
        selected_raw = selection_path.read_bytes()
        selected = json.loads(selected_raw)
        if (not selection_path.is_relative_to(ROOT / args.language)
                or selected["format"] != "entmoot.grammar_fixture_selection.v1"
                or selected["language"] != args.language
                or (selection_path.parent / selected["fixture_owner"]).resolve() != fixture_path):
            raise ValueError("fixture selection must identify this language's authoritative fixture owner")
        by_id = {case["id"]: case for case in cases}
        ids = selected["case_ids"]
        if not ids or len(ids) != len(set(ids)) or any(key not in by_id for key in ids):
            raise ValueError("fixture selection must name distinct existing cases")
        cases = [by_id[key] for key in ids]
        selection = {"path": str(selection_path), "digest": digest(selected_raw), "case_ids": ids}
    handles, parsers, libraries = [], {}, []
    for variant, name in json.loads(args.libraries.read_text(encoding="utf-8")).items():
        path = Path(name).resolve(strict=True)
        handle, grammar = load_language(path, args.language)
        handles.append(handle)
        parsers[variant] = tree_sitter.Parser(grammar)
        libraries.append({"variant": variant, "path": str(path), "digest": digest(path.read_bytes())})
    if args.candidate not in parsers:
        raise ValueError("candidate must identify one supplied library variant")
    clang = args.clang_root.resolve(strict=True)

    def native_path(path):
        path = Path(path).absolute().as_posix()
        if os.name == "nt":
            if not path.startswith("G:/"):
                raise ValueError("WSL inputs must be on the recorded G drive")
            return "/mnt/g/" + path[3:]
        return path

    compiler = native_path(clang / "bin" / ("clang++" if args.language == "cpp" else "clang"))
    env = {key: value for key, value in os.environ.items() if key not in
           ("CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "OBJC_INCLUDE_PATH", "LIBRARY_PATH")}
    env["LD_LIBRARY_PATH"] = native_path(clang / "lib")
    prefix = (["wsl", "-d", "Ubuntu", "--", "env", "-u", "CPATH", "-u", "C_INCLUDE_PATH", "-u", "CPLUS_INCLUDE_PATH",
               "-u", "OBJC_INCLUDE_PATH", "-u", "LIBRARY_PATH", "LD_LIBRARY_PATH=" + env["LD_LIBRARY_PATH"]] if os.name == "nt" else [])
    version = subprocess.check_output([*prefix, compiler, "--version"], env=env, text=True)
    if not version.startswith("clang version 23.1.3 "):
        raise ValueError("pinned Clang 23.1.3 required")
    manifest = {"format": "entmoot.c_family_feature_inputs.v1", "language": args.language,
                "driver_digest": digest(Path(__file__).read_bytes()), "fixture_digest": digest(fixture_path.read_bytes()),
                "runtime_driver_digest": runtime_digest(), "libraries": libraries, "compiler_version": version,
                "candidate": args.candidate, "selection": selection,
                "compiler_root": str(clang), "compiler_digest": digest((clang / "bin/clang-23").read_bytes()),
                "cases": [{"id": case["id"], "mode": case["mode"], "source_digest": digest(case["source"].encode("utf-8"))}
                          for case in cases]}
    manifest_path = out / "input_manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")
    rows = []

    def run(command, directory, name):
        result = subprocess.run([*prefix, *command], env=env, capture_output=True, timeout=90)
        raw = result.stdout + result.stderr
        (directory / (name + ".log")).write_bytes(raw)
        return {"command": [*prefix, *command], "exit_code": result.returncode,
                "output": raw.decode("utf-8", "replace"), "log_digest": digest(raw)}

    for case in cases:
        directory = out / case["id"]
        directory.mkdir()
        raw = case["source"].encode("utf-8")
        source = directory / ("fixture.cpp" if args.language == "cpp" else "fixture.c")
        source.write_bytes(raw)
        support = []
        for name, contents in case.get("files", {}).items():
            target = (directory / name).resolve()
            if not target.is_relative_to(directory):
                raise ValueError("support file escapes fixture")
            data = bytes.fromhex(contents["hex"]) if isinstance(contents, dict) else contents.encode("utf-8")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            support.append({"path": name, "digest": digest(data)})
        csts = []
        for variant, parser in parsers.items():
            tree = parser.parse(raw)
            initial_error = tree.root_node.has_error
            sites = []
            for expected in case["sites"]:
                matches = [node for node in nodes(tree.root_node) if node.type == expected["kind"] and
                           raw[node.start_byte:node.end_byte].decode("utf-8") == expected["text"]]
                ordinal = expected.get("ordinal", 0)
                unique = len(matches) == 1 if "ordinal" not in expected else ordinal < len(matches)
                found = matches[ordinal] if unique else None
                fields = {field: (raw[child.start_byte:child.end_byte].decode("utf-8") if child else None)
                          for field in expected.get("fields", {})
                          for child in [found.child_by_field_name(field) if found else None]}
                sites.append({"expected": expected, "match_count": len(matches), "fields": fields,
                              "span": ({"bytes": [found.start_byte, found.end_byte],
                                        "points": [list(found.start_point), list(found.end_point)]} if found else None),
                              "passed": unique and fields == expected.get("fields", {})})
            sibling = any(node.type == "function_definition" and raw[node.start_byte:node.end_byte].decode("utf-8") == case["after"]
                          for node in tree.root_node.named_children)
            recovered = None
            if not initial_error:
                expected_shape = shape(tree)
                inserted = b"int broken = ;\n"
                tree.edit(start_byte=0, old_end_byte=0, new_end_byte=len(inserted), start_point=(0, 0), old_end_point=(0, 0), new_end_point=endpoint(inserted))
                broken = parser.parse(inserted + raw, tree)
                had_error = broken.root_node.has_error
                broken.edit(start_byte=0, old_end_byte=len(inserted), new_end_byte=0, start_point=(0, 0), old_end_point=endpoint(inserted), new_end_point=(0, 0))
                repaired = parser.parse(raw, broken)
                recovered = had_error and not repaired.root_node.has_error and shape(repaired) == expected_shape
            csts.append({"variant": variant, "has_error": initial_error, "sibling_preserved": sibling,
                         "sites": sites, "expected_cst_met": (not initial_error) == case["parse"] and sibling and all(s["passed"] for s in sites),
                         "incremental_recovery": recovered})
        flags = ["-std=" + case["mode"], "-pedantic-errors", "-fno-color-diagnostics"]
        check = run([compiler, *flags, "-fsyntax-only", native_path(source)], directory, "compiler")
        accepted = check["exit_code"] == 0
        check.update(accepted=accepted, passed=accepted == case["accepted"] and
                     (not case.get("diagnostic") or case["diagnostic"].lower() in check["output"].lower()))
        link, execution = None, None
        if accepted and case["execute"]:
            binary = directory / "fixture.program"
            link = run([compiler, *flags, native_path(source), "-o", native_path(binary)], directory, "link")
            if link["exit_code"] == 0:
                execution = run([native_path(binary)], directory, "runtime")
                execution.update(passed=execution["exit_code"] == 0 and execution["output"].strip() == "oracle:ok",
                                 executable_digest=digest(binary.read_bytes()))
        row = {"case": case["id"], "expected": case, "support_files": support, "source_digest": digest(raw), "cst": csts,
               "compiler": check, "link": link, "runtime": execution, "native_resolver": "not_run"}
        rows.append(row)
        print(json.dumps({"case": case["id"], "cst": {c["variant"]: c["expected_cst_met"] for c in csts},
                          "compiler": check["passed"], "runtime": execution["passed"] if execution else None}), flush=True)
    report = {"format": "entmoot.c_family_feature_qualification.v1", "inputs": manifest, "input_manifest_digest": digest(manifest_path.read_bytes()),
              "cases": rows, "limits": ["Compiler modes are distinct from historical toolchain versions; native Entmoot producers unqualified",
                 "C++ remains experimental; draft modes never imply stable support", "Only authored programs execute; grammar parses do not establish binds or include closure"]}
    report["comparison_completed"] = True
    report["compiler_expectations_met"] = all(r["compiler"]["passed"] for r in rows)
    report["runtime_expectations_met"] = all(not r["expected"]["execute"] or r["runtime"] is not None and r["runtime"]["passed"] for r in rows)
    report["variant_qualification"] = {
        variant: {"cases": len(rows),
                  "cst_expectations_met": sum(c["expected_cst_met"] for r in rows for c in r["cst"] if c["variant"] == variant),
                  "passed": all(c["expected_cst_met"] and c["incremental_recovery"] is not False
                                for r in rows for c in r["cst"] if c["variant"] == variant)}
        for variant in parsers}
    report["passed"] = (report["compiler_expectations_met"] and report["runtime_expectations_met"]
                        and report["variant_qualification"][args.candidate]["passed"])
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
