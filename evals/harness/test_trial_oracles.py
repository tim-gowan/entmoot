from pathlib import Path
import tempfile
import unittest

from evals.harness.trial_oracles import grade_calls, python_calls


class SourceOracleTests(unittest.TestCase):
    def test_calls_are_distinct_from_definitions_and_attribute_reads(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'adapter.py').write_text(
                'class Adapter:\n'
                '    def get_connection(self):\n'
                '        return self.pool.get_connection()\n'
                '    def send(self):\n'
                '        getter = self.get_connection\n'
                '        return self.get_connection_with_tls_context()\n', encoding='utf-8')
            actual, materials = python_calls(root, ['adapter.py'], 'get_connection', 'head')
            self.assertEqual(actual, [
                {'side': 'head', 'path': 'adapter.py', 'line': 3, 'enclosing_scope': 'Adapter.get_connection', 'callee_spelling': 'get_connection'},
                {'side': 'head', 'path': 'adapter.py', 'line': 6, 'enclosing_scope': 'Adapter.send', 'callee_spelling': 'get_connection_with_tls_context'},
            ])
            self.assertEqual(list(materials), ['adapter.py'])

    def test_exact_calls_require_read_source_witnesses_not_only_an_id_or_guess(self):
        call = {'side': 'head', 'path': 'a.py', 'line': 9, 'enclosing_scope': 'send', 'callee_spelling': 'get_connection'}
        response = {'call_sites': [call], 'complete': True, 'evidence': ['r'],
                    'changes': {'added': [], 'deleted': [], 'modified': []}}
        self.assertFalse(grade_calls([call], response, {})['passed'])
        reference = {'r': {'operation': 'read', 'side': 'head', 'path': 'a.py', 'first_line': 1, 'last_line': 20}}
        self.assertTrue(grade_calls([call], response, reference)['passed'])
        response['call_sites'][0]['line'] = 10
        self.assertFalse(grade_calls([dict(call, line=9)], response, reference)['passed'])


if __name__ == '__main__':
    unittest.main()
