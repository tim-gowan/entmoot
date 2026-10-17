"""Freeze a bounded history frame, then grade source deltas in retained publications.

CI failure is a sampling stratum, never a bug oracle. This initial history lane
grades source inventory/content only; it cannot qualify dispatch or compatibility.
"""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime
import json
import os
from pathlib import Path
import re
import shutil

from evals.harness.contracts import ContractError
from evals.harness.measurement import command, database_bytes
from evals.harness.repository_campaign import run as run_repositories
from evals.harness.revision_campaign import canonical_json, digest, evaluator_digest, git_revision, material_digest


STRATA = ('failed_ci_candidate', 'ordinary', 'source_neutral')


def write(path: Path, document) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_json(document))


def configuration(path: Path) -> dict:
    spec = json.loads(path.read_bytes())
    required = {'id', 'seed', 'since', 'until', 'max_commits_per_repository', 'max_ci_pages',
                'max_source_files', 'max_source_bytes', 'command_timeout_seconds', 'quotas', 'repositories'}
    if set(spec) != required or set(spec['quotas']) != set(STRATA):
        raise ContractError('history configuration fields or strata differ')
    if datetime.fromisoformat(spec['since']) >= datetime.fromisoformat(spec['until']):
        raise ContractError('history interval must be increasing')
    for key in ('max_commits_per_repository', 'max_ci_pages', 'max_source_files',
                'max_source_bytes', 'command_timeout_seconds'):
        if type(spec[key]) is not int or spec[key] <= 0:
            raise ContractError('positive integer budget required: ' + key)
    if spec['max_ci_pages'] > 10 or spec['max_commits_per_repository'] > 10000:
        raise ContractError('metadata budgets exceed the local pilot bounds')
    if any(type(n) is not int or not 0 <= n <= 100 for n in spec['quotas'].values()):
        raise ContractError('stratum quotas must be integers between zero and 100')
    ids = set()
    for repo in spec['repositories']:
        if set(repo) != {'id', 'github', 'extensions'} or not re.fullmatch(r'[a-z0-9-]+', repo['id']):
            raise ContractError('invalid repository recipe')
        if repo['id'] in ids or not re.fullmatch(r'[\w.-]+/[\w.-]+', repo['github']):
            raise ContractError('duplicate repository or invalid GitHub coordinate')
        ids.add(repo['id'])
        if not repo['extensions'] or any(not re.fullmatch(r'\.[a-z]+', ext) for ext in repo['extensions']):
            raise ContractError('source extensions must be explicit')
    return spec


def select(frame: list[dict], quotas: dict, seed: int) -> tuple[list[dict], dict]:
    """Stable, disjoint sampling; adding failures never silently fills another stratum."""
    selected, counts = [], {}
    identities = [row['head'] for row in frame]
    if len(set(identities)) != len(identities):
        raise ContractError('duplicate revision in sampling frame')
    for stratum in STRATA:
        candidates = [row for row in frame if row['stratum'] == stratum]
        candidates.sort(key=lambda row: digest(canonical_json([seed, row['repository'], row['head']])))
        chosen = candidates[:quotas[stratum]]
        selected.extend(chosen)
        counts[stratum] = {'eligible': len(candidates), 'requested': quotas[stratum],
                          'selected': len(chosen), 'underfilled': max(0, quotas[stratum] - len(chosen))}
    return selected, counts


def source_delta(base: dict[str, str], head: dict[str, str]) -> dict:
    return {'added': sorted(head.keys() - base.keys()), 'deleted': sorted(base.keys() - head.keys()),
            'modified': sorted(path for path in base.keys() & head.keys() if base[path] != head[path])}


def grade_delta(expected: dict, observed: dict) -> dict:
    missing = {kind: sorted(set(expected[kind]) - set(observed[kind])) for kind in expected}
    unexpected = {kind: sorted(set(observed[kind]) - set(expected[kind])) for kind in expected}
    return {'passed': not any(missing.values()) and not any(unexpected.values()),
            'missing': missing, 'unexpected': unexpected, 'scope': 'selected-extension file content/membership'}


def grade_inventory(expected: dict, observed: dict) -> dict:
    missing = sorted(expected.keys() - observed.keys())
    unexpected = sorted(observed.keys() - expected.keys())
    different = sorted(path for path in expected.keys() & observed.keys() if expected[path] != observed[path])
    return {'passed': not (missing or unexpected or different), 'missing': missing,
            'unexpected': unexpected, 'different': different, 'expected_files': len(expected)}


def grade_episode(episode: dict, trees: dict, observed: dict, inventories: dict) -> dict:
    base, head = (episode['repository'] + '-' + episode[side] for side in ('base', 'head'))
    row = dict(episode, expected=source_delta(trees[base], trees[head]))
    if all(key in observed and key in inventories for key in (base, head)):
        row['observed'] = source_delta(observed[base], observed[head])
        row['grade'] = grade_delta(row['expected'], row['observed'])
        row['inventory_grades'] = {'base': inventories.get(base), 'head': inventories.get(head)}
        good_inventory = all(inventories.get(key, {}).get('passed', False) for key in (base, head))
        row['status'] = 'passed' if row['grade']['passed'] and good_inventory else 'failed'
    else:
        row['status'] = 'unavailable'
    return row


def blob_digests(raw: bytes) -> dict:
    """Decode Git's length-delimited batch output, independent of Entmoot storage."""
    offset, result = 0, {}
    while offset < len(raw):
        end = raw.index(b'\n', offset)
        oid, kind, size = raw[offset:end].split()
        if kind != b'blob':
            raise ContractError('oracle requires regular Git blobs')
        offset, size = end + 1, int(size)
        if offset + size >= len(raw) or raw[offset + size:offset + size + 1] != b'\n':
            raise ContractError('truncated Git oracle bytes')
        result[oid.decode('ascii')] = digest(raw[offset:offset + size])
        offset += size + 1
    return result


class Commands:
    def __init__(self, root: Path, timeout: int):
        self.root, self.timeout = root, timeout
        self.environment = os.environ.copy()

    def run(self, argv: list[str], name: str, *, allow_failure=False, stdin_path=None) -> bytes:
        result = command(argv, self.root, self.root / 'commands', name, self.environment, self.timeout, stdin_path=stdin_path)
        if result['status'] != 'complete':
            if allow_failure:
                return b''
            raise ContractError(f'{name}: {result["status"]}; see {result["stderr"]}')
        return Path(result['stdout']).read_bytes()


def harvest(spec_path: Path, root: Path) -> dict:
    spec = configuration(spec_path)
    root = root.resolve()
    root.mkdir(parents=True, exist_ok=False)
    write(root / 'configuration.json', spec)
    commands = Commands(root, spec['command_timeout_seconds'])
    selected, frames, repositories = [], [], []
    for repo in spec['repositories']:
        mirror = root / 'mirrors' / (repo['id'] + '.git')
        row = {'id': repo['id'], 'status': 'unavailable'}
        try:
            commands.run(['git', 'clone', '--bare', '--filter=blob:none',
                          'https://github.com/' + repo['github'] + '.git', str(mirror)], repo['id'] + '-clone')
            def git(*args):
                return commands.run(['git', '--git-dir', str(mirror), *args], repo['id'] + '-metadata')
            tip = git('rev-parse', 'HEAD').decode().strip()
            raw = git('log', '--first-parent', '--since=' + spec['since'], '--until=' + spec['until'],
                      '--max-count=' + str(spec['max_commits_per_repository'] + 1), '--format=%H', tip)
            heads = raw.decode().splitlines()
            capped = len(heads) > spec['max_commits_per_repository']
            heads = heads[:spec['max_commits_per_repository']]
            failed, ci_pages, ci_status = {}, [], 'available'
            for page in range(1, spec['max_ci_pages'] + 1):
                endpoint = (f'repos/{repo["github"]}/actions/runs?status=failure&per_page=100&page={page}'
                            f'&created={spec["since"][:10]}..{spec["until"][:10]}')
                raw_ci = commands.run(['gh', 'api', '-H', 'X-GitHub-Api-Version: 2022-11-28', endpoint],
                                      repo['id'] + '-ci', allow_failure=True)
                if not raw_ci:
                    ci_status = 'unavailable'
                    break
                document = json.loads(raw_ci)
                ci_pages.append({'endpoint': endpoint, 'total_count': document['total_count'],
                                 'retrieved': len(document['workflow_runs'])})
                for run in document['workflow_runs']:
                    failed.setdefault(run['head_sha'], []).append({key: run.get(key) for key in
                        ('id', 'run_attempt', 'head_sha', 'event', 'path', 'conclusion', 'html_url', 'created_at')})
                if len(document['workflow_runs']) < 100:
                    break
            frame = []
            for head in heads:
                commit = git_revision(mirror, head)
                if not commit['parents']:
                    continue
                parent = commit['parents'][0]
                paths = git('diff', '--name-only', '--no-renames', '-z', parent, head).decode('utf-8').split('\0')
                source_changes = [p for p in paths if Path(p).suffix in repo['extensions']]
                stratum = 'failed_ci_candidate' if head in failed else 'ordinary' if source_changes else 'source_neutral'
                frame.append({'repository': repo['id'], 'head': head, 'base': parent, 'parents': commit['parents'],
                              'tree': commit['tree_oid'], 'stratum': stratum, 'source_changed_paths': source_changes,
                              'ci_runs': failed.get(head, []), 'parent_policy': 'first_parent'})
            chosen, counts = select(frame, spec['quotas'], spec['seed'])
            selected.extend(chosen)
            frames.extend(frame)
            row.update(status='complete', tip=tip, mirror=str(mirror), counts=counts,
                       frame_capped=capped, ci_status=ci_status, ci_pages=ci_pages,
                       ci_unmatched_heads=len(set(failed) - set(heads)),
                       ci_qualification='metadata only; logs and code-failure causation unverified')
        except (ContractError, OSError, ValueError, KeyError) as error:
            row['error'] = str(error)
        repositories.append(row)
        print(json.dumps({'repository': repo['id'], 'status': row['status'], 'selected': len(selected)}), flush=True)
        write(root / 'selection.json', {'configuration': spec, 'repositories': repositories,
              'frame': frames, 'episodes': selected, 'sampler_digest': material_digest(Path(__file__))})
    return json.loads((root / 'selection.json').read_bytes())


def run(selection: Path, root: Path, cli: Path, retain: Path) -> dict:
    frozen = json.loads(selection.read_bytes())
    spec = frozen['configuration']
    root = root.resolve()
    root.mkdir(parents=True, exist_ok=False)
    commands = Commands(root, spec['command_timeout_seconds'])
    tools = {'cli': material_digest(cli), 'retain': material_digest(retain)}
    evaluator = evaluator_digest()
    sources = {}
    for name in ('history_pilot.py', 'repository_campaign.py', 'revision_campaign.py', 'measurement.py',
                 'run_revision_campaign.py', 'workflow_report.py', 'contracts.py', 'wire_schema.py', 'npm_lock_inventory.py'):
        path = Path(__file__).with_name(name)
        sources[name] = material_digest(path)
        saved = root / 'inputs' / 'harness' / name
        saved.parent.mkdir(parents=True, exist_ok=True)
        saved.write_bytes(path.read_bytes())
    write(root / 'inputs.json', {'selection_digest': material_digest(selection), 'tools': tools,
          'harness_sources': sources, 'evaluator_digest': evaluator, 'selection': frozen})
    configured = {repo['id']: repo for repo in spec['repositories']}
    mirrors = {}
    for repo in sorted({episode['repository'] for episode in frozen['episodes']}):
        url = 'https://github.com/' + configured[repo]['github'] + '.git'
        acquired = selection.resolve().parent / 'mirrors' / (repo + '.git')
        mirror = acquired if acquired.is_dir() else root / 'mirrors' / (repo + '.git')
        if not mirror.is_dir():
            commands.run(['git', 'clone', '--bare', '--filter=blob:none', url, str(mirror)], repo + '-clone')
        origin = commands.run(['git', '--git-dir', str(mirror), 'remote', 'get-url', 'origin'], repo + '-origin').decode().strip()
        if origin != url:
            raise ContractError('mirror origin differs from frozen repository: ' + repo)
        mirrors[repo] = str(mirror)
    recipes, trees, excluded = {}, {}, []
    # A partial clone's ls-tree -l can otherwise fetch one missing blob per
    # request. Fetch the selected revision closures once, with the same timeout.
    for repo, mirror in mirrors.items():
        revisions = sorted({ep[side] for ep in frozen['episodes'] if ep['repository'] == repo
                            for side in ('base', 'head')})
        if revisions:
            commands.run(['git', '--git-dir', mirror, '-c', 'gc.auto=0', 'fetch', '--refetch',
                          '--no-filter', 'origin', *revisions], repo + '-materialize-objects')
    for episode in frozen['episodes']:
        repo = episode['repository']
        for revision in (episode['base'], episode['head']):
            key = repo + '-' + revision
            if key in recipes or key in trees:
                continue
            raw = commands.run(['git', '--git-dir', mirrors[repo], 'ls-tree', '-r', '-l', '-z', revision], key + '-tree')
            files, sizes = {}, []
            for entry in raw.split(b'\0'):
                if not entry:
                    continue
                metadata, path = entry.split(b'\t', 1)
                mode, kind, oid, size = metadata.split()
                name = path.decode('utf-8')
                if Path(name).suffix in configured[repo]['extensions'] and mode in (b'100644', b'100755'):
                    files[name] = oid.decode()
                    sizes.append(int(size))
            trees[key] = files
            if len(files) > spec['max_source_files'] or sum(sizes) > spec['max_source_bytes']:
                excluded.append({'revision': key, 'reason': 'source_budget_exceeded',
                                 'files': len(files), 'bytes': sum(sizes)})
                continue
            recipes[key] = {'url': mirrors[repo], 'revision': revision, 'build': [], 'lock': None,
                            'subject': 'https://github.com/' + configured[repo]['github']}
    recipe_path = root / 'repositories.json'
    write(recipe_path, {'roots': list(recipes), 'repositories': recipes, 'packages': {},
                       'timeout_seconds': spec['command_timeout_seconds']})
    write(root / 'oracle-trees.json', trees)
    summary = run_repositories(recipe_path, root / 'publications', cli, 0, True)
    write(root / 'repositories-summary.json', summary)
    return finish(selection, root, cli, retain, excluded)


def verify_continuation(root: Path, inputs: dict, tools: dict, previous_runner: str) -> dict:
    """Permit a recorded coordinator transition, never a changed product or oracle."""
    if evaluator_digest() != inputs['evaluator_digest']:
        raise ContractError('original evaluator is required for continuation')
    if tools != inputs['tools']:
        raise ContractError('experiment executable changed')
    if previous_runner != inputs['harness_sources']['history_pilot.py']:
        raise ContractError('previous runner digest does not match frozen inputs')
    changes = {}
    for name, before in inputs['harness_sources'].items():
        if material_digest(root / 'inputs/harness' / name) != before:
            raise ContractError('archived harness source changed: ' + name)
        after = material_digest(Path(__file__).with_name(name))
        if before != after:
            if name != 'history_pilot.py':
                raise ContractError('continuation requires original harness dependency: ' + name)
            changes[name] = {'previous': before, 'current': after}
    return changes


def resume(selection: Path, root: Path, cli: Path, retain: Path, previous_runner: str) -> dict:
    """Reuse completed initialization; replay retention and grading after a clean stop.

    All retention operations replay through the idempotent production API. An
    interrupted child may have committed without a receipt, so neither existing
    locators nor old grades are treated as completed checks in this phase.
    """
    inputs = json.loads((root / 'inputs.json').read_bytes())
    if (root / 'report.json').exists() or (root / 'continuation.json').exists():
        raise ContractError('continuation requires an unfinished run with no prior continuation')
    if material_digest(selection) != inputs['selection_digest']:
        raise ContractError('frozen selection changed')
    tools = {'cli': material_digest(cli), 'retain': material_digest(retain)}
    changes = verify_continuation(root, inputs, tools, previous_runner)
    summary = json.loads((root / 'repositories-summary.json').read_bytes())
    recipes = json.loads((root / 'repositories.json').read_bytes())['repositories']
    if (set(recipes) != {row['id'] for row in summary['repositories']} or
            any(row.get('inventory_status') != 'complete' for row in summary['repositories'])):
        raise ContractError('continuation requires completed initialization for every recipe')
    interruption = json.loads((root / 'interruption.json').read_bytes())
    if interruption.get('status') != 'stopped_after_children_exited':
        raise ContractError('confirm the prior coordinator and children stopped before continuation')
    archive = root / 'before-continuation'
    archive.mkdir(exist_ok=False)
    for name in ('inputs.json', 'views.json', 'interruption.json'):
        if (root / name).exists():
            shutil.copyfile(root / name, archive / name)
    shutil.copyfile(Path(__file__), archive / 'continuation-runner.py')
    continuation = {'previous_inputs_digest': material_digest(root / 'inputs.json'),
        'previous_views_digest': material_digest(archive / 'views.json') if (archive / 'views.json').exists() else None,
        'interruption_digest': material_digest(root / 'interruption.json'), 'runner_changes': changes,
        'policy': 'Reuse initialization; replay all retention and independent byte/delta grades; preserve earlier measurements.'}
    write(root / 'continuation.json', continuation)
    trees = json.loads((root / 'oracle-trees.json').read_bytes())
    excluded = [{'revision': key, 'reason': 'not_initialized_in_original_run'} for key in trees if key not in recipes]
    return finish(selection, root, cli, retain, excluded, continuation)


def finish(selection: Path, root: Path, cli: Path, retain: Path, excluded: list,
           continuation: dict | None = None) -> dict:
    inputs = json.loads((root / 'inputs.json').read_bytes())
    frozen, tools = inputs['selection'], inputs['tools']
    evaluator = evaluator_digest() if continuation is not None else inputs['evaluator_digest']
    sources = ({name: material_digest(Path(__file__).with_name(name)) for name in inputs['harness_sources']}
               if continuation is not None else inputs['harness_sources'])
    spec = frozen['configuration']
    configured = {repo['id']: repo for repo in spec['repositories']}
    recipes = json.loads((root / 'repositories.json').read_bytes())['repositories']
    mirrors = {key.rsplit('-', 1)[0]: row['url'] for key, row in recipes.items()}
    trees = json.loads((root / 'oracle-trees.json').read_bytes())
    summary = json.loads((root / 'repositories-summary.json').read_bytes())
    commands = Commands(root, spec['command_timeout_seconds'])
    published = {row['id']: row for row in summary['repositories']}
    views, observed, inventories = {}, {}, {}
    for key, row in published.items():
        if row.get('inventory_status') != 'complete':
            continue
        if material_digest(cli) != tools['cli'] or material_digest(retain) != tools['retain']:
            raise ContractError('experiment executable changed')
        publication = row['publication']
        request = root / 'requests' / (key + '.json')
        write(request, {'graph': publication['graph'], 'source_store': publication['store'],
                        'database': str(root / 'knowledge.sqlite3'), 'locator': str(root / 'views' / (key + '.view.json'))})
        (root / 'views').mkdir(exist_ok=True)
        try:
            retained = json.loads(commands.run([str(retain), str(request)], key + '-retain'))
            graph = json.loads(Path(publication['graph']).read_bytes())
            repo = key.rsplit('-', 1)[0]
            extensions = configured[repo]['extensions']
            observed[key] = {item['path']: item['content_digest'] for item in graph['snapshot_files']
                             if Path(item['path']).suffix in extensions}
            blob_input = root / 'requests' / (key + '.blobs')
            blob_input.write_text('\n'.join(sorted(set(trees[key].values()))) + '\n', encoding='ascii')
            raw_blobs = commands.run(['git', '--git-dir', mirrors[repo], 'cat-file', '--batch'],
                                    key + '-oracle-bytes', stdin_path=blob_input)
            contents = blob_digests(raw_blobs) if trees[key] else {}
            expected_bytes = {path: contents[oid] for path, oid in trees[key].items()}
            inventories[key] = grade_inventory(expected_bytes, observed[key])
            views[key] = {'retained': retained, 'graph': publication['graph'], 'source_root': publication['source_root'],
                          'inventory_grade': inventories[key]}
        except (ContractError, OSError, ValueError) as error:
            excluded.append({'revision': key, 'reason': str(error)})
        write(root / 'views.json', views)
        print(json.dumps({'revision': key, 'graded_views': len(views),
                          'passed': inventories.get(key, {}).get('passed')}), flush=True)
    episodes = [grade_episode(episode, trees, observed, inventories) for episode in frozen['episodes']]
    result = {'episodes': episodes, 'counts': dict(Counter(row['status'] for row in episodes)),
              'requested_episodes': len(spec['repositories']) * sum(spec['quotas'].values()),
              'selected_episodes': len(episodes), 'retained_views': len(views), 'excluded': excluded,
              'database_bytes': database_bytes(root / 'knowledge.sqlite3') if views else None,
              'selection_digest': material_digest(selection), 'tools': tools,
              'limitations': ['Purposive repositories; bounded first-parent frame, not an ecosystem sample.',
                  'Source-neutral means unchanged selected-extension file bytes, not behavioral equivalence.',
                  'CI failures are metadata candidates; code failure and repair are not established.',
                  'This lane grades source deltas, not callable impact, native dispatch or security.',
                  'No language dependency installation or upstream code execution in this structural lane.']}
    if continuation is not None:
        result['continuation'] = continuation
        result['limitations'].append('Initialization predates a recorded coordinator change; implementation_unchanged covers the final retention/grading phase. Retention replay costs include repeated work.')
    result['implementation_unchanged'] = (evaluator_digest() == evaluator and
        all(material_digest(Path(__file__).with_name(name)) == value for name, value in sources.items()) and
        material_digest(cli) == tools['cli'] and material_digest(retain) == tools['retain'])
    write(root / 'report.json', result)
    if not result['implementation_unchanged']:
        raise ContractError('implementation changed during study; retained report is unqualified')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['harvest', 'run', 'resume'])
    parser.add_argument('--config', type=Path)
    parser.add_argument('--selection', type=Path)
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--cli', type=Path)
    parser.add_argument('--retain-helper', type=Path)
    parser.add_argument('--previous-runner-digest', help='original inputs.json history_pilot.py digest; required for resume')
    args = parser.parse_args()
    if args.operation == 'harvest':
        if not args.config:
            parser.error('harvest requires --config')
        result = harvest(args.config, args.run_dir)
        print(json.dumps({'selected': len(result['episodes'])}))
    else:
        if not all((args.selection, args.cli, args.retain_helper)):
            parser.error('run requires --selection, --cli and --retain-helper')
        if args.operation == 'resume':
            if not args.previous_runner_digest:
                parser.error('resume requires --previous-runner-digest')
            result = resume(args.selection, args.run_dir.resolve(), args.cli.resolve(), args.retain_helper.resolve(),
                            args.previous_runner_digest)
        else:
            result = run(args.selection, args.run_dir, args.cli.resolve(), args.retain_helper.resolve())
        print(json.dumps({key: result[key] for key in ('counts', 'selected_episodes', 'retained_views')}))


if __name__ == '__main__':
    main()
