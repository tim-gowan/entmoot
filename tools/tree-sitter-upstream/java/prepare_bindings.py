"""Apply retained qualification locks only to a verified Java scratch build."""
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
    ap.add_argument("--variant", choices=("baseline", "candidate"), required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    grammar = scratch_grammar(args.grammar)
    if args.receipt.exists():
        raise ValueError("select a fresh receipt")
    proposal = json.loads((ROOT / "modern-syntax/proposal.json").read_text(encoding="utf-8"))[args.variant]
    index = json.loads((REPO / "tools/tree-sitter-upstream/checkouts.json").read_text(encoding="utf-8"))
    source = next(r["path"] for r in index["checkouts"] if r["id"] == proposal["checkout_id"])
    if sys.platform == "linux" and source.startswith("G:/"):
        source = "/mnt/g/" + source[3:]
    expected = subprocess.check_output(["git", "-C", source, "show", proposal["commit"] + ":grammar.js"])
    if (grammar / "grammar.js").read_bytes() != expected:
        raise ValueError("scratch grammar does not match the pinned source")
    metadata = json.loads((grammar / "package.json").read_text(encoding="utf-8"))
    expected_name = "tree-sitter-java" + ("-orchard" if args.variant == "candidate" else "")
    if metadata["name"] != expected_name:
        raise ValueError("unexpected generated binding metadata")
    locks = ROOT / "modern-syntax/build-locks"
    manifest = json.loads((locks / "manifest.json").read_text(encoding="utf-8"))
    copies = []
    for row in manifest["inputs"]:
        if row["variant"] == args.variant:
            path = (locks / row["file"]).resolve(strict=True)
            if not path.is_relative_to(locks.resolve()):
                raise ValueError("lock path escapes packet")
            raw = path.read_bytes()
            if digest(raw) != row["digest"]:
                raise ValueError("retained lock digest mismatch")
            copies.append((grammar / path.name, raw))
    cargo = grammar / "Cargo.toml"
    raw_cargo = cargo.read_bytes()
    if b"[workspace]" not in raw_cargo:
        copies.append((cargo, raw_cargo + b"\n[workspace]\n"))
    rows = []
    for target, raw in copies:
        rows.append({"path": target.name, "before": digest(target.read_bytes()) if target.exists() else None,
                     "after": digest(raw)})
    for target, raw in copies:
        target.write_bytes(raw)
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump({"format": "entmoot.java_binding_preparation.v1", "variant": args.variant,
                   "driver_digest": digest(Path(__file__).read_bytes()), "grammar_digest": digest(expected),
                   "lock_manifest_digest": digest((locks / "manifest.json").read_bytes()), "files": rows,
                   "next": "npm ci --ignore-scripts; build the exact Node runtime peer, then run the shared platform driver"}, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
