"""Stream pinned Java examples, comparing grammar recovery and javac parsing only."""
from __future__ import annotations

import argparse
import base64
from collections import Counter
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from corpus_sources import source_blobs
from grammar_runtime import digest, load_language, runtime_digest
import tree_sitter

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SOURCES = ("tree-sitter-java", "tree-sitter-java-orchard", "elasticsearch", "guava", "RxJava", "flink", "log4j2", "cassandra")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--jdk", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    jdk = args.jdk.resolve(strict=True)
    suffix = ".exe" if os.name == "nt" else ""
    compiler, runtime = (jdk / "bin" / (name + suffix) for name in ("javac", "java"))
    env = {k: v for k, v in os.environ.items() if k not in ("JDK_JAVA_OPTIONS", "JDK_JAVAC_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")}
    classes, empty = out / "classes", out / "empty"
    classes.mkdir()
    empty.mkdir()
    oracle_path = ROOT / "modern-syntax/ParseOracle.java"
    subprocess.run([compiler, "-encoding", "UTF-8", "-proc:none", "--release", "27", "--class-path", str(empty),
                    "--source-path", str(empty), "-d", str(classes), str(oracle_path)], env=env, check=True, capture_output=True)
    libraries = [args.baseline.resolve(strict=True), args.candidate.resolve(strict=True)]
    handles = [load_language(path, name) for path, name in zip(libraries, ("java", "java_orchard"))]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    totals, sources, changed = Counter(), [], []
    files = out / "files.jsonl"
    command = [str(runtime), "-Xmx512m", "--class-path", str(classes), "ParseOracle"]
    with (out / "oracle-stderr.log").open("wb") as error_log, subprocess.Popen(
            command, cwd=out, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=error_log,
            text=True, encoding="utf-8") as oracle, files.open("x", encoding="utf-8") as output:
        for name in SOURCES:
            row = next(r for r in index["checkouts"] if r["id"] == name)
            counts = Counter()
            grammar_source = name.startswith("tree-sitter-")
            sources.append({"id": name, "path": row["path"], "upstream": row["upstream"], "commit": row["commit_sha"],
                            "selector": "examples/**/*.java" if grammar_source else "**/*.java", "counts": counts})
            with source_blobs(row["path"], row["commit_sha"], suffix=".java", prefix="examples/" if grammar_source else "") as blobs:
                for item, raw in blobs:
                    counts["selected"] += 1
                    if raw is None:
                        counts["excluded_nonregular"] += 1
                        output.write(json.dumps({"corpus": name, **item, "excluded": "nonregular_git_entry"}) + "\n")
                        continue
                    observations = []
                    for parser in parsers:
                        tree = parser.parse(raw)
                        observations.append({"has_error": tree.root_node.has_error, "named_cst_digest": digest(str(tree.root_node).encode())})
                    oracle.stdin.write(base64.b64encode(raw).decode() + "\n")
                    oracle.stdin.flush()
                    response = oracle.stdout.readline()
                    if not response:
                        raise ValueError("javac parse oracle ended before returning evidence")
                    outcome, diagnostic = response.rstrip("\n").split("\t", 1)
                    if outcome not in ("accepted", "rejected", "unqualified"):
                        raise ValueError("unknown oracle outcome")
                    old, new = observations
                    introduced = not old["has_error"] and new["has_error"]
                    recovered = old["has_error"] and not new["has_error"]
                    result = {"corpus": name, "commit": row["commit_sha"], **item, "source_digest": digest(raw), "bytes": len(raw),
                              "baseline": old, "candidate": new, "javac_parse": {"outcome": outcome, "diagnostic": diagnostic},
                              "introduced_error": introduced, "recovered_parse": recovered}
                    output.write(json.dumps(result, ensure_ascii=False) + "\n")
                    counts.update(parsed=1, source_bytes=len(raw), baseline_errors=int(old["has_error"]),
                                  candidate_errors=int(new["has_error"]), introduced_errors=int(introduced), recovered_parse=int(recovered),
                                  named_cst_changes=int(old["named_cst_digest"] != new["named_cst_digest"]))
                    counts["javac_parse_" + outcome] += 1
                    if introduced or recovered or new["has_error"]:
                        changed.append(result)
            totals.update(counts)
            print(json.dumps({"corpus": name, "counts": counts}), flush=True)
        oracle.stdin.close()
        oracle.stdout.close()
        if oracle.wait(timeout=30):
            raise ValueError("javac parse oracle failed")
    report = {"format": "entmoot.java_examples.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "source_driver_digest": digest((ROOT.parent / "corpus_sources.py").read_bytes()),
              "oracle_digest": digest(oracle_path.read_bytes()), "source_index_digest": digest(index_path.read_bytes()),
              "jdk": {"compiler_digest": digest(compiler.read_bytes()), "runtime_digest": digest(runtime.read_bytes()),
                      "modules_digest": digest((jdk / "lib/modules").read_bytes()), "oracle_command": command},
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries],
              "sources": sources, "counts": totals, "changed_or_error": changed,
              "files_receipt": {"path": str(files), "digest": digest(files.read_bytes())},
              "passed_no_new_errors": totals["introduced_errors"] == 0,
              "limits": ["All tracked regular Java files in six original upstream example repos; grammar examples included separately",
                         "No upstream workflow exclusions; nonregular Git entries excluded explicitly",
                         "JavacTask.parse uses source 27 with preview enabled; no attribution, type checking, dependency resolution or foreign code execution",
                         "Javac parse acceptance is neither stable-language acceptance nor a successful compilation",
                         "Named CST digests omit anonymous nodes; authored controls separately compare fields, spans and incremental recovery",
                         "Upstream example workflow is continue-on-error; retained errors remain unqualified, never binding negatives",
                         "No production visitor, native binding or pipeline performance qualification"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if report["passed_no_new_errors"] else 1)


if __name__ == "__main__":
    main()
