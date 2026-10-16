#!/usr/bin/env python3
"""Prepare upstream CMake CompDB for a modern private curl tip.

Build-context prep only (not entmoot-engine / contracts). Clones into
.tmp/evals/checkouts and does not mutate foreign sibling trees. See
docs/runbooks/corpus-builds.md.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CHECKOUTS = REPO_ROOT / ".tmp" / "evals" / "checkouts"
CURL_REMOTE = "https://github.com/curl/curl.git"
KIND_MARKER = ".entmoot_compdb_kind"
SOURCE_REVISION = "SOURCE_REVISION"
COMPDB_COMPOSE = (
    REPO_ROOT / "tools" / "resolver-adapters" / "clang" / "compdb" / "docker-compose.yml"
)
COMPDB_IMAGE = "entmoot-curl-compdb:bookworm"


def _run(cmd: list[str], cwd: Path | None = None, env: dict | None = None) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        check=True,
        env=env,
    )


def _which(name: str) -> str | None:
    return shutil.which(name)


def _git_rev(root: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "--short", "HEAD"],
        text=True,
    ).strip()


def _git_full(root: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        text=True,
    ).strip()


def _write_markers(root: Path, kind: str, extra: str = "") -> None:
    (root / KIND_MARKER).write_text(kind + "\n", encoding="utf-8")
    body = (
        f"kind={kind}\n"
        f"revision={_git_full(root)}\n"
        f"short={_git_rev(root)}\n"
        f"remote={CURL_REMOTE}\n"
        f"{extra}"
    )
    (root / SOURCE_REVISION).write_text(body, encoding="utf-8")


def _ensure_modern_curl(dest: Path, rev: str | None) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.is_dir() and (dest / ".git").exists():
        _run(["git", "-C", str(dest), "fetch", "--depth", "1", "origin", "master"])
        if rev:
            _run(["git", "-C", str(dest), "fetch", "--depth", "1", "origin", rev])
            _run(["git", "-C", str(dest), "checkout", "-q", rev])
        else:
            _run(["git", "-C", str(dest), "checkout", "-q", "FETCH_HEAD"])
            # After shallow fetch of master, reset to origin/master tip.
            try:
                _run(["git", "-C", str(dest), "checkout", "-q", "origin/master"])
            except subprocess.CalledProcessError:
                pass
    else:
        if dest.exists():
            shutil.rmtree(dest)
        cmd = ["git", "clone", "--depth", "1"]
        if rev:
            cmd.extend(["--branch", rev])
        cmd.extend([CURL_REMOTE, str(dest)])
        try:
            _run(cmd)
        except subprocess.CalledProcessError:
            # Branch may be a full SHA — clone default then fetch/checkout.
            if dest.exists():
                shutil.rmtree(dest)
            _run(["git", "clone", "--depth", "1", CURL_REMOTE, str(dest)])
            if rev:
                _run(["git", "-C", str(dest), "fetch", "--depth", "1", "origin", rev])
                _run(["git", "-C", str(dest), "checkout", "-q", rev])

    cmake = dest / "CMakeLists.txt"
    if not cmake.is_file():
        raise SystemExit(f"CMakeLists.txt missing at {dest} (need a modern curl tip)")


def _wsl_path(win_path: Path) -> str:
    # G:\foo -> /mnt/g/foo
    resolved = win_path.resolve()
    s = str(resolved).replace("\\", "/")
    if len(s) >= 2 and s[1] == ":":
        drive = s[0].lower()
        return f"/mnt/{drive}{s[2:]}"
    return s


def _rewrite_wsl_paths_in_compdb(compdb: Path, root: Path) -> None:
    """Map /mnt/<drive>/... CompDB paths back to Windows paths for host produce."""
    text = compdb.read_text(encoding="utf-8")
    # WSL mount paths (/mnt/<drive>/...) -> Windows drive paths
    import re

    def repl(m: re.Match[str]) -> str:
        drive = m.group(1).upper()
        rest = m.group(2).replace("/", "\\")
        return f"{drive}:{rest}".replace("\\", "/")

    rewritten = re.sub(r"/mnt/([a-zA-Z])(/[^\"'\\s]*)", repl, text)
    # Also normalize directory fields to the Windows root when still under root.
    win_root = str(root.resolve()).replace("\\", "/")
    wsl_root = _wsl_path(root)
    rewritten = rewritten.replace(wsl_root, win_root)
    if rewritten != text:
        compdb.write_text(rewritten, encoding="utf-8")
        print(f"rewrote WSL paths in {compdb} for Windows consumers", flush=True)


def _docker_available() -> bool:
    if not _which("docker"):
        return False
    try:
        subprocess.run(
            ["docker", "info"],
            check=True,
            capture_output=True,
            timeout=30,
        )
        return True
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError):
        return False


def _rewrite_docker_paths_in_compdb(compdb: Path, root: Path) -> None:
    """Map container /src/... CompDB paths back to the host private checkout."""
    text = compdb.read_text(encoding="utf-8")
    win_root = str(root.resolve()).replace("\\", "/")
    # Prefer exact /src prefix replacement for directory/file fields.
    rewritten = text.replace('"/src/', f'"{win_root}/')
    rewritten = rewritten.replace("/src/", f"{win_root}/")
    if rewritten != text:
        compdb.write_text(rewritten, encoding="utf-8")
        print(f"rewrote Docker /src paths in {compdb} for host consumers", flush=True)


def _cmake_configure_docker(root: Path, build_dir: Path) -> Path:
    """Run CMake CompDB inside Docker Desktop (preferred on Windows)."""
    if not COMPDB_COMPOSE.is_file():
        raise SystemExit(f"missing compose file: {COMPDB_COMPOSE}")
    if not _docker_available():
        raise SystemExit(
            "Docker Desktop not reachable; install/start Docker or use --wsl / host cmake"
        )
    build_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["ENTMOOT_COMPDB_HOST_SRC"] = str(root.resolve())
    _run(
        [
            "docker",
            "compose",
            "-f",
            str(COMPDB_COMPOSE),
            "build",
        ],
        env=env,
    )
    _run(
        [
            "docker",
            "compose",
            "-f",
            str(COMPDB_COMPOSE),
            "run",
            "--rm",
            "-v",
            f"{root.resolve()}:/src",
            "curl-compdb",
        ],
        env=env,
    )
    compdb = build_dir / "compile_commands.json"
    if not compdb.is_file():
        raise SystemExit(f"Docker CompDB prepare did not emit {compdb}")
    _rewrite_docker_paths_in_compdb(compdb, root)
    root_copy = root / "compile_commands.json"
    shutil.copy2(compdb, root_copy)
    _rewrite_docker_paths_in_compdb(root_copy, root)
    return compdb


def _cmake_configure(root: Path, build_dir: Path, use_wsl: bool) -> Path:
    build_dir.mkdir(parents=True, exist_ok=True)
    if use_wsl:
        wsl_root = _wsl_path(root)
        wsl_build = _wsl_path(build_dir)
        _run(
            [
                "wsl",
                "-e",
                "bash",
                "-lc",
                (
                    f"cmake -S '{wsl_root}' -B '{wsl_build}' "
                    f"-G Ninja "
                    f"-DCMAKE_EXPORT_COMPILE_COMMANDS=ON "
                    f"-DCMAKE_BUILD_TYPE=Debug "
                    f"-DBUILD_TESTING=OFF "
                    f"-DCURL_USE_LIBPSL=OFF "
                    f"-DCURL_DISABLE_TESTS=ON"
                ),
            ]
        )
    else:
        generator = "Ninja" if _which("ninja") else None
        cmd = [
            "cmake",
            "-S",
            str(root),
            "-B",
            str(build_dir),
            "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
            "-DCMAKE_BUILD_TYPE=Debug",
            "-DBUILD_TESTING=OFF",
            "-DCURL_USE_LIBPSL=OFF",
        ]
        if generator:
            cmd[1:1] = ["-G", generator]
        else:
            # Windows without Ninja: NMake may work if VS env is loaded.
            if os.name == "nt":
                cmd[1:1] = ["-G", "NMake Makefiles"]
            else:
                cmd[1:1] = ["-G", "Unix Makefiles"]
        _run(cmd)

    compdb = build_dir / "compile_commands.json"
    if not compdb.is_file():
        raise SystemExit(
            f"CMake did not emit {compdb}; try --wsl or install Ninja"
        )
    if use_wsl and os.name == "nt":
        _rewrite_wsl_paths_in_compdb(compdb, root)
    # Convenience copy at root (also a discovery path).
    root_copy = root / "compile_commands.json"
    shutil.copy2(compdb, root_copy)
    return compdb


def _print_next(root: Path, compdb: Path) -> None:
    print()
    print("Next (PowerShell):")
    print(
        "  # Build/install the clang adapter, then either set ENTMOOT_CLANG_ADAPTER"
    )
    print("  # or ENTMOOT_ADAPTERS_ROOT / install under ~/.entmoot/adapters/")
    print("  # Export LIBCLANG_PATH to the libclang shared library on this host.")
    print(f"  # CompDB checkout: {root}")
    print(f"  # CompDB: {compdb} (kind=upstream unless --bootstrap-only)")
    print()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--rev",
        default=None,
        help="Optional curl git ref (default: origin/master tip)",
    )
    parser.add_argument(
        "--docker",
        action="store_true",
        help="Run CMake+Ninja inside Docker Desktop (preferred on Windows)",
    )
    parser.add_argument(
        "--wsl",
        action="store_true",
        help="Run CMake+Ninja inside WSL (fallback when Docker is unavailable)",
    )
    parser.add_argument(
        "--bootstrap-only",
        action="store_true",
        help="Skip CMake; write bootstrap CompDB for discovery-gate testing",
    )
    parser.add_argument(
        "--dest",
        type=Path,
        default=None,
        help="Override private checkout path",
    )
    parser.add_argument(
        "--in-place",
        action="store_true",
        help="Use --dest as-is (no fetch/checkout); for pinned corpus trees",
    )
    args = parser.parse_args()

    if args.docker and args.wsl:
        raise SystemExit("pass only one of --docker or --wsl")
    if args.in_place and args.dest is None:
        raise SystemExit("--in-place requires --dest")

    # Prefer Docker Desktop on Windows; else WSL when Ninja missing; else host cmake.
    use_docker = bool(args.docker)
    use_wsl = bool(args.wsl)
    if not use_docker and not use_wsl and os.name == "nt" and not _which("ninja"):
        if _docker_available():
            print(
                "note: Ninja missing on Windows PATH; using Docker Desktop for CMake "
                "(pass --docker explicitly to silence, or --wsl / install Ninja)",
                flush=True,
            )
            use_docker = True
        elif _which("wsl"):
            print(
                "note: Ninja missing and Docker unavailable; using WSL for CMake "
                "(pass --wsl explicitly to silence, or install Ninja / start Docker)",
                flush=True,
            )
            use_wsl = True

    # Placeholder dest name until we know short rev.
    staging = CHECKOUTS / "curl-cmake-wip"
    dest = args.dest
    if dest is None:
        _ensure_modern_curl(staging, args.rev)
        short = _git_rev(staging)
        dest = CHECKOUTS / f"curl-cmake-{short}"
        if dest.resolve() != staging.resolve():
            if dest.exists():
                # Refresh existing named checkout instead of WIP rename when present.
                _ensure_modern_curl(dest, args.rev or short)
                if staging.exists() and staging.resolve() != dest.resolve():
                    shutil.rmtree(staging)
            else:
                staging.rename(dest)
    elif args.in_place:
        if not (dest / "CMakeLists.txt").is_file():
            raise SystemExit(f"CMakeLists.txt missing at {dest} (--in-place)")
    else:
        _ensure_modern_curl(dest, args.rev)

    if args.bootstrap_only:
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        from materialize_curl_compdb import write_bootstrap_compdb

        compdb = write_bootstrap_compdb(dest, "curl")
        _write_markers(dest, "bootstrap", "limitation=bootstrap_escape_not_cmake\n")
        _print_next(dest, compdb)
        return 0

    build_dir = dest / "build"
    generator = "docker" if use_docker else ("wsl+cmake" if use_wsl else "cmake")
    try:
        if use_docker:
            compdb = _cmake_configure_docker(dest, build_dir)
        else:
            compdb = _cmake_configure(dest, build_dir, use_wsl=use_wsl)
    except (subprocess.CalledProcessError, SystemExit) as exc:
        print(f"CMake CompDB prepare failed: {exc}", file=sys.stderr)
        print(
            "Retry with: python scripts/prepare_curl_compdb.py --docker\n"
            "  or: python scripts/prepare_curl_compdb.py --wsl",
            file=sys.stderr,
        )
        return 1

    entries = 0
    try:
        entries = len(json.loads(compdb.read_text(encoding="utf-8")))
    except (OSError, json.JSONDecodeError):
        pass

    _write_markers(
        dest,
        "upstream",
        f"generator={generator}\ncompdb={compdb.as_posix()}\nentries={entries}\n"
        f"image={COMPDB_IMAGE if use_docker else ''}\n",
    )
    print(f"upstream CompDB ready: {compdb} ({entries} entries)", flush=True)
    _print_next(dest, compdb)
    return 0


if __name__ == "__main__":
    sys.exit(main())
