"""Compiler-check exact authored revision sources; separate from compiler-free visitors.

This workstation lane uses the isolated October tool installations qualified in
the adjacent grammar packets. It never installs tools, executes fixture programs,
or claims native Entmoot provider qualification. Missing tools fail explicitly.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

import blake3

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "crates/entmoot-core/tests/fixtures/revision_pairs.json"


def digest(raw):
    return "blake3:" + blake3.blake3(raw).hexdigest()


def linux(path):
    value = Path(path).resolve().as_posix()
    if not value.startswith("G:/"):
        raise ValueError("this WSL qualification lane requires a recorded G drive path")
    return "/mnt/g/" + value[3:]


def run(command, directory, environment):
    command = list(map(str, command))
    start = time.perf_counter()
    try:
        process = subprocess.run(command, cwd=directory, env=environment, capture_output=True, timeout=90)
        raw = process.stdout + process.stderr
        code, error = process.returncode, None
    except (OSError, subprocess.TimeoutExpired) as exception:
        code, error = None, str(exception)
        raw = str(exception).encode("utf-8")
    return dict(command=command, exit_code=code, error=error, output=raw.decode("utf-8", "replace"),
                output_digest=digest(raw), wall_seconds=time.perf_counter()-start)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools", type=Path, default=REPO / ".tmp/tools")
    parser.add_argument("--selection", type=Path, required=True, help="JSON array of authored revision-pair IDs")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "nt":
        raise ValueError("use the recorded Windows/WSL compiler lane")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    root = args.tools.resolve(strict=True)
    qualification = root / "tree-sitter-qualification"
    ids = json.loads(args.selection.read_text(encoding="utf-8"))
    all_pairs = json.loads(FIXTURES.read_text(encoding="utf-8"))["pairs"]
    pairs = [pair for pair in all_pairs if pair["id"] in ids]
    if not ids or len(ids) != len(set(ids)) or len(pairs) != len(ids):
        raise ValueError("selection must identify distinct authored pairs")
    environment = {key: value for key, value in os.environ.items() if key.upper() not in {
        "NODE_OPTIONS", "NODE_PATH", "RUSTC_BOOTSTRAP", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC_WRAPPER", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "CLASSPATH",
        "CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "LIBRARY_PATH", "MSBUILDSDKSPATH",
        "DOTNET_STARTUP_HOOKS", "DOTNET_ADDITIONAL_DEPS", "DOTNET_SHARED_STORE"}}
    environment.update(GOTOOLCHAIN="local", GOENV="off", GOFLAGS="", GOWORK="off", GO111MODULE="off")
    python = root / "python-3.15.0/python.exe"
    go = root / "go1.27.2/go/bin/go.exe"
    javac = qualification / "java/jdk-27/bin/javac.exe"
    node = qualification / "ecmascript/node-v26.11.1-win-x64/node.exe"
    ts6 = root / "typescript-program-6.0.3/node_modules/typescript/lib/tsc.js"
    ts7 = qualification / "ecmascript/typescript-7.0.2/node_modules/typescript/lib/tsc.js"
    dotnet_root = qualification / "csharp/dotnet-10.0.401"
    dotnet = dotnet_root / "dotnet.exe"
    csc = dotnet_root / "sdk/10.0.401/Roslyn/bincore/csc.dll"
    references = sorted((dotnet_root / "packs/Microsoft.NETCore.App.Ref/10.0.12/ref/net10.0").glob("*.dll"))
    if not references:
        raise ValueError("missing qualified .NET reference pack")
    environment.update(DOTNET_ROOT=str(dotnet_root), DOTNET_ROOT_X64=str(dotnet_root),
                       DOTNET_MULTILEVEL_LOOKUP="0", DOTNET_ROLL_FORWARD="Disable", DOTNET_NOLOGO="1")
    rustc = qualification / "linux-tools/rustup/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin/rustc"
    clang_root = qualification / "c-family/llvm-23.1.3"
    wsl = ["wsl", "-d", "Ubuntu", "--", "env", "-u", "RUSTC_BOOTSTRAP", "-u", "RUSTFLAGS",
           "-u", "CPATH", "-u", "C_INCLUDE_PATH", "-u", "CPLUS_INCLUDE_PATH", "-u", "LIBRARY_PATH"]
    clang_prefix = [*wsl, "LD_LIBRARY_PATH="+linux(clang_root / "lib")]
    commands = {
        "python": ([python, "-I", "--version"], "Python 3.15.0"),
        "go": ([go, "version"], "go version go1.27.2 "),
        "java": ([javac, "-version"], "javac 27"),
        "node": ([node, "--version"], "v26.11.1"),
        "typescript6": ([node, ts6, "--version"], "Version 6.0.3"),
        "typescript7": ([node, ts7, "--version"], "Version 7.0.2"),
        "dotnet": ([dotnet, "--version"], "10.0.401"),
        "rust": ([*wsl, linux(rustc), "--version"], "rustc 1.99.0 "),
        "clang": ([*clang_prefix, linux(clang_root / "bin/clang"), "--version"], "clang version 23.1.3 "),
    }
    tools = {name: run(command, out, environment) for name, (command, _) in commands.items()}
    (out / "tool_preflight.json").write_text(json.dumps(tools,indent=2)+"\n",encoding="utf-8",newline="\n")
    for name, (_, expected) in commands.items():
        if tools[name]["exit_code"] != 0 or not tools[name]["output"].strip().startswith(expected):
            raise ValueError(f"unavailable or mismatched compiler: {name}: {tools[name]}")
    inputs = [python, go, javac, node, ts6, ts6.parent / "_tsc.js", ts7, dotnet, csc, rustc,
              clang_root / "bin/clang-23", *references]
    # The TypeScript 7 shim selects a native executable, not a compiler API.
    ts7_native = sorted((ts7.parents[2] / "@typescript").glob("typescript-win32-x64/lib/tsc.exe"))
    if len(ts7_native) != 1:
        raise ValueError("missing unique TypeScript 7 native executable")
    inputs.extend(ts7_native)
    manifest = dict(format="entmoot.visitor_compiler_inputs.v1", fixture_digest=digest(FIXTURES.read_bytes()),
                    selection=ids, driver_digest=digest(Path(__file__).read_bytes()), tools=tools,
                    tool_files=[dict(path=str(path), digest=digest(path.read_bytes())) for path in inputs],
                    sources=[dict(id=pair["id"], profile=pair["profile"], revisions={side:digest(pair[side]["source"].encode()) for side in ("base","head")}) for pair in pairs])
    (out / "input_manifest.json").write_text(json.dumps(manifest,indent=2)+"\n",encoding="utf-8",newline="\n")
    rows=[]
    for pair in pairs:
        for side in ("base", "head"):
            revision=pair[side]
            directory=out / pair["id"] / side
            directory.mkdir(parents=True)
            source=directory / Path(revision["path"]).name
            source.write_bytes(revision["source"].encode("utf-8"))
            language=pair["language"]
            if language=="python":
                checks={"python315":[python,"-I","-c","import pathlib,sys; compile(pathlib.Path(sys.argv[1]).read_bytes(),sys.argv[1],'exec')",source]}
            elif language=="go":
                checks={"go127":[go,"tool","compile","-lang=go"+pair["profile"],"-o",directory/"fixture.a",source]}
            elif language=="java":
                checks={"jdk27":[javac,"-encoding","UTF-8","-proc:none","--release",pair["profile"],"-d",directory,source]}
            elif language=="rust":
                checks={"rust199":[*wsl,linux(rustc),"--crate-type=lib","--crate-name=fixture","--edition=2024","--emit=metadata","-o",linux(directory/"fixture.rmeta"),linux(source)]}
            elif language in ("c","cpp"):
                mode=pair["profile"].split()[0].lower()
                executable=clang_root / "bin" / ("clang++" if language=="cpp" else "clang")
                checks={"clang23":[*clang_prefix,linux(executable),"-std="+mode,"-fsyntax-only",linux(source)]}
            elif language=="javascript":
                checks={"node26":[node,"--check",source]}
            elif language in ("typescript","tsx"):
                options=dict(target="esnext",module="nodenext",moduleResolution="nodenext",strict=True,types=[],noEmit=True,skipLibCheck=True,jsx="preserve",ignoreDeprecations="6.0")
                config=directory/"tsconfig.json"
                config.write_text(json.dumps(dict(compilerOptions=options,files=[source.name])),encoding="utf-8",newline="\n")
                checks={name:[node,compiler,"--project",config,"--pretty","false"] for name,compiler in [("typescript6",ts6),("typescript7",ts7)]}
            elif language=="csharp":
                file_mode="file-based" in pair["profile"]
                arguments=["-nostdlib+","-deterministic+","-utf8output","-langversion:"+pair["profile"].split()[0],"-target:"+("exe" if file_mode else "library"),'-out:"'+str(directory/'Fixture.dll')+'"']
                if file_mode:arguments.append("-features:FileBasedProgram")
                arguments.extend('-reference:"'+str(path)+'"' for path in references)
                arguments.append('"'+str(source)+'"')
                response=directory/"compile.rsp"
                response.write_text("\n".join(arguments)+"\n",encoding="utf-8",newline="\n")
                checks={"roslyn10":[dotnet,csc,"-noconfig","@"+str(response)]}
            else:
                raise ValueError("unqualified language: "+language)
            results={name:run(command,directory,environment) for name,command in checks.items()}
            row=dict(id=pair["id"],side=side,source_digest=digest(source.read_bytes()),profile=pair["profile"],
                     checks=results,expected="accepted",passed=all(result["exit_code"]==0 for result in results.values()),
                     native_target_mode="not_run",runtime_execution="not_run")
            rows.append(row)
            (directory/"receipt.json").write_text(json.dumps(row,indent=2)+"\n",encoding="utf-8",newline="\n")
    receipt=dict(format="entmoot.visitor_compiler_qualification.v1",manifest_digest=digest((out/"input_manifest.json").read_bytes()),
                 rows=rows,passed=all(row["passed"] for row in rows),
                 limits=["Compiler acceptance only: no execution or Entmoot native provider qualification",
                         "Current pinned compiler historical modes are not historical runtime verification",
                         "C# file feature mode does not qualify SDK directive dependency resolution"])
    (out/"receipt.json").write_text(json.dumps(receipt,indent=2)+"\n",encoding="utf-8",newline="\n")
    print(json.dumps(dict(rows=len(rows),passed=sum(row["passed"] for row in rows),receipt=str(out/"receipt.json"))))
    return 0 if receipt["passed"] else 1


if __name__=="__main__":
    raise SystemExit(main())
