import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from evals.harness.agent_trials import TrialTools, attempt, grade_answer, report, select_tasks
from evals.harness.agent_provider import AgentProvider, ProviderTurn, load_provider, sum_usage
from evals.harness.codex_provider import CodexProvider, model_arguments, model_result, reported_usage
from evals.harness.contracts import ContractError
from evals.harness import agent_trials
from evals.harness.history_pilot import write
from evals.harness.revision_campaign import material_digest


def answer(**values):
    return {'operation': 'finish', 'side': 'head', 'query': '', 'path': '', 'offset': 0,
            'complete': True, 'changes': {'added': [], 'deleted': [], 'modified': ['app.py']},
            'call_sites': [], 'evidence': ['head:diff:0'], 'limitations': [], **values}


class AgentTrialTests(unittest.TestCase):
    def test_prepare_freezes_inputs_without_model_calls_and_run_refuses_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pilot, prepared, cli = root / 'pilot', root / 'prepared', root / 'entmoot'
            cli.write_bytes(b'fixture executable, never launched')
            spec = json.loads(Path('evals/experiments/local-trials.json').read_bytes())
            spec.update(repository_order=['r'], task_limit=1, max_attempts=2, python_call_selectors={})
            write(root / 'config.json', spec)
            episode = {'repository': 'r', 'base': 'a', 'head': 'b', 'status': 'passed',
                       'expected': {'added': [], 'deleted': [], 'modified': ['app.py']}}
            write(pilot / 'report.json', {'episodes': [episode], 'implementation_unchanged': True})
            write(pilot / 'inputs.json', {'tools': {'cli': material_digest(cli)},
                'selection': {'configuration': {'repositories': [{'id': 'r', 'extensions': ['.py']}]}}})
            write(pilot / 'views.json', {})
            write(pilot / 'oracle-trees.json', {'r-a': {'app.py': 'old'}, 'r-b': {'app.py': 'new'}})
            for revision in ('a', 'b'):
                path = pilot / 'publications/sources' / ('r-' + revision) / 'app.py'
                path.parent.mkdir(parents=True)
                path.write_text('value = ' + repr(revision))
            provider = Mock(spec=AgentProvider)
            provider.identity.return_value = {'kind': 'fixture'}
            provider.respond.side_effect = AssertionError('preparation must not call a model')
            def git_command(arguments, cwd, commands, name, environment, timeout):
                output = root / 'git-output'
                output.write_text(arguments[2][-1] if arguments[3] == 'rev-parse' else '')
                return {'status': 'complete', 'stdout': str(output)}
            with patch.object(agent_trials, 'load_provider', return_value=provider), \
                    patch.object(agent_trials, 'command', side_effect=git_command), \
                    patch('tiktoken.get_encoding', return_value=None):
                result = agent_trials.prepare(root / 'config.json', pilot, prepared, cli)
                self.assertEqual(result['status'], 'prepared')
                self.assertEqual(result['planned_attempts'], 2)
                oracle_bytes = (prepared / 'oracles.json').read_bytes()
                self.assertEqual(len(json.loads(oracle_bytes)), 1)
                provider.respond.assert_not_called()
                self.assertFalse((prepared / 'attempts.json').exists())
                source = pilot / 'publications/sources/r-b/app.py'
                source.write_text('changed while paused')
                with self.assertRaisesRegex(ContractError, 'material changed'):
                    agent_trials.run(pilot, prepared, cli)
                provider.respond.assert_not_called()
                source.write_text("value = 'b'")
                (prepared / 'oracles.json').write_text('[]')
                with self.assertRaisesRegex(ContractError, 'oracle'):
                    agent_trials.run(pilot, prepared, cli)
                provider.respond.assert_not_called()
                (prepared / 'oracles.json').write_bytes(oracle_bytes)
                provider.identity.return_value = {'kind': 'changed fixture'}
                with self.assertRaisesRegex(ContractError, 'provider identity'):
                    agent_trials.run(pilot, prepared, cli)
                provider.respond.assert_not_called()
                provider.identity.return_value = {'kind': 'fixture'}
                provider.respond.side_effect = None
                provider.respond.return_value = ProviderTurn('complete', answer(evidence=[]),
                    {'input_tokens': 1, 'output_tokens': 1}, {'status': 'complete'})
                executed = agent_trials.run(pilot, prepared, cli)
                self.assertEqual(executed['executed_attempts'], 2)
                self.assertEqual(provider.respond.call_count, 2)
                with self.assertRaisesRegex(ContractError, 'already started'):
                    agent_trials.run(pilot, prepared, cli)
                self.assertEqual(provider.respond.call_count, 2)

    def test_usage_preserves_subsets_and_missing_counters(self):
        usage = {'input_tokens': 100, 'cached_input_tokens': 40, 'output_tokens': 20, 'reasoning_output_tokens': 10}
        raw = '\n'.join(json.dumps(event) for event in [
            {'type': 'item.completed', 'item': {'type': 'agent_message', 'text': json.dumps(answer())}},
            {'type': 'turn.completed', 'usage': usage}]).encode()
        result, measured = model_result(raw), reported_usage(raw)
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

    def test_trial_loop_accepts_a_non_codex_provider_without_exposing_the_oracle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'publications/sources/r-b'
            source.mkdir(parents=True)
            (source / 'app.py').write_text('def handler():\n    get_connection()\n')
            expected = {'side': 'head', 'path': 'app.py', 'line': 2,
                        'enclosing_scope': 'handler', 'callee_spelling': 'get_connection'}
            actions = [answer(operation='read', path='app.py'),
                       answer(changes={'added': [], 'deleted': [], 'modified': []},
                              call_sites=[expected], evidence=['head:read:0'])]

            class FixtureProvider:
                def identity(self):
                    return {'kind': 'authored_test_fixture'}

                def prepare(self, directory):
                    pass

                def verify_unchanged(self):
                    pass

                def respond(self, prompt, schema, directory, timeout_seconds, tokenizer):
                    context = json.loads(prompt.read_bytes())
                    self_test.assertNotIn('oracle_materials', context['question'])
                    self_test.assertNotIn('expected_calls', context['question'])
                    self_test.assertGreater(timeout_seconds, 0)
                    return ProviderTurn('complete', actions.pop(0),
                        {'input_tokens': 100, 'output_tokens': 20}, {'status': 'complete'})

            self_test = self
            task = {'id': 'r-b', 'repository': 'r', 'base': 'a', 'head': 'b', 'kind': 'python_calls',
                    'selector': 'get_connection', 'extensions': ['.py'], 'expected_calls': [expected]}
            spec = {'max_turns': 3, 'attempt_seconds': 120, 'max_context_characters': 80000,
                    'tool_output_characters': 16000, 'max_reported_tokens': 120000}
            result = attempt(task, {}, root, root / 'attempt', Path('unused'), FixtureProvider(),
                             'baseline', spec, None, repetition=2)
            self.assertTrue(result['grade']['passed'])
            self.assertEqual(result['usage']['input_tokens'], 200)
            self.assertIsNone(result['usage']['reasoning_output_tokens'])
            self.assertEqual(json.loads((root / 'attempt/attempt.json').read_bytes())['repetition'], 2)

    def test_unknown_provider_never_falls_back_to_codex(self):
        with self.assertRaisesRegex(ContractError, 'unsupported agent provider'):
            load_provider({'kind': 'unqualified_provider'})

    def test_partial_usage_keeps_known_tokens_without_counting_a_fully_measured_call(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'publications/sources/r-b'
            source.mkdir(parents=True)
            (source / 'app.py').write_text('value = 1\n')
            provider = Mock(spec=AgentProvider)
            provider.respond.side_effect = [
                ProviderTurn('complete', answer(operation='read', path='app.py'),
                             {'input_tokens': 10, 'output_tokens': 5}, {'status': 'complete'}),
                ProviderTurn('invalid_response', None, {'input_tokens': 20, 'output_tokens': None}, {'status': 'complete'}),
            ]
            task = {'id': 'r-b', 'repository': 'r', 'base': 'a', 'head': 'b', 'kind': 'file_delta',
                    'extensions': ['.py'], 'expected': {'added': [], 'deleted': [], 'modified': ['app.py']}}
            spec = {'max_turns': 2, 'attempt_seconds': 120, 'max_context_characters': 80000,
                    'tool_output_characters': 16000, 'max_reported_tokens': 120000}
            result = attempt(task, {}, root, root / 'attempt', Path('unused'), provider, 'baseline', spec, None)
            self.assertEqual(result['known_usage_subtotal']['input_tokens'], 30)
            self.assertEqual(result['known_usage_subtotal']['output_tokens'], 5)
            self.assertIsNone(result['usage']['output_tokens'])
            self.assertFalse(result['usage_complete'])
            self.assertEqual(result['measured_usage_calls'], 1)
            self.assertEqual(result['model_calls'][1]['reported_usage'], {'input_tokens': 20, 'output_tokens': None})
            summary = report([result], 2)['arms']['baseline']
            self.assertEqual(summary['unmeasured_usage_calls'], 1)
            self.assertEqual(summary['known_usage_subtotal']['output_tokens'], 5)

    def test_provider_retains_reported_usage_after_a_transport_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable, stdout = root / 'provider.exe', root / 'stdout'
            executable.write_bytes(b'fixture executable, never launched')
            stdout.write_bytes(b'{"type":"turn.completed","usage":{"input_tokens":51,"output_tokens":7}}\n')
            provider = CodexProvider({'kind': 'codex_cli', 'model': 'fixture', 'reasoning_effort': 'high'}, executable)
            measured = {'status': 'failed', 'stdout': str(stdout), 'launch_error': None}
            with patch('evals.harness.codex_provider.command', return_value=measured):
                result = provider.respond(root / 'prompt.json', root / 'schema.json', root, 1, None)
            self.assertEqual(result.status, 'failed')
            self.assertEqual(result.usage['input_tokens'], 51)
            self.assertIsNone(result.action)

    def test_known_partial_usage_still_enforces_the_token_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            provider = Mock(spec=AgentProvider)
            provider.respond.return_value = ProviderTurn('complete', answer(operation='read', path='app.py'),
                {'input_tokens': 15, 'output_tokens': None}, {'status': 'complete'})
            task = {'id': 'r-b', 'repository': 'r', 'base': 'a', 'head': 'b', 'kind': 'file_delta',
                    'extensions': ['.py'], 'expected': {'added': [], 'deleted': [], 'modified': []}}
            spec = {'max_turns': 2, 'attempt_seconds': 120, 'max_context_characters': 80000,
                    'tool_output_characters': 16000, 'max_reported_tokens': 10}
            result = attempt(task, {}, root, root / 'attempt', Path('unused'), provider, 'baseline', spec, None)
            self.assertEqual(result['stop'], 'reported_token_budget')
            self.assertEqual(provider.respond.call_count, 1)
            self.assertFalse(result['tools'])

    def test_non_object_provider_events_and_usage_fail_explicitly(self):
        for raw in (b'[]\n', b'{"type":"turn.completed","usage":null}\n'):
            with self.subTest(raw=raw), self.assertRaises(ContractError):
                reported_usage(raw)


if __name__ == '__main__':
    unittest.main()
