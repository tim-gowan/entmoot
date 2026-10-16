"""Build the pinned C# baseline or a verified minimal directive supplement."""
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
    ap.add_argument("--variant", choices=("baseline", "file-directives"), required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    args = ap.parse_args()
    proposal_path = ROOT / "file-based-directives/proposal.json"
    proposal = json.loads(proposal_path.read_text(encoding="utf-8"))
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    source = next(r["path"] for r in index["checkouts"] if r["id"] == "tree-sitter-c-sharp")
    if sys.platform == "linux" and source.startswith("G:/"):
        source = "/mnt/g/" + source[3:]
    grammar = args.grammar.absolute()
    scratch_grammar(grammar.parent)
    grammar.mkdir(exist_ok=False)
    grammar = scratch_grammar(grammar)
    receipt = args.receipt.resolve()
    if receipt.exists():
        raise ValueError("select a fresh receipt")
    logs = receipt.parent / (receipt.stem + "-logs")
    logs.mkdir(parents=True, exist_ok=False)
    parser_cache = logs / "parser-cache"
    parser_cache.mkdir()
    env = {**os.environ, "TREE_SITTER_LIBDIR": str(parser_cache)}
    cli = args.cli.resolve(strict=True)
    commands = []

    def run(name, command, *, cwd=grammar, data=None):
        result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, input=data, capture_output=True, timeout=300)
        log = logs / (name + ".log")
        log.write_bytes(result.stdout + result.stderr)
        commands.append({"name": name, "command": list(map(str, command)), "cwd": str(cwd),
                         "exit_code": result.returncode, "log_digest": digest(log.read_bytes())})
        result.check_returncode()
        print(json.dumps({"step": name, "passed": True}), flush=True)
        return result

    version = run("generator-version", [cli, "--version"]).stdout.decode().strip()
    if version.split()[1] != proposal["generator"]:
        raise ValueError("pinned generator required")
    archive = logs / "source.zip"
    run("archive", ["git", "-C", source, "archive", "--format=zip", "--output", archive, proposal["base"]])
    with zipfile.ZipFile(archive) as package:
        for entry in package.infolist():
            if not (grammar / entry.filename).resolve().is_relative_to(grammar) or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("unsafe source archive")
        package.extractall(grammar)
    original = {p: digest((grammar / p).read_bytes()) for p in GENERATED}
    patch_receipt = None
    if args.variant == "file-directives":
        raw = (ROOT / "file-based-directives/grammar.patch").read_bytes()
        before = digest((grammar / "grammar.js").read_bytes())
        command = ["git", "apply", "--directory=" + grammar.relative_to(REPO).as_posix()]
        run("patch-check", [*command, "--check", "-"], cwd=REPO, data=raw)
        run("patch-apply", [*command, "-"], cwd=REPO, data=raw)
        run("patch-reverse-check", [*command, "--reverse", "--check", "-"], cwd=REPO, data=raw)
        after = digest((grammar / "grammar.js").read_bytes())
        if before == after:
            raise ValueError("patch did not change the declared grammar target")
        fixture = (ROOT / "file-based-directives/fixtures/directives.txt").read_bytes()
        (grammar / "test/corpus/entmoot_file_directives.txt").write_bytes(fixture)
        patch_receipt = {"digest": digest(raw), "before": before, "after": after, "corpus_fixture_digest": digest(fixture)}
    run("generate-1", [cli, "generate", "--abi", str(proposal["abi"])])
    generated = {p: digest((grammar / p).read_bytes()) for p in GENERATED}
    run("generate-2", [cli, "generate", "--abi", str(proposal["abi"])])
    if generated != {p: digest((grammar / p).read_bytes()) for p in GENERATED}:
        raise ValueError("nondeterministic generation")
    run("corpus", [cli, "test", "--rebuild"])
    library = logs / ("c_sharp.dll" if os.name == "nt" else "c_sharp.so")
    run("build", [cli, "build", "--output", library])
    report = {"format": "entmoot.csharp_materialization.v1", "variant": args.variant, "grammar": str(grammar),
              "source_commit": proposal["base"], "source_index_digest": digest(index_path.read_bytes()),
              "proposal_digest": digest(proposal_path.read_bytes()), "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "source_generated": original, "generated_files": generated,
              "tracked_generated_match": original == generated, "patch": patch_receipt,
              "generator": version, "generator_digest": digest(cli.read_bytes()),
              "parser_cache": str(parser_cache),
              "library": {"path": str(library), "digest": digest(library.read_bytes())}, "commands": commands,
              "passed": True, "limits": ["Build/corpus only; compiler, SDK behavior, native resolver and platform qualification are separate"]}
    with receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
