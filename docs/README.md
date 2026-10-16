# Documentation

- [README](../README.md): product entry and commands.
- [AGENTS](../AGENTS.md): development guidance.
- [ARCHITECTURE](architecture/ARCHITECTURE.md): identities, ownership and storage.
- [PLANNING](../PLANNING.md): active work; [CHANGELOG](../CHANGELOG.md): delivered work.
- [Contracts](../crates/entmoot-contracts/src/lib.rs) and
  [generated schemas](../crates/entmoot-contracts/generated/README.md): wire shape.
- [Runbooks](runbooks/): procedures for operation and validation.
- [Campaign harness](../evals/harness/README.md): SQLite integration runs.
- [Local alpha](runbooks/local-alpha.md): operator storage/boundary semantics, release gates and historical corpus sample.
- [Grammar bundle](../tools/tree-sitter-upstream/README.md): production inputs.

## Compare and assess

Three read-only rungs (missing evidence is unknown, never absence):

1. **Impact** — same-checkout or subject-correspondence revision compare
   ([impact](runbooks/impact.md); meaning in [ARCHITECTURE](architecture/ARCHITECTURE.md)).
2. **Overlay** — inventory symbol presence across publications
   (`entmoot assess overlay` / MCP `assess_overlay`; same impact runbook).
3. **Compatibility** — optional Bound/Fits on `package_api`
   ([compatibility](runbooks/compatibility.md)).

Wire shapes: [generated schemas](../crates/entmoot-contracts/generated/README.md).

## Living vs dated

Edit or shorten the existing owner before adding prose. Keep Markdown in these
canonical files, README files, runbooks or rules. Prefer executable examples and
focused tests over parallel narratives. Git retains superseded material.

Keep evidence only when it explains a useful result or regression. Historical
measurements do not impose active commitments. Exact receipt bytes, append-only
archives and redundant copies are not general requirements. Reusable code stays
tracked; scratch and useful local evidence share one `.tmp/<task>/` root.
