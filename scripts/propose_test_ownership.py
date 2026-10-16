#!/usr/bin/env python3
"""Propose File/RepoPath owners for the test-ownership registry (review, don't auto-merge).

Near-term: Cargo.toml [[test]] + naming conventions → draft owners; diff vs
scripts/test_ownership.json. Optional: pytest --collect-only for eval trees.

The registry remains authoritative for W6 / pre-commit (INV-GATE-02). This script
only prints proposals or writes a draft JSON for humans to accept.
"""

from __future__ import annotations

import argparse
import ast
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

_REPO_ROOT = Path(__file__).resolve().parents[1]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

from scripts.test_ownership import (  # noqa: E402
    OWNERSHIP_SCHEMA,
    WORKSPACE_PACKAGES,
    CargoTarget,
    declared_core_tests,
    load_registry,
    workspace_manifests,
    integration_targets,
)

CORE_TESTS_DIR = "crates/entmoot-core/tests"
CORE_LANG_DIR = "crates/entmoot-core/src/languages"
CORE_SRC = "crates/entmoot-core/src"

# Convention: [[test]] name → default owners / extensions (Gazelle-style stubs).
LANG_PLUGIN_TESTS: dict[str, tuple[str, list[str]]] = {
    "test_python": ("python/", [".py"]),
    "test_javascript": ("javascript/", [".js", ".jsx", ".mjs", ".cjs"]),
    "test_typescript": ("typescript/", [".ts", ".tsx"]),
    "test_java": ("java/", [".java"]),
    "test_go": ("go/", [".go"]),
    "test_csharp": ("csharp/", [".cs"]),
    "test_rust": ("rust/", [".rs"]),
}

# Single-file or directory source owners keyed by test name.
NAMED_SRC_OWNERS: dict[str, list[dict[str, str]]] = {
    "c_family_inventory": [
        {"kind": "language_plugin", "path": f"{CORE_LANG_DIR}/c_family.rs"},
    ],
    "python_inventory": [{"kind": "language_plugin", "path": f"{CORE_LANG_DIR}/python/"}],
    "test_granular": [{"kind": "file_prefix", "path": f"{CORE_SRC}/granular.rs"}],
    "test_analysis": [{"kind": "file_prefix", "path": f"{CORE_SRC}/analysis/"}],
    "test_config_comparison": [
        {"kind": "file_prefix", "path": "crates/entmoot-contracts/src/normalization.rs"}
    ],
    "test_logic_address": [
        {"kind": "file_prefix", "path": f"{CORE_SRC}/logic_address.rs"}
    ],
    "test_index": [{"kind": "file_prefix", "path": f"{CORE_SRC}/index.rs"}],
    "test_callable_parity": [
        {"kind": "file_prefix", "path": f"{CORE_SRC}/callable_context.rs"}
    ],
    "file_revision_pairs": [
        {"kind": "file_prefix", "path": f"{CORE_SRC}/ground_truth/"}
    ],
}


@dataclass(frozen=True)
class ProposedTarget:
    cargo: dict[str, str]
    owners: list[dict[str, str]]
    extensions: list[str]
    notes: str
    source: str  # convention | pytest_collect


def propose_core_test(name: str) -> ProposedTarget:
    test_rs = f"{CORE_TESTS_DIR}/{name}.rs"
    owners: list[dict[str, str]] = [{"kind": "file_prefix", "path": test_rs}]
    extensions: list[str] = []
    notes = "convention from Cargo.toml [[test]] name"

    if name in LANG_PLUGIN_TESTS:
        plugin, extensions = LANG_PLUGIN_TESTS[name]
        owners.insert(
            0,
            {"kind": "language_plugin", "path": f"{CORE_LANG_DIR}/{plugin}"},
        )
    elif name in NAMED_SRC_OWNERS:
        owners.extend(NAMED_SRC_OWNERS[name])
    else:
        notes = (
            f"convention: no named source owner for {name!r}; "
            "only the test file is proposed — review before accepting"
        )

    return ProposedTarget(
        cargo={"package": "entmoot-core", "test": name},
        owners=owners,
        extensions=extensions,
        notes=notes,
        source="convention",
    )


def propose_package(package: str) -> ProposedTarget:
    return ProposedTarget(
        cargo={"package": package},
        owners=[{"kind": "crate", "path": f"crates/{package}/"}],
        extensions=[],
        notes="convention: workspace package → crate path",
        source="convention",
    )


def cargo_proposals() -> list[ProposedTarget]:
    proposed = [propose_core_test(name) for name in declared_core_tests()]
    for package in WORKSPACE_PACKAGES:
        if package == "entmoot-core":
            continue
        proposed.append(propose_package(package))
    return proposed


def registry_cargo_keys(registry_path: Path | None = None) -> set[str]:
    registry = load_registry(registry_path)
    keys: set[str] = set()
    for entry in registry.targets:
        if entry.workspace_expand or entry.cargo.test == "_workspace_root":
            continue
        keys.add(entry.cargo.key())
    return keys


def proposal_key(proposal: ProposedTarget) -> str:
    cargo = proposal.cargo
    return CargoTarget(cargo["package"], cargo.get("test")).key()


def diff_proposals(
    proposals: list[ProposedTarget], registry_path: Path | None = None
) -> dict[str, Any]:
    existing = registry_cargo_keys(registry_path)
    proposed_keys = {proposal_key(p) for p in proposals}
    missing = sorted(proposed_keys - existing)
    declared = {CargoTarget(package, name).key()
                for package, manifest in workspace_manifests(_REPO_ROOT).items()
                for name in integration_targets(_REPO_ROOT, package, manifest)}
    extra = sorted(existing - proposed_keys - declared)
    by_key = {proposal_key(p): p for p in proposals}
    return {
        "schema_version": OWNERSHIP_SCHEMA,
        "missing_from_registry": [
            {
                "cargo": by_key[k].cargo,
                "owners": by_key[k].owners,
                "extensions": by_key[k].extensions,
                "notes": by_key[k].notes,
                "source": by_key[k].source,
            }
            for k in missing
            if k in by_key
        ],
        "extra_in_registry": extra,
        "proposed_count": len(proposals),
        "registry_count": len(existing),
    }


def _local_import_owners(test_path: Path, tree_root: Path) -> list[dict[str, str]]:
    """Static import edges under tree_root → RepoPath file_prefix owners (Pants-like)."""
    owners: list[dict[str, str]] = [
        {
            "kind": "file_prefix",
            "path": test_path.relative_to(_REPO_ROOT).as_posix(),
        }
    ]
    try:
        source = test_path.read_text(encoding="utf-8")
        tree = ast.parse(source, filename=str(test_path))
    except (OSError, SyntaxError):
        return owners

    modules: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                modules.add(alias.name.split(".", 1)[0])
        elif isinstance(node, ast.ImportFrom) and node.module:
            modules.add(node.module.split(".", 1)[0])

    root = tree_root.resolve()
    for mod in sorted(modules):
        for candidate in (
            root / mod / "__init__.py",
            root / f"{mod}.py",
            root / "src" / mod / "__init__.py",
            root / "src" / f"{mod}.py",
        ):
            if candidate.is_file():
                rel = candidate.relative_to(_REPO_ROOT).as_posix()
                # Prefer package directory as owner when __init__.py
                if candidate.name == "__init__.py":
                    rel = candidate.parent.relative_to(_REPO_ROOT).as_posix() + "/"
                owners.append({"kind": "file_prefix", "path": rel})
                break
    return owners


def pytest_collect_proposals(
    collect_root: Path, *, pytest_bin: str = "pytest"
) -> list[ProposedTarget]:
    """Run pytest --collect-only; propose owners from test file + local imports."""
    completed = subprocess.run(
        [pytest_bin, "--collect-only", "-q", str(collect_root)],
        cwd=_REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode not in (0, 5):  # 5 = no tests collected
        raise RuntimeError(
            f"pytest --collect-only failed (exit {completed.returncode}):\n"
            f"{completed.stderr or completed.stdout}"
        )

    # Lines look like: path/to/test_foo.py::TestClass::test_bar
    node_re = re.compile(r"^(\S+?\.py)::")
    test_files: set[Path] = set()
    for line in completed.stdout.splitlines():
        match = node_re.match(line.strip())
        if not match:
            continue
        raw = match.group(1).replace("\\", "/")
        path = Path(raw)
        if not path.is_absolute():
            path = (_REPO_ROOT / path).resolve()
        if path.is_file():
            test_files.add(path)

    proposals: list[ProposedTarget] = []
    for test_path in sorted(test_files):
        rel = test_path.relative_to(_REPO_ROOT).as_posix()
        stem = test_path.stem
        proposals.append(
            ProposedTarget(
                cargo={"package": "pytest", "test": rel},
                owners=_local_import_owners(test_path, collect_root),
                extensions=[],
                notes=(
                    f"pytest collect proposal for {stem}; "
                    "schema uses package=pytest as a draft runner kind — "
                    "review before merging into test_ownership.json"
                ),
                source="pytest_collect",
            )
        )
    return proposals


AUTHORITATIVE_REGISTRY = _REPO_ROOT / "scripts" / "test_ownership.json"


def write_draft(path: Path, proposals: list[ProposedTarget]) -> None:
    resolved = path.resolve()
    if resolved == AUTHORITATIVE_REGISTRY.resolve():
        raise ValueError(
            "refusing to overwrite authoritative scripts/test_ownership.json; "
            "pass a different --write-draft path for human review"
        )
    payload = {
        "schema_version": OWNERSHIP_SCHEMA,
        "notes": (
            "DRAFT only — propose-and-review. Do not replace "
            "scripts/test_ownership.json without human acceptance."
        ),
        "targets": [
            {
                "cargo": p.cargo,
                "owners": p.owners,
                **({"extensions": p.extensions} if p.extensions else {}),
                "notes": p.notes,
                "proposal_source": p.source,
            }
            for p in proposals
        ],
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help="Write the full diff/proposal report as JSON",
    )
    parser.add_argument(
        "--write-draft",
        type=Path,
        default=None,
        help="Write a draft targets JSON (convention proposals, or pytest if requested)",
    )
    parser.add_argument(
        "--from-pytest-collect",
        type=Path,
        default=None,
        metavar="DIR",
        help="Also propose owners from pytest --collect-only under DIR (eval trees)",
    )
    parser.add_argument(
        "--pytest-bin",
        default="pytest",
        help="Pytest executable for --from-pytest-collect (default: pytest)",
    )
    args = parser.parse_args()

    proposals = cargo_proposals()
    pytest_proposals: list[ProposedTarget] = []
    if args.from_pytest_collect is not None:
        collect_root = args.from_pytest_collect
        if not collect_root.is_dir():
            print(f"not a directory: {collect_root}", file=sys.stderr)
            return 2
        try:
            pytest_proposals = pytest_collect_proposals(
                collect_root, pytest_bin=args.pytest_bin
            )
        except RuntimeError as err:
            print(str(err), file=sys.stderr)
            return 2
        proposals = proposals + pytest_proposals

    report = diff_proposals([p for p in proposals if p.source == "convention"])
    report["pytest_proposals"] = [
        {
            "cargo": p.cargo,
            "owners": p.owners,
            "notes": p.notes,
            "source": p.source,
        }
        for p in pytest_proposals
    ]

    print(
        f"registry: {report['registry_count']} targets; "
        f"convention proposals: {report['proposed_count']}; "
        f"missing: {len(report['missing_from_registry'])}; "
        f"extra: {len(report['extra_in_registry'])}; "
        f"pytest: {len(pytest_proposals)}"
    )
    for entry in report["missing_from_registry"]:
        print(f"  missing {entry['cargo']}: owners={entry['owners']}")
    for key in report["extra_in_registry"]:
        print(f"  extra in registry (no convention proposal): {key}")
    for entry in report["pytest_proposals"]:
        print(f"  pytest {entry['cargo']}: owners={entry['owners']}")

    if args.json_out is not None:
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.write_draft is not None:
        draft_src = (
            pytest_proposals
            if args.from_pytest_collect is not None and pytest_proposals
            else [p for p in cargo_proposals() if proposal_key(p) in {
                CargoTarget(e["cargo"]["package"], e["cargo"].get("test")).key()
                for e in report["missing_from_registry"]
            }]
        )
        if not draft_src and args.from_pytest_collect is None:
            # Full convention draft when nothing missing — useful for review
            draft_src = cargo_proposals()
        write_draft(args.write_draft, draft_src)
        print(f"wrote draft: {args.write_draft}")

    # Non-zero when convention proposals diverge from registry (drift signal).
    if report["missing_from_registry"] or report["extra_in_registry"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
