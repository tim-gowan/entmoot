"""Summarize retained pilot/trial receipts without executing tools or models."""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path

from evals.harness.contracts import ContractError
from evals.harness.history_pilot import write
from evals.harness.revision_campaign import material_digest
from evals.harness.workflow_report import distribution


def phase(arguments: list[str]) -> str:
    executable = Path(arguments[0]).stem.lower()
    if executable == 'entmoot':
        return 'init' if 'init' in arguments else 'product_read'
    if executable == 'retain_campaign_publication':
        return 'retain'
    if executable == 'gh':
        return 'ci_metadata'
    if 'cat-file' in arguments:
        return 'oracle_source_bytes'
    return 'git_acquisition_metadata'


def measurements(paths: list[Path]) -> dict:
    groups = defaultdict(list)
    for path in paths:
        row = json.loads(path.read_bytes())
        groups[phase(row['arguments'])].append(row)
    result = {}
    for name, rows in sorted(groups.items()):
        result[name] = {'attempts': len(rows), 'statuses': dict(Counter(row['status'] for row in rows)),
                        'sum_command_wall_seconds': sum(row['wall_seconds'] for row in rows)}
        for field in ('wall_seconds', 'sampled_peak_tree_rss_bytes', 'sampled_tree_cpu_seconds',
                      'sampled_tree_read_bytes', 'sampled_tree_write_bytes'):
            result[name][field] = distribution([row.get(field) for row in rows])
        result[name]['resource_limitations'] = sorted({reason for row in rows for reason in row['limitations']})
    return result


def trial_operations(attempts: list[dict]) -> dict:
    """Keep delivered context separate from full command output and model usage."""
    groups = defaultdict(list)
    for attempt in attempts:
        for call in attempt['tools']:
            groups[attempt['arm'] + ':' + call['operation']].append(call)
    result = {}
    for key, calls in sorted(groups.items()):
        result[key] = {'calls': len(calls),
            'wall_seconds': distribution([call['wall_seconds'] for call in calls]),
            'process_statuses': dict(Counter(call.get('process', {}).get('status', 'in_process') for call in calls))}
        for field in ('bytes', 'characters', 'tokens'):
            result[key]['delivered_' + field] = distribution([call['delivered'][field] for call in calls])
            result[key]['raw_stdout_' + field] = distribution([
                call.get('process', {}).get('output', {}).get('stdout', {}).get(field) for call in calls])
        for field in ('sampled_peak_tree_rss_bytes', 'sampled_tree_cpu_seconds',
                      'sampled_tree_read_bytes', 'sampled_tree_write_bytes'):
            result[key][field] = distribution([call.get('process', {}).get(field) for call in calls])
    return result


def summarize(pilot: Path, trials: Path | None) -> dict:
    study = json.loads((pilot / 'report.json').read_bytes())
    inputs = json.loads((pilot / 'inputs.json').read_bytes())
    views = json.loads((pilot / 'views.json').read_bytes())
    publications = json.loads((pilot / 'repositories-summary.json').read_bytes())['repositories']
    selection = inputs['selection']
    if not study['implementation_unchanged']:
        raise ContractError('pilot implementation changed while running')
    processes = sorted([*(pilot / 'commands').rglob('process.json'),
                        *(pilot / 'publications' / 'logs').rglob('process.json')])
    repositories = {}
    for row in selection['repositories']:
        cases = [ep for ep in study['episodes'] if ep['repository'] == row['id']]
        initial = [item for item in publications if item['id'].rsplit('-', 1)[0] == row['id']]
        repositories[row['id']] = {'selection_status': row['status'], 'strata': row.get('counts'),
            'frame_capped': row.get('frame_capped'), 'ci_status': row.get('ci_status'),
            'ci_unmatched_heads': row.get('ci_unmatched_heads'),
            'ci_retrieved_runs': sum(page['retrieved'] for page in row.get('ci_pages', [])),
            'ci_reported_total': max((page['total_count'] for page in row.get('ci_pages', [])), default=None),
            'grades': dict(Counter(case['status'] for case in cases)),
            'initial_publication_database_bytes': distribution([item.get('database_bytes') for item in initial])}
    result = {'scope': 'bounded history source inventory and local agent calibration',
              'pilot_report_digest': material_digest(pilot / 'report.json'),
              'pilot_inputs_digest': material_digest(pilot / 'inputs.json'),
              'sample': {key: study[key] for key in ('requested_episodes', 'selected_episodes', 'retained_views', 'counts')},
              'frame_candidates': len(selection['frame']),
              'selected_strata': dict(Counter(row['stratum'] for row in selection['episodes'])),
              'repositories': repositories, 'production_database_bytes': study['database_bytes'],
              'source_file_memberships': sum(view['inventory_grade']['expected_files'] for view in views.values()),
              'inventory_grades': dict(Counter('passed' if view['inventory_grade']['passed'] else 'failed' for view in views.values())),
              'operation_measurements': measurements(processes), 'trials': None,
              'limitations': study['limitations'] + [
                  'First-parent/date and metadata caps define the sampling frame; underfilled strata are retained.',
                  'Ordinary history is not verified passing CI; missing CI metadata is not absence of failures.',
                  'File membership observations and repeated revisions are not independent repository trials.',
                  'Resource counters cover instrumented child processes, not exact whole-study CPU or memory.',
                  'Checkout inside repository_campaign and coordinator overhead lack separate process receipts.',
                  'Database bytes are logical main/WAL/SHM bytes, not allocated storage or the entire acquisition directory.',
                  'Index runs use local debug binaries and warm/unspecified OS caches; no cold-cache or release-speed claim.',
                  'Host load is uncontrolled; measured local processes do not include remote model-server resources.',
                  'No time-to-first-correct-intermediate-signal metric or billed dollar cost is established.']}
    if trials:
        trial_inputs = json.loads((trials / 'inputs.json').read_bytes())
        if trial_inputs['pilot_report_digest'] != result['pilot_report_digest']:
            raise ContractError('trial receipts refer to another pilot')
        trial_report = json.loads((trials / 'report.json').read_bytes())
        result['trials'] = trial_report
        result['trial_configuration'] = trial_inputs['configuration']
        result['trial_report_digest'] = material_digest(trials / 'report.json')
        result['trial_operations'] = trial_operations(json.loads((trials / 'attempts.json').read_bytes()))
        pairs = trial_report['pairs']
        result['paired_differences'] = {
            cohort: {field: distribution([pair[field] for pair in selected])
                     for field in ('wall_seconds_difference', 'input_tokens_difference', 'total_tokens_difference')}
            for cohort, selected in [('all_pairs', pairs), ('both_answers_passed', [p for p in pairs if p['both_passed']])]}
    return result


def export_bundle(pilot: Path, trials: Path | None, destination: Path, summary: dict) -> None:
    """Publish bounded replay inputs and grades, without checkouts or model prompts."""
    inputs = json.loads((pilot / 'inputs.json').read_bytes())
    selection = inputs['selection']
    for repository in selection['repositories']:
        repository.pop('mirror', None)
    destination.mkdir(parents=True, exist_ok=False)
    write(destination / 'selection.json', selection)
    write(destination / 'summary.json', summary)
    # Keep case-level failures auditable; aggregate counts alone lose the
    # distinctions that the independent oracle checked. Locators stay local.
    write(destination / 'history/report.json', json.loads((pilot / 'report.json').read_bytes()))
    write(destination / 'history/inventory-grades.json', {
        key: view['inventory_grade'] for key, view in json.loads((pilot / 'views.json').read_bytes()).items()})
    write(destination / 'provenance.json', {
        'original_selection_digest': inputs['selection_digest'],
        'portable_selection_digest': material_digest(destination / 'selection.json'),
        'normalization': 'Remove machine-local mirror paths; preserve the frame, pins, selection and original sampler digest.',
        'tools': inputs['tools'], 'harness_sources': inputs['harness_sources'],
        'evaluator_digest': inputs['evaluator_digest'],
        'omitted': ['source checkouts', 'production databases', 'raw command streams', 'model prompts'],
        'limitations': ['Replay pins rather than reacquiring live CI metadata to reproduce the selected population.',
                        'Model aliases and server configuration are not immutable; repeated trials can differ.']})
    if trials:
        saved = destination / 'trials'
        trial_inputs = json.loads((trials / 'inputs.json').read_bytes())
        write(saved / 'inputs.json', trial_inputs)
        write(saved / 'oracles.json', json.loads((trials / 'oracles.json').read_bytes()))
        attempts = json.loads((trials / 'attempts.json').read_bytes())
        def portable_process(process):
            return {key: value for key, value in process.items()
                    if key not in ('arguments', 'cwd', 'stdout', 'stderr', 'launch_error')}
        for attempt in attempts:
            attempt['model_calls'] = [portable_process(process) for process in attempt['model_calls']]
            for call in attempt['tools']:
                if 'process' in call:
                    call['process'] = portable_process(call['process'])
        write(saved / 'attempts.json', attempts)
        write(saved / 'report.json', json.loads((trials / 'report.json').read_bytes()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pilot', type=Path, required=True)
    parser.add_argument('--trials', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, help='new directory for portable selection and regradable trial receipts')
    args = parser.parse_args()
    result = summarize(args.pilot, args.trials)
    write(args.out, result)
    if args.bundle:
        export_bundle(args.pilot, args.trials, args.bundle, result)
    print(json.dumps(result['sample']))


if __name__ == '__main__':
    main()
