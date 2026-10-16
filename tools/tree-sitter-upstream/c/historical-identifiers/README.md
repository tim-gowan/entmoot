# Preserve older keyword spellings as identifiers

`constexpr`, `thread_local`, `alignas` and `alignof` were ordinary identifiers in
the C17 controls. The baseline, PR #331 and early supplements consumed several
of these as C23 keywords, losing declarations or producing missing nodes.

This supplement admits the four spellings at identifier sites, including type,
field, assignment and call contexts. Declaration and parenthesized `alignof`
rules retain competing interpretations until later tokens distinguish them.
Explicit GLR conflicts preserve these alternatives; unconditional precedence
previously discarded valid historical forms. The full upstream corpus remains
a gate, and [measured cost](../qualification_2026-10-10.json) is reported separately
from correctness. This is not an optimization claim.

[Fixture selectors](fixtures.json) reference [authored controls](../cases.py)
for variables, functions, fields and typedefs under actual Clang 23.1.3 and GCC
11.4 C17 modes. Clang rejects the same variable declarations under C23. The
version-neutral parser intentionally preserves their structure in both cases.
Literal-like spellings such as `true` can still have literal CST nodes in older
source: source profiles and lexical scope must govern Entmoot interpretation.
Neither node kind nor spelling alone establishes a builtin or local bind.

Replay using the [owner commands](../README.md) and
`../modern-types/composed.json`. No Entmoot visitor is migrated by this packet.
Review identifier-site coverage and ADR-0025 scope controls before adopting it.
Remove the supplement when upstream provides equivalent qualified behavior;
reassess at dependency updates and the repository move. No upstream submission
or maintainer acceptance is claimed.
