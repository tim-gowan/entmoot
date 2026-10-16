# Draft pack indexing with historical controls

The existing grammar can accept `value...[0]` while assigning the wrong CST.
The independently authored selector therefore requires `pack_index_expression`
with `pack` and `index` fields; type positions require `pack_index_type`.

[PR #375](https://github.com/tree-sitter/tree-sitter-cpp/pull/375), pinned at
`11d27b974e1b10f93f99044030bd11a662074c7f`, contains a broad C++23/26 proposal.
This supplement selects only pack indexing. It keeps ellipsis and bracket as
separate tokens so intervening comments remain valid source and visible trivia.
Bounded conflicts preserve older variadic parameters, pack expansions and folds;
an early static-precedence attempt broke historical controls and is discarded.
The proposal's other changes are not adopted or semantically qualified here.

[Fixture selectors](fixtures.json) reference [shared authored controls](../cases.py).
Actual Clang 23.1.3 compiles and executes expression, comment-separated and type
forms in `-std=c++26`. These are **draft, experimental** observations and do not
claim a stable C++26 standard. Historical C++11 variadic code also compiles and
runs with Clang 23.1.3 and GCC 11.4. The full corpus and exact following-function
selectors guard against declaration loss.

See the [owner](../README.md) for replay commands and
[retained evidence](../../c/qualification_2026-10-10.json). This change composes
after `../if-consteval/grammar.patch`. Remove or reassess it at upstream updates
and the repository move; future production changes should cover the affected CST and occurrence behavior. No maintainer acceptance is claimed.
