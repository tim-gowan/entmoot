#!/usr/bin/env python3
"""Prepare upstream CompDB for a private OpenSSL tip (Configure + bear).

OpenSSL tip has no root CMakeLists; CompDB comes from Bear wrapping `make`.
Build-context prep only. See docs/runbooks/corpus-builds.md.
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
REMOTE = "https://github.com/openssl/openssl.git"
KIND_MARKER = ".entmoot_compdb_kind"
SOURCE_REVISION = "SOURCE_REVISION"


def _run(cmd: list[str], cwd: Path | None = None) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=str(cwd) if cwd else None, check=True)


def _which(name: str) -> str | None:
    return shutil.which(name)


def _git_rev(root: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "--short", "HEAD"], text=True
    ).strip()


def _git_full(root: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
    ).strip()


def _wsl_path(win_path: Path) -> str:
    resolved = win_path.resolve()
    s = str(resolved).replace("\\", "/")
    if len(s) >= 2 and s[1] == ":":
        return f"/mnt/{s[0].lower()}{s[2:]}"
    return s


def _write_markers(root: Path, kind: str, extra: str = "") -> None:
    (root / KIND_MARKER).write_text(kind + "\n", encoding="utf-8")
    (root / SOURCE_REVISION).write_text(
        f"kind={kind}\nrevision={_git_full(root)}\nshort={_git_rev(root)}\n"
        f"remote={REMOTE}\n{extra}",
        encoding="utf-8",
    )


def _ensure_clone(dest: Path, rev: str | None) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.is_dir() and (dest / ".git").exists():
        _run(["git", "-C", str(dest), "fetch", "--depth", "1", "origin", "master"])
        try:
            _run(["git", "-C", str(dest), "checkout", "-q", "origin/master"])
        except subprocess.CalledProcessError:
            _run(["git", "-C", str(dest), "checkout", "-q", "FETCH_HEAD"])
        if rev:
            _run(["git", "-C", str(dest), "fetch", "--depth", "1", "origin", rev])
            _run(["git", "-C", str(dest), "checkout", "-q", rev])
        return
    if dest.exists():
        shutil.rmtree(dest)
    _run(["git", "clone", "--depth", "1", REMOTE, str(dest)])
    if rev:
        _run(["git", "-C", str(dest), "fetch", "--depth", "1", "origin", rev])
        _run(["git", "-C", str(dest), "checkout", "-q", rev])


def _rewrite_wsl_paths(compdb: Path, root: Path) -> None:
    import re

    text = compdb.read_text(encoding="utf-8")

    def repl(m: re.Match[str]) -> str:
        return f"{m.group(1).upper()}:{m.group(2)}"

    rewritten = re.sub(r"/mnt/([a-zA-Z])(/[^\"'\\s]*)", repl, text)
    wsl_root = _wsl_path(root)
    win_root = str(root.resolve()).replace("\\", "/")
    rewritten = rewritten.replace(wsl_root, win_root)
    if rewritten != text:
        compdb.write_text(rewritten, encoding="utf-8")
        print(f"rewrote WSL paths in {compdb}", flush=True)


def _configure_bear_wsl(root: Path) -> Path:
    """Configure + bear-make under WSL; CompDB lands at repo root."""
    wsl_root = _wsl_path(root)
    # Bounded build: libraries only (skip tests/apps fuzz) to keep dogfood shorter.
    script = f"""
set -euo pipefail
cd '{wsl_root}'
# Windows checkouts often have CRLF shebangs; invoke perl/make explicitly.
command -v bear >/dev/null || (sudo apt-get update -qq && sudo apt-get install -y -qq bear)
command -v perl >/dev/null || (sudo apt-get update -qq && sudo apt-get install -y -qq perl)
perl Configure linux-x86_64 no-tests no-apps --prefix=/tmp/openssl-entmoot-prefix
bear -- make -j"$(nproc)" build_libs
test -f compile_commands.json
"""
    _run(["wsl", "-e", "bash", "-lc", script])
    compdb = root / "compile_commands.json"
    if not compdb.is_file():
        raise SystemExit(f"bear did not emit {compdb}")
    if os.name == "nt":
        _rewrite_wsl_paths(compdb, root)
    return compdb


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rev", default=None)
    parser.add_argument("--dest", type=Path, default=None)
    args = parser.parse_args()

    if os.name == "nt" and not _which("wsl"):
        print("WSL required on Windows for OpenSSL CompDB prepare", file=sys.stderr)
        return 2

    staging = CHECKOUTS / "openssl-wip"
    dest = args.dest
    if dest is None:
        _ensure_clone(staging, args.rev)
        short = _git_rev(staging)
        dest = CHECKOUTS / f"openssl-{short}"
        if dest.resolve() != staging.resolve():
            if dest.exists():
                _ensure_clone(dest, args.rev)
                if staging.exists() and staging.resolve() != dest.resolve():
                    try:
                        shutil.rmtree(staging)
                    except OSError as exc:
                        print(f"warning: could not remove staging {staging}: {exc}", file=sys.stderr)
            else:
                staging.rename(dest)
    else:
        _ensure_clone(dest, args.rev)

    if not (dest / "Configure").is_file():
        raise SystemExit(f"OpenSSL Configure missing at {dest}")

    try:
        compdb = _configure_bear_wsl(dest)
    except (subprocess.CalledProcessError, SystemExit) as exc:
        print(f"OpenSSL CompDB prepare failed: {exc}", file=sys.stderr)
        return 1

    entries = 0
    try:
        entries = len(json.loads(compdb.read_text(encoding="utf-8")))
    except (OSError, json.JSONDecodeError):
        pass
    _write_markers(
        dest,
        "upstream",
        f"generator=bear_configure\ncompdb={compdb.as_posix()}\nentries={entries}\n",
    )
    print(f"upstream CompDB ready: {compdb} ({entries} entries)", flush=True)
    print(f"  # --root {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
