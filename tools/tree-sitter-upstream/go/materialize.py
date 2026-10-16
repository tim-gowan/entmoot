"""Build pinned Go proposals in a fresh scratch tree, verifying every patch.

Git applies extended diffs relative to the repository root. Always name the
verified scratch directory explicitly and verify reverse applicability and
changed-file digests before running the generator.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, runtime_digest, scratch_grammar

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
GENERATED = ("src/parser.c", "src/grammar.json", "src/node-types.json")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--variant", choices=("new-expressions", "method-values", "combined"), required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    source_index = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(source_index.read_text(encoding="utf-8"))
    source = Path(next(row["path"] for row in index["checkouts"] if row["id"] == "tree-sitter-go"))
    methods = json.loads((ROOT / "generic-methods/proposal.json").read_text(encoding="utf-8"))
    expressions = json.loads((ROOT / "new-expressions/proposal.json").read_text(encoding="utf-8"))
    grammar = args.grammar.absolute()
    scratch_grammar(grammar.parent)
    grammar.mkdir(exist_ok=False)
    grammar = scratch_grammar(grammar)
    receipt = args.receipt.resolve()
    if receipt.exists():
        raise ValueError("choose a fresh receipt")
    logs = receipt.parent / (receipt.stem + "-logs")
    logs.mkdir(parents=True, exist_ok=False)
    cli = args.cli.resolve(strict=True)
    commands = []

    def run(name, command, *, cwd=grammar, data=None):
        result = subprocess.run(list(map(str, command)), cwd=cwd, input=data, capture_output=True, timeout=180)
        log = logs / (name + ".log")
        log.write_bytes(result.stdout + result.stderr)
        commands.append({"name": name, "command": list(map(str, command)), "cwd": str(cwd), "exit_code": result.returncode,
                         "log_digest": digest(log.read_bytes())})
        result.check_returncode()
        return result

    version = run("generator-version", [cli, "--version"]).stdout.decode().strip()
    if not version.startswith("tree-sitter 0.25.8 "):
        raise ValueError("locked generator 0.25.8 required")
    commit = methods["head"] if args.variant == "method-values" else expressions["head"]
    archive = logs / "source.zip"
    run("archive", ["git", "-C", source, "archive", "--format=zip", "--output", archive, commit])
    with zipfile.ZipFile(archive) as package:
        for entry in package.infolist():
            destination = (grammar / entry.filename).resolve()
            if not destination.is_relative_to(grammar) or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("source archive contains a link or unsafe path")
        package.extractall(grammar)
    inputs = {p: digest((grammar / p).read_bytes()) for p in ("grammar.js", "test/corpus/declarations.txt")}
    patches = []

    def apply(name, raw, files):
        before = {p: digest((grammar / p).read_bytes()) for p in files}
        directory = grammar.relative_to(REPO).as_posix()
        command = ["git", "apply", "--directory=" + directory]
        run(name + "-check", [*command, "--check", "-"], cwd=REPO, data=raw)
        run(name + "-apply", [*command, "-"], cwd=REPO, data=raw)
        run(name + "-reverse-check", [*command, "--reverse", "--check", "-"], cwd=REPO, data=raw)
        after = {p: digest((grammar / p).read_bytes()) for p in files}
        if any(before[p] == after[p] for p in files):
            raise ValueError("patch did not change every declared target: " + name)
        (logs / (name + ".patch")).write_bytes(raw)
        patches.append({"name": name, "digest": digest(raw), "before": before, "after": after})

    if args.variant == "combined":
        raw = run("declaration-diff", ["git", "-C", source, "diff", methods["base"], methods["head"], "--",
                                      "grammar.js", "test/corpus/declarations.txt"]).stdout
        apply("declarations", raw, ("grammar.js", "test/corpus/declarations.txt"))
    if args.variant != "method-values":
        apply("shadowing", (ROOT / "new-expressions/shadowing.patch").read_bytes(), ("grammar.js",))
        (grammar / "test/corpus/entmoot_shadowing.txt").write_bytes((ROOT / "new-expressions/fixtures/corpus.txt").read_bytes())
    if args.variant != "new-expressions":
        apply("method-values", (ROOT / "generic-methods/method-values.patch").read_bytes(), ("grammar.js",))
        (grammar / "test/corpus/entmoot_method_values.txt").write_bytes((ROOT / "generic-methods/fixtures/method_values.txt").read_bytes())
    run("generate-1", [cli, "generate", "--abi", "15"])
    generated = {p: digest((grammar / p).read_bytes()) for p in GENERATED}
    run("generate-2", [cli, "generate", "--abi", "15"])
    if generated != {p: digest((grammar / p).read_bytes()) for p in GENERATED}:
        raise ValueError("non-deterministic generation")
    run("corpus", [cli, "test", "--rebuild", "--stat", "total-only"])
    library = logs / ("go.dll" if os.name == "nt" else "go.so")
    run("build", [cli, "build", "--output", library])
    report = {"format": "entmoot.go_materialization.v1", "variant": args.variant, "grammar": str(grammar),
              "source_commit": commit, "source_index_digest": digest(source_index.read_bytes()),
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "source_inputs": inputs, "patches": patches, "grammar_digest": digest((grammar / "grammar.js").read_bytes()),
              "generated_files": generated, "generator": version, "generator_digest": digest(cli.read_bytes()),
              "library": {"path": str(library), "digest": digest(library.read_bytes())}, "commands": commands,
              "passed": True, "limits": ["Build/corpus evidence only; feature, compiler, native and platform qualification are separate"]}
    with receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"receipt": str(receipt), "grammar_digest": report["grammar_digest"], "passed": True}))


if __name__ == "__main__":
    main()
