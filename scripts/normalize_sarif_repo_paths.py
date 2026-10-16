"""Bounded SARIF repository-path projection (C4).

Policy: ``entmoot.sarif_repo_path_projection.v1`` (version ``1``).

This is an external producer step. The native SARIF remains the immutable source
artifact. The projection rebases only paths proven to reside beneath a declared
repository root into repository-relative POSIX paths suitable for attach. The
core SARIF adapter must not guess checkout paths. This projection does not make
malformed SARIF conformant.

Transformation receipt
----------------------
For each rewritten ``artifactLocation``:

1. Resolve ``uriBaseId`` against ``run.originalUriBaseIds`` when present
   (``urljoin`` of base URI + relative uri).
2. Classify the resulting URI; reject unsupported forms (see below).
3. Materialize a filesystem candidate and ``Path.resolve()`` it (follows
   symlinks/junctions).
4. Prove containment with ``resolved.relative_to(resolved_repo_root)``.
5. Write the POSIX relative path to ``uri``; drop a resolved ``uriBaseId``.
6. Preserve the pre-projection ``uri`` in ``properties.entmootOriginalUri``
   (first rewrite only). Preserve ``uriBaseId`` in
   ``properties.entmootOriginalUriBaseId`` when applicable.
7. Stamp ``entmootPathProjectionPolicy`` and ``entmootPathProjectionVersion``.

Document properties also receive the same policy/version plus
``entmootPathProjectionChanged`` (count of locations rewritten in the pass).

Supported input forms (after optional uriBaseId join)
-----------------------------------------------------
- Repo-relative POSIX paths (``src/a.py``), proven under the root.
- Leading checkout-directory segment matching ``repo_root.name`` (casefold),
  stripped once, then proven.
- POSIX absolute paths that resolve under the root.
- Windows absolute paths (``C:\\...`` / ``C:/...``) only when the host can prove
  containment (``os.name == "nt"``); otherwise rejected as unprovable.
- ``file:`` URIs with empty or ``localhost`` authority; percent-decoded; Windows
  ``file:///C:/...`` follows the Windows absolute rule above.

Fail-closed / unsupported URI forms (stable error prefixes)
-----------------------------------------------------------
- ``unsupported artifact URI scheme:`` — http, https, git, data, etc.
- ``unsupported file URI authority:`` — non-empty netloc other than localhost
- ``artifact URI is outside repository root:`` — other drive, unrelated absolute
  root, or symlink/junction that resolves outside
- ``artifact URI escapes repository root:`` — join that escapes after resolve
- ``unsafe relative artifact URI:`` — relative form containing ``..``
- ``artifact URI resolves to repository root:`` — empty relative result
- ``windows absolute path unprovable on this host:`` — drive-letter absolute or
  Windows file URI when not on Windows
- ``unsupported file URI path form:`` — ``file:`` URI whose path is neither
  absolute POSIX nor Windows-absolute after decode
- ``invalid SARIF URI base:`` — missing or non-string base mapping

Symlinks / junctions: containment is proven after resolve on both the declared
root and the candidate.

Case: output always uses POSIX ``/`` separators. Relative segment casing is
preserved from the successful relative path. Host filesystem case rules apply
only to the resolve/containment proof.

Idempotency: a second pass on an already-projected document leaves ``uri``
unchanged, does not overwrite ``entmootOriginalUri``, and reports zero changes
when every location is already at its proven relative form without ``uriBaseId``.
"""

from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path, PurePosixPath, PureWindowsPath
from urllib.parse import unquote, urljoin, urlparse

POLICY_ID = "entmoot.sarif_repo_path_projection.v1"
POLICY_VERSION = "1"

_WINDOWS_DRIVE_IN_POSIX = re.compile(r"^/[A-Za-z]:(/|$)")
_WINDOWS_DRIVE_HEAD = re.compile(r"^[A-Za-z]:(/|\\)")


def _is_windows_absolute(path_str: str) -> bool:
    return PureWindowsPath(path_str).is_absolute() or bool(
        _WINDOWS_DRIVE_HEAD.match(path_str.replace("\\", "/"))
    )


def _stamp_policy(properties: dict[str, object]) -> None:
    properties["entmootPathProjectionPolicy"] = POLICY_ID
    properties["entmootPathProjectionVersion"] = POLICY_VERSION


def _posix_under_root(candidate: Path, repo_root: Path, uri: str) -> str:
    resolved_root = repo_root.resolve()
    resolved = candidate.resolve()
    try:
        relative = resolved.relative_to(resolved_root)
    except ValueError as error:
        raise ValueError(f"artifact URI is outside repository root: {uri}") from error
    if not relative.parts:
        raise ValueError(f"artifact URI resolves to repository root: {uri}")
    return relative.as_posix()


def _normalize_absolute_path(path_str: str, repo_root: Path, uri: str) -> str:
    if _is_windows_absolute(path_str):
        if os.name != "nt":
            raise ValueError(f"windows absolute path unprovable on this host: {uri}")
        return _posix_under_root(Path(path_str), repo_root, uri)
    return _posix_under_root(Path(path_str), repo_root, uri)


def _normalize_file_uri(uri: str, repo_root: Path) -> str:
    parsed = urlparse(uri)
    authority = parsed.netloc.lower()
    if authority and authority not in {"localhost", "localhost."}:
        raise ValueError(f"unsupported file URI authority: {parsed.netloc}")
    raw_path = unquote(parsed.path)
    if not raw_path:
        raise ValueError(f"artifact URI resolves to repository root: {uri}")
    if _WINDOWS_DRIVE_IN_POSIX.match(raw_path):
        raw_path = raw_path[1:]
        return _normalize_absolute_path(raw_path, repo_root, uri)
    if _is_windows_absolute(raw_path):
        return _normalize_absolute_path(raw_path, repo_root, uri)
    if raw_path.startswith("/"):
        return _normalize_absolute_path(raw_path, repo_root, uri)
    raise ValueError(f"unsupported file URI path form: {uri}")


def _normalize_relative_or_bare(uri: str, repo_root: Path) -> str:
    if _is_windows_absolute(uri):
        return _normalize_absolute_path(uri, repo_root, uri)

    normalized_separators = uri.replace("\\", "/")
    if normalized_separators.startswith("/"):
        return _normalize_absolute_path(normalized_separators, repo_root, uri)

    posix = PurePosixPath(normalized_separators)
    if ".." in posix.parts:
        raise ValueError(f"unsafe relative artifact URI: {uri}")
    if posix.parts and posix.parts[0].casefold() == repo_root.name.casefold():
        posix = PurePosixPath(*posix.parts[1:])
    if not posix.parts:
        raise ValueError(f"artifact URI resolves to repository root: {uri}")
    candidate = repo_root.joinpath(*posix.parts)
    try:
        proven = _posix_under_root(candidate, repo_root, uri)
    except ValueError as error:
        message = str(error)
        if message.startswith("artifact URI is outside repository root:"):
            raise ValueError(f"artifact URI escapes repository root: {uri}") from error
        raise
    # Prefer the lexical relative segments when they already match the proven path
    # under ordinary (non-symlink) trees so casing from the URI is preserved.
    lexical = posix.as_posix()
    if proven.casefold() == lexical.casefold():
        return lexical
    return proven


def normalize_uri(uri: str, repo_root: Path) -> str:
    """Return a proven repository-relative POSIX path for ``uri``."""
    parsed = urlparse(uri)
    if parsed.scheme and parsed.scheme.lower() != "file":
        # Treat Windows drive letters as paths, not schemes (urlparse("C:/x")).
        if len(parsed.scheme) == 1 and parsed.scheme.isalpha():
            return _normalize_relative_or_bare(uri, repo_root)
        raise ValueError(f"unsupported artifact URI scheme: {parsed.scheme}")
    if parsed.scheme.lower() == "file":
        return _normalize_file_uri(uri, repo_root)
    return _normalize_relative_or_bare(uri, repo_root)


def _resolve_against_base(
    uri: str, base_id: object, uri_bases: dict[str, object]
) -> tuple[str, str | None]:
    if not isinstance(base_id, str):
        return uri, None
    if base_id not in uri_bases:
        # Common scanner placeholders meaning repository root when omitted.
        if base_id.casefold() in {"%srcroot%", "srcroot"}:
            return uri, base_id
        raise ValueError(f"invalid SARIF URI base: {base_id}")
    base_value = uri_bases[base_id]
    base_uri = base_value.get("uri") if isinstance(base_value, dict) else base_value
    if not isinstance(base_uri, str):
        raise ValueError(f"invalid SARIF URI base: {base_id}")
    return urljoin(base_uri.rstrip("/") + "/", uri), base_id


def normalize_document(document: object, repo_root: Path) -> int:
    """Project artifactLocation URIs; return the number of locations rewritten."""
    changed = 0
    resolved_root = repo_root.resolve()

    def visit(value: object, uri_bases: dict[str, object]) -> None:
        nonlocal changed
        if isinstance(value, dict):
            artifact = value.get("artifactLocation")
            if isinstance(artifact, dict) and isinstance(artifact.get("uri"), str):
                original = artifact["uri"]
                base_id = artifact.get("uriBaseId")
                resolved, preserved_base = _resolve_against_base(
                    original, base_id, uri_bases
                )
                normalized = normalize_uri(resolved, resolved_root)
                needs_rewrite = normalized != original or preserved_base is not None
                if needs_rewrite:
                    properties = artifact.setdefault("properties", {})
                    if not isinstance(properties, dict):
                        raise ValueError("artifactLocation.properties must be an object")
                    if "entmootOriginalUri" not in properties:
                        properties["entmootOriginalUri"] = original
                    if preserved_base is not None:
                        properties.setdefault(
                            "entmootOriginalUriBaseId", preserved_base
                        )
                        artifact.pop("uriBaseId", None)
                    _stamp_policy(properties)
                    artifact["uri"] = normalized
                    changed += 1
            for key, child in value.items():
                if key in {"artifactLocation", "originalUriBaseIds"}:
                    continue
                visit(child, uri_bases)
        elif isinstance(value, list):
            for child in value:
                visit(child, uri_bases)

    if not isinstance(document, dict):
        raise ValueError("SARIF document must be an object")
    runs = document.get("runs")
    if not isinstance(runs, list):
        raise ValueError("SARIF document must contain runs")
    for run in runs:
        if not isinstance(run, dict):
            raise ValueError("SARIF run must be an object")
        bases = run.get("originalUriBaseIds", {})
        if not isinstance(bases, dict):
            raise ValueError("originalUriBaseIds must be an object")
        visit(run, bases)

    properties = document.setdefault("properties", {})
    if not isinstance(properties, dict):
        raise ValueError("SARIF document properties must be an object")
    _stamp_policy(properties)
    properties["entmootPathProjectionChanged"] = changed
    return changed


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Project SARIF artifact URIs to repository-relative POSIX paths "
            f"under policy {POLICY_ID}."
        )
    )
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, required=True)
    args = parser.parse_args()
    repo_root = args.repo_root.resolve(strict=True)
    document = json.loads(args.input.read_text(encoding="utf-8-sig"))
    changed = normalize_document(document, repo_root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, separators=(",", ":")), encoding="utf-8")
    print(f"normalized_artifact_locations={changed}")


if __name__ == "__main__":
    main()
