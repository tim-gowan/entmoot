# Qualify the existing modern C proposal

Existing [PR #331](https://github.com/tree-sitter/tree-sitter-c/pull/331) is pinned
at `0e6495284f5b1b43d5fff00ad1903e8bad90d350`, over C 0.24.2
`b780e47fc780ddc8da13afa35a3f4ed5c157823d`. The proposal is open at the cutoff;
its description is not qualification evidence. `proposal.json` builds the
proposal unchanged; `composed.json` adds the three separately scoped supplements.

The initial 42 compiler-backed controls show the proposal fixes ten baseline
grammar gaps: complex types, `_Thread_local`, C23 inferred `auto`, expression
`typeof`, bit-precise integers, fixed enum underlying types and `#embed`.
Parameterized atomic types, historical keyword identifiers and trailing labels
require the separate supplements. Expanded [fixture selectors](fixtures.json)
and [shared cases](../cases.py) preserve exact node kinds/fields and following
declarations rather than accepting error-free trees alone.

The proposal also changes GNU/MS extension forms, pragmas and escapes. Its full
upstream corpus is exercised, but this packet does not give those additions
compiler-backed semantic qualification. Earlier broad PR #293 and narrow
macro-type PR #326 were inspected as prior proposals; no duplicate patch or
equivalent-behavior claim is made for them.

Use the [owner](../README.md) for generator, ABI, platform, historical compiler,
example and measured-cost results and replay commands. PR #331's generated
sources match CLI 0.26.13; the original baseline uses locked CLI 0.25.4.
No production adoption or native resolver qualification occurs here. Reassess
the pinned proposal at upstream changes and the repository move. Human review
must select the qualified scope and use focused visitor controls (ADR-0025).
