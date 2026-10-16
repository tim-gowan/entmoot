"""Run the upstream Windows binding lanes on a prepared scratch grammar.

Tool installation and Node's tree-sitter peer build are separate preparation.
Force fresh grammar builds; a failed build prevents its dependent smoke test.
This records upstream smoke tests, not semantic binding or platform equivalence.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from grammar_runtime import digest, runtime_digest, scratch_grammar
from grammar_layout import layout_digest, lint_sources, parser_abi, parser_commitment, parser_digests


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--node-gyp", type=Path, required=True)
    ap.add_argument("--node-modules", type=Path, required=True)
    ap.add_argument("--cargo-target", type=Path, required=True)
    ap.add_argument("--go", type=Path)
    ap.add_argument("--go-oracle", type=Path, help="Additional authored parser-installation check")
    ap.add_argument("--skip-node-tests", action="store_true", help="Record an upstream-disabled Node binding lane; lint still runs")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    if sys.platform != "win32":
        raise ValueError("this lane qualifies Windows only")
    grammar = scratch_grammar(args.grammar)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    cli, node_gyp, node_modules = (p.resolve(strict=True) for p in (args.cli, args.node_gyp, args.node_modules))
    env = os.environ.copy()
    # The C++20 override used to build Node 25's runtime is invalid for grammar C.
    env.pop("_CL_", None)
    env.pop("CL", None)
    env["NODE_PATH"] = str(node_modules)
    env["PYTHONPATH"] = str(grammar / "bindings/python")
    env["CARGO_TARGET_DIR"] = str(args.cargo_target.resolve())
    # Variants share the grammar export name. Even --rebuild can race with a
    # second process in the default loader cache; isolate each run's library.
    env["TREE_SITTER_LIBDIR"] = str(out / "parser-cache")
    (out / "parser-cache").mkdir()
    parser_digest = parser_commitment(grammar)
    qualified_parser_files = parser_digests(grammar)
    abi = parser_abi(grammar)
    rows = []

    def current_parser_digest():
        try:
            return parser_commitment(grammar), None
        except (OSError, ValueError) as error:
            return None, str(error)

    def run(name, command, *, extra_env=None):
        parser_before, error_before = current_parser_digest()
        start = time.perf_counter()
        result = subprocess.run(list(map(str, command)), cwd=grammar, env={**env, **(extra_env or {})},
                                capture_output=True, timeout=300)
        log = out / (name + ".log")
        log.write_bytes(result.stdout + result.stderr)
        parser_after, error_after = current_parser_digest()
        rows.append({"lane": name, "command": list(map(str, command)), "cwd": str(grammar),
                     "exit_code": result.returncode, "wall_seconds": time.perf_counter() - start,
                     "log": str(log), "log_digest": digest(log.read_bytes()), "parser_digest": parser_before,
                     "parser_digest_after": parser_after, "parser_error_before": error_before,
                     "parser_error_after": error_after})
        print(json.dumps({"lane": name, "exit_code": result.returncode}), flush=True)
        return result.returncode == 0

    versions = [("node-version", ["node", "--version"]), ("rust-version", ["rustc", "-Vv"]),
                ("cargo-version", ["cargo", "--version"]), ("python-version", [sys.executable, "--version"]),
                ("generator-version", [cli, "--version"]), ("cmake-version", ["cmake", "--version"])]
    for name, command in versions:
        if not run(name, command):
            raise ValueError("missing required tool: " + name)
    run("rust", ["cargo", "test", "--locked", "--quiet"])
    if run("python-build", [sys.executable, "setup.py", "build_ext", "--force", "--inplace"]):
        run("python-test", [sys.executable, "-m", "unittest", "discover", "-s", "bindings/python/tests", "-v"])
    if not args.skip_node_tests and run("node-build", ["node", node_gyp, "rebuild"]):
        run("node-test", ["node", "--test", "bindings/node/binding_test.js"])
    run("lint", ["node", grammar / "node_modules/eslint/bin/eslint.js", *lint_sources(grammar)])
    cmake_build = out / "cmake-build"
    if run("c-configure", ["cmake", "-S", ".", "-B", cmake_build, "-D", "TREE_SITTER_CLI=" + str(cli),
                           "-D", "TREE_SITTER_ABI_VERSION=" + str(abi)]):
        run("c-build", ["cmake", "--build", cmake_build, "--config", "Release", "--clean-first"])
    if args.go:
        run("go", [args.go.resolve(strict=True), "test", "./bindings/go"],
            extra_env={"CGO_ENABLED": "1", "GOTOOLCHAIN": "local"})
        if args.go_oracle:
            run("go-parser-oracle", [args.go.resolve(strict=True), "run", args.go_oracle.resolve(strict=True)],
                extra_env={"CGO_ENABLED": "1", "GOTOOLCHAIN": "local"})
    parser_unchanged = all(row["parser_digest"] == parser_digest == row["parser_digest_after"] for row in rows)
    passed = all(row["exit_code"] == 0 for row in rows) and parser_unchanged
    report = {"format": "entmoot.grammar_bindings.v1", "grammar": str(grammar),
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(), "parser_digest": parser_digest,
              "parser_abi": abi, "parser_unchanged": parser_unchanged, "parser_files": qualified_parser_files, "layout_driver_digest": layout_digest(),
              "node_tests_requested": not args.skip_node_tests,
              "inputs": {p: digest((grammar / p).read_bytes()) for p in
                         ("grammar.js", "src/scanner.c", "Cargo.toml", "Cargo.lock", "package-lock.json") if (grammar / p).exists()},
              "environment": {key: env[key] for key in ("NODE_PATH", "PYTHONPATH", "CARGO_TARGET_DIR", "TREE_SITTER_LIBDIR")},
              "rows": rows, "requested_commands_passed": passed,
              "limits": ["Only Windows upstream smoke tests; no Entmoot semantic binding qualification",
                         "Go is unrun unless explicitly supplied; missing CGo compiler is a failed lane",
                         "Swift and Linux/macOS lanes are not exercised by this driver",
                         "Prepared Node runtime peer and tool installation are separately recorded build inputs"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
