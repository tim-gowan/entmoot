#!/usr/bin/env python3
"""Pre-commit gate: policy checks, then scoped or workspace clippy+test.

Policy ownership: Cargo.toml sha2/sha256 bans (Python), AST policy via
`cargo dylint` (tools/entmoot-lints), licenses via `cargo deny --exclude-dev
check licenses` (deny.toml; copyleft not allowlisted), unused deps via
`cargo machete`.

Changed-file ownership is the minimum. Structural hashes cannot waive tests.
Unavailable ownership widens to the workspace. Installed native controls are
optional; authoritative CI always runs its full configured lanes.
Authoritative `ci.yml` is never taught to skip.
"""

from __future__ import annotations

import os
import json
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

# Identity digests must stay BLAKE3 — ban SHA-256 identity crates in manifests.
# Import/Sync AST policy lives in tools/entmoot-lints (cargo dylint).
BANNED_DEP_NAMES = ("sha2", "sha256")

# Local data roots may contain installed tools and third-party adapter sources.
SKIP_DIRS = {".git", ".tmp", ".entmoot", ".venv", "venv", "node_modules", "target", "archive"}
MARKDOWN_LINK_RE = re.compile(r"!?\[[^\]]*\]\((?P<target>[^)]+)\)")
CANONICAL_CRATES = {
    "entmoot-contracts",
    "entmoot-core",
    "entmoot-engine",
    "entmoot",
    "entmoot-mcp",
}
RETIRED_CRATES = {"entmoot-schema", "entmoot-query", "entmoot-topology-poc"}
DYLINT_CMD = ["cargo", "dylint", "--all", "--workspace", "--fail-on-no-libraries"]
DENY_CMD = ["cargo", "deny", "--exclude-dev", "check", "licenses"]
MACHETE_CMD = ["cargo", "machete"]


def _iter_manifest_files() -> list[Path]:
    """Return Cargo.toml / Cargo.lock paths without descending into build/vendor trees."""
    files: list[Path] = []
    for directory, child_dirs, filenames in os.walk(ROOT, topdown=True):
        child_dirs[:] = sorted(name for name in child_dirs if name not in SKIP_DIRS)
        base = Path(directory)
        for filename in sorted(filenames):
            if filename in {"Cargo.toml", "Cargo.lock"}:
                files.append(base / filename)
    return files


def check_banned_patterns() -> int:
    """Fail if Cargo.toml / Cargo.lock declares sha2/sha256 identity crates."""
    failures: list[str] = []
    for path in _iter_manifest_files():
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as err:
            failures.append(f"{path}: read error: {err}")
            continue
        rel = path.relative_to(ROOT).as_posix()
        for name in BANNED_DEP_NAMES:
            # Match dependency key lines; avoid false hits on comments.
            if re.search(rf'(?m)^\s*{re.escape(name)}\s*=', text) or (
                path.name == "Cargo.lock" and f'name = "{name}"' in text
            ):
                failures.append(f"{rel}: banned dependency `{name}` (use blake3 / native resolver)")

    if failures:
        print("pre-commit: banned dependency check failed:", file=sys.stderr)
        for line in failures:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("pre-commit: banned dependency check passed")
    return 0


def check_dylint() -> int:
    """Run Entmoot policy Dylint lints from tools/entmoot-lints."""
    if shutil.which("cargo-dylint") is None:
        # `cargo dylint` is provided by the cargo-dylint binary.
        print(
            "pre-commit: cargo-dylint not on PATH; install with "
            "`cargo install cargo-dylint dylint-link --locked`",
            file=sys.stderr,
        )
        return 1
    env = dict(os.environ)
    # Dylint owns target/dylint; do not inherit commit-check CARGO_TARGET_DIR.
    env.pop("CARGO_TARGET_DIR", None)
    if os.name == "nt":
        return _run_dylint_windows(env)
    print("+", " ".join(DYLINT_CMD), flush=True)
    result = subprocess.run(DYLINT_CMD, check=False, cwd=ROOT, env=env)
    if result.returncode != 0:
        print("pre-commit: cargo dylint failed", file=sys.stderr)
        return result.returncode
    print("pre-commit: cargo dylint passed")
    return 0


def check_cargo_deny() -> int:
    """Fail if non-dev dependency licenses violate deny.toml (copyleft not allowlisted)."""
    if shutil.which("cargo-deny") is None:
        print(
            "pre-commit: cargo-deny not on PATH; install with "
            "`cargo install cargo-deny --locked`",
            file=sys.stderr,
        )
        return 1
    print("+", " ".join(DENY_CMD), flush=True)
    result = subprocess.run(DENY_CMD, check=False, cwd=ROOT)
    if result.returncode != 0:
        print("pre-commit: cargo deny failed", file=sys.stderr)
        return result.returncode
    print("pre-commit: cargo deny passed")
    return 0


def check_cargo_machete() -> int:
    """Fail if workspace crates declare unused dependencies."""
    if shutil.which("cargo-machete") is None:
        print(
            "pre-commit: cargo-machete not on PATH; install with "
            "`cargo install cargo-machete --locked`",
            file=sys.stderr,
        )
        return 1
    print("+", " ".join(MACHETE_CMD), flush=True)
    result = subprocess.run(MACHETE_CMD, check=False, cwd=ROOT)
    if result.returncode != 0:
        print("pre-commit: cargo machete failed", file=sys.stderr)
        return result.returncode
    print("pre-commit: cargo machete passed")
    return 0


def _run_dylint_windows(env: dict[str, str]) -> int:
    """Map the repo to a short drive so grammar OUT_DIR stays under MAX_PATH."""
    for letter in "ZYXWVUTSRQPONMLKJIHGFED":
        drive = f"{letter}:"
        mapped = subprocess.run(
            ["subst", drive, str(ROOT)], check=False, capture_output=True, text=True
        )
        if mapped.returncode != 0:
            continue
        try:
            print("+", " ".join(DYLINT_CMD), f"(cwd={drive}\\)", flush=True)
            result = subprocess.run(DYLINT_CMD, check=False, cwd=f"{drive}\\", env=env)
            if result.returncode != 0:
                print("pre-commit: cargo dylint failed", file=sys.stderr)
                return result.returncode
            print("pre-commit: cargo dylint passed")
            return 0
        finally:
            subprocess.run(["subst", drive, "/d"], check=False, capture_output=True)
    print("pre-commit: no free drive letter for dylint subst", file=sys.stderr)
    return 1


def check_local_markdown_links(root: Path = ROOT) -> int:
    """Fail when a relative Markdown link points to a missing local target."""
    failures: list[str] = []
    markdown_files: list[Path] = []
    for directory, child_dirs, filenames in os.walk(root, topdown=True):
        child_dirs[:] = sorted(name for name in child_dirs if name not in SKIP_DIRS)
        base = Path(directory)
        markdown_files.extend(
            base / filename for filename in sorted(filenames) if filename.endswith(".md")
        )

    for path in markdown_files:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as err:
            failures.append(f"{path.relative_to(root).as_posix()}: read error: {err}")
            continue
        for match in MARKDOWN_LINK_RE.finditer(text):
            target = match.group("target").strip()
            if target.startswith("<") and target.endswith(">"):
                target = target[1:-1]
            target = unquote(target.split("#", 1)[0])
            if not target or re.match(r"^[a-z][a-z0-9+.-]*:", target, re.IGNORECASE):
                continue
            resolved = (path.parent / target).resolve()
            if not resolved.exists():
                line = text.count("\n", 0, match.start()) + 1
                failures.append(
                    f"{path.relative_to(root).as_posix()}:{line}: missing `{target}`"
                )

    if failures:
        print("pre-commit: local Markdown link check failed:", file=sys.stderr)
        for line in failures:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("pre-commit: local Markdown link check passed")
    return 0


def check_crate_boundaries(root: Path = ROOT) -> int:
    """Enforce the accepted inward dependency graph and canonical crate set."""
    failures: list[str] = []
    workspace_path = root / "Cargo.toml"
    try:
        workspace = tomllib.loads(workspace_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as err:
        print(f"pre-commit: crate boundary check failed: {err}", file=sys.stderr)
        return 1

    members = {
        tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8")).get("package", {}).get("name", Path(member).name) for member in workspace.get("workspace", {}).get("members", [])
    }
    if members != CANONICAL_CRATES:
        failures.append(
            f"workspace crates must be {sorted(CANONICAL_CRATES)}, found {sorted(members)}"
        )
    for retired in sorted(RETIRED_CRATES):
        if (root / "crates" / retired).exists():
            failures.append(f"retired crate directory remains: crates/{retired}")

    forbidden = {
        "entmoot-contracts": {
            "entmoot-core", "entmoot-engine", "entmoot", "entmoot-mcp",
            "gix", "rusqlite", "rmcp", "tokio", "clap",
        },
        "entmoot-core": {
            "entmoot-engine", "entmoot", "entmoot-mcp",
            "gix", "rusqlite", "rmcp", "tokio", "clap",
        },
        "entmoot-engine": {"entmoot", "entmoot-mcp", "rmcp", "clap"},
    }
    for crate, denied in forbidden.items():
        manifest_path = root / "crates" / crate / "Cargo.toml"
        try:
            manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError) as err:
            failures.append(f"{manifest_path.relative_to(root)}: {err}")
            continue
        dependencies = set(manifest.get("dependencies", {}))
        dependencies.update(manifest.get("dev-dependencies", {}))
        violations = sorted(dependencies & denied)
        tree_sitter = sorted(name for name in dependencies if name.startswith("tree-sitter"))
        if crate == "entmoot-contracts":
            violations.extend(tree_sitter)
        if violations:
            failures.append(f"{crate} has outward/engine dependency: {violations}")

    if failures:
        print("pre-commit: crate boundary check failed:", file=sys.stderr)
        for line in failures:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("pre-commit: crate boundary check passed")
    return 0


def check_test_ownership_registry() -> int:
    """Validate entmoot.test_ownership.v1 covers declared core tests and owner paths."""
    from scripts.test_ownership import load_registry, validate_registry

    errors = validate_registry(load_registry(), ROOT)
    if errors:
        print("pre-commit: test ownership registry check failed:", file=sys.stderr)
        for line in errors:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("pre-commit: test ownership registry check passed")
    return 0


def validation_environment(environment: dict[str, str]) -> dict[str, str]:
    """Isolate test repositories from Git's hook context and the live MCP binary."""
    # Repository-local variables from `git rev-parse --local-env-vars`. Keep the
    # original environment for staged-file selection, but never redirect Git in
    # a child fixture/worktree to the committing repository's index or objects.
    local = {"GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG", "GIT_CONFIG_PARAMETERS",
             "GIT_CONFIG_COUNT", "GIT_OBJECT_DIRECTORY", "GIT_DIR", "GIT_WORK_TREE",
             "GIT_IMPLICIT_WORK_TREE", "GIT_GRAFT_FILE", "GIT_INDEX_FILE",
             "GIT_NO_REPLACE_OBJECTS", "GIT_REPLACE_REF_BASE", "GIT_PREFIX",
             "GIT_SHALLOW_FILE", "GIT_COMMON_DIR"}
    clean = {key: value for key, value in environment.items()
             if key.upper() not in local
             and not key.upper().startswith(("GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_"))}
    # Windows process lookup can choose the hook interpreter's application
    # directory before PATH. Pin the existing campaign default from PATH once;
    # an explicit operator selection remains exclusive. Never install packages.
    selected_python = shutil.which("python", path=clean.get("PATH"))
    if selected_python:
        clean.setdefault("ENTMOOT_CAMPAIGN_PYTHON", selected_python)
    # Native-feature Windows test binaries have large linker working sets.
    # CPU-count fanout can exhaust the paging file before tests even start.
    # Respect explicit host capacity, otherwise keep the validation default bounded.
    if sys.platform == "win32":
        clean.setdefault("CARGO_BUILD_JOBS", str(min(6, os.cpu_count() or 1)))
    # Optional compile cache when the operator has sccache on PATH.
    if "RUSTC_WRAPPER" not in clean and shutil.which("sccache", path=clean.get("PATH")):
        clean["RUSTC_WRAPPER"] = "sccache"
    # Pin discovered floor adapters when present (never download). Explicit
    # ENTMOOT_*_ADAPTER overrides remain exclusive and are left untouched.
    adapters_root = ROOT / ".tmp" / "adapter-build" / "resolver-adapters"
    if adapters_root.is_dir():
        clean.setdefault("ENTMOOT_ADAPTERS_ROOT", str(adapters_root))
    clang_python = ROOT / ".tmp" / "adapter-build" / "clang-env" / "Scripts" / "python.exe"
    if not clang_python.is_file():
        clang_python = ROOT / ".tmp" / "adapter-build" / "clang-env" / "bin" / "python"
    if clang_python.is_file():
        clean.setdefault("ENTMOOT_PYTHON_EXECUTABLE", str(clang_python))
    for relative in (
        "clang-env/Lib/site-packages/clang/native/libclang.dll",
        "clang-env/lib/python3.13/site-packages/clang/native/libclang.so",
        "clang-env/lib/python3.12/site-packages/clang/native/libclang.so",
    ):
        libclang = ROOT / ".tmp" / "adapter-build" / relative
        if libclang.is_file():
            clean.setdefault("LIBCLANG_PATH", str(libclang))
            break
    if "GOTOOLCHAIN" not in clean:
        clean["GOTOOLCHAIN"] = "go1.26.0"
    return {**clean, "CARGO_TARGET_DIR": str(ROOT / "target/commit-check")}


def native_lane_enabled(environment: dict[str, str], producer: Path, paths: list[str] | None = None) -> bool:
    """A fresh checkout stays structural-only; preserve an operator's installed lane."""
    mode = environment.get("ENTMOOT_NATIVE_CHECK", "auto")
    if mode not in {"auto", "on", "off"}:
        raise ValueError("ENTMOOT_NATIVE_CHECK must be auto, on, or off")
    if mode != "auto":
        return mode == "on"
    if not producer.is_file():
        return False
    owners = json.loads((ROOT / "scripts/test_ownership.json").read_text(encoding="utf-8")).get("resolver_owners")
    return paths is None or not owners or any(path.startswith(owner) for path in paths for owner in owners)


def run(cmd: list[str], label: str) -> int:
    print("+", " ".join(cmd), flush=True)
    result = subprocess.run(cmd, check=False, cwd=ROOT,
                            env=validation_environment(dict(os.environ)))
    if result.returncode != 0:
        print(f"pre-commit: {label} failed", file=sys.stderr)
        return result.returncode
    print(f"pre-commit: {label} passed")
    return 0


def sidecar_controls(environment: dict[str, str], paths: list[str] | None) -> list[tuple[str, list[str]]]:
    from scripts.adapter_catalog import load_catalog, resolved_adapter, control_runtime_available
    from scripts.test_ownership import CargoTarget, cargo_commands_for_targets
    if environment.get("ENTMOOT_NATIVE_CHECK") == "off":
        return []
    shared = ("crates/entmoot-engine/src/native_resolver/producers/sidecar.rs",
              "crates/entmoot-engine/src/native_resolver/environment.rs",
              "crates/entmoot-engine/src/native_resolver/join.rs",
              "crates/entmoot-engine/src/process.rs", "tools/resolver-adapters/catalog.toml")
    selected = []
    for family in load_catalog():
        control = family.get("control")
        if not control:
            continue
        relevant = paths is None or environment.get("ENTMOOT_NATIVE_CHECK") == "on" or any(
            path.startswith(owner) for path in paths for owner in (*shared, *control["owners"]))
        if not relevant:
            continue
        if resolved_adapter(family, environment) is None or not control_runtime_available(family, environment):
            print(f"pre-commit: {family['id']} control unavailable (adapter/runtime absent or exclusive override invalid)")
            continue
        command = cargo_commands_for_targets([CargoTarget("entmoot", control["target"])])[0]
        selected.append((family["id"], command + [control["filter"], "--", "--include-ignored", "--exact"]))
    return selected


AccelerationMode = Literal["skip", "selected", "workspace"]


@dataclass
class AccelerationPlan:
    mode: AccelerationMode
    note: str
    cargo_commands: list[list[str]] = field(default_factory=list)
    clippy_packages: list[str] = field(default_factory=list)
    changed_paths: list[str] | None = None


def _acceleration_plan() -> AccelerationPlan:
    """Changed-byte ownership is the floor; no classifier binary can waive it."""
    from scripts.pr_structural_diff import cargo_commands, map_path_to_selection, path_preflight_index
    try:
        report = path_preflight_index(staged_only=True)
        changed = [f for f in report.files if f.classification != "skip"]
        if not changed:
            return AccelerationPlan("skip", "documentation/hook-only change; Python checks still run")
        for item in changed:
            for path in dict.fromkeys([item.path, item.previous_path or item.path]):
                map_path_to_selection(path, report, force=True)
        if any(note.startswith("unmapped path:") for note in report.notes):
            return AccelerationPlan("workspace", "unmapped change; full workspace checks")
        commands = cargo_commands(report)
        if not commands:
            return AccelerationPlan("workspace", "unmapped change; full workspace checks")
        packages = sorted({t.package for t in report.selected_targets} | set(report.selected_packages))
        if report.selected_entmoot_core_tests:
            packages = sorted(set(packages) | {"entmoot-core"})
        return AccelerationPlan("selected", "changed-file ownership; structural equality cannot skip tests", commands, packages, [p for f in changed for p in (f.path, f.previous_path or f.path)])
    except Exception as error:
        print(f"pre-commit: ownership unavailable ({error}); full workspace checks", file=sys.stderr)
        return AccelerationPlan("workspace", "ownership unavailable; full workspace checks")


def tests_build_cli_binaries(commands: list[list[str]]) -> bool:
    """Cargo builds executable targets for this crate's integration tests."""
    for command in commands:
        packages = {command[i + 1] for i, arg in enumerate(command[:-1]) if arg == "-p"}
        excluded = {command[i + 1] for i, arg in enumerate(command[:-1]) if arg == "--exclude"}
        if "entmoot" in excluded or not ("--workspace" in command or "entmoot" in packages):
            continue
        if "--test" in command or not any(flag in command for flag in ("--lib", "--doc", "--bin", "--example", "--bench")):
            return True
    return False


def main() -> int:
    policy = (
        check_banned_patterns,
        check_dylint,
        check_cargo_deny,
        check_cargo_machete,
    )
    if "--banned-only" in sys.argv[1:]:
        for check in policy:
            if check() != 0:
                return 1
        return 0
    for check in (
        *policy,
        check_crate_boundaries,
        check_local_markdown_links,
        check_test_ownership_registry,
    ):
        if check() != 0:
            return 1
    plan = _acceleration_plan()
    print(f"pre-commit: {plan.note}")
    suffix = ".exe" if os.name == "nt" else ""
    try:
        native = native_lane_enabled(dict(os.environ), ROOT / "target/commit-check/debug" / f"entmoot-producer{suffix}", plan.changed_paths)
    except ValueError as error:
        print(f"pre-commit: {error}", file=sys.stderr)
        return 1
    features = "entmoot-core/full-tests"
    if native:
        features += ",entmoot-engine/resolved-rust-analyzer,entmoot-engine/resolved-oxc,entmoot-engine/native-tests,entmoot/producer,entmoot/native-tests"
    else:
        print("pre-commit: native lane unavailable, disabled, or unrelated to changed owners")

    def with_features(cmd: list[str]) -> list[str]:
        cmd = list(cmd)
        from scripts.test_ownership import WORKSPACE_PACKAGES, workspace_manifests, workspace_dependencies
        selected = set(WORKSPACE_PACKAGES) if "--workspace" in cmd else {
            cmd[i + 1] for i, value in enumerate(cmd[:-1]) if value == "-p"}
        manifests = workspace_manifests(ROOT)
        dependency_index = workspace_dependencies(ROOT, manifests)
        dependencies = set(selected)
        while True:
            added = {dep for package in dependencies for dep in dependency_index.get(package, set())}
            if added <= dependencies:
                break
            dependencies.update(added)
        requested = features.split(",")
        if "--features" in cmd:
            requested.extend(cmd[cmd.index("--features") + 1].split(","))
        enabled = [f for f in dict.fromkeys(requested) if f.split("/")[0] in dependencies
                   and (f != "entmoot/producer" or "entmoot" in selected)]
        if "--features" in cmd:
            position = cmd.index("--features")
            del cmd[position:position + 2]
        if enabled:
            # Cargo features must precede the test/Clippy argument separator.
            position = cmd.index("--") if "--" in cmd else len(cmd)
            cmd[position:position] = ["--features", ",".join(enabled)]
        return cmd

    for label, cmd in (
        ("pre-commit Python tests", [sys.executable, "-m", "unittest",
            "scripts.test_pr_structural_diff", "scripts.test_test_ownership",
            "scripts.test_propose_test_ownership", "scripts.test_pre_commit_check",
            "scripts.test_normalize_sarif_repo_paths", "scripts.test_docs_workflow"]),
        ("cargo fmt", ["cargo", "fmt", "--", "--check"]),
    ):
        code = run(cmd, label)
        if code != 0:
            return code
    if plan.mode == "skip":
        print("pre-commit: no Cargo owners changed")
        return 0

    clippy = ["cargo", "clippy", "--all-targets", "--features", features]
    if plan.mode == "workspace":
        clippy.append("--workspace")
        tests = [["cargo", "test", "--workspace", "--features", features]]
    else:
        for package in plan.clippy_packages:
            clippy.extend(["-p", package])
        tests = [with_features(cmd) for cmd in plan.cargo_commands]
    clippy.extend(["--", "-D", "warnings"])
    checks = [("cargo clippy", with_features(clippy))] + [(f"cargo test {i+1}/{len(tests)}", cmd) for i, cmd in enumerate(tests)]
    failures = [label for label, cmd in checks if run(cmd, label) != 0]
    if failures:
        print("pre-commit: failed steps: " + ", ".join(failures), file=sys.stderr)
        return 1
    if native:
        # Package-wide engine tests already ran native controls with these features.
        engine_tested = plan.mode == "workspace" or any(
            "entmoot-engine" in cmd and not any(flag in cmd for flag in ("--test", "--lib", "--doc"))
            for cmd in tests)
        commands = [] if engine_tested else [
            ("native resolver controls", ["cargo", "test", "-p", "entmoot-engine", "--test", "callsites_rust", "--test", "callsites_javascript"])]
        if not tests_build_cli_binaries(tests):
            commands.append(("matched native CLI and producer", ["cargo", "build", "-p", "entmoot", "--profile", "test", "--bins"]))
        for label, cmd in commands:
            code = run(with_features(cmd), label)
            if code != 0:
                return code
    for family, cmd in sidecar_controls(dict(os.environ), plan.changed_paths):
        code = run(with_features(cmd), f"installed {family} control")
        if code != 0:
            return code
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
