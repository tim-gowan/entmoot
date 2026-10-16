"""Qualify authored C# CST controls against pinned compilers and offline file SDKs."""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from grammar_runtime import digest, endpoint, load_language, nodes, runtime_digest, shape
import tree_sitter

ROOT = Path(__file__).resolve().parent
PINS = {"9": ("9.0.318", "9.0.20"), "10": ("10.0.401", "10.0.12"),
        "11": ("11.0.100-rc.1.26425.128", "11.0.0-rc.1.26425.128")}
HELPER = "public static class Helper { public static int Value => 7; }\n"


def environment(root, directory):
    env = {k: v for k, v in os.environ.items() if k.upper() not in
           ("MSBUILDSDKSPATH", "MSBUILD_EXE_PATH", "DOTNET_ROLL_FORWARD", "DOTNET_STARTUP_HOOKS", "DOTNET_ADDITIONAL_DEPS", "DOTNET_SHARED_STORE")}
    for name in ("home", "tmp", "packages"):
        (directory / name).mkdir(exist_ok=True)
    env.update(DOTNET_ROOT=str(root), DOTNET_ROOT_X64=str(root), DOTNET_CLI_HOME=str(directory / "home"),
               DOTNET_NOLOGO="1", DOTNET_CLI_TELEMETRY_OPTOUT="1", DOTNET_SKIP_FIRST_TIME_EXPERIENCE="1",
               DOTNET_MULTILEVEL_LOOKUP="0", DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE="1",
               NUGET_PACKAGES=str(directory / "packages"), TEMP=str(directory / "tmp"), TMP=str(directory / "tmp"),
               MSBUILDDISABLENODEREUSE="1")
    return env


def run(command, directory, env, name):
    start = time.perf_counter()
    process = subprocess.run(list(map(str, command)), cwd=directory, env=env, capture_output=True, timeout=180)
    raw = process.stdout + process.stderr
    (directory / (name + ".log")).write_bytes(raw)
    return {"command": list(map(str, command)), "exit_code": process.returncode,
            "output": raw.decode("utf-8", "replace"), "log_digest": digest(raw), "wall_seconds": time.perf_counter() - start}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    for version in PINS:
        ap.add_argument("--sdk" + version, type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    fixture_path = ROOT / "file-based-directives/cases.py"
    spec = importlib.util.spec_from_file_location("csharp_cases", fixture_path)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    toolchains = {}
    tools = []
    for branch, (sdk_version, framework_version) in PINS.items():
        root = getattr(args, "sdk" + branch).resolve(strict=True)
        dotnet = root / ("dotnet.exe" if os.name == "nt" else "dotnet")
        compiler = root / "sdk" / sdk_version / "Roslyn/bincore/csc.dll"
        reference_root = root / "packs/Microsoft.NETCore.App.Ref" / framework_version / "ref" / ("net" + branch + ".0")
        references = sorted(reference_root.glob("*.dll"))
        if not compiler.is_file() or not references or not (root / "shared/Microsoft.NETCore.App" / framework_version).is_dir():
            raise ValueError("missing pinned compiler/reference/runtime: " + branch)
        directory = out / ("sdk-" + branch)
        directory.mkdir()
        env = environment(root, directory)
        version = run([dotnet, compiler, "-version"], directory, env, "compiler-version")
        if version["exit_code"]:
            raise ValueError("compiler cannot run")
        toolchains[branch] = (root, dotnet, compiler, references)
        tools.append({"branch": branch, "sdk": sdk_version, "framework": framework_version, "compiler_version": version,
                      "dotnet_digest": digest(dotnet.read_bytes()), "compiler_digest": digest(compiler.read_bytes()),
                      "references": [{"path": p.name, "digest": digest(p.read_bytes())} for p in references],
                      "core_runtime_digest": digest((root / "shared/Microsoft.NETCore.App" / framework_version / "System.Private.CoreLib.dll").read_bytes())})
    libraries = [p.resolve(strict=True) for p in (args.baseline, args.candidate)]
    handles = [load_language(p, "c_sharp") for p in libraries]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    rows, identities = [], set()
    for case in fixture.cases():
        if case["id"] in identities or not case["id"].replace("-", "").isalnum():
            raise ValueError("invalid or duplicate case identity")
        identities.add(case["id"])
        directory = out / case["id"]
        directory.mkdir()
        root, dotnet, compiler, references = toolchains[case["sdk"]]
        sdk_version, framework_version = PINS[case["sdk"]]
        env = environment(root, directory)
        source = directory / "Fixture.cs"
        raw = case["source"].encode("utf-8")
        source.write_bytes(raw)
        assembly = directory / "Fixture.dll"
        compile_args = ["-nostdlib+", "-target:exe", "-deterministic+", "-utf8output", "-langversion:" + case["language_mode"],
                        '-out:"' + str(assembly) + '"']
        compile_args.extend('-reference:"' + str(p) + '"' for p in references)
        if case["file_mode"]:
            compile_args.append("-features:FileBasedProgram")
        helper_receipt = None
        if case["helper"]:
            helper = directory / "helper"
            helper.mkdir()
            helper_source = helper / "Helper.cs"
            helper_source.write_text(HELPER, encoding="utf-8")
            # Compiler controls explicitly supply this source. A directive's
            # spelling alone never creates a semantic reference.
            compile_args.append('"' + str(helper_source) + '"')
            helper_receipt = {"source": HELPER, "source_digest": digest(helper_source.read_bytes())}
        compile_args.append('"' + str(source) + '"')
        response = directory / "compile.rsp"
        response.write_text("\n".join(compile_args) + "\n", encoding="utf-8")
        compiled = run([dotnet, compiler, "-noconfig", "@" + str(response)], directory, env, "compile")
        compiled["response_file"] = {"content": response.read_text(encoding="utf-8"), "digest": digest(response.read_bytes())}
        compiled["accepted"] = compiled["exit_code"] == 0
        compiled["passed"] = compiled["accepted"] == case["compiler_accepts"] and all(c in compiled["output"] for c in case["diagnostic_codes"])
        execution = None
        if compiled["accepted"]:
            config = {"runtimeOptions": {"tfm": "net" + case["sdk"] + ".0", "rollForward": "Disable",
                      "framework": {"name": "Microsoft.NETCore.App", "version": framework_version}}}
            (directory / "Fixture.runtimeconfig.json").write_text(json.dumps(config), encoding="utf-8")
            execution = run([dotnet, assembly], directory, env, "execute")
            execution["passed"] = execution["exit_code"] == 0 and execution["output"].strip() == "oracle:ok"
        sdk = None
        if case["sdk_accepts"] is not None:
            feed = directory / "feed"
            feed.mkdir()
            nuget = directory / "NuGet.Config"
            nuget.write_text('<configuration><packageSources><clear/><add key="local" value="' + str(feed) + '"/></packageSources></configuration>', encoding="utf-8")
            (directory / "global.json").write_text(json.dumps({"sdk": {"version": sdk_version, "rollForward": "disable"}}), encoding="utf-8")
            project = directory / "helper/Helper.csproj"
            if case["helper"]:
                project.write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><NuGetAudit>false</NuGetAudit><UseSharedCompilation>false</UseSharedCompilation></PropertyGroup></Project>', encoding="utf-8")
            if case["helper"] == "package":
                build = run([dotnet, "build", project, "-p:RestoreConfigFile=" + str(nuget), "-p:ImportDirectoryBuildProps=false", "-p:ImportDirectoryBuildTargets=false"], directory, env, "helper-build")
                helper_receipt["build"] = build
                if build["exit_code"]:
                    raise ValueError("owned local package could not be built")
                package = feed / "Entmoot.Oracle.1.0.0.nupkg"
                with zipfile.ZipFile(package, "x") as archive:
                    for name, data in [("Entmoot.Oracle.nuspec", b'<package><metadata><id>Entmoot.Oracle</id><version>1.0.0</version><authors>Entmoot fixture</authors><description>Authored offline qualification input</description></metadata></package>'),
                                       ("lib/net10.0/Helper.dll", (directory / "helper/bin/Debug/net10.0/Helper.dll").read_bytes())]:
                        entry = zipfile.ZipInfo(name, date_time=(2026, 10, 10, 0, 0, 0))
                        archive.writestr(entry, data)
                helper_receipt["package_digest"] = digest(package.read_bytes())
            sdk = run([dotnet, "run", "--file", source, "--no-launch-profile", "-p:PublishAot=false", "-p:NuGetAudit=false",
                       "-p:RestoreConfigFile=" + str(nuget), "-p:ImportDirectoryBuildProps=false", "-p:ImportDirectoryBuildTargets=false",
                       "-p:UseSharedCompilation=false"], directory, env, "sdk")
            sdk["accepted"] = sdk["exit_code"] == 0
            output_lines = sdk["output"].strip().splitlines()
            # SDK build warnings remain in the receipt. The executed program
            # must still produce its single exact final oracle line.
            sdk["passed"] = sdk["accepted"] == case["sdk_accepts"] and (not sdk["accepted"] or
                             (output_lines[-1:] == ["oracle:ok"] and output_lines.count("oracle:ok") == 1))
            if case["sdk_diagnostic"]:
                sdk["passed"] = sdk["passed"] and case["sdk_diagnostic"].lower() in sdk["output"].lower()
        observations = []
        for parser in parsers:
            tree = parser.parse(raw)
            valid = not tree.root_node.has_error
            after = [n for n in tree.root_node.named_children if n.type == "class_declaration"
                     and n.child_by_field_name("name") is not None and n.child_by_field_name("name").text == b"After"]
            sibling = len(after) == 1 and after[0].start_byte == raw.index(b"class After")
            ok, sites = valid == case["parse_accepts"] and sibling, []
            for expected in case["sites"]:
                text = expected["text"].encode()
                if raw.count(text) != 1:
                    raise ValueError("site must select exactly one source occurrence")
                begin = raw.index(text)
                matches = [n for n in nodes(tree.root_node) if n.start_byte == begin and n.end_byte == begin + len(text) and n.type == expected["kind"]]
                site_ok, fields = len(matches) == 1, {}
                if site_ok:
                    site_ok = tuple(matches[0].start_point) == endpoint(raw[:begin]) and tuple(matches[0].end_point) == endpoint(raw[:begin + len(text)])
                    for name, value in expected["fields"].items():
                        selected = matches[0].child_by_field_name(name)
                        fields[name] = selected.text.decode("utf-8") if selected else None
                        site_ok = site_ok and fields[name] == value
                sites.append({"expected": expected, "passed": site_ok, "observed_fields": fields})
                ok = ok and site_ok
            recovery = None
            if valid:
                begin = raw.index(b"class After")
                insertion = b"class Broken { void missing(\n"
                dirty = raw[:begin] + insertion + raw[begin:]
                tree.edit(begin, begin, begin + len(insertion), endpoint(raw[:begin]), endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]))
                edited = parser.parse(dirty, tree)
                malformed = edited.root_node.has_error
                edited.edit(begin, begin + len(insertion), begin, endpoint(raw[:begin]), endpoint(dirty[:begin + len(insertion)]), endpoint(raw[:begin]))
                recovery = malformed and shape(parser.parse(raw, edited)) == shape(parser.parse(raw))
            observations.append({"has_error": not valid, "expected_cst_met": ok, "sibling_preserved": sibling, "sites": sites,
                                 "incremental_recovery": recovery, "cst_digest": digest(json.dumps(shape(parser.parse(raw)), separators=(",", ":")).encode())})
        rows.append({"expectation": case, "source_digest": digest(raw), "baseline": observations[0], "candidate": observations[1],
                     "compiler": compiled, "runtime": execution, "sdk": sdk, "helper": helper_receipt, "native_resolver": "not_run"})
        print(json.dumps({"case": case["id"], "compiler": compiled["passed"], "runtime": execution["passed"] if execution else None,
                          "sdk": sdk["passed"] if sdk else None, "baseline": observations[0]["expected_cst_met"], "candidate": observations[1]["expected_cst_met"]}), flush=True)
    passed = all(r["compiler"]["passed"] and (r["runtime"] is None or r["runtime"]["passed"]) and (r["sdk"] is None or r["sdk"]["passed"])
                 and r["candidate"]["expected_cst_met"] and r["candidate"]["incremental_recovery"] is not False for r in rows)
    stable_passed = all(r["compiler"]["passed"] and (r["runtime"] is None or r["runtime"]["passed"]) and (r["sdk"] is None or r["sdk"]["passed"])
                        and r["candidate"]["expected_cst_met"] and r["candidate"]["incremental_recovery"] is not False
                        for r in rows if r["expectation"]["language_mode"] != "preview")
    report = {"format": "entmoot.csharp_feature_qualification.v1", "driver_digest": digest(Path(__file__).read_bytes()),
              "runtime_driver_digest": runtime_digest(), "fixture_digest": digest(fixture_path.read_bytes()),
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries], "tools": tools, "cases": rows,
              "all_requested_controls_passed": passed, "stable_controls_passed": stable_passed,
              "limits": ["Explicit pinned reference assemblies; no ambient or directive-inferred compiler reference",
                         "Actual SDK/runtime 9, 10, and preview 11 are separate from each selected language mode",
                         "FileBasedProgram enables compiler acceptance; the independent offline SDK interprets directive payloads",
                         "Local package/project/include inputs are authored fixtures, not foreign dependencies",
                         "Preview never implies stable support; no native Entmoot resolver or production visitor qualification"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if stable_passed else 1)


if __name__ == "__main__":
    main()
