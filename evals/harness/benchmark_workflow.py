"""Repeat read-only finding queries after setup; preserve exact-answer controls."""
from __future__ import annotations

import argparse
from importlib.metadata import version
import json
import os
from pathlib import Path
import platform

import psutil

from evals.harness.contracts import ContractError
from evals.harness.measurement import command, output_measurement
from evals.harness.revision_campaign import canonical_json, material_digest, evaluator_digest
from evals.harness.workflow_report import paired_payloads, summarize_steps


def finding_ids(response: dict) -> list[str]:
    if response['tier']=='compact':
        if response['page'].get('next_cursor') or response.get('search_truncated'):
            raise ContractError('benchmark answer is incomplete')
        return [item['id'] for item in response['items']]
    result=response['result']
    if result['bounds'].get('truncated'):
        raise ContractError('benchmark answer is incomplete')
    return [item['finding']['observation_id'] for item in result['items']]


def run(report: Path, cli: Path, destination: Path, repetitions: int, encoding: str | None) -> dict:
    if not 1 <= repetitions <= 100:
        raise ContractError('repetitions must be between 1 and 100')
    destination=destination.resolve()
    destination.mkdir(parents=True,exist_ok=False)
    cli=cli.resolve(strict=True)
    tokenizer=None
    if encoding:
        import tiktoken
        tokenizer=tiktoken.get_encoding(encoding)
    study=json.loads(report.read_bytes())
    context={'report_digest':material_digest(report),'cli_digest':material_digest(cli),
             'benchmark_digest':material_digest(Path(__file__)), 'repetitions':repetitions,
             'evaluator_digest':evaluator_digest(),
             'reporter_digest':material_digest(Path(__file__).with_name('workflow_report.py')),
             'tokenizer':None if not encoding else {'name':encoding,'package':version('tiktoken')},
             'host':{'platform':platform.platform(),'logical_cpus':psutil.cpu_count(),
                     'memory_bytes':psutil.virtual_memory().total},
             'cache':'warm OS/store caches; fresh CLI processes; alternating tier order'}
    (destination/'inputs.json').write_bytes(canonical_json(context))
    steps={}
    requests=[]
    relay_samples=[]
    for case in study['cases']:
        if case['status']!='complete':
            raise ContractError('benchmark requires completely graded cases')
        graph=Path(case['consumer_graph'])
        expected={item['observation_id'] for item in json.loads(graph.read_bytes())['findings']
                  if item['rule_id']=='runtime.observation'}
        for repetition in range(repetitions):
            for tier in (['compact','export'] if repetition % 2 == 0 else ['export','compact']):
                key=f"{case['id']}/find-{tier}-{repetition}"
                request=destination/'request.json'
                request.write_bytes(canonical_json({'operation':'find',
                    'request':{'kind':'finding','query':'runtime.observation','limit':100},
                    'output':{'tier':tier,'limit':100}}))
                request_cost=output_measurement(request,tokenizer=tokenizer)
                requests.append({'step':key,**request_cost})
                # Identity verification is outside the measured process interval.
                if material_digest(cli)!=context['cli_digest']:
                    raise ContractError('CLI changed during the benchmark')
                result=command([str(cli),'--standalone','--data-dir',str(destination/'data'),
                    'request','--graph',str(graph),'--store',case['consumer']['locator'],
                    '--request',str(request)],destination,destination/'commands',key.replace('/','-'),
                    os.environ.copy(),60,tokenizer=tokenizer)
                steps[key]=result
                if result['status']!='complete':
                    raise ContractError('query failed; measured attempt retained')
                identifiers=finding_ids(json.loads(Path(result['stdout']).read_bytes()))
                if len(identifiers)!=len(expected) or set(identifiers)!=expected:
                    raise ContractError('output tier changed the complete finding answer')
                (Path(result['stdout']).parent/'request.json').write_bytes(request.read_bytes())
                (Path(result['stdout']).parent/'request-metrics.json').write_bytes(canonical_json(request_cost))
    for base, head in zip(study['cases'],study['cases'][1:]):
        key='relay/'+base['id']+'-'+head['id']
        request=destination/'relay-request.json'
        request.write_bytes(canonical_json({'budget':{'max_hops':1,'max_expansions':10,
            'max_examined_edges':100,'max_edges_per_node':10,'max_results':1,'max_evidence_per_hop':1},
            'output':{'tier':'compact','limit':1,'max_bytes':8192},
            'callers_output':{'tier':'compact','limit':1,'max_bytes':4096}}))
        if material_digest(cli)!=context['cli_digest']:
            raise ContractError('CLI changed during the benchmark')
        result=command([str(cli),'--standalone','--data-dir',str(destination/'data'),'relay-impact',
            '--graph',head['library_graph'],'--compare-graph',base['library_graph'],
            '--store',head['library']['locator'],'--request',str(request)],destination,
            destination/'commands',key.replace('/','-'),os.environ.copy(),60,tokenizer=tokenizer)
        steps[key]=result
        if result['status']!='complete':
            raise ContractError('bounded relay failed; measured attempt retained')
        response=json.loads(Path(result['stdout']).read_bytes())
        if (response['base_snapshot']!=base['library']['snapshot']
                or response['head_snapshot']!=head['library']['snapshot'] or len(response['items'])>1):
            raise ContractError('relay changed the selected pair or exceeded its subject budget')
        relay_samples.append({'base':base['id'],'head':head['id'],'page':response['page'],
                              'gaps':response['gaps'], 'scope':'bounded initial signal; not complete impact or independent semantic truth'})
        request_cost=output_measurement(request,tokenizer=tokenizer)
        requests.append({'step':key,**request_cost})
        (Path(result['stdout']).parent/'request.json').write_bytes(request.read_bytes())
        (Path(result['stdout']).parent/'request-metrics.json').write_bytes(canonical_json(request_cost))
    if material_digest(cli)!=context['cli_digest']:
        raise ContractError('CLI changed during the benchmark')
    result={'context':context,'cases':len(study['cases']),'commands':len(steps),
            'operation_costs':summarize_steps(steps),'paired_output_tiers':paired_payloads(steps),
            'request_measurements':requests,
            'relay_samples':relay_samples,
            'actual_model_usage':None,'limitations':[
                'One correlated framework family; no population success-rate estimate.',
                'Payload reduction is not billed or reasoning-token savings.',
                'Tool schemas and adaptive agent context are outside this scripted query sample.',
                'Sampling may miss short-lived processes; shared pages may be counted twice.',
                'No controlled cold-cache measurement or equal-task grep/agent comparison.']}
    (destination/'report.json').write_bytes(canonical_json(result))
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--cli',type=Path,required=True)
    parser.add_argument('--run-dir',type=Path,required=True)
    parser.add_argument('--repetitions',type=int,default=5)
    parser.add_argument('--tokenizer')
    args=parser.parse_args()
    result=run(args.report,args.cli,args.run_dir,args.repetitions,args.tokenizer)
    print(json.dumps({key:result[key] for key in ('cases','commands')}))


if __name__=='__main__':
    main()
