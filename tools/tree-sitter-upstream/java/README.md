# tree-sitter-java

Upstream: <https://github.com/tree-sitter/tree-sitter-java>. License: MIT.
Entmoot crate pin: **0.23.5** (Cargo.lock is authoritative).
Inspected upstream revision: `e10607b45ff745f5f876bfa3e94fbcc6b44bdc11`. The upstream workflow metadata below is
research at the October 10, 2026 cutoff, not a claim that the installed crate has
already passed that workflow against new syntax.

Generator declaration: tree-sitter-cli `^0.24.4`. Original upstream tracks
generated parser sources; use its locked generator and check generation diffs.
Record parser ABI from the actual parser (`grammar_probe`); do not infer it from
the CLI version. Platform matrix: Windows, Linux, macOS. Enabled bindings:
Rust, Node, Python, Go and Swift. Example corpus: Elasticsearch, Guava, RxJava, Flink, Log4j2 and Cassandra; record continue-on-error examples honestly.
Run `npm ci --ignore-scripts`, the repository's lint/generation recipe,
`tree-sitter test`, every enabled binding check, and its example workflow. Inspect
package scripts before choosing a generation command; different pins use different
recipes. The [modern-syntax packet](modern-syntax/README.md) now records the full
local corpus/example comparison, authored compiler/CST controls, parser costs and
available platform lanes. Failed and unrun lanes remain explicit.

No grammar-specific CONTRIBUTING/AGENTS file was present in the inspected original
repository. Review its README, package scripts and .github/workflows again before
human submission. Core's maintainer-only contribution rule is separately scoped in
the [parent record](../README.md); it is not automatically a rule for this grammar.

Qualify module imports, constructor forms, patterns and CST differences against Orchard 0.5.22 individually. Original PR #233 closed without merge; this is not evidence of technical rejection. Orchard is temporary and qualified, not an ecosystem-wide migration.

No patch/fork is adopted by this record. Any temporary adoption must be removed
when upstream supplies qualified equivalent behavior, or reassessed at dependency
updates and the repository move. Maintainer acceptance is an external outcome.

## Orchard qualification boundary

Candidate: <https://codeberg.org/grammar-orchard/tree-sitter-java-orchard>, MIT,
v0.5.22, commit `7dc7faa53d68c9455c6f35f0d0f7583332127452`.
Follow [Orchard contribution guidance](https://codeberg.org/grammar-orchard/.profile/src/branch/main/CONTRIBUTING.md):
it does **not** track generated parser files; release artifacts provide them.
Do not copy the original repository's generated-file policy to an Orchard patch.

[Difftastic](https://github.com/Wilfred/difftastic/blob/master/Cargo.toml) and
[Mergiraf](https://codeberg.org/mergiraf/mergiraf/src/branch/main/Cargo.toml) provide
actual downstream adoption evidence. Multiple contributors are present. This does
not establish an ecosystem-wide migration; keep adoption temporary and qualified,
with reassessment on dependency updates and the repository move. The local packet
qualifies 33 Java 27 compiler/CST controls, including historical targets and explicit
preview separation. Production visitor/native adoption and outstanding platform
checks remain open. Orchard's shared workflow is pinned at
`475b73262c919f0b46da0a71f803345c4cd188ce`: CLI 0.26.12, ABI 14,
Linux Rust runner, generated bindings, corpus and WebAssembly build. Its metadata
enables C, Go, Node, Python, Rust and Swift bindings; smoke tests beyond the shared
workflow are recorded independently. No Java examples are configured by that
shared workflow, so this comparison also runs the original grammar's six examples.
