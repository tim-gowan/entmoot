import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import blake3

from evals.harness.contracts import ContractError
from evals.harness.history_pilot import blob_digests, grade_delta, grade_episode, grade_inventory, select, source_delta


class HistorySamplingTests(unittest.TestCase):
    def test_sampling_is_order_independent_disjoint_and_does_not_fill_missing_strata(self):
        frame = [{'repository': 'r', 'head': str(n), 'stratum': 'ordinary'} for n in range(12)]
        quotas = {'failed_ci_candidate': 4, 'ordinary': 4, 'source_neutral': 2}
        selected, counts = select(frame, quotas, 29)
        self.assertEqual(select(list(reversed(frame)), quotas, 29), (selected, counts))
        self.assertEqual(len(selected), 4)
        self.assertEqual(len({row['head'] for row in selected}), 4)
        self.assertEqual(counts['failed_ci_candidate']['underfilled'], 4)
        self.assertEqual(counts['source_neutral']['eligible'], 0)
        with self.assertRaises(ContractError):
            select(frame + [frame[0]], quotas, 29)

    def test_delta_distinguishes_rename_content_and_missing_evidence(self):
        expected = {'added': ['new.py'], 'deleted': ['old.py'], 'modified': ['body.py']}
        actual = source_delta({'old.py': 'a', 'body.py': 'b', 'stable.py': 'c'},
                              {'new.py': 'a', 'body.py': 'd', 'stable.py': 'c'})
        self.assertEqual(actual, expected)
        self.assertTrue(grade_delta(expected, actual)['passed'])
        self.assertFalse(grade_delta(expected, {'added': [], 'deleted': [], 'modified': ['body.py']})['passed'])

    def test_empty_deltas_cannot_hide_missing_unchanged_files_or_wrong_source_bytes(self):
        self.assertFalse(grade_inventory({'stable.py': 'correct'}, {})['passed'])
        grade = grade_inventory({'stable.py': 'correct'}, {'stable.py': 'wrong'})
        self.assertFalse(grade['passed'])
        self.assertEqual(grade['different'], ['stable.py'])
        self.assertTrue(grade_inventory({'stable.py': 'correct'}, {'stable.py': 'correct'})['passed'])

    def test_git_blob_oracle_uses_byte_lengths_and_rejects_incomplete_streams(self):
        content = b'first\r\n\x00last\n'
        raw = b'a blob ' + str(len(content)).encode() + b'\n' + content + b'\nb blob 0\n\n'
        expected = {'a': 'blake3:' + blake3.blake3(content).hexdigest(),
                    'b': 'blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262'}
        self.assertEqual(blob_digests(raw), expected)
        for malformed in (raw[:-1], b'a tree 0\n\n', b'a blob 999\nx\n'):
            with self.subTest(raw=malformed), self.assertRaises(ContractError):
                blob_digests(malformed)

    def test_missing_byte_oracle_is_unavailable_not_a_product_failure(self):
        episode = {'repository': 'r', 'base': 'a', 'head': 'b'}
        files = {'r-a': {'a.py': 'same'}, 'r-b': {'a.py': 'same'}}
        inventory = {'r-a': {'passed': True}}
        self.assertEqual(grade_episode(episode, files, files, inventory)['status'], 'unavailable')
        inventory['r-b'] = {'passed': False}
        self.assertEqual(grade_episode(episode, files, files, inventory)['status'], 'failed')
        inventory['r-b'] = {'passed': True}
        self.assertEqual(grade_episode(episode, files, files, inventory)['status'], 'passed')

    def test_checkout_ignores_host_eol_defaults_and_preserves_authored_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source, destination = root / 'source', root / 'checkout'
            source.mkdir()
            config = root / 'host.gitconfig'
            config.write_text('[core]\n autocrlf = true\n eol = crlf\n hooksPath = empty-hooks\n')
            environment = {**os.environ, 'GIT_CONFIG_GLOBAL': str(config), 'GIT_CONFIG_NOSYSTEM': '1'}
            def git(*args):
                return subprocess.check_output(['git', '-C', str(source), '-c', 'core.autocrlf=false',
                    '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', *args], env=environment)
            git('init', '--quiet')
            (source / '.gitattributes').write_bytes(b'raw.py -text\n')
            (source / 'raw.py').write_bytes(b'authored = 1\r\n')
            (source / 'source.py').write_bytes(b'first = 1\nsecond = 2\n')
            git('add', '.')
            git('commit', '--quiet', '--no-gpg-sign', '-m', 'source')
            spec = root / 'spec.json'
            spec.write_text(json.dumps({'url': str(source), 'revision': git('rev-parse', 'HEAD').decode().strip()}))
            invocation = [sys.executable, '-c',
                'import json,sys; from pathlib import Path; from evals.harness.repository_campaign import checkout; '
                'checkout(json.loads(Path(sys.argv[1]).read_bytes()), Path(sys.argv[2]))', str(spec), str(destination)]
            result = subprocess.run(invocation, env=environment, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
            self.assertEqual((destination / 'source.py').read_bytes(), b'first = 1\nsecond = 2\n')
            self.assertEqual((destination / 'raw.py').read_bytes(), b'authored = 1\r\n')
            result = subprocess.run(invocation, env=environment, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
            self.assertEqual((destination / 'source.py').read_bytes(), b'first = 1\nsecond = 2\n')
            subprocess.run(['git', '-C', str(destination), 'config', 'core.autocrlf', 'true'],
                           env=environment, check=True)
            result = subprocess.run(invocation, env=environment, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('use a fresh campaign directory', result.stderr.decode())
            self.assertEqual((destination / 'source.py').read_bytes(), b'first = 1\nsecond = 2\n')


if __name__ == '__main__':
    unittest.main()
