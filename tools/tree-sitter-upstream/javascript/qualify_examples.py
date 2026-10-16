"""Stream pinned JS/TS/TSX example objects; compare grammars and parse-only oracles."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from corpus_sources import source_blobs
from grammar_runtime import digest, load_language, runtime_digest
import tree_sitter

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--libraries", type=Path, required=True)
    ap.add_argument("--node", type=Path, required=True)
    ap.add_argument("--typescript6", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    node, compiler = args.node.resolve(strict=True), args.typescript6.resolve(strict=True)
    env = {key: value for key, value in os.environ.items() if key not in ("NODE_OPTIONS", "NODE_PATH") and not key.startswith("TS_NODE_")}
    if subprocess.check_output([node, "--version"], text=True, env=env).strip() != "v26.11.1":
        raise ValueError("Node 26.11.1 required")
    manifest = json.loads(args.libraries.read_text(encoding="utf-8"))
    handles, parsers, libraries = [], {}, []
    for language, variants in manifest.items():
        parsers[language] = {}
        for variant, path in variants.items():
            path = Path(path).resolve(strict=True)
            handle, grammar = load_language(path, language)
            handles.append(handle)
            parsers[language][variant] = tree_sitter.Parser(grammar)
            libraries.append({"language": language, "variant": variant, "path": str(path), "digest": digest(path.read_bytes())})
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    ts = next(r for r in index["checkouts"] if r["id"] == "tree-sitter-typescript")
    workflow = subprocess.check_output(["git", "-C", ts["path"], "show", ts["commit_sha"] + ":.github/workflows/ci.yml"])
    exclusions = set(re.findall(r"^\s+!(examples/[^\r\n]+)\s*$", workflow.decode(), re.MULTILINE))
    if len(exclusions) != 3 or any(c in p for p in exclusions for c in "*?[{"):
        raise ValueError("original TypeScript exclusions changed; reassess")
    matched, totals, sources, errors = set(), Counter(), [], []
    worker_path = ROOT / "parse_examples.cjs"
    worker_log = out / "parse-worker.stderr"
    files = out / "files.jsonl"
    with worker_log.open("xb") as stderr, subprocess.Popen([node, worker_path, compiler], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=stderr, env=env) as worker, files.open("x", encoding="utf-8", newline="\n") as stream:
        try:
            ordinal = 0
            for name in ("npm-cli", "redux", "desktop", "vscode"):
                source = next(r for r in index["checkouts"] if r["id"] == name)
                counts = Counter()
                sources.append({"id": name, "commit": source["commit_sha"], "upstream": source["upstream"], "counts": counts})
                suffix = (".js", ".jsx") if name == "npm-cli" else (".ts", ".tsx")
                with source_blobs(source["path"], source["commit_sha"], suffix=suffix) as blobs:
                    for item, raw in blobs:
                        ordinal += 1
                        counts["selected"] += 1
                        if raw is None:
                            counts["excluded_nonregular"] += 1
                            stream.write(json.dumps({"corpus": name, **item, "excluded": "nonregular_git_entry"}) + "\n")
                            continue
                        workflow_path = "examples/" + name + "/" + item["path"]
                        excluded = workflow_path in exclusions
                        if excluded:
                            matched.add(workflow_path)
                        language = "javascript" if name == "npm-cli" else ("tsx" if item["path"].endswith(".tsx") else "typescript")
                        observations = {}
                        for variant, parser in parsers[language].items():
                            tree = parser.parse(raw)
                            observations[variant] = {"has_error": tree.root_node.has_error, "named_cst_digest": digest(str(tree.root_node).encode())}
                        try:
                            text = raw.decode("utf-8")
                        except UnicodeDecodeError:
                            text = None
                        oracle = None
                        if text is not None and language == "javascript" and not item["path"].endswith(".jsx"):
                            oracle = {"kind": "node-syntax-modes", "modes": {}}
                            for mode in ("module", "commonjs"):
                                result = subprocess.run([node, "--check", "--input-type=" + mode], input=raw, capture_output=True, timeout=30, env=env)
                                oracle["modes"][mode] = {"accepted": result.returncode == 0, "exit_code": result.returncode,
                                                         "diagnostic_digest": digest(result.stdout + result.stderr)}
                            oracle["accepted_in_some_mode"] = any(r["accepted"] for r in oracle["modes"].values())
                        elif text is not None:
                            request = {"id": ordinal, "language": "jsx" if language == "javascript" else language, "path": item["path"], "source": text}
                            worker.stdin.write(json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n")
                            worker.stdin.flush()
                            response = json.loads(worker.stdout.readline())
                            if response["id"] != ordinal:
                                raise ValueError("parse worker occurrence mismatch")
                            oracle = {"kind": "typescript-6.0.3-frontend-only", "diagnostics": response["diagnostics"], "accepted": not response["diagnostics"]}
                        old, new = observations["baseline"], observations["composed"]
                        introduced = not old["has_error"] and new["has_error"]
                        changed = old["named_cst_digest"] != new["named_cst_digest"]
                        row = {"corpus": name, "commit": source["commit_sha"], **item, "language": language, "source_digest": digest(raw),
                               "bytes": len(raw), "workflow_excluded": excluded, "utf8_valid": text is not None, "grammars": observations,
                               "introduced_error": introduced, "named_cst_changed": changed, "oracle": oracle, "runtime": "not_run"}
                        stream.write(json.dumps(row, ensure_ascii=False) + "\n")
                        counts.update(parsed=1, source_bytes=len(raw), introduced_errors=int(introduced), named_cst_changes=int(changed), invalid_utf8=int(text is None))
                        for variant, observation in observations.items():
                            counts[variant + "_errors"] += int(observation["has_error"])
                        counts[("workflow_excluded_" if excluded else "comparison_included_") + "errors"] += int(new["has_error"])
                        if oracle:
                            accepted = oracle.get("accepted", oracle.get("accepted_in_some_mode"))
                            counts["oracle_accepted" if accepted else "oracle_rejected"] += 1
                        if new["has_error"] or text is None:
                            errors.append(row)
                totals.update(counts)
                print(json.dumps({"corpus": name, "counts": counts}), flush=True)
        finally:
            worker.stdin.close()
            worker.stdout.close()
            if worker.wait(timeout=30):
                raise ValueError("parse-only worker failed")
    report = {"format": "entmoot.ecmascript_examples.v1", "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "source_driver_digest": digest((ROOT.parent / "corpus_sources.py").read_bytes()), "source_index_digest": digest(index_path.read_bytes()),
              "worker_digest": digest(worker_path.read_bytes()), "compiler_digest": digest(compiler.read_bytes()), "node_digest": digest(node.read_bytes()),
              "libraries": libraries, "sources": sources, "counts": totals, "remaining_errors": errors,
              "typescript_workflow": {"digest": digest(workflow), "exclusions": sorted(exclusions), "matched": sorted(matched), "unmatched": sorted(exclusions-matched)},
              "javascript_workflow": {"patterns": ["examples/*.js", "examples/*.jsx"], "cloned_path": "examples/cli", "clone_files_matched": 0,
                                      "comparison_scope": "Explicit broader recursive *.js/*.jsx coverage over the pinned npm/cli clone"},
              "files_receipt": {"path": str(files), "digest": digest(files.read_bytes())}, "no_new_parse_errors": totals["introduced_errors"] == 0,
              "limits": ["Source objects streamed without rewriting or copying sibling corpora; no foreign code execution",
                         "JavaScript workflow patterns do not recurse into its clone; this broader comparison is separately scoped",
                         "Original TypeScript exclusions remain visible; proposal-expanded invalid-file lists do not redefine the denominator",
                         "Node checks module and commonjs modes independently, without resolving project environment or imports",
                         "TypeScript 6 frontend diagnostics are parse-only: no Program type checking, actual build or TypeScript 7 compiler qualification",
                         "No Entmoot production visitor, native resolver, pipeline, storage or cross-platform performance claim"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if report["no_new_parse_errors"] else 1)


if __name__ == "__main__":
    main()
