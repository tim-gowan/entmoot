# tree-sitter-cpp

Upstream: <https://github.com/tree-sitter/tree-sitter-cpp>. License: MIT.
Entmoot crate pin: **0.23.4** (Cargo.lock is authoritative).
Pinned upstream revision: `c009222808634c1014f82438d4883753516a2c24`.
The shared [C-family packet](../c/qualification_2026-10-10.json) records
October 10, 2026 qualification of the baseline and narrow local supplements.
**C++ remains strictly experimental.**

Locked generator: **0.26.5** (declaration `^0.26`), measured ABI **15**. Original
tracked generated files match regeneration. Both final platform runs pass all
198 corpus groups, repeat generation exactly and produce identical C/JSON bytes.
Upstream tracks generated files; its platform matrix is Windows/Linux/macOS,
with Rust/Python/Go/Swift bindings enabled and Node disabled. The source grammar
and query dependency remains **tree-sitter-c 0.24.1**. Substituting the qualified
C candidate fails generation and is explicitly unadopted.

[If consteval](if-consteval/README.md) adapts the narrow existing PR #362;
[pack indexing](pack-indexing/README.md) selects only that portion of broader
PR #375. Independent fields, type/expression distinctions, comment-separated
tokens and historical variadic controls expose errors hidden by parse success.
No other broad PR #375 changes are adopted by this packet.

All 39 [authored controls](cases.py) meet actual Clang 23.1.3 acceptance/rejection
expectations and candidate CST expectations, versus 32 baseline CST expectations.
All 32 eligible programs execute. All 36 error-free candidate trees pass full
incremental damage/repair parity; three expected error trees remain explicitly
unqualified for recovery. An actual GCC 11.4 lane additionally exercises older
language forms. C++26 pack indexing remains a draft experimental observation;
neither parser nor compiler acceptance implies stable-standard support, ABI,
template closure, CompDB fidelity or Entmoot semantic binding.

The complete literal `examples/*` scope is two regular Git objects, 13,280
bytes, with no exclusions, no parse errors and identical named CSTs. No foreign
example is compiled or executed. Generated C grows from 25,857,209 to 26,276,947
bytes and Windows library size from 5,573,632 to 5,673,984 bytes; states grow
11,734→11,864. Five fresh-parser and five unchanged-tree samples over each file
have overlapping ranges in all four comparisons. This small parser measurement
is neither an optimization claim nor a pipeline/storage/capacity benchmark.

The original Cargo lock pins incompatible Tree-sitter 0.24.4 against the
manifest's 0.26 requirement. The retained tool-generated
[lock](build-locks/Cargo.lock) pins runtime 0.26.5 and its resolved dependency
closure. The original Go 0.24.0 pointer smoke test passes, but
[actual installation](go_binding_oracle.go) rejects ABI 15. Qualified
[Go metadata](build-locks/go.mod) pins runtime 0.25.0 and Go 1.23.
[Preparation](../c/prepare_bindings.py) validates all source/parser fences
before applying metadata. Linux Rust/Go/Python/lint/CMake and the real Go parser
check pass. Windows passes except CGo, which lacks GCC; macOS/Swift and WebAssembly
execution are unrun. Original failures stay separate from repaired results.

No grammar-specific CONTRIBUTING/AGENTS file was present in the inspected original
repository. Review its README, package scripts and .github/workflows again before
human submission. Core's maintainer-only contribution rule is separately scoped in
the [parent record](../README.md); it is not automatically a rule for this grammar.

Use the [C owner commands](../c/README.md) with `--language cpp`,
`--family cpp` and `if-consteval/composed.json`. The same authored feature driver
and independent compiler/runtime lanes serve both grammars. Keep fresh output
names and isolated parser caches. Bindings use `--skip-node-tests` and this
grammar's Go oracle. Upstream example selection and all tool/source digests are
retained in the packet; a digest is not a claim to retain a binary.

No patch/fork is adopted by this record. Any temporary adoption must be removed
when upstream supplies qualified equivalent behavior, or reassessed at dependency
updates and the repository move. Maintainer acceptance is an external outcome.
