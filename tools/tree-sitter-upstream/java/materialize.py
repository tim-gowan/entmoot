"""Build the pinned Java grammars without changing their source checkouts."""
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
    ap.add_argument("--variant", choices=("baseline", "candidate"), required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    proposal_path = ROOT / "modern-syntax/proposal.json"
    proposal = json.loads(proposal_path.read_text(encoding="utf-8"))[args.variant]
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    source = Path(next(r["path"] for r in index["checkouts"] if r["id"] == proposal["checkout_id"]))
    if sys.platform == "linux" and str(source).startswith("G:/"):
        source = Path("/mnt/g") / str(source)[3:]
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

    def run(name, command):
        result = subprocess.run(list(map(str, command)), cwd=grammar, capture_output=True, timeout=300)
        log = logs / (name + ".log")
        log.write_bytes(result.stdout + result.stderr)
        commands.append({"name": name, "command": list(map(str, command)), "exit_code": result.returncode,
                         "log_digest": digest(log.read_bytes())})
        result.check_returncode()
        return result

    version = run("generator-version", [cli, "--version"]).stdout.decode().strip()
    if version.split()[1] != proposal["generator"]:
        raise ValueError("pinned generator required")
    archive = logs / "source.zip"
    run("archive", ["git", "-C", source, "archive", "--format=zip", "--output", archive, proposal["commit"]])
    with zipfile.ZipFile(archive) as package:
        for entry in package.infolist():
            destination = (grammar / entry.filename).resolve()
            if not destination.is_relative_to(grammar) or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("unsafe source archive")
        package.extractall(grammar)
    source_generated = {p: digest((grammar / p).read_bytes()) for p in GENERATED if (grammar / p).exists()}
    if args.variant == "candidate":
        if source_generated:
            raise ValueError("Orchard source unexpectedly contains generated files")
        run("init", [cli, "init"])
    run("generate-1", [cli, "generate", "--abi", str(proposal["abi"])])
    generated = {p: digest((grammar / p).read_bytes()) for p in GENERATED}
    run("generate-2", [cli, "generate", "--abi", str(proposal["abi"])])
    if generated != {p: digest((grammar / p).read_bytes()) for p in GENERATED}:
        raise ValueError("non-deterministic generation")
    run("corpus", [cli, "test", "--rebuild"])
    library = logs / (proposal["export"] + (".dll" if os.name == "nt" else ".so"))
    run("build", [cli, "build", "--output", library])
    report = {"format": "entmoot.java_materialization.v1", "variant": args.variant, "grammar": str(grammar),
              "source": proposal, "source_index_digest": digest(index_path.read_bytes()),
              "proposal_digest": digest(proposal_path.read_bytes()), "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "source_generated": source_generated,
              "generated_files": generated, "tracked_generated_match": source_generated == generated if source_generated else None,
              "grammar_digest": digest((grammar / "grammar.js").read_bytes()),
              "generator": version, "generator_digest": digest(cli.read_bytes()),
              "library": {"path": str(library), "digest": digest(library.read_bytes())},
              "commands": commands, "passed": True,
              "limits": ["Build/corpus only; feature, compiler, native and binding/platform qualification are separate"]}
    with receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"receipt": str(receipt), "passed": True, "tracked_generated_match": report["tracked_generated_match"]}))


if __name__ == "__main__":
    main()
