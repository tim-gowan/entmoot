"""Offline release structure, ownership and documentation checks."""
import argparse
from pathlib import Path
import subprocess
import sys
import tomllib
import re
import shlex
import json

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from scripts.pre_commit_check import check_crate_boundaries, check_local_markdown_links, check_test_ownership_registry

def check_test_command(argv, targets):
    packages = [argv[i + 1] for i, value in enumerate(argv[:-1]) if value in ("-p", "--package")]
    if "--workspace" in argv:
        packages = list(targets)
    excluded = {argv[i + 1] for i, value in enumerate(argv[:-1]) if value == "--exclude"}
    packages = [package for package in packages if package not in excluded]
    tests = [argv[i + 1] for i, value in enumerate(argv[:-1]) if value == "--test"]
    for test in tests:
        assert any(test in targets[p] for p in packages), (packages, test)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag")
    args = parser.parse_args()
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    version = workspace["package"]["version"]
    if args.tag and args.tag != "v" + version:
        raise SystemExit("Release tag does not match workspace package version")
    subprocess.run([sys.executable, str(ROOT / "scripts/prepare_release_inputs.py"), "--check"], check=True)
    for member in workspace["members"]:
        directory = ROOT / member
        manifest = tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8"))
        assert manifest["package"].get("publish") is not False, member
        for filename in ("README.md", "LICENSE-MIT", "LICENSE-APACHE"):
            assert (directory / filename).is_file(), (member, filename)
        for name, spec in manifest.get("dependencies", {}).items():
            if name.startswith("entmoot-"):
                assert spec["version"] == "=" + version, name
    for check in (check_crate_boundaries, check_local_markdown_links, check_test_ownership_registry):
        if check():
            raise SystemExit(1)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT))
    targets = {p["name"]: {t["name"] for t in p["targets"] if "test" in t["kind"]} for p in metadata["packages"]}
    for workflow in (ROOT / ".github/workflows").glob("*.yml"):
        for line in workflow.read_text(encoding="utf-8").splitlines():
            if "uses:" in line:
                action = line.split("uses:", 1)[1].split("#", 1)[0].strip()
                assert action.startswith("./") or re.fullmatch(r"[^@]+@[0-9a-f]{40}", action), (workflow, action)
            command = re.search(r"\bcargo test (.*)", line)
            if command:
                argv = shlex.split(command.group(1))
                check_test_command(argv, targets)

if __name__ == "__main__":
    main()
