"""Codex CLI adapter for the provider-neutral agent trial loop."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil

from evals.harness.agent_provider import ProviderTurn, USAGE_FIELDS
from evals.harness.contracts import ContractError
from evals.harness.measurement import command
from evals.harness.revision_campaign import material_digest


def events(raw: bytes) -> list[dict]:
    events = [json.loads(line) for line in raw.decode('utf-8').splitlines() if line.strip()]
    if any(not isinstance(event, dict) for event in events):
        raise ContractError('Codex events must be objects')
    return events


def reported_usage(raw: bytes) -> dict:
    stream = events(raw)
    completions = [event for event in stream if event.get('type') == 'turn.completed']
    if len(completions) != 1:
        raise ContractError('Codex usage requires exactly one completed turn')
    usage = completions[0].get('usage', {})
    if not isinstance(usage, dict):
        raise ContractError('Codex usage must be an object')
    for key in USAGE_FIELDS:
        value = usage.get(key)
        if value is not None and (type(value) is not int or value < 0):
            raise ContractError('invalid reported Codex usage: ' + key)
    return {key: usage.get(key) for key in USAGE_FIELDS}


def model_result(raw: bytes) -> object:
    stream = events(raw)
    completions = [event for event in stream if event.get('type') == 'turn.completed']
    if len(completions) != 1 or any(e.get('type') in ('turn.failed', 'error') for e in stream):
        raise ContractError('Codex did not complete exactly one turn')
    items = [event['item'] for event in stream if event.get('type') == 'item.completed']
    if any(not isinstance(item, dict) for item in items):
        raise ContractError('Codex completed items must be objects')
    if any(item.get('type') not in ('agent_message', 'reasoning') for item in items):
        raise ContractError('Codex used a built-in tool outside the controlled trial interface')
    messages = [item['text'] for item in items if item.get('type') == 'agent_message']
    if not messages:
        raise ContractError('completed Codex turn has no answer')
    return json.loads(messages[-1])


def model_arguments(codex: Path, schema: Path, spec: dict) -> list[str]:
    args = [str(codex), 'exec', '--ignore-user-config', '--skip-git-repo-check', '--ephemeral', '--json',
            '--sandbox', 'read-only', '-c', 'approval_policy="never"', '-c', 'web_search="disabled"',
            '-c', 'project_doc_max_bytes=0', '--model', spec['model'],
            '-c', 'model_reasoning_effort=' + json.dumps(spec['reasoning_effort'])]
    for feature in ('shell_tool', 'unified_exec', 'apps', 'plugins', 'multi_agent'):
        args.extend(['--disable', feature])
    return args + ['--output-schema', str(schema), '-']


class CodexProvider:
    def __init__(self, configuration: dict, executable: Path | None = None):
        if set(configuration) != {'kind', 'model', 'reasoning_effort'}:
            raise ContractError('Codex provider fields differ')
        if configuration['kind'] != 'codex_cli' or any(not isinstance(configuration[key], str)
                or not configuration[key] for key in ('model', 'reasoning_effort')):
            raise ContractError('Codex requires explicit nonempty model and reasoning settings')
        selected = str(executable) if executable is not None else shutil.which('codex')
        if selected is None or not Path(selected).is_file():
            raise ContractError('Codex executable unavailable')
        self.executable = Path(selected).resolve()
        self.configuration = dict(configuration)
        self.executable_digest = material_digest(self.executable)
        self.adapter_digest = material_digest(Path(__file__))
        self.cli_version = None

    def prepare(self, directory: Path) -> None:
        self.verify_unchanged()
        receipt = command([str(self.executable), '--version'], directory, directory / 'commands',
                          'provider-version', os.environ.copy(), 10)
        if receipt['status'] != 'complete':
            raise ContractError('Codex version probe failed: ' + receipt['status'])
        self.cli_version = Path(receipt['stdout']).read_text(encoding='utf-8').strip()

    def identity(self) -> dict:
        return {**self.configuration, 'executable_digest': self.executable_digest,
                'adapter_digest': self.adapter_digest, 'cli_version': self.cli_version,
                'model_identity': 'requested alias; server weights/system prompt are not independently pinned',
                'tool_policy': 'built-in execution disabled; completed event types checked'}

    def verify_unchanged(self) -> None:
        if material_digest(self.executable) != self.executable_digest:
            raise ContractError('Codex executable changed during run')
        if material_digest(Path(__file__)) != self.adapter_digest:
            raise ContractError('Codex adapter changed during run')

    def respond(self, prompt: Path, schema: Path, directory: Path,
                timeout_seconds: float, tokenizer) -> ProviderTurn:
        self.verify_unchanged()
        measured = command(model_arguments(self.executable, schema, self.configuration), directory,
                           directory / 'commands', prompt.stem, os.environ.copy(), timeout_seconds,
                           tokenizer=tokenizer, stdin_path=prompt)
        usage = None
        raw = Path(measured['stdout']).read_bytes()
        try:
            usage = reported_usage(raw)
        except (ContractError, ValueError, KeyError, TypeError):
            # A failed process can still report usage; an incomplete stream does
            # not justify inventing counters. Complete responses are checked below.
            pass
        if measured['status'] != 'complete':
            return ProviderTurn(measured['status'], None, usage, measured, measured.get('launch_error'))
        try:
            usage = reported_usage(raw)
            action = model_result(raw)
        except (ContractError, ValueError, KeyError, TypeError) as error:
            return ProviderTurn('invalid_response', None, usage, measured, str(error)[:2000])
        return ProviderTurn('complete', action, usage, measured)
