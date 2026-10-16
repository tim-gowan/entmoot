"""Prepare locked upstream binding metadata in a verified C# build materialization."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, scratch_grammar


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--materialization", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    raw = args.materialization.read_bytes()
    receipt = json.loads(raw)
    if receipt["format"] != "entmoot.csharp_materialization.v1" or not receipt["passed"]:
        raise ValueError("qualified materialization required")
    grammar = scratch_grammar(Path(receipt["grammar"]))
    for name, expected in receipt["generated_files"].items():
        if digest((grammar / name).read_bytes()) != expected:
            raise ValueError("materialized parser changed")
    if args.receipt.exists():
        raise ValueError("select a fresh preparation receipt")
    cargo = grammar / "Cargo.toml"
    before = cargo.read_bytes()
    if b"[workspace]" in before:
        raise ValueError("already prepared")
    cargo.write_bytes(before + b"\n[workspace]\n")
    names = ("Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "go.mod", "go.sum")
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump({"format": "entmoot.csharp_binding_preparation.v1", "driver_digest": digest(Path(__file__).read_bytes()),
                   "materialization_digest": digest(raw), "cargo_before": digest(before),
                   "files": {name: digest((grammar / name).read_bytes()) for name in names},
                   "limits": ["Only workspace isolation appended; upstream checked-in lockfiles unchanged",
                              "Run npm ci --ignore-scripts and build its exact tree-sitter peer separately"]}, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
