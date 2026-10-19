"""Independent process controls; run through the campaign Cargo test owner."""
from pathlib import Path
import json
import os
import sys
import tempfile
import unittest
import psutil

from evals.harness.measurement import command, output_measurement


class CommandMeasurementTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def run_python(self, source, *, timeout=5, name='control'):
        return command([sys.executable, '-X', 'utf8', '-c', source],
                       self.root, self.root, name, os.environ.copy(), timeout)

    def test_tokenizer_receives_exact_decoded_newlines_and_over_budget_stays_unknown(self):
        class CharacterTokenizer:
            name='character-control'
            def encode(self, text, **_):
                return list(text)
        output=self.root/'windows-output'
        output.write_bytes(b'A\r\nB')
        result=output_measurement(output,tokenizer=CharacterTokenizer())
        self.assertEqual(result['characters'],4)
        self.assertEqual(result['tokens'],4)
        bounded=output_measurement(output,tokenizer=CharacterTokenizer(),token_byte_limit=3)
        self.assertIsNone(bounded['tokens'])
        self.assertEqual(bounded['token_status'],'byte_budget_exceeded')

    def test_utf8_bytes_characters_and_failed_attempts_are_preserved(self):
        first = self.run_python("import sys; sys.stdout.buffer.write('Aé🙂'.encode()); sys.stderr.write('failure'); sys.exit(7)")
        second = self.run_python("print('recovered', end='')")
        self.assertEqual((first['status'], first['exit_code']), ('failed', 7))
        self.assertEqual(first['output']['stdout']['bytes'], 7)
        self.assertEqual(first['output']['stdout']['characters'], 3)
        self.assertIsNone(first['output']['stdout']['tokens'])
        self.assertEqual(first['output']['stdout']['token_status'], 'not_requested')
        self.assertEqual(Path(first['stdout']).read_bytes(), b'A\xc3\xa9\xf0\x9f\x99\x82')
        self.assertEqual(Path(first['stderr']).read_text(), 'failure')
        self.assertEqual(Path(second['stdout']).read_text(), 'recovered')
        self.assertNotEqual(first['stdout'], second['stdout'])
        self.assertEqual(json.loads((Path(first['stdout']).parent / 'process.json').read_bytes()), first)

    def test_missing_executable_is_unavailable_not_zero_resource_use(self):
        result = command([str(self.root / 'absent-tool')], self.root, self.root,
                         'missing', os.environ.copy(), 5)
        self.assertEqual(result['status'], 'tool_unavailable')
        self.assertIsNone(result['exit_code'])
        self.assertIsNone(result['sampled_peak_tree_rss_bytes'])
        self.assertIsNone(result['sampled_tree_cpu_seconds'])

    def test_explicit_stdin_is_measured_without_putting_prompt_in_arguments(self):
        prompt = self.root / 'prompt.txt'
        prompt.write_bytes(b'question\r\nwith context')
        result = command([sys.executable, '-c', 'import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())'],
                         self.root, self.root, 'stdin', os.environ.copy(), 5, stdin_path=prompt)
        self.assertEqual(Path(result['stdout']).read_bytes(), prompt.read_bytes())
        self.assertEqual(result['input']['content_digest'], result['output']['stdout']['content_digest'])
        self.assertNotIn('question', ' '.join(result['arguments']))

    def test_timeout_is_retained_and_sampled_memory_is_explicit(self):
        result = self.run_python("import time; data=bytearray(2_000_000); print('started', flush=True); time.sleep(30)", timeout=0.5)
        self.assertEqual(result['status'], 'timeout')
        self.assertIn('started', Path(result['stdout']).read_text())
        self.assertGreater(result['sampled_peak_tree_rss_bytes'], 0)
        self.assertGreaterEqual(result['sampled_tree_cpu_seconds'], 0)
        self.assertEqual(result['resource_method']['sampling_interval_seconds'], 0.05)
        self.assertIn('short_lived_children_may_be_unobserved', result['limitations'])

    def test_timeout_requires_positive_finite_budget(self):
        for timeout in (0, -1, float('nan'), float('inf')):
            with self.subTest(timeout=timeout), self.assertRaises(ValueError):
                self.run_python('pass', timeout=timeout)

    def test_exit_and_timeout_do_not_wait_for_the_next_resource_sample(self):
        for source,timeout,status in [('import time; time.sleep(0.05)',None,'complete'),
                                      ('import time; time.sleep(30)',0.2,'timeout')]:
            with self.subTest(status=status):
                result=command([sys.executable,'-c',source],self.root,self.root,'sparse-sampling',
                               os.environ.copy(),timeout,sampling_interval=10)
                self.assertEqual(result['status'],status)
                self.assertLess(result['wall_seconds'],3)

    def test_timeout_stops_an_observed_child_as_well_as_its_parent(self):
        result = self.run_python(
            "import subprocess,sys,time; from pathlib import Path; "
            "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); "
            "Path('child.pid').write_text(str(child.pid)); time.sleep(30)", timeout=1)
        self.assertEqual(result['status'],'timeout')
        self.assertGreaterEqual(result['resource_method']['observed_processes'],2)
        pid=int((self.root/'child.pid').read_text())
        try:
            child=psutil.Process(pid)
            child.wait(timeout=3)
        except psutil.NoSuchProcess:
            pass
        except psutil.TimeoutExpired:
            self.assertEqual(child.status(),psutil.STATUS_ZOMBIE)


if __name__ == '__main__':
    unittest.main()
