"""Qualified inputs and synchronous SQLite storage for repository campaigns.

Rust owns the manifest and its canonical input digest. Git and material bytes
are rechecked here; local paths are executor configuration, not wire identities.
Existing unrelated databases are never adopted or migrated.
"""
from __future__ import annotations

from contextlib import contextmanager
import argparse
from importlib.metadata import distribution
import json
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
from typing import Iterator

import blake3

from evals.harness.contracts import ContractError
from evals.harness.wire_schema import CATALOG, QUERY_CATALOG, validate_wire

OID = re.compile(r'(?:[0-9a-f]{40}|[0-9a-f]{64})\Z')
DIGEST = re.compile(r'blake3:[0-9a-f]{64}\Z')
SCHEMA = '''
CREATE TABLE campaign_context (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    manifest_digest TEXT NOT NULL,
    layout_digest TEXT NOT NULL
);
CREATE TABLE receipts (
    receipt_key TEXT PRIMARY KEY,
    body BLOB NOT NULL
);
'''


def digest(raw: bytes) -> str:
    return 'blake3:' + blake3.blake3(raw).hexdigest()


def evaluator_digest() -> str:
    """Identify evaluator sources, schemas and installed reader package versions.

    Interpreter/SQLite versions qualify this reader; native SDK/build closures
    remain separate from these evaluation inputs. Do not cache across mutations.
    """
    hasher = blake3.blake3(b'entmoot.revision_campaign_evaluator.v1\0')

    def field(value: bytes) -> None:
        hasher.update(len(value).to_bytes(8, 'little'))
        hasher.update(value)

    field(sys.version.encode())
    field(sqlite3.sqlite_version.encode())
    for name in ('revision_campaign.py', 'run_revision_campaign.py', 'revision_accounting.py', 'report_revision_pair.py',
                 'impact.py', 'wire_schema.py', 'contracts.py', 'npm_lock_inventory.py', 'measurement.py'):
        path = Path(__file__).with_name(name)
        for value in (name.encode(), path.read_bytes().replace(b'\r\n', b'\n')):
            field(value)
    for path in (CATALOG, QUERY_CATALOG):
        field(path.name.encode())
        field(path.read_bytes().replace(b'\r\n', b'\n'))
    for name in ('blake3', 'jsonschema', 'attrs', 'referencing', 'rpds-py', 'jsonschema-specifications', 'psutil'):
        package = distribution(name)
        field(name.encode())
        field(package.version.encode())
    return 'blake3:' + hasher.hexdigest()


def _run(argv: list[str], *, cwd: Path | None = None) -> bytes:
    result = subprocess.run(argv, cwd=cwd, capture_output=True, timeout=60, check=False)
    if result.returncode:
        raise ContractError(f'input verification command failed ({result.returncode}): '
                            + (result.stderr or result.stdout).decode('utf-8', errors='replace').strip())
    return result.stdout


def git_revision(repository: Path, revision: str) -> dict:
    """Read immutable commit headers without modifying the checkout or history."""
    if not OID.fullmatch(revision):
        raise ContractError('a full immutable Git object ID is required')
    if _run(['git', 'cat-file', '-t', revision], cwd=repository).strip() != b'commit':
        raise ContractError('revision object is not a Git commit')
    headers = _run(['git', 'cat-file', '-p', revision], cwd=repository).split(b'\n\n', 1)[0].splitlines()
    trees = [line.removeprefix(b'tree ').decode('ascii') for line in headers if line.startswith(b'tree ')]
    parents = [line.removeprefix(b'parent ').decode('ascii') for line in headers if line.startswith(b'parent ')]
    if len(trees) != 1 or not all(OID.fullmatch(oid) for oid in [*trees, *parents]):
        raise ContractError('invalid Git commit metadata')
    if _run(['git', 'cat-file', '-t', trees[0]], cwd=repository).strip() != b'tree':
        raise ContractError('commit tree object is unavailable or has the wrong kind')
    return {'revision': revision, 'tree_oid': trees[0], 'parents': parents}


def select_comparisons(repository: Path, *, heads: list[str] | None = None,
                       windows: list[tuple[str, str, str]] | None = None) -> tuple[list[dict], list[dict]]:
    """Direct parents are loaded even when absent from the selected head list.

    Windows are explicitly supplied base/head/reason triples. A root requested
    as a parent edge fails rather than being silently omitted from denominators.
    """
    revisions: dict[str, dict] = {}

    def load(revision: str) -> dict:
        if revision not in revisions:
            revisions[revision] = git_revision(repository, revision)
        return revisions[revision]

    comparisons = []
    seen = set()
    for head in heads or []:
        metadata = load(head)
        if not metadata['parents']:
            raise ContractError(f'root commit has no direct parent: {head}')
        base = metadata['parents'][0]
        load(base)
        comparisons.append({'base_revision': base, 'head_revision': head,
                            'relation': {'kind': 'direct_parent', 'parent_index': 0}})
    for base, head, reason in windows or []:
        if not reason.strip():
            raise ContractError('selected windows require an explicit reason')
        load(base)
        load(head)
        comparisons.append({'base_revision': base, 'head_revision': head,
                            'relation': {'kind': 'selected_window', 'reason': reason}})
    for pair in comparisons:
        key = pair['base_revision'], pair['head_revision']
        if key in seen:
            raise ContractError('duplicate comparison')
        seen.add(key)
    if not comparisons:
        raise ContractError('at least one comparison is required')
    return [revisions[key] for key in sorted(revisions)], comparisons


def material_digest(path: Path) -> str:
    """Stream exact file bytes; a bundle must be supplied as an explicit file."""
    path = path.resolve(strict=True)
    if not path.is_file():
        raise ContractError(f'material is not a file: {path}')
    hasher = blake3.blake3()
    with path.open('rb') as stream:
        while block := stream.read(1024 * 1024):
            hasher.update(block)
    return 'blake3:' + hasher.hexdigest()


def verify_inputs(manifest_path: Path, *, cli: Path, repository: Path, repository_id: str,
                  materials: dict[tuple[str, str], Path]) -> tuple[dict, str, str]:
    """No stored receipt may qualify its own source/tool/profile identity."""
    raw = manifest_path.read_bytes()
    inputs = json.loads(raw)
    validate_wire(inputs, 'RevisionCampaignInputs')
    if inputs['repository_id'] != repository_id:
        raise ContractError('campaign repository namespace mismatch')
    expected_materials = {(row['role'], row['id']): row for row in inputs['materials']}
    if len(expected_materials) != len(inputs['materials']) or set(materials) != set(expected_materials):
        raise ContractError('material bindings do not exactly match the manifest')
    if materials.get(('tool', 'entmoot'), Path()).resolve() != cli.resolve(strict=True):
        raise ContractError('manifest tool binding is not the selected CLI')
    for key, row in expected_materials.items():
        if material_digest(materials[key]) != row['content_digest']:
            raise ContractError(f'changed campaign material: {key}')
    for revision in inputs['revisions']:
        if git_revision(repository, revision['revision']) != revision:
            raise ContractError('source revision metadata differs from the manifest')
    validated = json.loads(_run([str(cli), 'validate-campaign-inputs', '--manifest', str(manifest_path),
                                '--evaluator-digest', evaluator_digest()]))
    if not isinstance(validated, str) or not DIGEST.fullmatch(validated):
        raise ContractError('invalid canonical campaign input digest from Rust')
    if manifest_path.read_bytes() != raw:
        raise ContractError('manifest changed during validation')
    return inputs, validated, digest(raw)


class CampaignStore:
    """Latest operation results within a qualified run; batches are atomic.

    Retries can replace failed results. Context and layout fences prevent mixing
    different runs or adopting unrelated databases. Raw output retention is a
    caller decision, not a storage requirement.
    """
    def __init__(self, path: Path, *, manifest_digest: str, create: bool = False):
        self.path = path.resolve()
        self.context = (manifest_digest, digest(SCHEMA.encode()))
        if not all(DIGEST.fullmatch(value) for value in self.context):
            raise ContractError('invalid campaign context digest')
        self.connection: sqlite3.Connection | None = None
        if create:
            # Exclusive creation refuses existing empty files as well as old DBs.
            with self.path.open('xb'):
                pass
            connection = sqlite3.connect(self.path.as_uri() + '?mode=rw', uri=True)
            try:
                connection.executescript('BEGIN IMMEDIATE;\n' + SCHEMA)
                connection.execute('INSERT INTO campaign_context VALUES (1,?,?)', self.context)
                connection.commit()
            finally:
                connection.close()
        connection = sqlite3.connect(self.path.as_uri() + '?mode=ro', uri=True)
        try:
            self._check_context(connection)
        finally:
            connection.close()
        self.connection = sqlite3.connect(self.path.as_uri() + '?mode=rw', uri=True)
        self.connection.execute('PRAGMA busy_timeout=30000')

    def _check_context(self, connection: sqlite3.Connection) -> None:
        try:
            rows = connection.execute('SELECT manifest_digest, layout_digest FROM campaign_context').fetchall()
        except sqlite3.Error as error:
            raise ContractError('missing campaign input/layout context; create a fresh run') from error
        if rows != [self.context]:
            raise ContractError('campaign input/layout fingerprint mismatch')

    @contextmanager
    def transaction(self) -> Iterator[sqlite3.Connection]:
        connection = self.connection
        if connection is None:
            raise ContractError('campaign store is closed')
        connection.execute('BEGIN IMMEDIATE')
        try:
            self._check_context(connection)
            yield connection
            connection.commit()
        except BaseException:
            connection.rollback()
            raise

    def put_receipts(self, rows: list[tuple[str, bytes]]) -> None:
        """Commit a bounded caller-supplied group atomically; no result on failure."""
        if any(not key.strip() or not isinstance(body, bytes) for key, body in rows):
            raise ContractError('receipts require a key and a byte payload')
        with self.transaction() as connection:
            for key, body in rows:
                connection.execute('INSERT INTO receipts VALUES (?,?) ON CONFLICT(receipt_key) DO UPDATE SET body=excluded.body', (key, body))

    def get_receipt(self, key: str) -> bytes | None:
        if self.connection is None:
            raise ContractError('campaign store is closed')
        self._check_context(self.connection)
        row = self.connection.execute('SELECT body FROM receipts WHERE receipt_key=?', (key,)).fetchone()
        if row is None:
            return None
        return row[0]

    def close(self) -> None:
        if self.connection is not None:
            self.connection.close()
            self.connection = None

    def __enter__(self) -> CampaignStore:
        return self

    def __exit__(self, *_args) -> None:
        self.close()


def canonical_json(value) -> bytes:
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode('utf-8')


def main() -> None:
    """Small operator interface, also used by Cargo process integration tests."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['create', 'put', 'get', 'comparisons'])
    parser.add_argument('--db', type=Path)
    parser.add_argument('--context', type=Path)
    parser.add_argument('--rows', type=Path)
    parser.add_argument('--key')
    parser.add_argument('--repository', type=Path)
    parser.add_argument('--head', action='append', default=[])
    parser.add_argument('--window', nargs=3, action='append', default=[])
    args = parser.parse_args()
    if args.operation == 'comparisons':
        revisions, comparisons = select_comparisons(args.repository, heads=args.head, windows=args.window)
        print(json.dumps({'revisions': revisions, 'comparisons': comparisons}))
        return
    if args.db is None or args.context is None:
        parser.error('--db and --context are required for storage operations')
    context = digest(canonical_json(json.loads(args.context.read_bytes())))
    with CampaignStore(args.db, manifest_digest=context, create=args.operation == 'create') as store:
        result = None
        if args.operation == 'put':
            if args.rows is None:
                parser.error('put requires --rows containing an object of operation results')
            rows = json.loads(args.rows.read_bytes())
            store.put_receipts([(key, canonical_json(value)) for key, value in rows.items()])
        elif args.operation == 'get':
            if args.key is None:
                parser.error('get requires --key')
            raw = store.get_receipt(args.key)
            result = json.loads(raw) if raw is not None else None
        print(json.dumps(result))


if __name__ == '__main__':
    main()
