#!/usr/bin/env python3
"""PR gate decision selector: scoped skip/run for acceleration workflows.

Policy Decision Table (typed evidence → GateAction). Not a merge oracle.
Full ci.yml remains the correctness backstop.

Writes markdown + JSON receipt (schema entmoot.pr_gate_decision.v1).
Path-preflight (--path-preflight) needs no CLI: docs/evals-only diffs skip
cargo build and tests. Structural classification requires --cli.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Literal

_REPO_ROOT = Path(__file__).resolve().parents[1]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

from scripts.test_ownership import (
    WORKSPACE_PACKAGES,
    CargoTarget,
    OwnershipRegistry,
    cargo_commands_for_targets,
    load_registry,
)


GATE_SCHEMA_VERSION = "entmoot.pr_gate_decision.v1"

GateAction = Literal[
    "skip_cli_build",
    "skip_cargo_tests",
    "run_cli_build",
    "run_cargo_tests",
    "fail_closed",
]


EXT_TO_LANG: dict[str, str] = {
    ".py": "python",
    ".js": "javascript",
    ".jsx": "javascript",
    ".mjs": "javascript",
    ".cjs": "javascript",
    ".ts": "typescript",
    ".tsx": "tsx",
    ".java": "java",
    ".go": "go",
    ".cs": "csharp",
    ".rs": "rust",
}

_REGISTRY: OwnershipRegistry | None = None


def get_registry() -> OwnershipRegistry:
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = load_registry()
    return _REGISTRY


@dataclass
class FileResult:
    path: str
    # noop | callable_changed | boundary_changed | module_changed | added | deleted | error | skip | path_changed
    classification: str
    previous_path: str | None = None
    language: str | None = None
    base_hash: str | None = None
    head_hash: str | None = None
    invalidated_callable_hashes: list[str] = field(default_factory=list)
    introduced_callable_hashes: list[str] = field(default_factory=list)
    retained_callable_count: int = 0
    boundary_changed_callable_count: int = 0
    module_shell_changed: bool = False
    detail: str | None = None


@dataclass
class DiffReport:
    base: str
    head: str
    config: str
    files: list[FileResult] = field(default_factory=list)
    selected_entmoot_core_tests: list[str] = field(default_factory=list)
    selected_packages: list[str] = field(default_factory=list)  # cargo -p names
    selected_targets: list[CargoTarget] = field(default_factory=list)
    selected_from_owners: list[dict[str, object]] = field(default_factory=list)
    obsolete_tests: list[dict[str, object]] = field(default_factory=list)
    ownerless_tests: list[dict[str, object]] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class ChangedPath:
    path: str
    base_path: str | None
    head_path: str | None
    status: str


@dataclass(frozen=True)
class GateDecision:
    action: GateAction
    scope: str
    reason: str


def run(cmd: list[str], *, input_bytes: bytes | None = None, check: bool = False) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(cmd, input=input_bytes, capture_output=True, check=check)


def git_show(sha: str, path: str) -> bytes | None:
    completed = run(["git", "show", f"{sha}:{path}"])
    if completed.returncode != 0:
        return None
    return completed.stdout


def git_show_head(path: str) -> bytes | None:
    return git_show("HEAD", path)


def git_show_index(path: str) -> bytes | None:
    completed = run(["git", "show", f":{path}"])
    if completed.returncode != 0:
        return None
    return completed.stdout


def worktree_blob(path: str, repo_root: Path) -> bytes | None:
    candidate = repo_root / path
    if not candidate.is_file():
        return None
    return candidate.read_bytes()


def blobs_for_local_change(
    change: ChangedPath, *, use_index: bool, repo_root: Path
) -> tuple[bytes | None, bytes | None]:
    """Return (base_blob, head_blob) for HEAD vs index or HEAD vs worktree."""
    base_blob = git_show_head(change.base_path) if change.base_path else None
    if change.head_path is None:
        return base_blob, None
    if use_index:
        return base_blob, git_show_index(change.head_path)
    return base_blob, worktree_blob(change.head_path, repo_root)


_UNSET = object()


def parse_name_status(payload: bytes) -> list[ChangedPath]:
    tokens = [
        token.decode("utf-8", errors="replace").replace("\\", "/")
        for token in payload.split(b"\0")
        if token
    ]
    changes: list[ChangedPath] = []
    index = 0
    while index < len(tokens):
        status = tokens[index]
        index += 1
        kind = status[:1]
        if kind in {"R", "C"}:
            if index + 1 >= len(tokens):
                raise ValueError(f"truncated git name-status record for {status}")
            base_path, head_path = tokens[index], tokens[index + 1]
            index += 2
            changes.append(
                ChangedPath(
                    path=head_path,
                    base_path=base_path,
                    head_path=head_path,
                    status=status,
                )
            )
            continue
        if index >= len(tokens):
            raise ValueError(f"truncated git name-status record for {status}")
        path = tokens[index]
        index += 1
        changes.append(
            ChangedPath(
                path=path,
                base_path=None if kind == "A" else path,
                head_path=None if kind == "D" else path,
                status=status,
            )
        )
    return changes


def list_changed_paths(base: str, head: str) -> list[ChangedPath]:
    completed = run(
        [
            "git",
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "--diff-filter=ACMRD",
            f"{base}...{head}",
        ],
        check=True,
    )
    return parse_name_status(completed.stdout)


def list_index_changed_paths(*, staged_only: bool = True) -> list[ChangedPath]:
    """Changed paths in the index (and optionally working tree) vs HEAD."""
    cmd = ["git", "diff", "--name-status", "-z", "--find-renames", "--diff-filter=ACMRD"]
    if staged_only:
        cmd.append("--cached")
    cmd.append("HEAD")
    completed = run(cmd, check=True)
    return parse_name_status(completed.stdout)


def ext_lang(path: str) -> str | None:
    lower = path.lower()
    for ext, lang in EXT_TO_LANG.items():
        if lower.endswith(ext):
            return lang
    return None


def hash_blob(
    cli: Path, path: str, lang: str, config: str, content: bytes
) -> tuple[dict[str, object] | None, str | None]:
    completed = run(
        [
            str(cli),
            "hash",
            "--lang",
            lang,
            "--config",
            config,
            "--callable-inventory",
            "--path",
            path,
        ],
        input_bytes=content,
    )
    if completed.returncode != 0:
        err = completed.stderr.decode("utf-8", errors="replace").strip()
        return None, err or f"exit {completed.returncode}"
    try:
        payload = json.loads(completed.stdout)
    except (json.JSONDecodeError, UnicodeDecodeError) as err:
        return None, f"invalid callable inventory: {err}"
    required = {
        "file_hash",
        "module_shell_hash",
        "occurrences",
        "callable_boundaries",
    }
    if not isinstance(payload, dict) or not required.issubset(payload):
        return None, "callable inventory omitted required fields"
    return payload, None


def callable_hashes(inventory: dict[str, object]) -> list[str]:
    occurrences = inventory.get("occurrences")
    if not isinstance(occurrences, list):
        raise ValueError("callable inventory occurrences must be a list")
    hashes: list[str] = []
    for occurrence in occurrences:
        if not isinstance(occurrence, dict) or not isinstance(occurrence.get("structural_hash"), str):
            raise ValueError("callable occurrence omitted structural_hash")
        hashes.append(occurrence["structural_hash"])
    return hashes


def callable_delta(
    base_inventory: dict[str, object], head_inventory: dict[str, object]
) -> tuple[list[str], list[str], int]:
    """Return invalidated, introduced, and retained callable counts as multisets."""
    base = Counter(callable_hashes(base_inventory))
    head = Counter(callable_hashes(head_inventory))
    invalidated = sorted((base - head).elements())
    introduced = sorted((head - base).elements())
    retained = sum((base & head).values())
    return invalidated, introduced, retained


def boundary_change_count(
    base_inventory: dict[str, object], head_inventory: dict[str, object]
) -> int:
    """Count retained body occurrences whose syntactic boundary changed."""
    base_occurrences = base_inventory.get("occurrences")
    head_occurrences = head_inventory.get("occurrences")
    base_boundaries = base_inventory.get("callable_boundaries")
    head_boundaries = head_inventory.get("callable_boundaries")
    if not isinstance(base_occurrences, list) or not isinstance(head_occurrences, list):
        raise ValueError("callable inventory occurrences must be a list")
    if not isinstance(base_boundaries, list) or not isinstance(head_boundaries, list):
        raise ValueError("callable inventory callable_boundaries must be a list")

    def pairs(
        occurrences: list[object], boundaries: list[object]
    ) -> Counter[tuple[str, str]]:
        signature_by_address: dict[str, str] = {}
        for boundary in boundaries:
            if not isinstance(boundary, dict):
                raise ValueError("callable boundary must be an object")
            address_id = boundary.get("address_id")
            signature_hash = boundary.get("signature_hash")
            if not isinstance(address_id, str) or not isinstance(signature_hash, str):
                raise ValueError("callable boundary omitted address_id or signature_hash")
            if address_id in signature_by_address:
                raise ValueError(f"duplicate callable boundary for address_id {address_id}")
            signature_by_address[address_id] = signature_hash

        result: Counter[tuple[str, str]] = Counter()
        occurrence_addresses: set[str] = set()
        for occurrence in occurrences:
            if not isinstance(occurrence, dict):
                raise ValueError("callable occurrence must be an object")
            address_id = occurrence.get("address_id")
            structural_hash = occurrence.get("structural_hash")
            if not isinstance(address_id, str) or not isinstance(structural_hash, str):
                raise ValueError("callable occurrence omitted address_id or structural_hash")
            if address_id in occurrence_addresses:
                raise ValueError(f"duplicate callable occurrence for address_id {address_id}")
            occurrence_addresses.add(address_id)
            signature_hash = signature_by_address.get(address_id)
            if signature_hash is None:
                raise ValueError(f"callable occurrence {address_id} omitted boundary record")
            result[(structural_hash, signature_hash)] += 1
        extra_boundaries = signature_by_address.keys() - occurrence_addresses
        if extra_boundaries:
            raise ValueError(
                "callable boundary has no occurrence: " + sorted(extra_boundaries)[0]
            )
        return result

    retained_bodies = Counter(callable_hashes(base_inventory)) & Counter(
        callable_hashes(head_inventory)
    )
    exact_boundary_matches = pairs(base_occurrences, base_boundaries) & pairs(
        head_occurrences, head_boundaries
    )
    return sum(retained_bodies.values()) - sum(exact_boundary_matches.values())


def is_doc_skip(path: str, registry: OwnershipRegistry | None = None) -> bool:
    return (registry or get_registry()).is_skip_path(path)


def language_plugin_lang(path: str) -> str | None:
    marker = "crates/entmoot-core/src/languages/"
    if not path.startswith(marker):
        return None
    rest = path[len(marker) :]
    if not rest or rest in {"mod.rs", "plugin.rs"}:
        return None
    # Single-file plugins (e.g. c_family.rs)
    if "/" not in rest:
        return Path(rest).stem if rest.endswith(".rs") else rest
    return rest.split("/", 1)[0]


def selection_boundary(path: str) -> str:
    """Return the coarse test-ownership boundary for rename safety."""
    plugin = language_plugin_lang(path)
    if plugin:
        return f"entmoot-core-language:{plugin}"
    if path.startswith("crates/"):
        parts = path.split("/", 2)
        return f"crate:{parts[1]}" if len(parts) > 1 else "crates"
    if is_doc_skip(path):
        return "skip"
    return f"language:{ext_lang(path)}" if ext_lang(path) else "unmapped"


def add_unique(seq: list[str], item: str) -> None:
    if item not in seq:
        seq.append(item)


def _record_target(
    report: DiffReport, path: str, target: CargoTarget, owner_kind: str
) -> None:
    if target not in report.selected_targets:
        report.selected_targets.append(target)
    if target.package == "entmoot-core" and target.test:
        add_unique(report.selected_entmoot_core_tests, target.test)
    elif target.test is None and target.kind == "package":
        add_unique(report.selected_packages, target.package)
    report.selected_from_owners.append(
        {
            "path": path,
            "owner_kind": owner_kind,
            "cargo": target.as_dict(),
        }
    )


def force_full_workspace_selection(report: DiffReport) -> None:
    """Fail-closed: widen to full CORE + workspace package tests (never silent green)."""
    for package in WORKSPACE_PACKAGES:
        _record_target(report, "*", CargoTarget(package=package), "fallback")
    report.notes.append(
        "fail_closed: insufficient or error evidence — forcing full workspace cargo selection"
    )


def map_path_to_selection(path: str, report: DiffReport, force: bool) -> None:
    """Map a non-noop path to cargo test selections via test_ownership.v1."""
    if not force:
        return

    registry = get_registry()
    if registry.is_skip_path(path):
        report.notes.append(f"skip retest (docs/evals/hooks): {path}")
        return

    targets = registry.targets_for_path(path)
    if not targets:
        report.notes.append(f"unmapped path: {path}")
        force_full_workspace_selection(report)
        return

    owner_kind = "registry"
    plugin = language_plugin_lang(path)
    if plugin:
        owner_kind = "language_plugin"
    elif path.startswith("crates/"):
        owner_kind = "crate"
    elif any(path.lower().endswith(ext) for t in registry.targets for ext in t.extensions):
        owner_kind = "extension"

    for target in targets:
        _record_target(report, path, target, owner_kind)


def path_exists_on_ref(sha: str, path: str) -> bool:
    """True if path is a blob or tree at sha (repo-relative POSIX)."""
    completed = run(["git", "cat-file", "-e", f"{sha}:{path.rstrip('/')}"])
    return completed.returncode == 0


def populate_obsolete_and_ownerless(
    report: DiffReport, *, head: str | None = None, repo_root: Path | None = None
) -> None:
    """Emit obsolete_tests (sole owners deleted) and ownerless_tests (registry drift)."""
    registry = get_registry()
    deleted_paths = [
        f.previous_path or f.path
        for f in report.files
        if f.classification == "deleted"
    ]
    for f in report.files:
        if f.previous_path and f.classification == "path_changed":
            deleted_paths.append(f.previous_path)

    head_exists: set[str] | None = None
    if head is not None and head not in {"INDEX", ""}:
        head_exists = set()
        for entry in registry.targets:
            for owner in entry.owners:
                if owner.kind == "shared_core":
                    continue
                owner_path = owner.path.rstrip("/")
                if path_exists_on_ref(head, owner_path):
                    head_exists.add(owner_path)
        if repo_root is not None:
            for entry in registry.targets:
                for owner in entry.owners:
                    if owner.kind == "shared_core":
                        continue
                    owner_path = owner.path.rstrip("/")
                    if (repo_root / owner_path).exists():
                        head_exists.add(owner_path)

    report.obsolete_tests = registry.obsolete_targets(deleted_paths, head_exists)
    if repo_root is not None:
        report.ownerless_tests = registry.ownerless_targets(repo_root)
    if report.obsolete_tests:
        report.notes.append(
            f"obsolete_tests: {len(report.obsolete_tests)} cargo target(s) lost all File/RepoPath owners"
        )


def classify_file(
    change: ChangedPath,
    base: str,
    head: str,
    cli: Path,
    config: str,
    *,
    base_blob_override: bytes | None | object = _UNSET,
    head_blob_override: bytes | None | object = _UNSET,
) -> FileResult:
    path = change.path
    base_lang = ext_lang(change.base_path) if change.base_path else None
    head_lang = ext_lang(change.head_path) if change.head_path else None
    lang = head_lang or base_lang
    if base_blob_override is _UNSET:
        base_blob = git_show(base, change.base_path) if change.base_path else None
    else:
        base_blob = base_blob_override  # type: ignore[assignment]
    if head_blob_override is _UNSET:
        head_blob = git_show(head, change.head_path) if change.head_path else None
    else:
        head_blob = head_blob_override  # type: ignore[assignment]
    previous_path = change.base_path if change.base_path != change.head_path else None

    if base_blob is not None and head_blob is not None and base_lang != head_lang:
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="path_changed",
            language=lang,
            detail=f"language boundary changed: {base_lang or 'unsupported'} -> {head_lang or 'unsupported'}",
        )
    if previous_path and selection_boundary(previous_path) != selection_boundary(path):
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="path_changed",
            language=lang,
            detail=(
                f"test ownership changed: {selection_boundary(previous_path)} -> "
                f"{selection_boundary(path)}"
            ),
        )

    if base_blob is None and head_blob is None:
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="error",
            detail="missing on both sides",
        )

    if lang is None:
        # Path-rule only (not hashed)
        if base_blob is None:
            return FileResult(
                path=path,
                previous_path=previous_path,
                classification="added",
                detail="non-source",
            )
        if head_blob is None:
            return FileResult(
                path=path,
                previous_path=previous_path,
                classification="deleted",
                detail="non-source",
            )
        if base_blob == head_blob:
            return FileResult(
                path=path,
                previous_path=previous_path,
                classification="noop",
                detail="byte-identical non-source",
            )
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="path_changed",
            detail="non-source content changed",
        )

    if base_blob is None:
        head_inventory, err = hash_blob(cli, path, lang, config, head_blob or b"")
        if err:
            return FileResult(
                path=path,
                previous_path=previous_path,
                classification="error",
                language=lang,
                detail=err,
            )
        try:
            introduced = sorted(callable_hashes(head_inventory or {}))
        except ValueError as err:
            return FileResult(path=path, classification="error", language=lang, detail=str(err))
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="added",
            language=lang,
            head_hash=str((head_inventory or {})["file_hash"]),
            introduced_callable_hashes=introduced,
            module_shell_changed=True,
            detail=f"{len(introduced)} callable occurrence(s) introduced; new module",
        )

    if head_blob is None:
        base_inventory, err = hash_blob(cli, path, lang, config, base_blob)
        if err:
            return FileResult(
                path=path,
                previous_path=previous_path,
                classification="error",
                language=lang,
                detail=err,
            )
        try:
            invalidated = sorted(callable_hashes(base_inventory or {}))
        except ValueError as err:
            return FileResult(path=path, classification="error", language=lang, detail=str(err))
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="deleted",
            language=lang,
            base_hash=str((base_inventory or {})["file_hash"]),
            invalidated_callable_hashes=invalidated,
            module_shell_changed=True,
            detail=f"{len(invalidated)} callable occurrence(s) invalidated; module removed",
        )

    base_inventory, base_err = hash_blob(cli, path, lang, config, base_blob)
    head_inventory, head_err = hash_blob(cli, path, lang, config, head_blob)
    if base_err or head_err:
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="error",
            language=lang,
            detail="; ".join(x for x in (base_err, head_err) if x),
        )
    assert base_inventory is not None and head_inventory is not None
    base_hash = str(base_inventory["file_hash"])
    head_hash = str(head_inventory["file_hash"])
    try:
        boundary_changed = boundary_change_count(base_inventory, head_inventory)
    except ValueError as err:
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="error",
            language=lang,
            base_hash=base_hash,
            head_hash=head_hash,
            detail=str(err),
        )
    if base_hash == head_hash and boundary_changed == 0:
        retained = len(callable_hashes(head_inventory))
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="noop" if base_blob == head_blob else "path_changed",
            language=lang,
            base_hash=base_hash,
            head_hash=head_hash,
            retained_callable_count=retained,
            detail=None if base_blob == head_blob else "structural identity retained; changed source bytes still require owner tests",
        )

    try:
        invalidated, introduced, retained = callable_delta(base_inventory, head_inventory)
    except ValueError as err:
        return FileResult(
            path=path,
            previous_path=previous_path,
            classification="error",
            language=lang,
            base_hash=base_hash,
            head_hash=head_hash,
            detail=str(err),
        )
    module_shell_changed = (
        base_inventory["module_shell_hash"] != head_inventory["module_shell_hash"]
    )
    classification = (
        "callable_changed"
        if invalidated or introduced
        else "boundary_changed"
        if boundary_changed
        else "module_changed"
    )
    detail = (
        f"{len(invalidated)} invalidated, {len(introduced)} introduced, "
        f"{retained} retained callable occurrence(s)"
    )
    if module_shell_changed:
        detail += "; module shell changed"
    if boundary_changed:
        detail += f"; {boundary_changed} callable boundary change(s)"
    return FileResult(
        path=path,
        previous_path=previous_path,
        classification=classification,
        language=lang,
        base_hash=base_hash,
        head_hash=head_hash,
        invalidated_callable_hashes=invalidated,
        introduced_callable_hashes=introduced,
        retained_callable_count=retained,
        boundary_changed_callable_count=boundary_changed,
        module_shell_changed=module_shell_changed,
        detail=detail,
    )


def needs_retest(classification: str) -> bool:
    return classification in {
        "callable_changed",
        "boundary_changed",
        "module_changed",
        "added",
        "deleted",
        "error",
        "path_changed",
    }


def classification_counts(report: DiffReport) -> dict[str, int]:
    counts: dict[str, int] = {}
    for file_result in report.files:
        counts[file_result.classification] = counts.get(file_result.classification, 0) + 1
    return counts


def apply_fail_closed_if_needed(report: DiffReport) -> bool:
    """Widen selection when any file classified as error. Returns True if fail-closed."""
    if any(file_result.classification == "error" for file_result in report.files):
        force_full_workspace_selection(report)
        return True
    return False


def build_gate_decisions(
    report: DiffReport, *, path_preflight_only: bool = False
) -> list[GateDecision]:
    """Policy Decision Table: evidence → scoped GateAction (not a merge oracle)."""
    counts = classification_counts(report)
    has_error = counts.get("error", 0) > 0
    retest_files = [f for f in report.files if needs_retest(f.classification)]
    only_skip_or_noop = (not report.files) or all(
        file_result.classification in {"skip", "noop"} for file_result in report.files
    )
    docs_only = bool(report.files) and all(
        file_result.classification == "skip" for file_result in report.files
    )
    has_commands = bool(report.selected_targets or report.selected_entmoot_core_tests or report.selected_packages)

    if has_error:
        return [
            GateDecision(
                action="fail_closed",
                scope="acceleration_workflow",
                reason="classifier or inventory error — forced full workspace cargo selection",
            ),
            GateDecision(
                action="run_cli_build",
                scope="entmoot",
                reason="fail_closed keeps workspace tests runnable",
            ),
            GateDecision(
                action="run_cargo_tests",
                scope="workspace",
                reason="fail_closed widened cargo_commands",
            ),
        ]

    if path_preflight_only:
        if docs_only or not report.files:
            return [
                GateDecision(
                    action="skip_cli_build",
                    scope="acceleration_workflow",
                    reason="path-preflight: docs/evals/hooks only (or empty diff)",
                ),
                GateDecision(
                    action="skip_cargo_tests",
                    scope="acceleration_workflow",
                    reason="path-preflight: no code paths require cargo tests",
                ),
            ]
        return [
            GateDecision(
                action="run_cli_build",
                scope="entmoot",
                reason="path-preflight: non-doc paths require structural classification",
            )
        ]

    if only_skip_or_noop and not has_commands:
        return [
            GateDecision(
                action="skip_cli_build",
                scope="acceleration_workflow",
                reason="structural noop and/or docs-only — no CLI needed for empty cargo selection",
            ),
            GateDecision(
                action="skip_cargo_tests",
                scope="acceleration_workflow",
                reason="no selected cargo tests",
            ),
        ]

    return [
        GateDecision(
            action="run_cli_build",
            scope="entmoot",
            reason="code or non-noop paths selected cargo tests",
        ),
        GateDecision(
            action="run_cargo_tests",
            scope="selected_packages",
            reason=(
                f"{len(retest_files)} path(s) need retest; "
                f"{len(report.selected_entmoot_core_tests)} core tests, "
                f"{len(report.selected_packages)} packages"
            ),
        ),
    ]


def should_skip_cli_build(decisions: list[GateDecision]) -> bool:
    return any(decision.action == "skip_cli_build" for decision in decisions)


def cargo_commands(report: DiffReport) -> list[list[str]]:
    targets: list[CargoTarget] = list(report.selected_targets)
    for name in report.selected_entmoot_core_tests:
        targets.append(CargoTarget("entmoot-core", name))
    for package in report.selected_packages:
        targets.append(CargoTarget(package))
    return cargo_commands_for_targets(targets)


def build_receipt(report: DiffReport, decisions: list[GateDecision]) -> dict[str, object]:
    counts = classification_counts(report)
    md = render_markdown(report, decisions)
    return {
        "schema_version": GATE_SCHEMA_VERSION,
        "base_sha": report.base,
        "head_sha": report.head,
        "config": report.config,
        "evidence": {
            "classification_counts": counts,
            "structural_noop": bool(report.files)
            and all(f.classification in {"skip", "noop"} for f in report.files),
            "docs_only": bool(report.files)
            and all(f.classification == "skip" for f in report.files),
            "fail_closed": any(d.action == "fail_closed" for d in decisions),
            "selected_from_owners": report.selected_from_owners,
            "obsolete_tests": report.obsolete_tests,
            "ownerless_tests": report.ownerless_tests,
            "files": [asdict(f) for f in report.files],
        },
        "decisions": [asdict(d) for d in decisions],
        "selected_entmoot_core_tests": report.selected_entmoot_core_tests,
        "selected_packages": report.selected_packages,
        "cargo_commands": [" ".join(c) for c in cargo_commands(report)],
        "skip_cli_build": should_skip_cli_build(decisions),
        "notes": report.notes,
        "summary": md,
        # Backward-compatible aliases for older workflow readers
        "base": report.base,
        "head": report.head,
        "files": [asdict(f) for f in report.files],
    }


def render_markdown(report: DiffReport, decisions: list[GateDecision]) -> str:
    counts = classification_counts(report)

    lines = [
        "## PR gate decision (acceleration)",
        "",
        f"- Schema: `{GATE_SCHEMA_VERSION}`",
        f"- Base: `{report.base}`",
        f"- Head: `{report.head}`",
        f"- Config: `{report.config}`",
        "",
        "### Decisions",
        "",
        "| Action | Scope | Reason |",
        "|--------|-------|--------|",
    ]
    for decision in decisions:
        reason = decision.reason.replace("|", "\\|")
        lines.append(f"| `{decision.action}` | `{decision.scope}` | {reason} |")

    lines.extend(
        [
            "",
            "### Counts",
            "",
            "| Class | Count |",
            "|-------|------:|",
        ]
    )
    for key in (
        "noop",
        "callable_changed",
        "boundary_changed",
        "module_changed",
        "added",
        "deleted",
        "error",
        "path_changed",
        "skip",
    ):
        if key in counts:
            lines.append(f"| {key} | {counts[key]} |")

    lines.extend(["", "### Selected retests", ""])
    if report.selected_entmoot_core_tests or report.selected_packages:
        if report.selected_entmoot_core_tests:
            lines.append(
                "- entmoot-core: "
                + ", ".join(f"`{t}`" for t in report.selected_entmoot_core_tests)
            )
        if report.selected_packages:
            lines.append(
                "- packages: " + ", ".join(f"`{p}`" for p in report.selected_packages)
            )
    else:
        lines.append("- _(none — structural noops / docs-only)_")

    lines.extend(
        ["", "### Files", "", "| Path | Class | Lang | Detail |", "|------|-------|------|--------|"]
    )
    for f in report.files:
        detail = (f.detail or "").replace("|", "\\|")
        if len(detail) > 80:
            detail = detail[:77] + "..."
        display_path = f"{f.previous_path} → {f.path}" if f.previous_path else f.path
        lines.append(
            f"| `{display_path}` | {f.classification} | {f.language or ''} | {detail} |"
        )

    if report.notes:
        lines.extend(["", "### Notes", ""])
        for note in report.notes:
            lines.append(f"- {note}")

    if report.obsolete_tests:
        lines.extend(["", "### Obsolete tests (sole File/RepoPath owners deleted)", ""])
        for entry in report.obsolete_tests:
            cargo = entry.get("cargo", {})
            lines.append(f"- `{cargo}` — {entry.get('reason', '')}")

    if report.ownerless_tests:
        lines.extend(["", "### Ownerless registry entries (drift)", ""])
        for entry in report.ownerless_tests:
            lines.append(f"- `{entry.get('cargo')}` missing {entry.get('missing_owners')}")

    lines.append("")
    lines.append(
        "_Scoped acceleration only. Skip does not authorize merge, SemVer safety, "
        "or skipping authoritative `ci.yml`. Ownership map: `scripts/test_ownership.json`. "
        "PublicationOutcome::Unchanged is an intra-job store no-op, not a PR base/head oracle._"
    )
    return "\n".join(lines) + "\n"


def path_preflight_from_changes(
    changes: list[ChangedPath], *, base: str, head: str, config: str
) -> DiffReport:
    """Classify given paths without CLI; docs/evals → skip."""
    report = DiffReport(base=base, head=head, config=config)
    if not changes:
        report.notes.append("No paths in base...head diff")
        return report
    registry = get_registry()
    for change in changes:
        path = change.path
        previous = change.base_path if change.base_path != change.head_path else None
        if registry.is_skip_path(path):
            report.files.append(
                FileResult(
                    path=path,
                    previous_path=previous,
                    classification="skip",
                    detail="docs/evals/hooks",
                )
            )
        else:
            report.files.append(
                FileResult(
                    path=path,
                    previous_path=previous,
                    classification="path_changed",
                    language=ext_lang(path),
                    detail="path-preflight: requires structural classification",
                )
            )
    return report


def path_preflight_report(base: str, head: str, config: str) -> DiffReport:
    """Classify paths without CLI; docs/evals → skip, anything else → path_changed stub."""
    return path_preflight_from_changes(
        list_changed_paths(base, head), base=base, head=head, config=config
    )


def path_preflight_index(*, staged_only: bool = True, config: str = "v1") -> DiffReport:
    """Path-preflight for local pre-commit (index vs HEAD)."""
    changes = list_index_changed_paths(staged_only=staged_only)
    if not changes and staged_only:
        # Fall back to working tree vs HEAD when nothing is staged yet.
        changes = list_index_changed_paths(staged_only=False)
    return path_preflight_from_changes(
        changes, base="HEAD", head="INDEX", config=config
    )


def classify_index(
    cli: Path,
    *,
    staged_only: bool = True,
    config: str = "v1",
    repo_root: Path | None = None,
) -> DiffReport:
    """Structural classification for local pre-commit (HEAD vs index or worktree)."""
    root = repo_root or _REPO_ROOT
    changes = list_index_changed_paths(staged_only=staged_only)
    use_index = staged_only
    if not changes and staged_only:
        changes = list_index_changed_paths(staged_only=False)
        use_index = False
    head_label = "INDEX" if use_index else "WORKTREE"
    report = DiffReport(base="HEAD", head=head_label, config=config)
    if not changes:
        report.notes.append("No paths in index/worktree diff")
        return report

    for change in changes:
        path = change.path
        previous_path = change.base_path if change.base_path != change.head_path else None
        if is_doc_skip(path):
            report.files.append(
                FileResult(
                    path=path,
                    previous_path=previous_path,
                    classification="skip",
                    detail="docs/evals/hooks",
                )
            )
            continue

        base_blob, head_blob = blobs_for_local_change(
            change, use_index=use_index, repo_root=root
        )
        result = classify_file(
            change,
            "HEAD",
            head_label,
            cli,
            config,
            base_blob_override=base_blob,
            head_blob_override=head_blob,
        )
        report.files.append(result)

        if result.classification in {"noop", "skip"}:
            continue
        map_path_to_selection(path, report, force=needs_retest(result.classification))
        if previous_path and needs_retest(result.classification):
            map_path_to_selection(previous_path, report, force=True)

    apply_fail_closed_if_needed(report)
    populate_obsolete_and_ownerless(report, head=head_label, repo_root=root)
    report.selected_entmoot_core_tests.sort()
    report.selected_packages.sort()
    return report


def classify_diff(base: str, head: str, cli: Path, config: str) -> DiffReport:
    report = DiffReport(base=base, head=head, config=config)
    changes = list_changed_paths(base, head)
    if not changes:
        report.notes.append("No paths in base...head diff")

    for change in changes:
        path = change.path
        if is_doc_skip(path):
            result = FileResult(
                path=path,
                previous_path=(
                    change.base_path if change.base_path != change.head_path else None
                ),
                classification="skip",
                detail="docs/evals/hooks",
            )
            report.files.append(result)
            continue

        result = classify_file(change, base, head, cli, config)
        report.files.append(result)

        if result.classification in {"noop", "skip"}:
            continue
        map_path_to_selection(path, report, force=needs_retest(result.classification))
        if result.previous_path and needs_retest(result.classification):
            map_path_to_selection(result.previous_path, report, force=True)

    apply_fail_closed_if_needed(report)
    populate_obsolete_and_ownerless(
        report,
        head=head,
        repo_root=Path(__file__).resolve().parents[1],
    )
    report.selected_entmoot_core_tests.sort()
    report.selected_packages.sort()
    return report


def write_outputs(
    report: DiffReport,
    decisions: list[GateDecision],
    json_out: Path,
    summary_out: Path | None,
    print_cargo: bool,
) -> None:
    receipt = build_receipt(report, decisions)
    json_out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    md = str(receipt["summary"])
    sys.stdout.write(md)
    if summary_out:
        summary_out.write_text(md, encoding="utf-8")
    if print_cargo:
        for cmd in cargo_commands(report):
            print(" ".join(cmd))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="Base commit SHA")
    parser.add_argument("--head", required=True, help="Head commit SHA")
    parser.add_argument(
        "--cli",
        type=Path,
        default=None,
        help="Path to entmoot binary (required unless --path-preflight)",
    )
    parser.add_argument("--config", default="v1", choices=("v1",))
    parser.add_argument(
        "--json-out",
        type=Path,
        default=Path("pr_gate_decision.json"),
    )
    parser.add_argument("--summary-out", type=Path, default=None)
    parser.add_argument(
        "--path-preflight",
        action="store_true",
        help="Classify paths only (no CLI). Docs-only → skip_cli_build + skip_cargo_tests.",
    )
    parser.add_argument(
        "--print-cargo",
        action="store_true",
        help="Print cargo test command lines (one per line) after JSON",
    )
    parser.add_argument(
        "--strict-owners",
        action="store_true",
        help="Non-zero exit when obsolete_tests is non-empty (gate unittest / CI optional).",
    )
    args = parser.parse_args()

    try:
        if args.path_preflight:
            report = path_preflight_report(args.base, args.head, args.config)
            decisions = build_gate_decisions(report, path_preflight_only=True)
            write_outputs(
                report, decisions, args.json_out, args.summary_out, args.print_cargo
            )
            if args.strict_owners and report.obsolete_tests:
                print("strict-owners: obsolete_tests present", file=sys.stderr)
                return 1
            return 0

        if args.cli is None or not args.cli.is_file():
            print(
                f"entmoot not found: {args.cli} (required without --path-preflight)",
                file=sys.stderr,
            )
            return 2

        report = classify_diff(args.base, args.head, args.cli, args.config)
        decisions = build_gate_decisions(report, path_preflight_only=False)
        write_outputs(
            report, decisions, args.json_out, args.summary_out, args.print_cargo
        )
        if args.strict_owners and report.obsolete_tests:
            print("strict-owners: obsolete_tests present", file=sys.stderr)
            return 1
        return 0
    except subprocess.CalledProcessError as exc:
        print(exc.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        return 2
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
