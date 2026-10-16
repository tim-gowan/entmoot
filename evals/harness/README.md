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

Run Cargo process controls with:

```text
python -m pip install -r evals/harness/requirements.txt
cargo test -p entmoot --test campaign_harness -- --ignored
cargo test -p entmoot-engine --test revision_receipts campaign_accounting -- --ignored
```

Tests author local Git sources and expectations independently. Real upstream
builds and native language qualification are separate, opt-in operations. Keep
scratch under `.tmp/<task>/`; use a fresh run after input/layout changes.
