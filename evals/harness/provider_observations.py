"""Subject-agnostic, presence-only SARIF for independently executed observations.

This fixture provider verifies supplied source spans and inventories observations
at scenario/source/configuration grain. It supplies no inferred call edges,
security verdicts, missing-behavior claims or negative coverage.
"""
from __future__ import annotations

import json
from pathlib import Path, PurePosixPath
import re

from evals.harness.revision_campaign import canonical_json, digest

PRODUCER = 'entmoot.fixture.runtime-observation'


def _observations(rows: list[dict]) -> dict[str, dict]:
    result = {}
    for row in rows:
        name = row.get('id')
        if not isinstance(name, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,127}', name):
            raise ValueError('observation requires a bounded scenario ID')
        if name in result or 'actual' not in row:
            raise ValueError('duplicate scenario or missing actual observation')
        result[name] = row
    return result


def grade_observations(rows: list[dict], expected: dict) -> dict:
    """Compare against authored expectations, preserving the complete denominator."""
    observations = _observations(rows)
    missing = sorted(set(expected) - set(observations))
    unexpected = sorted(set(observations) - set(expected))
    contradicted = sorted(name for name in expected.keys() & observations.keys()
                          if observations[name]['actual'] != expected[name])
    passed = len(expected) - len(missing) - len(contradicted)
    return {'expected': len(expected), 'observed': len(observations), 'passed': passed,
            'missing': missing, 'unexpected': unexpected, 'contradicted': contradicted,
            'complete': bool(expected) and not (missing or unexpected or contradicted)}


def observation_sarif(root: Path, rows: list[dict], context: dict) -> dict:
    """Verify UTF-16 spans against pinned UTF-8 files before encoding any result."""
    root = root.resolve(strict=True)
    results = []
    for name, row in sorted(_observations(rows).items()):
        location = row['location']
        relative = location['path']
        if ('\\' in relative or PurePosixPath(relative).is_absolute()
                or any(part in ('', '.', '..') for part in relative.split('/'))):
            raise ValueError('observation path must be confined and repository-relative')
        path = (root / relative).resolve(strict=True)
        if not path.is_relative_to(root):
            raise ValueError('observation path escapes source root')
        raw = path.read_bytes()
        if context.get('files', {}).get(relative) != digest(raw):
            raise ValueError('observation source digest is missing or stale')
        lines = raw.decode('utf-8').splitlines()
        line, column, end = (location[key] for key in ('line', 'column', 'end_column'))
        if any(type(value) is not int for value in (line, column, end)) or not 1 <= line <= len(lines):
            raise ValueError('invalid source line')
        encoded = lines[line - 1].encode('utf-16-le')
        if not 1 <= column < end <= len(encoded) // 2 + 1:
            raise ValueError('invalid UTF-16 source columns')
        excerpt = encoded[(column - 1) * 2:(end - 1) * 2].decode('utf-16-le')
        if excerpt != location.get('excerpt'):
            raise ValueError('observation excerpt differs from the pinned source span')
        results.append({
            'ruleId': 'runtime.observation', 'kind': 'informational', 'level': 'note',
            'message': {'text': f'Runtime observation {name}: ' + json.dumps(row['actual'], sort_keys=True)
                        + '. Applies only to the recorded scenario and configuration.'},
            'locations': [{'physicalLocation': {'artifactLocation': {'uri': relative},
                'region': {'startLine': line, 'startColumn': column, 'endLine': line,
                           'endColumn': end, 'snippet': {'text': excerpt}}}}],
            'partialFingerprints': {'observation/v1': digest(canonical_json([context, row]))},
            'properties': {'scenario': name, 'actual': row['actual'], 'verification': context,
                           'grain': 'scenario/source_span/configuration', 'fixtureEvidence': True},
        })
    return {'$schema': 'https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json',
            'version': '2.1.0', 'runs': [{'tool': {'driver': {'name': PRODUCER, 'version': '1.0.0',
                'rules': [{'id': 'runtime.observation', 'shortDescription': {'text': 'Observed runtime result'},
                           'fullDescription': {'text': 'Presence-only fixture observation; does not establish call-graph completeness, taint or security.'}}]}},
                'columnKind': 'utf16CodeUnits', 'results': results}]}
