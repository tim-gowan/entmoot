# Parameterized atomic types

Supplement to pinned [C PR #331](https://github.com/tree-sitter/tree-sitter-c/pull/331),
not a submitted upstream PR. The original 0.24.2 grammar and proposal reject the
valid C11 `_Atomic(int)` declaration. `grammar.patch` adds a type specifier with
an explicit `type` field, preserving the separate `_Atomic int` qualifier form.

[Fixture selectors](fixtures.json) reference the independently authored
[shared cases](../cases.py). Clang 23.1.3 and GCC 11.4 compile and execute scalar
and pointer forms. Both reject a qualified atomic operand; the grammar preserves
that syntax without asserting semantic validity. The complete [receipt](../qualification_2026-10-10.json)
records byte/point spans, following declarations, incremental damage/repair,
corpus, platform, example and cost results.

Reproduce through `../modern-types/composed.json` with the shared materializer,
then run `../qualify_features.py --language c --libraries <variants.json> --clang-root <pinned-clang> --out <fresh-directory>`.
The [owner](../README.md) records the pins, commands, contribution rules and limits.
Remove the supplement when an upstream release supplies equivalent qualified
behavior; reassess at dependency updates and the repository move. A human must
review the C type visitor with focused controls before production adoption.
