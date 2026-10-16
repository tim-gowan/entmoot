#!/usr/bin/env python3
"""ENTMOOT_GO_ADAPTER proxy: run the Linux Go sidecar via Docker.

Host Go may be missing, too old, or wrong arch (e.g. windows/386 while the
corpus needs go1.26+ amd64). This proxy:

1. Reads the sidecar JSON request from stdin
2. Bind-mounts ``request.root`` at ``/src`` in the container
3. Rewrites ``root`` to ``/src`` for the container
4. Prefetches modules inside the image entrypoint, then runs the offline sidecar
5. Rewrites absolute ``/src/...`` paths in the response back to the host root

Spin up / down::

    docker compose -f tools/resolver-adapters/go/docker-compose.yml build
    set ENTMOOT_GO_ADAPTER=<repo>/tools/resolver-adapters/go/docker_sidecar_proxy.py
    # ... entmoot-producer go produce ...
    docker compose -f tools/resolver-adapters/go/docker-compose.yml down --rmi local

Requires Docker Desktop (or Engine). Uses ``docker run`` (not ``compose run``)
so Windows ``\\\\?\\`` roots and compose locks cannot stall Safe-mode produce.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
COMPOSE_FILE = HERE / "docker-compose.yml"
IMAGE = os.environ.get("ENTMOOT_GO_DOCKER_IMAGE", "entmoot-go-sidecar:1.26")
CONTAINER_ROOT = "/src"
MODULE_CACHE_VOLUME = os.environ.get(
    "ENTMOOT_GO_DOCKER_MODCACHE_VOLUME", "entmoot-go-modcache"
)


def _host_path(path: str) -> str:
    resolved = str(Path(path).resolve())
    # Engine canonicalize often yields \\?\G:\... which breaks Docker -v parsing
    # ("too many colons"). Strip the extended prefix and normalize separators.
    if resolved.startswith("\\\\?\\"):
        resolved = resolved[4:]
    return resolved.replace("\\", "/")


def _docker_volume(host_root: str) -> str:
    # Docker Desktop on Windows accepts G:/path:/src:ro
    return f"{host_root}:{CONTAINER_ROOT}:ro"


def _rewrite_paths(value: object, host_root: str) -> object:
    host_prefix = host_root.rstrip("/")
    container_prefix = CONTAINER_ROOT.rstrip("/")
    if isinstance(value, str):
        normalized = value.replace("\\", "/")
        if normalized == container_prefix or normalized.startswith(container_prefix + "/"):
            suffix = normalized[len(container_prefix) :].lstrip("/")
            joined = Path(host_prefix) / suffix if suffix else Path(host_prefix)
            # Prefer host-native separators in rewritten absolute inputs.
            return str(joined)
        return value
    if isinstance(value, list):
        return [_rewrite_paths(item, host_root) for item in value]
    if isinstance(value, dict):
        return {key: _rewrite_paths(item, host_root) for key, item in value.items()}
    return value


def _docker_available() -> bool:
    try:
        process = subprocess.run(
            ["docker", "version"],
            capture_output=True,
            text=True,
            check=False,
        )
        return process.returncode == 0
    except OSError:
        return False


def _image_present(image: str) -> bool:
    process = subprocess.run(
        ["docker", "image", "inspect", image],
        capture_output=True,
        text=True,
        check=False,
    )
    return process.returncode == 0


def _ensure_image() -> str | None:
    if _image_present(IMAGE):
        return None
    build = subprocess.run(
        ["docker", "compose", "-f", str(COMPOSE_FILE), "build", "go-sidecar"],
        cwd=str(HERE),
        capture_output=True,
        text=True,
        check=False,
    )
    if build.returncode != 0:
        return f"docker compose build failed: {(build.stderr or build.stdout).strip()}"
    if not _image_present(IMAGE):
        return f"image {IMAGE} missing after compose build"
    return None


def _failure(message: str) -> dict:
    return {
        "build_context": {},
        "compiler_version": "docker_sidecar_proxy",
        "calls": [],
        "inputs": [],
        "diagnostics": [message],
        "invalid_environment": True,
    }


def main() -> int:
    raw = sys.stdin.buffer.read()
    try:
        request = json.loads(raw.decode("utf-8-sig"))
    except json.JSONDecodeError as error:
        print(f"docker_sidecar_proxy: invalid request JSON: {error}", file=sys.stderr)
        return 2

    host_root = request.get("root")
    if not isinstance(host_root, str) or not host_root.strip():
        print("docker_sidecar_proxy: request.root is required", file=sys.stderr)
        return 2
    host_root = _host_path(host_root)
    if not Path(host_root).is_dir():
        print(f"docker_sidecar_proxy: root is not a directory: {host_root}", file=sys.stderr)
        return 2

    if not _docker_available():
        json.dump(
            _failure(
                "docker unavailable; install Docker Desktop or unset "
                "ENTMOOT_GO_ADAPTER docker proxy"
            ),
            sys.stdout,
        )
        return 0

    ensure_error = _ensure_image()
    if ensure_error:
        json.dump(_failure(ensure_error), sys.stdout)
        return 0

    request = dict(request)
    request["root"] = CONTAINER_ROOT
    payload = json.dumps(request, separators=(",", ":")).encode("utf-8")

    prewarm = os.environ.get(
        "ENTMOOT_GO_PREWARM_GOPROXY", "https://proxy.golang.org,direct"
    )
    command = [
        "docker",
        "run",
        "--rm",
        "-i",
        "-v",
        _docker_volume(host_root),
        "-v",
        f"{MODULE_CACHE_VOLUME}:/go/pkg/mod",
        "-e",
        "ENTMOOT_GO_GOPROXY=off",
        "-e",
        "ENTMOOT_GO_GOTOOLCHAIN=local",
        "-e",
        f"ENTMOOT_GO_PREWARM_GOPROXY={prewarm}",
    ]
    # Engine sets ENTMOOT_GO_SINGLE_MODULE=1 for isolated module loads; forward it.
    for key in ("ENTMOOT_GO_SINGLE_MODULE", "ENTMOOT_GO_TAGS", "ENTMOOT_GO_TESTS"):
        value = os.environ.get(key)
        if value:
            command.extend(["-e", f"{key}={value}"])
    command.append(IMAGE)
    process = subprocess.run(
        command,
        cwd=str(HERE),
        input=payload,
        capture_output=True,
        check=False,
    )
    if process.returncode != 0:
        stderr = process.stderr.decode("utf-8", errors="replace").strip()
        json.dump(
            _failure(f"docker run failed ({process.returncode}): {stderr}"),
            sys.stdout,
        )
        return 0

    try:
        response = json.loads(process.stdout.decode("utf-8"))
    except json.JSONDecodeError as error:
        stderr = process.stderr.decode("utf-8", errors="replace").strip()
        json.dump(
            _failure(f"sidecar returned invalid JSON: {error}; stderr={stderr}"),
            sys.stdout,
        )
        return 0

    rewritten = _rewrite_paths(response, host_root)
    json.dump(rewritten, sys.stdout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
