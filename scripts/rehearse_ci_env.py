#!/usr/bin/env python3
"""Rehearse Linux CI environment assembly with the large GHA-like Docker image.

Requires Docker and nektos/act on PATH. Pulls
``ghcr.io/catthehacker/ubuntu:full-latest`` and runs, in order:

1. ci.yml jobs: repo, policy, structural (ubuntu matrix cell)
2. docs.yml build (workflow_dispatch), using the structural binary artifact
3. native.yml resolved (ubuntu matrix cell) — full job

Logs land under ``.tmp/ci-env-rehearsal/``. Cache hits never skip tests.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

IMAGE = "ghcr.io/catthehacker/ubuntu:full-latest"
REPO_ROOT = Path(__file__).resolve().parents[1]
LOG_ROOT = REPO_ROOT / ".tmp" / "ci-env-rehearsal"
ARTIFACT_DIR = LOG_ROOT / "artifacts"


def _run(cmd: list[str], log_path: Path, env: dict[str, str] | None = None) -> None:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    print(f"+ {' '.join(cmd)}", flush=True)
    with log_path.open("w", encoding="utf-8") as log:
        log.write(f"+ {' '.join(cmd)}\n")
        log.flush()
        proc = subprocess.run(
            cmd,
            cwd=REPO_ROOT,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            check=False,
        )
    if proc.returncode != 0:
        print(f"FAILED ({proc.returncode}); see {log_path}", flush=True)
        # Tail for the console.
        try:
            text = log_path.read_text(encoding="utf-8", errors="replace")
            print("\n".join(text.splitlines()[-80:]), flush=True)
        except OSError:
            pass
        raise SystemExit(proc.returncode)


def _require_tools() -> str:
    docker = shutil.which("docker")
    act = shutil.which("act")
    if not docker:
        raise SystemExit("docker not found on PATH")
    if not act:
        raise SystemExit("act (nektos/act) not found on PATH")
    return act


def _pull_image() -> None:
    _run(["docker", "pull", IMAGE], LOG_ROOT / "00-docker-pull.log")


def _act_base(act: str) -> list[str]:
    return [
        act,
        "--pull=false",
        f"--platform=ubuntu-latest={IMAGE}",
        f"--artifact-server-path={ARTIFACT_DIR}",
        "--env",
        "PYTHONUTF8=1",
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--skip-pull",
        action="store_true",
        help="Skip docker pull (image must already exist locally)",
    )
    parser.add_argument(
        "--from-step",
        choices=["pull", "repo", "policy", "structural", "docs", "native"],
        default="pull",
        help="Resume from this stage (inclusive)",
    )
    args = parser.parse_args()

    act = _require_tools()
    LOG_ROOT.mkdir(parents=True, exist_ok=True)
    ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)

    stages = ["pull", "repo", "policy", "structural", "docs", "native"]
    start = stages.index(args.from_step)
    env = os.environ.copy()
    # Prefer a writable cargo home inside the workspace scratch for act bind mounts.
    env.setdefault("CARGO_HOME", str(LOG_ROOT / "cargo-home"))

    if start <= 0 and not args.skip_pull:
        _pull_image()
    elif args.skip_pull:
        print(f"skipping pull; expecting local image {IMAGE}", flush=True)

    sha = (
        subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT)
        .decode()
        .strip()
    )
    bin_artifact = f"entmoot-bin-test-Linux-{sha}"

    if start <= stages.index("repo"):
        _run(
            _act_base(act) + ["-W", ".github/workflows/ci.yml", "-j", "repo"],
            LOG_ROOT / "01-repo.log",
            env=env,
        )
    if start <= stages.index("policy"):
        _run(
            _act_base(act) + ["-W", ".github/workflows/ci.yml", "-j", "policy"],
            LOG_ROOT / "02-policy.log",
            env=env,
        )
    if start <= stages.index("structural"):
        _run(
            _act_base(act)
            + [
                "-W",
                ".github/workflows/ci.yml",
                "-j",
                "structural",
                "--matrix",
                "os:ubuntu-latest",
            ],
            LOG_ROOT / "03-structural.log",
            env=env,
        )
    if start <= stages.index("docs"):
        event_path = LOG_ROOT / "docs-event.json"
        event_path.write_text(
            (
                "{\n"
                '  "inputs": {\n'
                f'    "binaries_artifact": "{bin_artifact}"\n'
                "  }\n"
                "}\n"
            ),
            encoding="utf-8",
        )
        _run(
            _act_base(act)
            + [
                "workflow_dispatch",
                "-W",
                ".github/workflows/docs.yml",
                "-j",
                "build",
                "-e",
                str(event_path),
            ],
            LOG_ROOT / "04-docs.log",
            env=env,
        )
    if start <= stages.index("native"):
        _run(
            _act_base(act)
            + [
                "-W",
                ".github/workflows/native.yml",
                "-j",
                "resolved",
                "--matrix",
                "os:ubuntu-latest",
            ],
            LOG_ROOT / "05-native.log",
            env=env,
        )

    print("rehearsal passed", flush=True)
    print(f"logs: {LOG_ROOT}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
