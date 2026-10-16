# Resource declaration line terminators

Node 26.11.1 and the TypeScript compiler controls distinguish a declaration
`using resource = …` from an ordinary `using` expression followed by a line
terminator and a separate assignment. The JavaScript baseline accepts both but
misclassifies the latter as a declaration. The old TypeScript assignment-prefix
workaround has a related loss of declaration identity.

[grammar.patch](grammar.patch) adds external keyword lookahead, preserving the
keyword's original byte span. It checks line terminators inside block comments
as well as LF, CR, U+2028 and U+2029; ordinary identifiers and keyword prefixes
remain ordinary names. [typescript-scanner.patch](typescript-scanner.patch)
aligns the shared dependency's new external-token positions with TypeScript's
separate scanner. This token-order change must be adopted together.

[Corpus fixtures](fixtures/resources.txt) and the shared
[compiler/runtime controls](../cases.py) cover disposal, async disposal, both
loop forms, comments, ASI and identifiers. The separate
[ASI correction](../automatic-semicolons/README.md) is necessary for CR and
block-comment line terminators. Compilers still own declaration legality and
resource protocol semantics. The observed TypeScript/Node disagreement over
`await` followed by a newline remains explicit.

The [shared receipt](../qualification_2026-10-10.json) records full corpus, both
dialects, available bindings, examples, incremental repair and measured cost.
No production adoption. Missing platforms, cost review, visitor qualification
and human review remain the adoption bar. Remove temporary changes after qualified upstream equivalence; reassess at
dependency updates and the repository move. No external submission is made.
