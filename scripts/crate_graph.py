"""Workspace package order and dependency-aware CI build cache inputs.

This selects cache namespaces, never which correctness checks to skip. Cargo
still validates fingerprints and every selected target executes on each commit.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def packages(root=ROOT):
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    result = {}
    for member in workspace["members"]:
        manifest = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))
        for table in dependency_tables(manifest, include_dev=True):
            for alias, spec in list(table.items()):
                table[alias] = resolve_dependency(alias, spec, workspace.get("dependencies", {}))
        result[manifest["package"]["name"]] = (root / member, manifest)
    return result


def dependency_tables(manifest, include_dev=False):
    sections = ("dependencies", "build-dependencies", "dev-dependencies") if include_dev else ("dependencies", "build-dependencies")
    for table in (manifest, *manifest.get("target", {}).values()):
        for section in sections:
            yield table.get(section, {})


def resolve_dependency(alias, spec, inherited):
    if isinstance(spec, dict) and spec.get("workspace"):
        if alias not in inherited:
            raise ValueError(f"unresolved workspace dependency: {alias}")
        return inherited[alias]
    return spec


def dependencies(manifest, include_dev=False, inherited=None):
    result = set()
    for table in dependency_tables(manifest, include_dev):
        for alias, spec in table.items():
            spec = resolve_dependency(alias, spec, inherited or {})
            result.add(spec.get("package", alias) if isinstance(spec, dict) else alias)
    return result


def closure(name, graph, include_dev=False):
    found = set()
    pending = [name]
    while pending:
        item = pending.pop()
        if item in found:
            continue
        found.add(item)
        pending.extend(dependencies(graph[item][1], include_dev) & graph.keys() - found)
    return found


def publication_order(graph):
    ordered = []
    pending = set(graph)
    while pending:
        ready = sorted(n for n in pending if not (dependencies(graph[n][1]) & pending))
        if not ready:
            raise ValueError(f"workspace dependency cycle: {sorted(pending)}")
        ordered.extend(ready)
        pending.difference_update(ready)
    return ordered


def cache_key(name, graph, root=ROOT):
    files = {root / "Cargo.lock", root / "Cargo.toml", root / "rust-toolchain.toml"}
    for member in closure(name, graph, include_dev=True):
        directory = graph[member][0]
        files.update(p for p in directory.rglob("*") if p.is_file())
    digest = hashlib.sha256()
    for path in sorted(files):
        if path.is_file():
            for field in (path.relative_to(root).as_posix().encode(), path.read_bytes()):
                digest.update(len(field).to_bytes(8, "little"))
                digest.update(field)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package")
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--matrix", action="store_true")
    args = parser.parse_args()
    graph = packages()
    if args.package:
        value = cache_key(args.package, graph)
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8") as out:
                out.write(f"key={value}\n")
        else:
            print(value)
    elif args.matrix:
        print(json.dumps(publication_order(graph)))
    else:
        print(" ".join(publication_order(graph)))


if __name__ == "__main__":
    main()
