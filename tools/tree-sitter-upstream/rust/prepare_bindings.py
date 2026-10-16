"""Apply only Cargo's qualified root-version correction to an owned Rust build."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, scratch_grammar

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    grammar = scratch_grammar(args.grammar)
    if args.receipt.exists():
        raise ValueError("select a fresh receipt")
    profile = json.loads((ROOT / "baseline.json").read_text(encoding="utf-8"))
    index = json.loads((REPO / "tools/tree-sitter-upstream/checkouts.json").read_text(encoding="utf-8"))
    source = next(r["path"] for r in index["checkouts"] if r["id"] == profile["source"]["checkout_id"])
    if sys.platform == "linux" and source.startswith("G:/"):
        source = "/mnt/g/" + source[3:]
    commit = profile["source"]["commit"]
    original = subprocess.check_output(["git", "-C", source, "show", commit + ":Cargo.lock"])
    source_grammar = subprocess.check_output(["git", "-C", source, "show", commit + ":grammar.js"])
    if (grammar / "grammar.js").read_bytes() != source_grammar:
        raise ValueError("unexpected scratch grammar")
    needle = b'name = "tree-sitter-rust"\nversion = "0.24.1"\n'
    if original.count(needle) != 1:
        raise ValueError("upstream metadata changed; reassess temporary correction")
    expected = original.replace(needle, b'name = "tree-sitter-rust"\nversion = "0.24.2"\n')
    qualified = (ROOT / "build-locks/Cargo.lock").read_bytes()
    if qualified != expected:
        raise ValueError("qualified lock changes more than the stale root-package version")
    target = grammar / "Cargo.lock"
    before = target.read_bytes()
    if before not in (original, qualified):
        raise ValueError("scratch lock has unrelated changes")
    target.write_bytes(qualified)
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump({"format": "entmoot.rust_binding_preparation.v1", "driver_digest": digest(Path(__file__).read_bytes()),
                   "source_commit": commit, "grammar_digest": digest(source_grammar), "original_lock_digest": digest(original),
                   "before_digest": digest(before), "qualified_lock_digest": digest(qualified),
                   "scope": "Cargo-generated root version 0.24.1 to declared 0.24.2 only; every dependency pin unchanged",
                   "limits": ["Foreign checkout untouched; upstream uncorrected --locked failure remains in the packet",
                              "Remove temporary lock when the upstream release metadata agrees"]}, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
