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
NATIVE_TOML = ROOT / "crates" / "entmoot-native" / "Cargo.toml"
MOD_RS = ROOT / "crates" / "entmoot-contracts" / "src" / "binding_evidence.rs"
CONST_RE = re.compile(r'pub const RUST_ADAPTER_PIN: &str = "(?P<ver>[^"]+)";')


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
    before = NATIVE_TOML.read_text(encoding="utf-8")
    pattern = re.compile(r'(?m)^(ra_ap_[^=\n]+ = \{[^\n]*version = "=)[^"]+(")')
    rewritten, count = pattern.subn(lambda match: match[1] + version + match[2], before)
    if count != 9:
        raise SystemExit(f"expected 9 aligned rust-analyzer dependencies, found {count}")
    NATIVE_TOML.write_text(rewritten, encoding="utf-8")
    mod = MOD_RS.read_text(encoding="utf-8")
    MOD_RS.write_text(
        CONST_RE.sub(f'pub const RUST_ADAPTER_PIN: &str = "{version}";', mod),
        encoding="utf-8",
    )
    print(f"updated RA_AP_PIN -> {version}")
    cmd = [
        "cargo",
        "check",
        "-p",
        "entmoot-native",
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
