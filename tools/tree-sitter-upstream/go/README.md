# tree-sitter-go

Upstream: <https://github.com/tree-sitter/tree-sitter-go>. License: MIT.
Entmoot crate pin: **0.25.0** (Cargo.lock is authoritative).
Inspected upstream revision: `2346a3ab1bb3857b48b29d779a1ef9799a248cd7`. The upstream workflow metadata below is
research at the October 10, 2026 cutoff, not a claim that the installed crate has
already passed that workflow against new syntax.

Locked generator: tree-sitter-cli **0.25.8**; declaration `^0.25.8`. Original
upstream tracks generated parser sources. Both measured parsers use ABI **15**.
PR #198's parser matches regenerated output except for its first-line generator
banner; exact byte equality remains false in the receipt. Repeated generation
is deterministic, and Windows/Linux generated C and JSON match exactly.
Platform matrix: Windows, Linux, macOS. Enabled bindings: Rust, Node, Python, Go
and Swift. Example corpus: Go and Moby. The workflow's 28 invalid-file patterns
classify expected failures; they are not exclusions. Its example job is
`continue-on-error`, so a successful workflow does not qualify every source.
Run `npm ci --ignore-scripts`, the repository's lint/generation recipe,
`tree-sitter test`, every enabled binding check, and its example workflow. Inspect
package scripts before choosing a generation command; different pins use different
recipes. Use `test --rebuild` to prevent another scratch variant's parser cache
from contaminating the result.

No grammar-specific CONTRIBUTING/AGENTS file was present in the inspected original
repository. Review its README, package scripts and .github/workflows again before
human submission. Core's maintainer-only contribution rule is separately scoped in
the [parent record](../README.md); it is not automatically a rule for this grammar.

The [generic-method packet](generic-methods/README.md) qualifies PR #198's
  declaration changes with 17 independently authored controls, full corpus,
available bindings, 22,524 example files, recovery and bounded parser measurements.
It also records the method-value instantiation gap addressed by the later
expression packet below.

Current ownership and residuals:

- **Existing upstream fix:** PR #198 preserves generic method declarations and
  following declarations. The declaration-only baseline meets 4/17 CST controls;
  the proposal meets 17/17. Both pass all 17 actual Go compiler/runtime or
  historical-mode expectations.
- **Grammar gaps:** some instantiated method values still fail under #198.
  Go 1.26 `new(expression)` and historical shadowed `new`/`make` are covered by
  existing [PR #193](https://github.com/tree-sitter/tree-sitter-go/pull/193),
  overlapping [PR #190](https://github.com/tree-sitter/tree-sitter-go/pull/190),
  and closed, unmerged [PR #199](https://github.com/tree-sitter/tree-sitter-go/pull/199).
  The [expression packet](new-expressions/README.md) now qualifies #193 with a
  minimal shadowing supplement adapted from #190, and a separate method-value
  supplement to #198. The composition passes 56 focused controls and recovers 14
further example parses without new grammar errors. Remaining errors stay explicit.
- **Entmoot visitor work:** production uses the qualified generated bundle
  ([generated grammar](../../../crates/entmoot-core/src/grammars.rs)).
  Chained, indexed and parenthesized-receiver callee expressions exposed an
  Entmoot visitor omission; function-literal bodies also needed distinct
  callable ownership. The existing grammar parses these forms. Shared
  `go114-callee-*` revision controls and the
  [native Go tests](../../../crates/entmoot-engine/tests/callsites_go.rs)
  cover inventory, uncertainty and runtime behavior. No additional upstream
  grammar patch is needed for this fix. Syntax spelling must never establish
  that a `new` or `make` call targets the predeclared builtin.
- **Native-provider limitation:** this packet does not run the Entmoot Go
  resolver. Current Go with `-lang` controls is not an old-runtime matrix.

Production bundle adoption remains temporary: remove these patches when upstream
supplies qualified equivalent behavior, and reassess at dependency updates and
the repository move. Maintainer acceptance is an external outcome.
