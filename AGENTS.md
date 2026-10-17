# Project instructions

Entmoot stores attributable repository knowledge across revisions: structural
inventory, identities, bindings and change impact. It is not a scanner or a
taint/dataflow engine. Rust, Python, JS/TS/TSX, Go, Java, C# and C have structural
inventory; C++ remains experimental. Native qualification is per provider,
construct and configuration, never inferred from parsing or test counts.

Read the relevant [PLANNING](PLANNING.md) scope, [ARCHITECTURE](docs/architecture/ARCHITECTURE.md)
ownership, Rust contracts, tests and [runbook](docs/README.md). Read only what the
task needs. Rust and generated JSON Schema own wire shape; [CHANGELOG](CHANGELOG.md)
records delivered work. Known defects belong in PLANNING, not weaker contracts.

## Invariants

- Structural parsing owns syntax, spans and identities and needs no analyzed
  language compiler at runtime. Native tools own typed dispatch and environment
  facts; their evidence never rewrites structural IDs. Building bundled grammars
  still uses the repository's C build toolchain.
- Preserve provenance, snapshot/configuration scope and explicit unknown/partial
  outcomes. Missing evidence and exhausted budgets never establish absence.
  Routes are not taint; matches are not verdicts. Keep repository, dependency and
  native-library call-graph completeness separate.
- KnowledgeRepository owns durable mutation; topology is a disposable view.
  Preserve source fences, atomic publication, producer isolation, idempotence,
  historical retrieval and recovery. Query-service reads never enqueue native
  producers. Frontends may bootstrap missing structural inventory; refreshing
  existing publications requires explicit init/bind.
- Keep the five crate boundaries. Contracts contain no vendor/compiler/DB types;
  CLI and MCP share engine read operations. Use typed library errors and anyhow
  at the CLI boundary; no production unwrap/expect. Identity digests use BLAKE3.
- Normalize provider vocabulary once at the adapter boundary into shared,
  language-neutral categories. Keep mechanisms as extensible attributed detail
  (for example `indirect_call` plus its mechanism). Preserve native type meaning;
  absence means unreported. Binding, dispatch, completeness, environment failure
  and budget exhaustion are distinct.
- Retain useful supplied facts with their meaning. Raw output and exact receipt
  bytes are optional unless a concrete debugging, replay or product need warrants
  them. Do not add retention ledgers or blanket archive gates.
- Entmoot-owned series stay v1 until the repository move. Fingerprints distinguish
  incompatible layouts and derivations; external versions retain their names.
  Never reset stores or rewrite fingerprints to admit incompatible evidence.

## Check the affected boundary when changing it

- **Identity, facts or terminology:** trace extraction/adapter → contract → join →
  persistence/reload → identity/canon → receipt → CLI/MCP. Check scope, direction,
  ambiguity, provenance, negative lookups and unknown future detail values.
  Equality tests need required-distinction pairs with independent expectations.
- **Storage, paths or configuration:** use shared location/locator helpers for
  publications, artifacts, profiles, locks, budgets and retention. Durable state
  belongs under `~/.entmoot`, overridden by `--data-dir` / `ENTMOOT_DATA_DIR` and
  workspace volume config, never a reconstructed `<source>/.entmoot` path.
  Exercise default and relocated roots, invalid/remote config, explicit override
  precedence, restart and every affected reader/writer. Invalid configuration
  must fail visibly rather than redirect writes. Verify native detail is readable
  from the published store and analyzed source bytes/paths remain unchanged.
- **Publication, cache or recovery:** test ordinary and interrupted results,
  rollback, stale workers/generations, conflicting producers and historical reads.
  Check added/deleted/renamed inputs and negative lookup invalidation. Reuse must
  preserve dependency/corruption checks and match a fresh production result.
- **CLI/MCP:** check shared defaults, limits, continuation, errors, stdout/stderr,
  exit status and help/schema examples. Distinguish failure from useful partial
  evidence; do not install tools or refresh native evidence from read paths.
- **Tests, features or CI:** update [ownership](scripts/test_ownership.json),
  Cargo target requirements and every generated/workflow invocation together.
  Confirm intended tests actually execute with the selected features and filters;
  zero tests, ignored tests and unavailable tools are not successful qualification.
  Pre-commit is an accelerator; never add path-based correctness skips to `ci.yml`.
- **Docs/comments/maturity:** update the owning description in the same turn.
  Rustdoc owns public semantics; README owns operator entry and maturity;
  ARCHITECTURE owns identity/ownership; runbooks own procedure; Clap/MCP/generated
  schema own interfaces. Distinguish required invariants, observed behavior and
  unresolved defects. A dated successful run does not qualify a later revision.
- **Evaluations:** follow [the protocol](evals/PROTOCOL.md). Freeze inputs and
  independent oracles before trials; retain failures, missing measurements and
  repository clusters. Source inventory, syntax, dispatch and runtime behavior
  are separate claims. Output size alone does not establish agent cost savings.

## Interface design

- Start with an operator question and reuse the shared engine operation. CLI verbs
  use kebab-case; wire fields and MCP operations keep their existing snake_case
  convention. Name selectors for their entity and scope; distinguish IDs, names,
  paths, counts, byte offsets and budgets rather than accepting ambiguous strings.
- Parameterize choices callers need: subject/revision, evidence scope, filters,
  limits and continuation. Keep implementation tuning internal unless a measured
  workflow needs control. Reuse typed selectors and shared defaults across CLI/MCP;
  do not introduce per-command aliases for the same concept or opaque JSON flags
  when a small typed option suffices. Preserve existing spellings or document an
  explicit compatibility change.
- Environment variables select process-wide deployment/tool defaults, not hidden
  query semantics. Prefer explicit request parameters for reproducible operations.
  Define precedence at the shared configuration owner, validate explicit values,
  and test unset, empty, invalid and conflicting selections in child processes.
- Rust contracts own input/output semantics and validation; regenerate schemas.
  Return decision-relevant summaries with stable IDs, provenance, uncertainty and
  actionable continuation. Keep explain/export for detail. Every truncated result
  needs a usable next step; cursors must bind the filters and publication they page.
  Machine-readable stdout must remain parseable; diagnostics belong on stderr.
- Identity encodings must frame variants, fields and optional presence before
  hashing. Presentation, correspondence and equivalence are separate policies;
  deduplication must not erase distinct evidence. Test required distinctions as
  well as intentional equivalences through persistence and public queries.

## Implementation and validation

- Fix the existing common owner before adding another abstraction. Split modules
  by responsibility or invariant when that clarifies ownership; file length alone
  is not a reason. Names should describe behavior, scope and uncertainty. Comments
  explain constraints or non-obvious reasons; remove stale paths and promises.
- New scheduling, queuing, checkpointing or orchestration requires measured need
  in representative existing workflows, including recovery costs where relevant.
  Preserve current guarantees and respect deferred scope in PLANNING.
- Write a failing regression for a behavioral defect before fixing it. Use real
  revision/occurrence fixtures and independently authored expectations. Helpers
  must not normalize away the distinction an assertion claims to verify. Keep
  layer tests and end-to-end controls for different claims; avoid duplicate cases.
- Fixtures own their source, storage and cleanup lifetimes. Pass paths/config
  explicitly; test process-global environment/CWD changes in child processes.
  Do not clear a caller's overrides or write into a maintainer's real home store.
  See [testing](docs/runbooks/testing.md) for assertion and isolation rules.
- Run focused checks first, then the required workspace, fmt, strict Clippy,
  doctest and schema checks for shared changes. Verify commands against Cargo
  metadata. Report passed, failed, unavailable and unrun checks separately;
  preserve baseline failures instead of weakening tests or skipping hooks.
- Policy owners: Cargo.toml sha2 bans (`scripts/pre_commit_check.py`), AST rules
  (`cargo dylint` / [entmoot-lints](tools/entmoot-lints)), licenses/advisories
  (`cargo deny --all-features --exclude-dev check licenses advisories`), unused
  dependencies (`cargo machete`). Optional `sccache` is allowed when on PATH.
- Regenerate contracts, grammars and release inputs through their scripts/Rust
  examples; never hand-edit generated outputs. Source comments also enter package
  derivation fingerprints. Verify extracted-package fingerprints before release.
- Prefer shortening existing documentation to creating parallel guidance. Keep
  `llms.txt` a static repository-root URL discovery stub. Use `.tmp/<task>/` for
  scratch/evidence; scans exclude scratch/caches. Close stores before deletion.
  Leave `.gitignore` unchanged during this maintenance pass.
- Preserve unrelated changes. Commit validated work unsigned with normal hooks;
  keep secrets and scratch out of commits. No Canvas unless requested. Study
  prior art in place; do not copy GPL code or vendor AST types into contracts.

## Native tools and vocabulary

Resolver adapters live under [tools/resolver-adapters](tools/resolver-adapters/README.md).
Build them explicitly with [adapter setup](docs/runbooks/resolver-adapters.md);
init/bind do not install adapters or SDKs. Build the CLI with `--features producer`
for in-process Rust Analyzer/Oxc and the matched `entmoot-producer` helper.
Init is structural by default and refreshes adapter discovery; bind requires
native support for inventory languages unless `--structural-only`.

Process `ENTMOOT_*` selections must remain exclusive over config/defaults; empty
config slots are unset. Default discovery is checkout → PATH → home adapters;
bind may reuse a recorded profile path after honoring explicit overrides. Missing/stale
tools stay visible in the adapter profile and status. Authored
provider data qualifies attachment; live controls qualify native semantics.

- **structural / native:** compiler-free inventory / ecosystem evidence lane.
- **adapter_path:** runtime produce/status path; env remains `ENTMOOT_*_ADAPTER`.
  Wire series stay `ArtifactFormat::Resolver` / `entmoot.native_resolver.v1`.
- **adapter profile / home config:** discovery readiness / volume and override
  selection under the data root. Rust contracts own their fields.
- **java:** javac Trees family, `ENTMOOT_JAVA_ADAPTER`; no Spoon engine.
- **index / source-fetch:** hidden low-level writes; operators use init.
- **subject:** declared PURL or repository[+path] publication identity; compare
  this instead of materialization `repository_id` alone.
- **correspondence:** compare-time join policy; never rewrites inventorial hashes.
- **inventory overlay / package_api:** presence-only symbol delta / optional
  stronger compatibility rung; see [impact](docs/runbooks/impact.md).

Local workflow skills and concise rules: [.agents](.agents/README.md).
If present, read `.agents/maintainer-configuration.mdc` for machine-local execution
paths. Local overrides do not change the portable defaults or invariants above.
