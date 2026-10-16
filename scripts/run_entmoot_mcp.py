#!/usr/bin/env python3
"""Launch the local entmoot-mcp binary for Cursor stdio MCP wiring.

Resolves the workspace debug binary (Windows .exe vs Unix), chdirs to the repo
root (Cursor mcp.json has no cwd field), and runs the binary with forwarded args
on inherited stdio. Exits non-zero with a build hint when the binary is missing.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def binary_path() -> Path:
    name = "entmoot-mcp.exe" if sys.platform == "win32" else "entmoot-mcp"
    return REPO_ROOT / "target" / "debug" / name


def main() -> int:
    binary = binary_path()
    if not binary.is_file():
        print(
            f"entmoot-mcp not found at {binary}\n"
            "Build with: cargo build -p entmoot --features producer -p entmoot-mcp",
            file=sys.stderr,
        )
        return 1
    os.chdir(REPO_ROOT)
    # Inherit stdin/stdout so Cursor speaks MCP to the binary; keep stderr for
    # diagnostics. subprocess (not execv) is the portable Windows path.
    return subprocess.call([str(binary), *sys.argv[1:]])


if __name__ == "__main__":
    raise SystemExit(main())
