# Resolver adapters

In-tree Go / Roslyn / Java (javac jar) / Clang Resolver adapter sources for Entmoot.

| Family | Path | Env |
|--------|------|-----|
| Go | `go/` | `ENTMOOT_GO_ADAPTER` |
| C# | `roslyn/` | `ENTMOOT_ROSLYN_ADAPTER` |
| Java | `java/` (javac Trees only) | `ENTMOOT_JAVA_ADAPTER` |
| C/C++ | `clang/` | `ENTMOOT_CLANG_ADAPTER` (+ `LIBCLANG_PATH`) |

**Catalog:** [`catalog.toml`](catalog.toml) · **Floors:** [`floors.toml`](floors.toml)
**Setup:** [`docs/runbooks/resolver-adapters.md`](../../docs/runbooks/resolver-adapters.md)
**Build outputs:** `.tmp/adapter-build/resolver-adapters/` via
`python scripts/build_resolver_adapters.py` (never invoked by `init`/`bind`).
After an explicit build, `entmoot init` records discovery in
`.entmoot/adapter-profile.json`; `entmoot status` reports readiness. `bind`
requires native for inventory languages unless `--structural-only`.

Program adapters (Pyright / TypeScript) stay under `.tmp/adapter-build/*-program/` with
their own build scripts. Rust Analyzer and Oxc remain in-process Cargo features.

Reference lock-selection producer (npm): [`../lock-instance-providers/npm.cjs`](../lock-instance-providers/npm.cjs).
