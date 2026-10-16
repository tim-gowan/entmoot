"""Run pinned Go compiler/runtime controls independently of parser acceptance."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

import blake3

ROOT = Path(__file__).resolve().parent


def digest(data: bytes) -> str:
    return "blake3:" + blake3.blake3(data).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    tool = args.go.resolve(strict=True)
    version = subprocess.check_output([str(tool), "version"], text=True).strip()
    if "go1.27.2 " not in version:
        raise ValueError("Qualification requires Go 1.27.2 exactly")
    env = {**os.environ, "GOTOOLCHAIN": "local", "GOPROXY": "off", "GOSUMDB": "off", "GOENV": "off",
           "GOROOT": str(tool.parent.parent), "GOFLAGS": "", "GOEXPERIMENT": "", "GOCACHE": str(args.cache.resolve())}
    cases = [
        ("generic-methods", "go1.27", (ROOT / "fixtures/positive.go").read_bytes(), True, "generic-method-and-following-declaration:ok"),
        ("historical-method", "go1.14", b'package main\nimport "fmt"\ntype Box struct { Value int }\nfunc (b Box) Get() int { return b.Value }\nfunc After() int { return 7 }\nfunc main() { if (Box{3}).Get()!=3 || After()!=7 { panic("bad") }; fmt.Println("historical:ok") }\n', True, "historical:ok"),
        ("interface-generic-method", "go1.27", b'package main\ntype Invalid interface { Map[T any](T) T }\nfunc main() {}\n', False, "type parameters"),
        ("unsatisfied-constraint", "go1.27", b'package main\ntype Box struct{}\nfunc (Box) Map[T ~int](x T) T { return x }\nfunc main() { Box{}.Map("wrong") }\n', False, "does not satisfy"),
    ]
    rows = []
    with tempfile.TemporaryDirectory(prefix="entmoot-go-oracle-") as temporary:
        for name, mode, source, success, expected in cases:
            file = Path(temporary) / (name + ".go")
            file.write_bytes(source)
            command = [str(tool), "run", "-gcflags=-lang=" + mode, str(file)]
            completed = subprocess.run(command, env=env, text=True, capture_output=True, timeout=60)
            output = completed.stdout + completed.stderr
            rows.append({"id": name, "source_hex": source.hex(), "source_digest": digest(source),
                         "target_mode": mode, "command": command, "exit_code": completed.returncode,
                         "stdout": completed.stdout, "stderr": completed.stderr,
                         "expected_success": success, "expected_output": expected,
                         "passed": (completed.returncode == 0) == success and expected in output,
                         "structural": "not_run", "native_resolver": "not_run"})
    receipt = {"format": "entmoot.grammar_oracle.v1", "release_cutoff": "2026-10-10",
               "tool": version, "tool_digest": digest(tool.read_bytes()),
               "compile_tool_digest": digest((tool.parent.parent / "pkg/tool/windows_amd64/compile.exe").read_bytes()) if os.name == "nt" else None,
               "driver_digest": digest(Path(__file__).read_bytes()), "cases": rows,
               "historical_runtime_matrix": "not_run", "passed": all(row["passed"] for row in rows)}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("x", encoding="utf-8") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(json.dumps({"receipt": str(args.out), "passed": receipt["passed"], "cases": len(rows)}))
    raise SystemExit(0 if receipt["passed"] else 1)


if __name__ == "__main__":
    main()
