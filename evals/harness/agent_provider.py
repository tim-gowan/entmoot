"""Provider boundary for constrained, one-decision agent trials.

Providers own invocation, response decoding, tool isolation and usage semantics.
The runner owns action validation, budgets, tools, history and grading. A provider
must not execute model-requested tools or receive an oracle. Missing usage is None;
cached input and reasoning output are subsets, never additional token charges.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from evals.harness.contracts import ContractError


USAGE_FIELDS = ('input_tokens', 'cached_input_tokens', 'cache_write_input_tokens',
                'output_tokens', 'reasoning_output_tokens')


@dataclass(frozen=True)
class ProviderTurn:
    status: str
    action: object | None
    usage: dict | None
    measurement: dict
    error: str | None = None


class AgentProvider(Protocol):
    """One isolated response, respecting the deadline and preserving failed usage.

    Return complete only for an action decoded without out-of-interface tool use.
    Invalid responses and timeouts still return a measurement and any known usage.
    Preparation may probe availability/version, but never install or authenticate.
    """

    def identity(self) -> dict: ...

    def prepare(self, directory: Path) -> None: ...

    def verify_unchanged(self) -> None: ...

    def respond(self, prompt: Path, schema: Path, directory: Path,
                timeout_seconds: float, tokenizer) -> ProviderTurn: ...


def sum_usage(rows: list[dict]) -> dict:
    return {key: sum(row[key] for row in rows) if rows and all(row.get(key) is not None for row in rows) else None
            for key in USAGE_FIELDS}


def load_provider(configuration: dict, executable: Path | None = None) -> AgentProvider:
    """Select explicitly; unsupported providers never silently fall back."""
    if not isinstance(configuration, dict):
        raise ContractError('agent provider configuration must be an object')
    if configuration.get('kind') == 'codex_cli':
        from evals.harness.codex_provider import CodexProvider
        return CodexProvider(configuration, executable)
    raise ContractError('unsupported agent provider: ' + str(configuration.get('kind')))
