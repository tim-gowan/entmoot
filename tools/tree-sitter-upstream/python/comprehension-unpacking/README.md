# Comprehension unpacking

Separate coupled change from lazy imports. Python 3.15 permits unpacking in
list/set/dictionary comprehensions and generator expressions. This is a
demonstrated compiler feature; the [minimal patch](grammar.patch) has local
qualification evidence, while Entmoot adoption remains open.

The [runtime fixture](fixtures/positive.py) checks all four forms, nesting,
filters, async iteration, overwriting dictionary keys and historical ordinary
comprehensions. [Negative oracles](../run_oracles.py) check illegal double-unpack
list and generator forms. [Retained receipt](../oracle_2026-10-10_checked.json).

Expected CST contract: preserve `list_comprehension`, `set_comprehension`,
`dictionary_comprehension` and `generator_expression`, with an explicit unpacking
body and unchanged `for_in_clause` / conditional children. Keep induction-variable
scope, expression call spans and following declarations intact. Dictionary `**`
must not be accepted as list/set/generator `*`. Include async and nested variants
in both full parses and incremental edit recovery.

The baseline already accepts simple list/set/generator unpacking, but rejects
dictionary unpacking and misplaces the star inside member/call CSTs. The patch
removes the globally permissive starred-pattern alias and permits unpacking in
the actual expression positions. It preserves starred returns, yields, loop
iterables, subscripts, f-strings and variadic annotations. The last form was a
real regression found by the full CPython example run and is now a narrow control.

Three historical upstream golden trees change: `[*a.b]`, `[*a[b].c]`, and `[*a()]`.
Their corrected star wraps the entire operand, independently checked by
[CPython AST/runtime assertions](fixtures/historical_unpacking.py), including
the current compiler's Python 3.8 AST mode. That mode is not a Python 3.8 runtime.
[Call-body oracles](fixtures/call_bodies.py) qualify the same ownership in new
comprehensions. The [authored corpus](fixtures/corpus.txt) covers all four forms,
nesting, async calls and a following declaration. Do not use parser-generated
trees to update these expectations.

The [receipt](../qualification_2026-10-10.json) demonstrates improved CST fields,
full upstream corpus/query checks, complete examples, incremental recovery and
Windows binding checks. It records other platforms separately. Open gates are
the remaining platform/binding matrix, the explicitly positive literal-pattern
gap, and Entmoot visitor/occurrence/native-provider controls. This is a local
human-review packet, not an upstream-ready or accepted proposal. A submission
must regenerate the upstream-tracked parser C/JSON files with the pinned generator;
those large generated files are deliberately absent from this patch packet.
