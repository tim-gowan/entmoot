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

These are selection and acceptance rules for extending the existing fixtures and
campaign harness, not a claim that historical CI/CVE campaigns are already qualified.

1. Pin the upstream repository and full commit IDs, with issue/PR/CI or advisory
   attribution. Name the selected baseline and head explicitly. For a merge, select
   its parent or comparison window deliberately; record additional parents when
   testing integration behavior. A temporary hosting-service merge ref is not a
   stable fixture. Linear and squash histories are valid when their scope is clear.
2. Reproduce the relevant failing and fixed regression controls under identified
   tools, locks, features and environment. Separate code behavior from infrastructure
   failures and flakiness. An unreproduced report remains candidate evidence, not
   ground truth. A CVE label, red build or accepted patch alone supplies no oracle.
3. Author Entmoot expectations independently: changed/stable occurrences and hash
   relationships, expected bindings/edges, represented caller witnesses and the
   minimum required tests. Include unaffected and deliberately unsupported controls.
   Define which evaluation each case qualifies; do not require all languages and
   evaluation types to pass one universal matrix or infer behavioral equivalence
   from structural identity. Consume pages or explicitly report unconsumed bounds.
4. Keep external runtime/exploit evidence separate from structural assertions. A fix
   can alter a guard while preserving the call route; a missing route can mean an
   unsupported construct. Reach tests must preserve those outcomes, source scope
   and provider attribution, never convert them into a vulnerability/fix verdict.
   Any executable regression control uses the host's isolated test environment.
5. Reduce each useful failure to a deterministic fixture in the owning Rust crate,
   while retaining selected complete repository cases for composition and scale.
   Reuse existing manifests, request files and harness storage; avoid a parallel
   evidence ledger. Measure each case before admitting it to recurring CI. Keep
   expensive corpus/native cases explicitly selected, report unavailable/partial
   work, and use existing interruption/restart controls where they actually apply.

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
