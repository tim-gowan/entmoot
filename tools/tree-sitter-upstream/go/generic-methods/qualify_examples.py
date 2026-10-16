"""Stream exact upstream example objects; separate CST and Go parser evidence."""
from __future__ import annotations

import argparse
from collections import Counter
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import corpus_sources
from corpus_sources import source_blobs
from grammar_runtime import digest, load_language, runtime_digest
import tree_sitter

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[3]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--go", type=Path, required=True)
    ap.add_argument("--cache", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    if sys.version_info < (3, 13):
        raise ValueError("this driver requires Python 3.13 glob matching")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    compiler = args.go.resolve(strict=True)
    env = {**os.environ, "GOTOOLCHAIN": "local", "GOPROXY": "off", "GOSUMDB": "off", "GOENV": "off",
           "GOROOT": str(compiler.parent.parent), "GOFLAGS": "", "GOEXPERIMENT": "", "CGO_ENABLED": "0",
           "GOCACHE": str(args.cache.resolve())}
    host = subprocess.check_output([compiler, "env", "GOHOSTOS", "GOHOSTARCH"], env=env, text=True).split()
    env.update(GOOS=host[0], GOARCH=host[1])
    version = subprocess.check_output([compiler, "version"], env=env, text=True).strip()
    if not version.startswith("go version go1.27.2 "):
        raise ValueError("Go 1.27.2 required")
    oracle = out / ("syntax-oracle.exe" if os.name == "nt" else "syntax-oracle")
    build = subprocess.run([compiler, "build", "-o", oracle, ROOT / "syntax_oracle.go"], env=env, capture_output=True)
    (out / "build.log").write_bytes(build.stdout + build.stderr)
    build.check_returncode()
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    grammar = next(row for row in index["checkouts"] if row["id"] == "tree-sitter-go")
    workflow = subprocess.check_output(["git", "-C", grammar["path"], "show", grammar["commit_sha"] + ":.github/workflows/ci.yml"])
    lines = workflow.decode().splitlines()
    start = next(i for i, line in enumerate(lines) if line.strip() == "invalid-files: |") + 1
    invalid_patterns = []
    for line in lines[start:]:
        if not line.startswith("            "):
            break
        invalid_patterns.append(line.strip())
    if len(invalid_patterns) != 28:
        raise ValueError("review changed upstream invalid-file declarations")
    libraries = [p.resolve(strict=True) for p in (args.baseline, args.candidate)]
    handles = [load_language(p, "go") for p in libraries]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    totals, sources, changed = Counter(), [], []
    with subprocess.Popen([oracle], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, text=True, encoding="utf-8") as native:
        with (out / "files.jsonl").open("x", encoding="utf-8") as stream:
            for name in ("tree-sitter-go", "go", "moby"):
                checkout = next(row for row in index["checkouts"] if row["id"] == name)
                counts = Counter()
                prefix = "examples/" if name == "tree-sitter-go" else ""
                sources.append({"id": name, "path": checkout["path"], "commit": checkout["commit_sha"],
                                "selector": prefix + "**/*.go", "counts": counts})
                with source_blobs(checkout["path"], checkout["commit_sha"], suffix=".go", prefix=prefix) as entries:
                    for entry, raw in entries:
                        counts["selected_paths"] += 1
                        result = {"corpus": name, "commit": checkout["commit_sha"], **entry}
                        if raw is None:
                            counts["excluded_nonregular"] += 1
                            stream.write(json.dumps({**result, "excluded": "nonregular_git_entry"}) + "\n")
                            continue
                        path = entry["path"]
                        workflow_path = path if name == "tree-sitter-go" else "examples/" + name + "/" + path
                        patterns = [p for p in invalid_patterns if PurePosixPath(workflow_path).full_match(p)]
                        observations = []
                        for parser in parsers:
                            start = time.perf_counter_ns()
                            tree = parser.parse(raw)
                            observations.append({"has_error": tree.root_node.has_error, "parse_ns": time.perf_counter_ns() - start,
                                                 "cst_digest": digest(str(tree.root_node).encode())})
                        native.stdin.write(json.dumps({"Path": path, "SourceHex": raw.hex()}) + "\n")
                        native.stdin.flush()
                        line = native.stdout.readline()
                        if not line:
                            raise ValueError("syntax oracle stopped before returning a result")
                        acceptance = json.loads(line)
                        old, new = observations
                        regression = not old["has_error"] and new["has_error"]
                        cst_changed = old["cst_digest"] != new["cst_digest"]
                        result.update(source_digest=digest(raw), bytes=len(raw), baseline=old, candidate=new,
                                      go_parser=acceptance, upstream_invalid_patterns=patterns, new_grammar_error=regression)
                        stream.write(json.dumps(result, ensure_ascii=False) + "\n")
                        counts.update({"parsed": 1, "source_bytes": len(raw), "baseline_errors": old["has_error"],
                                       "candidate_errors": new["has_error"], "go_parser_rejected": not acceptance["accepted"],
                                       "new_grammar_errors": regression, "cst_changes": cst_changed,
                                       "recovered_parse": old["has_error"] and not new["has_error"],
                                       "upstream_invalid_marked": bool(patterns),
                                       "upstream_invalid_accepted_by_grammar": bool(patterns) and not new["has_error"],
                                       "unmarked_candidate_errors": not patterns and new["has_error"]})
                        if cst_changed or regression:
                            changed.append(result)
                totals.update(counts)
                print(json.dumps({"corpus": name, "counts": counts}), flush=True)
        native.stdin.close()
        if native.wait(timeout=30):
            raise ValueError("syntax oracle failed")
    report = {"format": "entmoot.go_grammar_examples.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "corpus_driver_digest": digest(Path(corpus_sources.__file__).read_bytes()),
              "source_index_digest": digest(index_path.read_bytes()), "sources": sources, "counts": totals,
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries],
              "oracle": {"tool": version, "source_digest": digest((ROOT / "syntax_oracle.go").read_bytes()),
                         "executable_digest": digest(oracle.read_bytes()), "go_executable_digest": digest(compiler.read_bytes()),
                         "scope": "go/parser syntax only; no type checking or corpus execution"},
              "workflow": {"digest": digest(workflow), "invalid_patterns": invalid_patterns, "continue_on_error": True},
              "changes": changed, "files_receipt": {"path": str(out / "files.jsonl"), "digest": digest((out / "files.jsonl").read_bytes())},
              "no_new_grammar_errors": totals["new_grammar_errors"] == 0,
              "limits": ["Every tracked regular Go source at pinned example revisions; symlinks excluded, invalid patterns are classifications, not exclusions",
                         "Existing errors and semantic-invalid programs remain diagnostics; this is not blanket corpus qualification",
                         "Native resolver and actual compiler/runtime controls are separate lanes",
                         "Single-pass times are diagnostics, not cold/warm pipeline measurements"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if report["no_new_grammar_errors"] else 1)


if __name__ == "__main__":
    main()
