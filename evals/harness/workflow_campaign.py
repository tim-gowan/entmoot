"""Execute bounded workflows over pinned publications; grade independent controls.

The repository runner owns checkout/build/publication. This runner owns authored
observer invocation, fixture evidence, operation measurements and grades. Shared
production views are retained through Rust repository APIs, never SQL writes.
"""
from __future__ import annotations

import argparse
from importlib.metadata import version
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys

import jsonschema

from evals.harness.contracts import ContractError
from evals.harness.measurement import command, database_bytes
from evals.harness.provider_observations import PRODUCER, grade_observations, observation_sarif
from evals.harness.repository_campaign import beneath
from evals.harness.revision_campaign import CampaignStore, canonical_json, digest, evaluator_digest, git_revision, material_digest
from evals.harness.run_revision_campaign import run_lock, verify_checkout
from evals.harness.workflow_report import paired_payloads, summarize_steps


def read(path: Path):
    return json.loads(path.read_bytes())


def write(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_json(value))


def collect_pages(fetch, *, max_pages=100) -> list[dict]:
    """Follow bounded compact continuation; exhaustion cannot establish completeness."""
    items, seen, cursor = [], set(), None
    for _ in range(max_pages):
        response = fetch(cursor)
        items.extend(response['items'])
        cursor = response['page'].get('next_cursor')
        if cursor is None:
            return items
        key = canonical_json(cursor)
        if key in seen:
            raise ContractError('query continuation repeated a cursor')
        seen.add(key)
    raise ContractError('query continuation exhausted the campaign page budget')


def token_summary(steps: list[dict]) -> dict:
    values = [step['output'][stream]['tokens'] for step in steps for stream in ('stdout','stderr')]
    counted = sum(value for value in values if value is not None)
    missing = sum(value is None for value in values)
    return {'total':None if missing else counted,'counted':counted,'unmeasured_streams':missing}


def preserve_evidence(before: dict, after: dict) -> None:
    """Provider attachment must retain existing facts, including native provenance."""
    for collection, rows in before.items():
        if isinstance(rows, list) and rows and isinstance(rows[0], dict):
            original = {canonical_json(row) for row in rows}
            retained = {canonical_json(row) for row in after.get(collection, [])}
            if not original <= retained:
                raise ContractError('attachment discarded or changed existing '+collection)


def verify_tool(path: Path, expected: str) -> None:
    if material_digest(path) != expected:
        raise ContractError('selected executable changed during the experiment; use frozen binaries in a fresh run')


def fixture_checkout(root: Path, source: Path, files: list[str], selection: dict, environment: dict) -> None:
    """Author a separate deterministic fixture revision, with no inherited hooks."""
    expected = {name: beneath(source, name).read_bytes() for name in files}
    expected['selection.json'] = canonical_json(selection)
    if root.exists():
        if any(not (root / name).is_file() or (root / name).read_bytes() != raw for name, raw in expected.items()):
            raise ContractError('existing fixture bytes differ; use a fresh workflow run')
        return
    root.mkdir(parents=True)
    for name, raw in expected.items():
        target = beneath(root, name)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(raw)
    env = environment | {'GIT_AUTHOR_DATE':'2026-10-17T00:00:00Z', 'GIT_COMMITTER_DATE':'2026-10-17T00:00:00Z'}
    for args in (['init','--quiet'], ['config','core.autocrlf','false'],
                 ['config','core.hooksPath','empty-hooks'], ['config','user.name','Workflow fixture'],
                 ['config','user.email','fixture@example.invalid'], ['add','--',*expected],
                 ['commit','--no-gpg-sign','--quiet','-m','Pin independent workflow inputs']):
        result = subprocess.run(['git','-C',str(root),*args], env=env, capture_output=True, timeout=30)
        if result.returncode:
            raise ContractError(result.stderr.decode(errors='replace'))


class WorkflowRun:
    def __init__(self, args):
        self.args = args
        self.root = args.run_dir.resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        self.spec = read(args.config)
        self.repositories = read(args.repositories)
        self.fixture = (args.config.resolve().parent / self.spec['fixture']).resolve(strict=True)
        self.expected = read(self.fixture / self.spec['expectations'])['by_major']
        self.environment = os.environ.copy()
        self.tools = {name: path.resolve(strict=True) for name, path in
                      [('cli',args.cli),('node',args.node),('prepare',args.prepare_helper),('retain',args.retain_helper)]}
        self.tools['rg'] = Path(shutil.which('rg') or 'rg').resolve(strict=True)
        if args.native:
            self.tools['producer'] = self.tools['cli'].with_name('entmoot-producer'+('.exe' if os.name=='nt' else ''))
        self.schema = read(args.sarif_schema)
        self.tokenizer = None
        if args.tokenizer:
            import tiktoken
            self.tokenizer = tiktoken.get_encoding(args.tokenizer)
        self.summary = read(args.summary)
        self.publications = {row['id']:row for row in self.summary['repositories']}
        self.context = {
            'specification':self.spec, 'repositories':self.repositories,
            'summary_digest':material_digest(args.summary),
            'evaluator':evaluator_digest(),
            'workflow_sources':{name:material_digest(Path(__file__).with_name(name))
                                for name in ('workflow_campaign.py','provider_observations.py','workflow_report.py')},
            'fixture':{name:material_digest(beneath(self.fixture,name))
                       for name in [*self.spec['source_files'],self.spec['expectations']]},
            'tools':{name:material_digest(path) for name,path in self.tools.items()},
            'sarif_schema':material_digest(args.sarif_schema),
            'tokenizer':None if self.tokenizer is None else {'name':args.tokenizer,'package':version('tiktoken')},
            'host':{'system':platform.platform(),'machine':platform.machine(),'python':sys.version},
            'native':args.native,
        }
        names = [case['repository'] for case in self.spec['cases']]
        if len(names) != len(set(names)) or not set(names) <= self.repositories['repositories'].keys():
            raise ContractError('workflow cases must uniquely name configured repository pins')
        self.database = self.root / 'knowledge.sqlite3'
        campaign = self.root / 'campaign.sqlite3'
        self.store = CampaignStore(campaign, manifest_digest=digest(canonical_json(self.context)), create=not campaign.exists())
        self.store.put_receipts([('inputs',canonical_json(self.context))])
        # Retain the small evaluator sources needed to reproduce this study's
        # observations after the checkout evolves; no credentials are copied.
        sources = self.root/'inputs'/'harness'
        sources.mkdir(parents=True,exist_ok=True)
        for path in Path(__file__).parent.glob('*.py'):
            target=sources/path.name
            if target.exists() and target.read_bytes()!=path.read_bytes():
                raise ContractError('retained evaluator source changed')
            if not target.exists():
                target.write_bytes(path.read_bytes())

    def execute(self, key: str, argv: list[str], cwd: Path, *, timeout=120, json_output=True):
        selected = next((name for name,path in self.tools.items() if str(path)==argv[0]),None)
        if selected:
            verify_tool(self.tools[selected],self.context['tools'][selected])
        if self.args.native:
            verify_tool(self.tools['producer'],self.context['tools']['producer'])
        prior = self.store.get_receipt('step/'+key)
        if prior is None:
            result = command(argv,cwd,self.root/'commands',key.replace('/','-'),self.environment,timeout,tokenizer=self.tokenizer)
            self.store.put_receipts([('step/'+key,canonical_json(result))])
            if selected:
                verify_tool(self.tools[selected],self.context['tools'][selected])
        else:
            result = json.loads(prior)
            for stream in ('stdout','stderr'):
                if material_digest(Path(result[stream])) != result['output'][stream]['content_digest']:
                    raise ContractError('retained command output changed')
        if result['status'] != 'complete':
            raise ContractError(f'failed attempt retained at {result["stderr"]}; use a fresh run after repair')
        return read(Path(result['stdout'])) if json_output else result

    def cli(self, key: str, arguments: list[str], cwd: Path, **options):
        return self.execute(key,[str(self.tools['cli']),'--standalone','--data-dir',str(self.root/'data'),*arguments],cwd,**options)

    def retain(self, key: str, publication: dict) -> dict:
        request = self.root/'requests'/(key.replace('/','-')+'-retain.json')
        write(request,{'graph':publication['graph'],'source_store':publication['store'],
                       'database':str(self.database),'locator':str(self.root/'views'/(key.replace('/','-')+'.view.json'))})
        (self.root/'views').mkdir(exist_ok=True)
        return self.execute(key+'/retain',[str(self.tools['retain']),str(request)],self.root)

    def query(self, key: str, publication: dict, request: dict):
        path = self.root/'requests'/(key.replace('/','-')+'.json')
        write(path,request)
        return self.cli(key,['request','--graph',publication['graph'],'--store',publication['store'],
                             '--request',str(path)],self.root)

    def case(self, case: dict) -> dict:
        name = case['repository']
        spec = self.repositories['repositories'][name]
        publication = self.publications.get(name)
        if publication is None or publication.get('inventory_status') != 'complete':
            return {'id':name,'status':'unavailable','reason':'repository_publication_unavailable'}
        source = Path(publication['publication']['source_root'])
        original_graph = read(Path(publication['publication']['graph']))
        files = {item['path']:item['content_digest'] for item in original_graph['snapshot_files']}
        generated = {path:files[path] for path in self.spec.get('generated_source_paths',[]) if path in files}
        source = verify_checkout(source,source,spec['revision'],generated_paths=generated)
        if read(source/'package.json')['version'] != case['version']:
            raise ContractError('selected package version differs from source pin')
        revision = git_revision(source,spec['revision'])
        lock = source/'package-lock.json'
        selection = {'subject':spec.get('subject'), 'version':case['version'], 'revision':revision,
                     'lock_digest':material_digest(lock) if lock.is_file() else None}
        root = self.root/'fixtures'/name
        fixture_checkout(root,self.fixture,self.spec['source_files'],selection,self.environment)
        values = {'node':str(self.tools['node']),'fixture':str(root),'source':str(source)}
        argv = []
        for argument in self.spec['observer']:
            argv.append(re.sub(r'\{(node|fixture|source)\}', lambda match: values[match.group(1)],argument))
        observation = self.execute(name+'/runtime',argv,root,timeout=self.spec['runtime_timeout_seconds'])
        grade = grade_observations(observation['observations'],self.expected[case['expected_set']])
        # Freeze independent runtime evidence before querying Entmoot or attaching it.
        self.store.put_receipts([('oracle/'+name,canonical_json({'selection':selection,'grade':grade,'observations':observation}))])
        indexed = self.cli(name+'/fixture-init',['init','--root',str(root),'--repo',self.spec['subject'],
                           '--subject',self.spec['subject'],'--structural-only'],root,timeout=None)
        library = publication['publication']
        if self.args.native:
            indexed = self.cli(name+'/fixture-bind',['bind','--root',str(root),'--timeout-seconds','120'],root,timeout=None)
            native = self.cli(name+'/library-bind',['bind','--root',str(source),'--graph',library['graph'],
                             '--store',library['store'],'--timeout-seconds','120'],source,timeout=None)
            library = native['publication']
        retained_library = self.retain(name+'/library',library)
        consumer = indexed['publication']
        verification = {'files':{path:material_digest(root/path) for path in self.spec['provider_paths']},
                        'selection':selection,'observer_inputs':self.context['fixture'],
                        'execution_receipt':digest(self.store.get_receipt('step/'+name+'/runtime'))}
        sarif = observation_sarif(root,observation['observations'],verification)
        jsonschema.Draft7Validator(self.schema).validate(sarif)
        artifact = self.root/'providers'/name/'observations.sarif'
        write(artifact,sarif)
        prepare = artifact.with_name('prepare.json')
        write(prepare,{'graph':consumer['graph'],'artifact':str(artifact),'format':'sarif21',
                       'producer':PRODUCER,'version':'1.0.0','included_paths':self.spec['provider_paths'],
                       'config_digest':digest(canonical_json(verification))})
        self.execute(name+'/prepare',[str(self.tools['prepare']),str(prepare)],root,json_output=False)
        attached = {'graph':str(artifact.with_name('graph.json')),'store':str(artifact.with_name('knowledge.sqlite3'))}
        self.cli(name+'/attach',['attach','--graph',consumer['graph'],'--out',attached['graph'],
                 '--store',attached['store'],'--artifact',str(artifact)+'.manifest.json'],root)
        before, after = read(Path(consumer['graph'])), read(Path(attached['graph']))
        preserve_evidence(before, after)
        retained_consumer = self.retain(name+'/observations',attached)
        selected = {'graph':attached['graph'],'store':retained_consumer['locator']}
        page_number = 0
        def finding_page(cursor):
            nonlocal page_number
            result = self.query(name+f'/find-observations-{page_number}',selected,{
                'operation':'find','request':{'kind':'finding','query':'runtime.observation','limit':100},
                'output':{'tier':'compact','after':cursor}})
            page_number += 1
            return result
        findings = collect_pages(finding_page)
        observation_ids = {row['observation_id'] for row in after['findings'] if row['rule_id']=='runtime.observation'}
        surfaced = {item['id'] for item in findings}
        expanded = []
        for number, identifier in enumerate(sorted(surfaced)):
            detail = self.query(name+f'/inspect-observation-{number}',selected,{
                'operation':'inspect','request':{'kind':'evidence','id':identifier},'output':{'tier':'export'}})
            finding = detail['result']['body']['finding']
            expanded.append(finding['observation_id'])
            expected_finding = next(row for row in after['findings'] if row['observation_id']==identifier)
            if finding != expected_finding:
                raise ContractError('compact reference expansion changed the stored finding')
        attachments = [item for item in after['attachments'] if item['artifact_id']==read(Path(str(artifact)+'.manifest.json'))['artifact_id']]
        source_grain = all(item.get('selected_target',{}).get('kind')=='call_site'
                           and item.get('freshness')=='current' for item in attachments)
        grade['attachment'] = {'expected':len(observation['observations']), 'observed':len(attachments),
                               'exact_callsite':source_grain,'all_findings_surfaced':observation_ids==surfaced,
                               'exact_reference_expansions':len(expanded),'compact_pages':page_number}
        queries = [
            ('inventory', ['inventory','--graph',selected['graph'],'--store',selected['store'],'--limit','5']),
        ]
        for repetition in range(self.spec['query_repetitions']):
            for operation, arguments in queries:
                self.cli(f'{name}/{operation}-{repetition}',arguments,root)
            self.query(f'{name}/boundaries-{repetition}', selected, {
                'operation':'find','request':{'kind':'call_boundary','request':{'path':'probe.cjs','limit':50}},
                'output':{'tier':'compact','limit':5}})
            self.execute(f'{name}/grep-{repetition}',[str(self.tools['rg']),'-n','router.get|http.request|observe',str(root/'probe.cjs')],root,json_output=False)
            # Alternate output tiers to reduce fixed-order cache bias. Both arms
            # select the same finite observation set; model behavior is unmeasured.
            for tier in (['compact','export'] if repetition % 2 == 0 else ['export','compact']):
                self.query(f'{name}/find-{tier}-{repetition}',selected,{
                    'operation':'find','request':{'kind':'finding','query':'runtime.observation','limit':100},
                    'output':{'tier':tier,'limit':100}})
        row = {'id':name,'selection':selection,'status':'complete' if grade['complete'] and source_grain and observation_ids==surfaced and len(attachments)==len(observation_ids) else 'incorrect',
               'grade':grade,'library':retained_library,'consumer':retained_consumer,
               'library_graph':library['graph'],'consumer_graph':attached['graph'],
               'native_stages':{'consumer':indexed.get('stages',[]),'library':native.get('stages',[]) if self.args.native else []},
               'provider':{'producer':PRODUCER,'artifact':str(artifact),
               'grain':'scenario/source_span/configuration','distinct_sites':len({item.get('selected_target',{}).get('node_id') for item in attachments})}}
        self.store.put_receipts([('case/'+name,canonical_json(row))])
        return row

    def run(self):
        results=[]
        for case in self.spec['cases']:
            row=self.case(case)
            results.append(row)
            write(self.root/'progress.json',{'cases':results,'planned':len(self.spec['cases'])})
            print(json.dumps({'case':row['id'],'status':row['status']}),flush=True)
        for base,head in zip(results,results[1:]):
            if 'library_graph' in base and 'library_graph' in head:
                self.cli('impact/'+base['id']+'-'+head['id'],['impact','--base-graph',base['library_graph'],
                         '--head-graph',head['library_graph'],'--subject',head['selection']['subject'],
                         '--correspondence-mode','subject_match','--no-revalidate'],self.root,timeout=None)
        by_key = {key.removeprefix('step/'):json.loads(raw) for key,raw in self.store.connection.execute("SELECT receipt_key,body FROM receipts WHERE receipt_key LIKE 'step/%'")}
        steps = list(by_key.values())
        try:
            measured_bytes = database_bytes(self.database) if self.database.exists() else None
        except FileNotFoundError:
            measured_bytes = None
        result={'cases':results,'planned':len(self.spec['cases']),'correct':sum(r['status']=='complete' for r in results),
                'production_database':str(self.database),'database_bytes':measured_bytes,
                'commands':len(steps),'command_wall_seconds':sum(step['wall_seconds'] for step in steps),
                'median_command_seconds':(statistics.median(step['wall_seconds'] for step in steps) if steps else None),
                'output_tokens':token_summary(steps),
                'operation_costs':summarize_steps(by_key),'paired_output_tiers':paired_payloads(by_key),
                'actual_model_usage':None,'limitations':['No controlled model trial; payload tokens are not billed or reasoning usage.',
                    'Runtime scenarios and releases are correlated; this is a bounded capability sample, not ecosystem prevalence.',
                    'Native evidence remains scoped to the configured producer; no dependency-path completeness or security verdict.',
                    'Grep and individual queries are operation-volume controls, not equivalent completed agent tasks.']}
        write(self.root/'report.json',result)
        self.store.put_receipts([('report',canonical_json(result))])
        return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('config','repositories','summary','run-dir','cli','node','prepare-helper','retain-helper','sarif-schema'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--tokenizer')
    parser.add_argument('--native',action='store_true')
    args=parser.parse_args()
    args.run_dir.mkdir(parents=True,exist_ok=True)
    with run_lock(args.run_dir):
        run=WorkflowRun(args)
        try:
            result=run.run()
            print(json.dumps({key:result[key] for key in ('planned','correct','commands','production_database','database_bytes')}))
        finally:
            run.store.close()


if __name__=='__main__':
    main()
