# Resolver adapters (setup)

Operator guide for Entmoot **Resolver adapters**: in-process (Rust Analyzer, Oxc),
Program adapters (Pyright, TypeScript), and process sidecars (Go, Roslyn, Java
javac jar, Clang). Entmoot does **not** install JDK, Node, .NET, Go, or LLVM
([ownership](../architecture/ARCHITECTURE.md)).
`init` / `bind` never build or download adapters.
Durable family execution uses the same pipelines through a matching
`entmoot-producer` helper; see [agent runtimes](agent-runtimes.md) for host isolation,
environment assertions, lease controls and restart boundaries.

The default structural build uses the verified generated grammar bundle under
`crates/entmoot-core/generated/grammars/` and its existing C build toolchain. It
does not invoke Node, upstream grammar generators, or language-specific SDKs.
Adapter target-mode qualification remains separate from grammar syntax acceptance,
including TypeScript 7's lack of a compiler API.

**Product name:** Resolver adapters (ONTOLOGY). **Env shorthand:** `ENTMOOT_<FAMILY>_ADAPTER`.

Catalog (paths + env): [`tools/resolver-adapters/catalog.toml`](../../tools/resolver-adapters/catalog.toml).
Native floors (pins + language surfaces): [`tools/resolver-adapters/floors.toml`](../../tools/resolver-adapters/floors.toml).
Readiness: `entmoot status` (adapter profile in the selected workspace volume after `init`).
Maintainer terms: [AGENTS vocabulary](../../AGENTS.md#native-tools-and-vocabulary).
Bind/canon ownership: [architecture](../architecture/ARCHITECTURE.md#canon-and-clone-operations).

Java family id is `java` / `ENTMOOT_JAVA_ADAPTER` (javac Trees only).
Operators use `entmoot init`; the hidden `index` command is a low-level structural write.
Keep virtual environments, generated code and native build logs under `.tmp/<task>/`.
For C/C++ native (clang), supply an existing `compile_commands.json`; Entmoot does
not generate CompDBs. C++ remains experimental.

## Entmoot native floors

Qualified native evidence is limited to these adapter pins and language surfaces.
Structural inventory may accept broader syntax; missing or newer tools stay
unknown/partial. Host executable versions in receipts identify the machine, not
an automatic language floor. Machine-readable copy: `floors.toml`.

| Family | Adapter pin | Language / tool floor | Closed dispatch |
|--------|-------------|----------------------|-----------------|
| Rust | `ra_ap_*` **0.0.358** (host **rustc 1.98.0**) | Selected Cargo edition/cfg + `rust-src` | Free functions, inherent methods, associated (UFCS) |
| JS/TS Oxc | **oxc 0.153.0** | Lexical JS/TS/TSX; no type checker | Unique identifier callees only (member stays open) |
| JS/TS Program | **typescript@6.0.3** (`typescript.program/1`) | Program API **4.9.5–6.0.3**; **7.x unqualified** | Unique function signatures (call signatures stay open) |
| Python | **pyright 1.1.414** | Syntax targets **3.8–3.14** | Unique function and member (decorator stays open) |
| Go | **golang.org/x/tools v0.52.0** (Go **1.26+** module) | Selected module/tags/GOOS/GOARCH | Package free functions and concrete methods |
| C# | **Roslyn 4.14.0** / .NET 9 | Project language version | Static/sealed/base/partial/non-virtual instance |
| Java | **javac Trees** (family `java`) | JDK **17+** host; `--release` **8/11/17/21/25** | Static/private/final/super |
| C/C++ | **libclang 18.1.1** | One `compile_commands.json` config | Non-virtual functions/methods (virtual CXX stays open) |

TypeScript Program cannot move to 7.x until a qualified Program API exists; keep
6.0.3 as the typed ceiling. Migration onto the TypeScript **7.1** native
programmatic API (new adapter pin) is deferred in [PLANNING](../../PLANNING.md).
Oxc remains the default lexical JS/TS producer.

## Release operator path (host adapters)

Build adapters on the host, then let `init` record them in the workspace profile.
Entmoot does **not** download SDKs and does **not** read `.env`.

```text
python scripts/build_resolver_adapters.py
cargo build -p entmoot --features producer
entmoot init
entmoot status
entmoot bind
```

`init` writes the selected-volume adapter profile from discovery (catalog /
`ENTMOOT_ADAPTERS_ROOT` / process overrides). `bind` requires native for inventory
languages by default; use `--structural-only` to waive. Optional shell exports:
[`.env.example`](../../.env.example).

For a sandbox-accessible local installation, select a gitignored output with the
builders' `--out` flags and pass an explicit `--data-dir` / `ENTMOOT_DATA_DIR`.
Keep caches and fixture TEMP writable, outside analyzed source; Cargo fixtures
inside this checkout need the workspace-excluded `.tmp/` directory. File access
does not establish runtime permission: validate native controls in the actual
sandbox, including [Roslyn's MSBuild communication requirement](../../tools/resolver-adapters/roslyn/README.md#request).

Docker Go (`ENTMOOT_GO_VIA_DOCKER=1`) and Curl CompDB Docker helpers remain
**optional overrides**, not the default path.

**Do not set `ENTMOOT_TYPESCRIPT_ADAPTER` for default P1 JS/TS** — leave unset so
in-process Oxc is selected. TypeScript Program is opt-in only.

For clone analysis, explicitly run `entmoot bind --experimental canon` after
inventory. For typed JS/TS/TSX canon, select the TypeScript Program adapter;
Oxc lexical bindings alone do not establish scalar types. Configured native
adapters export bounded declaration/expression type sites during this operation.
Missing tools leave explicit partial forms. `assess clones` reads the retained
graph; request `{"canonical":true}` to select complete scalar canon candidates.
The default syntax/alpha/parameterized candidates also require this graph.
Neither mode invokes an SMT solver or claims behavioral equivalence.

## Shared discovery

Adapter terminology is normalized before resolver facts enter shared operations.
The Rust contracts own category names; attributed mechanism values can extend
without adding language-specific variants. A supplied indirect-call mechanism
does not establish a closed target set. Unknown provider reasons remain explicit
unresolved bindings, and unsupported lowering does not establish dynamic dispatch.
Missing symbols and overload candidates stay unresolved or ambiguous; only
native evidence of delegates, open virtual/interface dispatch, function
pointers or equivalent mechanisms may use `indirect_call` / `dynamic_dispatch`.
Provider-native symbols, types, versions and build settings retain their native
meaning in the existing attributed fields and input context.

Structural inventory, cached scans and snapshot validation honor the selected
root's `.gitignore` and nested rules by default, alongside fixed cache exclusions.
Directories are pruned before reading; coverage records the excluded scope.
Ancestor and machine-global ignore settings do not change an explicitly selected
root. Ignore files remain snapshot inputs so policy edits invalidate the view.

Discovery first honors process or nonempty config overrides, then a recorded
profile path if its file is present, before searching the default candidates below.
Init refreshes readiness; it does not download or qualify an adapter.

| Step | Behavior |
|------|----------|
| Process `ENTMOOT_*` when set | Exclusive path (canonicalize; relative → CWD); empty/missing → `invalid_environment` (no fallthrough) |
| else workspace/global `config.json` `env` slot | Same exclusive rules when non-empty |
| else recorded profile | Existing adapter path for this workspace |
| else analyzed checkout | Conventional `.tmp/adapter-build/...` under the checkout |
| else PATH basename | Installed CLI names when present |
| else `ENTMOOT_ADAPTERS_ROOT` / `~/.entmoot/adapters/` | Catalog relatives for sidecars and programs |

Readiness distinguishes process and configuration selections. Both use the same
exclusive path as runtime resolution, including invalid choices.

When multiple host tool versions exist, selection follows ecosystem priority
(`JAVA_HOME`, `DOTNET_ROOT`, active venv, rustup default, then PATH).

`bind` requires native by default; useful partial dispatch evidence can succeed.
Skipped/failed production, invalid environments and exhausted budgets fail
required-native bind. Use `--structural-only` to skip native producers.

Native production defaults to a one-hour wall budget **per producer** across
`init`, `bind` and standalone producer commands. Use `--timeout-seconds` for a
shorter run; the total multi-family operation can take longer than one hour.
Receipts distinguish inventoried and observed calls and count explicit provider
call gaps. Typecheckers and lexical adapters contribute observations without a
compilation requirement. Dynamic/indirect dispatch remains visible alongside
retained declaration targets; partial dispatch alone does not fail bind.
Environment failure, unavailable production and budget exhaustion do under
default required-native bind.

Launchers use explicit `ENTMOOT_JAVA`, `ENTMOOT_NODE`, `ENTMOOT_DOTNET` or
`ENTMOOT_PYTHON_EXECUTABLE` paths first. Java then uses `JAVA_HOME/bin/java`,
.NET uses `DOTNET_ROOT/dotnet`, and otherwise PATH is searched. A broken explicit
choice fails without fallback. Receipts record selected executable, selection
source and observed version; these identify the host, not a language qualification.

Native receipts, raw captures and normalized resolver artifacts require the current
contract and derivation fingerprints. The shared `v1` labels do not establish
compatibility. Reproduce stale derived artifacts from the recorded source/tool
inputs; retain raw output when it helps debugging or replay. Rust types and generated
JSON Schema own the capture and receipt formats. Producer context records the
adapter pin separately from reported compiler versions; neither establishes a
qualified source language mode. Missing compiler versions remain unknown.

```text
python scripts/build_resolver_adapters.py
# writes .tmp/adapter-build/resolver-adapters/ + build-receipt.json (maintainer/CI)
# $env:ENTMOOT_ADAPTERS_ROOT = "<that-out-dir>"   # or install under ~/.entmoot/adapters
entmoot init
entmoot status
```

Durable state defaults to `~/.entmoot/workspaces/<source-key>/` with `config.json`
(volume + empty `ENTMOOT_*` slots) and a `store/` volume. Publications, native
objects and readiness profiles use this selected volume, including when relocated.
Explicit index/attach writes honor its retention budget through database and view locators.

## Rust (in-process)

| | |
|--|--|
| Host | Rust toolchain **1.98.0** matching workspace `ra_ap_*` **0.0.358** |
| Env | Cargo features `resolved-rust-analyzer` (no `*_ADAPTER` binary) |
| Build | `cargo build -p entmoot --features producer` |
| Limitations | Metadata-safe default; Trusted policy for build scripts / proc macros |

The analyzed project's Cargo features are separate from Entmoot's build features.
`init`, `bind`, and `entmoot-producer rust produce` / `publish` share
`--cargo-features`, `--no-default-features`, `--cargo-target`, `--sysroot` and
`--rust-test-root`. Defaults keep `cfg(test)` disabled; feature selection alone
does not enable tests. For example, `entmoot bind --cargo-features full-tests
--rust-test-root tests/calls.rs` selects that repository-relative Cargo target
as a test. Only the selected crate gains `cfg(test)`; dependency libraries retain
their ordinary configuration. Missing required features, ambiguous targets and
invalid paths fail explicitly. Files with no unique active module stay unknown.
The typed requested profile participates in native build context and publication
input identities. Actual toolchain versions and target contexts are recorded
separately. These bounded controls establish test-cfg isolation, not complete Cargo
acceptance: IDE feature/dev-dependency resolution and custom test harnesses still
need qualification. Test-scoped production remains explicitly partial.
Native Rust loading discovers the selected toolchain's installed sysroot source;
`CargoConfig::default()` alone would disable it. Install `rust-src` explicitly
during setup (`rustup component add rust-src` for the selected toolchain).
`--sysroot` remains an explicit override. The raw context records this selection
and revalidates loaded source bytes, including standard-library sources. The
producer never installs missing components; unavailable targets stay unknown.
Target-query errors and partial Cargo metadata failures in rust-analyzer's IDE
loader are retained as native environment failures. They cannot silently qualify
a fallback model for the requested target. Missing sysroot source also fails the
native attempt while preserving its inventoried occurrence denominator.

The [production CLI tests](../../crates/entmoot-cli/tests/producer_cli.rs)
uses real adapters with independently authored occurrence selectors. It is
separate from compiler-free visitor fixtures; each result is limited to its recorded tools and inputs.

## JS/TS — Oxc (in-process)

| | |
|--|--|
| Host | Cargo feature `resolved-oxc` (**oxc 0.153.0**, rustc 1.98.0) |
| Env | Optional `ENTMOOT_OXC_RUNTIME=node\|browser\|unspecified` (default `unspecified`) |
| Limitations | Distinct from TypeScript Program; no silent merge of families; member calls open |

The runtime selection is operator evidence for the entire analyzed JS/TS scope
and is recorded in the native build context and reuse key. Select `node` only
for a Node scope. Recognized core specifiers use stdlib metadata there; bare
builtin names in unspecified/browser scopes retain unknown package ownership.
An explicit `node:` specifier supplies builtin evidence in unspecified scope.
These cases never fabricate an npm package coordinate or close dispatch.

## JS/TS — TypeScript Program

| | |
|--|--|
| Host | Node.js; pinned **TypeScript 6.0.3** Program API (ceiling for this adapter) |
| Env | `ENTMOOT_TYPESCRIPT_ADAPTER`; optional `ENTMOOT_TYPESCRIPT_CONFIG`, `ENTMOOT_TYPESCRIPT_LIBRARY` |
| Build | `python scripts/build_typescript_program.py --out .tmp/adapter-build/typescript-program` |
| Artifact | `.tmp/adapter-build/typescript-program/entmoot-typescript.cjs` |
| Source | `crates/entmoot-native/src/native_resolver/producers/` |
| Limitations | Requires `tsconfig.json`; identity distinct from Oxc; historical controls 4.9.5–5.9.3; **TypeScript 7.x has no compiler API and is not qualified** |

## Python — Pyright Program

| | |
|--|--|
| Host | Node.js + Pyright checkout for build |
| Env | `ENTMOOT_PYRIGHT_ADAPTER`, `ENTMOOT_PYTHON_ENVIRONMENT` |
| Build | `python scripts/build_pyright_program.py --source <pyright> --out .tmp/adapter-build/pyright-program` |
| Artifact | `.tmp/adapter-build/pyright-program/entmoot-pyright.cjs` |
| Limitations | Selected import roots are recorded; project typechecker config is not loaded |

`ENTMOOT_PYTHON_ENVIRONMENT` or the analyzed root's `.entmoot/python_environment.json`
can specify interpreter/import roots. Otherwise Python selects an explicit executable,
`VIRTUAL_ENV`, `CONDA_PREFIX`, project `.venv`/`venv`, then PATH. The selected
interpreter supplies site-package roots; only recorded roots are used. Discovery
does not invent distribution/package identities. Missing imports and unsupported
bindings remain explicit. Input text must be UTF-8.

Normal discovery respects `.gitignore` and cache/build directory exclusions.
An explicitly selected checkout inside an ancestor's ignored directory uses its
own ignore rules; the ancestor cannot erase the entire requested source scope.
Python native acceptance checks declaration lookup, exact spans and explicit
dynamic gaps. It does not require bytecode compilation or closed runtime dispatch.

Prepare local audits as separate buildable project scopes under `.tmp/<task>/`.
Discover manifests first: Maven/Gradle classpaths and source sets, restored
MSBuild projects, configured CMake compilation databases, Cargo profiles, Go
modules/tags, TypeScript configs and Python environments. Keep unrelated syntax
fixtures outside each selected program. Use each language's authored static call
and dynamic/indirect controls, then run the requested real project. Report the
remaining target, setup action and run cost from its receipt; a successful host
probe is only readiness to run that control.

## Go

| | |
|--|--|
| Host | Go **1.26+** matching `tools/resolver-adapters/go/go.mod` (`x/tools` **v0.52.0**) |
| Env | `ENTMOOT_GO_ADAPTER`; optional `ENTMOOT_GO_TAGS`, `ENTMOOT_GO_TESTS=include\|exclude` (default `include`). Docker (`ENTMOOT_GO_VIA_DOCKER=1`) is a non-default override only |
| Build | `python scripts/build_resolver_adapters.py --families go` (host). Optional Docker: `docker compose -f tools/resolver-adapters/go/docker-compose.yml build` + `go build -C tools/resolver-adapters/go/docker_proxy -o .tmp/adapter-build/resolver-adapters/go/entmoot-go-resolver-docker.exe .` |
| Artifact | Host: `.tmp/adapter-build/resolver-adapters/go/entmoot-go-resolver[.exe]`; optional Docker proxy: `entmoot-go-resolver-docker.exe` |
| Source | [`tools/resolver-adapters/go/`](../../tools/resolver-adapters/go) |
| Limitations | One build configuration per run; cgo disabled; reflection/unsafe boundaries |

Docker Desktop path: the proxy must forward `ENTMOOT_GO_SINGLE_MODULE=1` (set by
the engine for isolated module loads) into `docker run -e`. Without that ack the
engine reports `invalid_environment: Go sidecar must acknowledge single_module.v1`.
Optional `ENTMOOT_GO_PREWARM_GOPROXY` prefetches modules inside the image before
offline produce (`GOPROXY=off` in the container).

Test scope is recorded in the native build context. `include` loads the native
test variants under the same build tags and input fence; identical repeated
facts coalesce, while conflicting facts remain ambiguous. Test-only declarations
do not become production package exports. `exclude` keeps test sites in the
inventory denominator with explicit production-only native scope. Rebuild the
Go sidecar and Docker proxy/image to pick up this policy.

Interface method declarations retain their callable-signature kind and exact
native span. Their association with an implementation remains unknown, with
explicit dynamic dispatch. A same-named concrete method is not a substitute;
signature evidence does not create an executable callable in inventory.

## C# (Roslyn)

| | |
|--|--|
| Host | .NET 9 SDK; **Microsoft.CodeAnalysis 4.14.0** |
| Env | `ENTMOOT_ROSLYN_ADAPTER` |
| Build | `python scripts/build_resolver_adapters.py --families roslyn` |
| Artifact | `.tmp/adapter-build/resolver-adapters/roslyn/bin/Release/net9.0/Entmoot.RoslynSolver.dll` |
| Source | [`tools/resolver-adapters/roslyn/`](../../tools/resolver-adapters/roslyn) |
| Limitations | Static candidates only; MSBuild workspace failures are coverage signals |

MSBuild API references are compile-only so the selected SDK supplies runtime
assemblies, following [MSBuildLocator guidance](https://github.com/microsoft/MSBuildLocator/blob/main/docs/diagnostics/MSBL001.md).
Restore the analyzed project's dependencies before requesting native bindings;
building the adapter does not restore the analyzed project.

## Java (javac Trees)

| | |
|--|--|
| Host | Full JDK 17+ selected by override, JAVA_HOME or PATH; Maven for building the adapter |
| Env | `ENTMOOT_JAVA_ADAPTER` (jar pin for the JVM sidecar) |
| Build | `python scripts/build_resolver_adapters.py --families java` |
| Artifact | `.tmp/adapter-build/resolver-adapters/java/entmoot-java-resolver.jar` |
| Source | [`tools/resolver-adapters/java/`](../../tools/resolver-adapters/java) |
| Limitations | Selected compiler materialization only; compile-time bindings remain candidate call edges |

Materialization is **javac Trees only**. `ENTMOOT_JAVA_ENGINE` and Spoon CTModel
are retired; setting the engine env yields `invalid_environment` (no aliases).
Set `ENTMOOT_JAVA_RELEASE` and explicit jars through `ENTMOOT_JAVA_CLASSPATH`, or
point `ENTMOOT_JAVA_ENVIRONMENT` at a JSON **file** (not inline JSON). When
`source_roots` is present in that profile, it **replaces** host-discovered
`.java` inputs (so `src/test/java` does not fail the bind for missing junit jars). Public source package surfaces
need `ENTMOOT_PACKAGE_API` mappings; call bindings alone do not establish Maven
identity. See the [Java adapter guide](../../tools/resolver-adapters/java/README.md).

Pin the current in-tree jar with `ENTMOOT_JAVA_ADAPTER`; the same exclusive path
is reported in the workspace summary and used for production. Rebuild the CLI
with `--features producer` after updating it: an old executable cannot implement
new adapter discovery (pre-commit may land under `target/commit-check/release/`).
Set `JAVA_HOME` to the intended JDK or select `ENTMOOT_JAVA` explicitly. Never solve a stale pin by falling back to a sibling
checkout. After an engine or receipt-field rename (`adapter_pin`), wipe the
home-scoped workspace store before re-`init`—stale pre-javac receipts are not
forward-compatible. Impact pairs need a stable shared `--repo` /
`repository_id` across pins. Open-virtual Java getters stay candidate call
edges; empty `resolved_only` callers are not absence.

## C/C++ (Clang)

| | |
|--|--|
| Host | Python + libclang (`LIBCLANG_PATH`); operator-supplied CompDB required |
| Env | `ENTMOOT_CLANG_ADAPTER`; optional `ENTMOOT_CLANG_VFS`, `ENTMOOT_CLANG_COMPDB` (explicit database file) |
| Build | `python scripts/build_resolver_adapters.py --families clang` |
| Artifact | `.tmp/adapter-build/resolver-adapters/clang/entmoot-clang-resolver.py` |
| Source | [`tools/resolver-adapters/clang/`](../../tools/resolver-adapters/clang) |
| Limitations | Existing `compile_commands.json` required (Entmoot does not generate CompDBs); one build configuration per artifact; C++ experimental for release claims |

Call vs indirect vs macro gaps follow the OOTB
[classification discipline](../../tools/resolver-adapters/clang/README.md#classification-discipline-ootb)
(Multiplier-informed verbs; Entmoot span join and IDs unchanged).

## Archive note

Historical sibling checkout `entmoot-native-solvers` is no longer the operational
default. Sources were imported under `tools/resolver-adapters/`. Dated campaign
receipts may still mention old `ENTMOOT_*_SOLVER` names—live surfaces use `*_ADAPTER` only.
