#!/usr/bin/env python3
"""Private-materialize curl and write a bootstrap CompDB (discovery-gate only).

Clones under .tmp/evals/checkouts; does not mutate foreign siblings. Prefer
scripts/prepare_curl_compdb.py for upstream CMake CompDB. See
docs/runbooks/corpus-builds.md.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CHECKOUTS = REPO_ROOT / ".tmp" / "evals" / "checkouts"
DEFAULT_SOURCE = Path(r"G:\GitHub\curl")
COMPDB_INCLUDES = ["include", "lib", "src"]


def _run(cmd: list[str], cwd: Path | None = None) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=str(cwd) if cwd else None, check=True)


def _git_rev(source: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "--short", "HEAD"],
        text=True,
    ).strip()


def _ensure_private_clone(source: Path, dest: Path) -> None:
    if dest.is_dir() and (dest / ".git").exists():
        _run(["git", "-C", str(dest), "fetch", "--all", "--quiet"])
        rev = _git_rev(source)
        _run(["git", "-C", str(dest), "checkout", "-q", rev])
        return
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    _run(["git", "clone", "--local", "--no-hardlinks", str(source), str(dest)])


def _posix_rel(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def _collect_sources(root: Path) -> list[Path]:
    files: list[Path] = []
    for base in (root / "lib", root / "src"):
        if not base.is_dir():
            continue
        for pattern in ("*.c", "*.cc", "*.cpp", "*.cxx"):
            files.extend(base.rglob(pattern))
    return sorted({p.resolve() for p in files})


def write_bootstrap_compdb(root: Path, corpus: str = "curl") -> Path:
    if corpus != "curl":
        raise SystemExit(f"bootstrap materialize is curl-only (got {corpus})")
    include_args = [f"-I{rel}" for rel in COMPDB_INCLUDES if (root / rel).is_dir()]
    entries = []
    for src in _collect_sources(root):
        rel = _posix_rel(src, root)
        entries.append(
            {
                "directory": str(root.resolve()),
                "file": rel,
                "arguments": ["clang", "-c", rel, *include_args],
            }
        )
    if not entries:
        raise SystemExit(f"no C/C++ sources found under {root}")
    out = root / "compile_commands.json"
    out.write_text(json.dumps(entries, indent=2) + "\n", encoding="utf-8")
    (root / ".entmoot_compdb_kind").write_text("bootstrap\n", encoding="utf-8")
    print(f"wrote {out} ({len(entries)} entries; kind=bootstrap)", flush=True)
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument(
        "--skip-clone",
        action="store_true",
        help="Only rewrite CompDB in an existing private curl-* checkout",
    )
    args = parser.parse_args()
    if not args.source.is_dir():
        print(f"source checkout missing: {args.source}", file=sys.stderr)
        return 2

    rev = _git_rev(args.source)
    dest = CHECKOUTS / f"curl-{rev}"
    if args.skip_clone:
        if not dest.is_dir():
            matches = sorted(CHECKOUTS.glob("curl-*")) if CHECKOUTS.exists() else []
            matches = [p for p in matches if not p.name.startswith("curl-cmake")]
            if not matches:
                print(f"no private checkout at {dest}", file=sys.stderr)
                return 2
            dest = matches[-1]
    else:
        _ensure_private_clone(args.source, dest)

    compdb = write_bootstrap_compdb(dest, "curl")
    print()
    print("Next: prefer scripts/prepare_curl_compdb.py for upstream CMake CompDB.")
    print(f"  bootstrap CompDB: {compdb}")
    print(
        "Limitation: bootstrap CompDB is not a build-system compiledb; "
        "closure remains unqualified."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
