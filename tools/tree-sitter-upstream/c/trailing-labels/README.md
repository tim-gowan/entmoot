# Labels at the end of a compound statement

C23 permits a label immediately before a compound statement's closing brace.
The baseline and PR #331 require a following statement. The supplement adds
trailing `labeled_statement` nodes only within a compound statement, including
multiple and attributed labels. It preserves the `label` field and the existing
attributed-statement wrapper.

[Fixture selectors](fixtures.json) point to [authored controls](../cases.py).
Clang 23.1.3 accepts the C23 controls and rejects the historical C17 control;
GCC 11.4 also rejects the latter in strict C17 mode. A version-neutral CST does
not establish that a historical compiler accepts a program. Each probe preserves
the unrelated following function and checks the exact label node and span.

The first exploratory patch modified the wrong repeated block rule and is
discarded; only the final compound-specific patch, complete corpus and named
[qualification receipt](../qualification_2026-10-10.json) support this proposal.
Replay with the [owner commands](../README.md) and `../modern-types/composed.json`.
This is local human-review material. Remove or reassess it when upstream supplies
equivalent qualified behavior, at dependency updates and at the repository move.
