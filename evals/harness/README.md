# Campaign harness

The existing SQLite harness runs pinned repositories and revision comparisons.
It stores operation results, input identity and explicit limitations. Raw output
stays under the task directory when useful; native payloads remain in their
publication instead of being copied into campaign storage.

- `repository_campaign`: isolated checkout/build, automatic Entmoot publication
  and bounded lock-derived dependencies (root depth zero, default one).
- `run_revision_campaign`: direct-parent or explicitly selected revision windows,
  production occurrence correspondence and bounded caller accounting.
- `revision_campaign`: shared transactional storage and input validation.
- `knowledge_sql`: bounded read-only access to a qualified production store.
- `workflow_campaign`: independent observers, source-verified provider fixtures,
  exact-reference retrieval and production view retention over repository results.
- `measurement` / `workflow_report`: shared process observations and descriptive
  costs. Missing counters stay unknown; output tokens are not model usage.
- `history_pilot`: bounded, seeded history/CI candidate sampling, independent Git
  source-byte grading and retained production views. CI metadata is not a bug oracle.
- `agent_trials` / `trial_oracles`: paired local agent decisions through bounded
  source/Git tools and optional Entmoot reads, with independent source oracles.
  Model usage, failed attempts and incomplete answers remain visible. See the
  [local pilot recipe](../../docs/runbooks/revision-evaluation.md#local-history-pilot-and-agent-calibration).
- `agent_provider` / `codex_provider`: provider-neutral response/usage contract
  and the current Codex CLI adapter. The runner does not parse vendor events or
  assemble vendor flags. Additional providers implement the same bounded-action
  contract; a fixture provider tests substitution without consuming model usage.
- `pilot_report`: read-only summary of stored grades, operation resources and
  paired costs; no model calls, product queries or database mutation.

Run Cargo process controls with:

```text
python -m pip install -r evals/harness/requirements.txt
cargo test -p entmoot --test campaign_harness -- --ignored
cargo test -p entmoot --features producer,native-tests --test campaign_harness revision_campaign_joins_actual_occurrences_and_preserves_unconsumed_frontiers -- --exact --ignored
cargo test -p entmoot-engine --test revision_receipts campaign_accounting -- --ignored
cargo test -p entmoot-engine --example retain_campaign_publication
```

Tests author local Git sources and expectations independently. Real upstream
builds and native language qualification are separate, opt-in operations. Keep
scratch under `.tmp/<task>/`; use a fresh run after input/layout changes.

For a single pinned upstream structural demo (no adapters), use
[`../demo/getrandom-structural.json`](../demo/getrandom-structural.json) with
`repository_campaign run --depth 0 --structural-only`, or the dispatch workflow
`demo.yml`.
