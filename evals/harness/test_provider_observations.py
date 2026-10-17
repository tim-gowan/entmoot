"""Runtime evidence has a scenario and exact source grain, never a graph oracle."""
import copy
from pathlib import Path
import tempfile
import unittest

from evals.harness.provider_observations import observation_sarif, grade_observations
from evals.harness.revision_campaign import digest


class ObservationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = '/*🙂*/ invoke();\n'.encode()
        (self.root / 'app.js').write_bytes(self.source)
        self.row = {'id':'control', 'location':{'path':'app.js','line':1,'column':8,
                    'end_column':14,'excerpt':'invoke'}, 'actual':{'returned':False}}
        self.context = {'files':{'app.js':digest(self.source)},'execution':'authored-runtime-control'}

    def test_utf16_location_is_verified_and_does_not_invent_a_trace(self):
        sarif = observation_sarif(self.root, [self.row], self.context)
        result = sarif['runs'][0]['results'][0]
        self.assertEqual(result['locations'][0]['physicalLocation']['region']['startColumn'], 8)
        self.assertEqual(result['properties']['actual'], {'returned':False})
        self.assertEqual(result['properties']['scenario'], 'control')
        self.assertNotIn('codeFlows', result)
        self.assertNotIn('fixes', result)
        self.assertEqual(sarif['runs'][0]['columnKind'], 'utf16CodeUnits')

    def test_stale_source_escaping_path_and_duplicate_scenarios_are_rejected(self):
        with self.assertRaises(ValueError):
            observation_sarif(self.root, [self.row,self.row], self.context)
        row = copy.deepcopy(self.row)
        row['location']['path'] = '../app.js'
        with self.assertRaises(ValueError):
            observation_sarif(self.root, [row], self.context)
        (self.root / 'app.js').write_text('changed\n')
        with self.assertRaises(ValueError):
            observation_sarif(self.root, [self.row], self.context)

    def test_missing_extra_and_wrong_observations_remain_in_the_grade(self):
        rows=[self.row, {'id':'unexpected','actual':1}]
        grade=grade_observations(rows, {'control':{'returned':True},'missing':0})
        self.assertEqual(grade['expected'], 2)
        self.assertEqual(grade['passed'], 0)
        self.assertEqual(grade['contradicted'], ['control'])
        self.assertEqual(grade['missing'], ['missing'])
        self.assertEqual(grade['unexpected'], ['unexpected'])
        self.assertFalse(grade['complete'])
        self.assertFalse(grade_observations([], {})['complete'])


if __name__ == '__main__':
    unittest.main()
