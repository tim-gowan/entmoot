# Deferred imports and type-only namespace exports

The pinned TypeScript grammar rejects compiler-accepted `import defer * as …`
(introduced in TypeScript 5.9) and `export type * as …` (TypeScript 5.0).
Baseline/compiler evidence is produced by the
[shared qualification driver](../../javascript/qualify_features.py), using the
same authored source bytes for both dialects and actual 6.0.3/7.0.2 compilers.

The [minimal patch](grammar.patch) retains `import_statement`, `import_clause`
and `export_statement`. Deferred imports add an explicit anonymous `phase` field
and require a namespace import; ordinary default bindings named `defer` remain
ordinary imports. Type-only wildcard exports require a source clause.

Node 26.11.1 does not accept deferred-import syntax. These fixtures qualify
compiler acceptance and emitted syntax only; they do not claim runtime deferred
evaluation or an Entmoot semantic export closure. Native producers own those
semantic decisions. The [TypeScript 5.9 release notes](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-9.html)
describe the compiler support and lack of downlevel transformation.

The [shared receipt](../../javascript/qualification_2026-10-10.json) records
full corpus, both dialects, historical/default-name negatives, available
bindings, examples, incremental repair and measured cost. No production adoption.
Missing platforms, visitor/cost review and human review remain adoption gates. Remove temporary
patches after qualified equivalent upstream support; reassess at dependency
updates and the repository move. No maintainer interaction is performed here.
