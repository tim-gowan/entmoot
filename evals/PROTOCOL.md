# Workflow evaluation protocol

This protocol separates the implemented release cohort below from a proposed
historical CI/CVE and agent survey. The
[remaining work](../PLANNING.md#general-workflow-evaluation) separates harness
changes from product fixes. Runtime controls independently grade their finite
scenario set; graph accounting alone cannot establish semantic completeness.
A successful command, red build, advisory or accepted patch is not an oracle.

## Implemented bounded cohort

The [Express release profile](experiments/express-routing.workflow.json) implements
an initial calibration cohort: eight pinned releases from 4.18.2 through 5.1.0,
including both majors, with twelve authored runtime scenarios per release. It
uses the shared measurement owner, generic source-verified SARIF observations,
exact-reference checks and historical production views. Three query repetitions
per release are descriptive calibration, not the proposed five-repetition/three-
fresh-store survey. The source/lock and invocation recipes live in the
[revision runbook](../docs/runbooks/revision-evaluation.md#bounded-workflow-experiments).

This cohort is separate from the eight curated mechanism cases and 80-episode
history pilot below. Its 96 scenario executions are correlated, not 96 independent
ecosystem trials. No held-out agent trial, billed usage or reasoning-cost savings
is established. Local profiles remain harness configuration; portable wire shapes
continue to come from Rust and generated schemas.

## Reusable case and ownership

The evaluation unit is an operator question under a pinned source/configuration
scope, with admissible evidence, an independent oracle and a stopping rule. For
example: identify custom transport subclasses affected by a Requests upgrade,
return supported callsite witnesses and relevant tests, and explicitly mark any
unresolved dispatch. Parameters select real subjects and budgets; the workflow
must not contain fixture-specific answers or unrestricted shell templates.

Keep six responsibilities separate within the existing harness:

- **Selection** records the sampling frame and candidate provenance. It cannot
  declare a case correct or silently discard failures.
- **Materialization** reuses repository checkout/build recipes, exact Git objects,
  dependency locks and explicit source/data roots. Installation is an explicit
  setup phase; reads never install or refresh producers.
- **Oracle execution** runs independently authored tests and reviewed source
  expectations in isolated environments. Oracle material is unavailable to the
  agent when it would reveal the answer.
- **Workflow execution** uses public CLI/MCP operations with typed inputs and
  bounded continuation. Keep hidden index operations for existing harness
  controls; initialization UX must exercise public `init`. No SQL shortcut for
  the agent or presumed graph edge across an unqualified dependency boundary.
- **Measurement** observes commands and agent interactions through one shared
  process-measurement owner. Preserve each runner's publication, timeout and
  restart guarantees; instrumentation must not introduce a scheduler.
- **Grading/reporting** joins immutable observations with a separately frozen
  oracle. Reports consume stored evidence; they do not rerun tools or mutate truth.

Retain `revision_campaign` input validation and transactional `CampaignStore`,
the repository/revision entry points, and occurrence/page accounting. Extend
portable campaign contracts in Rust and regenerate their schemas. Keep host
collection and statistical analysis in Python, outside product query semantics.
Use existing receipt storage with distinct case/attempt/step identities before
adding indexed tables for a measured query need. Never overwrite a failed attempt
with its successful retry or admit old receipts by relaxing fingerprints.

## Real workflow candidates

These are research leads requiring local reproduction and full Git-object
verification. Upstream descriptions establish a reason to investigate, not an
Entmoot result. Map every public operation to a workflow/control, but run only the
operations needed for each question; a case need not execute every CLI leaf.

- **Navigation and dependency upgrades — JS/TS, Python, Rust.** Reuse the Express
  route-syntax migration with an independently selected real consumer and locked
  router dependencies. Trace URI location → registration → supported internal
  routing occurrence; HTTP association is separate from a call edge. Requests
  [PR 6710](https://github.com/psf/requests/pull/6710) provides a class-dispatch case:
  find `HTTPAdapter` subclasses affected by `get_connection_with_tls_context` and
  exercise a local fake transport. Axum
  [PR 2645](https://github.com/tokio-rs/axum/pull/2645) supplies route/nest/TypedPath
  controls for matchit 0.8. Compare unpruned and explicitly pruned witnesses;
  include an alternate route and an excluded endpoint. Neither pruning nor an
  unresolved constructor/dynamic call establishes global absence.
- **Change review, compatibility and test selection — Python, Go.** Select small
  response/model-construction subcases from FastAPI's Pydantic v2
  [PR 9816](https://github.com/fastapi/fastapi/pull/9816). Compare independently
  specified response tests across dependency configurations; keep the Python/Rust
  extension boundary explicit. For Go's
  [ServeMux changes](https://go.dev/blog/routing-enhancements), use
  `GET /posts/{id}` with GET/HEAD/POST and `PathValue` controls, pinning toolchain
  sources and the compatibility setting. An inventory overlay, API compatibility
  evidence and observed test behavior answer different questions.
- **Clone and component review — JS/TS, Go.** Independently select a handler or
  plugin family in Fastify, Gin or chi. Compare hash groups and observed outgoing
  calls, then check an audit/middleware policy with runtime counters. Include an
  intentional exception and a labelled injected omission. There is no claimed
  natural missing-log defect here. Plugin/callback entry points are controls
  against treating absent represented callers as proof of unused code.
- **Publication, claims and identity inspection — C#, Java, C.** ASP.NET Core
  [PR 61623](https://github.com/dotnet/aspnetcore/pull/61623) and its
  [servicing change](https://learn.microsoft.com/en-us/aspnet/core/breaking-changes/8/forwarded-headers-unknown-proxies?view=aspnetcore-10.0)
  give trusted/untrusted proxy configuration controls. Spring's
  [CVE-2024-38816](https://spring.io/security/cve-2024-38816/) gives explicitly
  configured `RouterFunctions`/`FileSystemResource` cases; separate WebMvc.fn and
  WebFlux.fn. Curl's [CVE-2023-38545](https://curl.se/docs/CVE-2023-38545.html)
  supplies a guard/state-machine change where a call route may remain unchanged.
  Exercise `init`, explicit binding/attachment, artifact and assertion inspection
  against these sources. Schema-valid mock SARIF qualifies attachment, location,
  provenance and query behavior only; independently pinned real producer output
  is another lane. A claim, advisory match or route is never a security verdict.
- **Producer handoff and deployment.** On a pinned Spring or ASP.NET build,
  prepare work with the existing analysis/work operations, execute the selected
  adapter in its qualified environment, and ingest its artifact. Check missing
  SDK, stale source/configuration, duplicate delivery and cancelled work. Compare
  ordinary sandbox operation with host setup; Roslyn ad-hoc success does not
  qualify MSBuild project mode. No remote service or new execution queue is needed.
- **Collaboration and recovery.** Select actual multi-parent Requests, Axum or
  Pydantic commits and matched linear/squash controls. Capture/register snapshots,
  compare each selected parent and replay an explicit integration order. Compare
  uninterrupted results with pause/restart and stale-worker controls in disposable
  stores. Only an independent parent/merge regression establishes an integration
  defect; a clean textual merge does not establish compatibility.
- **Retention.** Reuse that revision series to plan/apply retention in a disposable
  volume. Verify pinned publications and historical reads, then verify explicit
  unavailability for deliberately retired evidence. Measure storage reclaimed,
  query cost and restart behavior with stores closed where required. These are
  lifecycle invariants, not a reason to duplicate the entire semantic corpus.

## Sampling and historical layers

Start with eight curated cases drawn from the candidates above to qualify the
harness. Keep them separate from a proposed 80-episode pilot: ten revision
episodes in each of Express, Fastify, Requests, docker-py, Axum, getrandom, chi and
Gin. These emphasize JS/TS, Python, Rust and Go; they are purposively chosen, not
a random ecosystem sample. Java, C# and C initially remain configuration-specific
challenge cases. C++ remains experimental. A dependency-linked pair of repositories
is not two independent replications of the same fix.

Freeze the pilot frame before examining Entmoot results: proposed dates are
2025-10-01 through 2026-09-30, seed 20261017. Per repository, draw four failed-CI
repair candidates, four ordinary history episodes and two independently classified
neutral controls. Keep these strata disjoint; neutral does not mean merely that
Entmoot returned no change. Record the candidate universe, selection order,
eligibility, missing logs and exclusion reasons. Underfilled strata remain visible;
do not quietly substitute famous bugs. This stratification measures workflow
utility, not failure prevalence. The pilot informs a later sample-size calculation.
Declare metadata, source-size, build-time and disk budgets before harvesting;
record censored/over-budget candidates and the resulting sampling-frame limits.

The [Actions API](https://docs.github.com/en/rest/actions/workflow-runs) limits a
filtered search to 1,000 results. Partition dates, paginate, deduplicate run IDs
and record queries, API version, retrieval time and any cap. Preserve run attempt,
job/matrix, event, workflow revision and the actual checked-out commit, alongside
PR base/head and eventual landed commit. For
[`pull_request`](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request),
the default tested SHA can be a synthetic merge; inspect checkout configuration
instead of assuming the PR head. Expired logs, infrastructure failure, same-SHA
flakiness and reproduced code failure have different dispositions. Read historical
CI; do not rerun upstream workflows to populate the sample.

Verify full commit/tree/all-parent objects locally. For a two-parent merge, retain
merge base O, parents B/H and recorded merge M; use O→B, O→H, B→M and H→M only as
declared comparisons. Add M→F when investigating a later fix. Record the chosen
parent and merge-base policy, including multiple-base/unsupported outcomes.
Linear P→C and explicit non-parent windows remain valid. Runtime integration
claims require comparable parent/merge tests; missing tests or changed environments
cannot be treated as passing parents. Synthetic CI merges and landed merges are
distinct revisions.

Persist linked observations at their natural grain: selection event → revision
and configuration → file/occurrence → binding/witness/claim → executed step →
independent grade. The earlier survey's file, callable, binding and frontier
layers are useful discovery inputs. Its stored success flags, hashes and merge
fields do not qualify current semantics or ancestry. Extract candidate revisions
read-only, verify them, and rebuild current evidence; do not migrate old rows into
current publications or count binding rows as independent experiments.

Assign one primary question per episode; secondary questions share its cluster.
Separate development and held-out cases before tuning, targeting 60/20 where
stratum and fix-family grouping permit. Group backports, related dependency fixes
and repeated revisions together. Record both planned and achieved allocation.
Published cases can be present in model training; use recent held-out cases where
possible and disclose that contamination cannot be excluded.

## Independent correctness and soundness

Freeze expected source locations, relationships, changed/stable distinctions and
configuration before reading Entmoot answers; join product IDs afterwards. Use
buggy/fixed regression tests plus unaffected passing controls, following the
separation illustrated by [SWE-bench's grading harness](https://www.swebench.com/SWE-bench/api/harness/)
and [Defects4J](https://github.com/rjust/defects4j). Evaluate upstream source/tests
in a pinned isolated environment and record patch application, dependencies,
features and any reproduction failure. Independently review the oracle; a model
judge or Entmoot-generated expected graph cannot be the sole authority.

Grade individual assertions and the whole task. Preserve supported, contradicted,
unknown, unavailable and out-of-scope outcomes with denominators. Report precision
among adjudicated assertions alongside unknown counts and answer coverage. Report
recall only against a finite independently established reference set, naming its
scope. A dynamic trace confirms exercised edges; an unobserved edge is not a
negative oracle. Hash equivalence is not behavioral equivalence. Missing evidence,
pruned search and exhausted pages never establish absence. Validate every cited
location and returned reference, including compact-to-detail expansion.

For test selection, measure retention of independently known failing tests,
unaffected control coverage and execution cost; retain ownership floors and show
unselected/unknown obligations. Test coverage does not prove assertion strength.
Keep injected mutations separate from natural regressions. Blind reviewers to
the evaluated arm, double-review at least 20% of cases plus disputed outcomes,
and retain agreement and adjudication reasons. Predeclare whether the task is
prediction before a change or diagnosis after it; disclose exactly which patch,
CI logs and documentation both arms receive. Keep oracle tests and future fixes
out of the agent context when they leak the answer.

## Measurement and paired agent evaluation

Record exact source, dependency, binary/adapter, policy, workflow, oracle, scorer,
prompt/tool-schema and environment identities. Include OS/architecture, CPU/RAM,
toolchain/features, dirty changes and path overrides. Capture argument vectors,
working directory, allowlisted environment, monotonic durations, attempt order,
exit/timeout and output truncation. Never archive credentials or hidden reasoning.
Retain selected response bytes and failure logs when needed to reproduce a score,
audit a citation or recount tokens; digest/count other output. Record unavailable
measurements with a reason rather than zero.

- **Operational cost:** separate checkout/install/build, init/bind, query,
  projection/serialization, agent, recovery and cleanup. Measure elapsed and CPU
  time, process-tree coverage, peak memory, I/O where available, and logical/
  allocated storage growth including WAL, dependencies and retained artifacts.
  Label sampled process-tree RSS with its interval and missed-child limits; it
  is not an exact OS peak or unique physical memory. Prefer OS process/job
  accounting for terminated children where available; otherwise identify sampled
  CPU/process coverage. Avoid double-counting nested spans. Use internal phase
  timing only where instrumentation actually exists.
- **Agent cost:** retain bytes, Unicode characters and named-tokenizer counts
  separately from actual model usage. Count requests, schemas, stdout/stderr,
  all pages, failed attempts, source reads and repeated context. Record provider
  input/cached/output/reasoning usage only when exposed, with model/settings and
  accounting semantics; reasoning may be included in output totals. Missing
  reasoning usage cannot be inferred from characters. Price calculations need a
  dated price schedule; cache/transport duplication is not automatically double
  billing. Record observable actions, decisions and evidence references, not
  private chain of thought.
- **Operator utility:** time to first independently correct signal, task success,
  unsupported assertions, unresolved obligations, tool calls, selector errors,
  retries, continuation/drill-down burden and successful exact-reference expansion.
  Small responses earn credit only when the required answer remains correct.

Use two paired arms with identical tasks, source access, model/settings and
budgets: bounded `rg`/source reads plus ordinary Git/build/test tools, versus
Entmoot plus the same fallback tools. Disclose unavoidable tool-description
differences and count them. Separate scripted operation benchmarks from adaptive
agent trials. Test compact/full output and concise operating-skill guidance as
explicit ablations on a subset, not a large language × operation × mode matrix.
Skills must contain decision procedures, not case answers.

For the initial scripted pilot, propose five alternating warm query repetitions
and three fresh-store runs per selected case; record build/install separately.
Fresh store/process does not mean cold OS cache. Calibrate observer overhead and
record whether MCP process/store reuse differs from CLI startup. After harness
qualification, propose three agent repetitions on each of 20 held-out primary
tasks per arm (120 attempts), under predeclared per-task token/time limits. Freeze
limits after calibration; expose timeouts and stopping reasons. No paid runs are
implied by this protocol. Report setup cost separately and amortized over declared
task counts, including when grep is cheaper.

Report paired differences, medians/spread and per-repository results. Use
repository/fix-family clusters for uncertainty, not individual edges as samples;
eight repositories give limited inferential power. Include all-attempt cost and
success-conditioned cost, attrition and unknowns. Do not remove difficult failures
to improve the headline. Predeclare primary task success and paired cost outcomes;
label other subgroup comparisons exploratory and report interval methods. Choose
wider-survey languages/workflows from reproduced
correctness, answer coverage, utility and collection cost together. Pin the
analysis script and sampling seed; retain a bounded redistributable artifact with
licenses, input acquisition recipe, oracle, observations and report commands.
Distinguish rerunning that artifact from independent replication elsewhere.

## Fixes and operational guidance

For each defect: preserve the original attempt → reproduce in scratch → author an
independent failing control → fix the common owner → run focused and required
shared checks → rerun affected cases under a new implementation identity. Never
rewrite an oracle just to match a new output. Record legitimate oracle corrections
and rescore both arms. A held-out case used to tune a fix becomes development data;
reserve a new case for subsequent confirmatory evaluation.

Promote small offline controls to the existing owning test suites and retain only
selected pinned upstream/native cases for opt-in composition and scale checks.
Update test ownership/features and generated contracts with implementation.
Promote observed operational lessons into the existing operate/release skills
and runbooks: intent, prerequisites, bounded first query, sufficient evidence,
unknowns, exact follow-up and stopping rule. Initialization/deployment guidance
must cover matched binaries, explicit adapter setup, relocated roots, readiness,
native project-mode controls and passive reads. Deployment smoke checks and old
publication retrieval do not substitute for release qualification.

Keep tool descriptions short and typed: required context, result meaning,
limitations, continuation and one usable example. Rustdoc/schema own semantics;
runbooks own procedure; skills own decisions. AGENTS.md already names the affected
boundaries to check; add guidance only for a demonstrated missing invariant.
