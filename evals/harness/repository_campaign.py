"""Build pinned repositories and run Entmoot through a bounded dependency set.

This is a local executor over CampaignStore, not a new evidence format. Builds
may install prerequisites; only selected, pinned source repositories count as
analyzed. Lock placement is dependency evidence, never a semantic code bind.
"""
from __future__ import annotations

import argparse
from collections import deque
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tomllib
from urllib.parse import quote


from evals.harness.contracts import ContractError
from evals.harness.revision_campaign import (CampaignStore, OID, canonical_json, digest,
                                            evaluator_digest, material_digest, _run)
from evals.harness.run_revision_campaign import run_lock
from evals.harness.npm_lock_inventory import dependency_rows
from evals.harness.measurement import command, database_bytes


def beneath(root: Path, relative: str) -> Path:
    path = (root / relative).resolve()
    if path == root.resolve() or not path.is_relative_to(root.resolve()):
        raise ContractError(f'path must stay within the task directory: {relative}')
    return path


def package_url(ecosystem: str, name: str, version: str) -> str:
    return f'pkg:{ecosystem}/{quote(name, safe="/")}@{quote(version, safe="")}'


def lock_dependencies(root: Path, lock_name: str | None, include_dev: bool = False) -> tuple[list[str], list[str]]:
    """Return direct pinned dependencies, with unsupported cases left explicit."""
    if lock_name is None:
        return [], ['no_lock_configured']
    path = beneath(root, lock_name)
    if not path.is_file():
        return [], ['lock_unavailable:' + lock_name]
    gaps: list[str] = []
    packages: set[str] = set()
    if path.name == 'package-lock.json':
        document = json.loads(path.read_bytes())
        rows = dependency_rows(document)
        if '' not in document['packages']:
            return [], ['lock_root_unavailable']
        for row in rows:
            if row['source_path'] or (not include_dev and row['kind'] == 'dev'):
                continue
            if row['status'] == 'pinned_location' and row.get('version'):
                packages.add(package_url('npm', row['package_name'], row['version']))
            else:
                gaps.append(f"{row['name']}:{row['status']}")
    elif path.name in ('Cargo.lock', 'uv.lock'):
        data = tomllib.loads(path.read_text(encoding='utf-8'))
        entries = data.get('package', [])
        if path.name == 'Cargo.lock':
            manifest = tomllib.loads((path.parent / 'Cargo.toml').read_text(encoding='utf-8'))
            name = manifest.get('package', {}).get('name')
            roots = [entry for entry in entries if 'source' not in entry and (not name or entry['name'] == name)]
            local = lambda entry: 'source' not in entry
            ecosystem = 'cargo'
        else:
            roots = [entry for entry in entries if entry.get('source', {}).get('editable') == '.' or entry.get('source', {}).get('virtual') == '.']
            local = lambda entry: any(key in entry.get('source', {}) for key in ('editable', 'virtual', 'directory'))
            ecosystem = 'pypi'
        if not roots:
            return [], ['lock_root_unavailable']
        queue, visited = deque(roots), set()
        while queue:
            entry = queue.popleft()
            identity = entry['name'], entry.get('version'), canonical_json(entry.get('source'))
            if identity in visited:
                continue
            visited.add(identity)
            for dep in entry.get('dependencies', []):
                if isinstance(dep, str):
                    parts = dep.split(' ', 2)
                    name, version = parts[0], parts[1] if len(parts) > 1 else None
                    source = parts[2].strip('()') if len(parts) > 2 else None
                else:
                    name, version, source = dep['name'], dep.get('version'), dep.get('source')
                matches = [candidate for candidate in entries if candidate['name'] == name
                           and (version is None or candidate.get('version') == version)
                           and (source is None or candidate.get('source') == source)]
                if len(matches) != 1:
                    gaps.append(f'{name}:ambiguous_or_missing_lock_entry')
                elif local(matches[0]):
                    queue.append(matches[0])
                elif matches[0].get('version'):
                    target = matches[0]
                    registry = target.get('source', '')
                    if ecosystem == 'cargo' and not str(registry).startswith('registry+'):
                        gaps.append(f'{name}:non_registry_source')
                    elif ecosystem == 'pypi' and 'registry' not in registry:
                        gaps.append(f'{name}:non_registry_source')
                    else:
                        packages.add(package_url(ecosystem, name, target['version']))
                else:
                    gaps.append(f'{name}:version_or_registry_identity_unavailable')
        gaps.append('lock_scope_includes_resolved_platform_and_feature_variants')
    elif path.name == 'go.mod':
        # go.mod pins selected direct module versions. go.sum is an integrity
        # set and is never treated as a direct-dependency list.
        required, replaced, block = [], set(), None
        for line in path.read_text(encoding='utf-8').splitlines():
            content, _, comment = line.partition('//')
            content = content.strip()
            if not content:
                continue
            if content == ')':
                block = None
                continue
            fields = content.split()
            if fields[0] in ('require', 'replace'):
                kind, fields = fields[0], fields[1:]
                if fields == ['(']:
                    block = kind
                    continue
            else:
                kind = block
            if kind == 'require':
                if len(fields) != 2 or not fields[1].startswith('v'):
                    gaps.append('require_form_not_qualified')
                elif comment.strip() != 'indirect':
                    required.append(tuple(fields))
            elif kind == 'replace' and fields:
                replaced.add(fields[0])
        for name, version in required:
            if name in replaced:
                gaps.append(f'{name}:replacement_not_qualified')
            else:
                packages.add(package_url('golang', name, version))
        gaps.append('go_workspaces_and_environment_selection_not_qualified')
    elif path.name == 'packages.lock.json':
        document = json.loads(path.read_bytes())
        if document.get('version') not in (1, 2):
            return [], ['unsupported_nuget_lock_version']
        for framework, entries in document.get('dependencies', {}).items():
            for name, entry in entries.items():
                if entry.get('type') == 'Direct':
                    if entry.get('resolved'):
                        packages.add(package_url('nuget', name.lower(), entry['resolved']))
                    else:
                        gaps.append(f'{framework}:{name}:unresolved_direct_dependency')
                elif entry.get('type') not in ('Transitive', 'Project'):
                    gaps.append(f'{framework}:{name}:unsupported_dependency_kind')
        gaps.append('nuget_target_frameworks_combined')
    else:
        gaps.append('unsupported_lock_format:' + path.name)
    return sorted(packages), sorted(set(gaps))


def git(root: Path, *args: str) -> str:
    return _run(['git', '-C', str(root), *args]).decode('utf-8').strip()


def checkout(spec: dict, destination: Path) -> None:
    revision = spec['revision']
    if not OID.fullmatch(revision):
        raise ContractError('repository revisions must be full commit IDs')
    if not destination.exists():
        # Host line-ending defaults are not part of the pinned source recipe.
        # Persist the policy only in this owned clone; authored attributes still apply.
        _run(['git', 'clone', '--no-checkout', '--no-hardlinks', '--config', 'core.autocrlf=false',
              '--config', 'core.eol=lf', '--', spec['url'], str(destination)])
        # A pinned commit may not be on the remote default branch.
        if subprocess.run(['git', '-C', str(destination), 'cat-file', '-e', revision + '^{commit}'], capture_output=True).returncode:
            _run(['git', '-C', str(destination), 'fetch', '--depth=1', 'origin', revision])
        git(destination, 'checkout', '--detach', revision)
    else:
        # Reuse only this executor-owned checkout, restoring the configured pin.
        if git(destination, 'remote', 'get-url', 'origin') != spec['url']:
            raise ContractError('checkout identity differs from configured source')
        for setting, expected in (('core.autocrlf', 'false'), ('core.eol', 'lf')):
            selected = subprocess.run(['git', '-C', str(destination), 'config', '--local', '--get', setting],
                                      capture_output=True)
            if selected.returncode or selected.stdout.decode().strip() != expected:
                raise ContractError('checkout text policy differs; use a fresh campaign directory')
        if subprocess.run(['git', '-C', str(destination), 'cat-file', '-e', revision + '^{commit}'], capture_output=True).returncode:
            _run(['git', '-C', str(destination), 'fetch', '--depth=1', 'origin', revision])
        # Annotated tags are not campaign pins; configs must store peeled commits.
        commit = git(destination, 'rev-parse', revision + '^{commit}')
        if commit != revision:
            raise ContractError('repository revisions must be peeled commit IDs, not tags')
        git(destination, 'checkout', '--detach', '--force', commit)
        git(destination, 'reset', '--hard', commit)
    if git(destination, 'rev-parse', 'HEAD') != revision or git(destination, 'remote', 'get-url', 'origin') != spec['url']:
        raise ContractError('checkout identity differs from configured source')
    if not Path(git(destination, 'rev-parse', '--show-toplevel')).samefile(destination):
        raise ContractError('source must be the selected checkout root')
    if git(destination, 'ls-files', '--', '.entmoot'):
        raise ContractError('tracked .entmoot paths cannot be campaign publications')
    if git(destination, 'status', '--porcelain', '--untracked-files=no'):
        raise ContractError('tracked source changed in the campaign checkout')
    if any(line.startswith('160000 ') for line in git(destination, 'ls-files', '--stage').splitlines()):
        raise ContractError('submodule qualification is not implemented')


def native_inputs(environment: dict, repository: Path) -> dict:
    """Fence selected adapter/profile files, without hashing entire SDK trees.

    Discovery follows the shared adapter catalog; explicit paths are exclusive.
    Per-publication provenance still owns compiler and analyzed-source identity.
    """
    catalog_path = repository / 'tools/resolver-adapters/catalog.toml'
    catalog = tomllib.loads(catalog_path.read_text(encoding='utf-8'))
    paths = {catalog_path}
    selection = {key: value for key, value in environment.items()
                 if key.startswith('ENTMOOT_') and (key.endswith('_ADAPTER') or key in
                    ('ENTMOOT_ADAPTERS_ROOT', 'ENTMOOT_PYTHON_ENVIRONMENT', 'ENTMOOT_PACKAGE_API'))}
    adapter_root = Path(environment.get('ENTMOOT_ADAPTERS_ROOT', repository / '.tmp/adapter-build/resolver-adapters'))
    for family in catalog['family']:
        explicit = environment.get(family['env'])
        base = repository if family.get('artifact_kind') == 'repo_relative' else adapter_root
        candidates = [Path(explicit)] if explicit is not None else [base / path for path in family['relative_artifacts']]
        selected = next((path for path in candidates if path.is_file()), None)
        selection[family['env']] = {'configured': explicit, 'selected': str(selected.resolve()) if selected else None}
        if selected is not None:
            paths.add(selected)
            if selected.suffix == '.dll':
                for pattern in ('*.dll', '*.deps.json', '*.runtimeconfig.json'):
                    paths.update(selected.parent.glob(pattern))
    for key in ('ENTMOOT_PYTHON_ENVIRONMENT', 'ENTMOOT_PACKAGE_API'):
        if key in environment:
            paths.add(Path(environment[key]))
    if 'ENTMOOT_PYTHON_ENVIRONMENT' not in environment:
        paths.update(repository / path for path in ('.entmoot/python_environment.json',
                    '.tmp/adapter-build/pyright-program/python_environment.json'))
    return {'selection': selection, 'files': {str(path.resolve()): material_digest(path) if path.is_file() else None
            for path in sorted(paths)}}


def run(config_path: Path, directory: Path, cli: Path, depth: int, structural_only: bool) -> dict:
    config = json.loads(config_path.read_bytes())
    if set(config) - {'roots', 'repositories', 'packages', 'environment', 'include_dev', 'timeout_seconds', 'checks'}:
        raise ContractError('unsupported repository campaign configuration field')
    for name, spec in config['repositories'].items():
        if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_.-]*', name) or not OID.fullmatch(spec['revision']):
            raise ContractError('invalid repository ID or unpinned revision')
        allowed = {'url', 'revision', 'build', 'lock', 'subject'}
        if not {'url', 'revision', 'build', 'lock'}.issubset(spec) or set(spec) - allowed:
            raise ContractError('repository requires url, revision, build, lock; optional subject')
        if not all(isinstance(argv, list) and argv and all(isinstance(arg, str) for arg in argv) for argv in spec['build']):
            raise ContractError('build commands must be argument arrays')
    if not config['roots'] or not set(config['roots']).issubset(config['repositories']):
        raise ContractError('roots must name configured repositories')
    if not set(config['packages'].values()).issubset(config['repositories']):
        raise ContractError('package mapping names an unconfigured repository')
    check_names = set()
    for check in config.get('checks', []):
        if set(check) - {'name', 'arguments', 'request'} or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_.-]*', check.get('name', '')):
            raise ContractError('invalid check name or fields')
        if check['name'] in check_names or not isinstance(check.get('arguments'), list) or not check['arguments'] or not all(isinstance(arg, str) for arg in check['arguments']):
            raise ContractError('checks require unique names and command argument arrays')
        check_names.add(check['name'])
    environment = os.environ.copy()
    environment.update(config.get('environment', {}))
    # Configured paths are relative to the campaign invocation, not each clone.
    for key in list(environment):
        if key.startswith('ENTMOOT_') and (key.endswith('_ADAPTER') or key in
                ('ENTMOOT_ADAPTERS_ROOT', 'ENTMOOT_PYTHON_ENVIRONMENT', 'ENTMOOT_PACKAGE_API')):
            environment[key] = str(Path(environment[key]).resolve())
    # Record concrete executable selection, not a claim about SDK/build closure.
    names = {'git', sys.executable}
    names.update(argv[0].replace('{python}', sys.executable) for spec in config['repositories'].values() for argv in spec['build'])
    tools = {}
    for name in sorted(names):
        selected = shutil.which(name, path=environment.get('PATH'))
        tools[name] = {'path': selected, 'digest': material_digest(Path(selected)) if selected else None}
    cli = cli.resolve(strict=True)
    context = {'configuration': config, 'depth': depth, 'structural_only': structural_only,
               'cli': material_digest(cli), 'evaluator': evaluator_digest(),
               'repository_runner': material_digest(Path(__file__)), 'tools': tools,
               'native_inputs': None if structural_only else native_inputs(environment, Path(__file__).resolve().parents[2])}
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    data_directory = directory / 'data'
    data_directory.mkdir(exist_ok=True)
    database = directory / 'campaign.sqlite3'
    with run_lock(directory), CampaignStore(database, manifest_digest=digest(canonical_json(context)), create=not database.exists()) as store:
        store.put_receipts([('inputs', canonical_json(context))])
        queue = deque((name, 0) for name in config['roots'])
        selected = set(config['roots'])
        repositories, dependencies = [], []
        while queue:
            name, level = queue.popleft()
            spec = config['repositories'][name]
            root, build, logs = (beneath(directory, prefix + '/' + name) for prefix in ('sources', 'build', 'logs'))
            root.parent.mkdir(parents=True, exist_ok=True)
            build_existed = build.is_dir()
            build.mkdir(parents=True, exist_ok=True)
            logs.mkdir(parents=True, exist_ok=True)
            row = {'id': name, 'revision': spec['revision'], 'depth': level, 'limitations': []}
            try:
                checkout(spec, root)
            except (ContractError, OSError, ValueError) as error:
                row.update(build_status='unavailable', inventory_status='unavailable', limitations=[str(error)])
                repositories.append(row)
                continue
            prior = store.get_receipt('repository/' + name)
            prior = json.loads(prior) if prior else {}
            row['build_reused'] = build_existed and prior.get('build_status') == 'complete'
            lock = beneath(root, spec['lock']) if spec['lock'] else None
            if row['build_reused'] and prior.get('dependency_lock'):
                if lock is None or not lock.is_file() or material_digest(lock) != prior['dependency_lock']['content_digest']:
                    raise ContractError('retained dependency lock changed; use a fresh campaign')
            builds = prior.get('builds', []) if row['build_reused'] else []
            if not row['build_reused']:
                replacements = {'{checkout}': str(root), '{build}': str(build), '{python}': sys.executable}
                for number, recipe in enumerate(spec['build']):
                    argv = []
                    for arg in recipe:
                        for token, value in replacements.items():
                            arg = arg.replace(token, value)
                        argv.append(arg)
                    # Windows executable resolution requires the installed .cmd
                    # spelling for tools such as npm; no shell string is built.
                    argv[0] = shutil.which(argv[0], path=environment.get('PATH')) or argv[0]
                    result = command(argv, root, logs, 'build-' + str(number), environment, config.get('timeout_seconds', 1200))
                    builds.append(result)
                    if result['status'] != 'complete':
                        break
            row.update(builds=builds, build_status=builds[-1]['status'] if builds else 'not_configured')
            # Builds may rewrite lock/sum files; restore the pin so publication still
            # observes the configured revision (module caches stay outside the tree).
            if git(root, 'status', '--porcelain', '--untracked-files=no'):
                git(root, 'reset', '--hard', 'HEAD')
                row['limitations'].append('build_tracked_mutations_reset_to_pin')
            row['dependency_lock'] = None
            if lock is not None and lock.is_file():
                retained = build / ('resolved-' + lock.name)
                shutil.copyfile(lock, retained)
                row['dependency_lock'] = {'path':spec['lock'], 'retained':str(retained),
                                          'content_digest':material_digest(lock),
                                          'tracked':bool(git(root,'ls-files','--',spec['lock']))}
            pinned, gaps = lock_dependencies(root, spec['lock'], config.get('include_dev', False))
            row['limitations'].extend(gaps)
            for package in pinned:
                target = config['packages'].get(package)
                status = 'unmapped' if target is None else 'depth_limit' if level >= depth else 'already_selected' if target in selected else 'selected'
                dependencies.append({'source': name, 'package': package, 'target': target, 'status': status})
                if status == 'selected':
                    selected.add(target)
                    queue.append((target, level + 1))
            if git(root, 'status', '--porcelain', '--untracked-files=no'):
                row.update(inventory_status='unavailable')
                row['limitations'].append('build_modified_tracked_source')
            else:
                argv = [str(cli), '--data-dir', str(data_directory), 'init', '--root', str(root), '--repo', name]
                if structural_only:
                    argv.append('--structural-only')
                if spec.get('subject'):
                    argv.extend(['--subject', spec['subject']])
                result = command(argv, root, logs, 'entmoot-init', environment, config.get('timeout_seconds', 1200))
                row.update(inventory_status=result['status'], inventory=result)
                if result['status'] == 'complete':
                    summary = json.loads(Path(result['stdout']).read_bytes())
                    row.update(publication=summary['publication'], coverage=summary['coverage'], stages=summary.get('stages'))
                    if not structural_only:
                        bind_argv = [str(cli), '--data-dir', str(data_directory), 'bind', '--root', str(root)]
                        if spec.get('subject'):
                            bind_argv.extend(['--subject', spec['subject']])
                        bind = command(bind_argv, root, logs, 'entmoot-bind', environment, config.get('timeout_seconds', 1200))
                        row['bind'] = bind
                        row['bind_status'] = bind['status']
                        if bind['status'] == 'complete':
                            try:
                                bind_summary = json.loads(Path(bind['stdout']).read_bytes())
                                row.update(
                                    publication=bind_summary.get('publication', row['publication']),
                                    stages=bind_summary.get('stages', row.get('stages')),
                                )
                            except (ValueError, UnicodeDecodeError, KeyError):
                                row.setdefault('limitations', []).append('bind_summary_not_json')
                        else:
                            row.setdefault('limitations', []).append('native_bind_incomplete')
                else:
                    row['bind_status'] = 'skipped_init_incomplete'
            row['limitations'].append('build_success_is_not_language_or_dependency_bind_qualification')
            row['database_bytes'] = database_bytes(Path(row['publication']['store'])) if row.get('publication') else None
            repositories.append(row)
            store.put_receipts([('repository/' + name, canonical_json(row))])
        # Configured Entmoot operations can compare selected publications or run
        # typed reads. A missing publication is an unavailable check, not a
        # negative relation. The CLI remains the sole interpreter of semantics.
        bindings = {}
        for repository in repositories:
            if repository['inventory_status'] == 'complete':
                name = repository['id']
                bindings['root:' + name] = str(beneath(directory, 'sources/' + name))
                for field in ('graph', 'store'):
                    bindings[field + ':' + name] = repository['publication'][field]
        checks = []
        for check in config.get('checks', []):
            logs = beneath(directory, 'checks/' + check['name'])
            logs.mkdir(parents=True, exist_ok=True)
            values = dict(bindings)
            if 'request' in check:
                request = logs / 'request.json'
                request.write_bytes(canonical_json(check['request']))
                values['request'] = str(request)
            missing = []
            def substitute(match):
                token = match.group(1)
                if token not in values:
                    missing.append(token)
                return values.get(token, match.group(0))
            argv = [re.sub(r'\{([^{}]+)\}', substitute, arg) for arg in check['arguments']]
            if missing:
                result = {'name': check['name'], 'status': 'unavailable', 'limitations': ['publication_or_request_unavailable:' + token for token in sorted(set(missing))]}
            else:
                result = command([str(cli), '--data-dir', str(data_directory), *argv], directory, logs, 'entmoot', environment, config.get('timeout_seconds', 1200))
                result.update(name=check['name'], qualification='process_only; interpret the returned operation status and limitations')
                output = Path(result['stdout'])
                if output.is_file() and output.stat().st_size <= 65536:
                    try:
                        result['response'] = json.loads(output.read_bytes())
                    except (ValueError, UnicodeDecodeError):
                        result['limitations'] = ['no_json_response']
                else:
                    result['limitations'] = ['response_in_task_file']
            checks.append(result)
            store.put_receipts([('check/' + check['name'], canonical_json(result))])
        result = {'repositories': repositories, 'dependencies': dependencies, 'checks': checks,
                  'structural_only': structural_only, 'depth': depth,
                  'limitations': ['Unmapped and unsupported dependencies remain outside analyzed scope.',
                                 'Dependency selection does not create code edges; native joins remain producer-owned.']}
        store.put_receipts([('complete', canonical_json(result))])
        return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['run', 'locks'])
    parser.add_argument('--config', type=Path)
    parser.add_argument('--run-dir', type=Path)
    parser.add_argument('--cli', type=Path)
    parser.add_argument('--depth', type=int, default=1)
    parser.add_argument('--structural-only', action='store_true')
    parser.add_argument('--root', type=Path)
    parser.add_argument('--lock')
    args = parser.parse_args()
    if args.operation == 'locks':
        packages, limitations = lock_dependencies(args.root, args.lock)
        result = {'packages': packages, 'limitations': limitations}
    else:
        if args.config is None or args.run_dir is None or args.cli is None or not 0 <= args.depth <= 32:
            parser.error('run requires --config, --run-dir, --cli and a depth between 0 and 32')
        result = run(args.config, args.run_dir, args.cli, args.depth, args.structural_only)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
