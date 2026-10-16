"""Verify selected authored controls with the existing GCC 11.4 historical compiler."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest

ROOT = Path(__file__).resolve().parent.parent


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    prefix = ["wsl", "-d", "Ubuntu", "--", "env", "-u", "CPATH", "-u", "C_INCLUDE_PATH", "-u", "CPLUS_INCLUDE_PATH", "-u", "LIBRARY_PATH"] if os.name == "nt" else []

    def native(path):
        text = path.resolve().as_posix()
        if os.name == "nt":
            if not text.startswith("G:/"):
                raise ValueError("recorded historical lane requires G drive")
            return "/mnt/g/" + text[3:]
        return text

    manifest = {"format": "entmoot.c_family_historical_inputs.v1", "driver_digest": digest(Path(__file__).read_bytes()), "families": {}}
    selections = []
    for family, compiler, modes in [("c", "/usr/bin/gcc-11", ("c89", "c99", "c11", "c17")),
                                    ("cpp", "/usr/bin/g++-11", ("c++98", "c++11", "c++14", "c++17"))]:
        fixture = ROOT / family / "cases.py"
        spec = importlib.util.spec_from_file_location("historical_" + family, fixture)
        cases = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cases)
        selected = [case for case in cases.CASES if case["mode"] in modes]
        version = subprocess.check_output([*prefix, compiler, "--version"], text=True)
        if "11.4.0" not in version:
            raise ValueError("the recorded GCC 11.4.0 compiler is required")
        manifest["families"][family] = {"compiler": compiler, "version": version, "fixture_digest": digest(fixture.read_bytes()),
                                       "modes": modes, "cases": [{"id": c["id"], "source_digest": digest(c["source"].encode())} for c in selected]}
        selections.extend((family, compiler, case) for case in selected)
    (out / "input_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")
    rows = []
    for family, compiler, case in selections:
        directory = out / case["id"]
        directory.mkdir()
        source = directory / ("fixture.c" if family == "c" else "fixture.cpp")
        source.write_text(case["source"], encoding="utf-8", newline="\n")
        flags = ["-std=" + case["mode"], "-pedantic-errors", "-fdiagnostics-color=never"]

        def run(name, command):
            result = subprocess.run([*prefix, *command], capture_output=True, timeout=60)
            raw = result.stdout + result.stderr
            (directory / (name + ".log")).write_bytes(raw)
            return {"command": [*prefix, *command], "exit_code": result.returncode, "output": raw.decode("utf-8", "replace"), "log_digest": digest(raw)}

        check = run("compiler", [compiler, *flags, "-fsyntax-only", native(source)])
        check["passed"] = (check["exit_code"] == 0) == case["accepted"]
        link, execution = None, None
        if check["exit_code"] == 0 and case["execute"]:
            binary = directory / "fixture.program"
            link = run("link", [compiler, *flags, native(source), "-o", native(binary)])
            if link["exit_code"] == 0:
                execution = run("runtime", [native(binary)])
                execution["passed"] = execution["exit_code"] == 0 and execution["output"].strip() == "oracle:ok"
                execution["executable_digest"] = digest(binary.read_bytes())
        rows.append({"case": case["id"], "family": family, "mode": case["mode"], "compiler": check, "link": link, "runtime": execution,
                     "passed": check["passed"] and (not case["execute"] or execution is not None and execution["passed"])})
    report = {"format": "entmoot.c_family_historical_qualification.v1", "inputs": manifest, "rows": rows, "passed": all(r["passed"] for r in rows),
              "limits": ["GCC 11.4 explicit historical control, not an exhaustive compiler-version matrix",
                         "System compiler and headers; no hermetic sysroot or Entmoot native-provider qualification",
                         "Compiler diagnostic wording is not required to match Clang"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"cases": len(rows), "failures": [r["case"] for r in rows if not r["passed"]]}))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
