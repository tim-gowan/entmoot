"""Apply qualified build metadata only to a matching, owned grammar materialization."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_layout import generated_digests
from grammar_runtime import digest, scratch_grammar

ROOT = Path(__file__).resolve().parent.parent
REPO = ROOT.parents[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--family", choices=("javascript", "typescript"), required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--materialization", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    grammar = scratch_grammar(args.grammar)
    if args.receipt.exists():
        raise ValueError("select a fresh receipt")
    folder = ROOT / args.family / ("reserved-properties" if args.family == "javascript" else "javascript-025")
    profile_bytes = (folder / "composed.json").read_bytes()
    profile = json.loads(profile_bytes)
    materialized_bytes = args.materialization.read_bytes()
    materialized = json.loads(materialized_bytes)
    if (not materialized["passed"] or materialized["profile_digest"] != digest(profile_bytes)
            or materialized["generated_files"] != generated_digests(grammar)
            or Path(materialized["grammar"]).resolve() != grammar):
        raise ValueError("materialization does not match this profile, directory and generated parsers")
    index = json.loads((REPO / "tools/tree-sitter-upstream/checkouts.json").read_text(encoding="utf-8"))
    source = next(row["path"] for row in index["checkouts"] if row["id"] == profile["source"]["checkout_id"])
    if sys.platform == "linux" and source.startswith("G:/"):
        source = "/mnt/g/" + source[3:]
    commit = profile["proposal"]["commit"]

    def upstream(name):
        return subprocess.check_output(["git", "-C", source, "show", commit + ":" + name])

    cargo = upstream("Cargo.toml")
    if b"[workspace]" not in cargo:
        cargo += b"\n[workspace]\n"
    if (grammar / "Cargo.toml").read_bytes() != cargo:
        raise ValueError("scratch Cargo manifest has unrelated changes")
    plans = []
    for name in (["Cargo.lock"] if args.family == "typescript" else []) + ["go.mod", "go.sum"]:
        original = upstream(name)
        qualified_path = folder / "build-locks" / name
        qualified = qualified_path.read_bytes()
        target = grammar / name
        if target.resolve(strict=True).parent != grammar or target.is_symlink():
            raise ValueError("binding metadata must be a direct scratch file")
        before = target.read_bytes()
        if before not in (original, qualified):
            raise ValueError("scratch metadata has unrelated changes: " + name)
        plans.append((target, qualified, {"path": name, "original_digest": digest(original),
                      "before_digest": digest(before), "qualified_digest": digest(qualified)}))
    # Validate all input fences before changing any metadata; parser bytes remain untouched.
    for target, qualified, _ in plans:
        target.write_bytes(qualified)
    report = {"format": "entmoot.ecmascript_binding_preparation.v1", "family": args.family,
              "driver_digest": digest(Path(__file__).read_bytes()), "profile_digest": digest(profile_bytes),
              "materialization_digest": digest(materialized_bytes), "source_commit": commit,
              "files": [row for _, _, row in plans], "generated_files": generated_digests(grammar),
              "scope": "Go-generated ABI-15 runtime v0.25.0 requires Go 1.23; TypeScript also uses Cargo-generated tree-sitter 0.25.10/cc 1.2.57 lock repair",
              "limits": ["Original locked-runtime failures remain explicit; foreign checkout untouched",
                         "Reassess and remove corrections when upstream metadata supports its generated parser ABI",
                         "Build metadata qualification does not qualify Entmoot native producers"]}
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
