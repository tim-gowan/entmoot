# Entmoot

Entmoot gives developers and agents attributable structural knowledge about a
repository: source occurrences, bindings, caller/callee relationships and bounded
change impact. It preserves provenance and revision identity so that missing or
partial evidence stays visible. It is a defensive development substrate, not a
scanner, taint engine or security verdict.

Rust, Python, JS/TS/TSX, Go, Java, C# and C have structural inventory. C++ is
experimental. Structural operation requires no language compiler at runtime.
Native adapters add environment-dependent evidence when explicitly requested.

## Start locally

From this checkout (Rust toolchain selected by `rust-toolchain.toml`):

```text
cargo install --path crates/entmoot-cli --locked
cargo install --path crates/entmoot-mcp --locked
entmoot -C /path/to/project find work
entmoot -C /path/to/project status
entmoot -C /path/to/project boundaries --callee "*.toString()" --outcome unsupported
```

After the alpha is published, registry installation uses
`cargo install entmoot --version 1.0.0-alpha.1 --locked` and the equivalent command
for `entmoot-mcp`. The repository's version is a release candidate until the
protected release workflow succeeds; local tests do not establish publication.

Run inside the source repository or select it with `-C`. First use automatically
creates a structural inventory and private local storage under `~/.entmoot`
(override with `--data-dir` / `ENTMOOT_DATA_DIR`). Analyzing a checkout never
writes into that checkout. Later reads reuse the current immutable publication.
No database setup is required. Remote storage is not implemented.

```text
entmoot find work
entmoot inspect <id-returned-by-find>
entmoot callers <id-returned-by-find>
entmoot callees <id-returned-by-find>
entmoot read-source src/main.rs --start-line 1 --end-line 30
entmoot impact
entmoot init
```

`impact` compares the retained baseline with working-tree changes. `init`
explicitly refreshes that baseline and adapter discovery. A read does not silently
replace history, refresh stale source, launch native tools or migrate old stores.
Verified source reads report source drift. Terminals receive readable output;
`--json` and redirected output receive the typed JSON contract. Use `--detail
explain` for evidence and `--detail export` for the full bounded result. Search
limits and compact presentation limits are separate; preserve returned cursors.

## Native evidence and open boundaries

Build/install the CLI with `--features producer` for in-process Rust Analyzer/Oxc
and its matched `entmoot-producer` helper. Other adapters require explicit
[setup](docs/runbooks/resolver-adapters.md); setup and analyzed project builds can
execute project code and belong in a trusted, isolated environment.
`entmoot bind` requires native support for inventoried languages. Missing tools
fail visibly; `--structural-only` is an explicit waiver. `init`/`bind` never install
SDKs or download toolchains.

```text
entmoot boundaries --callee "object.toString()" --detail export
entmoot boundaries --callee "*.toString()" --outcome unresolved --detail export
entmoot boundaries --reason indirect_call --detail export
```

Boundary queries count distinct source call sites, preserve source spans/digests,
original unresolved attempts and attributed native targets/dispatch detail.
`unreported` means no retained observation, not an absent call. `*.toString()` is
a syntax pattern; it does not infer the receiver's native type. Providers may
report different outcomes for the same site; their partitions can overlap.
Select a `--build-context` to narrow evidence. Unknown contexts are rejected.
Results retain publication/site/artifact selectors that a future provider can
use to attach additional evidence without rewriting structural identity.

Call-graph completeness has independent **repository**, **dependencies** and
**native libraries** scopes. External edges do not by themselves invalidate
represented repository edges. They also do not prove repository closure:
implicit calls, unresolved local dispatch and external callbacks remain possible.
All three scopes currently report `not_established`; producer completion is not
an exhaustive closure claim.

## MCP and integrations

Start `entmoot-mcp -C /path/to/project`, optionally with the same `--data-dir`.
It prepares/reuses the same workspace and serves shared engine operations over
stdio. First-time inventory can delay startup; later publication loading is
asynchronous. `--tools-schema` emits generated input schemas without opening a
workspace. The [MCP guide](crates/entmoot-mcp/README.md) covers bounded output,
shared loopback service, authentication and explicit producer execution.

Compare workflows have distinct meanings: [impact and inventory overlay](docs/runbooks/impact.md)
versus optional [package API compatibility](docs/runbooks/compatibility.md).
Missing package evidence returns `incomplete_bind`; it is not a safe upgrade.

## Implementation maturity

Reviewed 2026-10-16. Maintain this table with the owning controls whenever behavior
or qualification changes. **Alpha candidate** means implemented and locally
controlled; **partial** means useful evidence with material gaps. No row claims
exhaustive language semantics or runtime/security proof.

The [fresh alpha sample](docs/runbooks/local-alpha.md#fresh-alpha-evidence-2026-10-16)
covers 5,407 call sites across four pinned publications and 47 verified source
digests/spans, with build and native-scope limitations kept explicit.

| Capability | Maturity | Utility and limit / verification owner |
|---|---|---|
| Structural inventory and identities | Alpha candidate | Compiler-free source/revision navigation; parsing is not compiler qualification. [Revision controls](crates/entmoot-core/tests/revision_pairs.rs) |
| Automatic local storage and recovery | Alpha candidate | First-use structural preparation, optional home-directory location, immutable history and fingerprint fencing. [Workspace controls](crates/entmoot-cli/tests/workspace_defaults.rs) |
| CLI/MCP navigation and source | Alpha candidate | Shared bounded operations and digest-verified reads; first inventory has a startup cost. [Transport controls](crates/entmoot-mcp/tests/stdio_e2e.rs) |
| Source/native call boundaries | Alpha candidate | Distinct-site counts, attributed native detail, explicit unreported evidence and three closure scopes. [Boundary controls](crates/entmoot-engine/tests/call_boundaries.rs) |
| Native binding/dispatch | Partial, per configuration | Repository/external/intrinsic/unknown observations; no universal closed-world claim. [Native qualification](.github/workflows/native.yml) |
| Revision impact and scoped overlay | Alpha candidate | Scope/signature/multiplicity-aware presence deltas; not API compatibility. [Overlay owner](crates/entmoot-engine/src/overlay.rs) |
| Consumer/provider compatibility | Partial | Authored positive/negative controls; missing import/export scope remains incomplete. [Controls](crates/entmoot-engine/tests/package_compatibility.rs) |
| Dependency materialization | Alpha candidate | Confined staged copying, opaque version keys, symlink rejection and resource bounds. [Safety owner](crates/entmoot-engine/src/materialize.rs) |
| Test impact / unused candidates | Partial | Review and test-selection assistance; no coverage or dead-code proof. [Testing](docs/runbooks/testing.md) |
| Canon / clone candidates / collaboration | Experimental | Explicit evidence policies and local recovery; no runtime equivalence proof or remote collaboration service. [Architecture](docs/architecture/ARCHITECTURE.md) |
| Registry packages / generated docs / CI | Alpha candidate; external release pending | Five independently extractable packages, package fingerprint equality rehearsal, protected publication and generated docs. [Release gates](docs/runbooks/local-alpha.md) |
| Remote storage / global package catalog | Planned, unimplemented | Future location transport must preserve publication, provenance, query and mutation semantics. |

## Maintain and release

[Architecture](docs/architecture/ARCHITECTURE.md), [planning](PLANNING.md),
[changelog](CHANGELOG.md), [runbooks](docs/README.md) and [AGENTS](AGENTS.md) are the
owners. Rust and [generated JSON Schema](crates/entmoot-contracts/generated/README.md)
own wire shapes. Repository skills under [.agents/skills](.agents/README.md) give
short operator/release workflows. No generated docs need hand-maintained copies.

The [local alpha guide](docs/runbooks/local-alpha.md) covers the prior corpus
sample, validation, upgrade/rebuild policy, private-repository Pages setup and
handoff to `tim-gowan/entmoot`. CI separates compiler-free tests from provisioned
native controls. Publish only the exact revision that passes both.

MIT OR Apache-2.0. Bundled grammar sources retain their upstream licenses.
