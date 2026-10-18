# Local alpha operation and release

Target: `tim-gowan/entmoot`, the workspace version in [Cargo.toml](../../Cargo.toml),
five crates, v1 wire series. Confirm repository access and the target branch at
handoff. Resolve [current qualification blockers](../../PLANNING.md#current-qualification-blockers)
before preparing a clean copy; copying, committing, pushing and
registry publication are separate operator steps. Local success is not a claim
that GitHub CI, registry ownership or publication has succeeded.

## Local operation

The [README](../../README.md) owns executable quickstart and maturity.
CLI/MCP frontends automatically bootstrap missing structural inventory and use
one existing workspace knowledge repository with immutable publication views.
No ordinary database or publication flags are necessary. `-C` selects the source.
Durable state defaults to `~/.entmoot` (`--data-dir` / `ENTMOOT_DATA_DIR`
overrides the home root). Source-root namespaces under that directory prevent two
projects from sharing a current view. Structural publications and native artifacts
use that selected volume; invalid configuration fails before choosing storage.
An existing broken pointer is an error, never silently replaced. Repeated reads
reuse the current view; `init` refreshes explicitly and
`bind` adds native evidence.

Advanced graph/publication overrides retain reproducible historical selection.
Collaboration uses the same repository by default; native/inventory execution
through MCP still needs an explicitly configured source root. Reads of the query
service do not enqueue native production. First-use frontend inventory is a
separate preparation step; on large sources it can delay MCP startup.

A future remote location must be a transport at the local composition boundary,
not a database path embedded in portable requests. It must preserve snapshot
selection, source verification, producer isolation, atomic publication, history,
authentication and bounded reads. Reads and mutations reject URI locations.
No remote protocol or universal
subject/version catalog is claimed.

`boundaries` in CLI/MCP selects exact callee syntax or `*` patterns, optional path,
normalized outcome/reason and build context. Distinct sites and attributed
observations stay separate. Source spans plus content digest permit verification;
native targets retain provider details. Missing retained bodies stay explicit,
including empty filtered results. A future provider may use these exact scoped
selectors to add a claim; it cannot silently close the graph or rewrite identity.
Repository, dependency and native-library completeness are independent and all
remain unestablished without matching closure evidence.

## Corrections delivered from the audit

- Shared dependency materialization rejects path-shaped versions, overlaps,
  symlinks and special files; enforces depth/entry/byte budgets; uses capability
  directories, opaque destination keys and staged publication with cleanup.
- Package subjects are parsed/canonicalized at the shared contract boundary;
  the PURL cannot contain an embedded version.
- Overlay identity includes occurrence scope, kind and signature; duplicate
  declarations retain multiplicity. Presence never becomes Bound/Fits.
- Navigation verbs are thin CLI wrappers over the same typed MCP/engine service.
  Human terminal rendering and stable JSON preserve bounded-query semantics.
- Packages have exact prerelease dependencies, READMEs/licenses and package-local
  composed fingerprints. Installed adapter defaults never refer to build-machine
  paths. Explicit process adapter overrides remain exclusive, including empty
  values; invalid paths fail visibly.
- Structural CI excludes suites gated by the `native-tests` feature. Native jobs
  select dispatch and package-compatibility controls as well as adapter tests.
  Preflight discovers profiles through init's returned publication/store locator.

## Upgrade and recovery

Pin the prerelease and install with `--locked`; install the producer helper from
the same CLI package/version. Contract, derivation and layout fingerprints fence
incompatible stores. Alpha does not promise automatic store migration. Preserve
needed historical publications and use their matching reader. For a new reader,
select a fresh `--data-dir` and regenerate inventory/evidence from the same source
revision. Never edit a stored fingerprint to force admission.

An interrupted publication leaves the prior current pointer intact. Retry the
explicit operation after correcting the reported source/environment/budget error.
Retention remains opt-in with reviewed `storage-plan` / `storage-apply`; closing
active stores and preserving needed history precedes deletion. Native environment
failures and unsupported constructs remain different observations.

## Validation and release gates

1. Run focused controls, then `cargo test --workspace --locked --features
   entmoot-core/full-tests`, formatting, strict Clippy and doctests. Regenerate
   schema owners with their Rust examples; golden tests must match.
2. `python scripts/prepare_release_inputs.py --check` verifies package-local
   normalized release inputs. Regenerate after Cargo dependency/feature changes.
   `python scripts/check_release.py` checks ownership, links, versions and metadata.
   Run Python harness tests and the existing Dylint/license/unused dependency gates.
3. `python scripts/package_smoke.py` packages all five crates, builds only their
   extracted contents in an isolated workspace and compares implementation
   fingerprints. Its patches point exclusively to extracted siblings, never the
   checkout; this supports first releases before dependencies exist on crates.io.
   Native CI repeats it with `--producer`, checking a real Oxc binding and the
   packaged helper's matching fingerprint.
4. Build the producer-enabled CLI and matching sidecars explicitly. Run authored
   native dispatch/positive/negative compatibility controls with one implementation
   and pinned source/environment. Run bounded fresh corpus reads; historical
   success below is not a release gate. Do not remove unknowns to obtain green CI.
5. Build docs with `cargo doc --workspace --locked --no-deps`, install
   `docs/requirements.txt`, then `python scripts/build_docs.py --cli
   target/debug/entmoot --mcp target/debug/entmoot-mcp --out .tmp/staging` (add `.exe`
   on Windows). Existing Markdown owners, actual CLI help, actual MCP input schemas
   and Rust JSON Schema form versioned docs; executable examples run on clean roots.
6. Commit and push the validated candidate. Configure the `crates-io` environment
   and tag `v<workspace-version>` only when ready. `release.yml` gates publication
   on that tag's structural, native and docs jobs; inspect their actual results
   and the subsequent registry-install checks before declaring a release.

The [publishing procedure](../publishing.md) owns first-publish token setup,
subsequent OIDC configuration, immutable release tags and optional Pages hosting.
Docs called by a release inherit its push event; supplied inputs select release
output. Direct docs push/PR runs stage only. A successful artifact build alone
does not mean the site was deployed.

### Local qualification and copy handoff (2026-10-16)

This historical handoff report recorded the following results without a source
commit pin. It is not current qualification: the subsequent audit reproduced
storage-test failures and a broken native preflight (see PLANNING above).

The handoff reported 1,553 passing workspace tests/doctests, eight compiler-free
campaign controls, one revision-accounting control and 81 Python harness tests.
Provisioned native lanes reportedly passed 101 additional controls, including the
five-case Python compatibility test, historical Rust/TypeScript and native revision joins.
At that time, Clang VFS / javac binary-classpath controls were only qualified by
the Linux CI lane. Subsequent local provisioning validated both on Windows and
removed those platform ignores. Other ignored tests retain their explicit
provisioning requirements.

Formatting, strict structural and producer Clippy, generated contracts/inputs,
test ownership, Dylint, licenses, advisories, unused dependencies and workflow
syntax checks were reported passing locally. Extracted default and producer
packages passed their smoke checks with identical checkout/package fingerprints.
The report counted 17 owner pages, 83 actual CLI help sections and 22 MCP tool
schemas; fresh-workspace examples and strict Rust API documentation reportedly passed.

The clean source export contains tracked and non-ignored source files, including
dot directories such as `.github`, `.cargo` and `.agents`. It excludes `.git`,
scratch, stores, toolchains, build products and credentials. Its separate BLAKE3
manifest verifies every copied file and ZIP member. Copy the **contents** of that
export into the sibling repository, including dot directories; preserve that
repository's `.git`. Do not copy this working directory's caches wholesale.

The remaining external gates are the exact pushed commit's Linux/Windows checks,
all five registry package owners, protected release authentication and optional
Pages hosting. No local check claims those services have run or that a crate has
been published. Re-run qualification after any implementation change.

### GitHub Actions

Lane model: local pre-commit is fast and ownership-scoped; CI is authoritative;
tag CD creates the GitHub Release and publishes to crates.io.

- **Pre-commit** (`scripts/pre_commit_check.py`): developer accelerator—policy,
  fmt, ownership-scoped clippy/test. Not a substitute for CI; never teach
  `ci.yml` path-based correctness skips.
- `ci.yml`: `structural` (compiler-free tests, packaging, test-profile CLI/MCP artifacts),
  `policy` (fmt/Clippy/deny), `repo` (Python script/configuration/harness tests). Cargo/pip caches;
  ubuntu uploads `.crate` artifacts. No path-based correctness skips; no remote
  corpus clones. Linux jobs call `.github/actions/assemble-ci-env` before build/test.
- `native.yml` (`resolved` job): provisioned Windows/Linux adapters, pinned
  Pyright/TypeScript, Rust/Oxc producer, dispatch/compatibility; rust-cache;
  producer CLI artifact on success. Linux assembly uses the same composite action
  (tools + adapter tree); Windows keeps inline setup.
- Local Linux rehearsal: `python scripts/rehearse_ci_env.py` (Docker + act;
  `ghcr.io/catthehacker/ubuntu:full-latest`). Logs under `.tmp/ci-env-rehearsal/`.
- `demo.yml`: dispatch-only structural corpus (`evals/demo/`); reuses CI CLI
  artifact. Not a release gate.
- `docs.yml` (called by CI): reuses that run's qualified Linux test-profile CLI/MCP,
  builds rustdoc and the site into `.tmp/staging`, and uploads a staging artifact.
  After successful tag publish, `release.yml` deploys that artifact when
  `PAGES_ENABLED=true`. Enable Pages with GitHub Actions as its source.
  Versioned docs match the package build; a GitHub Wiki is not a second source
  of truth.
- `release.yml` (**sole CD**): on tag `v*`, runs ci/native/docs on that SHA, asserts
  tag↔version, creates the GitHub Release with `.crate` assets, Trusted
  Publishing to crates.io, then optional Pages deployment. Bounded timeouts, minimal permissions,
  verified immutable action pins. Trusted Publisher binding stays on this
  workflow filename + `crates-io`.
- `publish.yml`: manual dry-run / bootstrap gate only (`publish=false` by
  default). Tags go through `release.yml`; do not use `publish.yml` as the
  primary release path.

Cargo publication is immutable and packages must carry versioned dependencies;
see [Cargo publishing](https://doc.rust-lang.org/cargo/reference/publishing.html).
Pages deployment permissions and environments follow
[GitHub's custom workflow guidance](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).
The [Rust trusted-publishing action](https://github.com/rust-lang/crates-io-auth-action)
issues a temporary credential and revokes it after the job.

## Fresh alpha evidence (2026-10-16)

These four recorded publications used one implementation, identified below.
They do not qualify a later source revision or repair the current blockers: contract
`60ef27792c8a30dd3a36eeeaa00fdd81104f69a4cacc69085d03dfef37cfd662`, derivation
`f818d53bf77fc626081d19603b7254b229094086f8c98702b563bf05454da0dc` (BLAKE3).
Historical stores were preserved. The chi subject is now explicitly
`pkg:golang/github.com/go-chi/chi/v5`.

| Publication | Sites | Repository callable / symbol | External | Intrinsic | Unresolved / ambiguous | Unsupported |
|---|---:|---:|---:|---:|---:|---:|
| annotated-types 0.7.0 `0735cd3d` | 253 | 2 / 119 | 98 | 0 | 14 | 20 |
| qs 6.13.0 `5cf516c0` | 1,783 | 1 / 0 | 13 | 0 | 100 | 1,669 |
| getrandom 0.2.15 `cf65e83f` | 386 | 37 / 2 | 38 | 0 | 3 | 306 |
| chi `67be7d9c` | 2,985 | 199 / 13 | 319 | 172 | 23 | 2,259 |

These are native-provider outcome partitions, not independent correctness rates.
Go production explicitly excludes tests; unsupported includes inventoried calls
that the selected native scope did not assess. The per-site limitation explains
that distinction. Oxc supplies lexical evidence, not TypeScript compiler closure.
The qs packaging recipe lacks its npm packaging tools; getrandom's recipe requests
`--locked` although that revision has no committed lockfile. Their build steps
fail visibly while structural and partial native observations remain usable.
Neither row certifies a complete project build or dependency universe.

Ten stable-ID-selected sites per publication plus seven selected examples were
checked against source: **47/47 BLAKE3 file digests and callee byte spans match**.
This verifies attachment, not every native target or runtime behavior.

- qs `boundaries --callee "*.toString()" --detail export` finds two unsupported
  sites: `buf.readUInt8(i).toString` in `test/stringify.js:900` and `i.toString`
  in `lib/utils.js:11`. Their syntax does not establish a receiver type.
- chi `boundaries --path mux.go --outcome external_symbol --detail export`
  finds 31 sites. `handler.ServeHTTP` at line 327 retains the native target
  `(net/http.Handler).ServeHTTP`, standard-library PURL, declaration category,
  artifact pointer and build context. Its structured path now separates
  namespace `net/http`, type `Handler` and method `ServeHTTP`. Generic/unfamiliar
  Go spellings remain in the native field without invented ancestry.
- Seven independent native fixtures (Python, typed TypeScript, Go, C#, Java, C,
  Rust) retain an authored local leaf target and an explicit indirect/dynamic
  site. No fixture fails or silently falls back. Partial Java evidence remains
  partial. These controls qualify their selected constructs/configurations only.

Three repeated standalone CLI processes per operation, using the debug producer
build on this Windows host, produced these medians. OS caches were uncontrolled
and compilation was concurrent; these are observations, not an SLA or a speedup.

| Publication | Status | Inventory | Boundaries (10-site search page) | All workspace storage |
|---|---:|---:|---:|---:|
| annotated-types | 0.184 s | 0.179 s | 0.254 s | 17.43 MiB |
| qs | 0.731 s | 0.706 s | 1.048 s | 58.08 MiB |
| getrandom | 0.254 s | 0.247 s | 0.365 s | 20.45 MiB |
| chi | 1.722 s | 1.668 s | 2.987 s | 140.95 MiB |

Storage includes `.entmoot` databases, WAL/SHM, views, profiles and objects at
measurement time; it excludes checkout/build products. Reuse the existing MCP
service or request batch before adding caching or orchestration. Local evidence
is under `.tmp/local-alpha-impl/corpus-final/` and `fresh-final-evidence.json`;
it is optional scratch and is excluded from the source-copy payload.

## Historical audit evidence

The following sample predates these corrections and intentionally retains its
original qualification limits. It must not be read as a fresh alpha certification.

## Corpus sample and practical utility

Read `.tmp/corpus-tdd/run2/campaign.sqlite3` in SQLite read-only mode, all 36
receipt rows (22 repository cells, 12 checks, inputs and completion), all 20
remaining current pointers/summaries, and selected production records through
the fingerprint-checking `evals.harness.knowledge_sql` adapter. Hydrated sample
payloads were digest-checked. No existing corpus source or publication was changed.

The original campaign reports 20 complete inventories / 2 unavailable, and
9 successful native processes / 11 failed / 2 unattempted. Later retries are
separate JSONL files and current publications; they have not updated that campaign
snapshot. Its 12 checks report 11 completed processes and one unavailable YAML
overlay. Completion is not a semantic pass. Ten of twelve JSON files in the prior
manual-review directory are empty. Do not promote that report to a release gate.

Current pointers cover 20 cells; Guava's two pointers are absent. Eighteen
current summaries have native production stages, while both curl summaries are
structural-only. Across these pointers are four derivation fingerprints with the
same contract/layout fingerprints. The newest corpus CLI opens seven stores and
rejects thirteen; the existing debug CLI independently qualifies the two
getrandom stores. Rejection is honest compatibility fencing, not evidence of
corruption. The 20 main SQLite files occupy approximately 6.47 GiB, excluding
graph/object files, WAL and build products; do not call this total storage cost.

The following coverage partitions were hydrated with a matching reader. These
are provider-attributed observations, not independent compiler qualification.

| Publication | Call sites | Repository | External | Intrinsic | Unresolved/ambiguous | Unsupported | Meaning |
|---|---:|---:|---:|---:|---:|---:|---|
| chi `67be7d9c` | 2,985 | 956 | 1,268 | 335 | 50 | 376 | Useful repository navigation with substantial external/open boundaries |
| yaml 3.0.1 `f6f7691b` | 2,579 | 1,101 | 360 | 624 | 5 | 489 | Native run excludes tests; missing scope stays visible |
| Newtonsoft `0a2e291c` | 25,793 | 8,665 | 3,400 | 0 | 11,155 | 2,573 | Partial evidence; process success does not make the graph complete |
| getrandom 0.2.15 `cf65e83f` | 386 | 39 | 38 | 0 | 3 | 306 | Limited selected native environment; broad Rust qualification is unjustified |
| curl `b1ef0e1a` | 18,950 | — | — | — | — | — | Structural call sites only; no native coverage record |

Ten deterministically selected call sites from each row were checked against
the local source bytes: **50/50 file digests and 50/50 callee spans match**.
Selection was by record ID, not a random/statistical correctness sample. This
checks source attachment, not the correctness or closure of every native target.

Concrete operator insights:

- chi `mux.go::Mux.routeHTTP` is found at line 426. Its callee query reports
  eleven represented relationships, with a three-item page including
  `MethodNotAllowedHandler` and external `net/http` `ServeHTTP` endpoints.
  Verified source lines 449/459/463 show the corresponding handler calls.
  The caller query returns **unknown** with
  `call_graph_completeness_not_established`; it cannot justify deleting the method.
- chi's `mux.go` facet queries expose 167 unresolved relation attempts and
  34 external/dependency boundary records. These are review anchors, not counts
  of unique unresolved call sites or vulnerabilities.
- The stored Flask/Jinja compatibility check exits zero but returns
  **`incomplete_bind`**, no evaluated sites, and missing package API/import scope
  evidence. This is an actionable request for evidence, not a safe upgrade result.
- Historical downgrade overlays report annotated-types 0.7→0.6: 0 introduced,
  4 removed, 31 retained; getrandom 0.2.15→0.1.16: 26/39/29;
  Jinja 3.1.6→2.11.3: 151/232/1,147. These are the stored name/path presence
  results, subject to the scope-collision defect corrected above. qs reports 0/0/31, which says
  nothing about changed bodies or dependency compatibility.
- Current chi's subject omits `/v5`, while chi-5.0.0 includes it. That accidental
  retry difference must be repaired in fresh evidence, not hidden with an implicit
  alias. A release/commit label alone does not establish equivalent subjects.

Five status and five inventory commands succeeded against matching stores.
Single observations: status 0.33–7.05s, inventory 0.25–3.10s; Newtonsoft was the
slowest. A four-request chi batch took 0.40s and returned 7,228 UTF-8 bytes.
These are local observations, not cold/warm distributions or an SLA. Reuse the
existing batch/backend before adding caches or execution machinery.

Useful local evidence and reproductions are in `.tmp/local-v1-audit/`:
`sample.json`, `queries.json`, request/response JSON, `deps-repro.json`,
`overlay-repro/result.json`, package diagnostics and test logs. They are scratch,
not a mandatory raw-output archive or a second durable product schema.
