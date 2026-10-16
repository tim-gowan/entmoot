"""Project npm v2/v3 lock placement into exact dependency-instance evidence.

This is evaluation evidence, not a compiler resolver or a compatibility verdict.
It does not execute installs, solve ranges, infer imports, or invent call edges.
"""
from pathlib import PurePosixPath


def dependency_rows(lock: dict) -> list[dict]:
    if lock.get('lockfileVersion') not in (2, 3) or not isinstance(lock.get('packages'), dict):
        raise ValueError('npm v2/v3 packages table required')
    packages = lock['packages']
    for path in packages:
        if path and (PurePosixPath(path).is_absolute() or '\\' in path or ':' in path
                     or any(p in ('', '.', '..') for p in path.split('/'))):
            raise ValueError('package location must be a relative POSIX path')
    rows = []
    for source, package in sorted(packages.items()):
        declarations = {}
        for field, kind in [('devDependencies', 'dev'), ('dependencies', 'required'),
                            ('optionalDependencies', 'optional')]:
            if field == 'devDependencies' and source:
                continue
            declarations.update({name: (kind, requested) for name, requested in package.get(field, {}).items()})
        for name, requested in package.get('peerDependencies', {}).items():
            optional = package.get('peerDependenciesMeta', {}).get(name, {}).get('optional', False)
            declarations.setdefault(name, ('optional_peer' if optional else 'peer', requested))
        for name, (kind, requested) in sorted(declarations.items()):
            parts = name.split('/')
            if not (len(parts) == 1 or (len(parts) == 2 and parts[0].startswith('@'))) or any(
                    part in ('', '.', '..') or '\\' in part or ':' in part for part in parts):
                raise ValueError('invalid dependency package name')
            parent = PurePosixPath(source)
            candidates = [parent, *parent.parents]
            target = next((str(p / 'node_modules' / name) for p in candidates
                           if p.name != 'node_modules' and str(p / 'node_modules' / name) in packages), None)
            row = {'source_path': source, 'name': name, 'kind': kind, 'requested': requested,
                   'status': 'missing', 'range_satisfaction': 'not_assessed'}
            if target is not None:
                value = packages[target]
                row.update(target_path=target, package_name=value.get('name', name),
                           version=value.get('version'), integrity=value.get('integrity'),
                           status='unsupported_link' if value.get('link') else 'pinned_location')
            rows.append(row)
    return rows
