"""Qualify available upstream Linux lanes using explicitly prepared local tools.

Run in Linux/WSL after preparing its scratch grammar, Node, Go, Tree-sitter CLI
and Python build dependencies. No global installation or source checkout writes.
Rust is opt-in with an explicit toolchain path; Swift remains a separate lane.
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
from grammar_layout import generated_digests, layout_digest, lint_sources, parser_commitment, parser_digests, parser_roots

GENERATED = ("src/parser.c", "src/grammar.json", "src/node-types.json")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--grammar", type=Path, required=True)
    ap.add_argument("--tools", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--abi", type=int, choices=(14, 15), default=15)
    ap.add_argument("--node-bin", type=Path)
    ap.add_argument("--node-peer-version", choices=("0.21.1", "0.25.0"), default="0.25.0")
    ap.add_argument("--rust-bin", type=Path)
    ap.add_argument("--go-oracle", type=Path, help="Additional authored parser-installation check")
    ap.add_argument("--wasm", action="store_true")
    ap.add_argument("--skip-node-tests", action="store_true", help="Record an upstream-disabled Node binding lane; lint still runs")
    ap.add_argument("--prepared-npm", action="store_true", help="Use the materializer's separately receipted locked install, preserving qualified dependency patches")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    if sys.platform != "linux":
        raise ValueError("Linux lane requires a Linux process")
    grammar, prepared = scratch_grammar(args.grammar), args.tools.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    node_bin = args.node_bin.resolve(strict=True) if args.node_bin else prepared / "node-v25.1.0-linux-x64/bin"
    cli = args.cli.resolve(strict=True)
    env = os.environ.copy()
    (out / "parser-cache").mkdir()
    env["TREE_SITTER_LIBDIR"] = str(out / "parser-cache")
    rust_path = ":" + str(args.rust_bin.resolve(strict=True)) if args.rust_bin else ""
    env.update(PATH=str(node_bin) + ":" + str(prepared / "go/bin") + rust_path + ":/usr/bin:/bin",
               PYTHONPATH=str(grammar / "bindings/python") + ":" + str(prepared / "python-deps"),
               GOTOOLCHAIN="local", CGO_ENABLED="1", GOCACHE=str(prepared / "go-cache"),
               GOPATH=str(prepared / "go-path"), npm_config_cache=str(prepared / "npm-cache"),
               npm_config_devdir=str(prepared / "node-headers"))
    rows = []

    def current_parser_digest():
        try:
            return parser_commitment(grammar), None
        except (OSError, ValueError) as error:
            return None, str(error)

    def run(name, command, cwd=grammar, extra_env=None):
        parser_before, error_before = current_parser_digest()
        start = time.perf_counter()
        result = subprocess.run(list(map(str, command)), cwd=cwd, env={**env, **(extra_env or {})},
                                capture_output=True, timeout=300)
        log = out / (name + ".log")
        log.write_bytes(result.stdout + result.stderr)
        parser_after, error_after = current_parser_digest()
        rows.append({"lane": name, "command": list(map(str, command)), "cwd": str(cwd),
                     "exit_code": result.returncode, "wall_seconds": time.perf_counter() - start,
                     "log": str(log), "log_digest": digest(log.read_bytes()), "build_overrides": extra_env or {},
                     "parser_digest": parser_before, "parser_digest_after": parser_after,
                     "parser_error_before": error_before, "parser_error_after": error_after})
        print(json.dumps({"lane": name, "exit_code": result.returncode}), flush=True)
        return result.returncode == 0

    for name, command in [("platform", ["uname", "-srmo"]), ("generator-version", [cli, "--version"]),
                          ("node-version", ["node", "--version"]), ("go-version", ["go", "version"]),
                          ("cc-version", ["gcc", "--version"]), ("python-version", [sys.executable, "--version"])]:
        if not run(name, command):
            raise ValueError("missing required tool: " + name)
    for root in parser_roots(grammar):
        if not run("generate-1-" + root.name, [cli, "generate", "--abi", str(args.abi)], cwd=root):
            raise ValueError("generation failed")
    generated = generated_digests(grammar)
    for root in parser_roots(grammar):
        if not run("generate-2-" + root.name, [cli, "generate", "--abi", str(args.abi)], cwd=root):
            raise ValueError("repeat generation failed")
    if generated != generated_digests(grammar):
        raise ValueError("non-deterministic generation")
    parser_digest = parser_commitment(grammar)
    qualified_parser_files = parser_digests(grammar)
    qualified_grammar_digests = {(root / "grammar.js").relative_to(grammar).as_posix(): digest((root / "grammar.js").read_bytes())
                                 for root in parser_roots(grammar)}
    # CLI 0.24 does not have --stat; --rebuild is shared by all qualified pins.
    run("corpus", [cli, "test", "--rebuild"])
    if args.rust_bin:
        if run("rust-version", ["rustc", "-Vv"]) and run("cargo-version", ["cargo", "--version"]):
            run("rust", ["cargo", "test", "--locked", "--quiet"])
    run("go", ["go", "test", "./bindings/go"])
    if args.go_oracle:
        run("go-parser-oracle", ["go", "run", args.go_oracle.resolve(strict=True)])
    if run("python-build", [sys.executable, "setup.py", "build_ext", "--force", "--inplace"]):
        run("python-test", [sys.executable, "-m", "unittest", "discover", "-s", "bindings/python/tests", "-v"])
    npm = ["node", node_bin.parent / "lib/node_modules/npm/bin/npm-cli.js"]
    installed = (grammar / "node_modules").is_dir() if args.prepared_npm else run("npm-locked-install", [*npm, "ci", "--ignore-scripts"])
    if args.prepared_npm and not installed:
        raise ValueError("prepared npm dependencies are missing")
    if installed and not args.skip_node_tests:
        # PR #336's lock omits its optional runtime peer. Install that exact peer
        # in a separate prepared directory; never rewrite the grammar's lock.
        local_peer = grammar / "node_modules/tree-sitter"
        peer = prepared / ("node-peer" if args.node_peer_version == "0.25.0" else "node-peer-" + args.node_peer_version)
        peer.mkdir(exist_ok=True)
        # The runtime peer pins C++17, while Node 25 headers require C++20.
        # Append the override only for runtime C++ compilations, never grammar C.
        cxx = prepared / "node-runtime-cxx20"
        cxx.write_text('#!/bin/sh\nexec /usr/bin/g++ "$@" -std=c++20\n', encoding="utf-8")
        cxx.chmod(0o755)
        runtime_env = {"CXX": str(cxx)}
        # Build exactly the runtime used by the smoke test. Local peers shadow
        # NODE_PATH; preparing both copies wastes work and can test the wrong pin.
        if local_peer.exists():
            actual = json.loads((local_peer / "package.json").read_text())["version"]
            if actual != args.node_peer_version:
                raise ValueError("unexpected local Node runtime peer: " + actual)
            runtime_ready = run("node-local-peer-build", [*npm, "run", "install"], cwd=local_peer, extra_env=runtime_env)
            peer_cwd = grammar
        else:
            if not (peer / "node_modules/tree-sitter/build/Release/tree_sitter_runtime_binding.node").exists():
                run("node-peer-prepare", [*npm, "install", "--prefix", peer, "--no-package-lock", "tree-sitter@" + args.node_peer_version],
                    cwd=peer, extra_env=runtime_env)
            env["NODE_PATH"] = str(peer / "node_modules")
            runtime_ready = True
            peer_cwd = peer
        if runtime_ready:
            runtime_ready = run("node-peer-test", ["node", "-e",
                "if(require('tree-sitter/package.json').version !== '" + args.node_peer_version + "') process.exit(1); require('tree-sitter')"], cwd=peer_cwd)
        gyp = node_bin.parent / "lib/node_modules/npm/node_modules/node-gyp/bin/node-gyp.js"
        if runtime_ready and run("node-build", ["node", gyp, "rebuild"]):
            run("node-test", ["node", "--test", "bindings/node/binding_test.js"])
    if installed:
        run("lint", ["node", grammar / "node_modules/eslint/bin/eslint.js", *lint_sources(grammar)])
    cmake_build = out / "cmake-build"
    if run("c-configure", ["cmake", "-S", ".", "-B", cmake_build, "-D", "TREE_SITTER_CLI=" + str(cli),
                           "-D", "TREE_SITTER_ABI_VERSION=" + str(args.abi)]):
        run("c-build", ["cmake", "--build", cmake_build, "--clean-first"])
    if args.wasm:
        run("wasm", [cli, "build", "--wasm"])
    # Generation establishes the selected ABI; every later binding lane must use
    # and preserve that exact parser, including CMake's own generation recipe.
    binding_rows = rows[next(i for i, row in enumerate(rows) if row["lane"] == "corpus"):]
    parser_unchanged = all(row["parser_digest"] == parser_digest == row["parser_digest_after"] for row in binding_rows)
    report = {"format": "entmoot.grammar_linux.v1", "grammar": str(grammar),
              "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(), "generated_files": generated,
              "grammar_digests": qualified_grammar_digests,
              "parser_files": qualified_parser_files, "layout_driver_digest": layout_digest(),
              "inputs": {p: digest((grammar / p).read_bytes()) for p in
                         ("src/scanner.c", "Cargo.toml", "Cargo.lock", "go.mod", "go.sum", "package-lock.json") if (grammar / p).exists()},
              "rows": rows, "parser_unchanged": parser_unchanged,
              "requested_commands_passed": all(row["exit_code"] == 0 for row in rows) and parser_unchanged,
              "configuration": {"abi": args.abi, "node_peer": args.node_peer_version, "parser_cache": env["TREE_SITTER_LIBDIR"],
                                "rust_requested": bool(args.rust_bin), "wasm_requested": args.wasm,
                                "node_tests_requested": not args.skip_node_tests},
              "prepared_npm": args.prepared_npm,
              "limits": ["Linux/WSL GCC upstream corpus and smoke tests, not a macOS or native adapter qualification",
                         "Rust unrun unless requested; Swift unrun; no Linux historical runtime matrix or pipeline performance measurement",
                         "Compiler oracles, feature CST controls, examples and recovery are separate receipts"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(0 if report["requested_commands_passed"] else 1)


if __name__ == "__main__":
    main()
