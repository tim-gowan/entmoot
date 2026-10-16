# Documentation

Public meaning converges on crate rustdoc, live CLI help, MCP tool schemas and
generated contract JSON Schema. Markdown points; it does not paraphrase those
surfaces.

- [README](../README.md): main operator entry, quickstart and maturity.
- [ARCHITECTURE](architecture/ARCHITECTURE.md): dedicated identity, ownership and
  storage walkthrough.
- [Runbooks](runbooks/): short procedures (commands and links into README,
  ARCHITECTURE, rustdoc or help)—not API or architecture essays.
- [AGENTS](../AGENTS.md): maintainer/agent invariants (curate every turn);
  [llms.txt](../llms.txt) is a static repo-root discovery stub only.
- [PLANNING](../PLANNING.md): active work; [CHANGELOG](../CHANGELOG.md): delivered work.
- [Contracts](../crates/entmoot-contracts/src/lib.rs) and
  [generated schemas](../crates/entmoot-contracts/generated/README.md): wire shape.
- [Campaign harness](../evals/harness/README.md): SQLite integration runs.
- [Local alpha](runbooks/local-alpha.md): release gates and historical corpus sample.
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

Edit or shorten the existing owner before adding prose. Prefer rustdoc and
executable interfaces for durable semantics; keep Markdown in README,
ARCHITECTURE, runbooks, AGENTS or concise rules. Prefer focused tests over
parallel narratives. Git retains superseded material.

Keep evidence only when it explains a useful result or regression. Historical
measurements do not impose active commitments. Exact receipt bytes, append-only
archives and redundant copies are not general requirements. Reusable code stays
tracked; scratch and useful local evidence share one `.tmp/<task>/` root.
