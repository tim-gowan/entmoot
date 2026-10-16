# Lazy imports: qualify existing PR #336

Existing proposal: <https://github.com/tree-sitter/tree-sitter-python/pull/336>.
Pinned proposal head: `6c03cce2073fcf04e583361565789d86a762be01`.
Status at research cutoff: open, not merged. This packet supplements that
proposal; it does not duplicate it or claim upstream acceptance.

The [positive fixture](fixtures/positive.py) verifies deferred import execution,
aliases, lazy `from` imports, a callable using the imported module, and `lazy` as
an ordinary local/parameter name. The [oracle driver](../run_oracles.py) separately
checks illegal function/class/try placement, star imports and future imports.
Its twelve-case [receipt](../oracle_2026-10-10_checked.json) includes exact source digests
and the isolated interpreter digest. Historical identifier controls run on the
current interpreter, not an asserted Python 3.8 runtime.

Expected CST contract: existing `import_statement` / `import_from_statement`
nodes retain their import `name`, `module_name` and alias fields; the lazy token
must not absorb a following declaration. Ordinary `lazy` uses remain `identifier`.
Entmoot must preserve lazy/eager spelling in structural identity while inventorying
literal module syntax. Deferred execution does not establish runtime availability
or make a lexical name uniquely bound.

Compiler restrictions on where lazy imports execute must not turn the grammar
into a scope/type checker. Record grammar and compiler negatives independently.

The [pinned proposal](proposal.json) and [qualification receipt](../qualification_2026-10-10.json)
now compare baseline/proposal/combined CSTs, preserving import name, module and
alias fields under `lazy_import_statement`, ordinary identifiers and following
declarations. Compiler-rejected placement remains independently recorded from
grammar acceptance. Full Windows corpus/query checks, incremental recovery and
binding lanes pass; the combined example corpus retains the upstream historical
examples and explicit exclusions. Remaining platform/binding gates and Entmoot
visitor/native-provider controls stay open in the [owner record](../README.md).
No patch is adopted until those gates pass.
