# Expressions in `new` and ordinary shadowed names

This packet qualifies existing [PR #193](https://github.com/tree-sitter/tree-sitter-go/pull/193),
head `5a6af13a0a5b45bc76cac289c783b315b2b74e13`, against the common baseline
`2346a3ab1bb3857b48b29d779a1ef9799a248cd7`. Both are MIT-licensed upstream
material. [proposal.json](proposal.json) records the proposal pin and overlapping
work. PR #193 is open; [PR #199](https://github.com/tree-sitter/tree-sitter-go/pull/199)
closed without merging. This is a local human-review packet, not another submission.

[Go 1.26](https://go.dev/doc/go1.26#language) permits an expression as the argument
of the predeclared `new`. That language change does not make `new` or `make`
keywords. The [authored cases](cases.py) cover expression values, evaluation once,
historical allocation forms, generic and ordinary shadowed functions, indexed
and sliced variables, negative type/arity cases and historical source modes.
All 29 compiler/runtime expectations and CST controls pass with the supplement.
The compiler is Go 1.27.2 with explicit `-lang`; no old compiler installation is
implied. Native Entmoot resolver qualification remains separate and unrun.

The full example comparison found a regression in unmodified #193: its optional
type arguments commit to a call parse for `new[0]` and `new[low:high]`. Six
independently compiled Go 1.14 controls reproduce indexing/slicing/calling a
shadowed variable or parameter. [shadowing.patch](shadowing.patch) adapts the
hidden callee rule, explicit GLR conflict and dynamic preference from existing
[PR #190](https://github.com/tree-sitter/tree-sitter-go/pull/190), pinned at
`97cacdcb9e1e699f7b3dfed21386da60ed6e63cd`. It preserves #193's generic-call CST
while allowing ordinary index and slice expressions. The first attempt omitted
the dynamic preference and failed an existing upstream CST group; the retained
patch passes that group unchanged. No golden was regenerated from parser output.

Expected CSTs retain `call_expression.function`, optional `type_arguments`, and
`arguments`; ordinary indexing and slicing retain their `operand` fields.
[corpus.txt](fixtures/corpus.txt) adds a historical control. A grammar field named
`type`, or a callee spelled `new`, is syntax evidence only. Scope/type-aware
binding must determine what that occurrence actually denotes.

The [combined evidence receipt](qualification_2026-10-10.json) also qualifies
composition with PR #198 and the local method-value supplement. All 56 authored
controls pass: 17 declarations, 29 allocation/shadowing and ten method values.
The supplemented #193 corpus has 70 groups; the full composition has 72. Both
use CLI 0.25.8, ABI 15 and forced parser rebuilds. Available platform checks and
their failures are recorded separately; the [Go owner](../README.md) lists the
remaining platform gates.

Across the same 22,524 pinned example files, the composition reduces errors from
156 to 142, recovers 14 parses, changes 22 named-node CST signatures and introduces
no new grammar errors. The remaining 142 errors are not qualified away: 138 match
upstream invalid-file patterns, and four do not. Those patterns are classifications,
not proof that a program is invalid. All four unmarked errors also fail Go's
syntax parser; syntax-parser acceptance is distinct from compilation/execution.
Historical and semantic negatives remain visible even when a grammar accepts
their structure.

Of the 142 remaining errors, 37 are accepted by Go's syntax parser and match an
upstream invalid-file pattern. Their semantic validity is not established by
either observation; they remain a separate triage set. The other 105 are rejected
by that syntax oracle. None is converted into a negative binding conclusion.

Five fresh-parser and five unchanged-tree timing samples on each of four pinned,
unchanged-CST files have overlapping ranges in all comparisons. That small,
variable sample is not a broad performance gate. Relative to #198, the composed
parser adds 104,390 generated C bytes, 12,800 Windows library bytes and 74 states.
These are measured costs of syntax corrections, not an optimization result.

Build only fresh scratch materializations. [materialize.py](../materialize.py)
verifies source pins, uses an explicit repository-relative patch destination,
checks forward/reverse applicability and changed-file digests, then runs repeated
generation and the full corpus. A preliminary composition silently skipped the
extended Git diff when applied from a scratch subdirectory; the original
declaration controls caught it. Its failed receipts are excluded from qualification.

```text
python tools/tree-sitter-upstream/go/materialize.py --variant combined --grammar <fresh-scratch-directory> --cli <tree-sitter-0.25.8> --receipt <fresh-json>
python tools/tree-sitter-upstream/go/qualify_expressions.py --feature new-expressions --baseline <original-PR193-library> --candidate <supplemented-library> --go <go1.27.2> --cache <scratch-cache> --out <fresh-directory>
```

Use `--feature declarations` and `--feature method-values` for the other controls.
The original 17 declaration shapes are shared with the existing qualification
driver; production output does not author expected results. Upstream generated
files remain tracked and must accompany a human submission according to the
upstream policy; this repository retains minimal patches and evidence only.

Production selection is recorded in the bundle manifest. Remove the supplement
when a qualified upstream release includes equivalent behavior, or reassess it
at dependency updates and the repository move. Future parser changes use focused occurrence controls; native qualification is a separate lane. Parser-only
cost measurements do not qualify pipeline, storage or process-tree memory costs.
