"""Compare tracked/regenerated Rust grammars over exact upstream example pins."""
import argparse
from collections import Counter
import json
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
SOURCES = ("tree-sitter-rust", "bitflags", "libc", "regex", "serde", "tokio")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tracked", type=Path, required=True)
    ap.add_argument("--regenerated", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    libraries = [p.resolve(strict=True) for p in (args.tracked, args.regenerated)]
    handles = [load_language(p, "rust") for p in libraries]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    grammar_source = next(r for r in index["checkouts"] if r["id"] == SOURCES[0])
    workflow = subprocess.check_output(["git", "-C", grammar_source["path"], "show", grammar_source["commit_sha"] + ":.github/workflows/ci.yml"])
    exclusions = re.findall(r"^\s+!(examples/[^\r\n]+\.rs)\s*$", workflow.decode(), re.MULTILINE)
    if len(set(exclusions)) != len(exclusions) or any(c in p for p in exclusions for c in "*?[{"):
        raise ValueError("workflow exclusions changed; reassess selection semantics")
    excluded_paths, matched_exclusions = set(exclusions), set()
    totals, sources, errors = Counter(), [], []
    files = out / "files.jsonl"
    with files.open("x", encoding="utf-8", newline="\n") as stream:
        for name in SOURCES:
            row = next(r for r in index["checkouts"] if r["id"] == name)
            counts = Counter()
            prefix = "examples/" if name == SOURCES[0] else ""
            sources.append({"id": name, "path": row["path"], "upstream": row["upstream"], "commit": row["commit_sha"],
                            "selector": prefix + "**/*.rs", "counts": counts})
            with source_blobs(row["path"], row["commit_sha"], suffix=".rs", prefix=prefix) as blobs:
                for item, raw in blobs:
                    counts["selected"] += 1
                    if raw is None:
                        counts["excluded_nonregular"] += 1
                        stream.write(json.dumps({"corpus": name, **item, "excluded": "nonregular_git_entry"}) + "\n")
                        continue
                    workflow_path = ("examples/" + name + "/" if name != SOURCES[0] else "") + item["path"]
                    excluded = workflow_path in excluded_paths
                    if excluded:
                        matched_exclusions.add(workflow_path)
                    try:
                        raw.decode("utf-8")
                        utf8 = True
                    except UnicodeDecodeError:
                        utf8 = False
                    observations = []
                    for parser in parsers:
                        tree = parser.parse(raw)
                        observations.append({"has_error": tree.root_node.has_error, "named_cst_digest": digest(str(tree.root_node).encode())})
                    old, new = observations
                    changed = old["named_cst_digest"] != new["named_cst_digest"]
                    introduced = not old["has_error"] and new["has_error"]
                    result = {"corpus": name, "commit": row["commit_sha"], **item, "source_digest": digest(raw), "bytes": len(raw),
                              "workflow_excluded": excluded, "utf8_valid": utf8, "tracked": old, "regenerated": new,
                              "introduced_error": introduced, "named_cst_changed": changed, "compiler_runtime": "not_run"}
                    stream.write(json.dumps(result, ensure_ascii=False) + "\n")
                    counts.update(parsed=1, source_bytes=len(raw), tracked_errors=int(old["has_error"]), regenerated_errors=int(new["has_error"]),
                                  introduced_errors=int(introduced), named_cst_changes=int(changed), invalid_utf8=int(not utf8))
                    scope = "workflow_excluded_" if excluded else "workflow_included_"
                    counts[scope + "parsed"] += 1
                    counts[scope + "errors"] += int(new["has_error"])
                    if changed or new["has_error"] or not utf8:
                        errors.append(result)
            totals.update(counts)
            print(json.dumps({"corpus": name, "counts": counts}), flush=True)
    report = {"format": "entmoot.rust_examples.v1", "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "source_driver_digest": digest((ROOT.parent / "corpus_sources.py").read_bytes()), "source_index_digest": digest(index_path.read_bytes()),
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries], "sources": sources, "counts": totals,
              "workflow": {"path": ".github/workflows/ci.yml", "digest": digest(workflow), "exclusions": exclusions,
                           "matched_exclusions": sorted(matched_exclusions), "unmatched_exclusions": sorted(excluded_paths - matched_exclusions)},
              "errors_or_changed": errors, "files_receipt": {"path": str(files), "digest": digest(files.read_bytes())},
              "passed_cst_parity": totals["named_cst_changes"] == 0 and totals["introduced_errors"] == 0,
              "limits": ["Tracked and regenerated parser differ in declared release metadata only; this is a baseline comparison, not a syntax fix",
                         "All tracked regular Rust source objects selected by upstream examples, including explicit excluded files counted separately",
                         "No rustc/Cargo compilation or foreign code execution; editions, conditional compilation and macro expansion unqualified",
                         "Parse success is not language/semantic acceptance; malformed inputs and invalid UTF-8 stay explicit",
                         "No Entmoot visitor, native resolution, pipeline or storage performance claim"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if report["passed_cst_parity"] else 1)


if __name__ == "__main__":
    main()
