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
confirm all five crate owners, configure the protected release environment and
Pages, and require the exact tag's CI/native/docs gates. Do not claim publication
from local packaging. Beta requires operator feedback, representative native
project qualification and an exercised upgrade/history policy.

A remote location is a future transport boundary, not an implemented feature.
Do not add a scheduler, global package catalog or merge provenance to deliver
local automatic storage. First-use MCP inventory startup cost is measurable
follow-up if large-source operator feedback warrants asynchronous preparation.

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

## Implementation quality

- Audit shared normalization, traversal, initialization, binding, edge stitching,
  canon, persistence and processing owners. Reproduce defects with independent
  expectations, then fix the common owner and its consumers. Prefer consolidating
  or shortening existing logic over adding parallel abstractions.
- Preserve the five crate boundaries, compiler-free structural builds, typed
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
Public acknowledgements live in [README](README.md#acknowledgements).

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
   thin, share repeated logic, minimize public surface and retain the five crates.
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
