# Callable parity fixtures (Wave C Segment 8+)

Generic cases for bound-expression callables. Inventory names use the **LHS binding**
(`handler` for `export const handler = () => {}` / `handler = lambda ...`). Callable
**span** covers the arrow/`function`/lambda expression node so body locations enclose.

Shared helper: `entmoot_core::analysis::bound_callable` (`bound_name_from_ancestors` +
language binder specs). Only expressions that are the binder's value (modulo transparent
wrappers like parentheses) get a name — not list/call nested expressions.

## Must-capture

| Pattern | Files |
|---------|-------|
| `export const f = () => {…}` | `export_const_arrow.ts`, `.js` |
| `const f = () => {…}` / `const f = function(){…}` | `const_arrow.js`, `const_function_expr.js` |
| `export function f()` regression | `export_function.ts` |
| `handler = lambda …` | `assigned_lambda.py`, `routes/handler.py` + `handler_lambda.sarif` |
| `def keep` regression | `def_function.py` |
| Min SARIF inside arrow/lambda body → callable attach | `handler_arrow.sarif`, `handler_lambda.sarif` |

## Must-not-capture (nested / hash-only)

| Pattern | Files |
|---------|-------|
| Nested `const nested = () => {}` inside `function outer` | `nested_const_arrow.js` |
| Nested `nested = lambda` inside `def outer` | `nested_assigned_lambda.py` |

## Nice-to (deferred)

`exports.x = () => {}`, object/class field arrows, default-export arrow, Python
`obj.fn = lambda`, unpacking LHS — out of this slice unless the same helper covers them.
Nested binders inside another callable stay hash-only (rename-stable file_hash); module-level
`const`/`export const` / simple assignment remain must-capture.

## Hash note

Promoting arrows/assignments/lambdas into the callables Merkle layer **changes**
`file_hash` for those sources. Markers on/off isolation must still hold for pure
`function_declaration` / `def` forms.
