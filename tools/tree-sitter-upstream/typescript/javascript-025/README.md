# Qualify existing PR #357

Existing [upstream proposal](https://github.com/tree-sitter/tree-sitter-typescript/pull/357),
pinned by [proposal.json](proposal.json). The baseline accepts `using resource = …`
as an assignment rather than a declaration, and a resource-management loop can
absorb the following function into an ERROR node. The shared
[authored controls](../../javascript/cases.py) check declaration occurrences,
following siblings, contextual identifiers, runtime disposal and both dialects.

This packet qualifies the proposal's actual source, not every claim in its PR
description. It updates the JavaScript dependency, removes the assignment-prefix
workaround and adjusts decorator parsing and lint. It depends on the separate
JavaScript reserved-property proposal for additional modern grammar regressions.
The locked generator is 0.25.8; tracked files were generated with 0.26.6. Both
tracked and regenerated parsers must be compared explicitly.

The [composition supplement](composition.patch) resolves ten TSX keyword/type
conflicts and preserves JSX member names; [type-query names](type-query-names.patch)
preserve reserved properties such as `typeof import(...).default`. These are
separate local corrections, not hidden changes to the pinned proposal.

The [shared receipt](../../javascript/qualification_2026-10-10.json) records
full local qualification, original Cargo/Go metadata failures and guarded
tool-generated lock repairs. The packet does not claim complete platform coverage.
Keep compiler-only semantics and the unavailable TypeScript 7 Program API
separate. When changing the bundle, cover both dialects and relevant recovery and binding behavior. Remove
temporary adoption when upstream supplies qualified equivalent behavior; reassess
at dependency updates and the repository move. Upstream submission is deferred.
