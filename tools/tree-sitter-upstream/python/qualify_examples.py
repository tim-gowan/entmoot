"""Compare grammars over exact Git objects from indexed upstream example clones.

Streams source objects and one compact receipt per file. Never executes corpus
code, follows source symlinks, copies corpus trees, or changes foreign checkouts.
Compiler acceptance, grammar recovery and CST changes are separate observations.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess
import sys
import time

import tree_sitter

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from grammar_runtime import digest, load_language, runtime_digest

REPO = Path(__file__).resolve().parents[3]
EXCLUSIONS = {
    "cpython": {"Lib/test/test_annotationlib.py", "Lib/test/test_compile.py", "Lib/test/test_type_params.py"},
}
KNOWN_INVALID = {("cpython", "Lib/test/tokenizedata/badsyntax_3131.py")}

COMPILER = """import json,sys
for line in sys.stdin:
 request=json.loads(line)
 try: compile(bytes.fromhex(request['source_hex']),request['path'],'exec')
 except (SyntaxError,ValueError) as error: value={'outcome':'rejected','reason':str(error)}
 else: value={'outcome':'accepted'}
 print(json.dumps(value),flush=True)
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--python", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    baseline, candidate, interpreter = (p.resolve(strict=True) for p in (args.baseline, args.candidate, args.python))
    handles = [load_language(path, "python") for path in (baseline, candidate)]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    sources = []
    totals = Counter()
    errors = []
    with subprocess.Popen([str(interpreter), "-I", "-u", "-c", COMPILER], stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, encoding="utf-8") as compiler:
        with (out / "files.jsonl").open("x", encoding="utf-8") as output:
            for name in ("tree-sitter-python", "cpython", "numpy", "django", "flask"):
                row = next(item for item in index["checkouts"] if item["id"] == name)
                root, commit = Path(row["path"]).resolve(strict=True), row["commit_sha"]
                # Locked Git source wins over working-tree state or mutable branch names.
                listing = subprocess.check_output(["git", "-C", str(root), "ls-tree", "-rz", "--full-tree", commit])
                counts = Counter()
                sources.append({"id": name, "path": str(root), "commit": commit, "upstream": row["upstream"],
                                "selector": "examples/**/*.py" if name == "tree-sitter-python" else "**/*.py", "counts": counts})
                with subprocess.Popen(["git", "-C", str(root), "cat-file", "--batch"], stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as git:
                    for entry in listing.split(b"\0"):
                        if not entry:
                            continue
                        metadata, raw_path = entry.split(b"\t", 1)
                        mode, kind, object_id = metadata.decode().split()
                        path = raw_path.decode("utf-8")
                        if not path.endswith(".py") or (name == "tree-sitter-python" and not path.startswith("examples/")):
                            continue
                        counts["selected_python_paths"] += 1
                        if mode not in ("100644", "100755") or kind != "blob":
                            counts["excluded_nonregular"] += 1
                            output.write(json.dumps({"corpus": name, "path": path, "mode": mode, "excluded": "nonregular_git_entry"}) + "\n")
                            continue
                        if path in EXCLUSIONS.get(name, set()):
                            counts["upstream_exclusions"] += 1
                            output.write(json.dumps({"corpus": name, "path": path, "object": object_id, "excluded": "upstream_workflow"}) + "\n")
                            continue
                        git.stdin.write(object_id.encode() + b"\n")
                        git.stdin.flush()
                        header = git.stdout.readline().split()
                        if len(header) != 3 or header[1] != b"blob":
                            raise ValueError("source object unavailable: " + path)
                        size = int(header[2])
                        source = git.stdout.read(size)
                        if len(source) != size or git.stdout.read(1) != b"\n":
                            raise ValueError("truncated source object: " + path)
                        observations = []
                        for parser in parsers:
                            start = time.perf_counter_ns()
                            tree = parser.parse(source)
                            observations.append({"has_error": tree.root_node.has_error,
                                                 "parse_ns": time.perf_counter_ns() - start,
                                                 "cst_digest": digest(str(tree.root_node).encode())})
                        compiler.stdin.write(json.dumps({"path": path, "source_hex": source.hex()}) + "\n")
                        compiler.stdin.flush()
                        line = compiler.stdout.readline()
                        if not line:
                            raise ValueError("compiler process ended before returning an outcome")
                        acceptance = json.loads(line)
                        old, new = observations
                        known_invalid = (name, path) in KNOWN_INVALID
                        regression = not old["has_error"] and new["has_error"] and acceptance["outcome"] == "accepted"
                        # Python 3.15 cannot qualify Python 2 runtime behavior, but
                        # a changed grammar must still preserve upstream's old CST lane.
                        historical_regression = name == "tree-sitter-python" and not old["has_error"] and new["has_error"]
                        result = {"corpus": name, "commit": commit, "path": path, "git_object": object_id,
                                  "source_digest": digest(source), "bytes": size, "baseline": old, "candidate": new,
                                  "compiler": acceptance, "upstream_known_invalid": known_invalid,
                                  "accepted_source_regression": regression, "upstream_historical_regression": historical_regression}
                        output.write(json.dumps(result, ensure_ascii=False) + "\n")
                        counts["parsed"] += 1
                        counts["source_bytes"] += size
                        counts["baseline_errors"] += old["has_error"]
                        counts["candidate_errors"] += new["has_error"]
                        counts["compiler_rejected"] += acceptance["outcome"] == "rejected"
                        counts["accepted_source_regressions"] += regression
                        counts["upstream_historical_regressions"] += historical_regression
                        counts["recovered_parse"] += old["has_error"] and not new["has_error"]
                        counts["cst_changes"] += old["cst_digest"] != new["cst_digest"]
                        counts["upstream_invalid_missed"] += known_invalid and not new["has_error"]
                        if regression or historical_regression:
                            errors.append({"corpus": name, "path": path, "source_digest": result["source_digest"]})
                    git.stdin.close()
                    if git.wait(timeout=30):
                        raise ValueError("Git source reader failed")
                totals.update(counts)
                print(json.dumps({"corpus": name, "counts": counts}), flush=True)
        compiler.stdin.close()
        if compiler.wait(timeout=30):
            raise ValueError("compiler reader failed")
    files = out / "files.jsonl"
    receipt = {"format": "entmoot.grammar_examples.v1", "source_index_digest": digest(index_path.read_bytes()),
               "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(), "release_cutoff": "2026-10-10",
               "libraries": {"baseline": {"path": str(baseline), "digest": digest(baseline.read_bytes())},
                             "candidate": {"path": str(candidate), "digest": digest(candidate.read_bytes())}},
               "compiler": {"path": str(interpreter), "digest": digest(interpreter.read_bytes())},
               "sources": sources, "counts": totals, "regressions": errors,
               "files_receipt": {"path": str(files), "digest": digest(files.read_bytes())},
               "limits": ["All tracked regular .py objects in the four example clones and upstream examples/ at indexed pins; three exact exclusions retained",
                          "Upstream Python 2 examples have a separate no-new-grammar-error gate; Python 3.15 rejection is not historical runtime evidence",
                          "No working-tree, untracked, directory-link, .pyi or runtime execution coverage",
                          "Compiler rejection is distinct from a grammar error; corpus contains intentional invalid programs",
                          "Single-pass per-file times are diagnostic and not a cold/warm performance qualification",
                          "Zero regressions does not qualify remaining parse errors, native bindings or Entmoot visitors"]}
    (out / "summary.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(1 if errors or totals["upstream_invalid_missed"] else 0)


if __name__ == "__main__":
    main()
