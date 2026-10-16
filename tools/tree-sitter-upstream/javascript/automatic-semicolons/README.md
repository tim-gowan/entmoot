# CR and comment line terminators in ASI

The resource-declaration controls expose pre-existing scanner gaps: lone CR and
line terminators inside block comments can prevent automatic semicolon insertion
between an ordinary identifier expression and a following assignment.

[javascript.patch](javascript.patch) and [typescript.patch](typescript.patch)
scan trivia with explicit LF/CR/U+2028/U+2029 accounting, then apply each grammar's
existing continuation rules. TypeScript retains its typed-object-pattern and
function-signature controls. JavaScript retains the declaration/expression
distinction for inline-comment attachment. Superseded trivia scanners are removed.

The [authored oracle cases](../cases.py) include call, operator and property
continuations that must stay one expression, plus return boundaries. Original
corpus tests caught comment-attachment regressions during development; those
failed materializations remain diagnostic and are not final qualification.

The [shared receipt](../qualification_2026-10-10.json) records full corpus, both
TS dialects, available bindings, examples, incremental repair and measured cost.
These are syntax-scanner changes, not semantic binding. Missing platforms,
comment/CST visitor review, cost review and human review remain adoption gates.
Remove temporary changes when upstream supplies qualified equivalent behavior;
reassess at dependency updates and the repository move. Human submission only.
