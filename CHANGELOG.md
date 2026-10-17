# Changelog

## 2026-10-16: Release documentation workflow

- Honor reusable docs inputs when the caller event is a tag push. Direct push/PR
  runs still stage only; explicit false inputs remain false. Exercise the actual
  workflow parameter script in local hooks and repository CI.
- Require documentation qualification before registry publication; deploy Pages
  after publication only when the repository has enabled it.

## 2026-10-16: Local adapter qualification repairs

- Preserve atomic artifact, JSON and ledger publication under deeply nested
  Windows data roots, retaining no-clobber and corruption checks. Resolve the
  same paths for SQLite repository, maintenance and queue readers/writers.
- Keep Roslyn project-load failures as invalid environments; do not substitute
  successful ad-hoc evidence for a failed MSBuild compilation scope.
- Run Clang VFS and Java binary-classpath controls in both native CI platforms,
  removing obsolete host-specific ignores. Add malformed-project coverage.
- Exclude local `.entmoot` tools/data from repository Markdown and dependency
  policy scans, with controls that still reject defects in repository sources.
- Resolve Program test tools through shared adapter discovery, including relocated
  adapter roots, without selecting a different provider for default CLI fixtures.
- Prune hidden local data and scratch trees from Go module discovery while
  retaining visible nested modules and explicitly selected roots.
- Select a run-owned data volume explicitly for revision campaigns, preserving
  analyzed checkouts and existing local data across pauses and resumed reads.
- Add the optional local maintainer-rule entry point and correct selected-volume
  adapter-profile and release procedures. Portable storage defaults are unchanged.

## 2026-10-16: Storage and test-qualification repairs

- Archive native evidence in the publication's selected volume and retain readable
  detail after restart/rebind, including historical views and relocated storage.
- Make storage selection return configuration errors; reject malformed/remote
  global and workspace config without fallback publication. Apply retention
  budgets to explicit database/view writes regardless of volume directory name.
- Restore selected-volume syntax-cache lookup; migrate obsolete path controls and
  native CI discovery, preserving lock, corruption, recovery and source fences.
- Isolate environment-sensitive location/profile tests in child processes and
  keep CLI source/data fixtures in one cleanup lifetime. Keep mock-adapter
  synchronization outside analyzed source and assert the intended failure reason.
- Select Kubernetes fixture resources by kind and name so same-named Services
  cannot replace Deployments or StatefulSets through hash-dependent ordering.
- Derive native test features from Cargo target requirements, including installed
  sidecar controls. Match dispatch categories and details exactly; optional target
  expectations reject unexpected or duplicate callees.
- Align checkout/PATH/home precedence in runtime and maintainer discovery. Empty
  process overrides stay exclusive; profiles honor config-selected adapters and
  distinguish them from process selections. Live sidecar qualification remains
  an explicit native CI requirement.

## 2026-10-16: Maintainer guidance and documentation audit

- Map changes to identity, storage, publication, frontend and test-lane checks in
  AGENTS; clarify fixture isolation, independent assertions and evidence limits.
- Correct storage/discovery/help comments, native feature commands and release
  version examples. Mark previous qualification reports as historical and record
  reproduced implementation/test blockers in PLANNING. These are documentation
  corrections, not fixes for the runtime defects or renewed release qualification.

## 2026-10-16: Docs after publish; parameterized pipelines

- Move CD docs to after successful crates.io publish; CI/PR docs stage under
  `.tmp/staging` via shared parameterized `docs.yml`.
- Parameterize workflow env/inputs across ci/native/docs/demo/release for reuse.

## 2026-10-16: Phased CI coverage and corpus demo

- Split `ci.yml` into structural, policy, and repo jobs with explicit coverage
  claims; upload release CLI artifacts from structural builds.
- Rename native job to `resolved`, add rust-cache, upload producer CLI artifacts.
- Add dispatch-only `demo.yml` plus `evals/demo/getrandom-structural.json`
  (pinned `getrandom-0.1.16`, structural-only).

## Local candidate preparation (2026-10-16)

- Default durable state to `~/.entmoot` (workspaces, `config.json` volume +
  `ENTMOOT_*` slots, adapters). Native artifact placement remains a known gap.
  Current discovery: nonempty process/config overrides → checkout → home → PATH;
  PLANNING tracks the discrepancy from the intended precedence.
- Prepare and reuse local workspace storage automatically in CLI/MCP; optional
  `ENTMOOT_DATA_DIR` / `--data-dir` overrides the home root. Native production remains explicit.
- Add find/inspect/read-source/callers/callees/boundaries CLI verbs, shared boundary
  queries with native attribution and distinct-site counts, and separate repository,
  dependency and native-library completeness statements.
- Confine dependency materialization with budgets/staging; canonicalize versionless
  PURLs; preserve scoped overlay signatures and declaration multiplicity.
- Correct Go receiver/package path normalization while retaining native spelling;
  keep expected structural preparation quiet and context-specific attempts isolated.
- Publish metadata for five crates (`entmoot` is the CLI package), package-local
  composed implementation fingerprints, installed adapter discovery, generated
  docs and protected CI/release workflows. Publication remains pending external gates.


## 2026-10-16: Local alpha readiness audit

- Added a maintained README maturity table and an ordered release plan targeting
  `tim-gowan/entmoot`, with generated documentation and CI/CD acceptance gates.
- Audited corpus receipts/current publications, source-verified a bounded sample,
  and documented reproduced dependency-path and overlay-scope defects. These
  remain implementation work; this documentation does not certify a release.

## 2026-10-16: Corpus native friction (go examples, Java monorepos, Roslyn SDK, CompDB)

- Go module discovery skips `_examples` / `examples` / `example` so Safe
  `GOPROXY=off` binds do not fail on nested demo modules (chi).
- Java source walk ignores Guava-style alternate universes (`guava-gwt`,
  `guava-testlib`, `guava-tests`, `futures`, `android`, `benchmark`,
  `src-super` / `test-super`, `*-gwt`).
- Roslyn: prefer .NET SDK MSBuild via Locator; disable invariant globalization;
  set `DOTNET_ROOT` / `DOTNET_HOST_PATH`; fall back to adhoc parse if MSBuild
  workspace load fails.
- `prepare_curl_compdb.py --in-place` prepares CompDB on pinned corpus trees
  without fetch/checkout (Docker Desktop path on Windows).

## 2026-10-15: Corpus campaign fencing and bind diagnostics

- Init `--subject` rehashes coverage observations; `verify_sources` fences on
  materialization identity only (declared subject is publication metadata).
- Campaign checkout restores dirty pins on reuse and requires peeled commit IDs;
  openssl submodule roots dropped (curl covers C overlay).
- `Cargo.toml` `workspace.exclude` includes `.tmp` so nested Rust campaign
  checkouts are not attached to the Entmoot workspace.
- `require_native` failure messages include native receipt limitations.

## 2026-10-15: uv.lock placement uniqueness for multi-version rows

- Auto `uv.lock` lock selections place packages as `name@version` so duplicate
  resolution-marker rows no longer fail `require_native` with
  `duplicate installation placement`.

## 2026-10-15: Init `--subject` keeps coverage and source fence consistent

- Stamping `subject` / `subject_aliases` on publish re-aligns embedded coverage
  snapshots and regenerates `coverage_id` so graph validation does not fail.
- `verify_sources` fences on materialization identity (repository, revision,
  digests, file set) and ignores declared subject metadata stamped after
  observation.

## 2026-10-15: Adapter naming cleanup and dead-surface removal

- Runtime produce/status field is `adapter_path` (was `solver_path`); unavailable
  native producers report `AdapterUnavailable*`. CLI `plan-analysis` uses
  `--adapter-pass`. Wire `ArtifactFormat::Resolver` unchanged.
- CLI `neighborhood` and `trace-seeded-paths` match MCP stems; `index` and
  `source-fetch` are hidden. Walk contraction type is `ShortcutOverlay`.
- Removed unused engine ports (`knowledge_exchange`, test-only
  `property_worklist`), unused pubs (`apply_display_refs`, `KnowledgeLookup`),
  `tools/native-solvers/` stub, and zero-ref maintainer scripts.

## 2026-10-15: Subject correspondence and inventory overlay

- `SourceSnapshot` carries optional `subject` / `subject_aliases`; compares use
  `CorrespondencePolicy` (subject_match / relative_occurrence / explicit_alias)
  instead of hard `repository_id` equality alone.
- Impact joins callables on subject-relative occurrence keys at compare time;
  inventorial `logic_address` hashing unchanged (v1 series).
- New `entmoot assess overlay` inventory presence delta; `package_api`
  compatibility stays an optional Bound/Fits rung with loud
  `consumer_missing_importer_refs:<coord>` preflight.
- CLI: `--subject` on init/bind, impact correspondence flags, `deps materialize`,
  optional `uv.lock` lock-instance assertions when lock_bindings are absent.
- Entry docs (README, docs map, AGENTS glossary, generated-schema notes, MCP /
  agent-runtimes) distinguish impact / overlay / compatibility rungs and subject
  correspondence for cross-publication compares.

## 2026-10-14: Closed dispatch promotion for unique binds

- Producers emit floor-backed `CallDispatch::Closed` for unique deterministic
  binds: rust free/inherent/UFCS; go free/concrete method; Roslyn
  static/sealed/base/partial/instance; javac static/private/final/super; clang
  non-virtual; Oxc lexical identifier; pyright unique fn/member; TypeScript
  Program unique function. Open gaps (trait/iface/virtual/decorator/callback)
  stay Open.
- Floors, resolver-adapters runbook, and `expectations_dispatch` reclaim
  `closed_dispatch: true` with policy + assumption fences. Join still validates
  unique `RepositoryCallable` + owner/policy/assumptions. Historical/host
  `#[ignore]` rows unchanged.

## 2026-10-14: unignore native claim cohort at floor pins

- Claim/deep/adapter tests no longer `#[ignore]` for missing adapters; they
  resolve catalog builds under `.tmp/adapter-build/` (pre-commit pins
  `ENTMOOT_ADAPTERS_ROOT`, clang Python/libclang, `GOTOOLCHAIN=go1.26.0`).
- Expectations keep Closed only for floor-backed rust/go free functions; other
  unique binds demote to non-Closed until a policy exists. Historical
  multi-toolchain / Acorn / known Windows jar+VFS rows stay ignored.

## 2026-10-14: precommit claim/dispatch cohort ownership

- `test_ownership.json` co-selects `dispatch_expectations`, all `callsites_*`,
  `joins_soundness`, `adapter_controls`, and `program_adapters` when any member
  or shared callsites fixture/support path changes.
- Named targets stay named (no package widen); pending `#[ignore]` Closed rows
  remain outside the gate. Testing runbook notes the cohort.

## 2026-10-14: align resolver tests to claim style

- Thin `callsites_*` so product Open/Closed rows live only in expectations /
  `dispatch_expectations`; keep language-deep producer/env controls.
- Joins reject hollow Closed; adapter Closed asserts require policy + assumptions.
- Testing runbook documents claim vs deep vs joins/adapters ownership split.

## 2026-10-14: dispatch suite reconcile (Closed fences, drop Phase harness)

- Expectations keep Open/Closed product fields only; strip suite `phase` /
  red-green taxonomy. Closed rows require `policy_id` + `assumption_contains`.
- Qualification moves to engine crate APIs (`dispatch_expectations` +
  `tests/support/callsites_expect.rs`); pending Closed uses stock `#[ignore]`.
- Remove CLI `dispatch_closed` / `dispatch_expect` subprocess qualify harness.
  `producer_cli` keeps loud missing-adapter only; smoke Closed lives on engine.
- Unique-bind Closed expectations retained (incl. JS/Python/TS); Open rows
  guard target-universe completeness, not logic-hash collision.

## 2026-10-14: machete, deny, cache, cast/`redundant_clone`

- Re-enabled Clippy `cast_possible_truncation` / `cast_possible_wrap` /
  `cast_sign_loss` and `redundant_clone` at warn (CI `-D warnings`); pedantic
  style allows unchanged. SQLite/protocol boundaries use `TryFrom`.
- Added `deny.toml` and `cargo deny check licenses --exclude-dev` (copyleft
  not allowlisted). Wired with `cargo machete` into CI and
  `pre_commit_check.py --banned-only`. Narrow machete ignores for
  `tree-sitter-language` and `rmcp-macros`.
- Local `.cargo/config.toml`; optional `sccache` via `RUSTC_WRAPPER` when on
  PATH in pre-commit. CI uses `Swatinem/rust-cache` plus `~/.cargo/bin` cache
  for pinned deny/machete/dylint/llvm-cov installs.

## 2026-10-14: call-site dispatch red suite and fixtures rename

- Moved authored resolver fixtures from `fixtures/native/` to
  `fixtures/callsites/` (`smoke/` denominators + per-construct leaves).
- Shared CLI harness `tests/support/dispatch_expect.rs`; smoke stays in
  `producer_cli`, Closed-red and Open invariants in ignored `dispatch_closed`.
- Renamed engine/CLI targets: `callsites_*`, `joins_soundness`,
  `adapter_controls`, `program_adapters`. Ownership key is `resolver_owners`.
- Precommit still does not clone corpora or run campaign/eval harnesses.

## 2026-10-14: Clippy pack + Dylint policy lints

- Workspace Clippy enables pedantic (noisy style/cast members allowed), plus
  unsafe/hygiene warns; keeps `unwrap_used`/`expect_used`/`unused`/`dead_code`
  deny. Generated Tree-sitter FFI is scoped `allow(unsafe_code)`. CI still
  uses `-D warnings`. `.gitattributes` forces `*.rs` LF so Windows rustfmt
  matches Unix newline style.
- New `tools/entmoot-lints` Dylint library enforces BLAKE3 identity (no sha2
  imports), Sync byte-Mutex ban, anyhow surface-only, and contracts vendor
  import ban. Pre-commit/CI run `cargo dylint`; Python keeps Cargo.toml sha2
  dependency bans only (AST regex retired).

## 2026-10-14: prior-art acknowledgements and Clang OOTB plan

- README adds an Acknowledgements section for leveraged tools/papers and
  comparative prior art (Multiplier, Trailmark, PASTA/blight, and deferred
  research pointers).
- PLANNING records Multiplier → OOTB Clang classification work (call vs
  address-of, macros, redecls, open dispatch) without adopting Multiplier’s
  index store. Clang adapter README and the resolver-adapters runbook document
  the same discipline for the shipped CompDB sidecar.

## 2026-10-14: Java family supersedes spoon (javac-only)

- Bind family id is `java` (was `spoon`); env is `ENTMOOT_JAVA_ADAPTER`.
  Artifact tree is `tools/resolver-adapters/java/` → `entmoot-java-resolver.jar`.
  `ENTMOOT_SPOON_ADAPTER`, `ENTMOOT_JAVA_ENGINE`, and Spoon CTModel are removed with
  no aliases—setting the retired engine yields `invalid_environment`. CTModel
  sources may exist locally under gitignored `.tmp/retired/spoon-ctmodel/` only.
  Producer receipt stays `javac_trees`. BindingFamily wire value is `java`.

## 2026-10-14: adapter profile rename

- Workspace readiness is `.entmoot/adapter-profile.json` (`entmoot.adapter_profile.v1`).
  Pre-release `native-profile.json` is refused (no alias). Status field and MCP gate
  are `adapter_profile` / `--require-adapter-config`. Evidence-lane terms stay
  `native` (`require_native`, `NativeReceipt`). Maintainer glossary added in AGENTS.

## 2026-10-13: native workspace profile and required-native bind

- `entmoot init` refreshes workspace adapter readiness from host discovery only
  (no downloads, no implicit adapter builds). `status` and MCP boot surface the
  profile; MCP hard gate is opt-in. (File/schema renamed to adapter_profile on
  2026-10-14.)
- `entmoot bind` requires native adapters for inventory languages by default; waive
  with `--structural-only`. Adapter resolve order: process `ENTMOOT_*` (exclusive) →
  profile paths → catalog. Host adapters remain the default; Docker Go is a non-default
  override. Entmoot still does not load `.env`.

## 2026-10-13: native solver pins and language floors

- Raised the workspace toolchain to **rustc 1.98.0** so in-process producers can
  track current crates: `ra_ap_*` **0.0.358**, **oxc 0.153.0**. Adjusted rust-analyzer
  macro-export detection for the HasAttrs API. When a workspace has no
  `Cargo.lock`, redirect Cargo's lockfile write under `.entmoot/native-cargo/` so
  bind does not mutate inventorial sources. Clippy 1.98 `as_chunks` cleanups.
- Upgraded Go sidecar to **golang.org/x/tools v0.52.0** (module Go **1.26**), Roslyn
  **Microsoft.CodeAnalysis 4.14.0**. TypeScript Program remains **6.0.3** (Program API
  ceiling; 7.x unqualified). Pyright **1.1.414** and libclang **18.1.1** stay current.
- Documented Entmoot native floors in `tools/resolver-adapters/floors.toml` and the
  resolver-adapters runbook: adapter pins, language surfaces, Closed-dispatch limits,
  and the Oxc vs TypeScript Program split. Deferred in PLANNING: migrate the typed
  JS/TS Program adapter to the TypeScript **7.1** native API for coverage beyond
  the 6.0.3 ceiling.

## 2026-10-13: measured shared payload hydration

- Typed SQLite reads decode the reconstructed JSON tree directly after canonical
  digest verification, removing a redundant text parse across inventory, knowledge,
  retention and durable-operation reads. Byte-based membership checks, semantic
  validation, publication transactions and recovery fences retain their behavior.
- Added regressions for payload/shared-metadata tampering, malformed metadata,
  typed contract rejection, numeric limits, signed zero and extensible provider
  detail. An opt-in retained-workflow probe separates loading, projection,
  validation, impact and pair comparison costs.
- Three alternating baseline/current 512-file/four-worktree development CLI runs
  reduced median edit → refresh → Git capture → conflict latency from 18.89s to
  18.18s (3.8%) and one-exit/resume latency from 21.40s to 20.19s (5.6%). Complete
  reports agree apart from the derivation fingerprint; saved output prefixes and
  source refs/indexes remain intact. Initial publication is outside those times;
  native workloads, release builds, build overhead and larger repositories are
  not qualified. No new execution machinery was added.

## 2026-10-13: bounded artifact revalidation projection

- Fixed artifact ledgers acquiring unrelated structural coverage, calls and
  declarations, which incorrectly consumed policy record budgets. Project attributed
  provider evidence directly while preserving independent and conjunctive support.
- Validate immutable input graphs once and reuse policy roots during preparation.
  Unselected artifacts in the same producer series remain explicitly unselected.
  Whole-policy commits, atomic ledger publication and recovery fences are unchanged.
- Added source/budget invariance, malformed-input, same-series sibling, SARIF trace,
  CycloneDX dependency/VEX, resource relationship and multi-family native support
  regressions. Existing rollback, killed-process, restart and concurrency fixtures
  exercise the same shared projection through durable publication.
- A 32-policy in-memory probe with 256 unrelated files reduced median preparation
  and revalidation time from 2.182s to 0.075s; selected semantic records remain 462
  regardless of those files. This is a narrow probe, not total workflow qualification.
- Three paired 512-file/four-worktree CLI probes measured edit → warm refresh →
  Git capture → conflict reporting at median 19.02s uninterrupted and 21.10s with
  one stop/resume (+11%), preserving identical reports and source refs/indexes.
  Refresh (7.86s) and comparison (6.91s) dominate. Planning now targets those shared
  costs; this pass adds no checkpoint granularity or scheduler capability.

## 2026-10-13: shared scheduler profiles and leased FIFO admission

- Added repository-scoped, generation-checked admission policies for concurrent
  slots, native-binding slots and weighted logical work credits. Existing workers
  retain ownership when limits change. Host isolation owns actual CPU/memory limits.
- Explicit run attempts preserve FIFO interest across process restarts and retries.
  A heavy head cannot be bypassed by smaller newcomers; abandoned interest expires.
  Controls remove tickets atomically, and disabled binding jobs yield ordering to
  enabled kinds. No transaction waits for capacity or spans producer execution.
- Added coherent bounded scheduler reads and shared CLI/MCP configuration, with
  configurable MCP operation workers/queue and policy updates in the control lane.
  Reads do not initialize storage or dispatch work. Rust-generated schema bounds,
  test ownership and v1 storage fingerprints cover the new contracts and layout.
- Expanded tests for 24 simultaneous clients, weighted/FIFO limits, abandonment,
  restart, disabled lanes, configuration CAS, pinned snapshots, repository isolation,
  bounded pages, storage-failure rollback and frontend restart parity. Three
  concurrent inventory worktrees match serial views; three canon workers retain
  independent publication scopes. Fixed a signed SQLite clock overflow in claims.
- Alternating 32/192-file probes measured at most 6.1% additional median scheduler
  cost. At 192 files, baseline/current medians were 2.170/2.153s uninterrupted and
  3.582/3.800s over eleven reopenings; median database growth was 0.2–1.1%. This measures
  the added scheduler cost on uncontended fixtures, not total recovery overhead or
  large-repository/build/memory qualification. Network collaboration remains deferred.

## 2026-10-13: source-qualified inventory and call-decision reuse

- Ordinary initialization and durable producers now reuse file-local syntax parts
  and structural call decisions across revisions and same-repository worktrees.
  Exact source bytes, relative paths, repository identity, policies and existing
  implementation fingerprints qualify reuse. No native evidence enters the cache.
- Call decisions trace their actual callable-index reads, including absent paths,
  ambiguous identities and caller membership. Added/deleted/moved targets invalidate
  affected decisions; unchanged target addressing allows body-edit cutoff. Replayed
  decisions create fresh snapshot-scoped claims through shared relation owners.
- Durable cache replacement commits atomically with graph sealing and checkpoint
  cleanup under the worker fence. Pinned cache reads and membership commitments
  reject torn or altered fragments. Parse/extraction/stitching reuse is observable
  in ordinary summaries and live durable progress, through shared CLI/MCP paths.
  Membership commits verified payload hashes; unchanged caches skip refresh and
  reusable fragments avoid redundant cloning and tree serialization.
- Added full-graph revision sequences, all-family reuse expectations, 144 finite
  index revision pairs, shared-worktree/repository isolation, source/policy/native
  contamination checks, cache-fragment corruption and atomic failure/restart tests.
  Existing concurrency, retention, process-exit and source-drift controls remain.
- The disposable cache is latest-only. Source observation, cache hydration, callable
  index construction and final assembly still require whole-operation work. Network
  collaboration and cross-repository reuse remain out of scope; the host continues
  to own pinned tooling and environment isolation.
- Alternating 192/1,024-file CLI probes measured 3–19% median overhead against the
  previous implementation after fixing a warm-cache serialization regression.
  At 1,024 files, no-change initialization was 1.437/1.710s baseline/current;
  cold initialization was 15.845/16.674s. Main database growth was 13% initially
  and 5% after two edits. These authored fixtures stay within the 25% operational
  allowance; compilation, memory and larger workloads remain unqualified.
- The optimized 192-file recovery probe measured 0.574s inventory/registration,
  0.977s durable execution and 1.844s over 20 reopenings. Stored size was
  11.69/18.03MB (+54%); ordinary/durable outliers reached 2.36/2.79s. Durable
  execution remains an explicit bounded opt-in recovery exception.

## 2026-10-13: durable inventory extraction and call stitching

- Added separate bounded file frontiers for parsing, syntax extraction and
  cross-file call stitching. Completed parts survive restart without replay;
  stitching uses a complete callable index shared across batches in each run.
  Ordinary and durable inventory share the extractor and resolution owners.
- Private part membership, phase cursor and worker fence commit atomically.
  Retention preserves live parts; sealing/cancellation releases membership.
  Source checks and final registry publication remain separate, so partial graphs
  never become discoverable. The v1 storage layout fingerprint fences old stores.
- Expanded production fixtures across language families, cross-file imports,
  ambiguity, skipped/invalid inputs, source drift and zero-time budgets. Added
  failed-write rollback, saved-prefix non-replay, premature-publication rejection,
  abrupt process exit, collection, stale-worker and CLI/MCP restart controls.
  Rust owns regenerated schema descriptions and test ownership covers shared code.
- Fixed Python analysis markers that omitted relative alias-only imports and
  every name after the first in a multi-import statement. Grammar-field traversal
  now preserves all targets/aliases, including parenthesized Unicode imports;
  duplicate aliases stay ambiguous. Existing structural normalization is unchanged.
- The three-run 192-file/576-callable probe measured median 0.553s ordinary,
  0.895s durable and 3.504s with 20 one-step reopenings. Main database size was
  11.70MB / 16.16MB (+38%, before collecting unreferenced payloads). The added
  342ms durable / 2.951s repeated-restoration cost exceeds 25%; this remains an
  explicit bounded opt-in recovery tradeoff, not ordinary overhead qualification.
- Source reobservation, configuration inventory, index hydration and final graph
  assembly/validation remain whole units. Cross-revision affected-frontier reuse,
  finer native recovery and large-repository performance qualification remain
  planned; network collaboration remains out of scope.

## 2026-10-13: compositional clone summaries and scope coverage

- Replaced repeated clone token walks with shared bottom-up syntax, alpha and
  parameterized summaries. Binding declarations and uses carry distinct positions;
  persistent balanced maps, small-to-large merging and tagged positions reuse
  unchanged work across bounded statement windows. Derived hashes change under
  the existing implementation fingerprint; structural identities remain unchanged.
- Fixed statement-window alpha hashes that erased names captured outside the
  extent. Rust now resolves nested blocks, sequential shadowing, initializer
  visibility and scope exit. Unlowered C/Go declarations and additional loop/pattern
  binders remain explicitly unavailable. Fixed missing Go statement-list windows.
- Added independently authored cross-family source relationships, capture and
  literal-partition regressions, admission parity, 139,776 explicit-position oracle
  cases, persistent-map insertion/update checks and an 8,192-binding stress fixture.
  Rust contracts own the regenerated schema descriptions; test ownership includes
  the new shared modules. Closures and broader family scope lowering remain open.
- The local 32/256/1,024-declaration probe halved inventory-plus-window time
  (12.4/106.5/434.4ms to 6.3/49.6/195.9ms). Callable-only totals rose 26–29%
  (2.3/18.8/74.1ms to 3.0/23.7/95.2ms). This is a bounded explicit producer
  tradeoff for scope-aware reuse; callable-only optimization and ordinary
  build/large-repository qualification remain planned.

## 2026-10-13: durable native family execution

- Added explicit binding plans and shared CLI/MCP workers over frozen input views,
  source manifests, workspace generations and host-attested environments. Each
  family runs through the ordinary native pipeline in a controlled helper process
  with private scratch and Cargo/Go caches. Setup remains host-owned; no tools
  install during execution and structural inventory stays compiler-free.
- Graph prefix, normalized attachment, attempt receipt and cursor commit together.
  Completed families survive restart; failed/budget-limited attempts preserve
  useful facts and pause without advancing. Final publication is atomic and
  source/generation checked. Partial native coverage remains explicit.
- Added lease renewal that preserves progress and rejects expired/revoked workers.
  Pause/cancel stops controlled descendants. Nested Unix adapter runners retain
  their outer process group; already-reaped groups are successful cleanup.
  Abrupt parent death still requires host job cleanup. Generic checkpoints now
  reject attempts to bypass producer/revalidation evidence-publication owners.
- Added real-process serial/restart/concurrent branch fixtures, unavailable-tool
  retries, source/environment/generation fences, receipt-write rollback, heartbeat
  and stale-worker controls, MCP restart routing and opt-in descendant cancellation
  tests. Family identity/dispatch is shared with ordinary binding; generated schema
  and test ownership follow the new contracts.
- Within-family recovery, typed canon, configurable scheduler resource profiles
  and large-repository/build qualification remain planned. Networking and
  cross-repository collaboration remain out of scope.
- The alternating 32-file Windows probe measured medians of 91ms ordinary,
  376ms durable and 456ms with a restart; one durable sample took 1.51s.
  Database growth was 1.20MB ordinary / 1.31MB durable (+9%). The extra median
  285–365ms exceeds 25% on this small lexical workload and stays an explicit
  opt-in tradeoff for saved native-family work and attributable failed attempts.
  It does not qualify large repositories or cold/warm build overhead.

## 2026-10-13: durable conditional revalidation

- Added shared CLI/MCP submission, bounded execution and completed-ledger reads
  over frozen retained repository views. Each policy commits actual evidence,
  applicability, output and cursor atomically; the final step seals publication.
  Record/time/step exhaustion preserves completed work and permits larger
  allowances on resume. Native producer execution and proof checking are excluded.
- Reused ordinary policy validation, input roots and KnowledgeRepository owner
  mutation. Fixed evidence-reference ordering that changed after SQLite reload,
  making receipts deterministic across in-memory and retained graphs.
- Added real source/SARIF tests for changed, stable, fresh and unselected producers;
  serial/restart/concurrent equivalence; cumulative budgets; cancellation and
  source drift; storage-failure rollback; process termination during publication;
  worker/view/snapshot fences; collection; and CLI/MCP parity. Private ledgers
  remain sealed between slices and isolated across jobs.
- The 32-policy Windows probe measured median 0.40s synchronous / 0.447s durable
  execution (+12%), with a durable outlier of 2.31s. Single-policy restart slices
  took 2.26–4.75s. Median database growth was 1.36MB synchronous / 1.97MB durable
  (+45%, including saved policy reports). This explicit recovery workflow exceeds
  the ordinary allowance for frequent restarts and storage; it remains opt-in,
  bounded and resumable. Larger-scale/build qualification is still open.

## 2026-10-13: durable ordered Git integration

- Added explicit ordered integration requests and shared CLI/MCP execution for
  frozen same-repository workspace states. Cumulative prefixes catch conflicts
  missed by comparing only the initial tree against each member. The first
  conflict stops with remaining members explicitly untested.
- Retained deterministic synthetic merge ancestry in private incremental Git
  packs. Pack, cursor and output publish atomically under the worker fence;
  restart restores completed work without replaying merges. Storage/time/step
  exhaustion pauses safely, and execution allowances can be raised on resume.
- Shared isolated Git execution with pairwise observations. Source refs/index
  remain untouched; original history and the host's pinned Git remain required.
  Completion/cancellation release private packs while retaining attributable
  reports. Network transport and combined native/build qualification stay deferred.
- Added real-Git cumulative, clean-union, ancestry, concurrent and abrupt-exit
  fixtures; unavailable-history, tool-drift, corrupted-pack, rollback, idempotence,
  stale-worker, retention and CLI/MCP restart controls. Committed binary/deletion/
  Unicode inputs restore to an independently authored expected tree.
- Full validation exposed Windows paging-file exhaustion from CPU-count linker
  fanout. The hook now defaults to at most six build jobs on Windows while
  preserving explicit `CARGO_BUILD_JOBS`; a regression covers host overrides,
  smaller/unknown CPU counts and other platforms.
- A three-run alternating Windows probe (Git 2.51.2) measured median 7.71s for
  eight members in one run and 14.99s in eight one-merge slices, with identical
  output and 4.1KB peak saved Git deltas. Initial capture of nine workspaces took
  25.39s: combined totals were 33.10s/40.37s (+22%). Frequent restoration adds 94%
  to warm execution in this fixture; this is a bounded opt-in recovery cost.
  The 1.24MB store included all six executions. Larger-scale/build qualification
  remains open; the probe is reproducible from the revision-evaluation runbook.

## 2026-10-13: retained precommit Git observations

- Added explicit bounded raw working-tree captures, atomically attached to a
  workspace generation with source/index/HEAD/file fences and shared retained
  bytes. Frozen batches retain exact capture IDs and Git execution mode.
- Added isolated synthetic-tree replay and Git merge observations for uncommitted
  branches through CLI and MCP. Reads remain passive; no source index/ref changes,
  hooks, filters, custom drivers or network layer are introduced.
- Added real Git fixtures for conflicts, independent edits, source drift,
  binary/CRLF/Unicode/add/delete inputs, ignored tracked files, CAS/idempotence,
  concurrent capture, unsupported index states, retention and frontend restarts.
- Closed a preexisting low-level queue gap: conflict workers now verify that
  repository identity and view pins match their frozen inputs before claiming a
  lease. A regression first reproduced execution without the required pins.
- Documented the host's Hermit-like tool-environment prerequisite. Raw capture
  does not apply clean/EOL filters or index-only selection; sparse worktrees,
  submodules and ordered integration sets remain explicitly outside this slice.
- A single local Windows probe of 67 paths took 2.63s to capture and 1.18s to
  replay/compare a conflicting pair; its store occupied 1.31MB. Capture and Git
  comparison are explicit opt-in work. Large-repository and overhead qualification
  remain open; these absolute costs are not a general performance guarantee.

## 2026-10-13: file-level canon recovery

- Added resumable canon file folds with cumulative pattern/byte limits, frozen
  inputs and private partial results. Ordinary and durable production share one
  file worker and final claim assembly; pause/restart never replenishes a budget.
- Consolidated inventory/canon file membership, transactional cursors, worker
  fences, retention and cancellation cleanup in one repository owner.
- Rejected cached folds whose source, language or pattern count disagrees with
  the recorded inputs, closing a possible pattern-counter underflow.
- Added multi-language claim/edge equivalence, cumulative-limit and cache-budget
  regressions, abrupt-exit/collection recovery, stale-worker rollback, concurrent
  canon publication and CLI/MCP process-restart coverage.
- A five-run alternating local probe (192 Rust files, 1,536 callables, optimized
  test profile) measured median canon submit/run time of 1.02s versus 0.94s (+8.5%)
  for the previous whole-canon worker. All file claims and inventory membership
  matched. Uncollected store size was 29.86MB versus 24.12MB (+23.8%); larger-scale
  and build-time qualification remain open.

## 2026-10-13: file-level inventory recovery

- Added frozen file manifests and fenced, transactional parsing checkpoints.
  Resume skips completed files while preserving unsupported/read-failure outcomes.
  Shared decoding, hashing and final stitching keep sliced inventories equivalent
  to ordinary builds; incomplete files never imply absence.
- Added configurable files per step and cooperative time admission to CLI/MCP
  producer execution. Revoked workers stop at batch boundaries. Retention preserves
  active checkpoint payloads; preparation and cancellation release private progress.
- Added multi-language/CRLF slice equivalence, zero-budget, source drift,
  abrupt-exit, collection, Unicode/ignore scope and stale-worker/batch rollback
  coverage. A five-run alternating local probe against `abc262d` (192 Python
  files, 1,536 callables, optimized test profile) measured median complete
  source/submit/run time of 0.87s versus 0.78s (+11%), with identical inventory
  membership. Uncollected store size was 13.05MB versus 10.51MB (+24%). This
  fixture does not qualify large-repository performance or full build overhead.

## 2026-10-13: recoverable structural producer publication

- Added durable inventory and structural canon workers with frozen source and
  target identities, sealed prepared results and atomic registry/job completion.
  CLI and MCP share submission, execution and source observation. Paused work
  survives process exit; completed receipts replay without rerunning production.
- Producer results retain digest-verified working-tree source through the shared
  source store, including when an identical imported view was already sealed.
- Fixed concurrent graph registration by importing knowledge and inventory in
  one transaction. Prepared views stay outside collaborative reads and new inputs
  until publication. Source drift, expired workers, cancellation and competing
  target generations cannot publish a stale result.
- Replaced workspace lock-file ownership with an OS lock that releases on process
  exit while preserving the sidecar identity. Added abrupt-exit, restart, CAS,
  cancellation, concurrent import and frontend recovery fixtures.
- Reserved bounded MCP control capacity so pause/cancel/resume can revoke workers
  while every producer slot is occupied, without consuming interactive read slots.
- Checkpoints currently cover complete inventory/canon production; file-level
  progress, native binding and revalidation recovery remain explicit planned work.

## 2026-10-13: local branch collaboration and durable execution

- Added local workspace registration, immutable retained-view seals, generation
  checked publication/retirement and frozen peer capture. CLI and MCP share the
  same engine operations; networking and cross-repository collaboration remain
  outside this implementation.
- Added durable, idempotent operation submission, cross-process admission,
  prerequisite checks, fenced leases, pause/resume/cancel and paginated pair
  reports committed with their cursors. Concurrent/restarted branch comparisons
  match serial reports. Structural overlap remains a review signal, with explicit
  unknowns; optional committed Git checks use isolated objects and built-in merges.
- Added opt-in durable path/neighborhood frontier and replay checkpoints with
  publication, checksum, sequence, capacity and expiry fences. Revision campaigns
  now pause between completed units and resume qualified receipts; OS-owned run
  locks recover after process exit.
- Fixed concurrent first-store/queue initialization and duplicate legacy work
  submission races through shared bounded SQLite WAL startup. Fixed alpha fingerprint collisions involving hoisted/function-local
  lookups and Java/C# initializer self-references. Unsupported binding scope stays
  explicit instead of producing an equivalence signal.
- A local 10,000-row storage probe (seven alternating runs, optimized test profile)
  measured median writes +4.2%, reads +2.0%, unchanged indexed query time and
  storage +0.6% against the pre-change source. A 424-node fixture completed 32
  conflict comparisons with a fresh connection per slice in 1.96s. These are local
  probes; full build and large-repository qualification remain outstanding.
- Fixed hook-local Git variables leaking into fixture repositories and pinned
  the campaign Python selected from PATH, preserving explicit overrides.
- Expanded authored multi-language/CRLF branch fixtures, concurrent workers,
  restarts, stale generations, large paged outputs, immutable-view mutation
  attempts, query corruption/retries and CLI/MCP controls. Producer-wide staged
  recovery, working-tree Git simulation and integration sets remain in PLANNING.

## 2026-10-12: binding failures stay unresolved

- Roslyn, Spoon/javac, Clang and rust-analyzer no longer label missing symbols or
  overload candidates as `indirect_call` / `dynamic_dispatch`. Those gaps require
  native evidence of indirection (delegates, open virtual/interface, function
  pointers, trait containers). Ambiguous candidate targets project as
  `ambiguous_target` even if a mechanism string is misattached.
- Added independently authored Roslyn and Java controls for the distinction.
- Project qualification under `.tmp/native-audit/`: Rust checkout bind cleared the
  prior rust `budget_exceeded` under `--no-default-features` (partial, 477s);
  commons-codec and CliWrap produced; curl CompDB hit the 3600s ceiling.
- Release `query_scale_bench` on the 107,392-node publication kept semantic parity
  across scan/indexed/lazy arms; results under `.tmp/native-audit/results/perf/`.

## 2026-10-12: shared solver terminology

- Established adapter-boundary terminology audits in AGENTS. Shared native fact
  types now use `Native` names, including compiler-free typechecker observations.
- Unified indirect-call gaps under an extensible `indirect_call` mechanism value.
  Provider aliases normalize once; unknown reasons remain unresolved bindings.
  Receipts and relationship frontiers share categories and retain attribution.
- Separated ambiguous bindings from indirect dispatch and named coverage's
  aggregate `unresolved_or_ambiguous`. Aligned budget and unresolved-binding terms
  across relationships, coverage and query projections; regenerated contracts.
- Removed canon's call-name heuristic for reflection and dynamic dispatch.
  Unsupported call lowering reports `unsupported_semantics` without inferring
  runtime behavior. Added normalization, projection and unknown-mechanism controls.

## 2026-10-12: native audit defaults and dynamic observations

- Unified the native producer wall-clock default at one hour; explicit budgets
  remain available for shorter runs and deadline controls.
- Shared structural scanning now honors root-local and nested `.gitignore` rules
  by default, pruning excluded directories before reading. Full, cached and
  snapshot scans share this policy; coverage records exclusions without counting
  them as analyzed files. Explicit roots remain independent of ancestor ignores.
- Renamed receipt call counts to `observed_calls` and added counts of existing
  provider call-gap reasons. Partial dynamic dispatch stays processable and
  visible alongside static targets, independently of environment failures.
- Fixed Pyright scans of explicit checkouts under ignored ancestor directories
  while preserving local ignore rules and cache exclusions. Python callbacks and
  TypeScript call signatures now retain explicit indirect-call gaps. Added controls and
  documented project-scoped build-system preparation for local audits.

## 2026-10-12: shared query indexes, bounded shortcuts and resumable reads

- Interned relationship addresses and shared deterministic adjacency, scope
  transitions, iterative SCC and union-find mechanics. Conservative cached chain
  contraction preserves original distances, edge identities and current claim
  classification. Cycles, branches, parallel occurrences and context boundaries
  remain visible; bounded construction falls back to ordinary traversal.
- Added authenticated loopback MCP serving and explicit CLI backend selection,
  bounded worker admission and validated publication refresh. Resumable path and
  neighborhood searches retain bounded frontier/replay state, fence requests and
  evidence visibility, and separate execution continuations from output pagination.
  No producer startup, claim propagation or new public shortcut relationship.
- Batched original-edge/proposition claim reads, excluded canon payloads from
  assessments, indexed provenance identities, and deferred clone member copies to
  returned pages. Fixed rejected-candidate work charging and exhausted assessment
  truncation; call components now accept explicit node/edge limits.
- Removed unused visitor state and duplicate scope setup; bounded nested callable
  containment and lazy syntax traversal preserve existing identity fingerprints.
  Added independent graph oracles, cache/continuation controls, transport parity and
  separate query-scale timing/work/retained-memory measurements.
- The saved-source replay preserved 107,392 structural IDs and all 340 clone
  groups/fingerprints. The authored 120-hop chain reduced expansions from 126 to
  6 and edge inspections from 360 to 3 while reconstructing all original edges;
  the structural parallel-call control remained uncontracted.

## 2026-10-12: conservative alpha name boundaries

- Retain caller-bindable Python/C# parameter names in alpha and parameterized
  fingerprints while continuing to normalize positional-only/variadic Python
  names and unambiguous locals. Known name-observing constructs, including
  reflection builtin references, debug f-strings, C# `nameof` and preprocessor
  syntax, leave these tiers explicitly unavailable through existing fields.
- Added authored positive/negative controls in the existing clone test target.
  No new schema objects, provider layer or traversal; canon's explicit scalar
  assumptions remain unchanged. The implementation fingerprint fences prior
  materializations; unchanged Rust stream fingerprints remain pinned.

## 2026-10-12: explicit canon production and retained clone reads

- Added opt-in `bind --experimental canon`, typed file/graph observations,
  occurrence-to-form membership, native scalar sites, bounded term DAGs and
  hashed obligation ingredients. Dynamic, unresolved, unsupported and exhausted
  work stays explicit; no SMT solving or behavioral-equivalence claims.
- Clone operations require compatible retained canon and provide an actionable
  bind nudge when unavailable. Queries share the engine read path, select file
  shards before hydration, and do not read or parse the working checkout.
- One parsed file/index feeds canon production; syntax/alpha/parameterized hashes
  share a traversal. Exact-input reuse avoids repeating unchanged pure folds
  after fresh native collection. Existing snapshot and atomic publication fences
  retain provenance and historical applicability.
- Promoted common source access above ECMAScript, removed unused normalization
  helpers and empty visitor branches, and preserved pinned clone fingerprints.
  Scalar coverage is deliberately bounded; conversions, effects, unqualified
  calls, floating-point and checked/undefined arithmetic remain partial.

## 2026-10-12: source scan boundary

- Exclude conventional .NET `bin`/`obj` outputs beside C# project files from
  inventory and source fences, recording the excluded paths. Authored directories
  with those names elsewhere remain visible. A repository probe exposed Roslyn
  updating generated assembly metadata during workspace loading; an adversarial
  scan fixture now covers that case and unaffected siblings.

The solver-enriched repository probe at `ff05308` returned 104 callable clone
groups across 322 crate files. Its 1,891 unused candidates remain review signals:
sampled test entrypoints, trait hooks and aliased helpers have real consumers.
All 60,858 inventoried callsites were accounted for; qualified closed dispatch
covered 2,456 Rust and 44 Go sites. Missing project inputs and unselected/dynamic
behavior remain open. Accounting is not graph completeness.
End-to-end native replay fixtures now declare the Rust/Go free-call closure
expectations per occurrence, validate their evidence pointers and audit counts,
and require failed, indirect and interface evidence to remain open.

Five fresh/warm assessment repetitions on an 11,607-node visitor subtree and nine
local language fixtures preserved scan/indexed/lazy parity. Subtree unused and
no-change test-impact reads took roughly 0.42–0.56 s; callable clones 0.12–0.13 s.
No-op SQLite synchronization made zero row mutations. Revision-scoped updates
still churn rows despite stable proposition IDs; no storage savings are claimed.

## 2026-10-12 — Shared analysis and conservative assessment foundations

Source-byte changes retain their test owners even when structural hashes match.
Focused hooks preserve named targets, unit/doctests, downstream consumers and
explicitly owned ignored campaigns. Fixture ownership includes both revision-pair
runners. Compatible package checks are batched, native checks follow affected
owners, and content-keyed manifest parsing avoids repeated TOML work.
The hook also avoids a separate CLI/producer rebuild after CLI integration tests;
unit/doctest-only selections still require that build.

Seven visitors share traversal, depth accounting, comments and content checks.
JS/TS share their identical declaration, import, parameter, body and lexical-scope
helpers; dialect-specific import/parameter rules stay at the language boundary. All 362 pinned
inventory inputs retained identical output digests; five fresh-process and five
warm timings stayed within baseline variability.

CLI/MCP share `assess clones`, conservative unused-candidate and two-sided test
impact operations. Targets retain ownership and fixture reasons; recursive
islands, outside-inventory callers, missing ownership and exhausted budgets have
local controls. Unused completeness and test assertion coverage remain explicitly
unqualified. Rust native evidence now qualifies direct free-function dispatch
under the selected build context, with exact-join and real compiler controls.
Go also qualifies direct free-function calls; methods and indirect calls stay open.

Typed usage, compiler diagnostics and target/execution observations share existing
artifact ingestion and SQLite storage. Exact source/occurrence joins, replay,
restart and lazy/eager parity are covered locally. Native function-value references
can suppress unreferenced candidates; native Rust controls preserve inactive cfg
and indirect-call limits. Explicit `validate-tests` uses shared process-tree cleanup,
tool selection and source fencing; uninstrumented process success never claims
coverage. Installed sidecar controls follow their catalog owners in the hook.

Targeted fault injection detected loss of base-revision paths and non-call-use
suppression. Clone traversal reuses descendant indexes and avoids per-node child
vectors: on 362 pinned inputs, five fresh-process/warm medians fell from
0.450/0.450 s to 0.402/0.406 s in the final batch, with identical fingerprints and effectively unchanged
sampled peak memory (~49 MB). Filesystem caches were not flushed; build/native
startup was excluded. Broader deployment or storage savings are not claimed.
An adversarial native control caught IDE name resolution inside disabled Rust
constants/statements; a function-value use now requires an inferred active HIR
expression. Inactive syntax cannot suppress an unused review candidate.

## 2026-10-11 — Runtime discovery, coverage and clone candidates

Native launchers select installed hosts and record their executable, version and
selection source. Python can discover the selected interpreter's import roots;
explicit broken settings fail without fallback. Source and coordinate attachment
use strict UTF-8 with original bytes and line endings. Import environments remain
separate from publication source, including project-local virtual environments.
Shared discovery prunes scratch before opening compiler projects. Windows Rust
path joins, Roslyn SDK loading and explicit Clang compilation-database selection
have focused regression controls. Existing native installations enable the optional
pre-commit Rust/Oxc lane and matched CLI/producer build.

`audit-calls` reports each producer/build-context denominator and potential
reachability from supplied occurrence roots. The repository probe inventoried
4,701 callables and 59,842 explicit callsites, with no source-inventory diagnostics
or normalized producer omissions. This is accounting, not dispatch completeness:
current producers leave dispatch unqualified. Oxc, Go, Roslyn and Clang reported
OK; Rust and Java were partial. Python retained missing-import/syntax failures,
and TypeScript Program lacked a project configuration.

The shared `clones` query adds bounded exact, syntax, conservative local-renaming
and literal-parameterized fingerprints over real occurrences and statement spans.
Written types remain distinct; inferred types and behavioral equivalence are
unassessed. Family, scope, interpolation, Unicode/CRLF, budget and stale-source
controls cover the new path. A bounded visitor query examined 3,925 patterns in
24 files and returned 504 candidate groups before pagination; overlapping groups
are review leads, not additive savings. Local probe evidence lives under the
ignored solver-coverage task root.

## 2026-10-11 — Retire the legacy campaign framework

Moved the remaining useful accounting, npm-location and native pipeline controls
into Cargo. CI now uses these suites instead of the old horizon/dogfood, LSP
prototype and duplicate TypeScript runners. Removed their unused fixture copies,
corpus envelopes and historical signoff gates. The SQLite harness and one
original-upstream configuration remain; living procedures are in runbooks.
Adapter/profile contents participate in campaign input identity. Current tests
cover failed native attempts, exact occurrence selectors, source drift, replay,
empty/incomparable bindings and both caller-pagination dimensions.

## 2026-10-11 — Fixture and evidence consolidation

Retired the obsolete compatibility packet cluster and moved the 586 qualified
whole-file controls beside Cargo tests with explicit expectations. The diagnostic
validator remains available; 262 unqualified historical labels and their inference
helper are gone. Cargo now owns native sidecar/historical-mode and SQLite-reader
controls. Relative Clang header locations use each compilation directory, with a
regression for same-named headers in distinct directories. Adapter builds run in
scratch copies. Grammar inputs have a separate, smaller checkout map; product
campaigns use original upstream configuration and can run typed Entmoot checks.

A bounded Pydantic probe reconciled all 2,337 inventoried callsites and verified
exact declaration ranges for 632 attached callable targets. Its environment had
35 missing-import diagnostics and 117 unmatched provider callable ranges; it
remains partial and is not a bind-accuracy score. A sequential Flask operating
probe reused all 83 parsed files on no-change and 82 after each one-file edit.
These local observations do not establish a performance improvement or broad
native qualification.

## 2026-10-11 — SQLite campaigns and focused native controls

Added pinned original-repository builds and bounded lock-derived dependency
selection to the existing SQLite harness. Atomic operation results support
retries without duplicate raw-provider archives. Cargo now owns campaign and
Program-provider controls; live Pyright, TypeScript, historical TypeScript/TSX
and npm-provider controls passed. Active source fixtures moved beside their
Cargo owners. Retired model/retrieval experiments, trace hooks and duplicate
architecture catalogs were removed. A bounded Flask/Jinja build and structural
publication succeeded; it does not qualify native binds or unmapped dependencies.

## 2026-10-11 — Declared relationship contracts

Added source-declared callable parameters/results, direct members, type syntax,
written defaults and modifiers on production occurrences. SQLite inventory,
shared inspect reads and revision impact retain these objects. Native resolution
and runtime behavior remain separate; complex declarators and missing observations
are explicit limitations. Compiler-free family fixtures, nested Unicode/CRLF,
parameter mode/default/optional controls and retained-view lifecycle tests cover
this change. C++ controls remain experimental.

## 2026-10-11 — Maintenance scope reconciliation

The v1 pass centers on structural contracts, retained knowledge, revision impact
and the existing SQLite campaign harness. Earlier universal audits, exact-receipt
retention, global cleanup and October certification gates are historical, not
current obligations. Backup implementation and a separate survey product are
out of scope. Current design and ontology guidance now lives in ARCHITECTURE;
operating procedures live in runbooks and concise rules.

## Previous work

Earlier releases established exact native occurrence attachment, independent
producer support, storage transactions, bounded traversal and revision impact.
Detailed dated records remain in Git history. Current claims and limitations
belong in the architecture and the executable tests, not an accumulating ledger.
