"""Independent Python 3.15/3.13 and CST controls for explicitly positive patterns.

Both interpreters and grammar libraries are explicit. Expected outcomes, node
kinds and source markers are authored here; no parser output generates them.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from grammar_runtime import digest, endpoint, load_language, nodes, runtime_digest, shape
import tree_sitter


def cases():
    for literal, kind in [("+0", "integer"), ("+0.0", "float"), ("+0j", "integer"),
                          ("+1", "integer"), ("+1.5", "float"), ("+1j", "integer"),
                          ("+0.25+1.75j", "complex_pattern"), ("+1e1000", "float")]:
        yield literal, True, False, True, kind, literal
    yield "{+1: [+2, item]}", True, False, True, "dict_pattern", "{1: [2, 'nested']}"
    yield "[+1, item]", True, False, True, "list_pattern", "[1, 'nested']"
    for literal, kind in [("-1", "integer"), ("-0.25-1.75j", "complex_pattern"), ("0", "integer")]:
        yield literal, True, True, True, kind, literal
    for literal in ("++1", "+-1", "+name", "+True", '+"value"'):
        yield literal, False, False, False, None, None
    # Numeric real/imaginary restrictions are compiler checks. The old grammar
    # already accepts analogous minus-prefixed forms; do not add type checking.
    for literal in ("+1+2", "+1j+2j", "-1+2", "-1j+2j"):
        yield literal, False, False, True, "complex_pattern", None


CHECK = """import json,sys
source=bytes.fromhex(sys.stdin.read())
try: code=compile(source,'positive-pattern-control','exec')
except SyntaxError as error: result={'accepted':False,'reason':str(error),'runtime':'not_run'}
else:
 exec(code,{})
 result={'accepted':True,'runtime':'passed'}
print(json.dumps(result))
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--python", type=Path, required=True)
    ap.add_argument("--historical-python", type=Path, required=True)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    output = args.out.resolve()
    if output.exists():
        raise ValueError("use a fresh receipt path")
    interpreters = [p.resolve(strict=True) for p in (args.python, args.historical_python)]
    versions = [subprocess.check_output([str(p), "-I", "-c", "import sys;print(sys.version)"], text=True).strip()
                for p in interpreters]
    if not versions[0].startswith("3.15.0 ") or not versions[1].startswith("3.13."):
        raise ValueError("oracles require actual Python 3.15.0 and 3.13 interpreters")
    libraries = [p.resolve(strict=True) for p in (args.baseline, args.candidate)]
    handles = [load_language(p, "python") for p in libraries]
    rows = []
    for pattern, new_accepts, old_accepts, parses, kind, probe in cases():
        source = (f"# π\r\ndef select(value):\r\n    match value:\r\n        case {pattern}:\r\n"
                  "            return True\r\n        case _:\r\n            return False\r\n"
                  "def after():\r\n    return 17\r\n")
        if probe is not None:
            source += f"assert select({probe}) is True\r\nassert select(object()) is False\r\nassert after() == 17\r\n"
        raw = source.encode()
        compiler = []
        for interpreter, expected in zip(interpreters, (new_accepts, old_accepts)):
            result = subprocess.run([str(interpreter), "-I", "-c", CHECK], input=raw.hex().encode(), capture_output=True, timeout=10)
            actual = json.loads(result.stdout) if result.returncode == 0 else {"accepted": None, "error": result.stderr.decode(errors="replace")}
            compiler.append({"expected_acceptance": expected, "actual": actual,
                             "passed": result.returncode == 0 and actual["accepted"] == expected})
        observations = []
        for _, language in handles:
            parser = tree_sitter.Parser(language)
            tree = parser.parse(raw)
            root = tree.root_node
            ok = (not root.has_error) == parses
            recovered = None
            if not root.has_error:
                definitions = [n for n in root.named_children if n.type == "function_definition"]
                ok = ok and [n.child_by_field_name("name").text.decode() for n in definitions] == ["select", "after"]
                ok = ok and definitions[-1].start_byte == raw.index(b"def after")
                start = raw.index(b"case ") + len(b"case ")
                end = start + len(pattern.encode())
                selected = [n for n in nodes(root) if n.type == "case_pattern" and n.start_byte == start and n.end_byte == end]
                ok = ok and len(selected) == 1 and selected[0].named_children[0].type == kind
                if kind == "dict_pattern":
                    mapping = selected[0].named_children[0]
                    # A hidden signed-number rule assigns the key field to both
                    # the anonymous sign and number; never discard the sign.
                    ok = ok and [n.text for n in mapping.children_by_field_name("key")] == [b"+", b"1"]
                dirty = raw + b"(\n"
                tree.edit(len(raw), len(raw), len(dirty), endpoint(raw), endpoint(raw), endpoint(dirty))
                changed = parser.parse(dirty, tree)
                malformed = changed.root_node.has_error
                changed.edit(len(raw), len(dirty), len(raw), endpoint(raw), endpoint(dirty), endpoint(raw))
                recovered = malformed and shape(parser.parse(raw, changed)) == shape(parser.parse(raw))
            observations.append({"has_error": root.has_error, "expected_cst_met": ok, "incremental_recovery": recovered})
        rows.append({"pattern": pattern, "source": source, "source_digest": digest(raw), "compiler": compiler,
                     "baseline": observations[0], "candidate": observations[1], "native_target_mode": "not_run"})
    passed = all(all(c["passed"] for c in row["compiler"]) and row["candidate"]["expected_cst_met"] and
                 row["candidate"]["incremental_recovery"] is not False for row in rows)
    report = {"format": "entmoot.positive_pattern_oracles.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(),
              "interpreters": [{"path": str(p), "digest": digest(p.read_bytes()), "version": version}
                               for p, version in zip(interpreters, versions)],
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries],
              "cases": rows, "passed": passed,
              "limits": ["Actual compiler/runtime controls cover 3.15.0 and 3.13 only; no exhaustive historical runtime matrix",
                         "Native semantic target-mode qualification and Entmoot visitor adoption are separate, unrun lanes",
                         "Numeric compiler restrictions remain distinct from structural grammar acceptance"]}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"receipt": str(output), "cases": len(rows), "passed": passed}))
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
