"""Read-only eval adapter for the fingerprinted private SQLite v1 layout.

Select one publication and hydrate only payloads selected by a SQL query. Old
physical layouts must be regenerated; integer row IDs never leave this adapter.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import sqlite3
import subprocess

import blake3

from scripts.cargo_paths import debug_directory
from evals.harness.wire_schema import validate_wire


def reader_fingerprint() -> dict:
    """Ask the current Rust reader; never trust a database to qualify itself."""
    cli = debug_directory() / ("entmoot.exe" if os.name == "nt" else "entmoot")
    result = subprocess.run([str(cli), "fingerprint"], capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError("current CLI cannot provide a storage fingerprint; rebuild the selected Cargo target")
    value = json.loads(result.stdout)
    validate_wire(value, "StorageFingerprint")
    return value


class _FloatLexeme:
    def __init__(self, text: str):
        self.text = text


def _canonical(value) -> str:
    # Preserve Rust's serialized float spelling (e.g. 1e-5 versus Python 1e-05).
    # This is a storage commitment, not a new normalization policy.
    if isinstance(value, _FloatLexeme):
        return value.text
    if isinstance(value, dict):
        return "{" + ",".join(_canonical(k) + ":" + _canonical(v) for k, v in sorted(value.items())) + "}"
    if isinstance(value, list):
        return "[" + ",".join(map(_canonical, value)) + "]"
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False)


def connect_store(path: Path, *, expected_fingerprint: dict | None = None) -> sqlite3.Connection:
    # Explicit expectations are for a trusted reader receipt or isolated unit
    # fixtures. Callers must never copy this value from the store being opened.
    expected = reader_fingerprint() if expected_fingerprint is None else expected_fingerprint
    validate_wire(expected, "StorageFingerprint")
    path = path.resolve()
    view = "default"
    if path.name.endswith(".view.json"):
        locator = json.loads(path.read_text(encoding="utf-8"))
        if set(locator) != {"database", "view"}:
            raise ValueError("invalid knowledge view locator")
        view = locator["view"]
        if not isinstance(view, str) or len(view) != 71 or not view.startswith("blake3:"):
            raise ValueError("invalid view digest")
        bytes.fromhex(view[7:])
        database = Path(locator["database"])
        path = database if database.is_absolute() else path.parent / database
    # Rust canonical paths on Windows use the extended DOS prefix. as_uri()
    # treats that prefix as a URI authority, which SQLite correctly refuses.
    if os.name == "nt" and str(path).startswith("\\\\?\\") and not str(path).startswith("\\\\?\\UNC\\"):
        path = Path(str(path)[4:])
    connection = sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5)
    try:
        connection.execute("BEGIN")
        if connection.execute("PRAGMA user_version").fetchone()[0] != 1:
            raise ValueError("unsupported knowledge schema; regenerate with the current CLI")
        exists = connection.execute("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='storage_fingerprint')").fetchone()[0]
        row = connection.execute("SELECT body FROM storage_fingerprint WHERE singleton=1").fetchone() if exists else None
        if row is None:
            raise ValueError("unsupported knowledge fingerprint: missing envelope; regenerate with the current CLI")
        actual = json.loads(row[0])
        validate_wire(actual, "StorageFingerprint")
        if actual != expected:
            raise ValueError("unsupported knowledge fingerprint: reader/store mismatch; regenerate with the current CLI")
        connection.execute("CREATE TEMP TABLE selected_knowledge_view(id TEXT NOT NULL)")
        connection.execute("INSERT INTO selected_knowledge_view VALUES(?)", (view,))

        def hydrate(identifier: int) -> bytes:
            row = connection.execute("SELECT digest,body FROM shared_payloads WHERE payload_id=?", (identifier,)).fetchone()
            if row is None:
                raise ValueError("missing shared payload")
            digest, encoded = row
            payload = json.loads(encoded, parse_float=_FloatLexeme)
            if set(payload) != {"body", "metadata"}:
                raise ValueError("invalid shared payload")
            value = payload["body"]
            for key, metadata in payload["metadata"].items():
                entry = connection.execute("SELECT body FROM shared_metadata WHERE metadata_id=?", (metadata,)).fetchone()
                if entry is None or not isinstance(value, dict) or key in value:
                    raise ValueError("missing or conflicting metadata")
                value[key] = json.loads(entry[0], parse_float=_FloatLexeme)
            canonical = _canonical(value).encode("utf-8")
            if blake3.blake3(canonical).digest() != digest:
                raise ValueError("payload content digest mismatch")
            return canonical

        connection.create_function("entmoot_payload", 1, hydrate)
        columns = "collection,record_id,artifact_id,subject_id,object_id,proposition_id,predicate,location_id,entmoot_payload(payload) AS payload,visible,ownership_managed"
        scope = "view_id=(SELECT id FROM temp.selected_knowledge_view)"
        connection.execute(f"CREATE TEMP VIEW knowledge_records AS SELECT {columns} FROM main.scoped_records WHERE {scope}")
        connection.execute(f"CREATE TEMP VIEW knowledge_records_by_subject AS SELECT {columns} FROM main.scoped_records INDEXED BY knowledge_by_subject_predicate WHERE {scope}")
        connection.execute(f"CREATE TEMP VIEW knowledge_metadata AS SELECT key,value FROM main.scoped_metadata WHERE {scope}")
        return connection
    except BaseException:
        connection.close()
        raise
