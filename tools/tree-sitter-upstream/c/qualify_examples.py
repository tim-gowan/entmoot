"""Stream the exact C/C++ upstream example scope without executing foreign code."""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from corpus_sources import source_blobs
from grammar_runtime import digest, load_language, runtime_digest
import tree_sitter

ROOT = Path(__file__).resolve().parent.parent
REPO = ROOT.parents[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--language", choices=("c", "cpp"), required=True)
    ap.add_argument("--libraries", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    handles, parsers, libraries = [], {}, []
    for variant, name in json.loads(args.libraries.read_text(encoding="utf-8")).items():
        path = Path(name).resolve(strict=True)
        handle, language = load_language(path, args.language)
        handles.append(handle)
        parsers[variant] = tree_sitter.Parser(language)
        libraries.append({"variant": variant, "path": str(path), "digest": digest(path.read_bytes())})
    if "tracked" not in parsers or "composed" not in parsers:
        raise ValueError("tracked and composed comparisons required")
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    source = next(row for row in index["checkouts"] if row["id"] == "tree-sitter-" + args.language)
    workflow = subprocess.check_output(["git", "-C", source["path"], "show", source["commit_sha"] + ":.github/workflows/ci.yml"])
    # Fail closed if this small upstream workflow changes its selection contract.
    if b"files: examples/*" not in workflow or b"!examples/" in workflow:
        raise ValueError("upstream example selector changed; review include/exclude rules")
    rows, counts = [], Counter()
    with source_blobs(source["path"], source["commit_sha"], suffix="", prefix="examples/") as blobs:
        for item, raw in blobs:
            # The literal workflow glob includes direct children only.
            if "/" in item["path"][len("examples/"):]:
                counts["excluded_nested"] += 1
                continue
            if raw is None:
                rows.append({**item, "excluded": "nonregular_git_entry"})
                counts["excluded_nonregular"] += 1
                continue
            observations = {}
            for variant, parser in parsers.items():
                tree = parser.parse(raw)
                observations[variant] = {"has_error": tree.root_node.has_error,
                                         "named_cst_digest": digest(str(tree.root_node).encode())}
                counts[variant + "_errors"] += int(tree.root_node.has_error)
            introduced = not observations["tracked"]["has_error"] and observations["composed"]["has_error"]
            changed = observations["tracked"]["named_cst_digest"] != observations["composed"]["named_cst_digest"]
            counts.update(parsed=1, source_bytes=len(raw), introduced_errors=int(introduced), named_cst_changes=int(changed))
            rows.append({**item, "source_digest": digest(raw), "bytes": len(raw), "parsers": observations,
                         "introduced_error": introduced, "named_cst_changed": changed, "compiler_runtime": "not_run"})
    report = {"format": "entmoot.c_family_examples.v1", "language": args.language,
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "source_driver_digest": digest((ROOT / "corpus_sources.py").read_bytes()),
              "source_index_digest": digest(index_path.read_bytes()), "source": source, "libraries": libraries,
              "workflow": {"digest": digest(workflow), "selector": "examples/*", "exclusions": []},
              "counts": counts, "files": rows, "no_new_errors": counts["introduced_errors"] == 0,
              "limits": ["Only the complete small upstream example scope; not a representative real-world campaign",
                         "No foreign source compilation or execution; include/macro/type resolution unqualified",
                         "Parse success does not establish language acceptance or semantic binds; C++ remains experimental"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps(counts))
    raise SystemExit(0 if report["no_new_errors"] else 1)


if __name__ == "__main__":
    main()
