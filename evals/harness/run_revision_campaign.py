"""Execute immutable revision campaigns against explicitly supplied Git worktrees.

Local configuration supplies paths and the complete child-process environment.
It is pinned as a profile material. Every CLI command explicitly selects the
run-owned data volume. This runner never clones, checks out, resets, writes to,
or removes source trees. Native enrichment is automatic and safe-mode; unavailable
producers remain explicit. External build closure and compiler-mode qualification
are not established by a successful execution or by material hashes.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from evals.harness.contracts import ContractError
from evals.harness.measurement import command as measured_command
from evals.harness.revision_campaign import (CampaignStore, _run, digest, evaluator_digest,
                                            git_revision, material_digest, select_comparisons, verify_inputs)
from evals.harness.revision_accounting import occurrence_accounting, summarize_occurrences, summarize_relay
from evals.harness.wire_schema import validate_wire


def encoded(value) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, indent=2) + '\n').encode('utf-8')


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def inside(path: Path, root: Path) -> bool:
    # samefile compares actual filesystem identity, including Windows verbatim
    # path spelling; resolve removes link indirection before the ancestor check.
    return any(parent.samefile(root) for parent in path.resolve(strict=True).parents)


def configuration(path: Path) -> tuple[dict, dict[tuple[str, str], Path]]:
    config = json.loads(path.read_bytes())
    required = {'repository', 'repository_id', 'cli', 'sources', 'materials', 'environment',
                'heads', 'windows', 'budget', 'relay_request'}
    _require(set(config) == required, 'local campaign configuration fields differ from the supported recipe')
    for field in ('repository', 'cli'):
        _require(Path(config[field]).is_absolute(), f'{field} requires an absolute local path')
    _require(isinstance(config['environment'], dict)
             and all(isinstance(k, str) and isinstance(v, str) for k, v in config['environment'].items()),
             'a complete explicit child-process environment is required')
    validate_wire(config['relay_request'], 'ImpactRelayRequest')
    for name in ('output', 'callers_output'):
        options = config['relay_request'].get(name, {})
        _require(options.get('after') is None and options.get('tier', 'compact') == 'compact',
                 'campaign delivery starts at the first compact page')
    materials = {('tool', 'entmoot'): Path(config['cli']), ('profile', 'execution'): path.resolve(strict=True),
                 ('tool', 'campaign-python'): Path(sys.executable)}
    # Pin executable selection as well as PATH spelling. These are entry-point
    # bytes, not a claim to capture SDK libraries or an external build closure.
    for name in ('git', 'node', 'java', 'dotnet', 'go', 'cargo', 'rustc', 'python'):
        selected = shutil.which(name, path=next((value for key, value in config['environment'].items()
                                               if key.upper() == 'PATH'), ''))
        if selected:
            materials[('tool', 'path-' + name)] = Path(selected).resolve(strict=True)
    # Git verification itself uses the host's Git, independently of child PATH.
    selected_git = shutil.which('git')
    _require(selected_git is not None, 'Git input verification tool unavailable')
    materials[('tool', 'verification-git')] = Path(selected_git).resolve(strict=True)
    for key, value in config['environment'].items():
        if key.startswith('ENTMOOT_') and (key.endswith('_ADAPTER') or key == 'ENTMOOT_PYTHON_ENVIRONMENT'):
            candidate = Path(value)
            _require(candidate.is_absolute(), 'adapter/environment selection requires an absolute path')
            if candidate.is_file():
                role = 'environment' if key == 'ENTMOOT_PYTHON_ENVIRONMENT' else 'tool'
                materials[(role, key)] = candidate
    for row in config['materials']:
        _require(set(row) == {'role', 'id', 'path'}, 'invalid material binding')
        key = row['role'], row['id']
        _require(key not in materials, 'duplicate material binding')
        _require(Path(row['path']).is_absolute(), 'material binding requires an absolute path')
        materials[key] = Path(row['path'])
    for revision, root in config['sources'].items():
        _require(Path(root).is_absolute(), f'source root requires an absolute path: {revision}')
    return config, materials


def _git(root: Path, *args: str) -> bytes:
    return _run(['git', '-c', 'core.fsmonitor=false', *args], cwd=root)


def verify_checkout(repository: Path, root: Path, revision: str, *, generated_paths: dict[str, str] | None = None) -> Path:
    """Fence source bytes and return Git's path spelling for external tools.

    Node's module loader rejects Windows verbatim paths accepted by Rust. Check
    filesystem identity before using Git's spelling; never change source bytes.
    Optional generated inputs require separately pinned digests.
    """
    selected = Path(_git(root, 'rev-parse', '--show-toplevel').decode().strip())
    _require(root.samefile(selected),
             'source root must be the Git worktree top level')
    common = lambda path: Path(_git(path, 'rev-parse', '--path-format=absolute', '--git-common-dir').decode().strip()).resolve()
    _require(common(root).samefile(common(repository)), 'source worktree belongs to a different repository')
    _require(_git(root, 'rev-parse', 'HEAD').decode().strip() == revision, 'source worktree HEAD differs from requested revision')
    _require(not _git(root, 'ls-files', '-z', '--', '.entmoot'), 'tracked .entmoot paths cannot be qualified source')
    permitted = set()
    for relative, expected in (generated_paths or {}).items():
        _require(relative and '\\' not in relative and not Path(relative).is_absolute()
                 and not any(part in ('', '.', '..', '.git', '.entmoot') for part in relative.split('/')),
                 'invalid generated source path')
        path = root / relative
        _require(inside(path, root) and path.is_file() and digest(path.read_bytes()) == expected,
                 'generated source input is missing or differs from its pinned digest')
        permitted.add(b'?? ' + relative.encode('utf-8'))
    changes = _git(root, 'status', '--porcelain=v1', '-z', '--untracked-files=all', '--', '.', ':(exclude).entmoot')
    _require(all(entry in permitted for entry in changes.split(b'\0') if entry),
             'source worktree contains modified or unqualified untracked source')
    entries = _git(root, 'ls-files', '--stage')
    _require(not entries.startswith(b'160000 ') and b'\n160000 ' not in entries,
             'submodule source qualification is not implemented for this campaign lane')
    return selected


def prepare(config_path: Path, destination: Path) -> Path:
    config, materials = configuration(config_path)
    repository, cli = Path(config['repository']), Path(config['cli'])
    revisions, comparisons = select_comparisons(repository, heads=config['heads'],
                                               windows=[tuple(row) for row in config['windows']])
    _require(set(config['sources']) == {row['revision'] for row in revisions},
             'source bindings must exactly match the campaign revisions')
    for row in revisions:
        verify_checkout(repository, Path(config['sources'][row['revision']]), row['revision'])
    fingerprint = json.loads(_run([str(cli), 'fingerprint']))['implementation']
    policy = json.loads(_run([str(cli), 'policy', '--config', 'v1']))['policy_id']
    inputs = {'schema_version': 'entmoot.revision_campaign_inputs.v1',
              'implementation': fingerprint, 'evaluator_digest': evaluator_digest(),
              'repository_id': config['repository_id'], 'policy_ids': [policy], 'enrichment': 'automatic',
              'materials': [{'role': role, 'id': name, 'content_digest': material_digest(path)}
                            for (role, name), path in sorted(materials.items())],
              'revisions': revisions, 'comparisons': comparisons, 'budget': config['budget']}
    # Exclusive namespace: even an empty existing directory is not a fresh run.
    destination.mkdir(parents=False, exist_ok=False)
    (destination / 'data').mkdir()
    manifest = destination / 'inputs.json'
    with manifest.open('xb') as stream:
        stream.write(encoded(inputs))
    _, canonical, _ = verify_inputs(manifest, cli=cli, repository=repository,
                                     repository_id=config['repository_id'], materials=materials)
    with CampaignStore(destination / 'campaign.sqlite3', manifest_digest=canonical, create=True) as store:
        store.put_receipts([('inputs', manifest.read_bytes()), ('execution-profile', config_path.read_bytes())])
    return manifest


@contextmanager
def run_lock(directory: Path):
    """An OS-owned lock releases on process exit; the sidecar is never unlinked."""
    with (directory / 'run.lock').open('a+b') as stream:
        stream.seek(0, 2)
        if stream.tell() == 0:
            stream.write(b'\0')
            stream.flush()
        stream.seek(0)
        try:
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise ContractError('another campaign executor owns this run') from error
        try:
            yield
        finally:
            stream.seek(0)
            if os.name == 'nt':
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(stream, fcntl.LOCK_UN)


class Executor:
    def __init__(self, directory: Path, config_path: Path):
        self.directory = directory.resolve(strict=True)
        self.data_directory = (self.directory / 'data').resolve(strict=True)
        _require(inside(self.data_directory, self.directory), 'campaign data escaped the owned run directory')
        self.config_path = config_path.resolve(strict=True)
        self.config, self.materials = configuration(self.config_path)
        self.cli = Path(self.config['cli']).resolve(strict=True)
        self.repository = Path(self.config['repository']).resolve(strict=True)
        self.inputs, self.input_digest, _ = self.verify()
        self.store = CampaignStore(self.directory / 'campaign.sqlite3', manifest_digest=self.input_digest)
        try:
            _require(json.loads(self.store.get_receipt('inputs')) == self.inputs,
                     'stored inputs differ from the active manifest')
            _require(json.loads(self.store.get_receipt('execution-profile')) == self.config,
                     'retained execution profile differs from current configuration')
        except BaseException:
            self.store.close()
            raise

    def verify(self):
        config, materials = configuration(self.config_path)
        _require(config == self.config, 'execution configuration changed during the run')
        return verify_inputs(self.directory / 'inputs.json', cli=self.cli, repository=self.repository,
                             repository_id=self.config['repository_id'], materials=materials)

    def command(self, key: str, args: list[str], cwd: Path) -> bytes:
        """Retain stdout/stderr and process result before interpreting semantic output.

        Failed/interrupted attempts are never reclassified or implicitly overwritten.
        A new namespace is required to rerun one; completed revision receipts can resume.
        CLI producer budgets govern native subprocesses. No outer timeout kills a CLI
        while it owns live stores or leaves native children running behind a receipt.
        """
        _require(self.store.get_receipt(key + '/process') is None, 'incomplete or failed operation requires a fresh run')
        self.verify()
        # Explicit CLI selection wins over the pinned environment and workspace
        # config, so publication and scratch writes stay within the run lifetime.
        args = ['--data-dir', str(self.data_directory), *args]
        result = measured_command([str(self.cli), *args], cwd,
                                  self.directory / 'commands', 'entmoot',
                                  self.config['environment'], None)
        stdout, stderr = Path(result['stdout']).read_bytes(), Path(result['stderr']).read_bytes()
        self.store.put_receipts([(key + '/stdout', stdout), (key + '/stderr', stderr),
                                 (key + '/process', encoded(result))])
        _require(result['status'] == 'complete', f'campaign command failed; retained process receipt: {key}')
        self.verify()
        return stdout

    def source_graph(self, graph: dict, root: Path, revision: str) -> None:
        _require(graph['implementation'] == self.inputs['implementation'], 'graph implementation mismatch')
        _require(graph['policy_ids'] == self.inputs['policy_ids'], 'graph policy mismatch')
        snapshot = graph['snapshot']
        _require(snapshot['repository_id'] == self.inputs['repository_id'] and snapshot['revision'] == revision,
                 'graph snapshot attribution mismatch')
        # Production correctly describes a checked-out scan as working_tree. Its
        # represented bytes must additionally match immutable Git blobs here.
        _require(snapshot['source_state'] == 'working_tree' and snapshot.get('snapshot_digest'),
                 'checkout scan lacks its working-tree digest')
        verify_checkout(self.repository, root, revision)
        for file in graph['snapshot_files']:
            path = file['path']
            _require(path and '\\' not in path and not path.startswith('/')
                     and not any(part in ('.', '..', '.git', '.entmoot') for part in path.split('/')),
                     'administration or invalid path in source coverage')
            blob = _git(self.repository, 'cat-file', 'blob', f'{revision}:{path}')
            _require(digest(blob) == file['content_digest'], 'inventoried bytes differ from the immutable Git source')

    def revision(self, revision: str) -> dict:
        key = 'revision/' + revision
        root = Path(self.config['sources'][revision]).resolve(strict=True)
        verify_checkout(self.repository, root, revision)
        complete = self.store.get_receipt(key + '/complete')
        if complete is not None:
            receipt = json.loads(complete)
            graph = json.loads(self.store.get_receipt(key + '/graph'))
            self.source_graph(graph, root, revision)
            _require(material_digest(Path(receipt['graph'])) == digest(self.store.get_receipt(key + '/graph')),
                     'published graph changed since receipt')
            for retained in receipt['retained']:
                _require(self.store.get_receipt(retained) is not None, 'completed revision lacks retained evidence')
            return receipt
        output = self.directory / revision
        output.mkdir(exist_ok=False)
        publication = self.data_directory
        indexed = output / 'indexed.json'
        self.command(key + '/index', ['index', '--root', str(root), '--repo', self.inputs['repository_id'],
                                     '--revision', revision, '--out', str(indexed), '--config', 'v1'], root)
        self.source_graph(json.loads(indexed.read_bytes()), root, revision)
        plan = json.loads(self.command(key + '/plan', ['bind', '--root', str(root), '--graph', str(indexed),
                                                      '--plan-only'], root))
        for stage in plan['stages']:
            if stage.get('adapter_path'):
                selected = Path(stage['adapter_path'])
                _require(any(role == 'tool' and path.samefile(selected)
                             for (role, _), path in self.materials.items()),
                         'automatically selected native adapter is not pinned; add its material and prepare a fresh run')
        summary = json.loads(self.command(key + '/bind', ['bind', '--root', str(root), '--graph', str(indexed),
                                                         '--timeout-seconds', str(self.inputs['budget']['producer_timeout_seconds'])], root))
        _require(not summary['structural_only'] and not summary['plan_only'], 'automatic enrichment was not executed')
        graph_path = Path(summary['publication']['graph']).resolve(strict=True)
        _require(inside(graph_path, publication), 'publication graph escaped the owned output root')
        graph_bytes = graph_path.read_bytes()
        graph = json.loads(graph_bytes)
        self.source_graph(graph, root, revision)
        _require(summary['snapshot'] == graph['snapshot'] and summary['policy_ids'] == graph['policy_ids'],
                 'native execution summary attribution mismatch')
        retained = []
        statuses = []
        for stage in summary['stages']:
            receipt = stage.get('native_receipt')
            statuses.append({'family': stage['family'], 'stage': stage['stage'],
                             'semantic_status': receipt['semantic_status'] if receipt else None,
                             'limitations': stage['limitations'],
                             'native_limitations': receipt.get('limitations', []) if receipt else []})
            if receipt:
                validate_wire(receipt, 'NativeReceipt')
                _require(receipt['implementation'] == self.inputs['implementation'], 'native receipt implementation mismatch')
            if stage.get('manifest'):
                manifest = Path(stage['manifest']).resolve(strict=True)
                _require(inside(manifest, publication), 'native evidence escaped the owned publication')
                # The publication already owns native evidence; keep only its
                # compact status here instead of copying provider payloads.
        receipt = {'graph': str(graph_path), 'store': summary['publication']['store'],
                   'stages': statuses, 'coverage': summary['coverage'], 'retained': retained,
                   'native_target_mode': 'unqualified', 'compiler_runtime_verification': 'unqualified',
                   'external_build_closure': 'unqualified'}
        self.verify()
        self.store.put_receipts([(key + '/graph', graph_bytes), (key + '/complete', encoded(receipt))])
        return receipt

    def pair(self, pair: dict, revisions: dict[str, dict]) -> dict:
        base_id, head_id = pair['base_revision'], pair['head_revision']
        key = 'pair/' + base_id + '/' + head_id
        base, head = revisions[base_id], revisions[head_id]
        complete = self.store.get_receipt(key + '/complete')
        if complete:
            saved = json.loads(complete)
            checked = self.account_pair(pair, key, saved['relay_keys'], saved['caller_keys'])
            _require(all(saved.get(field) == value for field, value in checked.items()),
                     'saved campaign accounting disagrees with retained evidence')
            return saved
        args = ['impact', '--base-graph', base['graph'], '--head-graph', head['graph'], '--no-revalidate']
        self.command(key + '/impact', args, self.directory)
        request, relay_keys, caller_keys = copy.deepcopy(self.config['relay_request']), [], {}
        request_path = self.directory / ('request-' + base_id + '-' + head_id + '.json')
        for page_number in range(self.inputs['budget']['subject_pages']):
            request_path.write_bytes(encoded(request))
            page_key = key + f'/relay/{page_number}'
            page = json.loads(self.command(page_key, ['relay-impact', '--graph', head['graph'], '--compare-graph', base['graph'],
                                                     '--store', head['store'], '--request', str(request_path)], self.directory))
            validate_wire(page, 'ImpactRelayResult')
            relay_keys.append(page_key + '/stdout')
            for item in page['items']:
                callers = item.get('callers')
                address = item['callable']['occurrence']['address_id']
                for number in range(self.inputs['budget']['caller_pages_per_subject']):
                    if callers is None or not callers['page'].get('next_cursor'):
                        break
                    query = copy.deepcopy(callers['query'])
                    query['output'] = copy.deepcopy(self.config['relay_request'].get('callers_output', {}))
                    query['output']['after'] = callers['page']['next_cursor']
                    request_path.write_bytes(encoded(query))
                    caller_key = key + f'/callers/{address}/{number}'
                    callers = json.loads(self.command(caller_key, ['request', '--graph', head['graph'], '--store', head['store'],
                                                                 '--request', str(request_path)], self.directory))
                    validate_wire(callers, 'QueryResponse')
                    caller_keys.setdefault(address, []).append(caller_key + '/stdout')
            if not page['page'].get('next_cursor'):
                break
            request = copy.deepcopy(page['request'])
            # Receipt requests reset delivery controls to identify the semantic
            # query. Preserve the run's pinned page size/bytes on continuation.
            request['output'] = copy.deepcopy(self.config['relay_request'].get('output', {}))
            request['output']['after'] = page['page']['next_cursor']
        result = self.account_pair(pair, key, relay_keys, caller_keys)
        result.update({'relay_keys': relay_keys, 'caller_keys': caller_keys})
        self.verify()
        self.store.put_receipts([(key + '/complete', encoded(result))])
        return result

    def account_pair(self, pair: dict, key: str, relay_keys: list[str], caller_keys: dict) -> dict:
        read = lambda name: json.loads(self.store.get_receipt(name))
        base = read('revision/' + pair['base_revision'] + '/graph')
        head = read('revision/' + pair['head_revision'] + '/graph')
        impact = read(key + '/impact/stdout')
        expected = self.inputs['implementation']
        counts = summarize_occurrences(occurrence_accounting(base, head, impact, expected=expected))
        relay = summarize_relay(impact, (read(name) for name in relay_keys), expected=expected,
                                caller_pages={address: [read(name) for name in names] for address, names in caller_keys.items()})
        return {'comparison': pair, 'occurrences': counts, 'callers': relay}

    def run(self, *, max_operations: int | None = None, wall_seconds: float | None = None) -> dict:
        """Yield only between committed revision/comparison units, never kill a producer.

        Admission budgets affect scheduling only. Source/tool/configuration receipts
        are revalidated even for units reused from an earlier invocation.
        Creating run-dir/stop.request requests a graceful stop at the next boundary;
        remove it explicitly before resuming the same command and run directory.
        """
        _require(max_operations is None or max_operations >= 0, 'max-operations must be nonnegative')
        _require(wall_seconds is None or (wall_seconds >= 0 and wall_seconds < float('inf')),
                 'wall-seconds must be finite and nonnegative')
        started, executed = time.monotonic(), 0
        revisions, pairs = {}, []

        def admit(key):
            if self.store.get_receipt(key + '/complete') is not None:
                return True
            reason = ('stop_requested' if (self.directory / 'stop.request').exists() else
                      'operation_budget' if max_operations is not None and executed >= max_operations else
                      'wall_budget' if wall_seconds is not None and time.monotonic() - started >= wall_seconds else None)
            if reason is None:
                return True
            self.verify()
            self.store.put_receipts([('progress', encoded({'state': 'paused', 'reason': reason,
                'next_operation': key, 'input_digest': self.input_digest,
                'completed_revisions': list(revisions), 'completed_pairs': len(pairs)}))])
            return False

        try:
            with run_lock(self.directory):
                for row in self.inputs['revisions']:
                    key = 'revision/' + row['revision']
                    if not admit(key):
                        return json.loads(self.store.get_receipt('progress'))
                    reused = self.store.get_receipt(key + '/complete') is not None
                    revisions[row['revision']] = self.revision(row['revision'])
                    executed += not reused
                for pair in self.inputs['comparisons']:
                    key = 'pair/' + pair['base_revision'] + '/' + pair['head_revision']
                    if not admit(key):
                        return json.loads(self.store.get_receipt('progress'))
                    reused = self.store.get_receipt(key + '/complete') is not None
                    pairs.append(self.pair(pair, revisions))
                    executed += not reused
                summary = {'state': 'completed', 'input_digest': self.input_digest, 'revisions': revisions, 'pairs': pairs,
                           'process_success': True, 'qualification': 'bounded execution and receipt accounting only',
                           'limitations': ['Native target modes, runtime acceptance, external build closure and performance remain unqualified.',
                                           'Working-tree snapshots are checked against immutable Git source blobs; administrative files are excluded.',
                                           'Failed or forcibly interrupted revision attempts require a fresh run namespace; graceful stops retain completed units.']}
                self.verify()
                self.store.put_receipts([('complete', encoded(summary)), ('progress', encoded({'state': 'completed', 'input_digest': self.input_digest}))])
                return summary
        finally:
            self.store.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['prepare', 'run'])
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--max-operations', type=int, help='gracefully pause after this many new revision/comparison units')
    parser.add_argument('--wall-seconds', type=float, help='stop admitting new units after this wall-clock allowance')
    args = parser.parse_args()
    if args.operation == 'prepare':
        print(prepare(args.config, args.run_dir))
    else:
        print(json.dumps(Executor(args.run_dir, args.config).run(max_operations=args.max_operations, wall_seconds=args.wall_seconds), indent=2))


if __name__ == '__main__':
    main()
