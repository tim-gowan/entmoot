# Explicit positive literal patterns

Separate grammar gap discovered by the pinned CPython example corpus. Python
3.15 accepts unary plus in literal patterns, including mapping keys and the real
part of complex patterns. Python 3.13 rejects those new forms. The
[language grammar](https://docs.python.org/3.15/reference/grammar.html) explains
the syntax; the executable compiler/runtime controls establish the checked behavior.

The [minimal patch](grammar.patch) changes two sign choices and adds an
[independently authored corpus](fixtures/corpus.txt). It uses the same upstream
baseline, MIT license, CLI 0.25.9 and ABI 15 as the [grammar owner](../README.md).
No separate existing upstream proposal is pinned for this gap. Recheck upstream
issues and contribution instructions before a human submits the packet.

Expected CSTs retain `case_pattern`, `complex_pattern`, `dict_pattern` and
`list_pattern`; no semantic node family or type checker is introduced. The sign
is an anonymous token. A mapping key's hidden signed-number rule assigns `key`
to both the sign and numeric child: consumers must preserve the whole key span,
not read only its first field child. Following function declarations and their
exact source coordinates remain intact.

The [oracle driver](run_oracles.py) independently checks 22 controls under actual
Python 3.15.0 and 3.13 interpreters, with runtime assertions for accepted cases.
It checks scalar/complex/nested patterns, mapping fields, Unicode/CRLF markers,
following declarations, malformed signs and edit recovery. Real/imaginary-number
restrictions remain compiler outcomes, as they were for minus-prefixed patterns.
They do not become invented Tree-sitter type checking.

```text
python tools/tree-sitter-upstream/python/positive-patterns/run_oracles.py --python <python-3.15.0> --historical-python <python-3.13> --baseline <baseline-library> --candidate <patched-library> --out <new-receipt.json>
```

Apply this patch to either the pinned baseline or the lazy-import/comprehension
candidate in a fresh scratch build. Always use `tree-sitter test --rebuild`:
the CLI cache can otherwise select another scratch variant with the same grammar
name. Do not run corpus updates to create expected trees. Generate upstream's
tracked C/JSON files with the pinned CLI when preparing a human submission.

[Validation](qualification_2026-10-10.json) compares both patch applications and
the combined example scope. It qualifies available Windows and Linux lanes,
with remaining platforms and native/visitor adoption explicitly unrun. The patch
recovers the last compiler-accepted example failure in `Lib/test/test_patma.py`;
the two remaining errors in the complete example scope are intentional invalid
programs. No production grammar pin changes here.

The combined parser keeps 2,722 states, 278 node kinds and a 452,096-byte Windows
library; generated C grows by 1,520 bytes. All four unchanged-CST microbenchmark
files have overlapping baseline/candidate sample ranges across five fresh-parser
and five unchanged-tree samples. These measurements are descriptive and do not
qualify pipeline performance, memory or storage.

Remove a temporary patch when upstream supplies qualified equivalent behavior;
otherwise reassess at every dependency update and the repository move. Upstream
acceptance is external. Human review must cover the anonymous sign/key-field
contract, historical controls, generated diffs and every unrun lane before
a production grammar update.
