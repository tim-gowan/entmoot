# Derived JSON Schema (not normative)

Generated from Rust interchange contracts via `schemars`:

```bash
cargo run -p entmoot-contracts --example emit_json_schema
cargo run -p entmoot-contracts --example emit_query_json_schema
```

- `entmoot_contracts.schema.json` — snapshot (including `SourceSubject` /
  subject aliases), artifact, claim, attachment, finding, assertion, resource
  inventory, scope, markers/index records, typed IDs, nested `package_api`
  shapes.
- `entmoot_query_contracts.schema.json` — query, revision impact
  (`CorrespondencePolicy` / receipts), inventory overlay
  (`InventoryOverlayRequest` / `InventoryOverlayReport`), compatibility
  (`CompatibilityVerdict`, `CompatibilityReason`), status, find/traverse, ….

Validates **wire structure** only. Epistemic and identity invariants remain in
`docs/architecture/ARCHITECTURE.md` and Rust/fixture tests. Drift is caught by
`json_schema_emit::tests::contract_schemas_match_golden` and
`query::tests::query_contract_schemas_match_golden`.

## Taxonomy export (repo-local review utility)

Flatten `$defs` keys and enum/const variants into scratch (gitignored):

```bash
python scripts/export_contract_taxonomy.py
python scripts/export_contract_taxonomy.py --regen
python scripts/export_contract_taxonomy.py --dogfood-self
```

``--dogfood-self`` also re-emits into scratch, diffs against these goldens, runs
full ``entmoot init`` (producer bind when the CLI is producer-enabled) on
``crates/entmoot-contracts``, captures normalized ``status`` /
``assess compatibility`` / ``assess impact`` receipts, lists MCP read tools when
``entmoot-mcp`` is present, and writes ``dogfood_compare.json`` (unknown receipt
paths / enum values / missing MCP tools fail closed).
