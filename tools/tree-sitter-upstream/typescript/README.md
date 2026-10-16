# tree-sitter-typescript and TSX

Upstream: [tree-sitter-typescript](https://github.com/tree-sitter/tree-sitter-typescript),
MIT. Original 0.23.2 commit `75b3874edb2dc714fb1fd77a32013d0f8699989f` is Entmoot's
current crate baseline. The [shared October 10 receipt](../javascript/qualification_2026-10-10.json)
qualifies both dialects without production adoption. The original release uses
JavaScript 0.23.1, locked generator 0.24.4 and ABI 14. It regenerates exactly.

[PR #357](javascript-025/README.md), pinned at
`6f7fa0e3ef0e95f26c46ad32f970a77f851f40de`, upgrades the shared JavaScript grammar
to 0.25.0 and removes the unsound `using` assignment prefix. Its source does not
contain every fix described in its PR body. Composition separately pins
[JavaScript PR #378](../javascript/reserved-properties/README.md),
[variance PR #361](variance-annotations/README.md),
[module declarations](module-declarations/README.md), and the JavaScript resource,
ASI and reserved-export supplements. Local TSX conflict/member-name and type-query
corrections are explicit patches under `javascript-025/`.

Tree-sitter observes variance annotations but never checks variance; compilers
own that behavior. Deferred imports require namespace syntax, type-only namespace
exports retain their source clause, and default bindings named `defer` remain
ordinary. `typeof import(...).default` and reserved export names remain valid.
These rules preserve declarations following modern forms and are tested in both
dialects. Entmoot's strict language/error-tree boundary remains independent.

## Evidence and limits

The [shared owner](../javascript/README.md) records exact compiler/CST/example
counts. Actual TypeScript **6.0.3** and native **7.0.2** independently compile the
same authored sources; Node executes eligible output. The Program adapter stays
pinned to 6.0.3 because [TypeScript 7 has no compiler API](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/).
No receipt here qualifies a native producer. Historical adapter controls
4.9.5/5.0.4/5.5.4/5.9.3 remain separate from current target-mode tests.

The proposal declares/locks CLI **0.25.8**, ABI **15**, but its tracked parser
files were generated with **0.26.6**. The receipt preserves that mismatch;
qualification uses repeatable 0.25.8 generation on Windows and Linux. Original
corpus output has 112 passed tests; proposal-only has 115 parses and the composed
package has 122. Both dialects, including the Flow alias of the TSX root, are
covered without duplicate generation. Shared tests prove a TSX-only parser
change invalidates the package fingerprint and mixed ABIs fail.

The original Cargo lock is incompatible with the proposal's manifest.
[Retained tool-generated metadata](javascript-025/build-locks/) locks the already
declared development runtime to tree-sitter 0.25.10 and cc 1.2.57. Actual Go parser
installation exposes a second mismatch: v0.24.0 accepts ABI 14 only. The qualified
v0.25.0 metadata requires Go 1.23. A guarded
[preparation command](../javascript/prepare_bindings.py) verifies the profile,
materialization, generated files and original metadata before mutation. Original
locked Cargo/Go failures stay visible; no foreign checkout is rewritten.

The corrected Linux Rust/Go/Python/Node/lint/CMake lanes pass; Windows passes
except unavailable GCC/CGo. Baseline lint errors remain historical evidence.
Upstream enables Rust, Node, Python, Go and Swift on Windows/Linux/macOS. Swift,
macOS and WebAssembly execution are unrun. Tracked generated-file policy,
`npm ci --ignore-scripts`, generation, full `tree-sitter test --rebuild`, bindings
and lint follow the pinned repository's recipes. No grammar-specific
CONTRIBUTING/AGENTS file was present; recheck before human submission. The
[core AI/contribution policy](../README.md) is separately scoped.

VS Code, Redux and GitHub Desktop are pinned examples. Their original three
exact exclusions remain unchanged; PR #357's expanded list must not hide the
90 additional proposal-only parse errors in this corpus. The supplemented
candidate restores zero-new-error parity while retaining all 49 existing
cross-family errors. Ninety-six named CST differences require visitor review;
comments are among them. Parse-only TypeScript frontend acceptance does not
establish a valid project or binding.

Five fresh and five unchanged-tree samples per fixed source show no broad speed
improvement. TypeScript unchanged-tree medians rise 24–124%, and TSX rises
1–38%, outside observed sample ranges. These are small parser-only intervals,
not pipeline costs or a statistical bound. The unchanged-CST bar rejects a
proposal-only TypeScript comparison because that parser fails `async.ts`; no
partial result is promoted. A separate valid TSX comparison observes similar
unchanged-tree increases in the upstream proposal itself; this is supporting
evidence, not an attribution proof for TypeScript or every supplement. Grammar
correctness costs remain separate from future optimization.

## Adoption work

Reproduction commands and artifact retention rules are in the
[shared owner](../javascript/README.md#reproduce-and-adopt). Temporary adoption
must be removed when equivalent upstream behavior passes these controls, and
reassessed at dependency updates and the repository move. No external submission
or maintainer acceptance is claimed.
