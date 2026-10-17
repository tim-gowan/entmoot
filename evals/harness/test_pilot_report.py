import json
from pathlib import Path
import tempfile
import unittest

from evals.harness.pilot_report import export_bundle, measurements, trial_operations
from evals.harness.history_pilot import write
from evals.harness.revision_campaign import material_digest


class PilotMeasurementTests(unittest.TestCase):
    def test_failed_and_unavailable_measurements_remain_in_the_denominator(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for index, (status, memory, elapsed) in enumerate([('complete', 100, 2), ('timeout', None, 5)]):
                path = Path(directory) / f'{index}.json'
                path.write_text(json.dumps({'arguments': ['entmoot.exe', 'init'], 'status': status,
                    'wall_seconds': elapsed, 'sampled_peak_tree_rss_bytes': memory,
                    'limitations': ['sampled_only']}))
                paths.append(path)
            result = measurements(paths)['init']
            self.assertEqual(result['attempts'], 2)
            self.assertEqual(result['sum_command_wall_seconds'], 7)
            self.assertEqual(result['statuses'], {'complete': 1, 'timeout': 1})
            self.assertEqual(result['sampled_peak_tree_rss_bytes']['unavailable'], 1)
            self.assertIsNone(result['sampled_tree_cpu_seconds']['median'])

    def test_delivered_context_is_not_raw_output_or_model_usage(self):
        call = {'operation': 'read', 'wall_seconds': 0.1,
                'delivered': {'bytes': 100, 'characters': 90, 'tokens': 25}}
        row = trial_operations([{'arm': 'baseline', 'tools': [call]}])['baseline:read']
        self.assertEqual(row['delivered_tokens']['median'], 25)
        self.assertIsNone(row['raw_stdout_tokens']['median'])
        self.assertIsNone(row['sampled_peak_tree_rss_bytes']['median'])

    def test_portable_bundle_keeps_pins_and_oracle_bytes_without_host_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pilot, trials, bundle = (root / name for name in ('pilot', 'trials', 'bundle'))
            write(pilot / 'inputs.json', {'selection': {'repositories': [{'id': 'r', 'mirror': str(root)}],
                'episodes': [{'base': 'a' * 40, 'head': 'b' * 40}]}, 'selection_digest': 'original',
                'tools': {}, 'harness_sources': {}, 'evaluator_digest': 'evaluator'})
            write(pilot / 'report.json', {'episodes': [{'repository': 'r', 'status': 'failed'}],
                                        'implementation_unchanged': True})
            inventory_grade = {'passed': False, 'expected_files': 1, 'missing': ['app.py']}
            write(pilot / 'views.json', {'r-' + 'b' * 40: {'graph': str(root / 'graph.json'),
                'retained': {'locator': str(root / 'view.json')}, 'source_root': str(root),
                'inventory_grade': inventory_grade}})
            write(pilot / 'commands/failed-init/process.json', {
                'arguments': ['entmoot.exe', '--data-dir', str(root), 'init'], 'cwd': str(root),
                'status': 'timeout', 'wall_seconds': 5, 'sampled_peak_tree_rss_bytes': None})
            abandoned = pilot / 'commands/interrupted-retain/stdout'
            abandoned.parent.mkdir(parents=True)
            abandoned.write_text('committed, but the coordinator stopped before its receipt')
            write(pilot / 'before-continuation/views.json', {'r-old': {'inventory_grade': {'passed': False}}})
            write(trials / 'oracles.json', [{'expected': ['app.py']}])
            oracle_digest = material_digest(trials / 'oracles.json')
            write(trials / 'inputs.json', {'oracle_digest': oracle_digest})
            write(trials / 'report.json', {'passed': 1})
            process = {'arguments': ['tool', str(root)], 'cwd': str(root), 'stdout': str(root / 'stdout'),
                       'wall_seconds': 3, 'status': 'complete'}
            write(trials / 'attempts.json', [{'model_calls': [process], 'tools': [{'process': process}]}])
            export_bundle(pilot, trials, bundle, {'sample': 1})
            self.assertEqual(material_digest(bundle / 'trials/oracles.json'), oracle_digest)
            selected = json.loads((bundle / 'selection.json').read_bytes())
            self.assertEqual(selected['episodes'][0]['head'], 'b' * 40)
            self.assertNotIn('mirror', selected['repositories'][0])
            self.assertNotIn(str(root), (bundle / 'trials/attempts.json').read_text())
            self.assertEqual(json.loads((bundle / 'provenance.json').read_bytes())['original_selection_digest'], 'original')
            self.assertEqual(material_digest(bundle / 'history/report.json'), material_digest(pilot / 'report.json'))
            self.assertEqual(json.loads((bundle / 'history/inventory-grades.json').read_bytes()),
                             {'r-' + 'b' * 40: inventory_grade})
            measurements = json.loads((bundle / 'history/processes.json').read_bytes())
            self.assertEqual(measurements, [{'phase': 'init', 'attempt': 'failed-init',
                'measurement': {'status': 'timeout', 'wall_seconds': 5, 'sampled_peak_tree_rss_bytes': None}}])
            self.assertEqual(json.loads((bundle / 'history/unfinalized-processes.json').read_bytes()),
                             ['commands/interrupted-retain'])
            self.assertEqual(json.loads((bundle / 'history/prior-inventory-grades.json').read_bytes()),
                             {'r-old': {'passed': False}})


if __name__ == '__main__':
    unittest.main()
