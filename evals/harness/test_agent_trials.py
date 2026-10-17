import json
from pathlib import Path
import tempfile
import unittest

from evals.harness.agent_trials import (TrialTools, grade_answer, model_arguments,
                                       model_result, report, reported_usage, select_tasks, sum_usage)
from evals.harness.contracts import ContractError


def answer(**values):
    return {'operation': 'finish', 'side': 'head', 'query': '', 'path': '', 'offset': 0,
            'complete': True, 'changes': {'added': [], 'deleted': [], 'modified': ['app.py']},
            'call_sites': [], 'evidence': ['head:diff:0'], 'limitations': [], **values}


class AgentTrialTests(unittest.TestCase):
    def test_usage_preserves_subsets_and_missing_counters(self):
        usage = {'input_tokens': 100, 'cached_input_tokens': 40, 'output_tokens': 20, 'reasoning_output_tokens': 10}
        raw = '\n'.join(json.dumps(event) for event in [
            {'type': 'item.completed', 'item': {'type': 'agent_message', 'text': json.dumps(answer())}},
            {'type': 'turn.completed', 'usage': usage}]).encode()
        result, measured = model_result(raw)
        self.assertEqual(result, answer())
        totals = sum_usage([measured, measured])
        self.assertEqual(totals['input_tokens'] + totals['output_tokens'], 240)
        self.assertEqual(totals['reasoning_output_tokens'], 20)
        self.assertIsNone(totals['cache_write_input_tokens'])
        self.assertIsNone(sum_usage([])['input_tokens'])

    def test_builtin_execution_and_incomplete_model_turn_are_not_valid_trials(self):
        for events in [
            [{'type': 'turn.failed', 'error': {'message': 'budget'}}],
            [{'type': 'item.completed', 'item': {'type': 'command_execution', 'command': 'read oracle'}},
             {'type': 'turn.completed', 'usage': {}}],
            [{'type': 'turn.completed'}, {'type': 'turn.completed'}],
        ]:
            with self.subTest(events=events), self.assertRaises(ContractError):
                model_result('\n'.join(json.dumps(row) for row in events).encode())

    def test_usage_is_recoverable_even_when_the_answer_is_malformed(self):
        raw = b'{"type":"turn.completed","usage":{"input_tokens":51,"output_tokens":7}}\n'
        self.assertEqual(reported_usage(raw)['input_tokens'], 51)
        with self.assertRaises(ContractError):
            model_result(raw)

    def test_grading_requires_the_exact_answer_and_complete_cited_evidence(self):
        expected = {'added': [], 'deleted': [], 'modified': ['app.py']}
        issued = {'head:diff:0': {'operation': 'diff', 'offset': 0, 'end': 10, 'total': 10}}
        self.assertTrue(grade_answer(expected, answer(), issued)['passed'])
        for bad in [answer(complete=False), answer(evidence=['invented']),
                    answer(changes={'added': [], 'deleted': [], 'modified': ['app.py', 'app.py']}),
                    answer(changes={'added': [], 'deleted': [], 'modified': []})]:
            self.assertFalse(grade_answer(expected, bad, issued)['passed'])
        issued['head:diff:0']['end'] = 5
        self.assertFalse(grade_answer(expected, answer(), issued)['passed'])

    def test_task_selection_does_not_hide_product_failures(self):
        rows = [{'repository': repo, 'head': str(n), 'status': status}
                for repo in ('python', 'go') for n, status in enumerate(('failed', 'passed'))]
        tasks = select_tasks({'episodes': rows}, ['python', 'go'], 4, 7)
        self.assertEqual(len(tasks), 4)
        self.assertEqual(sum(row['status'] == 'failed' for row in tasks), 2)
        self.assertEqual([row['repository'] for row in tasks], ['python', 'go', 'python', 'go'])

    def test_model_command_disables_execution_without_copying_credentials(self):
        argv = model_arguments(Path('codex'), Path('schema'), {'model': 'configured-model', 'reasoning_effort': 'high'})
        self.assertIn('read-only', argv)
        self.assertIn('shell_tool', argv)
        self.assertIn('--ignore-user-config', argv)
        self.assertNotIn('--dangerously-bypass-approvals-and-sandbox', argv)

    def test_baseline_cannot_use_entmoot_or_read_the_oracle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'publications' / 'sources' / 'r-b'
            source.mkdir(parents=True)
            (root / 'oracle.json').write_text('secret expected answer')
            tools = TrialTools({'repository': 'r', 'base': 'a', 'head': 'b'}, {}, root, root,
                               Path('unused'), 'baseline', {'tool_output_characters': 100}, None)
            for request in [answer(operation='find'), answer(operation='read', path='../../../oracle.json')]:
                self.assertTrue(tools.execute(request)['unknown'])
            self.assertFalse(tools.issued)

    def test_a_truncated_source_line_is_not_a_complete_citation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'publications' / 'sources' / 'r-b'
            source.mkdir(parents=True)
            (source / 'app.py').write_text('x = ' + 'a' * 1000)
            tools = TrialTools({'repository': 'r', 'base': 'a', 'head': 'b'}, {}, root, root,
                               Path('unused'), 'baseline', {'tool_output_characters': 20}, None)
            observation = tools.execute(answer(operation='read', path='app.py'))
            self.assertTrue(observation['truncated'])
            self.assertEqual(tools.issued[observation['reference']]['last_line'], 0)

    def test_paired_report_keeps_failed_attempts_and_unknown_costs(self):
        base = {'task': 'r-a', 'repository': 'r', 'task_kind': 'file_delta', 'repetition': 0,
                'arm': 'baseline', 'grade': {'passed': True}, 'stop': 'finished', 'wall_seconds': 2,
                'usage': {'input_tokens': 100, 'output_tokens': 20, 'reasoning_output_tokens': 5},
                'usage_complete': True, 'model_calls': [{}], 'measured_usage_calls': 1, 'tools': []}
        timed_out = {**base, 'arm': 'entmoot', 'grade': {'passed': False}, 'stop': 'model_timeout',
                     'wall_seconds': 120, 'usage_complete': False, 'model_calls': [{}, {}]}
        result = report([base, timed_out], 4)
        self.assertEqual(result['unexecuted_attempts'], 2)
        self.assertFalse(result['pairs'][0]['both_passed'])
        self.assertIsNone(result['pairs'][0]['total_tokens_difference'])
        self.assertEqual(result['arms']['entmoot']['known_usage_subtotal']['input_tokens'], 100)
        self.assertEqual(result['arms']['entmoot']['unmeasured_usage_calls'], 1)
        with self.assertRaises(ContractError):
            report([base, base], 4)


if __name__ == '__main__':
    unittest.main()
