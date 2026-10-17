"""Source fences exercised against real, fixture-owned Git repositories."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from evals.harness.contracts import ContractError
from evals.harness.revision_campaign import digest
from evals.harness.run_revision_campaign import verify_checkout
from evals.harness.workflow_campaign import fixture_checkout, collect_pages, token_summary, preserve_evidence, verify_tool


class PageTests(unittest.TestCase):
    def test_collects_continuation_and_rejects_cycles_or_exhaustion(self):
        requests = []
        def fetch(cursor):
            requests.append(cursor)
            return {'items':[{'id':'first' if cursor is None else 'second'}],
                    'page':{'next_cursor':{'offset':1}} if cursor is None else {}}
        self.assertEqual([item['id'] for item in collect_pages(fetch)], ['first','second'])
        self.assertEqual(requests,[None,{'offset':1}])
        with self.assertRaises(ContractError):
            collect_pages(lambda _: {'items':[], 'page':{'next_cursor':{'offset':1}}})
        with self.assertRaises(ContractError):
            collect_pages(fetch,max_pages=1)

    def test_unmeasured_tokens_never_become_zero(self):
        rows = [{'output':{'stdout':{'tokens':7},'stderr':{'tokens':None}}}]
        self.assertEqual(token_summary(rows),{'total':None,'counted':7,'unmeasured_streams':1})

    def test_provider_attachment_must_preserve_native_provenance(self):
        before = {'resolutions':[{'id':'native','scope':'configuration-a'}]}
        preserve_evidence(before, before)
        with self.assertRaises(ContractError):
            preserve_evidence(before, {'resolutions':[{'id':'native','scope':'configuration-b'}]})


class CheckoutTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        template = self.directory/'template'
        template.mkdir()
        (template/'source.js').write_text('function stable() { return 1; }\n')
        self.root = self.directory/'source'
        fixture_checkout(self.root,template,['source.js'],{'pin':'authored'},os.environ.copy())
        self.revision = subprocess.check_output(['git','-C',str(self.root),'rev-parse','HEAD']).decode().strip()

    @unittest.skipUnless(os.name == 'nt', 'Windows extended-path alias control')
    def test_windows_extended_source_path_is_the_same_checkout(self):
        extended = Path('\\\\?\\'+str(self.root.resolve()))
        selected = verify_checkout(self.root,extended,self.revision)
        self.assertIsInstance(selected, Path)
        self.assertTrue(selected.samefile(self.root))
        self.assertFalse(str(selected).startswith('\\\\?\\'))

    def test_generated_lock_requires_explicit_digest_and_never_waives_tracked_changes(self):
        lock = self.root/'package-lock.json'
        lock.write_bytes(b'{"lockfileVersion":3}\n')
        with self.assertRaises(ContractError):
            verify_checkout(self.root,self.root,self.revision)
        accepted = {'package-lock.json':digest(lock.read_bytes())}
        verify_checkout(self.root,self.root,self.revision,generated_paths=accepted)
        lock.write_bytes(b'{"lockfileVersion":2}\n')
        with self.assertRaises(ContractError):
            verify_checkout(self.root,self.root,self.revision,generated_paths=accepted)
        lock.write_bytes(b'{"lockfileVersion":3}\n')
        (self.root/'source.js').write_text('function stable() { return 2; }\n')
        with self.assertRaises(ContractError):
            verify_checkout(self.root,self.root,self.revision,generated_paths=accepted)

    def test_replacing_a_selected_tool_invalidates_the_experiment(self):
        tool=self.directory/'tool'
        tool.write_bytes(b'first build')
        expected=digest(tool.read_bytes())
        verify_tool(tool,expected)
        tool.write_bytes(b'second build')
        with self.assertRaises(ContractError):
            verify_tool(tool,expected)


if __name__ == '__main__':
    unittest.main()
