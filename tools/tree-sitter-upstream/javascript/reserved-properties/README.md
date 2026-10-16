# Qualify existing PR #378

Pinned [upstream proposal](https://github.com/tree-sitter/tree-sitter-javascript/pull/378)
in [proposal.json](proposal.json). Qualified source is the commit's actual grammar,
not the PR description. The associated issue is
[#377](https://github.com/tree-sitter/tree-sitter-javascript/issues/377).

The [shared authored controls](../cases.py) cover reserved property names after
optional chaining and JSX member names. Node verifies ordinary JavaScript;
TypeScript 6.0.3 and 7.0.2 compile JSX with an authored local factory, then Node
executes the output. Neither compiler success nor grammar success establishes an
Entmoot semantic target. Both TypeScript dialects must qualify the changed shared
dependency, including controls that already passed under JavaScript 0.23.1.

The local [module-name supplement](module-names.patch) preserves reserved export
names while keeping the existing anonymous `default` node. Full examples exposed
this regression; independent TypeScript 6/7 and JavaScript execution controls
cover it. A coupled TypeScript supplement preserves reserved property names in
`typeof import(...).default`.

The [shared receipt](../qualification_2026-10-10.json) retains the complete local
qualification and its limits. The packet does not establish complete platform coverage or upstream acceptance. Remove a temporary patch when upstream provides
qualified equivalent behavior; reassess at dependency updates and the repository
move. Upstream submission is deferred; future bundle changes use focused visitor controls.
