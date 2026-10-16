"""Stream pinned C# examples with exact workflow exclusions and a parse-only Roslyn oracle."""
from __future__ import annotations

import argparse
import base64
from collections import Counter
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from corpus_sources import source_blobs
from grammar_runtime import digest, load_language, runtime_digest
import tree_sitter
from qualify_features import environment, PINS

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SOURCES = ("tree-sitter-c-sharp", "Newtonsoft.Json", "nunit", "orchard", "PowerToys")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, required=True)
    ap.add_argument("--sdk", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    index_path = REPO / "tools/tree-sitter-upstream/checkouts.json"
    index = json.loads(index_path.read_text(encoding="utf-8"))
    grammar_source = next(r for r in index["checkouts"] if r["id"] == SOURCES[0])
    workflow = subprocess.check_output(["git", "-C", grammar_source["path"], "show", grammar_source["commit_sha"] + ":.github/workflows/ci.yml"])
    exclusions = re.findall(r"^\s+!(examples/[^\r\n]+\.cs)\s*$", workflow.decode(), re.MULTILINE)
    if len(set(exclusions)) != len(exclusions) or any(c in p for p in exclusions for c in "*?[{"):
        raise ValueError("workflow exclusions changed; qualify selection semantics")
    excluded_paths = set(exclusions)
    sdk_version, framework_version = PINS["10"]
    sdk_root = args.sdk.resolve(strict=True)
    dotnet = sdk_root / "dotnet.exe"
    roslyn = sdk_root / "sdk" / sdk_version / "Roslyn/bincore"
    compiler = roslyn / "csc.dll"
    references = sorted((sdk_root / "packs/Microsoft.NETCore.App.Ref" / framework_version / "ref/net10.0").glob("*.dll"))
    if not references:
        raise ValueError("pinned references missing")
    oracle_source = ROOT / "file-based-directives/ParseOracle.cs"
    library = out / "ParseOracle.dll"
    response = out / "compile.rsp"
    assemblies = [roslyn / "Microsoft.CodeAnalysis.dll", roslyn / "Microsoft.CodeAnalysis.CSharp.dll"]
    options = ["-nostdlib+", "-target:exe", "-langversion:14", "-deterministic+", '-out:"' + str(library) + '"']
    options.extend('-reference:"' + str(p) + '"' for p in references + assemblies)
    options.append('"' + str(oracle_source) + '"')
    response.write_text("\n".join(options), encoding="utf-8")
    env = environment(sdk_root, out)
    built = subprocess.run([dotnet, compiler, "-noconfig", "@" + str(response)], cwd=out, env=env, capture_output=True, timeout=60)
    (out / "compile.log").write_bytes(built.stdout + built.stderr)
    built.check_returncode()
    for path in assemblies:
        shutil.copyfile(path, out / path.name)
    (out / "ParseOracle.runtimeconfig.json").write_text(json.dumps({"runtimeOptions": {"tfm": "net10.0", "rollForward": "Disable",
        "framework": {"name": "Microsoft.NETCore.App", "version": framework_version}}}), encoding="utf-8")
    libraries = [args.baseline.resolve(strict=True), args.candidate.resolve(strict=True)]
    handles = [load_language(p, "c_sharp") for p in libraries]
    parsers = [tree_sitter.Parser(language) for _, language in handles]
    totals, sources, changed = Counter(), [], []
    files = out / "files.jsonl"
    matched_exclusions = set()
    command = [str(dotnet), str(library)]
    with (out / "oracle-stderr.log").open("wb") as error_log, subprocess.Popen(command, cwd=out, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=error_log, text=True, encoding="utf-8") as oracle, files.open("x", encoding="utf-8", newline="\n") as output:
        for name in SOURCES:
            row = next(r for r in index["checkouts"] if r["id"] == name)
            counts = Counter()
            prefix = "examples/" if name == SOURCES[0] else ""
            sources.append({"id": name, "path": row["path"], "upstream": row["upstream"], "commit": row["commit_sha"],
                            "selector": prefix + "**/*.cs", "counts": counts})
            with source_blobs(row["path"], row["commit_sha"], suffix=".cs", prefix=prefix) as blobs:
                for item, raw in blobs:
                    counts["selected"] += 1
                    if raw is None:
                        counts["excluded_nonregular"] += 1
                        output.write(json.dumps({"corpus": name, **item, "excluded": "nonregular_git_entry"}) + "\n")
                        continue
                    workflow_path = ("examples/" + name + "/" if name != SOURCES[0] else "") + item["path"]
                    excluded = workflow_path in excluded_paths
                    if excluded:
                        matched_exclusions.add(workflow_path)
                    observations = []
                    for parser in parsers:
                        tree = parser.parse(raw)
                        observations.append({"has_error": tree.root_node.has_error, "named_cst_digest": digest(str(tree.root_node).encode())})
                    oracle.stdin.write(base64.b64encode(raw).decode() + "\n")
                    oracle.stdin.flush()
                    response_line = oracle.stdout.readline()
                    if not response_line:
                        raise ValueError("Roslyn oracle stopped before returning evidence")
                    outcome, diagnostic = response_line.rstrip("\n").split("\t", 1)
                    if outcome not in ("accepted", "rejected", "unqualified"):
                        raise ValueError("unknown Roslyn outcome")
                    old, new = observations
                    introduced = not old["has_error"] and new["has_error"]
                    recovered = old["has_error"] and not new["has_error"]
                    result = {"corpus": name, "commit": row["commit_sha"], **item, "source_digest": digest(raw), "bytes": len(raw),
                              "workflow_excluded": excluded, "baseline": old, "candidate": new,
                              "roslyn_parse": {"outcome": outcome, "diagnostic": diagnostic}, "introduced_error": introduced, "recovered_parse": recovered}
                    output.write(json.dumps(result, ensure_ascii=False) + "\n")
                    counts.update(parsed=1, source_bytes=len(raw), baseline_errors=int(old["has_error"]), candidate_errors=int(new["has_error"]),
                                  introduced_errors=int(introduced), recovered_parse=int(recovered), named_cst_changes=int(old["named_cst_digest"] != new["named_cst_digest"]))
                    scope = "workflow_excluded_" if excluded else "workflow_included_"
                    counts[scope + "parsed"] += 1
                    counts[scope + "baseline_errors"] += int(old["has_error"])
                    counts[scope + "candidate_errors"] += int(new["has_error"])
                    counts["roslyn_parse_" + outcome] += 1
                    if introduced or recovered or new["has_error"]:
                        changed.append(result)
            totals.update(counts)
            print(json.dumps({"corpus": name, "counts": counts}), flush=True)
        oracle.stdin.close()
        oracle.stdout.close()
        if oracle.wait(timeout=30):
            raise ValueError("Roslyn parse oracle failed")
    report = {"format": "entmoot.csharp_examples.v1", "driver_digest": digest(Path(__file__).read_bytes()), "runtime_driver_digest": runtime_digest(),
              "source_driver_digest": digest((ROOT.parent / "corpus_sources.py").read_bytes()), "compiler_driver_digest": digest((ROOT / "qualify_features.py").read_bytes()),
              "oracle_digest": digest(oracle_source.read_bytes()), "source_index_digest": digest(index_path.read_bytes()),
              "sdk": {"version": sdk_version, "compiler_digest": digest(compiler.read_bytes()), "dotnet_digest": digest(dotnet.read_bytes()),
                      "assemblies": [{"path": p.name, "digest": digest(p.read_bytes())} for p in assemblies], "oracle_command": command},
              "workflow": {"path": ".github/workflows/ci.yml", "digest": digest(workflow), "exclusions": exclusions,
                           "matched_exclusions": sorted(matched_exclusions), "unmatched_exclusions": sorted(excluded_paths - matched_exclusions)},
              "libraries": [{"path": str(p), "digest": digest(p.read_bytes())} for p in libraries], "sources": sources, "counts": totals,
              "changed_or_error": changed, "files_receipt": {"path": str(files), "digest": digest(files.read_bytes())},
              "passed_no_new_errors": totals["introduced_errors"] == 0,
              "limits": ["All tracked regular C# files parsed; upstream exact exclusions counted separately instead of silently dropped",
                         "CSharpSyntaxTree.ParseText uses C# 14 and FileBasedProgram with no preprocessor symbols; UTF-8 decode failures unqualified",
                         "Roslyn parsing is neither attribution, compilation, dependency resolution nor execution of foreign code",
                         "No Entmoot native resolver, production visitor, historical runtime or whole-pipeline performance qualification"]}
    with (out / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    raise SystemExit(0 if report["passed_no_new_errors"] else 1)


if __name__ == "__main__":
    main()
