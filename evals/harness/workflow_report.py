"""Descriptive operation costs; never infer model usage or population accuracy."""
from __future__ import annotations

import re
import statistics


def distribution(values: list[float | int | None]) -> dict:
    measured = sorted(value for value in values if value is not None)
    return {'observed':len(measured),'unavailable':len(values)-len(measured),
            'minimum':min(measured) if measured else None,
            'median':statistics.median(measured) if measured else None,
            'maximum':max(measured) if measured else None}


def summarize_steps(steps: dict[str, dict]) -> dict:
    """Keep setup and repeated queries separate, retaining all-attempt costs."""
    groups = {}
    for key, row in steps.items():
        parts = key.split('/')
        operation = parts[0] if parts[0] in ('impact','relay') else re.sub(r'-\d+$','',parts[-1])
        if operation == 'retain':
            operation = parts[-2]+'-retain'
        groups.setdefault(operation,[]).append(row)
    result = {}
    for operation, rows in sorted(groups.items()):
        result[operation] = {
            'attempts':len(rows),'complete':sum(row['status']=='complete' for row in rows),
            'wall_seconds':distribution([row['wall_seconds'] for row in rows]),
            'sampled_peak_tree_rss_bytes':distribution([row['sampled_peak_tree_rss_bytes'] for row in rows]),
            'sampled_tree_cpu_seconds':distribution([row['sampled_tree_cpu_seconds'] for row in rows]),
            'resource_samples':distribution([row['resource_method']['samples'] for row in rows]),
            'resource_limitations':sorted({reason for row in rows for reason in row['limitations']}),
            'stdout_bytes':distribution([row['output']['stdout']['bytes'] for row in rows]),
            'stdout_characters':distribution([row['output']['stdout']['characters'] for row in rows]),
            'stdout_tokens':distribution([row['output']['stdout']['tokens'] for row in rows]),
        }
    return result


def paired_payloads(steps: dict[str,dict]) -> list[dict]:
    """Pair identical finding selectors by case/repetition, not by sorted position."""
    pairs = []
    for key, compact in sorted(steps.items()):
        if '/find-compact-' not in key:
            continue
        export = steps.get(key.replace('/find-compact-','/find-export-'))
        if export is None:
            continue
        a,b = compact['output']['stdout'],export['output']['stdout']
        pairs.append({'case':key.split('/')[0],'repetition':int(key.rsplit('-',1)[1]),
                      'compact_tokens':a['tokens'],'export_tokens':b['tokens'],
                      'payload_tokens_avoided':None if a['tokens'] is None or b['tokens'] is None else b['tokens']-a['tokens'],
                      'bytes_avoided':b['bytes']-a['bytes'],
                      'characters_avoided':b['characters']-a['characters']})
    return pairs
