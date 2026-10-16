#!/usr/bin/env python3
"""Update all aligned ra_ap_* pins together and report dependency deltas.

Usage:
  python scripts/update_ra_ap_pins.py show
  python scripts/update_ra_ap_pins.py set 0.0.358
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENGINE_TOML = ROOT / "crates" / "entmoot-engine" / "Cargo.toml"
MOD_RS = ROOT / "crates" / "entmoot-engine" / "src" / "native_resolver" / "mod.rs"
CONST_RE = re.compile(r'pub const RA_AP_PIN: &str = "(?P<ver>[^"]+)";')
VERSION_RE = re.compile(r'(version = "=)\d+\.\d+\.\d+(")')


def current_pin() -> str:
    text = MOD_RS.read_text(encoding="utf-8")
    match = CONST_RE.search(text)
    if not match:
        raise SystemExit("RA_AP_PIN not found")
    return match.group("ver")


def show() -> int:
    pin = current_pin()
    print(f"RA_AP_PIN={pin}")
    return 0


def set_pin(version: str) -> int:
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise SystemExit("version must look like 0.0.358")
    before = ENGINE_TOML.read_text(encoding="utf-8")
    # Only rewrite the optional ra_ap dependency block.
    start = before.find("# Pinned, aligned rust-analyzer")
    if start < 0:
        raise SystemExit("ra_ap pin block not found in entmoot-engine Cargo.toml")
    block = before[start:]
    rewritten = VERSION_RE.sub(rf"\g<1>{version}\2", block)
    ENGINE_TOML.write_text(before[:start] + rewritten, encoding="utf-8")
    mod = MOD_RS.read_text(encoding="utf-8")
    MOD_RS.write_text(
        CONST_RE.sub(f'pub const RA_AP_PIN: &str = "{version}";', mod),
        encoding="utf-8",
    )
    print(f"updated RA_AP_PIN -> {version}")
    cmd = [
        "cargo",
        "check",
        "-p",
        "entmoot-engine",
        "--features",
        "resolved-rust-analyzer",
    ]
    print("+", " ".join(cmd), flush=True)
    return subprocess.call(cmd, cwd=ROOT)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("show")
    set_parser = sub.add_parser("set")
    set_parser.add_argument("version")
    args = parser.parse_args(argv)
    if args.cmd == "show":
        return show()
    return set_pin(args.version)


if __name__ == "__main__":
    raise SystemExit(main())
