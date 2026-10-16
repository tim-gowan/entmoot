"""Qualify a prepared grammar build; never install tools or author expectations.

Run separately for the pinned baseline, lazy-import proposal, comprehension patch
and combined candidate. The caller supplies scratch grammar trees, an explicit
generator and Python 3.15. Source checkouts and Entmoot's Cargo pin are untouched.
Requires the independently pinned tree-sitter==0.25.2 and blake3 Python packages.
"""
from __future__ import annotations

import argparse
import importlib.metadata
import json
from pathlib import Path
import re
import subprocess
import sys
import time

import tree_sitter

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent))
from grammar_runtime import digest, endpoint, load_language, nodes, runtime_digest, scratch_grammar, shape

GENERATED = ("src/parser.c", "src/grammar.json", "src/node-types.json")


def cases():
    # Expected kinds/fields are authored independently of Tree-sitter output.
    for delimiter, kind, star in [("[]", "list_comprehension", "*"),
                                  ("{}", "set_comprehension", "*"),
                                  ("{}", "dictionary_comprehension", "**"),
                                  ("()", "generator_expression", "*")]:
        for asynchronous in (False, True):
            expression = f"{delimiter[0]}{star}chunk.values() {'async ' if asynchronous else ''}for chunk in chunks{delimiter[1]}"
            source = f"# π\r\n{'async ' if asynchronous else ''}def collect(chunks):\r\n    return {expression}\r\ndef after():\r\n    return 17\r\n"
            yield {"id": kind + ("-async" if asynchronous else ""), "source": source,
                   "compiler": "accepted", "parse": True, "definitions": ["collect", "after"],
                   "body": {"kind": kind, "type": "dictionary_splat" if star == "**" else "list_splat",
                            "operand": "call", "text": star + "chunk.values()"}}
    for name, source in [
        ("lazy-module", "lazy import json as lazy_json\nlazy from math import sqrt as root\ndef after():\n    return lazy_json.dumps(root(4))\n"),
        ("lazy-identifiers", "lazy = 1\ndef lazy(lazy):\n    return lazy\ndef after():\n    return object.lazy\n"),
        ("lazy-import-name", "import lazy as lazy\nfrom lazy import lazy\ndef after():\n    return lazy\n"),
    ]:
        case = {"id": name, "source": source, "compiler": "accepted", "parse": True,
                "definitions": ["lazy", "after"] if name == "lazy-identifiers" else ["after"]}
        if name == "lazy-module":
            case["imports"] = [
                {"kind": "import_statement", "module": None, "name": "json", "alias": "lazy_json", "lazy": True},
                {"kind": "import_from_statement", "module": "math", "name": "sqrt", "alias": "root", "lazy": True},
            ]
        elif name == "lazy-import-name":
            case["imports"] = [
                {"kind": "import_statement", "module": None, "name": "lazy", "alias": "lazy", "lazy": False},
                {"kind": "import_from_statement", "module": "lazy", "name": "lazy", "alias": None, "lazy": False},
            ]
        yield case
    # Grammar accepts statement syntax; compiler scope restrictions remain separate.
    for name, source in [
        ("lazy-function", "def f():\n    lazy import json\n"),
        ("lazy-class", "class C:\n    lazy import json\n"),
        ("lazy-try", "try:\n    lazy import json\nexcept ImportError:\n    pass\n"),
        ("lazy-star", "lazy from math import *\n"),
        ("lazy-future", "lazy from __future__ import annotations\n"),
    ]:
        yield {"id": name, "source": source, "compiler": "syntax_error", "parse": True}
    for name, source in [("list-double", "[**d for d in maps]\n"),
                         ("generator-double", "(**d for d in maps)\n"),
                         ("malformed-lazy", "lazy import\n")]:
        yield {"id": name, "source": source, "compiler": "syntax_error", "parse": False}
    for operand, kind in [("chunk.values", "attribute"), ("chunk[0].values", "attribute"), ("chunk()", "call")]:
        yield {"id": "historical-" + operand, "source": "[*" + operand + "]\n", "compiler": "accepted", "parse": True,
               "body": {"kind": "list", "type": "list_splat", "operand": kind, "text": "*" + operand}}
    for name, source in [
        ("starred-return", "def f(values):\n    return *values,\n"),
        ("starred-yield", "def f(values):\n    yield *values,\n"),
        ("starred-for", "for value in *left, *right:\n    pass\n"),
        ("starred-subscript", "value[*indices]\n"),
        ("variadic-type", "tuple[*Ts]\n"),
        ("variadic-annotation", "def f(*args: *tuple[int, str]):\n    pass\n"),
        ("starred-fstring", 'f"{*values,}"\n'),
        ("starred-assignment", "head, *tail = values\n"),
        ("unpacked-generator-argument", "f(*value for value in rows)\n"),
        ("unpacked-walrus", "[*(xs := [1, 2]) for value in [0]]\n"),
    ]:
        yield {"id": name, "source": source, "compiler": "accepted", "parse": True}


COMPILER = """import json,sys
source=bytes.fromhex(sys.stdin.read())
try: compile(source,'qualification','exec')
except SyntaxError: result='syntax_error'
else: result='accepted'
print(json.dumps({'outcome':result}))
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--variant", choices=("baseline", "lazy-proposal", "comprehension", "combined"), required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--python", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    grammar = scratch_grammar(args.grammar)
    cli, interpreter = (p.resolve(strict=True) for p in (args.cli, args.python))
    output = args.out.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise ValueError("receipt already exists; select a fresh namespace")
    logs = output.parent / (output.stem + "-logs")
    logs.mkdir(exist_ok=False)
    commands = []

    def run(name, command, **kwargs):
        started = time.perf_counter()
        result = subprocess.run(command, capture_output=True, timeout=180, **kwargs)
        log = logs / (name + ".log")
        log.write_bytes(result.stdout + result.stderr)
        commands.append({"name": name, "command": list(map(str, command)), "cwd": str(kwargs.get("cwd", Path.cwd())),
                         "exit_code": result.returncode, "wall_seconds": time.perf_counter() - started,
                         "log": str(log), "log_digest": digest(log.read_bytes())})
        return result

    version = run("generator-version", [str(cli), "--version"])
    if version.returncode or not version.stdout.startswith(b"tree-sitter 0.25.9 "):
        raise ValueError("generator must be the upstream lock's 0.25.9")
    python_version = run("compiler-version", [str(interpreter), "-I", "-c", "import sys;print(sys.version)"])
    if python_version.returncode or not python_version.stdout.startswith(b"3.15.0 "):
        raise ValueError("October compiler oracle requires Python 3.15.0")
    if importlib.metadata.version("tree-sitter") != "0.25.2":
        raise ValueError("CST probe requires tree-sitter==0.25.2")
    before = {p: digest((grammar / p).read_bytes()) for p in GENERATED}
    for iteration in (1, 2):
        run(f"generate-{iteration}", [str(cli), "generate", "--abi", "15"], cwd=grammar).check_returncode()
        current = {p: digest((grammar / p).read_bytes()) for p in GENERATED}
        if iteration == 1:
            generated = current
        elif generated != current:
            raise ValueError("generation is not deterministic")
    # CLI parser caches are keyed by grammar name; separate Python variants can
    # otherwise reuse a newer compiled library from another scratch directory.
    corpus = run("upstream-corpus", [str(cli), "test", "--rebuild", "--stat", "total-only"], cwd=grammar)
    dll = logs / "python.dll"
    run("build", [str(cli), "build", "--output", str(dll)], cwd=grammar).check_returncode()
    library, language = load_language(dll, "python")
    parser = tree_sitter.Parser(language)
    rows = []
    for case in cases():
        source = case["source"].encode("utf-8")
        tree = parser.parse(source)
        root = tree.root_node
        ok = (not root.has_error) == case["parse"]
        if "definitions" in case:
            names = [n.child_by_field_name("name").text.decode() for n in nodes(root) if n.type == "function_definition"]
            ok = ok and names == case["definitions"]
        if "imports" in case:
            imports = []
            for node in nodes(root):
                if node.type not in ("import_statement", "import_from_statement"):
                    continue
                module = node.child_by_field_name("module_name")
                for name in node.children_by_field_name("name"):
                    alias = name.child_by_field_name("alias") if name.type == "aliased_import" else None
                    imported = name.child_by_field_name("name") if alias is not None else name
                    imports.append({"kind": node.type, "module": module.text.decode() if module else None,
                                    "name": imported.text.decode(), "alias": alias.text.decode() if alias else None,
                                    "lazy": node.parent.type == "lazy_import_statement"})
            ok = ok and imports == case["imports"]
        if "body" in case:
            expected = case["body"]
            targets = [n for n in nodes(root) if n.type == expected["kind"]]
            bodies = [n.child_by_field_name("body") if expected["kind"] != "list" else n.named_children[0] for n in targets]
            ok = ok and len(bodies) == 1 and all(n is not None and n.type == expected["type"] and
                n.text.decode() == expected["text"] and n.named_children[0].type == expected["operand"] for n in bodies)
        compiled = run("compile-" + re.sub(r"[^a-zA-Z0-9-]", "_", case["id"]),
                       [str(interpreter), "-I", "-c", COMPILER], input=source.hex().encode())
        actual = json.loads(compiled.stdout)["outcome"] if compiled.returncode == 0 else "process_error"
        incremental = None
        if not root.has_error:
            # Add an incomplete expression, then recover, preserving UTF-8/CRLF spans.
            dirty = source + b"(\n"
            tree.edit(len(source), len(source), len(dirty), endpoint(source), endpoint(source), endpoint(dirty))
            changed = parser.parse(dirty, tree)
            malformed = changed.root_node.has_error
            changed.edit(len(source), len(dirty), len(source), endpoint(source), endpoint(dirty), endpoint(source))
            recovered = parser.parse(source, changed)
            incremental = malformed and shape(recovered) == shape(parser.parse(source))
        rows.append({"expectation": case, "source_digest": digest(source),
                     "structural": {"has_error": root.has_error, "expected_cst_met": ok,
                                    "cst": str(root), "incremental_recovery": incremental},
                     "native_target_mode": "not_run", "compiler": {"actual": actual, "passed": actual == case["compiler"]}})
    fixtures = []
    for relative in ["lazy-imports/fixtures/positive.py", "comprehension-unpacking/fixtures/positive.py",
                     "comprehension-unpacking/fixtures/call_bodies.py", "comprehension-unpacking/fixtures/historical_unpacking.py"]:
        path = ROOT / relative
        result = run("runtime-" + path.parent.parent.name + "-" + path.stem, [str(interpreter), "-I", str(path)])
        source = path.read_bytes()
        fixtures.append({"path": relative, "source_digest": digest(source), "runtime_passed": result.returncode == 0,
                         "structural_has_error": parser.parse(source).root_node.has_error, "native_target_mode": "not_run"})
    report = {"format": "entmoot.grammar_qualification.v1", "release_cutoff": "2026-10-10", "variant": args.variant,
              "grammar_build": str(grammar), "grammar_digest": digest((grammar / "grammar.js").read_bytes()),
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "generator_digest": digest(cli.read_bytes()),
              "interpreter_digest": digest(interpreter.read_bytes()), "generator": version.stdout.decode().strip(),
              "compiler": python_version.stdout.decode().strip(), "runtime": "tree-sitter==0.25.2", "platform": sys.platform,
              "generated_files": generated, "generation_matches_input": before == generated, "generation_repeatable": True,
              "parser": {"abi": language.abi_version, "states": language.parse_state_count,
                         "node_kinds": language.node_kind_count, "c_bytes": (grammar / "src/parser.c").stat().st_size,
                         "library_bytes": dll.stat().st_size, "library_digest": digest(dll.read_bytes())},
              "corpus_passed": corpus.returncode == 0, "commands": commands, "cases": rows, "fixtures": fixtures,
              "limits": ["Windows-only local qualification; other upstream platforms and binding lanes are reported separately",
                         "CST assertions are authored; CPython independently checks acceptance and runtime behavior",
                         "Grammar acceptance does not qualify Pyright, Entmoot visitors or historical compiler installations",
                         "Per-command times are diagnostic; no five-cold/five-warm pipeline performance claim",
                         "Grammar-invalid and compiler-invalid are independent outcomes; no semantic type checking is added"]}
    with output.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"receipt": str(output), "corpus_passed": report["corpus_passed"],
                      "cst_controls_passed": sum(row["structural"]["expected_cst_met"] for row in rows), "cst_controls": len(rows),
                      "compiler_controls_passed": sum(row["compiler"]["passed"] for row in rows)}))
    # Baseline and single-feature probes intentionally retain documented gaps.
    # The combined candidate cannot pass by merely completing the process.
    common_passed = report["corpus_passed"] and all(row["compiler"]["passed"] for row in rows) and all(
        fixture["runtime_passed"] for fixture in fixtures) and all(
        row["structural"]["incremental_recovery"] is not False for row in rows)
    combined_passed = all(row["structural"]["expected_cst_met"] for row in rows) and all(
        not fixture["structural_has_error"] for fixture in fixtures)
    if not common_passed or (args.variant == "combined" and not combined_passed):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
