# Testing

Use the narrowest suite that owns a behavior, then the workspace checks for
shared changes. `scripts/test_ownership.json` maps changed owners to Cargo targets.
Changed source bytes retain those targets even when structural hashes match:
line-sensitive macros are one reason syntax identity cannot authorize a skip.
Named integration targets stay named; source changes include unit/doctests and
downstream packages. Missing ownership widens to workspace checks.

- Structural: `revision_pairs`, declaration/port inventory and language suites
  exercise production visitors without compilers. Select exact occurrences and
  author expected relationships independently.
- Joins: `joins_soundness` and adapter parity use authored provider inputs with
  real inventory. They qualify attachment, not compiler semantics.
- Call sites / dispatch: authored fixtures under
  `crates/entmoot-engine/tests/fixtures/callsites/` (`smoke/` denominators and
  per-construct leaves). Split ownership:
  - **Product claims** — `expectations_*.json` + engine `dispatch_expectations`
    (`qualify_*` via crate APIs). Fields: `closed_dispatch`, `dispatch_gap`,
    `policy_id`, `assumption_contains`, site/owner/target selectors, `outcomes`.
    Historical/host `#[ignore]` rows stay for multi-toolchain and known
    Windows jar/VFS flakes only; Open rows must not claim Closed.
  - **Language-deep** — `callsites_*` keep producer/environment specials (canon,
    cfg/edition, package_api, raw sidecar, ESM/CJS quirks). Do not re-assert
    product Open/Closed rows already covered by expectations.
  - **Joins / adapters** — `joins_soundness` (authored provider) and CLI
    `adapter_controls` / `program_adapters` keep layer-specific controls; use
    the same Closed fence vocabulary (nonempty policy + assumptions).
  Pre-commit co-selects the whole claim/deep/joins/adapters cohort whenever any
  member, `fixtures/callsites/`, or `support/callsites_expect.rs` changes (named
  targets stay named; source under `crates/entmoot-engine/src/` still runs the
  package). Claim/deep/adapter tests resolve catalog adapters under
  `.tmp/adapter-build/` when `ENTMOOT_*_ADAPTER` is unset; build with
  `scripts/build_resolver_adapters.py` (and program builders) at the floor pins.
  Floor-backed Closed policies are listed in `tools/resolver-adapters/floors.toml`
  (producers emit Closed; join validates fences). Historical multi-toolchain /
  Acorn rows stay `#[ignore]`. Campaigns stay opt-in.
- Storage/application: engine store tests and CLI workspace/artifact suites
  exercise transactions, restart, producer isolation, historical reads and impact.
- Live resolver controls: explicit validation against configured adapters. A
  missing tool or unsupported mode remains unavailable; do not report it as a
  successful control. Campaign/eval harnesses (`campaign_harness`, `evals/`) stay
  opt-in and outside precommit; they must not clone remote corpora for the gate.

## Claim-oriented assertions

Write the failing control before fixing or extending the shared owner. For each
minimal pair, state what must stay equal, what must differ and under which policy,
snapshot and provider/configuration scope. Test structural, occurrence, alpha,
parameterized, resolved and canonical identities according to their separate
contracts. Neither hash equality nor inequality establishes whole-program
behavioral equivalence or non-equivalence.

For nodes and edges, use production extraction/attachment with independent expected
subjects, directions, call sites, targets, provenance and uncertainty. Include
near misses, unsupported inputs, ambiguity and empty evidence. Minimize each fixture
enough to diagnose one invariant without dropping full graph, persistence, revision
and CLI/MCP controls that establish composition. Property/metamorphic/differential
checks should retain deterministic counterexamples and verify that the intended
mutation changes the expected assertion. Update test ownership when targets change.

Ordinary Rust tests belong in the corresponding crate. Calling them native Rust
tests does not qualify another language's compiler semantics: authored provider
records test joins/normalization; live adapter controls qualify their exact inputs.
Historical CI/CVE cases follow [revision-case qualification](revision-evaluation.md#historical-regression-case-qualification).

## Running checks

Run `cargo test --workspace --features entmoot-core/full-tests`, `cargo fmt
--all -- --check` and strict workspace Clippy before shared changes are complete.
Normal pre-commit hooks also check Markdown links, schema goldens, crate boundaries
and test ownership. Doctests are part of workspace tests.

`ENTMOOT_NATIVE_CHECK=auto` also runs Rust/Oxc controls for affected native owners
when a producer already exists under `target/commit-check/debug`. CLI integration
tests build the matching executables; a separate binary build runs only when that
lane was not selected. `on` explicitly enables native checks; `off` keeps structural checks only.
Compatible Cargo invocations share feature settings and whole-package tests are
batched. Native controls already included in those tests are not repeated.
Fresh checkouts do not acquire native dependencies automatically. Relevant installed
sidecar controls also run when their catalog owners change; invalid explicit adapter
or host overrides stay exclusive. Missing adapter/runtime controls print unavailable.
Use `off` to opt out; full CI remains authoritative. These controls qualify their
fixtures and selected environment, not every source configuration in the repository.

Keep small deterministic fixtures under their Cargo suites. Campaign runs use
the existing SQLite harness and `.tmp/<task>/`; test-only temporary directories
need not be retained. Remove obsolete tests with their consumers, preserving
useful assertions rather than their old scripts or byte-for-byte receipt format.

The `code_similarity` suite owns clone scope, capture and literal relationships.
Core summary unit tests compare both encodings with an independent explicit-position
oracle over 139,776 bounded cases, exercise all six-key insertion orders and retain
snapshots across an 8,192-binding stress fixture. These are finite tests, not proofs.
Run its ignored `clone_summary_cost_probe` with `--nocapture` for separate inventory,
callable-only and overlapping-window timings at 32/256/1,024 local declarations.
Timing is evidence for the fixture, never a correctness assertion or scale promise.

Inventory `producer_operations` fixtures reopen every parse/extraction/stitching
frontier and compare complete production graphs, including cross-file targets,
ambiguity, unsupported files and diagnostics. Repository unit controls cover
process exit, retention, stale workers, atomic rollback and premature publication.
The ignored `inventory_frontier_cost_probe` reports ordinary/durable/restarted
time and store growth for 192 files; final assembly is still a whole unit.
`declaration_initializers` also compares full cold graphs after target appearance,
body edits, moved definitions, ambiguity, deletion and importer rename. Reuse counters
have independent expectations. Resolver unit controls enumerate 144 index revision
pairs, including negative lookups and ambiguous fallback identities. Shared-store
producer tests exercise worktree reuse, repository isolation and rollback of cache
publication together with graph sealing. Core batch tests reject stale bytes,
renamed paths, mismatched normalization and native evidence in cached syntax.

The ordinary SQLite campaigns use local authored repositories. Keep optional original
upstream probes only for additional scale/environment behavior; the small rendered
resource fixture retains deployment/stitching/SARIF assertions beyond callable tests.

`validate-tests --graph <graph> --request <request.json> --out .tmp/<task>/execution`
runs declared argv with a process-tree deadline and verifies source identity before
and after execution. It records target process status, tool and build context through
the existing artifact lane. Runner names describe the intended framework; they do
not imply discovered tests, instrumented hits or assertion coverage. Supply attributed
instrumentation facts separately. Never remove owned tests because hits are missing.

The optional SQLite lane runs with Python dependencies installed:

```text
cargo test -p entmoot --test campaign_harness -- --ignored
cargo test -p entmoot-engine --test revision_receipts campaign_accounting -- --ignored
```

Traversal controls include an independently computed shortest-path oracle over
all five-vertex DAGs, contracted/original witness comparisons, scope and cycle
barriers, bounded caches, and continuation replay/concurrency/expiry tests. The
MCP `http_e2e` target builds the matching CLI binary and checks authenticated
cross-client replay alongside stdio parity. No elapsed-time assertion establishes
correctness.

`query_scale_bench` replays the same saved requests with acceleration disabled,
enabled and inventory-backed loading. It reports graph loading, engine construction,
query/index/evidence work, response serialization and conservative retained cache
bytes. Search timing includes witness reconstruction; evidence timing includes
classification and witness JSON construction. These counters are distinct from
process RSS and uncontrolled OS-cache effects.

Use `cargo run -p entmoot-engine --example query_scale_bench -- --chain-fixture
.tmp/query-scale-chain` for the authored 120-hop control. It retains the structural
extractor's parallel aggregate/site call edges as a noncontracting control and adds
single-occurrence dependency edges over the same production callable inventory.
Compare work counters separately from total latency: every returned witness still
hydrates and serializes the original edges.

After [adapter setup](resolver-adapters.md), live controls are `program_adapters`
and `adapter_controls` in the CLI crate, plus engine `dispatch_expectations` /
`callsites_*` (and CLI `producer_cli` loud missing-adapter). Producer suites need
`--features producer` (or engine `resolved-rust-analyzer` / `resolved-oxc`); use
`--include-ignored --test-threads=1` for adapter-backed and pending-Closed lanes.
Authored selectors live under `fixtures/callsites/`. Precommit never fetches
remote corpus blobs.

## Release lane ownership

The CLI `adapter_controls` and `program_adapters` targets and engine
`dispatch_expectations` / `callsites_*` require `native-tests`. They must not run in the clean
compiler-free lane or silently pass when tools are missing. The native workflow
provisions adapters and enables the feature explicitly; producer controls also
require `producer` on the CLI / the matching engine features. Known Windows VFS
and jar-classpath limitations remain named; Linux runs those explicit controls.

`call_boundaries` uses authored native observations over production inventory. It
qualifies source attachment, counting, context and attribution, not native compiler
semantics. `workspace_defaults` and MCP stdio tests own automatic storage, relocated
data directories, explicit refresh and restart behavior. Release packaging uses
`scripts/package_smoke.py`; workflows' named targets are checked against Cargo
metadata by `scripts/check_release.py`.

The ignored revision-campaign execution control requires the CLI `producer`
feature (in-process Oxc). Native CI invokes it explicitly; the other Python/Git
campaign accounting controls run in structural CI.
