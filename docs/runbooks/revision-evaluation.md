# Revision evaluation

[Occurrence revision pairs](../../crates/entmoot-core/tests/fixtures/revision_pairs.json)
are the primary visitor controls: explicit source selectors, real occurrence IDs,
hash relationships, spans and bounded bindings. The complementary
[file revision pairs](../../crates/entmoot-core/tests/fixtures/file_revision_pairs.json)
cover reviewed whole-file hash and parse expectations. Neither lane claims
compiler acceptance. The diagnostic `validate-ground-truth` command reads the
same file-pair corpus or an explicit `--dataset`.

```text
cargo test -p entmoot-core --test revision_pairs --test file_revision_pairs
cargo test -p entmoot-engine --test revision_receipts
cargo test -p entmoot --test campaign_harness -- --ignored
```

The [SQLite harness](../../evals/harness/README.md) runs direct-parent pairs or explicitly
selected comparison windows. Each run identifies its source/tool/profile inputs.
Production measurement joins pair inventory occurrences with binding evidence;
empty reference sets, missing sides and incomparable evidence stay separate.
Caller results distinguish returned witnesses, unknowns, search truncation and
unconsumed pages. A zero denominator is ungraded, never a negative conclusion.

`report_revision_pair` reads supplied inventory, impact and relay JSON. Repeated
`--relay` and `--caller-page <address_id> <query-response.json>` arguments provide
continuation pages. The report describes those inputs, not repository ancestry,
runtime behavior or semantic agreement between producers.

Keep useful reports and inputs under `.tmp/<task>/`. A fresh namespace is needed
when source/tool/profile identities change; historical output is not a current
release gate. Keep raw output only when replay or debugging warrants its cost.

## Historical regression case qualification

The planned [workflow evaluation protocol](../../evals/PROTOCOL.md) owns upstream
case selection, sampling, independent grading and agent/resource measurement.
The [implementation sequence](../../PLANNING.md#general-workflow-evaluation)
separates harness changes from product fixes; these are not qualified CI/CVE
campaigns yet. Current occurrence/page accounting checks consistency of supplied
Entmoot outputs, not independent semantic correctness.

Before promoting a historical case, pin and verify its source/parent/configuration,
reproduce failing/fixed and unaffected controls, and author Entmoot expectations
independently. Preserve unknowns and unsupported constructs. A red CI run,
advisory or accepted patch alone is not an oracle; a route is not a security
verdict. Reduce useful failures into the owning Rust tests and retain selected
full repository cases for opt-in composition and cost checks.

## Bounded workflow experiments

Start with the pinned [Express 4.18.2–5.1.0 release cohort](../../evals/experiments/express-routing.repositories.json)
and its [workflow profile](../../evals/experiments/express-routing.workflow.json).
Eight releases cover both major versions in that interval. Twelve independently
authored routing scenarios include stable GET/HEAD/method controls and changed
optional-parameter, wildcard and regex behavior. This is one correlated framework
sample; it does not replace the proposed multi-repository historical survey.

Build matched CLI/producer and evaluation helpers, explicitly provision Node/npm
and the Python campaign requirements, and supply an official SARIF 2.1.0 schema.
Tokenizer counts additionally require `requirements-tokenizer.txt` and a populated
`TIKTOKEN_CACHE_DIR`; omit `--tokenizer` when unavailable. Use a fresh task directory:

```text
cargo build -p entmoot --features producer -p entmoot-mcp
cargo build -p entmoot-engine --features resolved-oxc,resolved-rust-analyzer --example prepare_artifact --example retain_campaign_publication
python -m evals.harness.repository_campaign run --config evals/experiments/express-routing.repositories.json --run-dir .tmp/my-eval/repositories --cli target/debug/entmoot --depth 0 --structural-only
```

Save the last command's JSON stdout as `.tmp/my-eval/repositories.json`, then run
the observer and evidence workflow (use `.exe` for Windows binaries):

```text
python -m evals.harness.workflow_campaign --config evals/experiments/express-routing.workflow.json --repositories evals/experiments/express-routing.repositories.json --summary .tmp/my-eval/repositories.json --run-dir .tmp/my-eval/workflows --cli target/debug/entmoot --node /absolute/path/to/node --prepare-helper target/debug/examples/prepare_artifact --retain-helper target/debug/examples/retain_campaign_publication --sarif-schema /absolute/path/to/sarif-schema-2.1.0.json --tokenizer o200k_base --native
```

The source/build runner owns installation and retains each resolved lockfile in
`build/<case>/`. The profile explicitly enables npm lockfile generation despite
upstream `.npmrc` defaults. Replay the retained lock with `npm ci --omit=dev
--ignore-scripts --no-audit --no-fund --package-lock=true`; resolving dependencies
again is a new configuration, even at the same source commit. Keep binaries,
adapters, lockfiles and source bytes fixed during a run. Copy matched CLI,
`entmoot-producer` and helper executables into a task-owned directory before
running a study alongside Cargo; pass those copied paths. The runner rejects
changed tool identities and preserves its evaluator sources. Initialization uses an
explicit experiment data root and read commands never install tools.

The workflow keeps six kinds of evidence separate:

- `campaign.sqlite3` contains pinned inputs, command observations and independent
  grades. Every attempt has distinct stdout/stderr and `process.json`, including
  failed attempts; changed inputs or failed steps require a fresh run namespace.
- `knowledge.sqlite3` is the production KnowledgeRepository database. Its
  `views/*.view.json` locators select library or consumer revisions, retaining
  source inventory, native facts and provider findings through Rust APIs.
- `fixtures/` contains independently versioned observer sources and dependency
  selections. Oracle expectations remain outside those analyzed checkouts.
- `providers/` inventories SARIF by case and producer. The reusable provider
  verifies source digests, exact UTF-16 spans and scenario/configuration identity.
  Twelve scenarios at one callsite are twelve observations, not twelve sites.
  `actual` and verification properties remain in the raw artifact; typed findings
  expose message, rule, location and provenance. No call edges are invented.
- `progress.json` preserves completed case dispositions; `report.json` records
  final grades, view locators, operation costs and paired compact/export payloads.
  Compact finding pages must all be consumed and exact references expanded.
- Native gaps, missing optional raw objects and dependency scope remain explicit.
  Retaining facts does not qualify an unsupported dynamic or cross-package edge.

Process-tree memory and CPU are sampled every 50 ms; short-lived children can be
missed and summed RSS can count shared pages twice. Report per-operation spread,
setup/retention separately and host contention. Output bytes, Unicode characters
and named-tokenizer counts measure payload volume. Grep snippets are a volume
control, not an equivalent completed investigation. Actual model input/output,
reasoning usage and agent savings remain unavailable without paired agent trials.

After builds and other study processes finish, repeat just the read operations:

```text
python -m evals.harness.benchmark_workflow --report .tmp/my-eval/workflows/report.json --cli /absolute/path/to/frozen/entmoot --run-dir .tmp/my-eval/query-benchmark --repetitions 5 --tokenizer o200k_base
```

This alternates compact/export order, records request and response costs, and
rejects missing pages or changed finding sets. It measures fresh CLI processes
with warm caches. Tool hashing and the Python coordinator are outside measured
child-process intervals; total coordinator memory and cold-cache costs are not
established. Compact listing savings exclude subsequent evidence expansion.

### Release cohort evidence, 2026-10-17

The frozen local study completed all eight selected releases: 96/96 authored
runtime observations, 96 exact-callsite attachments and 96 exact-reference
expansions. All 16 historical views returned verified source, and 1,859 source-file
memberships matched their original bytes. Independent `npm ci` materializations
reproduced all eight lock digests, indexed file sets and snapshot identities.
The shared production database occupies 1,357,611,008 logical bytes. Source,
native evidence, provider claims and evaluation grades retain their separate roles.

The study, dataset locators and preserved failures are under
`.tmp/workflow-evaluation/`; `research-summary.json` names the selected artifacts.
`express-routing-frozen/report.json` owns the 303-command study and
`query-benchmark-quiet/report.json` owns the final 87 read-only measurements.
Earlier calibration runs exposed Windows path handling, incomplete pagination,
disabled upstream lockfiles, incorrect tag-object pins, newline/token accounting
and executable replacement during a concurrent build. They remain separate from
the frozen run. Full-cohort timing retains its earlier 50 ms polling method;
the final query sample uses exit-aware waiting after validation processes ended.

Across 40 paired listing queries, median stdout was 1,455 `o200k_base` tokens
for compact and 9,091.5 for export; the median paired reduction was 84.1%.
Both returned the same twelve finding IDs. Median process wall time was about
34 ms for either tier. Fast commands have incomplete CPU/RSS samples; these
figures do not establish exact peaks, cold-cache behavior or total coordinator
cost. No model was run, so reasoning/billing savings remain unmeasured.

Raw impact exports were much larger: a median 10.66 MB / 4.07 million tokens per
comparison. Keep these offline. The existing `relay-impact` operation returned
an initial bounded page at a median 1,519 tokens / 2.59 s across seven pairs.
The 4.21.2→5.0.0 page exposed one of 65 changed callables and retained continuation
and completeness gaps. That first page is not equivalent to the complete impact
report; follow cursors when the question requires more evidence.

For an upgrade investigation, query `finding` with `query: "optional_parameter"`
against each consumer view and inspect the returned exact ID. The observed
registration changes from accepted in the selected 4.x configurations to throwing
in 5.x; literal-route and method controls remain stable. These are recorded runtime
conditions, not an invented call path into Express or a security conclusion.

The sample supports bounded observation/attachment/retrieval utility. It is one
correlated framework family, not 96 independent ecosystem trials. Native dispatch
and dependency-path completeness remain partial; the proposed 80-episode history
survey and paired agent trials are still planned. The maturity table therefore
adds this scoped evidence without promoting those broader capabilities.

### Local history pilot and agent calibration

The local [history configuration](../../evals/experiments/local-pilot.json) fixes
eight repositories, a date window, seed, metadata/source budgets and disjoint
stratum quotas. `harvest` freezes the observed frame before Entmoot runs. It reads
public Git objects and bounded GitHub Actions metadata through an authenticated
`gh`; it never reruns upstream CI. Missing/capped metadata and underfilled strata
remain in `selection.json`. An ordinary episode is not an independently verified
passing build. A failed-CI candidate is not a reproduced defect. Source-neutral
means no selected-extension byte change, not behavioral equivalence.

Use a matched locally built CLI and retention helper, an isolated data directory
and a new run directory for each attempt. Keep the selected binaries unchanged
while a study runs. On Windows, activate the maintainer tool environment first;
append `.exe` to executable paths below. These recipes run from this repository.

```text
python -m pip install -r evals/harness/requirements-agent.txt
cargo build --locked -p entmoot --features producer --bins
cargo build --locked -p entmoot-engine --example retain_campaign_publication
python -m evals.harness.history_pilot harvest --config evals/experiments/local-pilot.json --run-dir .tmp/pilot/frame
python -m evals.harness.history_pilot run --selection .tmp/pilot/frame/selection.json --run-dir .tmp/pilot/study --cli target/debug/entmoot --retain-helper target/debug/examples/retain_campaign_publication
```

The structural pilot independently compares selected source paths and content
against Git blobs, then checks added/deleted/modified file sets. It retains
immutable views through the production repository API in `knowledge.sqlite3`;
evaluation grades stay outside that database. This lane installs no upstream
language dependencies and does not qualify typed dispatch or behavioral impact.
Keep the shared database, view locators, source acquisitions, commands and frozen
inputs together when moving the local study. The run command can reacquire pinned
objects when the original frame's mirror directory is absent.
Owned clones set local `core.autocrlf=false` and `core.eol=lf` before checkout,
preserving authored attributes while excluding host defaults. Reuse rejects a
different checkout text policy; use a fresh run instead of rewriting prior study
sources. Explicit repository checkout transforms can still differ from raw Git
blobs and must remain visible to the oracle.

If initialization completed but retention was stopped, preserve the run. Once the
coordinator and its children have exited, record that observation in
`interruption.json` with `status: "stopped_after_children_exited"`. A single
explicit continuation can reuse initialization:

```text
python -m evals.harness.history_pilot resume --selection .tmp/pilot/frame/selection.json --run-dir .tmp/pilot/study --cli target/debug/entmoot --retain-helper target/debug/examples/retain_campaign_publication --previous-runner-digest <inputs.json harness_sources.history_pilot.py>
```

Continuation requires the original product, evaluator and harness dependencies.
It records an explicit coordinator version transition, archives earlier grades,
and replays **all** retention and independent grading through the same owner as a
fresh run. Existing locators are not proof of a completed check. Repeated work
remains in measured costs; command streams without final receipts have unknown
costs. Finished reports and existing continuations cannot be overwritten.

The [agent configuration](../../evals/experiments/local-trials.json) defaults to
six paired tasks, one repetition and at most twelve attempts, each capped at two
minutes. Its `provider` configuration records the adapter, model and reasoning
setting; review these for
the local account before running. The checked-in values match the calibration
environment, not a claim about the best available model. A local authenticated
Codex executable is required for the implemented `codex_cli` adapter; model calls
use that account's allowance. `--provider-executable` selects an explicit path.

```text
python -m evals.harness.agent_trials prepare --config evals/experiments/local-trials.json --pilot .tmp/pilot/study --run-dir .tmp/pilot/trials --cli target/debug/entmoot
```

`prepare` checks the provider version, freezes tasks, independent answers, source
bytes, store/graph identities, tools and budgets, then stops **without model
calls**. Review `preparation.json`, `inputs.json` and `oracles.json` at this boundary.
Keep the pilot quiescent. The following explicit `run` starts model calls; it
rechecks frozen inputs and refuses changed material or an already-started schedule:

```text
python -m evals.harness.agent_trials run --pilot .tmp/pilot/study --run-dir .tmp/pilot/trials --cli target/debug/entmoot
python -m evals.harness.agent_trials report --run-dir .tmp/pilot/trials
python -m evals.harness.pilot_report --pilot .tmp/pilot/study --trials .tmp/pilot/trials --out .tmp/pilot/summary.json
```

The last two commands regrade and summarize stored evidence without calling a model. The two
arms have the same bounded Git, literal-search and source-read interface; one arm
also has `find`, `inspect`, `boundaries` and an initial `relay-impact` page. Built-in
model execution tools are disabled. The host executes validated JSON actions and
keeps oracle files out of model context. This is a constrained action experiment,
not a security sandbox or a full CLI/MCP session benchmark.

The trial loop depends on `AgentProvider`, which returns a decoded action,
normalized usage, status and measurement. Invocation flags, response events,
tool-isolation checks and executable identity belong to the provider adapter;
the loop owns action validation, budgets, tools and grading. A non-Codex fixture
provider exercises that boundary without model calls. Only Codex has a live
adapter today; other providers need their own adapter and qualification before
comparative claims. Unknown provider selections fail rather than falling back.

File-review tasks use Git as a strong baseline. Python callsite tasks use an
independent `ast` oracle: direct-name/attribute calls with a selected spelling
prefix, their source lines and named lexical scope. Definitions, attribute reads,
computed names and runtime dispatch are different claims. The grader requires
source-read witnesses for returned callsites and complete cited pages for file
delta answers; shorter incomplete answers do not earn a pass.

Reported input/cached/output/reasoning tokens come from Codex completion events.
Cached tokens are a subset of input; reasoning tokens are a subset of output.
Do not add either subset twice. Each action uses an explicit-history model call,
whose repeated context and startup cost are included. Token stopping is checked
between calls, so the final call can exceed that allowance; wall time is enforced
per attempt. Failed/incomplete usage remains unknown, with known subtotals shown.
Per-call counters remain in the receipts. Known subtotals still enforce the token
allowance when a provider omits another counter; missing costs are never zero.
Billing, private reasoning and exact server model weights are not inferred.

Keep calibration separate from confirmation. The larger schedule is explicit
on `prepare`: `--task-limit 20 --repetitions 3 --max-attempts 120`. It requires a separately
authorized model budget and frozen, eligible tasks; increasing repetitions does
not create new repository families or qualify semantic-upgrade workflows. Retain
all attempted cases and report setup separately from query/trial costs.

#### Source navigation example: Requests

The pilot includes Requests commit
`00600b38f45dc234f825f609cea63d24e7e690af`. After running the frozen selection,
these PowerShell commands reuse its existing structural publication:

```powershell
$cli = './target/debug/entmoot.exe'
$source = '.tmp/pilot/study/publications/sources/requests-00600b38f45dc234f825f609cea63d24e7e690af'
$data = '.tmp/pilot/study/publications/data'
& $cli --standalone --data-dir $data -C $source find get_connection --output-limit 5
& $cli --standalone --data-dir $data -C $source boundaries --callee '*get_connection*' --path src/requests/adapters.py --output-limit 5
& $cli --standalone --data-dir $data -C $source read-source src/requests/adapters.py --start-line 606 --end-line 618 --context-lines 0 --output-limit 13
```

The first query has 17 matching subjects and returns five: callable definitions,
local declarations and call stubs can all match the name. The boundary query
narrows this file to one represented call, `self.get_connection_with_tls_context`.
The verified source places it at line 612 inside `HTTPAdapter.send`. That is
useful navigation for a transport review; it does not resolve the receiver's
runtime class or establish a safe dependency upgrade. Repository, dependency and
native-library completeness remain unestablished in this structural publication.

Use exact IDs for inspection and retain the source digest with a cited span.
Advance `page.next_cursor` when the complete subject set is needed; use explicit
line and presentation limits for source reads. For exact added/deleted/modified
file lists, use `git diff --no-renames --name-status <base> <head>` with the same
source-extension scope. A bounded callable-impact page answers another question.

## Local branch collaboration

Register already produced graph exports in one repository store:

```text
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 register --graph base.json --workspace base --publisher agent
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 register --graph topic.json --workspace topic --publisher agent
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 capture --repository example/repo --base-workspace base --workspace topic
```

Save the capture response as request JSON, then use `submit --request <file>
--key <unique-key> --owner <agent>`. An empty peer selection captures every other
active workspace. `run --id <id> --worker <agent> --pairs 10` executes a bounded
slice; `resume --id <id>` makes a paused job runnable. `status`, `pause`, and
`cancel` operate on the same durable ID. Read committed reports with `outputs
--id <id> --after-sequence 0 --limit 20`, advancing the returned sequence. The final
operation receipt contains the completed pair count. Registration updates require the current
`--expected-generation`; `retire` releases a workspace pin without resetting its
generation. Reusing a submission key with different inputs is an error.

`compare --base base.json --left left.json --right right.json` performs a direct
structural comparison. Add `--git-root <checkout>` for isolated Git observations
of committed snapshots. For uncommitted inputs, first register each structural
view, then run `capture-git --workspace <id> --expected-generation <generation>
--publisher <agent> --source-root <worktree>`. Capture advances the generation and
retains raw Git inputs atomically. Defaults are 10,000 paths and 64MiB; explicit
`--max-files`/`--max-bytes` can raise them up to 100,000 paths/1GiB. Publication is
atomic. After interruption, inspect the workspace generation or retry unchanged
inputs: the commit may have succeeded before its response arrived. It rechecks source,
index and file inputs; quiesce edits during capture for a consistent observation.
Add `--git-merge` to the subsequent peer `capture` request and provide
`--git-root <checkout>` on every `run` slice. Later edits do not change those
frozen inputs. Parent Git objects must remain available in that checkout.
Raw-byte merges exclude clean/EOL/encoding transforms and index-only selection.
Ignored untracked files and `.tmp/entmoot-conflicts/` scratch are excluded; tracked
ignored files are retained. Unmerged indexes, sparse/skip-worktree inputs and
submodules return unavailable.
No finding, or a clean textual merge, proves semantic
compatibility. Current comparisons include direct represented dependencies;
transitive and unrepresented interactions remain explicit limitations.

To test cumulative integration, freeze one explicit order after capturing all
working-tree inputs (committed snapshots need no raw capture):

```text
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 capture-integration --repository example/repo --workspace base --member topic-a --member topic-b
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 submit-integration --request ordered.json --key integration-1 --owner agent
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 run-integration --id <id> --worker agent --git-root <checkout> --steps 1 --max-staged-bytes 67108864
entmoot collaborate --store .tmp/collaboration/knowledge.sqlite3 resume --id <id>
```

Save the first response in `ordered.json` before submission. Repeated `--member`
options preserve order; duplicates are rejected. Repeat `run-integration` after
`resume` to continue. Each saved prefix includes synthetic merge ancestry, its
new private Git objects and an output report committed atomically. Interrupted
work can repeat the current merge; completed merges are restored from saved
objects. `--wall-time-ms` stops admission between Git steps. The default 64MiB
saved-pack allowance can be raised on later runs up to 1GiB; it does not bound
temporary/decompressed Git objects. Each Git process has a 30-second limit.

Use `outputs` for step reports and `status` for counts. Completed with `conflict`
means the first conflict stopped the specified order; later members are untested.
Unavailable history, tool-version changes and exhausted allowances preserve the
last prefix. Restore the original history/environment before retry; changed Git
versions require a new request. Completed/cancelled jobs release private objects,
so the final synthetic commit ID is descriptive and never updates source refs.
Clean textual integration does not establish combined build/semantic correctness
or compatibility under another order. The opt-in cost probe is:

```text
cargo test -p entmoot-engine --test integration_operations integration_restart_cost_probe -- --ignored --nocapture
```

The eight-member Windows probe measured 7.71s median in one run versus 14.99s
across eight single-merge slices; initial nine-workspace capture took 25.39s.
Larger slices amortize private repository/process restoration. These small-input
costs justify explicit recovery, not a large-repository throughput guarantee.

MCP accepts `--collaboration-store <database>` and exposes `collaboration_read`
and `collaboration_write` using the same portable contracts and engine functions.
Their MCP arguments contain a `request` object holding the tagged operation.
This is local operation; remote networking and upload/authentication are deferred.
MCP capture and Git-enabled slices use `--collaboration-source-root` as their
host-local locator. Inventory/canon recovery is described in
[agent runtimes](agent-runtimes.md), together with native family recovery and
conditional revalidation. The explicit local revalidation probe
compares 32 changed/stable/fresh policies in a shared store across a synchronous
ledger, one durable run and single-policy restart slices:

```text
cargo test -p entmoot-engine --test revalidation_operations revalidation_restart_cost_probe -- --ignored --nocapture
```

It reports alternating samples, setup time and per-mode/total database growth
for all nine executions. Costs include retained-view loading and ledger publication;
durable runs also include submission/control and report persistence. This small
fixture does not qualify large-repository memory, build or throughput costs.
The Windows sample measured median 0.40s synchronous / 0.447s durable, with one
durable run at 2.31s. Single-policy slices took 2.26–4.75s. Median database growth
was 1.36MB / 1.97MB, including stored reports. Use larger execution slices to
amortize retained-view loading; saved policies remain recoverable within a slice.
Storage and frequent-restoration costs exceed 25% and stay an explicit opt-in
tradeoff for durable recovery, not a default overhead guarantee.

For standalone traversal use `neighborhood --resumable --durable` (optionally
`--continuation-idle-seconds <seconds>` for longer retention); for the backend use
`--durable-continuations`. Checkpoints retain exact search/replay state and expire
under the configured idle lifetime (backend default 300 seconds). Restart does
not waive request, evidence, publication or implementation fences. Capacity/disk
failures preserve the prior persisted checkpoint; start a new search or explicitly
raise the relevant resource allowance. These flags write operational state only.

The revision campaign runner passes `--data-dir <run-dir>/data` to every CLI
command. This run-owned volume takes precedence over environment and workspace
configuration; existing source-tree `.entmoot` data is left untouched. Use a fresh
run namespace after input or runner-layout changes.

The runner accepts `--max-operations <count>` and
`--wall-seconds <seconds>` on `run`. Both stop admission between completed
revision/comparison units. Create `<run-dir>/stop.request` for the same graceful
stop; remove it to resume. Repeat the command with the same configuration and run
directory. A paused response is not a completed campaign. The OS releases the
executor lock on exit; the lock sidecar remains reusable. A forcibly killed or
failed producer unit still requires a fresh namespace because its external side
effects are not safely replayable. Producer timeouts remain the per-unit bound.

Campaign checks require `evals/harness/requirements.txt` in the selected Python.
`ENTMOOT_CAMPAIGN_PYTHON` explicitly selects it. The commit gate resolves its
default from PATH and strips repository-local Git hook variables from test child
processes; staged-file selection still uses the original hook context.
On Windows the validation environment defaults to at most six Cargo build jobs
to bound large native-feature linker working sets. An explicit `CARGO_BUILD_JOBS`
from the host takes precedence; this setting does not change the selected tests.
