# Entmoot MCP transport

Operator entry: [README](https://github.com/tim-gowan/entmoot#readme). Meaning:
[ARCHITECTURE](https://github.com/tim-gowan/entmoot/blob/main/docs/architecture/ARCHITECTURE.md).
Tool input schemas: `entmoot-mcp --tools-schema` (and generated docs)—do not
hand-copy them here. Procedures below.

Call results have independent concrete-edge and unresolved-attempt bounds.
Follow `unresolved_continuation` through `inspect` (or as the request member of
`query` with operation `inspect`). Expanded results continue after the displayed
attempts; compact call results defer all attempts and therefore start at the
first one. Finish each compact output page before advancing the semantic query.
An exhausted edge cursor does not exhaust unresolved attempts or prove closure.
CLI users can submit the same envelope with `entmoot request --request query.json`;
`entmoot request --example unresolved` prints a complete starting request.

Build `cargo build -p entmoot --features producer -p entmoot-mcp`.
Cursor local wiring: [`.agents/mcp.json`](https://github.com/tim-gowan/entmoot/blob/main/.agents/mcp.json) (symlink `.cursor` → `.agents`) via
[`scripts/run_entmoot_mcp.py`](https://github.com/tim-gowan/entmoot/blob/main/scripts/run_entmoot_mcp.py); usage guidance lives in the [agent runtime runbook](https://github.com/tim-gowan/entmoot/blob/main/docs/runbooks/agent-runtimes.md).
CLI init/bind channels: stdout holds the JSON workspace receipt; stderr holds
tracing stage progress and an always-on operator summary (not gated by
`RUST_LOG`). Default filter `warn,entmoot=info,entmoot_engine=info`; raise
solver noise with e.g. `RUST_LOG=warn,entmoot_engine=info,ra_ap_=debug`. Incomplete
`entmoot bind` requires native by default; use `--structural-only` to waive.
Boot prints adapter-profile awareness from the home-scoped store; pass
`--require-adapter-config` to fail closed on hard gaps. Start `entmoot-mcp -C <source>` without a publication. Contract tools read the snapshot from `init`. Publication tools need `bind`.
Storage defaults to `~/.entmoot` (`--data-dir` / `ENTMOOT_DATA_DIR` override).
`entmoot init` captures the contract snapshot. `entmoot bind` publishes native evidence.
Startup discovers the home-scoped `current.json`; bare MCP `impact` compares that
baseline with a freshly indexed, twice-verified working tree. The same
working-tree path applies when an advanced `--publication` selection has
`base: null`.
An immutable pair is used only when `--compare-graph` is set
or the publication pointer names a non-null `base`. Cross-publication compares
need declared snapshot subjects (or CLI correspondence), not folder
`repository_id` equality alone; `assess_overlay` reports inventory presence
without Bound/Fits. Missing inventory is prepared automatically. Damaged or incompatible existing
publications fail with recovery guidance. For an explicit immutable pair:

~~~text
entmoot-mcp --graph <head>/graph.json --store <head>/knowledge.sqlite3 --compare-graph <base>/graph.json
~~~

`--publication <current.json>` explicitly selects that pointer's immutable base/head pair.
`--source-root <checkout>` enables bounded digest-verified source reads.

Local collaboration uses the workspace repository automatically;
`--collaboration-store <db>` is an advanced override. Explicit
inventory/canon execution and digest-only source observation use
`--collaboration-source-root <worktree>`; source paths never come from portable
requests. `collaboration_write` submits, controls and runs durable operations;
`collaboration_read` observes status and published states. See the
[agent runbook](https://github.com/tim-gowan/entmoot/blob/main/docs/runbooks/agent-runtimes.md) for prepared-result recovery.
These operations do not add a network collaboration layer.
`--operation-workers` (default 2, range 1–64) and `--operation-queue` (default 64,
range 0–4096) bound this server's active and waiting collaboration writes. Controls
and scheduler configuration use a separate bounded lane. The shared repository's
persisted scheduler policy also applies across server/CLI processes; use
`collaboration_read` with `operation: "scheduler"` and an explicit
`configure_scheduler` write as described in the runbook. FIFO tickets require
explicit run retries; no background dispatcher is started.
Ordinary operation manages storage internally. Advanced explicit graph layouts
retain `--store` overrides.
`--ready-timeout-seconds` (default 180) bounds how long each `tools/call` waits for
publication open.

`--read-workers` (default 4, range 1–256) bounds active blocking reads;
`--read-queue` (default 64, range 0–4096) bounds waiting reads. Excess work returns
`kind: unavailable` with a retry message. Read tools run through this executor,
including warming waits and working-tree impact. Protocol tasks stay responsive.
Cancelling a running request cannot stop its blocking computation, so capacity
is retained until it completes. These limits apply per server process; deploying
one server per agent still duplicates publication memory.

**Deferred open (stdio contract):** the process answers `initialize` / `tools/list`
before loading an existing graph/store/catalog. First-use structural preparation
currently precedes the handshake; `entmoot init` can prepare large sources first. Shared engine
[`open_query_bundle`](https://github.com/tim-gowan/entmoot/blob/main/crates/entmoot-engine/src/workspace/open.rs) runs on a worker
thread (same path CLI reads use synchronously). Bounded read workers wait until ready
or return portable `kind: unavailable` with `publication_warming:` /
`publication_open_failed:`. After first-use preparation, stdio clients can list tools immediately;
calls wait on the same gate. `tools/list` advertises **input** schemas only—
omitting `outputSchema` keeps the list under Cursor’s practical size limit
(hosts that exceed it show Connected with 0 tools). Call results still use
typed structured content; full output shapes live in contracts / generated
JSON Schema.

The server uses shared engine operations directly, with the same
portable requests, receipts, budgets, evidence filters, and unknown outcomes as
the CLI. Read tools never start native producers; explicitly invoked
`collaboration_write` may execute configured operations. When configured with a publication pointer,
it monitors that pointer and warms a validated replacement before swapping engines.
In-flight requests keep their original generation. Warming and refresh failures
return explicit unavailable outcomes; explicit graph paths stay fixed.

## Shared loopback backend

Start one backend explicitly, with a randomly generated bearer token of at least
32 characters in `ENTMOOT_QUERY_TOKEN` (or a local `--token-file`):

~~~text
entmoot-mcp --listen 127.0.0.1:7341
entmoot --backend http://127.0.0.1:7341/mcp status
entmoot --backend http://127.0.0.1:7341/mcp request --request query.json
~~~

CLI also accepts `ENTMOOT_QUERY_BACKEND` and `--backend-token-file`; `--standalone`
overrides the environment. Connections never start a server. HTTP binds only to
loopback, authenticates every request, rejects browser Origin requests and shares
the existing bounded read executor. A remote read uses the server's publication;
local graph/store overrides require standalone execution. Expert store queries
and local composition commands remain standalone. Existing stdio startup is
unchanged when `--listen` is absent.

`--no-contraction` enables the original-adjacency reference mode. Shortcut retention
defaults to 64 MiB and sixteen predicate selectors; `--shortcut-cache-bytes`,
`--shortcut-cache-entries` and `--max-index-edges` configure it. Construction limits
include repeated edge inspections for SCC and chain analysis. Oversized, unfinished
or unavailable overlays fall back without changing relationship meaning.

Opt into resumable path/neighborhood requests with `resumable: true`. Pass the
returned `search_receipt.continuation` as the same request's `continuation`.
Expansion, examined-edge and result budgets may change between slices; hop,
predicate, scope, endpoint, fanout and evidence selection stay fixed. A slice's
`slice_token` supports exact retry with its original slice budget. Neighborhood
chunks return newly discovered endpoints; path chunks wait for stable candidate
ordering. Cumulative counters distinguish execution from replay and output paging.

The backend defaults to 64 searches, 64 MiB aggregate retained state, 16 MiB per
search, five minutes idle expiry and two available generations. Configure these
with `--continuation-capacity`, `--continuation-bytes`,
`--continuation-search-bytes`, `--continuation-idle-seconds` and
`--continuation-generations` (one or two). Retention includes replay results.
Memory exhaustion, expiry, retirement or restart returns actionable unavailable;
the engine never silently restarts a continuation. Authoritative knowledge survives.

## Information delivery (CLI and MCP)

Navigation defaults to **compact**: a bounded page of readable subjects, directed
edge references, path classifications and original role summaries. Exact snapshot,
scopes, provider attribution, attachment outcomes, unresolved counts and gaps stay
visible. Full evidence remains addressable; deferring it never strengthens a claim.

Use **explain** to examine selected evidence, or **export** for consumers that need
the original bounded result. Traversal detail limits are applied before per-hop
arrays are built; every applicable claim still participates in classification.
Export does not enumerate all possible paths or remove search/selection budgets.
No tier runs a producer, changes evidence, or establishes dataflow or exploitability.

The typed owners are [`OutputOptions` and `QueryResponse`](../entmoot-contracts/src/query/output.rs),
with rustdoc examples and the [generated query schema](../entmoot-contracts/generated/entmoot_query_contracts.schema.json).
MCP `tools/list` generates **input** schemas from these types (not output
schemas—see deferred-open note above). Compact uses `items` with page-local
subject/edge/scope dictionaries. Explain/export wrap the original query under
`result`; seeded explain/export wrap it under `report`.

CLI `request`, `neighborhood`, and `trace-seeded-paths` accept `--detail compact|explain|export`,
`--output-limit`, `--output-max-bytes`, and `--output-after <cursor.json>`.
Explicit flags override the request's `output` fields. MCP navigation tools accept
`output`; the passthrough `query` uses `request.output` inside its `QueryRequest`.
For example, add `"output":{"tier":"explain"}` to a task tool's arguments, or
use `entmoot neighborhood <node-id> --detail explain`. Report consumers should request
`entmoot trace-seeded-paths --graph <graph.json> --detail export`.

Compact defaults to ten items and 16 KiB; explain to 64 KiB; export to 64 MiB.
The byte ceiling measures each serialized JSON response, excluding transport
framing and the CLI newline; a batch has a separate response for each query.
Whole answer items are paged, never silently cut. An envelope or first item that
cannot fit returns `invalid_request`; narrow the query or explicitly raise the
byte budget. These are delivery controls, independent of search budgets.

Interpret the three limits separately:

- `search_truncated` reports an incomplete traversal search under its algorithm;
  `result_bounds` retains the underlying operation's own bounds. Neither establishes
  graph closure, even when false.
- `page.next_cursor` continues delivery within that bounded result. Pass it unchanged
  as `output.after` with the same semantic request. Snapshot, publication contents
  and knowledge visibility changes reject continuation. For resumable queries the
  cursor pins an already executed slice; page it before requesting the next search
  slice. A retained prior generation remains available until retirement or expiry.
- `details_deferred` means richer records are available via inspect/explain/export.
  It does not mean the traversal stopped early. Receipt-specific omission flags
  and unresolved/role selection bounds remain explicit.

`status`, revision `impact`, `compatibility`, expert storage queries and effectful
command receipts retain their dedicated contracts; these navigation tiers do not
silently reshape them. CLI and MCP execute the same delivery operation, including
byte limits, cursor validation and typed failures.

`relay_impact` has a separate compact `ImpactRelayRequest` / `ImpactRelayResult`
contract shared with CLI `relay-impact --graph <head> --compare-graph <base>`.
It selects changed callables, walks head callers through the existing neighborhood
operation, and reports removed callables as explicit missing-head gaps. It requires
an immutable pair and never indexes a working tree or runs producers.

`output` pages changed subjects (maximum sixteen caller searches per request);
`callers_output` controls compact delivery for each subject. The outer cursor
resumes through `request.output.after`. Each caller page contains an ordinary
`query`; resume it through the query tool with that page's `output.after`.
`budget` applies per caller search, independently of byte limits. Default
`resolved_only` excludes candidates; `any` retains all represented evidence.
Frontiers are not closed coverage. Narrow oversized compact pages; request
single-subject explain/export with explicit `max_bytes` for large evidence.

`compatibility` uses the same `CompatibilityRequest` and `CompatibilityResult`
as CLI `assess compatibility`. Configure repeated `--provider <graph|current.json|checkout>` inputs,
or let startup discover at most 256 direct directories under the workspace storage volume's `deps/` directory.
Each may contain `logic.graph.v1` or a local `current.json`; discovered graph
and store paths must remain inside that dependency directory. `{}` assesses all
configured providers; `{"request": ...}` selects their publication digests.
Requests cannot supply paths or start producers. Missing resolver objects,
incomplete export scopes, and missing expected boundaries stay
`incomplete_bind`. An export absent from a producer-stated **complete** surface
is `broken_at` only when the consumer recorded an assessed expected boundary
and the surface enumerated that exact symbol-scheme/boundary-policy universe;
legacy/unassessed bindings never treat absence as a break. See the
[compatibility runbook](https://github.com/tim-gowan/entmoot/blob/main/docs/runbooks/compatibility.md).
Explicit graphs use the adjacent default object store. Explicit and discovered
publication pointers retain their configured store paths. CLI `assess compatibility`
uses the same discovery when providers are omitted, and defaults its consumer
to the current workspace publication. No read invokes an adapter or fetches source.

Read tools: `status`, `find`, `inspect`, `read_source`, `query`, `traverse`,
`callers`, `callees`, `neighborhood`, `trace_seeded_paths`, `impact`, `relay_impact`, and `compatibility`.
MCP `tools/list` supplies their generated input schemas. `query`
accepts `{ "request": <QueryRequest> }` wrapping the same QueryRequest used by
CLI `request`. Default `impact` computes the same working-tree receipt as CLI
without moving the pointer; explicit publication or graph startup arguments
compare the configured base/head. Both return the same breaking-change signal
facet. Signals are consumer-review hints, not SemVer or vulnerability-fixed
verdicts. `neighborhood` exposes direction, evidence filters, optional
`endpoint_role` and walk budgets. It is the same operation as CLI `neighborhood`;
source/sink endpoint selection retains original assertions and does not infer
value flow. Hypothesis inclusion and candidate-edge inclusion are independent.
See the [rooted workflow recipes](https://github.com/tim-gowan/entmoot/blob/main/docs/runbooks/impact.md).

Indexing, binding, artifact attachment, revalidation, planning, and durable work
remain CLI composition-root operations. Use `entmoot init`, `entmoot bind`,
`entmoot attach`, `impact [--revalidate]`, and `bind --from-impact` for operator
effects; MCP reads never enqueue producers or mutate publications.

Parity controls live in `cargo test -p entmoot-mcp --test stdio_e2e --test http_e2e` and the
CLI `artifact_cli` and `workspace_defaults` suites. They exercise shared reads,
unknown/invalid requests, publication discovery and read purity. Query timing is
excluded from equality. Repository runs use the [SQLite harness](https://github.com/tim-gowan/entmoot/blob/main/evals/harness/README.md).

`inspect` also accepts `kind: "unresolved"`, an exact subject `id`, and a bounded
`limit`. In explain/export, pass `result.next_cursor` unchanged as the inspect
request's `after` to continue original attempt details independently of the
neighborhood cap. This collection cursor is distinct from compact delivery's
`page.next_cursor` / `output.after`. A changed snapshot,
subject, publication or collection invalidates continuation; restart from the
first page. Exhausting stored attempts does not establish dispatch completeness.
CLI uses the same payload through `entmoot request`.

Python Program bind is a CLI operation. MCP `compatibility` reads its native
package evidence through the same service and receipt schema. The live
source-derived ecosystem controls and pinned real dependencies verify exact
CLI/MCP receipt equality; setup,
scope and residuals are in the [compatibility runbook](https://github.com/tim-gowan/entmoot/blob/main/docs/runbooks/compatibility.md).

## Boundary queries and generated interfaces

`boundaries` accepts the shared CallBoundaryRequest and output options. Example:
`{"request":{"callee":"*.toString()","outcome":"unsupported"},"output":{"tier":"export"}}`.
Distinct-site totals, provider/build-context evaluations, source spans/digests and
explicit missing-detail limits remain available. Source syntax does not imply
native type meaning. Repository, dependency and native-library completeness are
separate; none is inferred from provider completion.

`entmoot-mcp --tools-schema` prints generated tools/input schemas without opening
storage. Rust contracts own output shape. After publication, install with
`cargo install entmoot-mcp --version <published-version> --locked` after publication.
