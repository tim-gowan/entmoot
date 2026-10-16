"""Rehearse extracted packages without depending on unpublished registry siblings.

Only extracted crate tarballs are patched into the isolated rehearsal workspace.
Builds use the release lockfile. No source checkout paths occur in its manifest.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]

def run(args, cwd=ROOT):
    subprocess.run([str(a) for a in args], cwd=cwd, check=True)

def read(binary, args, cwd, env=None):
    return json.loads(subprocess.check_output([str(binary), *args], cwd=cwd, env=env))

def fingerprint(binary, directory):
    directory.mkdir()
    (directory / "app.py").write_text("def work(object):\n    object.toString()\n", encoding="utf-8")
    found = read(binary, ["-C", str(directory), "find", "work"], directory)
    assert found["result_bounds"]["total_matches"] == 1, found
    result = read(binary, ["-C", str(directory), "boundaries", "--callee", "*.toString()"], directory)
    assert result["result_bounds"]["total_matches"] == 1, result
    publication = json.loads((directory / ".entmoot/current.json").read_text(encoding="utf-8"))
    graph = json.loads(Path(publication["current"]["graph"]).read_text(encoding="utf-8"))
    return graph["implementation"]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--producer", action="store_true", help="also qualify the packaged native CLI/helper")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    args = parser.parse_args()
    target = args.target_dir.resolve()
    features = ["--features", "entmoot/producer"] if args.producer else []
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    scratch = ROOT / ".tmp/package-smoke"
    scratch.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix="extracted-", dir=scratch))
    suffix = ".exe" if os.name == "nt" else ""
    run(["cargo", "build", "--locked", "--target-dir", target, "-p", "entmoot", "-p", "entmoot-mcp", *features])
    baseline = fingerprint(target / "debug" / ("entmoot" + suffix), stage / "baseline")
    run(["cargo", "package", "--workspace", "--locked", "--target-dir", target, "--allow-dirty", "--no-verify"])
    names = ["entmoot-contracts", "entmoot-core", "entmoot-engine", "entmoot", "entmoot-mcp"]
    for name in names:
        archive = target / "package" / f"{name}-{version}.crate"
        assert archive.stat().st_size < 10 * 1024 * 1024, archive
        with tarfile.open(archive) as package:
            package.extractall(stage, filter="data")
    members = [f"{n}-{version}" for n in names]
    manifest = "[workspace]\nresolver = '2'\nmembers = " + json.dumps(members) + "\n[patch.crates-io]\n"
    manifest += "".join(f'{name} = {{ path = "{name}-{version}" }}\n' for name in names)
    (stage / "Cargo.toml").write_text(manifest, encoding="utf-8")
    shutil.copyfile(ROOT / "Cargo.lock", stage / "Cargo.lock")
    # Reuse third-party compilation products, while Cargo recompiles the extracted sources.
    run(["cargo", "build", "--locked", "--target-dir", target, "-p", "entmoot", "-p", "entmoot-mcp", *features], stage)
    actual = fingerprint(target / "debug" / ("entmoot" + suffix), stage / "installed")
    assert actual == baseline, (actual, baseline)
    tools = read(target / "debug" / ("entmoot-mcp" + suffix), ["--tools-schema"], stage)
    assert {"boundaries", "find", "read_source", "status"}.issubset({t["name"] for t in tools})
    if args.producer:
        native = stage / "native"
        native.mkdir()
        (native / "app.js").write_text("function leaf() { return 1; }\nfunction caller() { return leaf(); }\n", encoding="utf-8")
        # Exercise the packaged in-process Oxc producer independently of a host's
        # explicit TypeScript Program override (qualified in its own native lane).
        native_env = os.environ.copy()
        native_env.pop("ENTMOOT_TYPESCRIPT_ADAPTER", None)
        result = read(target / "debug" / ("entmoot" + suffix), ["-C", str(native), "bind"], native, env=native_env)
        assert any((s.get("attachment") or {}).get("resolver_targets", 0) > 0 for s in result["stages"]), result
        helper = read(target / "debug" / ("entmoot-producer" + suffix), ["fingerprint"], stage)
        assert helper == actual, (helper, actual)
    receipt = {"version": version, "implementation": actual, "packages": names, "producer": args.producer, "extracted": str(stage), "status": "passed"}
    (scratch / ("receipt-producer.json" if args.producer else "receipt.json")).write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print("Extracted package CLI/MCP smoke and fingerprint equality passed")

if __name__ == "__main__":
    main()
