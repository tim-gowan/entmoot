# Qualify if consteval

The original grammar rejects valid C++23 `if consteval`. Existing
[PR #362](https://github.com/tree-sitter/tree-sitter-cpp/pull/362), pinned at
`92fda59cbcfe37c302e3c4babe56aada68228b54`, proposes the basic branch split.
`grammar.patch` adapts that narrow change onto the current pinned baseline,
preserves `not` as an alternative to `!`, and requires compound statements for
both consteval branches. Ordinary `if`/`if constexpr` keep their existing forms.

[Fixture selectors](fixtures.json) reference [authored cases](../cases.py).
Clang 23.1.3 compiles and executes positive, negated and ordinary historical
controls; strict C++20 rejects the new syntax and C++23 rejects non-compound
branches. Exact consequence/alternative fields and the unrelated following
function are checked independently of parse success.

The [owner](../README.md) records pins, complete corpus, binding, example,
recovery and cost results. Replay using `composed.json` and the shared C-family
feature driver. This remains experimental C++; it does not qualify template
closure, ABI, compilation databases or Entmoot native binding. Reassess the
temporary supplement at upstream updates and the repository move, remove it
when equivalent behavior is qualified upstream, and test affected occurrences before changing the production bundle. No upstream submission or acceptance is claimed.
