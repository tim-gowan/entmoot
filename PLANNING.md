# Remaining work

Delivered changes belong in [CHANGELOG](CHANGELOG.md). Current design belongs
in [ARCHITECTURE](docs/architecture/ARCHITECTURE.md); Rust and generated schemas
own contract fields. This plan supersedes earlier implementation sequences.

## Alpha handoff and beta qualification

The [local alpha release guide](docs/runbooks/local-alpha.md) owns operation,
validation and the sibling-repository handoff. The implementation now includes
automatic workspace storage, source/native boundary queries, scoped completeness,
confined materialization, occurrence-preserving overlay and self-contained package
fingerprints. Keep the README maturity table aligned with verified evidence.

Resolve the qualification blockers below before release. External steps remain:
copy/commit/push to `tim-gowan/entmoot`,
confirm all workspace crate owners, configure the protected release environment and
Pages, and require the exact tag's CI/native/docs gates. Do not claim publication
from local packaging. Beta requires operator feedback, representative native
project qualification and an exercised upgrade/history policy.

A remote location is a future transport boundary, not an implemented feature.
Do not add a scheduler, global package catalog or merge provenance to deliver
local automatic storage. First-use MCP inventory startup cost is measurable
follow-up if large-source operator feedback warrants asynchronous preparation.

## Architecture qualification follow-up

The canonical-file substrate cutover is in progress. File facts now precede
graph production, sealed units can survive an interrupted init, source-byte
admission is bounded, syntax graph fragments are no longer cached, and managed
publications no longer retain mandatory graph JSON. Immutable publication selects
one sealed view without per-fact owner/visibility histories. Opening a query,
verified source reads and bounded subject searches no longer hydrate a graph;
unscoped text search still scans node pages. Exact hash selectors use indexed
node columns, and comparison bases hydrate only when required. Inventory
membership uses integer revision keys. The
[10,020-node layout sample](evals/experiments/substrate-node-layout-2026-10-19.json)
uses 10.37 MB versus 13.32 MB for the prior membership/payload layout, including
the additional indexes and node commitments. This excludes unchanged shared
metadata and is not whole-store or query-latency qualification. Phase measurements separate
source observation, file-fact production, graph projection, structural resolution
and publication, with RSS endpoints explicitly distinguished from peaks.

Ordinary structural init now writes file projections directly to a sealed view;
only its callable lookup index spans files. Verified unchanged refreshes skip
projection/resolution/publication. Callers, callees and unresolved reads use
retained indexes. Node versions and syntax bodies share storage across revisions
and referring records; the earlier layout sample does not measure this newer
schema. Incompatible stores require an explicit rebuild and are preserved.

The [full streamed-publisher characterization](evals/experiments/performance-streamed-go-2026-10-19.json)
at `84bd4a7` completed all planned product commands on the same Go revision.
Fresh publication took 1,430.7s, with 0.50GB sampled peak RSS and 7.25GB retained.
That is slower than the intermediate checkpoint's 1,136s despite its much lower
RSS. No-change refresh took 27.8s; comment/body/add/rename took 750–828s and each
new view added about 3.25GB. Recovery took 1,781.8s; host build/validation jobs
overlapped this later trial. The harness then failed a wrong counter assertion
(11,832 reused versus 0 newly parsed). Independent post-hoc checks verified all
source bytes, unchanged binaries, historical reads, rollback and reused parser
units; the failed runner and missing completion marker remain preserved.

Exact path/ID find and source reads took 3–5ms inside the engine. Caller/callee
queries took about 0.49s; their six-repetition processes took about 3s. Cold
subject context took 153.7s and roughly 9.65GB process peak RSS; the body comparison
took 247.9s and 11.79GB. Those full-graph consumers remain scale blockers.
This single-trial characterization does not qualify later revisions or native
semantics. The incremental publisher on the same corpus is recorded below.

Native/canon production still uses graph views; native output is one
complete family artifact, although every family now consumes compact pinned
structural input. Subject-context, traversal, impact and advanced reads still
hydrate a deferred graph. Replace those only with scoped reads that preserve
full counts, provenance and explicit unknowns; do not claim a total RSS ceiling
or sub-second latency from worker admission limits or successful fixture tests.

The [intermediate checkpoint](evals/experiments/performance-substrate-checkpoint-2026-10-19.json)
completed the same full Go workflow: fresh publication took 1,136s, peaked at
6.03GB process RSS and retained 8.16GB; no-change refresh took 102s. Caller/callee
processes still took 124/142s and about 9.65GB RSS. That frozen build predates the
streamed publisher, shared node versions and indexed calls. It motivates those
changes but does not qualify them. Comment/body/add/rename/delete trials completed;
current/base comparisons and interrupted recovery were not measured there.

The same full Go run at `84bd4a7` attributes the one-file body refresh to publication:
749.7s total, including 23.5s parser-fact reuse and a 723.5s publication interval.
Within that interval, syntax projection took 345.0s and structural resolution
114.4s. Comment refresh took 828.0s. These are observed single trials, not a
latency distribution. Parser reuse alone still rebuilt every file projection,
rebound snapshot-qualified records and serialized nodes twice for commitments.
The incremental publisher now reuses verified file-local node/edge references,
rebinds current-snapshot claims, copies unchanged memberships in SQL and shares
one node serialization across both commitments. Separate finalization timings
attribute ordering, source membership, commitments, file selection and commit.
The [512-file development comparison](evals/experiments/performance-incremental-512-2026-10-19.json)
reused 509 of 510 parsed file projections and reduced a comment-edit refresh from
23.6s to 17.5s (26% in this single fixed-order trial). All 68,835 node versions
and 69,925 structural edges matched the baseline exactly. Initial retention grew
from 251.9MB to 259.7MB for projection references. The edit still spent 6.3s in
syntax projection/claim rebinding, 3.5s in structural resolution, 3.0s verifying
publication commitments and 1.6s selecting file facts/call summaries. Parent and
child phases overlap.

The [full incremental-publisher characterization](evals/experiments/performance-incremental-go-2026-10-19.json)
at `352cf60` completed the same Go revision and sequence with pinned release
binaries. Fresh publication took 1,297.6s (about 9% faster than the streamed
baseline in this single trial) with 0.49GB sampled peak RSS. Comment/body/add/rename
took 576–590s (about 23–31% faster than 750–828s streamed) while reusing 11,831
of 11,832 sealed syntax projections on one-file edits. Recovery after an injected
transactional abort took 567.9s versus 1,781.8s streamed; the publication pointer
and prior views stayed put, completed parser units survived, and the retry reused
all 11,832 sealed syntax files. Source bytes, binary hashes and historical reads
were verified; an earlier attempt that failed comment with a store path I/O error
is retained beside the completed run and is not this completion. Delete and
no-change stayed near 28–29s. Cold subject context still took 150.1s and about
9.66GB RSS; body comparison took 259.0s and 11.79GB. Exact find/source warm
medians stayed 3–4ms; caller/callee warm medians about 0.51s.

These are measured single-trial improvements on edit refresh and recovery, not
repository-wide readiness. Complete callable index construction, claim rebinding
and canonical verification still scale with the selected repository; this is not
an O(changed-files) refresh. A further cut must share revision-independent records
through file membership and preserve deletion/corruption detection when projecting
snapshot-qualified records or deferring full export commitments. Import statements
alone cannot close native dependency or dispatch frontiers. A reference is not
necessarily a call, and provider relationships do not prove a closed dispatch set.
Retain unmatched sites as explicit unknowns; do not invent type, dispatch or
completeness facts a provider did not report. Claims that a call, dependency or
dispatch frontier is open or closed must stay explicit and evidence-scoped.
Qualify provider-specific binding, dispatch, dependency coverage and configuration
separately from the compiler-free structural lane.

The split preserves a conservative v1 whole-engine derivation fence. Qualify
component-specific fingerprints before narrowing its invalidation scope. Measure
the standalone query benchmark after resolving the initial-publication scale
blocker. The [2026-10-18 pinned Go attempt](evals/experiments/performance-go-2026-10-18.json)
at `8d3b225` timed out after 1,202s on 16,087 tracked files (11,836 Go files),
with 23.1GB sampled peak RSS and 8.02GB of incomplete artifacts. No current
publication was exposed. Warm reads and edit→refresh distributions remain
unavailable; two remaining fresh trials were unrun. Profile the initial
publication owner before attributing an inner bottleneck or narrowing the
workload. The crate split alone establishes no latency improvement or monorepo
readiness; the Windows host was under memory pressure.

The [full-repository rerun](evals/experiments/performance-go-current-2026-10-18.json)
with the optimized `57c2bf3` CLI verified the same source bytes and used the same
fresh-store command and 1,200s timeout. It timed out again at 1,200.6s without a
current publication. Peak RSS fell to 6.88GB (about 70% lower), while incomplete
artifacts reached 12.78GB, including a 6.83GB WAL and 2.97GB disposable staging
database. Memory reduction did not resolve publication latency. CPU and WAL
growth continued; phase timings and profiling are still needed to attribute the
remaining cost. Warm reads, refresh and interrupted-run recovery remain unqualified.

The [unbounded full-repository rerun](evals/experiments/performance-go-unbounded-2026-10-18.json)
with the same `57c2bf3` release binary completed successfully in 2,720.1s with
7.17GB peak RSS and 11,832 indexed files. Its fresh structural publication returned
`replaced`; the earlier timeouts do not establish a deadlock. Initial publication
latency remains a blocker. This single run does not qualify warm reads, refresh,
recovery or native semantics. Prior failed-run stores were removed after retaining
their measurement evidence; the successful publication remains available.

The [memory attribution follow-up](evals/experiments/memory-go-2026-10-18.json)
prioritizes representation and ownership before worker orchestration. Reloading
the retained export used 6.13GB of live Rust allocations: 2.43GB for the node
vector (1,160 bytes per node slot) and 0.90GB for runtime indexes. A 512-file
subset reached 839MB during publisher preparation; cached fragments and retained
typed/serialized copies are measured contributors. Optional node payloads are now
boxed; initialization stages per-file IR and syntax fragments on disk before
downstream phases, and publisher preparation retains payload commitments rather
than a second serialized row set. The
[IR-first comparison](evals/experiments/memory-ir-first-2026-10-18.json) on the
same frozen 512-file frame reduced graph/cache live Rust allocations from 484MB
to 195MB and publisher-preparation peak from 841MB to 411MB. Node slots are
440 bytes. Current ordinary/staged graph digests agree at all four frame sizes;
inventory and extracted record counts also match baseline. This is an allocation
diagnostic with temporary-file I/O, not latency qualification or a strict RSS cap.
Publication now stages bounded serialized batches and validation context on disk;
graph-backed emission avoids a second complete knowledge projection. Smaller repeated
graph-build residual growth remains unattributed; isolated source-observation and
scan controls did not reproduce sustained growth. The current 20-cycle 128-file
repeat still grows by 0.33MB between its first and twentieth released cycles after
a 200ms grace period, usually in 17,504-byte steps; this smaller issue is not fixed
by the representation changes. These allocation probes do not
qualify latency or account for all native allocations in the failed initial run.

### Remaining operation memory qualification

The [2026-10-18 schema/consumer audit](evals/experiments/operation-schema-memory-2026-10-18.json)
at `6ebaf50` records 13 opportunities and 33 compiled inline type sizes. The
operation fixes now consume loaded knowledge, stage publication context, share
unchanged syntax during rollback, release native payloads within workers, stage
Git blobs, narrow impact consumers and stream canon assembly, hashes and output.
Revalidation counts selected records before cloning policy evidence. These changes
preserve source fences, complete validation, atomicity and unknown outcomes.
The [operation comparison](evals/experiments/memory-operations-2026-10-18.json)
reduces the frozen 512-file publisher-preparation peak from 411MB to 238MB;
full validation/publication reaches 239MB. Relation claims now occupy 648 rather
than 960 inline bytes. The structural rollback-sharing control adds 38.5MB of
live allocations while sharing syntax buffers/indexes; artifact-heavy attachment
and syntax mutation costs remain unmeasured. These are Rust allocation diagnostics,
not release latency or process RSS bounds.

Remaining costs require representative operation measurements before further
representation or concurrency changes:

- Artifact rollback still clones evidence collections; resource edits detach the
  shared syntax buffers. Measure this before replacing the validated candidate
  with an artifact delta. Normalization still hydrates one complete artifact.
- Revalidation and comparisons still hydrate complete graph views. Qualify pinned
  artifact/support read projections without losing independent/derived support,
  applicability, exact membership or uncertainty from unselected evidence.
- Canon bounds encoded file shards after term construction; clone reads hydrate
  a complete bounded shard. Measure term peaks and a verified fingerprint-only
  read projection while preserving candidate semantics and cumulative limits.
- Git integration packs remain materialized. Qualify file-backed pack transport
  separately from the now staged capture blobs, including restart and final CAS.
- Measure native target occupancy and query index ownership before boxing more
  payloads or adopting generation-local interning/dense postings. Allocating a
  box for a mostly populated field can increase total memory. Keep occurrence,
  evidence and scope distinctions and checked cardinality conversions.

Measure preparation, validation, publication, reload and recovery separately.
The complete graph and compact publication identity/dependency maps still grow
with repository size; record limits and worker counts do not bound process RSS.
Operation-control JSON rewrites remain lower priority until profiles show them
dominating. Monorepo and warm-refresh latency remain unqualified; no new scheduler
or durable storage protocol is proposed.

## Current qualification blockers

The storage, configuration/admission, obsolete fixture paths, process isolation,
native-feature selection and permissive assertion defects from the 2026-10-16
audit are repaired with regression controls. Adapter default precedence now
matches checkout → PATH → home; process overrides remain exclusive, and readiness
uses workspace config selections consistently with runtime resolution.

Provision pinned sidecar/Program adapters explicitly for native qualification.
Roslyn MSBuild project loading needs an approved host run when the sandbox denies
its child named-pipe connection; ad-hoc C# success does not qualify that mode.
Release still requires the exact candidate's structural/native CI and extracted-
package gates on both supported operating systems. Local installation and
historical passing receipts do not qualify a later revision.

## Active scope

All active goals serve these three priorities:

1. Implementation quality: architecture, performance, maintainability and precise
   naming and semantics in the existing shared components.
2. CLI/MCP usability for the existing local workflows, with consistent behavior,
   useful explanations and actionable failures.
3. Test-driven expansion and hardening of native resolver evidence and logic-hash
   input coverage, increasing what can be claimed from demonstrated behavior.

Existing workflow objectives are inventory → optional explicit binding/canon →
inspect/query → edit/refresh → impact/test selection or local pre-commit conflict
review, including existing pause/resume/cancel and retained-evidence operations.
Multiple agents may use captured branch/worktree states in one repository.
These are the workflow basis for quality, UX and coverage work, not a mandate to
add execution capabilities. Preserve publication, recovery and producer isolation.

Implemented structural families are Rust, Python, JS/TS/TSX, Go, Java, C# and C;
C++ remains experimental. Native qualification is per provider, construct, project
and configuration. Parsing is not compiler qualification. Assume a host-supplied,
pinned and isolated environment such as [Hermit](https://github.com/cashapp/hermit);
setup remains host-owned and init/bind never install tools or run scanners.

Everything outside the three active priorities is deferred below. Deferred work
requires explicit reprioritization; a promising experiment or available library
does not automatically reactivate it. Further execution machinery must earn its
place through measured need. Do not add scheduler capabilities or checkpoint
granularity during this pass.

## Operator showcases: scratch qualification before promotion

Use the locally built, matched CLI/producer against a realistic support portal
under `.tmp/operator-showcase/`. Promote reusable fixtures into the existing
crate `tests/fixtures` owners only after their independent controls work. Keep
downloads, dependency trees, publications and measurements in scratch. This work
does not qualify the pending release candidate or expand Entmoot into a scanner.

The first scenario pins Express 4.21.2 and 5.1.0, with document downloads,
ticket/invoice/private-note handlers, audit events and a dynamic export formatter.
The old optional-extension route fails during Express 5 registration in the
router/path-to-regexp dependency; the corrected route and unrelated routes are
positive controls. This is an internal routing change, not a demonstrated public
signature break. Express 5 moved its router into a separate package; an inventory
removal from Express alone must not imply that the functionality disappeared.
See the upstream [migration guide](https://expressjs.com/en/guide/migrating-5/#path-route-matching-syntax).

Implement and qualify in this order:

1. **Fixture and expectations.** Keep locked npm installations with lifecycle
   scripts disabled and real loopback HTTP controls. Author expected route,
   constructor, formatter and audit relationships independently of Entmoot's
   outputs. Record original, upgraded and repaired states. Verify the frontend
   URI is an HTTP endpoint association, not a JavaScript call edge to the server.
2. **Navigation and dependency evidence.** Start with URI text/location discovery,
   exact subjects, inspected calls and open boundaries. Qualify Oxc and explicit
   TypeScript Program separately. Join materialized dependency publications only
   through supported source/package/build-context evidence; retain ambiguity and
   external targets. A successful endpoint path must reach the internal changed
   occurrence, not stop at the Express import. Constructor and dynamic targets
   need native controls; an unresolved call cannot become a plausible invented edge.
3. **Shared traversal pruning.** Extend the existing query contract and adjacency
   owner, then expose a thin `path` CLI command and the same MCP request. Prefer
   explicit edge/callsite IDs, qualified names and bounded path/callee patterns
   over guessing the meaning of an untyped string. Apply exclusions during both
   search frontiers and seed discovery, including contracted chain interiors,
   neighborhood walks and retained continuations. Include selectors in cache and
   continuation identity. Receipts must report applied exclusions and counts;
   a pruned negative answer is scoped to the selected graph. Reject malformed
   selectors. Test alternate routes, parallel occurrences, cycles, excluded
   endpoints, zero matches, changed filters on resume and immutable graph reuse.
   Test-impact ownership floors must never be weakened by navigation pruning.
4. **Claims and clone review.** Reuse generic SARIF attachment, assertions and role
   queries. Validate mock SARIF against the official 2.1.0 schema, label the mock
   producer, preserve exact locations/provenance and test stale/ambiguous inputs,
   idempotence and absent roles. Standard `threadFlowLocation.kinds` already
   supplies provider-stated roles; ordered trace steps must not invent call edges.
   Extend the common adapter only for a demonstrated missing claim representation.
   Parameterized hashes should group the two logged handlers while distinguishing
   the unlogged handler; the independently specified route family and runtime
   audit-event control establish the omission, not hash inequality alone.
5. **Complete operation workflows.** Enumerate public leaves from live Clap help
   (initial inventory: 76 leaves / 83 help sections). Cover navigation; publication
   and attachment; impact/overlay/compatibility/test selection; producer handoff;
   collaboration/recovery; retention; and identity/evidence inspection. Give every
   leaf an operator question, executable example, independent expected observation
   and failure/control case. Exercise mutations only in fixture-owned stores and
   disposable Git repositories. Keep expert lifecycle procedures in their existing
   runbooks rather than putting them on the ordinary navigation path.
6. **Promotion and documentation.** Promote small offline semantic controls to
   existing test owners, with optional pinned native/runtime controls separate.
   Update ownership, required features, CLI/MCP/schema examples and the owning
   runbooks together. Write regressions before behavioral fixes. Require focused
   tests, workspace/fmt/strict Clippy/doctests/schema checks and executable docs;
   unavailable tools, ignored tests and empty result sets are not qualification.

Measure each operation's wall time, CPU, peak main-process working set, sampled
process-tree RSS, output bytes/characters/tokens and storage delta. Separate fresh
stores, warm queries, indexing/binding costs, dependency installation and cleanup.
Record binary fingerprints, source/configuration digests, tools, repetitions and
sampling limits. Compare complete operator tasks against a bounded `rg` plus
source-reading baseline using identical sources and independently scored answers.
Count all request/output pages, failed attempts and setup amortization. Named
tokenizer counts are output-volume measurements, not billed model usage or hidden
reasoning tokens. Measure actual agent input/output/reasoning usage only from a
controlled model run that exposes it; otherwise report unavailable, not estimates
of savings. A small fixture may favor grep, and that result must remain visible.

Initial scratch observations (2026-10-17 UTC; not release qualification): runtime
controls pass for both pinned Express versions; parameterized clone candidates
separate the logged pair from the missing-log handler; generic SARIF attaches and
its roles query a supported Program-bound `exportTicket → createFormatter` edge.
URI text search, full frontend-to-dependency traversal, constructor/dynamic paths,
pruning and coverage of every operation remain unqualified. The full Express
impact response is 806,123 bytes / 289,063 `o200k_base` tokens in this run: reduce
delivery cost through existing output/page owners before advertising agent savings.

### Response cost and agent navigation

The 2026-10-17 scratch response audit is under
`.tmp/operator-showcase/token-review/`: `REVIEW.txt` owns the detailed field
disposition evidence; `execution-ledger.json`, `attribute-costs.csv` and
`results.json` retain commands, measurements and projection comparisons. These
scratch outputs are not shipped interfaces or qualification of unprobed commands.
Thirty response samples cover 2,581 normalized attribute paths. Current controls
already reduce the 21-site boundary response from 34,502 export tokens to 2,079
compact tokens with all site IDs, and the same one-edge supported path from 3,252
explain tokens to 1,188 compact tokens. Detail differs; first-page reductions must
not be advertised as complete-task savings.

The largest opportunity is impact delivery: declaration bodies occupy 53.3% of
the measured response, contracts 18.5% and base evidence records 12.1%. A scratch
projection retains all 38 review signals, exact occurrence references, partition
counts, snapshots, correspondence and limitations in 5,881 tokens, deferring full
delta bodies. A clone projection retains both groups/all members, adds names and
lines and defers redundant member hashes: 1,394 → 659 tokens. These projections
are proposals, not production implementations or measured reasoning savings.

Implement in the existing contracts/query-service/projection owners, in this order:

1. **Consistent bounded delivery.** Extend existing `OutputOptions` and
   compact/explain/export semantics to impact, status, inventory, assessments and
   accounting. Add explicit text/JSON rendering independent of TTY detection;
   keep current machine exports compatible while introducing opt-in projections.
   Lead with outcome, named locations, reason, counts, uncertainty and an exact
   expandable reference. Default impact to a bounded review view when selected,
   with counts for every partition and full deltas available explicitly. Preserve
   the full executable test selection and `full_suite_required` even when its
   explanation is paged. Move successful infrastructure path diagnostics behind
   verbosity; keep failures and actionable warnings visible.
2. **Retain signal while removing repetition.** Preserve status, effective scope,
   provider/epistemic state, freshness, ambiguity, open boundaries, completeness
   and limits. Separate search exhaustion, delivery pagination and deferred
   detail. Retain exact snapshot/configuration/generation/visibility fences and
   IDs; intern repeated context once. Defer request echoes, healthy unrelated
   adapter paths, repeated identity/implementation hashes, raw syntax trees and
   per-record evidence ledgers to inspect/export. Keep identity-focused commands
   exact. Unknown/unexamined values must not become zero. Use typed optional detail
   groups, not arbitrary field selection that can erase mandatory limitations.
3. **Improve compact items.** Boundary compact rows currently lose line, owner,
   native outcome/reason and the engine's existing callee/outcome counts through
   generic facet projection. Add typed boundary items and expose those aggregates.
   Finding/role rows need rule, named location/target, provider, attachment grain
   and epistemic status. Clone/component/unused samples need readable subjects
   and locations. Deduplicate shared receipt/context dictionaries across seeded
   routes and impact caller pages; retain independent scope and search outcomes.
4. **Reduce request construction and round trips.** Expose existing clone path,
   statement, threshold and page controls, and status suggestion/sample controls,
   as ordinary flags. Add subject-kind/exact-match filtering to find, bounded
   literal source discovery with location joins, and inspect/read-source by exact
   subject or callsite with selected source/call/claim details. Literal and endpoint
   associations remain distinct from call edges. Reuse existing boundary callee,
   path, outcome, reason and build-context selectors; add owner/site and counts
   views. Add impact change/section/path selectors and explicit base-side callers
   for removed occurrences. Do not overload impact's existing correspondence
   `--subject` with a different symbol-filter meaning. Extend existing
   `SuggestedSelector` for bounded executable follow-ups, not another planner.
5. **Graph questions and filter placement.** Deliver the shared pruning/path work
   above through existing adjacency/search owners. Default connectivity questions
   to a bounded representative witness with scope/evidence classification; keep
   explicit bounded candidate requests. Reuse the scoped weighted bidirectional
   search; it does not enumerate every possible path. Include prunes in both
   frontiers, seed discovery, cache identity and continuation, and retain applied
   exclusions/counts. For the
   logging showcase, compare scoped observed outgoing-call sets within the
   independently specified handler family; normalized hashes supply peer groups,
   not a missing-behavior verdict. Push filters and counts before hydration; use
   deterministic ordering and disclose selection rules. Preserve test ownership
   floors. Measure before adding new indexes or graph algorithms. Current impact
   relay still computes a full manifest before paging, so a smaller response alone
   does not establish lower computation or memory cost.
6. **MCP schema and transport.** The built server advertises 22 tools whose compact
   serialized definitions total 32,043 tokens; actual client injection/caching is
   unmeasured. Offer an explicit small navigation profile while retaining a full
   tool profile. Reduce repeated cursor/schema description overhead and avoid
   eagerly advertising the generic query union plus every alias without need.
   Generate bounded typed output schemas; the current tool definitions have none.
   Preserve canonical cursor checks if its representation changes. The pinned
   rmcp helper repeats JSON in text and `structuredContent`; this follows the
   [MCP compatibility recommendation](https://modelcontextprotocol.io/specification/2025-11-25/server/tools#structured-content).
   Shrink the common payload first. Qualify actual clients before any opt-in
   alternative presentation; wire duplication is not proof of double token billing.

Before fixes, add regressions for same-name sites at different locations, ambiguous
subjects, multiple configurations, external/open dispatch, stale/conflicting/mock
claims, removed callables, source mismatch and missing test ownership. Assert that
compact/explain/export preserve applicable classification, count and limitation
semantics; every returned reference must expand exactly. Reject stale or altered
query/page cursors. Test aggregation denominators, parallel occurrences and
unknown values independently. Generate schemas from Rust, retain owned v1 series
with correct fingerprints, update CLI/MCP examples and run the shared checks.

Validate initial delivery targets on the fixture: five ordinary navigation items
within 1,000 tokens, one supported witness within 2,000, and a five-item impact
overview within 2,000, including mandatory caveats. These are named-tokenizer
benchmark targets, not model-specific engine budgets. Use existing byte ceilings;
required context that cannot fit must produce a useful budget error or explicit
continuation, never an overconfident answer. Compare complete operator tasks with
equal independently scored answers against bounded grep/source reads, including
schemas, requests, stderr, pages, retries and drill-downs. Track fresh/warm runtime,
CPU, memory and storage separately. Reasoning/billing savings remain unavailable
until a controlled agent run exposes actual usage; do not estimate them from bytes.

## General workflow evaluation

Generalize the scratch demonstrations into operator questions with pinned inputs,
independent oracles and explicit stopping rules. The
[workflow evaluation protocol](evals/PROTOCOL.md)
owns case qualification, real upstream examples, sampling, measurement and grading.
The bounded Express release profile now independently grades runtime scenarios,
provider attachment and exact-reference retrieval, retaining historical production
views and measured command outputs. Local history and paired agent runners now
provide frozen selection, independent file-byte/syntactic-call oracles, bounded
tools, provider usage and regrading without model calls. The default calibration
is 12 attempts of at most two minutes; implementation is not evidence of benefit.

The [2026-10-17 history pilot](docs/runbooks/revision-evaluation.md#history-inventory-evidence-2026-10-17)
completed 72 selected comparisons and 139 retained revision views, with independent
source-byte and persisted-membership checks. The six-task paired calibration is
prepared but unrun; its results and any agent-benefit claims remain outstanding.

The 2026-10-17 investigation is under `.tmp/workflow-evaluation/`: the candidate
catalog records upstream anchors and unresolved qualification; the read-only
legacy inventory records the earlier survey's actual schema/counts. Reuse its
revision/path leads, not its semantic success flags or historical fingerprints.
The large binding-row count is not a large independent sample. Repository and
revision commands now share process-tree sampling, output measurements and
attempt-preserving logs. The bounded release study remains one correlated family.

Remaining qualification, beyond the local calibration:

1. **Qualify semantic cases and truth.** Reproduce the curated mechanism and
   configuration candidates in scratch. The local history profile targets 80
   episodes across eight JS/TS, Python, Rust and Go repositories, preserving
   underfilled strata. Its source-byte and lexical-call oracles do not qualify
   semantic repairs. Separate related fixes from held-out cases. Java/C#/C need
   their own native qualification; C++ remains experimental. Failed CI and merge
   topology locate candidates, not causes or expected answers.
2. **Extend observation only where measured.** Shared process measurement now
   covers child timeout, failed attempts, unavailable counters, byte limits and
   exact decoded token inputs. Quantify observer/coordinator overhead and add
   OS accounting only where sampled counters leave a material research gap.
3. **Extend qualified workflows.** Keep selection/materialization, independent
   oracles, product operations, measurement and grading in their existing owners.
   The provider boundary isolates invocation, decoding and usage; Codex CLI is
   the only live adapter. Qualify another adapter before claiming provider parity.
   Parameterize selectors, dependency/configuration scope and budgets; keep
   expected answers out of workflow code. Extend Rust-owned portable campaign
   contracts only when replay needs new wire data. Cover operation families with
   representative tasks and focused lifecycle controls.
4. **Qualify independent grading.** Reproduce buggy/fixed and unaffected controls;
   assert source-backed occurrences, edges, claims and test obligations separately.
   Keep authored SARIF, real provider output and runtime controls distinct. Test
   wrong scope, ambiguous joins, partial pagination, missing tools, empty
   denominators, stale inputs and retry identity. Preserve unknowns and compare
   only the declared reference universe. Existing graph self-consistency checks
   remain useful but cannot grade graph completeness.
5. **Evaluate transfer after calibration.** Report the small local run before
   proposing a wider agent budget. Use paired tasks and identical fallback tools
   and limits, keeping actual usage separate from text volume. Include failed
   attempts, repeated context and setup cost. Add held-out semantic tasks and
   explicit compact-output/skill ablations. Repeated edges, revisions or model
   attempts do not create independent repository samples.
6. **Repair and promote.** Preserve failures, add independent regressions, fix
   common owners and rerun affected cases with new implementation identities.
   Keep before/after evidence and record oracle corrections. Move small controls
   into owned tests only after scratch qualification; keep full upstream cases
   opt-in. Update existing docs/tool descriptions/skills with demonstrated
   decision procedures and honest limits. Select a wider survey from measured
   correctness, answer coverage, agent benefit and reproduction cost together.

Acceptance requires independently graded answers and reproducible cost accounting,
not a target token reduction alone. Publish denominators, unavailable measurements,
sampling limits and per-repository results. The current plan does not authorize
inferring reasoning savings from output size or treating release/deployment as
qualified by a corpus run.

The release cohort measured raw impact exports near 4 million tokens each, while
the existing bounded relay supplies a much smaller initial navigation page. Keep
full manifests outside agent context; evaluate selector/filter improvements in
the shared relay owner before adding another summarizer. Initial-page size is not
whole-task savings. Exact resource peaks, coordinator overhead and per-view
allocated storage still need separate instrumentation if required by the wider survey.

## Implementation quality

- Audit shared normalization, traversal, initialization, binding, edge stitching,
  canon, persistence and processing owners. Reproduce defects with independent
  expectations, then fix the common owner and its consumers. Prefer consolidating
  or shortening existing logic over adding parallel abstractions.
- Preserve the crate dependency DAG, compiler-free structural builds, typed
  library errors and shared CLI/MCP engine operations. KnowledgeRepository owns
  durable mutation; topology and indexes remain disposable views. Preserve atomic
  publication, historical retrieval, idempotence, source fences and passive reads.
- Audit naming end to end through Rust contracts, adapter boundaries, joins,
  storage, hashes, receipts, generated schema and CLI/MCP projections. Distinguish
  syntax, binding, dispatch, completeness, environment failure, exhausted budgets,
  applicability and checked evidence. Converge aliases at one boundary, retain
  attributed provider detail and accept unknown extensible detail honestly.
- Profile and reduce measured costs in existing owners: source observation,
  cache/retained-view hydration, complete index assembly, validation, publication,
  queries and result serialization. Measure storage retention and row churn as
  well as latency; preserve dependency and corruption checks when optimizing.
- Keep documentation concise and owned, derive wire shape from Rust, retain v1
  series with compatibility fingerprints, and remove obsolete duplication through
  ordinary changes. Do not add retention ledgers or blanket archive requirements.

The next performance investigation is the remaining warm-refresh cost in cache
loading, complete index assembly, source reobservation and publication. The latest
three alternating baseline/current development-CLI probe pairs used four Git worktrees
and 512 Python/TypeScript files. Median edit → refresh → Git capture → two peer
comparisons was 18.18s, or 20.19s with one process exit/resume (+11.1%). Initial
publication/capture took about 43s separately; main SQLite files were about 184MB.
Refresh (7.57s), conflict execution (6.41s) and Git capture (4.07s) dominate.
Reports and saved prefixes matched, and source refs/indexes were unchanged.
These fixtures exclude native runs and do not qualify build overhead, peak memory,
release performance or larger repositories. Reproduction and raw evidence are in
`.tmp/refresh-profile/`; earlier costs and recovery exceptions remain in CHANGELOG.
Use representative current-workflow scale probes to extend those measurements;
do not launch a universal performance campaign or infer an inner bottleneck from
whole-phase timings.

## CLI/MCP workflow usability

- Exercise the complete existing workflows through both frontends, from a fresh
  workspace and from retained state. Users and agents should be able to discover
  available operations, supply correct inputs and understand the next valid step
  without learning private storage or execution details.
- Keep CLI and MCP results, defaults, limits, filtering, pagination, continuation
  and status meanings consistent through shared engine operations. Test CLI exit
  behavior and MCP structured errors alongside successful JSON/schema responses.
- Make evidence provenance, source/configuration scope, freshness, coverage gaps
  and unknown/partial outcomes clear in concise default output and deeper explain
  views. Distinguish textual conflicts, structural overlap, potential interaction
  and compatibility not established. Frozen observations do not track later edits.
- Make unavailable/stale adapters, incompatible stores, stale workspace generations,
  source drift, budget exhaustion and cancelled/paused jobs actionable. Point to
  existing setup/recovery commands or runbooks without implicit installation,
  hidden producer dispatch or unrequested mutation.
- Harden existing controls under concurrent use and restart: bounded results,
  responsive status/pause/cancel/resume, preserved saved work and predictable
  refusal/retry behavior. Fix defects in the existing implementation; new queue,
  scheduling and recovery granularity remain deferred.
- Keep test impact and unused/clone results faithful to their evidence. Ownership
  is the minimum test selection; static paths do not establish assertion coverage,
  missing references do not prove dead code, and hash matches are policy-bounded
  candidates rather than whole-program equivalence verdicts.

## Expand claimable coverage through TDD

For each increment:

1. State the exact claim, supported construct/provider/configuration and the cases
   that must remain unknown. Put expectations in owned source/revision fixtures
   and tests beside the relevant implementation, not a new parallel coverage ledger.
2. Write a regression or new-behavior test before changing implementation and
   confirm it fails for the intended reason. Pair positive cases with near misses,
   unsupported cases and counterexamples; do not derive all expected results from
   the same algorithm being tested. Add bounded property, metamorphic, differential
   and fuzz checks where they expose shared normalization, traversal or identity
   gaps; reduce failures to deterministic regression seeds.
3. Fix or extend the shared owner, then follow the fact through attachment,
   persistence/reload, revalidation, identity and both frontends as applicable.
4. Use actual native controls to qualify native semantics; authored provider data
   qualifies normalization/attachment only. Missing tools remain unavailable and
   live qualification stays opt-in. Preserve useful partial facts and provenance.
5. Run focused checks first, then required workspace, formatting, strict Clippy,
   doctest and schema checks. Maintain test ownership and report which scenarios
   passed, remained unsupported or were not run. Broaden the claim only as far as
   the evidence supports, and retain regression seeds for discovered failures.

Priority coverage frontiers within the existing families and workflows:

- Native resolution: declaration/site attachment, direct and indirect calls,
  overloads, receivers, generics, inherited/virtual dispatch, aliases, macros or
  generated sources, external package boundaries and build/configuration changes.
  Expand supported cases incrementally with independently authored native oracles;
  never infer dispatch closure from successful definition lookup alone.
- Logic-hash inputs: exact body versus occurrence/boundary identity, normalization
  policy, free/captured names, shadowing and initializer scope, closures/patterns,
  name-observing constructs, literal partitions, ordering and encoding. Exercise
  required equality and required distinction across edits, moves and configurations.
  For existing typed canon, qualify native types, width/signedness/overflow,
  assumptions and partial outcomes before admitting another construct. Structural,
  alpha, parameterized, resolved and canon hashes retain distinct meanings.
- Dependency and lifecycle correctness: negative/ambiguous lookups, target
  appearance/deletion, missing or changed inputs, source and environment drift,
  independent versus conjunctive evidence support, producer retraction, corruption,
  budget boundaries and concurrent/restarted publication. Unchanged structural
  input alone cannot justify reusing context-dependent evidence.
- Per-call-site contract facts: test what declared signatures and native evidence
  actually establish about arguments, receivers, types, targets and dispatch.
  Preserve any qualified mapping supplied by a provider with its limitations.
  Two calls to the same callee may have different obligations. A resolved edge,
  aggregate caller-to-callee relation or stored derivation must not imply checked
  preconditions, postconditions, effects or inter-procedural proof composition.
  Unsupported mapping or contract applicability remains explicit; proof checking
  and automatic contract propagation are deferred.

Success means a larger, accurately bounded set of supported claims, backed by
production fixtures and the appropriate native evidence, with consistent frontend
explanations. Test counts or a successful parser alone do not establish that result.

## Prior-art evaluation (local)

Local Trail of Bits checkouts live under `G:\GitHub\trailofbits\` (sibling of
this repository): competing tooling `trailmark`, `multiplier`, `mrva`,
`codeql-queries`; solver-adapter candidates `pasta`, `blight`, `vast`, `macroni`;
plus `fickling`. Study in place; do not copy GPL code or vendor AST types into
contracts. These are evaluation tasks, not adoption projects until measured need.

- Evaluate **PASTA + blight** as a Clang/libclang A/B replacement for the C/C++
  native adapter: compare bind/dispatch evidence quality, build capture
  (`compile_commands` / blight instrumentation), cost and host deps against the
  current libclang floor; keep structural IDs Entmoot-owned.
- **Build and launch** local **mrva**, **Multiplier**, and **Trailmark** and
  exercise representative inventory/call-graph/query workflows for hands-on
  comparison with Entmoot CLI/MCP (note security/taint framing and store shape
  differences; do not merge their databases into KnowledgeRepository).

### Multiplier → OOTB Clang adapter (classification, not store)

Multiplier remains comparative prior art: do not adopt its Cap’n Proto/SQLite
entity index, packed `RawEntityId` keys or IR lane as Entmoot product storage.
Steal **reference-verb discipline** for the shipped libclang (and later
PASTA) CompDB sidecar. Owner: [clang adapter README](tools/resolver-adapters/clang/README.md).

Plan (implementation quality / native coverage; no Multiplier dependency):

1. **Call vs address-of vs use** — Emit call facts only for true call expressions.
   Function/method pointer decay and `&fn` feed `indirect_call` (optional
   `indirect_call:<mechanism>`), not silent `CALLS`. Aligns with Multiplier’s
   `CALLS` vs `TAKES_ADDRESS` split and Entmoot `DispatchGap::IndirectCall`.
2. **Site / owner / verb** — Keep join key = exact UTF-8 callee span; owner =
   enclosing callable; classify the verb separately from DeclRef presence
   (Multiplier `from` / `to` / `context` / `kind`).
3. **Macro honesty** — Spelling/expansion divergence stays
   `unsupported_construct` (or attributed expansion detail); never a wrong file
   span. Multiplier’s `EXPANSION_OF` motivates the gap, not a new ID space.
4. **Redecls + mangled / USR** — Correlate targets across TUs with definition
   USR/mangled name and in-repo redecl spans for join; spelling-only for
   externals. Never promote provider IDs to structural LogicIds.
5. **C++ structural edges later** — `OVERRIDES` / `EXTENDS` / `SPECIALIZES` as
   separate normalized assessments when the wire grows; not fake call targets.
   Virtuals remain open `dynamic_dispatch`; template specialization stays
   unassessed until inventory can join. Floor stays `closed_dispatch = none`.
6. **Compilation provenance** — One CompDB configuration per artifact; retain
   command identity (and env capture when blight/PASTA evaluation lands). Treat
   Multiplier’s multi-compilation DB as a contrast, not a target shape.
7. **Hands-on check** — Index a small CompDB fixture with local Multiplier
   (container on Windows hosts) and A/B call/override classification against the
   OOTB sidecar on the same sources; record gaps only—no DB merge.

Success: clearer OOTB gaps and call/indirect separation without changing join
method `entmoot.native_compiler_exact_spans.v1` or KnowledgeRepository shape.

## Instruction review and adoption

Grades assess the proposed instruction's precision and scope, not implementation
readiness: **A** adopt; **B** adopt with corrections; **C** retain a narrower goal;
**D** defer the proposed capability. The corrected instructions below govern this
pass. Initialization remains separately owned; operational UX work excludes it.

1. **Shared logging — B; active.** A common convention is useful; requiring every
   vendor/debug message to become a new shared wire schema is unnecessary.
   **Instruction:** Reconcile Entmoot-owned tracing spans/events around existing
   operation and phase boundaries. Standardize names, levels, correlation,
   durations/units, work counts and outcome meanings in the owning Rust helpers.
   Keep CLI diagnostics on stderr and machine results on stdout; preserve MCP
   protocol output. Attribute vendor diagnostics without rewriting their meaning.
   Keep high-volume detail opt-in, avoid sensitive payloads, and measure added cost.
   Logs explain execution; durable receipts and evidence remain authoritative.

2. **Operation output conventions — A; active.** Standardize semantics and
   defaults without forcing unlike operations into one response shape.
   **Instruction:** Reconcile success, partial, unavailable and failure output
   for each existing operation family through shared CLI/MCP contracts. Provide
   bounded summaries, attributable scope/limits, stable error categories and
   concise next valid actions; preserve detailed evidence access. Align defaults
   and reject ignored/inapplicable options. Keep result paging, search continuation
   and deferred detail distinct, with exact continuation guidance. Test stdout,
   stderr, exit behavior, MCP results and request/response parity together.

3. **Decompose monolithic Rust files — B; active.** File length alone is not an
   architectural defect, and splitting files can merely scatter coupling.
   **Instruction:** Split large modules along established responsibilities and
   cohesive invariants when it improves ownership or testing. Keep orchestration
   thin, share repeated logic, minimize public surface and retain the documented dependency DAG.
   Preserve observable behavior and wire contracts with focused regression tests;
   do not create new abstractions or a module-per-type solely to meet line quotas.

4. **Separate durability from processing — B; active audit/refactoring.** Ordinary
   and durable paths already share extraction, resolution and revalidation owners;
   atomic progress/evidence publication deliberately crosses some boundaries.
   **Instruction:** Audit remaining coupling before extracting it. Share computation
   over explicit inputs/results; keep leases, checkpoint persistence and publication
   in their existing repository/orchestration owners. Preserve source/generation
   fences and atomic evidence/progress commits. Compare ordinary, durable and
   interrupted results, including rollback and stale-worker cases. Use existing
   storage boundaries; do not expand scheduling or checkpoint granularity.

5. **Minimal node/edge assertions — A; active.** More independently specified
   behavior is the goal; a larger test count is not sufficient.
   **Instruction:** Expand Rust unit and integration tests in the owning crates
   with minimal equality and required-distinction pairs for each relevant identity,
   hash tier, attachment and edge relation. Include scope, direction, dispatch,
   provenance, ambiguity, negative lookups and unknown/partial cases. Retain complete
   production-graph/revision tests to catch interactions between those assertions.
   Use the [testing rules](docs/runbooks/testing.md#claim-oriented-assertions).
   Rust tests over authored facts do not replace actual native-provider controls.

6. **Failed-CI revision pairs — B; active, incremental.** A red CI run identifies
   a candidate regression, not its cause or an Entmoot expectation. Merge-only
   selection also misses useful linear and squash histories.
   **Instruction:** Select pinned upstream regressions with independently
   reproduced failing/fixed controls, including complex merges where useful. Name
   exact commits and the compared parent/window, tool/configuration inputs and
   source-backed expected impact. Separate infrastructure/flaky failures from code
   regressions. Expand supported language/evaluation combinations by demonstrated
   gaps; include unaffected controls and explicit unknowns. Reduce failures into
   fast owned Rust fixtures and retain selected full corpus cases in bounded CI.
   Follow [revision-case qualification](docs/runbooks/revision-evaluation.md#historical-regression-case-qualification).

7. **CVE/exploit/fix corpus pairs — C; active only as bounded regression fixtures.**
   Vulnerability labels or a patch cannot establish a route oracle, and structural
   reachability cannot validate exploitability or remediation.
   **Instruction:** Select published vulnerability/fix pairs whose relevant
   behavior has an independent exploit or upstream regression control with explicit
   environment and assumptions. Treat that result as attributed runtime evidence.
   Test Entmoot's changed occurrences, represented call routes, boundary evidence,
   impact and honest gaps against separately authored expectations on both sides.
   A successful fix may leave the same route intact. Never equate route absence,
   presence or a hash change with vulnerability absence, exploitability or fix
   effectiveness. A security verdict engine remains outside product scope.

8. **Whole dependency-closure scale — D; full cross-repository stitching deferred.**
   The existing repository campaign can select locked dependencies; that does not
   establish a single completely resolved cross-repository graph.
   **Instruction:** In the active pass, benchmark bounded pinned root/dependency
   corpus selections using existing publication and boundary-query capabilities.
   Account separately for analyzed repositories, build-only dependencies, unmapped
   packages, selected configurations and depth/budget gaps. Measure load, binding,
   query, storage and memory costs with correctness controls. Defer all-dependency
   graph closure and cross-repository stitching/scaling to explicit reprioritization;
   do not infer their feasibility from successful independent publications.

9. **Dispatchable asynchronous CI — B; conditional support for active tests.**
   Hosted validation fits this pass when it wraps qualified tests; a second job
   scheduler or automatic unbounded campaign does not.
   **Instruction:** Once cases and their costs are reproducible locally and justify
   a separate CI lane, add a manually dispatched GitHub Actions entry point around
   the existing campaign commands. Select reviewed pinned cases/profiles, bound
   concurrency and duration, and report passed, failed, unavailable and interrupted
   work separately. Reuse existing safe stop/restart boundaries and identity checks;
   cancellation does not imply resumability or completion. Test failure/status
   reporting; retain concise results and useful failure artifacts. Keep expensive
   cases opt-in, retain the ordinary CI backstop, and do not activate deferred cases.
   This is test hosting, not a substrate network or remote-execution feature.

10. **Churn/SAST heatmap UIs — D; graphical prototypes deferred.** Current UX scope
    is CLI/MCP, and a density metric is neither taint propagation nor a risk score.
    **Instruction for later:** Prototype read-only historical views over existing
    query results, with pinned revision comparisons, explicit churn definitions,
    normalization/denominators and attribution. If external SAST facts are supplied,
    show their provider-defined categories and coverage separately; do not rename
    generic call edges as propagators. Validate navigation and task usefulness
    before proposing a product UI. Do not add scanners, semantic propagation or
    new evidence production to make a visualization appear complete.

## Acceptance and cost controls

Preserve the existing atomicity, recovery, provenance, scope and input-identity
invariants in every active track. Use relevant serial/concurrent/restarted and
fault-injection controls for affected shared owners. Do not weaken tests, erase
limitations or silently reinterpret facts to make a broader claim pass.

Measure cold/warm, changed-input and interrupted versions of the affected workflow
with comparable source/tool/configuration fingerprints. Retain commands, results
and useful failure seeds in `.tmp/<task>/`. Report absolute cost, distributions,
build/operational overhead and storage/memory where measured, and identify gaps.
The 25% build/operational allowance remains conditional on demonstrated coverage or
utility. Larger exceptions require a reproducible high-value existing workflow,
explicit costs, bounded resources and resumability; they do not reactivate deferred
scope. Existing small-fixture recovery overheads are documented exceptions, not
blanket performance qualification. Measure first and optimize dominant shared work.

## Deferred

These are not active goals or prerequisites for completing the current quality,
UX and coverage pass. Preserve existing behavior and report its limitations; do
not expand these capabilities without explicit reprioritization.

- Remote connections/transports, network collaboration, uploads, authentication
  protocols, distributed replicas/consensus, remote execution and cross-repository
  scaling, including whole direct/transitive dependency-graph stitching. Existing
  local CLI/MCP interfaces and bounded corpus test hosting remain supported.
- SMT/Z3 integration, unsat-core/MUS extraction, proof-certificate checkers,
  standardized verification-witness interchange, theorem/assumption discharge,
  checked compositional solver proofs and automatic inter-procedural contract
  propagation. Existing evidence attachment and honest contract limits remain active.
- Proof migration across revisions, semantic refactoring verification, e-graph
  congruence closure/equality saturation and automatic claims over equivalent forms.
- New schedulers, queues, worker lanes, resource profiles or orchestration;
  additional checkpoint granularity, within-family/within-policy recovery and
  intra-producer campaign recovery. Host environment/job-cleanup machinery and
  new durable typed-canon execution remain deferred. Harden existing guarantees.
- General incremental reasoning engines, dynamic SCC/dominator maintenance,
  reachability labels and cross-revision witness repair. Measured improvements
  inside existing shared query/cache owners stay within the active quality track.
- New framework test-discovery/instrumented-coverage integrations, new automatic
  escape/exposure/entrypoint producers, alternative integration-order exploration
  and combined semantic/native/build merge qualification beyond current workflows.
- New language families, blanket C++ promotion, universal language certification,
  upstream contribution campaigns, cross-producer semantic correction and
  universal performance campaigns. Focused qualification of existing family
  constructs is active; no universal compiler matrix is required.
- TypeScript Program migration to the TypeScript **7.1** native programmatic API
  (and any stable language-server/checker surface that replaces Strada
  `createProgram` / `getTypeChecker`) for typed JS/TS coverage beyond the
  **6.0.3** Program ceiling. Keep `typescript.program/1` on 6.0.3 until a new
  adapter pin is qualified; Oxc remains the default lexical P1 producer. Do not
  treat npm `typescript@7.0` CLI/LSP alone as typed-resolution readiness.
- Disaster-recovery/import migration after store loss or incompatible layout
  changes, new signing/capability-token ecosystems and unrelated product features.
- Graphical churn/SAST-density dashboards or other new product UIs. Current UX
  work improves existing CLI/MCP operations and their attributable results.

The deferred proof milestone retains its acceptance intent: independently checked
callee evidence, call-site substitutions and guards, precondition discharge,
qualified conclusions for all relevant dispatch targets, independent derivations,
transitive invalidation and irrelevant-edit reuse, with restart/concurrency and
proof-specific cost measurements. Operational benchmarks cannot qualify it.

Prior-art references remain available for scoped implementation decisions; they
are not separate adoption projects. Existing-owner dependency tracing, early cutoff,
alpha hashing and file-local construction are described in ARCHITECTURE. Deferred
research includes [verification caching](https://pm.inf.ethz.ch/publications/LeinoWuestholz2015.pdf),
[Alethe/Carcara](https://github.com/ufmg-smite/carcara),
[verification witnesses](https://www.sosy-lab.org/research/verification-witnesses/),
[egg/egglog](https://egraphs-good.github.io/),
[GumTree](https://github.com/GumTreeDiff/gumtree),
[RefactoringMiner](https://github.com/tsantalis/RefactoringMiner),
[SymDiff](https://www.microsoft.com/en-us/research/project/symdiff-differential-program-verifier/publications/),
[CALM](https://doi.org/10.1145/3369736),
[DDlog](https://github.com/ddlog-dev/differential-datalog),
[Ascent](https://github.com/s-arash/ascent),
[Biscuit](https://www.biscuitsec.org/), [in-toto](https://in-toto.io/) and
[DSSE](https://github.com/secure-systems-lab/dsse).

Taint/dataflow analysis, scanner behavior and security verdicts remain outside
Entmoot's product scope, rather than implied future commitments.
