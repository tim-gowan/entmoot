#!/usr/bin/env python3
"""Build the pinned engine batch adapter from a read-only Pyright checkout.

Explicit setup only: init/bind never downloads dependencies or builds tooling.
All npm dependencies and generated code go under --out, never into --source.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve(strict=True)
    revision = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != 'a07bbdeae085f59396acee8739c7d862dd3a1ff1':
        parser.error('unqualified Pyright source revision; expected a07bbdeae085f59396acee8739c7d862dd3a1ff1')
    if subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain'], text=True).strip():
        parser.error('Pyright source checkout must match its clean pin')
    out = args.out.resolve()
    if out == source or source in out.parents:
        parser.error("--out must be outside the upstream checkout")
    package = source / "packages/pyright-internal"
    metadata = json.loads((package / "package.json").read_text())
    if metadata["version"] != "1.1.414":
        parser.error("adapter requires Pyright 1.1.414")
    out.mkdir(parents=True, exist_ok=True)
    deps = dict(metadata["dependencies"])
    deps.update({"esbuild": "0.25.10", "@noble/hashes": "1.8.0"})
    (out / "package.json").write_text(json.dumps({"private": True, "dependencies": deps}))
    shutil.copyfile(Path(__file__).with_name('pyright_program.package-lock.json'), out / 'package-lock.json')
    npm = shutil.which("npm.cmd" if os.name == "nt" else "npm")
    subprocess.run([npm, "ci", "--ignore-scripts", "--no-audit", "--no-fund"], cwd=out, check=True)
    entry = Path(__file__).resolve().parents[1] / "crates/entmoot-native/src/native_resolver/producers/pyright_program.ts"
    build = {
        "entryPoints": [str(entry)], "outfile": str(out / "entmoot-pyright.cjs"),
        "bundle": True, "platform": "node", "format": "cjs", "target": "node20",
        "nodePaths": [str(out / "node_modules")],
        "alias": {"@pyright": str(package / "src")},
        "define": {"ENTMOOT_PYRIGHT_TYPESHED": json.dumps(str(package / "typeshed-fallback"))},
    }
    (out / "build.cjs").write_text("require('esbuild').buildSync(" + json.dumps(build) + ");\n")
    subprocess.run(["node", "build.cjs"], cwd=out, check=True)
    (out / "source.json").write_text(json.dumps({"version": metadata["version"], "revision": revision}))
    print(out / "entmoot-pyright.cjs")


if __name__ == "__main__":
    main()
