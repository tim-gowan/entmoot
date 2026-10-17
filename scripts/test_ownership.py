#!/usr/bin/env python3
"""Load and query entmoot.test_ownership.v1 (File/RepoPath owners → cargo targets)."""

from __future__ import annotations

import json
import re
import tomllib
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Any

OWNERSHIP_SCHEMA = "entmoot.test_ownership.v1"
DEFAULT_REGISTRY = Path(__file__).resolve().parent / "test_ownership.json"
CORE_CARGO_TOML = (
    Path(__file__).resolve().parents[1] / "crates" / "entmoot-core" / "Cargo.toml"
)

WORKSPACE_PACKAGES: tuple[str, ...] = (
    "entmoot-contracts",
    "entmoot-core",
    "entmoot-engine",
    "entmoot",
    "entmoot-mcp",
)


@dataclass(frozen=True)
class CargoTarget:
    package: str
    test: str | None = None
    kind: str = "package"

    def key(self) -> str:
        return f"{self.package}:{self.test or ('*' if self.kind == 'package' else '@' + self.kind)}"

    def as_dict(self) -> dict[str, str]:
        if self.kind != "package":
            return {"package": self.package, "kind": self.kind}
        if self.test is None:
            return {"package": self.package}
        return {"package": self.package, "test": self.test}


@dataclass(frozen=True)
class OwnerRef:
    kind: str
    path: str


@dataclass
class OwnershipTarget:
    cargo: CargoTarget
    owners: list[OwnerRef]
    extensions: list[str]
    also_select: list[CargoTarget]
    workspace_expand: bool
    notes: str | None


@dataclass
class OwnershipRegistry:
    skip_prefixes: list[str]
    shared_core_owners: list[str]
    shared_core_tests: list[str]
    targets: list[OwnershipTarget]
    path: Path

    def is_skip_path(self, path: str) -> bool:
        normalized = path.replace("\\", "/")
        # Explicit consumers take precedence over broad documentation skips.
        if any(self.path_matches_owner(normalized, owner)
               for entry in self.targets for owner in entry.owners
               if owner.kind == "file_prefix" and not entry.workspace_expand):
            return False
        if normalized.endswith(".md"):
            return True
        for prefix in self.skip_prefixes:
            if normalized == prefix or normalized.startswith(prefix):
                return True
        return False

    def path_matches_owner(self, path: str, owner: OwnerRef) -> bool:
        normalized = path.replace("\\", "/")
        owner_path = owner.path.replace("\\", "/")
        if owner.kind == "shared_core":
            # shared_core is the curated shared_core_owners set, not a blanket src/ prefix
            return self.is_shared_core_path(normalized)
        if owner_path.endswith("/"):
            return normalized.startswith(owner_path) or normalized + "/" == owner_path
        return normalized == owner_path or normalized.startswith(owner_path + "/")
    def is_shared_core_path(self, path: str) -> bool:
        normalized = path.replace("\\", "/")
        if normalized in (
            "crates/entmoot-core/src/languages/mod.rs",
            "crates/entmoot-core/src/languages/plugin.rs",
        ):
            return True
        for prefix in self.shared_core_owners:
            if normalized == prefix.rstrip("/") or normalized.startswith(prefix):
                return True
        if not normalized.startswith("crates/entmoot-core/"):
            return False
        for entry in self.targets:
            for owner in entry.owners:
                if owner.kind == "language_plugin" and self.path_matches_owner(
                    normalized, owner
                ):
                    return False
        return True

    def targets_for_path(self, path: str) -> list[CargoTarget]:
        """Return cargo targets owned by this path (modified/deleted)."""
        normalized = path.replace("\\", "/")
        selected: list[CargoTarget] = []
        seen: set[str] = set()

        def add(target: CargoTarget) -> None:
            key = target.key()
            if key in seen or target.test == "_workspace_root":
                return
            seen.add(key)
            selected.append(target)

        if self.is_skip_path(normalized):
            return []

        root = self.path.parents[1]
        manifests = workspace_manifests(root)
        # Cargo declares the executable boundary; a helper or fixture stem is
        # never invented as a test target. Removed targets fall back below.
        for package, manifest in manifests.items():
            for name, source in integration_targets(root, package, manifest).items():
                if normalized == source:
                    add(CargoTarget(package, name))
                    for entry in self.targets:
                        if entry.cargo == CargoTarget(package, name):
                            for extra in entry.also_select:
                                add(extra)
                    return selected

        # Workspace expand (Cargo.toml / .github)
        for entry in self.targets:
            if not entry.workspace_expand:
                continue
            if any(self.path_matches_owner(normalized, owner) for owner in entry.owners):
                for package in WORKSPACE_PACKAGES:
                    add(CargoTarget(package))
                return selected

        # Direct owner matches
        for entry in self.targets:
            if entry.workspace_expand:
                continue
            matched = any(self.path_matches_owner(normalized, owner) for owner in entry.owners)
            if matched:
                add(entry.cargo)
                for extra in entry.also_select:
                    add(extra)

        # Shared-core path → all shared_core_tests
        if self.is_shared_core_path(normalized) and not (selected and normalized.startswith("crates/entmoot-core/tests/")):
            for test_name in self.shared_core_tests:
                add(CargoTarget("entmoot-core", test_name))

        # Extension match for non-crate paths
        if not normalized.startswith("crates/"):
            lower = normalized.lower()
            for entry in self.targets:
                for ext in entry.extensions:
                    if lower.endswith(ext):
                        add(entry.cargo)
                        break

        # Source/library or manifest changes can affect consumers across crates.
        # Test-only edits do not change the dependency's production interface.
        changed = next((p for p in manifests if normalized.startswith(f"crates/{p}/src/")
                        or normalized in {f"crates/{p}/Cargo.toml", f"crates/{p}/build.rs"}), None)
        if changed:
            if not selected:
                add(CargoTarget(changed))
            if not any(t.package == changed and t.test is None and t.kind == "package" for t in selected):
                add(CargoTarget(changed, kind="lib"))
                add(CargoTarget(changed, kind="doc"))
            affected = {changed}
            dependencies = workspace_dependencies(root, manifests)
            while True:
                consumers = {p for p, deps in dependencies.items() if deps & affected}
                if consumers <= affected:
                    break
                affected.update(consumers)
            for package in sorted(affected - {changed}):
                add(CargoTarget(package))

        # Only the validated registry names Cargo targets. Files under tests/
        # also include shared modules and source/JSON fixtures; their stems are
        # not integration targets. The owner rules above select their consumers.
        return selected

    def impact_request(self, paths: list[str]) -> dict[str, Any]:
        """Export the same ownership floor consumed by the hook for a graph pair.

        Paths from both sides preserve removed fixtures. Unknown paths remain
        unmapped so the engine widens; this never invents occurrence coverage.
        """
        targets: dict[str, dict[str, Any]] = {}
        for path in sorted(set(paths)):
            for cargo in self.targets_for_path(path):
                key = cargo.key()
                if key not in targets:
                    targets[key] = {
                        "id": key, "runner": "cargo",
                        "command": cargo_commands_for_targets([cargo], self.path.parents[1])[0],
                        "owners": [], "fixtures": [], "occurrences": [],
                    }
                targets[key]["owners"].append(path)
        return {"targets": [targets[key] for key in sorted(targets)]}

    def owner_paths_for_target(self, cargo: CargoTarget) -> list[str]:
        paths: list[str] = []
        for entry in self.targets:
            if entry.cargo.key() != cargo.key():
                continue
            for owner in entry.owners:
                if owner.kind == "shared_core":
                    paths.extend(self.shared_core_owners)
                else:
                    paths.append(owner.path)
        return paths

    def obsolete_targets(
        self, deleted_paths: list[str], head_exists: set[str] | None = None
    ) -> list[dict[str, Any]]:
        """Cargo targets whose every concrete owner path is deleted and absent on head."""
        deleted = {p.replace("\\", "/") for p in deleted_paths}
        obsolete: list[dict[str, Any]] = []
        for entry in self.targets:
            if entry.workspace_expand or entry.cargo.test == "_workspace_root":
                continue
            concrete = [
                o.path.replace("\\", "/")
                for o in entry.owners
                if o.kind != "shared_core"
            ]
            if not concrete:
                continue
            # Sole-owner delete: every concrete owner was deleted and none exist on head
            all_deleted = all(
                any(
                    d == owner or owner.startswith(d.rstrip("/") + "/") or d.startswith(owner.rstrip("/") + "/")
                    for d in deleted
                )
                for owner in concrete
            )
            if not all_deleted:
                continue
            if head_exists is not None:
                still_present = any(
                    owner in head_exists
                    or any(h.startswith(owner.rstrip("/") + "/") for h in head_exists)
                    for owner in concrete
                )
                if still_present:
                    continue
            obsolete.append(
                {
                    "cargo": entry.cargo.as_dict(),
                    "deleted_owners": concrete,
                    "reason": "all concrete File/RepoPath owners deleted with no remaining owner on head",
                }
            )
        return obsolete

    def ownerless_targets(self, repo_root: Path) -> list[dict[str, Any]]:
        """Registry entries whose owner paths are missing on disk (drift)."""
        missing: list[dict[str, Any]] = []
        for entry in self.targets:
            if entry.workspace_expand:
                continue
            absent: list[str] = []
            for owner in entry.owners:
                if owner.kind == "shared_core":
                    continue
                path = repo_root / owner.path.rstrip("/")
                if not path.exists():
                    absent.append(owner.path)
            if absent and len(absent) == len(
                [o for o in entry.owners if o.kind != "shared_core"]
            ):
                missing.append({"cargo": entry.cargo.as_dict(), "missing_owners": absent})
        return missing


def _parse_target(raw: dict[str, Any]) -> OwnershipTarget:
    cargo_raw = raw["cargo"]
    cargo = CargoTarget(cargo_raw["package"], cargo_raw.get("test"), cargo_raw.get("kind", "package"))
    owners = [OwnerRef(o["kind"], o["path"]) for o in raw.get("owners", [])]
    also = [
        CargoTarget(a["package"], a.get("test")) for a in raw.get("also_select", [])
    ]
    return OwnershipTarget(
        cargo=cargo,
        owners=owners,
        extensions=list(raw.get("extensions", [])),
        also_select=also,
        workspace_expand=bool(raw.get("workspace_expand", False)),
        notes=raw.get("notes"),
    )


def load_registry(path: Path | None = None) -> OwnershipRegistry:
    registry_path = path or DEFAULT_REGISTRY
    payload = json.loads(registry_path.read_text(encoding="utf-8"))
    if payload.get("schema_version") != OWNERSHIP_SCHEMA:
        raise ValueError(
            f"unsupported ownership schema {payload.get('schema_version')!r}; "
            f"expected {OWNERSHIP_SCHEMA}"
        )
    return OwnershipRegistry(
        skip_prefixes=list(payload.get("skip_prefixes", [])),
        shared_core_owners=list(payload.get("shared_core_owners", [])),
        shared_core_tests=list(payload.get("shared_core_tests", [])),
        targets=[_parse_target(t) for t in payload.get("targets", [])],
        path=registry_path,
    )


def declared_core_tests(cargo_toml: Path | None = None) -> list[str]:
    text = (cargo_toml or CORE_CARGO_TOML).read_text(encoding="utf-8")
    names: list[str] = []
    in_test = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped == "[[test]]":
            in_test = True
            continue
        if stripped.startswith("[") and stripped != "[[test]]":
            in_test = False
        if in_test:
            match = re.match(r'^name\s*=\s*"([^"]+)"\s*$', stripped)
            if match:
                names.append(match.group(1))
                in_test = False
    return names


def validate_registry(registry: OwnershipRegistry, repo_root: Path | None = None) -> list[str]:
    """Return human-readable validation errors (empty if ok)."""
    root = repo_root or registry.path.parents[1]
    errors: list[str] = []
    test_section_names = set(declared_core_tests())

    covered = {
        t.cargo.test
        for t in registry.targets
        if t.cargo.package == "entmoot-core"
        and t.cargo.test
        and t.cargo.test != "_workspace_root"
    }
    missing = sorted(test_section_names - covered)
    extra = sorted(covered - test_section_names)
    if missing:
        errors.append(f"registry missing entmoot-core tests: {', '.join(missing)}")
    if extra:
        errors.append(f"registry has unknown entmoot-core tests: {', '.join(extra)}")

    package_targets = {
        t.cargo.package for t in registry.targets if t.cargo.test is None and not t.workspace_expand
    }
    for required in ("entmoot-contracts", "entmoot-engine", "entmoot", "entmoot-mcp"):
        if required not in package_targets:
            errors.append(f"registry missing workspace package target: {required}")

    for entry in registry.ownerless_targets(root):
        errors.append(
            f"ownerless {entry['cargo']}: missing {', '.join(entry['missing_owners'])}"
        )
    return errors


def package_directory(root: Path, package: str) -> Path:
    direct = root / "crates" / package
    if direct.is_dir():
        return direct
    return root / "crates" / ("entmoot-cli" if package == "entmoot" else package)


def workspace_manifests(root: Path) -> dict[str, dict[str, Any]]:
    return {p: _parse_manifest((package_directory(root, p) / "Cargo.toml").read_bytes())
            for p in WORKSPACE_PACKAGES if (package_directory(root, p) / "Cargo.toml").is_file()}


def workspace_dependencies(root: Path, manifests: dict[str, dict[str, Any]]) -> dict[str, set[str]]:
    """Keep all configurations/optional edges; resolve Cargo aliases and inheritance."""
    root_manifest = root / "Cargo.toml"
    inherited = (_parse_manifest(root_manifest.read_bytes()).get("workspace", {}).get("dependencies", {})
                 if root_manifest.is_file() else {})
    result: dict[str, set[str]] = {}
    for package, manifest in manifests.items():
        dependencies: set[str] = set()
        for table in [manifest, *manifest.get("target", {}).values()]:
            for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                for alias, spec in table.get(section, {}).items():
                    if isinstance(spec, dict) and spec.get("workspace"):
                        if alias not in inherited:
                            raise ValueError(f"unresolved workspace dependency: {package}/{alias}")
                        spec = inherited[alias]
                    name = spec.get("package", alias) if isinstance(spec, dict) else alias
                    if name in manifests:
                        dependencies.add(name)
        result[package] = dependencies
    return result


@lru_cache(maxsize=32)
def _parse_manifest(source: bytes) -> dict[str, Any]:
    # Content-keyed: changed/new dependencies invalidate even with unchanged mtimes.
    return tomllib.loads(source.decode("utf-8"))


def integration_targets(root: Path, package: str, manifest: dict[str, Any]) -> dict[str, str]:
    directory = package_directory(root, package)
    targets = {t["name"]: directory.relative_to(root).as_posix() + "/" + t.get("path", f"tests/{t['name']}.rs")
               for t in manifest.get("test", [])}
    if manifest.get("package", {}).get("autotests", True):
        for source in (directory / "tests").glob("*.rs"):
            targets.setdefault(source.stem, source.relative_to(root).as_posix())
    return targets


def cargo_commands_for_targets(targets: list[CargoTarget], root: Path = DEFAULT_REGISTRY.parent.parent) -> list[list[str]]:
    """Batch whole packages; keep named targets scoped to their Cargo package."""
    full = sorted({t.package for t in targets if t.test is None and t.kind == "package"})
    def prefix_for(packages: list[str]) -> list[str]:
        return ["cargo", "test"] + (["--features", "entmoot-core/full-tests"]
                                   if any(p != "entmoot-contracts" for p in packages) else [])
    commands = [prefix_for(full) + [arg for p in full for arg in ("-p", p)]] if full else []
    # Ignored, explicitly owned campaigns remain required even if normal package
    # tests subsume their nonignored siblings. Do not enable every native probe.
    for target in targets:
        if target.package in full and target.test == "campaign_harness":
            # Keep the same roots: dropping the MCP/dev dependencies here changes
            # feature unification and recompiles the libraries just qualified.
            command = prefix_for(full) + [arg for p in full for arg in ("-p", p)] + ["--test", target.test, "--", "--include-ignored"]
            if command not in commands:
                commands.append(command)
    for package in sorted({t.package for t in targets} - set(full)):
        prefix = prefix_for([package])
        selected = {t for t in targets if t.package == package}
        flags = [arg for name in sorted({t.test for t in selected if t.test}) for arg in ("--test", name)]
        if any(t.kind == "lib" for t in selected):
            flags.append("--lib")
        if flags:
            command = prefix + ["-p", package] + flags
            if any(t.test == "campaign_harness" for t in selected):
                command += ["--", "--include-ignored"]
            commands.append(command)
        if any(t.kind == "doc" for t in selected):
            commands.append(prefix + ["-p", package, "--doc"])
    # Cargo owns target requirements. Preserve them even when a named target is
    # subsumed into a package-wide invocation; otherwise cfg gates can run zero tests.
    manifests = workspace_manifests(root)
    requirements = {(package, test["name"]): test.get("required-features", [])
                    for package, manifest in manifests.items() for test in manifest.get("test", [])}
    for command in commands:
        packages = {command[i + 1] for i, arg in enumerate(command[:-1]) if arg == "-p"}
        names = {command[i + 1] for i, arg in enumerate(command[:-1]) if arg == "--test"}
        selected = [t for t in targets if t.package in packages and t.test
                    and (not names or t.test in names)]
        features = sorted({f"{t.package}/{feature}" for t in selected
                           for feature in requirements.get((t.package, t.test), [])})
        if features:
            feature = ",".join(features)
            if "--features" in command:
                command[command.index("--features") + 1] += "," + feature
            else:
                command[2:2] = ["--features", feature]
    return commands


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="Export the validated ownership floor for entmoot assess impact --tests")
    parser.add_argument("--base-graph", type=Path, required=True)
    parser.add_argument("--head-graph", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    registry = load_registry()
    errors = validate_registry(registry)
    if errors:
        parser.error("; ".join(errors))
    paths = [entry["path"] for graph in (args.base_graph, args.head_graph)
             for entry in json.loads(graph.read_text(encoding="utf-8"))["snapshot_files"]]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(registry.impact_request(paths), indent=2) + "\n", encoding="utf-8")
