import unittest

import blake3

from evals.harness.contracts import ContractError
from evals.harness.history_pilot import blob_digests, grade_delta, grade_inventory, select, source_delta


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


if __name__ == '__main__':
    unittest.main()
