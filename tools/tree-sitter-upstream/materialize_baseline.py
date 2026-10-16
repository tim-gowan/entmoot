"""Build pinned grammars and explicit source supplements with isolated caches."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

from grammar_runtime import digest, runtime_digest, scratch_grammar
from grammar_layout import generated_digests, layout_digest, parser_roots

REPO = Path(__file__).resolve().parents[2]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--profile", type=Path, required=True)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--receipt", type=Path, required=True)
    ap.add_argument("--node", type=Path, help="Explicit Node executable for a profile's locked npm dependencies")
    args = ap.parse_args()
    profile_raw = args.profile.read_bytes()
    profile = json.loads(profile_raw)
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    source = next(r for r in index["checkouts"] if r["id"] == profile["source"]["checkout_id"])
    if source["commit_sha"] != profile["source"]["commit"]:
        raise ValueError("profile/index source pin mismatch")
    source_path = source["path"]
    if sys.platform == "linux" and source_path.startswith("G:/"):
        source_path = "/mnt/g/" + source_path[3:]
    grammar = args.grammar.absolute()
    scratch_grammar(grammar.parent)
    grammar.mkdir(exist_ok=False)
    grammar = scratch_grammar(grammar)
    receipt = args.receipt.resolve()
    if receipt.exists():
        raise ValueError("select a fresh receipt")
    logs = receipt.parent / (receipt.stem + "-logs")
    logs.mkdir(parents=True, exist_ok=False)
    (logs / "parser-cache").mkdir()
    env = {**os.environ, "TREE_SITTER_LIBDIR": str(logs / "parser-cache")}
    cli = args.cli.resolve(strict=True)
    commands = []

    def run(name, command, cwd=grammar, data=None):
        result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, input=data, capture_output=True, timeout=300)
        raw = result.stdout + result.stderr
        (logs / (name + ".log")).write_bytes(raw)
        commands.append({"name": name, "command": list(map(str, command)), "cwd": str(cwd),
                         "exit_code": result.returncode, "log_digest": digest(raw)})
        result.check_returncode()
        print(json.dumps({"step": name, "passed": True}), flush=True)
        return result

    version = run("generator-version", [cli, "--version"]).stdout.decode().strip()
    if version.split()[1] != profile["generator"]:
        raise ValueError("pinned generator required")
    archive = logs / "source.zip"
    archive_commit = profile.get("proposal", {}).get("commit", source["commit_sha"])
    run("archive", ["git", "-C", source_path, "archive", "--format=zip", "--output", archive, archive_commit])
    with zipfile.ZipFile(archive) as package:
        for entry in package.infolist():
            if not (grammar / entry.filename).resolve().is_relative_to(grammar) or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("unsafe source archive")
        package.extractall(grammar)
    if profile.get("npm_install"):
        override = profile.get("npm_lock_override")
        if override:
            qualified = (args.profile.resolve().parent / override["source"]).resolve(strict=True)
            if not qualified.is_relative_to(Path(__file__).resolve().parent):
                raise ValueError("qualified npm lock must be retained inside the grammar packet tree")
            lock = grammar / "package-lock.json"
            if digest(lock.read_bytes()) != override["original_digest"]:
                raise ValueError("upstream npm lock changed; reassess the qualified correction")
            lock.write_bytes(qualified.read_bytes())
        node = args.node.resolve(strict=True) if args.node else None
        if node is None:
            raise ValueError("locked npm preparation requires an explicit Node executable")
        npm = node.parent / ("node_modules/npm/bin/npm-cli.js" if os.name == "nt" else "../lib/node_modules/npm/bin/npm-cli.js")
        env["PATH"] = str(node.parent) + os.pathsep + env["PATH"]
        run("npm-locked-install", [node, npm.resolve(strict=True), "ci", "--ignore-scripts"])
    dependencies = []
    for item in profile.get("dependency_proposals", []):
        checkout = next(r for r in index["checkouts"] if r["id"] == item["checkout_id"])
        source_root = checkout["path"]
        if sys.platform == "linux" and source_root.startswith("G:/"):
            source_root = "/mnt/g/" + source_root[3:]
        for relative in item["files"]:
            target = (grammar / "node_modules" / item["package"] / relative).resolve(strict=True)
            if not target.is_relative_to(grammar / "node_modules"):
                raise ValueError("dependency proposal escapes scratch package")
            original = subprocess.check_output(["git", "-C", source_root, "show", item["base_commit"] + ":" + relative])
            if target.read_bytes() != original:
                raise ValueError("dependency package differs from the proposal base")
            candidate = subprocess.check_output(["git", "-C", source_root, "show", item["commit"] + ":" + relative])
            target.write_bytes(candidate)
            dependencies.append({"proposal": item, "file": relative, "before": digest(original), "after": digest(candidate)})
    roots = parser_roots(grammar)
    exports = profile.get("exports", {".": profile.get("export")})
    if set(exports) != {root.relative_to(grammar).as_posix() for root in roots} or any(not name or not name.isidentifier() for name in exports.values()):
        raise ValueError("profile exports must identify every distinct parser root")
    original = generated_digests(grammar)
    extension = ".dll" if os.name == "nt" else ".so"
    source_libraries, libraries = {}, {}
    for root in roots:
        export = exports[root.relative_to(grammar).as_posix()]
        library = logs / (export + "-tracked" + extension)
        run("build-tracked-" + export, [cli, "build", "--output", library], root)
        source_libraries[export] = {"path": str(library), "digest": digest(library.read_bytes())}
    if original != generated_digests(grammar):
        raise ValueError("tracked-parser build changed generated sources")
    patches = []
    for item in profile.get("patches", []):
        patch = (args.profile.parent / item["path"]).resolve(strict=True)
        if not patch.is_relative_to(Path(__file__).resolve().parent):
            raise ValueError("patch must belong to the qualification packet")
        patch_root = (grammar / item.get("root", ".")).resolve(strict=True)
        targets = [(patch_root / p).resolve(strict=True) for p in item["files"]]
        if not patch_root.is_relative_to(grammar) or any(not target.is_relative_to(patch_root) for target in targets):
            raise ValueError("patch target escapes scratch grammar")
        before = {str(p.relative_to(grammar)): digest(p.read_bytes()) for p in targets}
        raw = patch.read_bytes()
        command = ["git", "apply", "--directory=" + patch_root.relative_to(REPO).as_posix()]
        run("patch-check-" + patch.parent.name, [*command, "--check", "-"], cwd=REPO, data=raw)
        run("patch-apply-" + patch.parent.name, [*command, "-"], cwd=REPO, data=raw)
        run("patch-reverse-" + patch.parent.name, [*command, "--reverse", "--check", "-"], cwd=REPO, data=raw)
        after = {str(p.relative_to(grammar)): digest(p.read_bytes()) for p in targets}
        if any(before[p] == after[p] for p in before):
            raise ValueError("patch did not change a declared target")
        patches.append({"path": str(patch), "digest": digest(raw), "before": before, "after": after})
    fixtures = []
    for item in profile.get("corpus_fixtures", []):
        source_fixture = (args.profile.parent / item["source"]).resolve(strict=True)
        target = (grammar / item["target"]).resolve()
        if not source_fixture.is_relative_to(Path(__file__).resolve().parent) or not target.is_relative_to(grammar / "test/corpus"):
            raise ValueError("invalid packet corpus fixture")
        raw = source_fixture.read_bytes()
        with target.open("xb") as stream:
            stream.write(raw)
        fixtures.append({"source": str(source_fixture), "target": item["target"], "digest": digest(raw)})
    for root in roots:
        run("generate-1-" + exports[root.relative_to(grammar).as_posix()], [cli, "generate", "--abi", str(profile["abi"])], root)
    generated = generated_digests(grammar)
    for root in roots:
        run("generate-2-" + exports[root.relative_to(grammar).as_posix()], [cli, "generate", "--abi", str(profile["abi"])], root)
    if generated != generated_digests(grammar):
        raise ValueError("nondeterministic generation")
    run("corpus", [cli, "test", "--rebuild"])
    for root in roots:
        export = exports[root.relative_to(grammar).as_posix()]
        library = logs / (export + extension)
        run("build-" + export, [cli, "build", "--output", library], root)
        libraries[export] = {"path": str(library), "digest": digest(library.read_bytes())}
    binding_metadata = None
    cargo = grammar / "Cargo.toml"
    if cargo.exists():
        before = cargo.read_bytes()
        if b"[workspace]" not in before:
            cargo.write_bytes(before + b"\n[workspace]\n")
        binding_metadata = {"cargo_before": digest(before), "cargo_after": digest(cargo.read_bytes()),
                            "lockfiles": {p: digest((grammar / p).read_bytes()) for p in ("Cargo.lock", "package-lock.json", "go.sum") if (grammar / p).exists()},
                            "scope": "Scratch Cargo workspace isolation only; no upstream dependency lock changes"}
    report = {"format": "entmoot.grammar_baseline_materialization.v1", "grammar": str(grammar), "profile": profile,
              "profile_digest": digest(profile_raw), "source_index_digest": digest(index_path.read_bytes()),
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(), "layout_driver_digest": layout_digest(),
              "source_generated": original, "generated_files": generated, "tracked_generated_match": original == generated,
              "generator": version, "generator_digest": digest(cli.read_bytes()), "parser_cache": env["TREE_SITTER_LIBDIR"],
              "source_libraries": source_libraries, "libraries": libraries, "binding_metadata": binding_metadata,
              "archive_commit": archive_commit, "dependency_proposals": dependencies,
              "npm_lock_override": ({"source": override["source"], "original_digest": override["original_digest"],
                                      "qualified_digest": digest((grammar / "package-lock.json").read_bytes())}
                                     if profile.get("npm_install") and override else None),
              "patches": patches, "corpus_fixtures": fixtures,
              "commands": commands, "passed": True,
              "limits": ["Pinned grammar and explicitly listed proposals/supplements only; no compiler, runtime, binding or production visitor claim"]}
    with receipt.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
