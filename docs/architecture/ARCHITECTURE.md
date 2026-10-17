# Entmoot architecture

Entmoot is a repository knowledge substrate: deterministic code inventory,
attributable observations and bounded revision impact. Consumers make judgments;
Entmoot reports evidence, correspondence and limits. Current work is in
[PLANNING](../../PLANNING.md).

## Identity and meaning

A definition is implementation content under a structural policy. An occurrence
is one declaration site, addressed by repository, language, path, scope and name
(with spans for collisions). Equal bodies do not merge different occurrences.
Structural hashes do not recursively include callee bodies. A signature boundary
is separate from a body. Exact source digests verify attachment, not semantic
equivalence. Paths are repository-relative POSIX; digests use BLAKE3.

Resolved identity belongs to an occurrence and its attributable binding evidence.
Unknown, incomplete and conflicting bindings stay explicit. Native enrichment
does not change a structural hash or logic address. A move may appear as removal
and introduction unless an explicit correspondence supports a stronger result.

Publication identity for compares is a declared **subject** (package PURL
coordinate or repository id with optional path) plus optional aliases—not the
materialization folder basename in `repository_id`. Impact may join callables on
subject-relative occurrence keys (`lang|path|scope|symbol|kind`) under a
`CorrespondencePolicy` without rewriting inventorial `logic_address` hashes
(series stay v1). Cross-subject inventory presence uses the separate
`inventory_overlay` assessment; `package_api` Bound/Fits remains an optional
stronger rung. Hard `RepositoryMismatch` applies only when subjects differ and
no correspondence policy is supplied.

A proposition describes a relationship; a relation edge places it in scope; a
claim records a producer's assertion about it. Status and confidence are distinct.
Scope keys retain field names and optional presence; composite addresses frame
their components before hashing. Identity is not correspondence: delimiters in
producer strings cannot merge targets or remove distinct candidates. These
derivation changes remain in the v1 series and invalidate older fingerprints;
use a matching historical reader or explicitly rebuild, never relabel a store.
Explicit `contradicts` links between active claims surface both applicable edges
as conflicted without choosing a target, rewriting endpoints or claiming closure.
Absent or inactive linked claims do not establish active disagreement.
No missing edge establishes falsehood without relevant completeness evidence.
Finding locations and ordered provider traces keep their supplied meanings;
neither becomes a source/sink or exploit claim by position alone.

## Ownership

The structural pass owns files, declarations, syntax sites, source spans, literal
imports/exports, structural hashes and logic addresses. Bounded local binding
requires a unique target justified by language-aware scope. This is the retained
ADR-0025 decision; it does not require an all-language audit or contribution
campaign for each change.

Native adapters own type-directed dispatch, overload selection, macro expansion
and build/environment-dependent facts. Attachment checks source identity and
occurrence coordinates. Unavailable, stale, partial and incomparable results are
limitations, not negative conclusions. Parsing alone does not qualify compiler
acceptance. C++ remains experimental.

The workspace has five crates:

- `entmoot-contracts`: typed portable payloads, validation, generated JSON Schema.
- `entmoot-core`: Tree-sitter inventory and structural identity, without compilers.
- `entmoot-engine`: sources, adapters, joins, storage, bounded reads and impact.
- `entmoot` (directory `entmoot-cli`): operator composition and durable mutations.
- `entmoot-mcp`: shared query transport and explicitly invoked collaboration writes.

Contracts contain no compiler, scanner, database or agent implementation types.
Adapters translate external evidence; they do not create structural identities.
CLI init/bind may use configured producers but never install tools or run scanners.
Query-service reads do not schedule native producers or mutate knowledge.
Frontends may prepare a missing compiler-free structural publication on first use;
existing views are reused until explicit init/bind. `-C` selects the source root.
Optional `--data-dir` / `ENTMOOT_DATA_DIR` maps canonical roots to isolated local
namespaces. Database paths do not enter portable queries. Remote locations must
be rejected. Shared checked location helpers select the volume for publication,
native objects, profiles, locks and retention; malformed configuration cannot
redirect a read or write to a default volume. A future transport
must preserve authentication, atomic publication, source verification, history
and producer isolation.

Call boundaries expose source spelling/spans and distinct-site counts alongside
attributed native observations. Native types are never inferred from callee text.
Unknown contexts and corrupt evidence fail; unretained detail remains a limitation
even when a filter returns no sites. Repository, dependency and native-library
completeness are independent universes. None is established without exhaustive
evidence for the relevant dispatch, implicit calls and external reentry. Future
boundary claims attach to immutable source and artifact identities.

The derivation fence composes package-local contracts/core/engine fingerprints.
Each commits production sources/build support and generated normalized manifest
and lock inputs; core verifies bundled grammar commitments. Checkout and extracted
packages agree. `scripts/prepare_release_inputs.py --check` rejects stale release
inputs; supported installs use the committed lockfile. Alpha fingerprint changes
require matching historical readers or explicit rebuilds, not forced migration.

## Declared code contracts

Inventory attaches structured callable parameters/results and direct object
members to real occurrences. Types remain language-specific written expressions;
unspecified and unsupported forms are distinct. Defaults are unevaluated syntax.
Modifiers, source spans and extraction limits remain visible. No argument-to-
parameter mapping, inherited shape closure or compiler type equivalence is implied.

These objects persist with inventory nodes in SQLite. CLI/MCP `inspect` exposes
contracts in explain/export detail; impact reports declared changes separately
from body and binding dimensions. Missing observations remain not-comparable.

## Canon and clone operations

Inventory remains compiler-free. `bind --experimental canon` is one explicit
post-inventory operation: collect native facts for the selected project, fence
their source bytes, prepare callable/binding indexes, then fold each bounded body.
Native tools resolve at declaration and expression spans, within a project or
compilation unit; they do not provide a replacement structural AST or hash.
Those dependencies prevent collapsing inventory and native preparation into an
unqualified single visitor. The emission fold creates terms and their BLAKE3
digests together, followed by a hashed envelope of assumptions and definedness
ingredients. There is no SMT invocation or equivalence proof.

The portable kernel owns term interning, local SSA values, branch merging,
budgets and hash framing across languages. Syntax adapters retain language
semantics. Shared traversal/source access lives above the ECMAScript visitor;
that visitor owns only JS/TS conventions. Syntax, alpha and literal-parameterized
clone fingerprints share one bottom-up traversal and cached statement summaries.
The alpha kernel adapts [compositional alpha hashing](https://arxiv.org/abs/2105.02856):
binding declarations and uses have separate position summaries; free entries keep
their names until the extent includes their declaration. Small-to-large merges,
height-tagged positions and a persistent balanced map share unchanged branches.
Only small-map entries are updated; unique-key entry digests form a commutative
aggregate inside a framed BLAKE3 envelope. This independently tested incidence
encoding is not the paper's lambda encoding or a formal equivalence proof.
Rust lowering resolves sequential shadowing and nested blocks, including initializer
visibility and scope exit. Other families retain the conservative simple-binding
lowering. Closures, complex patterns and unlowered declarations remain unavailable.
Statement captures retain spelling and callable-relative declaration order; this
can conservatively separate moved windows. Root-free names are never erased.
Parameterized literals preserve repeated-value partitions and written token kinds.
Parameterization remains a weaker
review signal because erasing a literal can erase logic; it is not canon identity.
Alpha retains caller-bindable Python and C# parameter names. Known name-observing
constructs (including debug f-strings and `nameof`) leave alpha and parameterized
unavailable through the existing optional hashes and omission count. This is a
conservative syntax boundary, not an exhaustive reflection or framework model.
Canon's separate positional-input assumption remains explicit; alpha availability
does not qualify its scalar model.

The first canon subset models primitive booleans and qualified integer operations.
Types, signedness, width, supported overflow modes, branch order, literals and
written result boundaries persist. Dynamic callers must satisfy exact primitive
input assumptions. Conversions, floats, loops, async/decorators, heap effects and
unqualified calls stay partial. Checked/undefined arithmetic needs future
definedness lowering; pure-call abstractions need actual purity evidence.
Opaque source commitments and explicit limits participate in form identity;
partial forms cannot enter canonical clone groups. Equal canon hashes are
model-relative candidates, never a statement of whole-program equivalence.

Canon file shards and their graph manifest use existing typed relation claims.
Occurrence-to-form membership edges avoid quadratic pairwise clone edges.
Publication uses the existing atomic knowledge transaction and producer scope.
Unchanged pure folds can be reused after exact source, contract, policy and fresh
native-fact checks; native collection itself is not cached by structural equality.
Reads consume bounded, compatible snapshot shards from KnowledgeRepository and
do not parse source or enqueue producers. Missing or incompatible canon yields an
explicit availability result and the bind command as a nudge.

## Storage and revisions

KnowledgeRepository is the source of record. SQLite shares immutable payloads
while owners, dependencies and visibility are scoped to a view. Transactions
publish complete owner replacements; one producer cannot erase another's rows.
Readers observe a coherent publication. Reingestion is idempotent. Inactive
historical payloads can remain available without implying current applicability.
Hydration reconstructs shared metadata and verifies the canonical payload digest
before typed decoding. Typed readers consume that verified JSON tree directly;
byte-based membership checks retain the same canonical representation.

Workspace publications name source snapshots and supporting inputs. Retention
keeps current/base and pinned roots subject to configured limits; active leases
protect open stores. Topology JSON and indexes are rebuildable projections.
Storage fingerprints reject incompatible readers rather than silently resetting
data. Entmoot-owned format labels remain v1 until the repository move; labels
alone do not establish compatibility.

Revision impact compares materialized snapshots, distinguishing bodies,
boundaries and observed binding changes. Bounded caller/dependency traversal
describes potentially affected subjects. It is not proof of runtime behavior or
a breaking API verdict. Applicability, historical retention and durable
invalidation are separate. Automatic revalidation is bounded by the implemented
producer's input declaration; unqualified families require explicit refresh.

## Evidence and cost

Read engines belong to immutable publication generations. Dense address IDs and
deterministic original-edge postings preserve inventory occurrences, ambiguous
attachments and external endpoints. Scope states, transitions, SCC traversal and
union-find are shared query primitives; evidence-bearing parallel edges remain
distinct. Assessment connectivity may deduplicate endpoint pairs without changing
the authoritative relationships.

Endpoint path search uses weighted bidirectional queues. A bounded, lazy selector
cache contracts only directed degree-one chains outside cycles, ambiguous joins,
scope changes and observation bridges. Chain offsets preserve interior endpoints;
weight is the original hop count. Complete weak components can reject disconnected
endpoints. Failed or oversized index construction falls back to original adjacency.
Neighborhoods, direct callers/callees, components and unused assessments use original
edges. Witnesses reconstruct those edges before current claim classification.
Shortcuts are never published relations, inherited claims or direct calls.

Resumable traversal retains exact frontier positions, scope states and witness
references in bounded memory. Execution slices and output pages have separate
cursors. Retries replay a retained slice; concurrent advancement is refused.
Request, publication and evidence fences reject incompatible continuations.
Expiry or eviction requires an explicit new search. Explicit durable continuations
checkpoint frontier, scope, witnesses and replay slices in KnowledgeRepository;
they survive backend restart within the configured idle lifetime. Checkpoint
publication uses generation/sequence compare-and-swap and checksums. Default
in-memory continuations remain disposable across restart. Limits describe
observed work and results, never unseen closure. Index inspections, search work,
original-edge hydration and output serialization remain separately measurable.

One explicitly started MCP backend can serve local CLI and MCP clients through
authenticated loopback HTTP; stdio and standalone CLI remain supported. A pointer
monitor validates and warms replacements before swapping the engine. In-flight
reads retain their generation, and a bounded prior generation serves continuations.
Refresh failure and warming are explicit. Reads never start producers.

## Local collaborative states

One shared knowledge store can register several workspaces in the same repository.
Each registration names an immutable view and source snapshot, a publisher and a
generation. Branch labels are mutable metadata. Registration and retirement use
compare-and-swap; retirement preserves the generation fence and releases its view
pin. Captured peer lists remain fixed while branch references advance. Registered
views are sealed against inventory, visibility and ownership mutations; publish a
new view for new evidence. Read hydration uses one SQLite snapshot.

Local operation records are operational metadata, not evidence. Submission is
idempotent by repository/key with exact-input collision checks; prerequisite
operations must already exist, making cycles impossible to introduce. Claims use
expiring leases and increasing worker generations. Checkpoints, terminal receipts
and pin release are transactional. Worker handoff also checks its generation.
Repository-wide admission defaults to two active operations, one binding operation
and three logical work credits (binding costs two, other kinds one). A persisted,
generation-checked policy configures slots, per-kind costs and bounded waiting
interest. Credits are operator estimates, not measured CPU/memory or OS limits.
Policy changes do not revoke running workers; reductions defer new admissions
until usage drains. Distinct repositories have independent admission budgets.

An eligible explicit run attempt obtains a leased FIFO ticket in a short SQLite
immediate transaction. A capacity refusal commits only waiting interest, leaving
the operation generation, progress and evidence unchanged. Retries refresh the
ticket's lifetime without changing its position. The oldest enabled ticket must
fit before later tickets can run, so a heavy job cannot be starved by smaller
newcomers; some capacity may sit idle while that head waits. This follows the
weighted fairness tradeoff documented by [Tokio's semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html).
Unrequested or prerequisite-blocked jobs never obtain tickets. Pause/cancel and
other successful state transitions remove interest atomically; abandoned tickets
expire. Disabled binding tickets are skipped for ordering but still count toward
the bounded waiting capacity. No background dispatcher or transport is implied:
the requesting client explicitly retries, and an expired ticket loses its place.
Scheduler reads report a bounded coherent observation without enqueuing work,
renewing leases or pruning tickets. No transaction sleeps waiting for capacity.

MCP reserves separate read and configurable operation capacity plus a bounded
control lane, so policy updates and pause/cancel/resume admission cannot wait
behind occupied producer slots. Process-local limits and shared repository limits
both apply. Running a conflict slice never holds a write transaction during graph
comparison. Failed external effects cannot be assumed exactly once.

Inventory and structural canon operations freeze a complete source-manifest
digest and a target workspace generation at submission. Local source roots come
from CLI/server composition. Computation runs outside a database transaction;
the complete result, immutable seal, retention pin and prepared checkpoint commit
together. Prepared views are excluded from collaborative reads and operation
inputs until publication. Registry advancement and operation completion then
commit together after another source check. Restarted workers recover the sealed
result with a new lease generation. Pause/cancel revokes stale workers; failed CAS
preserves the previous registry and the prepared result. Inventory freezes the
shared walk's file manifest and checkpoints parsed/skipped file outcomes in bounded
batches. Canon derives its file manifest from the pinned inventory and checkpoints
file claims with cumulative pattern/byte limits; pausing never resets those limits.
Both producers use one private file-checkpoint owner for membership, worker fences,
cursor transitions and cleanup. The next cursor, outcomes and operation fence commit together;
retention marks these payloads until preparation or termination releases them.
Source bytes are checked against each selected input; a final full path/digest
check prevents new or removed inputs from hiding behind cached records. Inventory
then checkpoints file-local syntax parts and bounded call-stitching frontiers.
Stitching starts only after the complete syntax inventory is available. A disposable
callable index is shared across slices within a run; restarting rebuilds that index
but retains completed extraction and call parts. Ordinary and durable paths share
the same extractor and structural resolver. This adopts the file-owned construction
boundary illustrated by [stack graphs](https://docs.rs/stack-graphs/latest/stack_graphs/),
while retaining Entmoot's conservative structural resolution and native qualification.
It does not introduce stack-graph language rules. Ordinary initialization and durable
inventory also share a disposable latest-only cache of source-qualified syntax
parts and scope-free call decisions. Repository identity, relative path, exact source
digest and policies qualify each file; implementation/layout fingerprints fence
derivation changes. Host roots do not prevent reuse across worktrees in the same
repository. Structural decisions record the exact callable-index reads, including
absent paths, ambiguous results and caller membership. Unchanged read digests allow
early cutoff; newly appearing targets invalidate negative reads. Applying a decision
creates fresh snapshot-scoped claims through the ordinary relation owners and never
replays native evidence. This follows the tracked-input/unchanged-result discipline
of [rustc incremental queries](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html)
without introducing a general query engine. Cache membership has a content commitment
over verified payload bytes and pinned reads, avoiding repeated tree serialization.
An unchanged complete cache needs no refresh. Durable cache updates commit with the fenced graph seal and progress
cleanup. Concurrent jobs may replace the cache, but cannot change immutable results.
Cache hydration and complete index construction remain whole-operation costs.
Part membership, phase cursor and worker fence commit together. Retention preserves
parts through live checkpoints; sealing or cancellation releases their membership.
Cooperative file/step/time budgets stop admission between steps, never kill a
parser or interpret unprocessed files as absent. Interrupted batches can repeat;
committed file outcomes survive. Canon preserves explicit analysis-limit outcomes
and assembles its final graph claim only after the bounded file manifest is complete.
Source reobservation, configuration inventory, final assembly/validation and atomic
publication remain whole units. Finer recovery is deferred; measured performance
qualification of existing workflows remains active in PLANNING.
Canon tasks explicitly request structural
claims without fresh native facts. Concurrent graph registration shares this
atomic import owner. Workspace writer locks are OS-owned persistent sidecars;
process exit releases ownership without unlinking the lock identity.

Binding operations freeze an ordered family list, input view, source manifest,
workspace generation and host-attested environment digest. A matching standalone
producer runs each family in a controlled process tree with private scratch,
Cargo target and Go cache directories. The host supplies the pinned, isolated
tool/dependency/configuration environment and prevents concurrent edits; the
digest is an assertion by that host, not an independently verified environment
closure. Existing safe native execution and attributed receipts remain in force.
No installation or compiler execution enters structural inventory.

The worker renews its lease without advancing progress. Each family commits the
normalized graph prefix, seal, retention pin, receipt and cursor in one fenced
transaction; no transaction spans native execution. Failed/budget-limited native
outputs retain useful facts but pause on the same family. Successful partial
binding still carries its explicit coverage limitations. Completed families are
never replayed; an interrupted family may repeat. Final registry publication is
a separate atomic step after a fresh source check. Generic checkpoints cannot
bypass the inventory, canon, binding or revalidation publication owners.
Pause/cancel revokes running execution; nested Entmoot adapter runners stay in
the outer Unix process group. Windows uses Job Objects for explicit tree termination. Abrupt parent death or
tools escaping their process group require host job isolation/cleanup; a stale worker cannot publish. Prefixes remain
private until publication and become collectible after terminal pin release.
Within-family recovery and typed-canon execution are outside this worker.

The initial executable collaboration workflow compares frozen base/left/peer
views and checkpoints completed pairs. Each pair report is committed atomically
with its cursor in a separate bounded output row; paginated reads avoid collecting
the full batch in memory. Reports remain available after terminal pin release. Path/occurrence overlaps and represented
direct dependency interactions are review signals. Optional Git checks use a
private bare repository and the explicitly supplied parent object database;
source refs/index are never changed, and hooks, filters and custom merge drivers
are not run. Explicit
working-tree capture observes tracked and nonignored untracked files separately
from structural inventory. File/byte caps reject incomplete capture; HEAD, index,
path set, modes, bytes and structural source identity are rechecked before CAS.
Capture metadata and replay bytes commit atomically with the workspace generation.
Bytes share inventory's digest store; capture membership follows owning-view
retention and operation pins. Identical structural views can have distinct Git
captures, so batches freeze capture IDs rather than looking up a current tree.
[Fast-import](https://git-scm.com/docs/git-fast-import) reconstructs each captured
raw tree as a synthetic child of its observed HEAD in the private object store;
[merge-tree](https://git-scm.com/docs/git-merge-tree) observes the resulting pair.
Git mode is frozen with the request. Missing parent history remains unavailable.
An arbitrary supplied structural baseline is not called a proven Git merge base.
Raw bytes do not predict index-only selection or clean/EOL/encoding transforms;
sparse/skip-worktree, unmerged indexes and submodules are explicitly unavailable.
Capture is a rechecked observation, not an OS filesystem snapshot against arbitrary
concurrent writers. Symlink targets are retained without reading their referents.

Ordered integration freezes one explicit sequence of registered snapshots and
capture IDs. The shared isolated Git runner merges each member into the previous
result and creates a deterministic synthetic commit with both parents, preserving
ancestry for later members. Each nonterminal clean step stores only newly reachable
objects in a non-thin [Git pack](https://git-scm.com/docs/git-pack-objects).
KnowledgeRepository commits that pack, its BLAKE3 digest, the step report and the
next cursor in the same fenced transaction. No transaction spans Git execution.
A new worker [imports](https://git-scm.com/docs/git-index-pack) the saved deltas
into a fresh private repository; it never reruns completed merges. Original
history remains a dependency of the explicit source object database. Git version
and implementation fences reject incompatible continuation.

The first textual conflict completes the observation with the remaining members
untested; unavailable evidence pauses it. Clean means only this specified textual
integration order. It does not qualify combined structural, native, build or
runtime behavior. Saved-pack limits and cooperative step/time allowances can be
raised on resume without changing frozen inputs. Time is checked before admitting
each merge/restoration; an individual Git process retains its 30-second limit.
The byte allowance bounds retained compressed packs, not transient Git disk use
or decompressed objects. Cancellation/completion releases private packs while
preserving reports; final synthetic IDs describe the observation and do not
promise retained or published Git objects. Existing per-repository queue admission
bounds concurrent integrations, each with independent private Git state.

The hosting workflow supplies consistent isolated tools, for example through
[Hermit](https://github.com/cashapp/hermit). Entmoot uses that environment rather
than installing tools. This prerequisite does not replace source identity,
attribution, transaction fences or independent worktree observations.

Conditional revalidation freezes two retained views and explicitly supplied
producer input policies. The ordinary ledger publisher and durable worker share
policy validation, input-root computation, artifact projection and owner/dependency
mutation. One policy's evidence, applicability changes, receipt and cursor commit
in the same fenced transaction. The private ledger is sealed between steps;
only its owning worker may advance it. A final transaction publishes the immutable
ledger and completes the job. Receipt pages remain available during execution,
while the collaboration record-state read requires a completed retained ledger.
Unreported records return unknown; collection returns unavailable.

This applies dependency-trace and unchanged-result-cutoff ideas from
[Build Systems a la Carte](https://www.microsoft.com/en-us/research/publication/build-systems-la-carte/)
to declared inputs, preserving the distinction in
[verification caching](https://pm.inf.ethz.ch/publications/LeinoWuestholz2015.pdf)
between a dependency checksum and a checked proof obligation. Changed inputs
invalidate applicability; immutable provider evidence retains its source snapshot.
Fresh evidence must already be attached to the exact head. The worker does not
execute native tools, extract unsat cores or validate proof certificates.
Artifact ledgers project only attributed provider records; structural inventory
never belongs to an artifact owner or consumes its record allowance. Independent
relation support is restricted to the selected artifact; conjunctive or derived
support is retained only when all its evidence belongs to that artifact. Both
immutable graphs are validated before mutation; policy preparation reuses their
validated input roots instead of cloning and filtering whole graphs. Unselected
artifacts, including siblings in a selected producer series, remain unknown.
Cumulative record allowances can increase
on resume; cooperative time/step allowances stop between whole policies. An
interrupted transaction rolls back that policy; saved policies are not republished.
Cancellation removes only the operation-owned private ledger and releases pins,
preserving receipts. Existing queue admission bounds independent jobs and SQLite
serializes publication; no transaction spans source projection or process work.

Networking, authentication/upload protocols, replicas and cross-repository
collaboration are deferred. Engine requests and results remain portable; local
database/source locators stay in composition code. Further native recovery and
combined integration qualification are deferred. PLANNING prioritizes existing
implementation quality, CLI/MCP usability and test-driven native/hash coverage.

Dynamic SCC maintenance, dominator invalidation, cross-revision witness repair,
reachability labels and claim propagation are deferred. A dominator could prove
loss of reachability only from specified roots in a complete represented graph;
it would not establish dead code or transfer risk claims to intermediate subjects.

Store enough supplied facts and provenance to explain results. Preserve external
semantics, useful unknown fields and source scope. Raw bytes and repeated full
exports are optional when structured facts suffice; keep them when replay or
debugging warrants the cost. No universal exact-receipt or append-only archive
requirement applies to development evidence.

Normalize provider vocabulary at the adapter boundary. Declaration categories,
binding outcomes and dispatch gaps use shared contracts; native symbols, types
and mechanisms retain attributed detail. An open `indirect_call` reason carries
an extensible language-qualified mechanism. Missing bindings, provider failures,
unsupported semantics and exhausted budgets remain distinct. Receipts and query
frontiers aggregate those categories without discarding the source observation.
Syntax alone cannot establish reflection or a runtime dispatch mechanism.

Reuse parsed trees, indexes and unchanged inputs where practical. Keep bounded
queries and explicit truncation. Optimize measured bottlenecks without weakening
transactions or source attachment. The existing SQLite campaign harness measures
scoped runs; historical measurements are not release commitments.

## Validation and operation

[Testing](../runbooks/testing.md) distinguishes structural, authored-provider,
storage/application and real-native lanes. Focused fixtures author occurrence
selectors and expected relationships independently of production output.
[Adapter setup](../runbooks/resolver-adapters.md) describes optional tool builds.
[Impact](../runbooks/impact.md) and
[compatibility](../runbooks/compatibility.md) contain operator
recipes. Fields and executable invariants belong with Rust types and tests.
