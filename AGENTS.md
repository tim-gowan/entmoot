# Project instructions

Entmoot stores attributable knowledge across repository revisions and provides
structural inventory, identities, bindings and change impact. It is not a scanner
or a taint/dataflow engine. Rust, Python, JS/TS/TSX, Go, Java, C# and C are the
implemented structural families; C++ remains experimental.

Read only the owners needed for the task: [PLANNING](PLANNING.md),
[ARCHITECTURE](docs/architecture/ARCHITECTURE.md), touched Rust contracts and
tests, and the relevant [runbook](docs/runbooks/). Rust and generated JSON Schema
own wire shape. [CHANGELOG](CHANGELOG.md) records delivered work.

## Correctness

- Structural parsing owns syntax, spans and identities. Native tools own typed
  dispatch and environment-dependent resolution. Native evidence never rewrites
  structural IDs. Keep structural builds compiler-free.
- Preserve provenance, snapshot scope and explicit unknown/partial outcomes.
  Missing evidence and exhausted budgets do not establish absence. A parse is
  not compiler qualification; routes are not taint; matches are not verdicts.
- KnowledgeRepository owns durable mutation; topology is a disposable view.
  Preserve atomic publication, producer isolation, idempotence and historical
  retrieval. Validate source identity before attaching evidence.
- Retain useful supplied facts with their meaning. Raw provider output or exact
  receipt bytes are optional unless a concrete debugging, replay or product need
  justifies their cost. Do not add retention ledgers or blanket archive gates.
- Entmoot-owned series stay v1 until the repository move. Fingerprints distinguish
  incompatible layouts and derivations. External versions retain their names.
- Normalize solver facts at the adapter boundary into shared Entmoot semantics.
  Prefer established, language-neutral terms; keep language-specific mechanisms
  as extensible values under a common marker (for example `indirect_call`).
  Consumers use the shared category; absence means unreported, never disproved.
  Preserve source attribution, native type meaning and explicit behavioral
  exceptions. Provider spellings belong in attributed detail or raw captures.
- Audit terminology when changing any adapter entity or specification: follow it
  through contracts, joins, persistence, canon, receipts and CLI/MCP projections.
  Converge aliases in one boundary owner. Keep binding, dispatch, completeness,
  environment failure and budgets distinct; test unknown future detail values.

## Working conventions

- **Every turn:** when this session changes durable behavior, contracts, CLI/MCP
  surfaces, maturity, meaning or procedures, update the matching owner in the
  same turn—do not defer a docs pass. Prefer inline rustdoc (`//!` / `///`) for
  public API and engine semantics; [README](README.md) for operator entry and
  maturity; [ARCHITECTURE](docs/architecture/ARCHITECTURE.md) for the identity
  and ownership walkthrough; short runbooks for procedure steps only (link, do
  not restate essays); Clap help / MCP `--tools-schema` / contract schema
  examples for interface and wire shape. [llms.txt](llms.txt) is a static LLM
  discovery stub (GitHub repo root URL only)—not a second copy of this file.
- Further execution machinery must earn its place through measured need. Use
  representative end-to-end workflows, including interruption/recovery where
  relevant, to justify additional scheduling, queuing, checkpointing or
  orchestration. Prefer improving existing shared logic; preserve publication
  and recovery guarantees.
- Prefer changing or shortening an existing owner. Markdown belongs in README,
  AGENTS, ARCHITECTURE, PLANNING, CHANGELOG, runbooks or concise rules. Keep fields
  in APIs/schema/tests, not duplicated prose. Git retains obsolete guidance.
- Use `.tmp/<task>/` for scratch, isolated campaign checkouts/builds and useful
  local evidence. Do not modify foreign sibling source/history. Normal source
  scans exclude scratch and caches. Close active stores before deleting them.
  Leave `.gitignore` unchanged during the maintenance pass.
- Use production revision/occurrence fixtures with independently authored
  expectations. Keep native unavailability honest (missing tools stay
  unknown/partial, never silent absence). Bind requires adapters for inventory
  languages unless `--structural-only`. Run focused checks before
  workspace/fmt/Clippy/doctests/schema checks. Maintain
  [test ownership](scripts/test_ownership.json) when targets change.
- Keep the five crate boundaries. Contracts contain no vendor/compiler/DB types;
  CLI and MCP share engine read operations. Reads do not enqueue native producers.
  Frontends may bootstrap missing structural inventory; existing publications
  require explicit init/bind to refresh. Durable state defaults to `~/.entmoot`
  (never `<source>/.entmoot`); `ENTMOOT_DATA_DIR` / `--data-dir` override the
  home root. Keep repository, dependency and native-library call-graph
  completeness separate; syntax is not native type evidence.
  Use typed errors in libraries, anyhow at CLI; no production unwrap/expect.
  Identity digests use BLAKE3. Policy: Cargo.toml sha2 bans (pre-commit),
  AST (`cargo dylint` / [tools/entmoot-lints](tools/entmoot-lints)), licenses
  (`cargo deny --all-features --exclude-dev check licenses advisories`), unused
  deps (`cargo machete`).
  Optional local `sccache` via `RUSTC_WRAPPER` when on PATH. Do not hand-edit
  generated contracts or grammars.
- Preserve unrelated worktree changes. Commit validated work unsigned with normal
  hooks. Keep secrets and scratch out of commits. No Canvas unless requested.

## Native tools

Adapters live in [tools/resolver-adapters](tools/resolver-adapters/README.md).
Build explicitly with the setup runbook (`scripts/build_resolver_adapters.py`);
`init`/`bind` never install or download toolchains. Build the CLI with
`--features producer` for in-process Rust Analyzer and Oxc. `init` refreshes the
home-scoped adapter profile from host discovery; `bind` requires native for
inventory languages unless `--structural-only`. Process `ENTMOOT_*` overrides
win over per-workspace/global `config.json` slots and are exclusive when set
(no fallthrough). Adapter discovery: checkout conventional paths → PATH →
`~/.entmoot/adapters/` (`ENTMOOT_ADAPTERS_ROOT` overrides). Missing or stale
tools remain visible in the profile and `status`.

### Maintainer glossary

- **structural** — compiler-free Tree-sitter inventory (syntax, spans, identities).
- **native** — language/ecosystem evidence lane (typed dispatch, env facts);
  contrasts with structural; `require_native` / `NativeReceipt`.
- **Resolver adapters** — product name for tools that produce native evidence
  (in-process, Program, sidecars) under `tools/resolver-adapters/`.
- **adapter_path** — runtime binary/path on produce stages and status JSON;
  env remains `ENTMOOT_*_ADAPTER`. Wire artifact series stay
  `ArtifactFormat::Resolver` / `entmoot.native_resolver.v1`.
- **adapter profile** — readiness file under the home-scoped store
  (`entmoot.adapter_profile.v1`); discovery only, no downloads.
- **home config** — `~/.entmoot/config.json` and per-workspace
  `workspaces/<key>/config.json` (`entmoot.config.v1`): volume pointer plus
  empty `ENTMOOT_*` override slots.
- **`--features producer`** — Cargo feature enabling in-process RA/Oxc in the CLI.
- **entmoot-producer** — helper binary for isolated family produce runs.
- **`--structural-only`** — waiver from default required-native bind.
- Family id **`java`** / `ENTMOOT_JAVA_ADAPTER` — javac Trees jar (no Spoon engine).
- Hidden CLI **`index`** / **`source-fetch`** — low-level structural write and
  immutable revision fetch; operators use **`init`**.
- **subject** — declared publication identity (PURL or repository[+path]) on
  `SourceSnapshot`; compare floor instead of materialization `repository_id`
  alone ([ARCHITECTURE](docs/architecture/ARCHITECTURE.md)).
- **correspondence** — compare-time join policy (`CorrespondencePolicy`); may
  remap occurrence keys without rewriting inventorial `logic_address` hashes
  ([impact](docs/runbooks/impact.md)).
- **inventory overlay** — presence-only cross-pub symbol delta; not Bound/Fits.
  **`package_api`** — optional stronger compatibility rung.

Study prior art in place; do not copy GPL code or vendor AST types into contracts.

Local workflow skills: [.agents](.agents/README.md). Maintain the README maturity
table with owning controls. Regenerate package release inputs and schemas through
their scripts/Rust examples; verify extracted-package fingerprints before release.
