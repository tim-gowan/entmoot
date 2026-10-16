"""Apply qualified C-family binding metadata inside a matching scratch build."""
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
    ap.add_argument("--family", choices=("c", "cpp"), required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--materialization", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    grammar = scratch_grammar(args.grammar)
    if args.receipt.exists():
        raise ValueError("select a fresh preparation receipt")
    folder = ROOT / args.family
    profile_raw = (folder / ("modern-types" if args.family == "c" else "if-consteval") / "composed.json").read_bytes()
    profile = json.loads(profile_raw)
    materialized_raw = args.materialization.read_bytes()
    materialized = json.loads(materialized_raw)
    if (not materialized["passed"] or materialized["profile_digest"] != digest(profile_raw)
            or materialized["generated_files"] != generated_digests(grammar)
            or Path(materialized["grammar"]).resolve() != grammar):
        raise ValueError("profile, directory or generated-parser fence mismatch")
    index = json.loads((REPO / "tools/tree-sitter-upstream/checkouts.json").read_text(encoding="utf-8"))
    source = next(row["path"] for row in index["checkouts"] if row["id"] == profile["source"]["checkout_id"])
    if sys.platform == "linux" and source.startswith("G:/"):
        source = "/mnt/g/" + source[3:]
    commit = profile.get("proposal", profile["source"])["commit"]

    def upstream(name):
        return subprocess.check_output(["git", "-C", source, "show", commit + ":" + name])

    cargo = upstream("Cargo.toml")
    if b"[workspace]" not in cargo:
        cargo += b"\n[workspace]\n"
    if (grammar / "Cargo.toml").read_bytes() != cargo:
        raise ValueError("unrelated Cargo manifest change")
    plans = []
    for name in ["go.mod", "go.sum"] + (["Cargo.lock"] if args.family == "cpp" else []):
        original = upstream(name)
        qualified = (folder / "build-locks" / name).read_bytes()
        target = grammar / name
        if target.resolve(strict=True).parent != grammar or target.is_symlink():
            raise ValueError("metadata must be a direct regular scratch file")
        before = target.read_bytes()
        if before not in (original, qualified):
            raise ValueError("unrelated metadata change: " + name)
        plans.append((target, qualified, {"path": name, "original_digest": digest(original),
                                        "before_digest": digest(before), "qualified_digest": digest(qualified)}))
    patch, command, patch_files = None, None, []
    if args.family == "c":
        patch = (folder / "build-metadata/bindings.patch").read_bytes()
        command = ["git", "apply", "--directory=" + grammar.relative_to(REPO).as_posix()]
        for name in ("CMakeLists.txt", "bindings/python/tests/test_binding.py"):
            target = (grammar / name).resolve(strict=True)
            if not target.is_relative_to(grammar):
                raise ValueError("binding patch target escapes scratch")
            patch_files.append({"path": name, "before_digest": digest(target.read_bytes())})
            if target.read_bytes() != upstream(name):
                raise ValueError("binding patch requires original source bytes")
        subprocess.run([*command, "--check", "-"], input=patch, cwd=REPO, check=True, capture_output=True)
    # All original-source, profile and parser fences pass before mutation.
    if command:
        subprocess.run([*command, "-"], input=patch, cwd=REPO, check=True, capture_output=True)
        for row in patch_files:
            row["qualified_digest"] = digest((grammar / row["path"]).read_bytes())
    for target, qualified, _ in plans:
        target.write_bytes(qualified)
    if generated_digests(grammar) != materialized["generated_files"]:
        raise ValueError("binding preparation changed generated parser sources")
    report = {"format": "entmoot.c_family_binding_preparation.v1", "family": args.family,
              "driver_digest": digest(Path(__file__).read_bytes()), "profile_digest": digest(profile_raw),
              "materialization_digest": digest(materialized_raw), "source_commit": commit,
              "files": [row for _, _, row in plans], "patch_digest": digest(patch) if patch else None,
              "patched_files": patch_files, "generated_files": generated_digests(grammar),
              "scope": "Go runtime 0.25.0/Go 1.23 metadata; C Python imports and portable CMake generation; C++ Cargo runtime 0.26.5 lock",
              "limits": ["Original failures retained separately; no foreign checkout writes",
                         "Remove or reassess when upstream fixes the corresponding metadata",
                         "Binding smoke tests are independent of Entmoot native semantic qualification"]}
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
