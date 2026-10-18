"""Rehearse extracted packages without depending on unpublished registry siblings.

Only extracted crate tarballs are patched into the isolated rehearsal workspace.
Builds use the release lockfile. No source checkout paths occur in its manifest.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]

def rehearsal_manifest(names, version, source):
    """Use extracted siblings and the checkout's compilation profile settings."""
    members = [f"{name}-{version}" for name in names]
    manifest = "[workspace]\nresolver = '2'\nmembers = " + json.dumps(members) + "\n[patch.crates-io]\n"
    manifest += "".join(f'{name} = {{ path = "{name}-{version}" }}\n' for name in names)
    # Copy each profile table verbatim, including future per-package overrides.
    # Other workspace tables can refer to checkout-only paths and stay excluded.
    profiles = re.findall(r"(?ms)^\[profile\.[^\n]+\]\s*\n.*?(?=^\[|\Z)", source)
    return manifest + "\n" + "\n".join(profiles)

def run(args, cwd=ROOT, env=None):
    subprocess.run([str(a) for a in args], cwd=cwd, env=env, check=True)

def read(binary, args, cwd, env=None):
    return json.loads(subprocess.check_output([str(binary), *args], cwd=cwd, env=env))

def fixture_repository(directory):
    """Keep root discovery inside the fixture, even below the source checkout."""
    directory.mkdir()
    env = {key: value for key, value in os.environ.items()
           if not key.upper().startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    run(["git", "init", "--quiet", directory], env=env)
    return env

def fingerprint(binary, directory):
    env = fixture_repository(directory)
    # Keep durable state outside the analyzed source tree.
    data_dir = directory.parent / f"{directory.name}-data"
    data_dir.mkdir(exist_ok=True)
    (directory / "app.py").write_text(
        "def entmoot_smoke_fn(object):\n    object.toString()\n", encoding="utf-8"
    )
    common = ["--data-dir", str(data_dir), "-C", str(directory)]
    found = read(binary, [*common, "find", "entmoot_smoke_fn"], directory, env=env)
    assert found["result_bounds"]["total_matches"] == 1, found
    result = read(binary, [*common, "boundaries", "--callee", "*.toString()"], directory, env=env)
    assert result["result_bounds"]["total_matches"] == 1, result
    currents = list(data_dir.glob("workspaces/*/store/current.json"))
    assert len(currents) == 1, currents
    publication = json.loads(currents[0].read_text(encoding="utf-8"))
    graph = json.loads(Path(publication["current"]["graph"]).read_text(encoding="utf-8"))
    return graph["implementation"]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--producer", action="store_true", help="also qualify the packaged native CLI/helper")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--profile", choices=("dev", "test", "release"), default="dev",
                        help="Cargo build profile; use test to reuse CI's tested dependencies")
    parser.add_argument("--baseline-bin-dir", type=Path,
                        help="reuse matching checkout CLI/MCP binaries; extracted packages still build independently")
    args = parser.parse_args()
    target = args.target_dir.resolve()
    binary_dir = target / ("release" if args.profile == "release" else "debug")
    features = ["--features", "entmoot/producer"] if args.producer else []
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    scratch = ROOT / ".tmp/package-smoke"
    scratch.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix="extracted-", dir=scratch))
    suffix = ".exe" if os.name == "nt" else ""
    baseline_bin = args.baseline_bin_dir.resolve() if args.baseline_bin_dir else binary_dir
    build = ["cargo", "build", "--locked", "--profile", args.profile, "--target-dir", target,
             "-p", "entmoot", "-p", "entmoot-mcp", *features]
    if args.baseline_bin_dir is None:
        run(build)
    baseline = fingerprint(baseline_bin / ("entmoot" + suffix), stage / "baseline")
    baseline_tools = read(baseline_bin / ("entmoot-mcp" + suffix), ["--tools-schema"], stage)
    run(["cargo", "package", "--workspace", "--locked", "--target-dir", target, "--allow-dirty", "--no-verify"])
    names = ["entmoot-contracts", "entmoot-core", "entmoot-engine", "entmoot", "entmoot-mcp"]
    for name in names:
        archive = target / "package" / f"{name}-{version}.crate"
        assert archive.stat().st_size < 10 * 1024 * 1024, archive
        with tarfile.open(archive) as package:
            package.extractall(stage, filter="data")
    manifest = rehearsal_manifest(names, version, (ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    (stage / "Cargo.toml").write_text(manifest, encoding="utf-8")
    shutil.copyfile(ROOT / "Cargo.lock", stage / "Cargo.lock")
    # Reuse third-party compilation products, while Cargo recompiles the extracted sources.
    run(build, stage)
    actual = fingerprint(binary_dir / ("entmoot" + suffix), stage / "installed")
    assert actual == baseline, (actual, baseline)
    tools = read(binary_dir / ("entmoot-mcp" + suffix), ["--tools-schema"], stage)
    assert tools == baseline_tools, "packaged MCP schema differs from checkout"
    assert {"boundaries", "find", "read_source", "status"}.issubset({t["name"] for t in tools})
    if args.producer:
        native = stage / "native"
        native_env = fixture_repository(native)
        (native / "app.js").write_text("function leaf() { return 1; }\nfunction caller() { return leaf(); }\n", encoding="utf-8")
        # Exercise the packaged in-process Oxc producer independently of a host's
        # explicit TypeScript Program override (qualified in its own native lane).
        native_env.pop("ENTMOOT_TYPESCRIPT_ADAPTER", None)
        native_data = stage / "native-data"
        native_data.mkdir(exist_ok=True)
        result = read(
            binary_dir / ("entmoot" + suffix),
            ["--data-dir", str(native_data), "-C", str(native), "bind"],
            native,
            env=native_env,
        )
        assert any((s.get("attachment") or {}).get("resolver_targets", 0) > 0 for s in result["stages"]), result
        helper = read(binary_dir / ("entmoot-producer" + suffix), ["fingerprint"], stage)
        assert helper == actual, (helper, actual)
    receipt = {"version": version, "implementation": actual, "packages": names, "producer": args.producer, "profile": args.profile, "extracted": str(stage), "status": "passed"}
    (scratch / ("receipt-producer.json" if args.producer else "receipt.json")).write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print("Extracted package CLI/MCP smoke and fingerprint equality passed")

if __name__ == "__main__":
    main()
