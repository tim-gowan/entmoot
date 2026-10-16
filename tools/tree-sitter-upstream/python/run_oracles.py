"""Execute independent Python compiler/runtime controls; never install a tool.

The driver uses workstation Python + blake3. Each oracle runs under the explicit,
isolated target interpreter. Structural and native-resolver qualification are
reported separately and cannot be promoted by a compiler pass.
"""
from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

import blake3

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]

NEGATIVES = {
    "lazy-function": "def f():\n    lazy import json\n",
    "lazy-class": "class C:\n    lazy import json\n",
    "lazy-try": "try:\n    lazy import json\nexcept ImportError:\n    pass\n",
    "lazy-star": "lazy from math import *\n",
    "lazy-future": "lazy from __future__ import annotations\n",
    "list-double-unpack": "[**d for d in [{}]]\n",
    "generator-double-unpack": "(**d for d in [{}])\n",
}

CHILD = '''import json, sys
request = json.loads(sys.stdin.read())
source = bytes.fromhex(request["source_hex"])
try:
    code = compile(source, request["id"], "exec")
except SyntaxError as error:
    print(json.dumps({"outcome": "syntax_error", "message": str(error)}))
else:
    try:
        exec(code, {"__name__": "__main__"})
    except Exception as error:
        print(json.dumps({"outcome": "runtime_error", "message": repr(error)}))
    else:
        print(json.dumps({"outcome": "passed"}))
'''


def digest(data: bytes) -> str:
    return "blake3:" + blake3.blake3(data).hexdigest()


def run(interpreter: Path) -> dict:
    interpreter = interpreter.resolve(strict=True)
    pin = subprocess.run(
        [str(interpreter), "-I", "-c", "import json,sys; print(json.dumps({'version':sys.version,'version_info':list(sys.version_info[:3]),'executable':sys.executable}))"],
        check=True, capture_output=True, text=True, timeout=20,
    )
    target = json.loads(pin.stdout)
    if target["version_info"] != [3, 15, 0]:
        raise ValueError("These October qualification oracles require Python 3.15.0 exactly")
    tool_files = {
        path.name: digest(path.read_bytes())
        for path in sorted(interpreter.parent.iterdir())
        if path.is_file() and path.suffix.lower() in (".exe", ".dll", ".pyd", ".zip", "._pth")
    }
    cases = []
    for group in ("lazy-imports", "comprehension-unpacking"):
        path = ROOT / group / "fixtures/positive.py"
        cases.append((group, path.read_bytes(), "passed", path.relative_to(REPO).as_posix()))
    cases.extend((name, source.encode(), "syntax_error", None) for name, source in NEGATIVES.items())
    cases.extend([
        ("historical-identifiers", b"lazy = 7\nfrozendict = lambda **kw: kw\nsentinel = lambda name: name\nassert frozendict(a=lazy) == {'a': 7}\nassert sentinel('local') == 'local'\n", "passed", None),
        ("encoding-cookie", b"# coding: latin-1\nvalue = 'caf\xe9'\nassert len(value) == 4\n", "passed", None),
        ("utf8-default", b"import sys\nassert sys.flags.utf8_mode == 1\n", "passed", None),
    ])
    rows = []
    for name, source, expected, path in cases:
        completed = subprocess.run(
            [str(interpreter), "-I", "-c", CHILD],
            input=json.dumps({"id": name, "source_hex": source.hex()}),
            capture_output=True, text=True, timeout=30,
        )
        if completed.returncode:
            result = {"outcome": "process_error", "stderr": completed.stderr}
        else:
            result = json.loads(completed.stdout)
        rows.append({
            "id": name, "source_path": path, "source_digest": digest(source),
            # Retain inline source bytes, not only their digest.
            "source_hex": source.hex() if path is None else None,
            "expected": expected, "compiler_runtime": result,
            "passed": result["outcome"] == expected,
            "structural": "not_run", "native_target_mode": "not_run",
        })
    return {
        "format": "entmoot.grammar_oracle.v1", "release_cutoff": "2026-10-10",
        "driver_digest": digest(Path(__file__).read_bytes()),
        "interpreter": target, "interpreter_digest": digest(interpreter.read_bytes()),
        "interpreter_top_level_files": tool_files,
        "platform": "windows" if interpreter.suffix.lower() == ".exe" else "host",
        "historical_runtime_matrix": "not_run", "cases": rows,
        "passed": all(row["passed"] for row in rows),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    receipt = run(args.python)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    # A receipt is immutable; a rerun needs a fresh output path.
    with args.out.open("x", encoding="utf-8") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"receipt": str(args.out), "passed": receipt["passed"], "cases": len(receipt["cases"])}))
    raise SystemExit(0 if receipt["passed"] else 1)


if __name__ == "__main__":
    main()
