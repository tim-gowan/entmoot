#!/usr/bin/env python3
"""Build in-tree resolver adapters into .tmp/adapter-build/resolver-adapters.

Explicit operator setup; init/bind never invokes this builder.
Sources live under tools/resolver-adapters/{go,roslyn,java,clang}/.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SOURCE_ROOT = REPO / "tools" / "resolver-adapters"
DEFAULT_OUT = REPO / ".tmp" / "adapter-build" / "resolver-adapters"


def run(command: list[str], cwd: Path) -> None:
    print("+", " ".join(command), f"(cwd={cwd})")
    subprocess.run(command, cwd=cwd, check=True)


def build_go(src: Path, out: Path) -> Path:
    out.mkdir(parents=True, exist_ok=True)
    exe = "entmoot-go-resolver" + (".exe" if os.name == "nt" else "")
    dest = out / exe
    run(["go", "build", "-o", str(dest), "."], cwd=src)
    return dest


def build_roslyn(src: Path, out: Path) -> Path:
    run(["dotnet", "build", "-c", "Release", "-o", str(out / "bin" / "Release" / "net9.0")], cwd=src)
    dll = out / "bin" / "Release" / "net9.0" / "Entmoot.RoslynSolver.dll"
    # Older builds copied compiler API assemblies here. Locator must load the
    # selected installed SDK; these four managed leftovers can shadow that SDK.
    for name in ("Microsoft.Build", "Microsoft.Build.Framework", "Microsoft.Build.Tasks.Core", "Microsoft.Build.Utilities.Core"):
        dll.with_name(name + ".dll").unlink(missing_ok=True)
    exe = dll.with_suffix(".exe")
    if dll.is_file():
        return dll
    if exe.is_file():
        return exe
    raise SystemExit(f"Roslyn build missing DLL under {out}")


def build_java(src: Path, out: Path) -> Path:
    mvn = shutil.which("mvn.cmd" if os.name == "nt" else "mvn")
    if not mvn:
        raise SystemExit("mvn not on PATH (JDK 17+ required for Java adapter jar)")
    run([mvn, "-q", "-DskipTests", "package"], cwd=src)
    jar_candidates = [
        src / "entmoot-java-resolver.jar",
        src / "target" / "entmoot-java-resolver.jar",
    ]
    if (src / "target").is_dir():
        jar_candidates.extend(sorted((src / "target").glob("*-resolver*.jar")))
    if (src / "build" / "libs").is_dir():
        jar_candidates.extend(sorted((src / "build" / "libs").glob("*.jar")))
    jar = next((path for path in jar_candidates if path.is_file()), None)
    if jar is None:
        raise SystemExit(f"Java package produced no jar under {src}")
    out.mkdir(parents=True, exist_ok=True)
    dest = out / "entmoot-java-resolver.jar"
    shutil.copy2(jar, dest)
    return dest


def build_clang(src: Path, out: Path) -> Path:
    script = src / "entmoot-clang-resolver.py"
    if not script.is_file():
        raise SystemExit(f"missing {script}")
    compile(script.read_text(encoding="utf-8"), str(script), "exec")
    out.mkdir(parents=True, exist_ok=True)
    dest = out / "entmoot-clang-resolver.py"
    shutil.copy2(script, dest)
    return dest


BUILDERS = {
    "go": build_go,
    "roslyn": build_roslyn,
    "java": build_java,
    "clang": build_clang,
}

ENV_KEYS = {
    "go": "ENTMOOT_GO_ADAPTER",
    "roslyn": "ENTMOOT_ROSLYN_ADAPTER",
    "java": "ENTMOOT_JAVA_ADAPTER",
    "clang": "ENTMOOT_CLANG_ADAPTER",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source",
        type=Path,
        default=SOURCE_ROOT,
        help="tools/resolver-adapters tree (default: in-repo)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=DEFAULT_OUT,
        help="build output root (default: .tmp/adapter-build/resolver-adapters)",
    )
    parser.add_argument(
        "--families",
        nargs="+",
        choices=sorted(BUILDERS),
        default=sorted(BUILDERS),
    )
    args = parser.parse_args()
    source = args.source.resolve()
    out = args.out.resolve()
    if not source.is_dir():
        parser.error(f"--source missing: {source}")
    environment: dict[str, str] = {}
    for family in args.families:
        family_src = source / family
        family_out = out / family
        if not family_src.is_dir():
            raise SystemExit(f"missing family source: {family_src}")
        family_out.mkdir(parents=True, exist_ok=True)
        # Build systems may write obj/, target/ or generated files beside sources.
        # A fresh working copy keeps these under the selected task output root.
        with tempfile.TemporaryDirectory(prefix="build-", dir=family_out) as work:
            work_path = Path(work).resolve()
            if not work_path.is_relative_to(out) or work_path.is_symlink():
                raise SystemExit(f"build scratch escapes output root: {work_path}")
            working_source = work_path / "source"
            shutil.copytree(family_src, working_source,
                            ignore=shutil.ignore_patterns("bin", "obj", "target", "build", "__pycache__"))
            artifact = BUILDERS[family](working_source, family_out)
        environment[ENV_KEYS[family]] = str(artifact)
    receipt = {
        "source": str(source),
        "out": str(out),
        "families": args.families,
        "environment": environment,
        "hint": "export these or set ENTMOOT_ADAPTERS_ROOT=" + str(out),
    }
    out.mkdir(parents=True, exist_ok=True)
    (out / "build-receipt.json").write_text(
        json.dumps(receipt, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
