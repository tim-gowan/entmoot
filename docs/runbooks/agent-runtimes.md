# Agent runtimes

Start `entmoot-mcp -C <source>` or run the CLI inside the source repository.
Frontends prepare missing structural inventory and select local storage automatically;
`--data-dir` optionally selects its location. Use `init` to refresh the baseline and
`bind` to request native evidence. Explicit collaboration writes use shared engine
operations through CLI or MCP; query-service reads never submit producer work.
The [MCP README](../../crates/entmoot-mcp/README.md) owns setup and transport options.

Run agents and their explicit producers inside the host's pinned tool environment
(for example [Hermit](https://github.com/cashapp/hermit)). Tool provisioning and
environment activation are host responsibilities. Register each worktree separately
and keep publication/source fences even when all agents use identical tooling.

Start with status and scope. Use impact for revision changes (declare subjects
for cross-materialization pairs), `assess_overlay` for inventory presence across
publications, compatibility for optional Bound/Fits on fixed consumer evidence
against a candidate provider, and inspect/read for the cited occurrence and
source. A package match or call path does not establish runtime behavior, taint
or exploitability. Keep unknown and truncated frontiers visible. Do not rebind
the consumer against a candidate merely to assess it.

For explicit revision pairs, select the base/head publications instead of a
working-tree comparison. Reads do not run producers. There is no required host
telemetry, agent trace, model experiment or receipt archive.

## Optional durable producer recovery

Ordinary navigation needs no scheduler configuration. Local clients using explicit
durable producers coordinate admission by repository identity. Use
`entmoot collaborate scheduler --repository <id>` to read the policy,
active leases and the first 100 live waiting tickets. The frontend selects the
workspace repository; `--store` remains an expert override.
Write a `SchedulerPolicy` JSON file using the generated contract, then apply it with
`configure-scheduler --repository <id> --expected-generation <observed-generation>
--policy <json>`. An absent policy has generation zero. Updates use compare-and-swap;
reread after a concurrent configuration change. Defaults allow two running jobs,
one binding job and three work credits. Binding costs two credits; other jobs cost
one. Costs are operator estimates; host isolation enforces actual CPU/memory limits.

An explicit run refused for capacity retains its FIFO interest for the configured
waiting lease (one minute by default). Retry that same run before expiry to keep
its place. The read's `next_can_run` is advisory, not a reservation or automatic
dispatch. A heavy head may leave capacity idle until it fits; abandoned interest
expires. Unrequested jobs and blocked prerequisites do not enter this order.
`pause`/`cancel` releases interest; after `resume`, a new run joins at the tail.
Setting binding slots to zero disables new binding grants while allowing other
kinds past those tickets. Disabled tickets still consume waiting capacity until
expiry/control. Reducing policy limits preserves existing workers and stops new
grants until usage drains. CLI and MCP use the same policy and transaction owner.
MCP's process-local worker/queue flags additionally bound each server; controls
have separate capacity. Reads never refresh interest or run queued jobs.

For durable local inventory/canon production, capture source identity with
`entmoot collaborate source --source-root <worktree> --repository <id>`.
Build a `ProducerRequest` using the generated contract schema, then use
`submit-producer --request <json> --key <idempotency-key> --owner <agent>`.
`run-producer --id <id> --worker <worker> --source-root <worktree> --steps 1`
computes and checkpoints one batch, then pauses. Inventory and canon accept
`--files-per-step` (default 128) and `--wall-time-ms` to stop admitting steps at a
cooperative deadline. Inventory runs separate parsing, syntax-extraction and
call-stitching file frontiers; status exposes their completed counts. Stitching
uses the complete callable index and begins only after extraction finishes.
Ordinary `init` and durable inventory reuse unchanged file syntax and structural
call decisions across revisions. The shared store can reuse work from another
worktree with the same repository identity and matching source/policy. Status
reports parse, extraction and stitching reuse separately while frontiers are live;
ordinary initialization returns the same counters in its local summary. Changes
to previously missing or ambiguous targets invalidate affected call decisions.
Reused decisions receive the new snapshot scope; native evidence is not reused by
this structural cache. Cache collection affects speed, never historical results.
`resume --id <id>` followed by another slice continues at the next unfinished
frontier. Final assembly/sealing and publication each consume a step; small
inventories can finish with the CLI's default five steps in one invocation.
`status`, `pause` and `cancel` share the existing operation controls. A completed
retry returns its original publication receipt. A source change requires a fresh
request; a competing workspace update never silently rebases the frozen target.

MCP uses `collaboration_read` source observation and `collaboration_write`
submission/control/execution with explicit `--collaboration-source-root`.
The workspace repository is selected automatically; `--collaboration-store` is an
expert override. The portable request contains no host path. Source observation
itself does not create a missing store. These jobs update the shared registry;
they do not replace `.entmoot/current.json` or alter a worktree's ignore rules.
Inventory stays compiler-free; canon is an explicit structural experiment. Steps
are checkpoint boundaries, not hard elapsed-time limits. An interrupted file batch
can repeat; completed file outcomes survive restart and collection. Canon carries
its cumulative analysis limits across slices; those limits remain explicit in
the resulting claims. Partial canon folds are private until publication. Shared
inventory stitching checkpoints completed file parts. Source verification,
configuration inventory and final graph validation/assembly remain whole units;
further producer recovery remains in
[PLANNING](../../PLANNING.md). Network collaboration remains out of scope.

Durable inventory is an explicit recovery workflow. The local 192-file probe
measured 0.574s ordinary / 0.977s durable inventory and registration, or 1.844s with
a reopening after each of 20 steps. Ordinary/durable outliers reached 2.36/2.79s.
Main database size grew 54% before collecting unreferenced payloads. These costs
exceed the ordinary 25% allowance; use larger slices when frequent interruption
is unnecessary. The separate 192/1,024-file CLI initialization comparison stayed
within 3–19% median overhead, including no-change and edited-source runs. These
fixtures do not qualify compilation, memory or large-repository costs.

For durable native binding, register an inventory view and submit a binding
`ProducerRequest` with explicit families and the host's environment digest. The
host must maintain its pinned tool/dependency/configuration environment and an
isolated, quiescent source root for the job. Matching a supplied digest checks the
host's assertion; Entmoot does not independently inventory all external or absent
inputs. Configure exclusive `ENTMOOT_PRODUCER` to a matching producer executable,
or place it beside the CLI/MCP executable. Build it with
`cargo build -p entmoot --features producer`; nothing installs during execution.
Adapter setup remains in [resolver adapters](resolver-adapters.md).

```text
entmoot collaborate --store <db> submit-producer --request binding.json --key <key> --owner <agent>
entmoot collaborate --store <db> run-binding --id <id> --worker <worker> --environment <digest> --source-root <worktree> --steps 1 --timeout-seconds 3600
entmoot collaborate --store <db> outputs --id <id>
entmoot collaborate --store <db> resume --id <id>
```

Each step runs one family and atomically saves its graph and receipt; one final
step publishes the saved prefix. `--wall-time-ms` stops admission between steps.
Each family has a 1..3600 second execution allowance (plus bounded helper cleanup);
the default native cardinality bounds still apply. A failed/budget-limited receipt
pauses without advancing the family. Restore the same pinned environment or
increase the time allowance before resuming; changed inputs need a new request.
An incomplete attempt retains useful facts without establishing absence. At most
1,000 attempts are retained per operation. Completed families do not repeat,
including after the helper is unavailable at final publication. Current-family
work can repeat after interruption; typed canon and within-family recovery remain
open. These jobs use private `.tmp/entmoot-binding/` attempt directories and
private Cargo/Go caches. Other native build isolation belongs to the host.

Lease heartbeats keep long-running families alive without changing their cursor.
`pause`/`cancel` revokes execution and stops its controlled process tree; after a
crash use `pause` then `resume` to avoid waiting for the one-minute lease. Explicit termination uses Windows Job Objects or Unix process groups. The host
must clean up a killed parent's job or tools escaping the controlled process group.
Unpublished prefixes stay private and pinned; terminal jobs release their pins
for ordinary collection while retaining attempt reports. MCP uses the same
`run_binding` request and requires its configured collaboration source root.

The explicit process-fault control requires an installed Node host, and the cost
probe compares 32-file ordinary binding, one durable run and resumed execution:

```text
cargo test -p entmoot --features producer --test binding_operations_cli pause_and_cancel_stop_native_descendants -- --ignored --nocapture
cargo test -p entmoot --features producer --test binding_operations_cli binding_restart_cost_probe -- --ignored --nocapture
```

The alternating Windows probe measured median 91ms ordinary, 376ms durable and
456ms resumed execution, with a 1.51s durable outlier. Database growth was 1.20MB
versus 1.31MB (+9%). The additional median 285–365ms is an explicit opt-in cost for
saved native-family work and failed-attempt attribution; it exceeds 25% on this
small lexical workload. This probe does not qualify large-repository or build
costs. Logs and build roots remain local scratch, not required retained evidence.

For durable conditional revalidation, register base/head graphs with their already
attached artifacts, then supply a `RevalidationRequest` with explicit producer
input policies. The generated schema owns its fields. Use:

```text
entmoot collaborate --store <db> submit-revalidation --request <json> --key <key> --owner <agent>
entmoot collaborate --store <db> run-revalidation --id <id> --worker <worker> --steps 1 --max-records 100000
entmoot collaborate --store <db> resume --id <id>
entmoot collaborate --store <db> outputs --id <id>
```

Repeat execution after resume. Each step commits one whole policy; one final
step publishes the ledger. `--max-records` is cumulative across slices and may
increase up to one million. It counts selected artifact evidence and applicability,
excluding unrelated structural inventory. `--wall-time-ms` stops admission between policies,
not midway through one. Paused outputs are committed policy receipts, not a
completed ledger. `revalidation-record --id <id> --collection <collection>
--record-id <record>` reads the completed ledger: `visible: null` is unreported,
`false` is retained inactive evidence/applicability. Collected ledgers are
unavailable. Cancellation discards only the private ledger and retains receipts.

After a crashed worker, `pause` revokes its generation, then `resume` permits a
new slice without waiting for its one-hour lease to expire. The current policy
may repeat; saved policies do not. Inputs remain pinned even if workspaces move
or retire. A changed implementation requires a new request. MCP exposes the
same operations without requiring `--collaboration-source-root`: this worker
uses retained views and never invokes a producer. Historical evidence and
conditional reuse do not establish a fresh head observation or a verified proof.
