import unittest

from evals.harness.workflow_report import distribution, paired_payloads
from evals.harness.benchmark_workflow import finding_ids
from evals.harness.contracts import ContractError


class ReportingTests(unittest.TestCase):
    def test_tier_comparison_reads_actual_records_and_rejects_unconsumed_pages(self):
        compact={'tier':'compact','items':[{'id':'a'}],'page':{}}
        export={'tier':'export','result':{'items':[{'finding':{'observation_id':'a'}}], 'bounds':{}}}
        self.assertEqual(finding_ids(compact),finding_ids(export))
        compact['page']['next_cursor']={'offset':1}
        with self.assertRaises(ContractError):
            finding_ids(compact)
        export['result']['bounds']['truncated']=True
        with self.assertRaises(ContractError):
            finding_ids(export)

    def test_unknown_cost_is_not_a_zero_and_small_samples_use_descriptive_spread(self):
        self.assertEqual(distribution([3, None, 1]),
                         {'observed':2,'unavailable':1,'minimum':1,'median':2,'maximum':3})
        self.assertIsNone(distribution([None])['median'])

    def test_payload_pairs_require_the_same_case_and_repetition(self):
        def row(tokens, size):
            return {'output':{'stdout':{'tokens':tokens,'bytes':size,'characters':size}}}
        pairs = paired_payloads({'a/find-compact-0':row(10,20), 'a/find-export-0':row(8,18),
                                 'b/find-compact-0':row(None,20),'c/find-export-0':row(10,30),
                                 'b/find-export-1':row(20,40)})
        self.assertEqual(len(pairs),1)
        self.assertEqual(pairs[0]['case'],'a')
        self.assertEqual(pairs[0]['payload_tokens_avoided'],-2)


if __name__ == '__main__':
    unittest.main()
