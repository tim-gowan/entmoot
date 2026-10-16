# Qualify existing PR #361

Pinned proposal: [PR #361](https://github.com/tree-sitter/tree-sitter-typescript/pull/361),
commit `d5d7b1a83899ef39f4c08b385c2d5529b999e204`, MIT. The
[grammar patch](grammar.patch) and [upstream corpus additions](fixtures/variance.txt)
are extracted from that commit without generated parser sources. Baseline
`75b3874edb2dc714fb1fd77a32013d0f8699989f` rejects the independently authored
`in`, `out` and `in out` controls; both actual TypeScript compilers accept them.

Composition with JavaScript PR #378 exposes TSX ambiguities between keyword JSX
names and type parameters, variance, constraints, type queries, primary types,
literal types and predefined types. The separate
[composition patch](../javascript-025/composition.patch) declares those ten
bounded conflicts and adopts the shared JSX member-name rule. It leaves both
parse branches available and is a local supplement, not part of upstream #361.
The [shared controls](../../javascript/cases.py) exercise both branches and
compiler-rejected variance placement. The grammar does not validate variance.

The [shared receipt](../../javascript/qualification_2026-10-10.json) records both
dialects, historical syntax, incremental repair, full corpus, available bindings,
examples and measured cost. Platform and cost limits remain specific to that observation. Remove the supplement when an upstream combination passes the
same controls; reassess at dependency updates and the repository move. Upstream submission is deferred; future production changes use focused visitor controls.
