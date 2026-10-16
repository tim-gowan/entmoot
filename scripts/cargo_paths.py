"""Resolve evaluation binaries from the same target directory as Cargo."""
from __future__ import annotations

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def debug_directory() -> Path:
    configured = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    return (ROOT / configured).resolve() / "debug"
