# tree-sitter-javascript

Upstream: [tree-sitter-javascript](https://github.com/tree-sitter/tree-sitter-javascript),
MIT. Pinned release 0.25.0 is commit
`58404d8cf191d69f2674a8fd507bd5776f46cb11`; this is also Entmoot's current crate
version. Cutoff: **2026-10-10**. The shared
[qualification receipt](qualification_2026-10-10.json) covers JavaScript and both
TypeScript dialects; it does not adopt any production grammar.

The repository has active issue/proposal work but the pinned release lacks the
qualified forms below. No grammar-specific CONTRIBUTING or AGENTS file was
present in the inspected tree. Human contributors must recheck README, package
scripts and workflows before submission. The [core contribution policy](../README.md)
is separately scoped; this packet makes no maintainer contact or acceptance claim.

## Changes and ownership

- [Existing PR #378](reserved-properties/README.md): reserved property and JSX
  member names. A local supplement preserves reserved module export names,
  including the existing anonymous `default` node.
- [Resource line terminators](resource-line-terminators/README.md): distinguish
  declarations from ordinary `using` identifiers across LF, CR, U+2028/U+2029 and
  block comments. Its external-token order must agree with TypeScript's scanner.
- [Automatic semicolons](automatic-semicolons/README.md): retain expression
  continuations while recognizing CR and comment line terminators.
- [Source-phase imports](source-phase-imports/README.md): actual Node/Wasm
  controls preserve the ordinary binding named `source`. This is qualified
  implementation syntax, not a claim that the proposal belongs to ES2026.

Tree-sitter owns syntax, fields and occurrences. Oxc/Program producers retain
semantic dispatch, module closure and environment resolution. Entmoot's former
recovered-tree/cross-grammar fallback was an Entmoot defect, already removed;
these patches do not weaken strict ERROR/MISSING rejection. A spelled builtin
never establishes a binding. Neither JSX grammar acceptance nor an outer regexp
node establishes semantic legality.

## Qualification

[74 authored controls](cases.py) yield 170 source/dialect rows. Baseline,
proposal-only and composed grammars meet 113, 121 and 167 CST expectations,
respectively. The three remaining failures are malformed-source sibling recovery,
one per dialect. All 161 applicable composed incremental-edit checks pass; nine
error-tree cases are explicitly unqualified for that repair check.

All **291** compiler acceptance/rejection expectations match actual Node 26.11.1,
TypeScript 6.0.3 and native TypeScript 7.0.2. Of 243 executions, **242 pass**:
Node lacks `Math.sumPrecise`. ES2026 library APIs are separate from grammar syntax.
The TypeScript/Node `await` newline disagreement and Node's rejection of deferred
imports remain explicit. Current compiler modes do not prove old-runtime behavior;
existing historical Entmoot adapter controls remain a separate lane.

The locked generator is **0.25.8**, ABI **15**. Original source generation matches
tracked files; candidate generation is repeatable and identical across Windows
and Linux. Full JavaScript corpora pass: 116 baseline groups, 118 PR #378 groups,
122 composed groups; highlight/tag checks in the same command also pass.
The original repository tracks generated parser files. Proposed submissions must
include generated outputs according to that policy; this Entmoot tree retains
patch material rather than a vendored parser repository.

Across **16,271** pinned example files (188,375,284 bytes), baseline and composed
have 49 errors, with **zero introduced errors** and 96 named-CST changes. The
proposal-only composition has 139 errors. The 49 retained errors include
intentional malformed fixtures and real grammar gaps, not a success denominator.
TypeScript's original three exact exclusions remain visible: two matched files
contain errors, leaving 47 included errors. No proposal-expanded exclusion list
is used. Frontend checks accept 16,251 files and reject 20; they do not type-check
or execute foreign projects. The JavaScript workflow's `examples/*.js`/`*.jsx`
globs do not recurse into its npm clone; our recursive 1,553-file npm comparison
is explicitly additional scope.

Available composed Linux Rust/Go/Python/Node/lint/CMake lanes pass. Available
Windows lanes pass except CGo: this host lacks GCC. macOS, Swift and WebAssembly
execution are unrun; the upstream platform matrix is Windows/Linux/macOS and all
five language bindings are enabled. The original Go v0.24.0 pointer-only smoke
test hides an ABI mismatch. [Preparation](prepare_bindings.py) applies separately
retained Go-generated v0.25.0 metadata (Go 1.23 floor), then an actual
[parser installation and parse](reserved-properties/go_binding_oracle.go) passes.
Original failures remain retained. TypeScript additionally needs a repaired Cargo
lock, documented by its [owner](../typescript/README.md).

Five fresh-parser and five unchanged-tree samples, each 25 parses, use four fixed
files per dialect. This is parser cost, not OS cold-cache or pipeline performance.
JavaScript's npm.js fresh median rises 2.5% outside the observed sample ranges;
other JS ranges overlap. TypeScript/TSX unchanged-tree increases require a cost
review before adoption; see the receipt for absolute samples and the
[TypeScript owner](../typescript/README.md). `install.js` was excluded from the
unchanged-CST microbenchmark before timing because comment attachment changes;
`config.js` satisfies the predeclared unchanged-CST requirement.

## Reproduce and adopt

Use [baseline.json](baseline.json), the pinned
[proposal](reserved-properties/proposal.json) and
[composition](reserved-properties/composed.json) with `../materialize_baseline.py`.
Supply fresh `--grammar` and `--receipt` paths, exact `--cli`, and an explicit
Node 22 executable for locked npm preparation. The driver isolates parser caches,
compares tracked/regenerated sources, runs generation twice, the full
`tree-sitter test --rebuild` corpus and each parser build. Both TS dialects are
built independently. `prepare_bindings.py` requires the matching materialization
receipt before applying qualified metadata. `../qualify_windows.py` and
`../qualify_linux.py --prepared-npm` run enabled bindings; supply `--go-oracle`.
Use the exact recorded command arguments and tool digests in the receipt.

`qualify_features.py` requires `--libraries` (language → variant → library path),
`--node`, `--typescript6`, `--typescript7` and a fresh `--out`. `qualify_examples.py`
uses the same libraries, Node, `--typescript6` pointing to `lib/typescript.js`,
and a fresh `--out`. `../measure_parser.py` takes `--language`, `--baseline`,
`--candidate`, and a fresh `--out`. Source objects come from the checkout index;
foreign source trees remain unchanged. Full raw evidence is local; retained
hashes do not imply retained binary/source bytes in Git.

Adoption remains temporary and qualified. Remove supplements when upstream
passes the same controls; reassess at dependency updates and the repository move.
Maintainer acceptance and human audit closure remain external outcomes.
