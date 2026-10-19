"""Check persisted memberships/digests and Git-backed source availability.

Requires the matched reader fingerprint. Reads one consistent SQLite transaction;
never edits the database, source checkouts, original reports or measurements.
"""
import argparse
import json
from pathlib import Path
import time

from evals.harness.contracts import ContractError
from evals.harness.history_pilot import Commands, blob_digests, grade_inventory, write
from evals.harness.knowledge_sql import connect_store
from evals.harness.revision_campaign import digest, material_digest


def audit(pilot, cli, destination):
    destination.mkdir(parents=True, exist_ok=False)
    began = time.monotonic()
    views = json.loads((pilot / 'views.json').read_bytes())
    inputs = json.loads((pilot / 'inputs.json').read_bytes())
    report = json.loads((pilot / 'report.json').read_bytes())
    if not report['implementation_unchanged'] or material_digest(cli) != inputs['tools']['cli']:
        raise ContractError('requires a completed pilot and its matched CLI')
    fingerprint = json.loads(Commands(destination, 30).run([str(cli), 'fingerprint'], 'reader-fingerprint'))
    trees = json.loads((pilot / 'oracle-trees.json').read_bytes())
    extensions = {repo['id']: repo['extensions'] for repo in inputs['selection']['configuration']['repositories']}
    receipts = {}
    for path in (pilot / 'commands').glob('*-oracle-bytes-*/process.json'):
        process = json.loads(path.read_bytes())
        if process['status'] == 'complete':
            key = path.parent.name.split('-oracle-bytes-')[0]
            receipts.setdefault(key, []).append(process)
    first = next(iter(views.values()))
    connection = connect_store(Path(first['retained']['locator']), expected_fingerprint=fingerprint)
    grades = {}
    try:
        for key, view in views.items():
            locator = json.loads(Path(view['retained']['locator']).read_bytes())
            if locator != view['retained']['publication']:
                raise ContractError('locator differs from retention receipt: ' + key)
            process = sorted(receipts[key], key=lambda row: row['started_utc'])[-1]
            oracle_output = Path(process['stdout'])
            if material_digest(oracle_output) != process['output']['stdout']['content_digest']:
                raise ContractError('captured Git oracle changed: ' + key)
            contents = blob_digests(oracle_output.read_bytes())
            expected = {path: contents[oid] for path, oid in trees[key].items()}
            rows = connection.execute('''SELECT p.path, s.digest, s.bytes FROM inventory_files i
                JOIN shared_paths p ON p.path_id=i.path_id
                JOIN shared_source_files s ON s.file_id=i.file_id WHERE i.view_id=?''', (locator['view'],))
            actual, wrong_digest, git_backed = {}, [], []
            for path, recorded, content in rows:
                if Path(path).suffix not in extensions[key.rsplit('-', 1)[0]]:
                    continue
                actual[path] = recorded
                if content is None:
                    git_backed.append(path)
                elif digest(content) != recorded:
                    wrong_digest.append(path)
            header = connection.execute('SELECT entmoot_payload(payload), source_checked FROM inventory_headers WHERE view_id=?',
                                        (locator['view'],)).fetchone()
            snapshot = json.loads(header[0])['snapshot'] if header else {}
            raw_tree = Commands(destination, 30).run(
                ['git', '-C', view['source_root'], 'ls-tree', '-r', '-z', key.rsplit('-', 1)[1]], key + '-source-tree')
            source_tree = {}
            for entry in raw_tree.split(b'\0'):
                if entry:
                    metadata, name = entry.split(b'\t', 1)
                    mode, kind, oid = metadata.split()
                    path = name.decode('utf-8')
                    if mode in (b'100644', b'100755') and Path(path).suffix in extensions[key.rsplit('-', 1)[0]]:
                        source_tree[path] = oid.decode('ascii')
            blob_input = destination / (key + '.blobs')
            blob_input.write_text('\n'.join(sorted(set(trees[key].values()))) + '\n', encoding='ascii')
            source_output = Commands(destination, 30).run(
                ['git', '-C', view['source_root'], 'cat-file', '--batch'], key + '-source-availability', stdin_path=blob_input)
            available = blob_digests(source_output)
            unavailable = sorted(path for path in git_backed if available.get(trees[key].get(path)) != expected.get(path))
            grade = grade_inventory(expected, actual)
            grade.update(retained_byte_digest_mismatches=sorted(wrong_digest), unavailable_git_paths=unavailable,
                         git_backed_memberships=len(git_backed),
                         checkout_tree_matches_oracle=source_tree == trees[key],
                         revision_matches=snapshot.get('revision') == key.rsplit('-', 1)[1],
                         source_checked=bool(header and header[1]))
            grade['passed'] = (grade['passed'] and not wrong_digest and not unavailable
                               and grade['revision_matches'] and grade['source_checked'] and grade['checkout_tree_matches_oracle'])
            grades[key] = grade
        database_views = connection.execute('SELECT count(*) FROM inventory_headers').fetchone()[0]
        source_identities, retained_blobs, source_bytes = connection.execute(
            'SELECT count(*),count(bytes),COALESCE(sum(length(bytes)),0) FROM shared_source_files').fetchone()
    finally:
        connection.close()
    result = {'scope': 'Persisted selected-extension membership/digests, snapshot revision and Git-backed byte availability',
        'pilot_report_digest': material_digest(pilot / 'report.json'), 'script_digest': material_digest(Path(__file__)),
        'reader_digest': material_digest(cli), 'reader_fingerprint': fingerprint,
        'knowledge_sql_digest': material_digest(Path('evals/harness/knowledge_sql.py')),
        'views': grades, 'passed_views': sum(row['passed'] for row in grades.values()),
        'checked_views': len(grades), 'database_views': database_views,
        'source_file_memberships': sum(row['expected_files'] for row in grades.values()),
        'git_backed_memberships': sum(row['git_backed_memberships'] for row in grades.values()),
        'unique_source_identities': source_identities, 'retained_source_blobs': retained_blobs, 'stored_source_bytes': source_bytes,
        'wall_seconds': time.monotonic() - began,
        'limitations': ['Private storage layout qualified against the matched Rust reader fingerprint.',
            'Does not grade callable identities, native dispatch, bindings or behavior.',
            'Immutable source bytes may remain in owned Git acquisitions instead of being duplicated in SQLite.',
            'Byte expectations come from digest-checked captured Git cat-file output, not product exports.']}
    write(destination / 'report.json', result)
    print(json.dumps({key: result[key] for key in ('checked_views', 'passed_views', 'database_views', 'source_file_memberships')}))
    if result['passed_views'] != len(trees) or database_views != len(trees):
        raise ContractError('persisted source audit incomplete or failed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pilot', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--out-dir', type=Path, required=True)
    arguments = parser.parse_args()
    audit(arguments.pilot.resolve(), arguments.cli.resolve(), arguments.out_dir.resolve())
